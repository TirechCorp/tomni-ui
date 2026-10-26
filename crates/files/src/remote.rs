//! A [`Remote`]: uma conexão de arquivos numa thread, comandada pela tela.
//!
//! A tela manda pedidos ([`Remote::run`] e os atalhos) e lê as respostas e o estado por
//! [`Live`], que acorda a interface. Um erro de conexão reabre uma vez e repete o pedido;
//! soltar a última cópia da `Remote` fecha a conexão.

use std::io;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, SystemTime};

use crate::entry::{Caps, Entry};
use crate::fs::{Fs, is_connection_error};
use crate::live::Live;

/// Depois de quanto tempo parada a conexão manda um `keepalive`.
pub const KEEPALIVE_PADRAO: Duration = Duration::from_secs(60);

/// Abre uma conexão nova ao mesmo lugar. A [`Remote`] e cada trabalhador das
/// [`Transfers`](crate::Transfers) chamam o seu.
pub type Opener = Arc<dyn Fn() -> io::Result<Box<dyn Fs>> + Send + Sync>;

/// A resposta de um pedido: `None` até chegar; depois o valor, ou a mensagem de erro em
/// português.
pub type Reply<T> = Live<Option<Result<T, String>>>;

/// O estado de uma [`Remote`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteState {
    /// Abrindo a conexão.
    Connecting,
    /// Pronta para os pedidos.
    Ready,
    /// A conexão falhou ou caiu; a mensagem diz por quê. `reconnect` tenta de novo.
    Failed(String),
    /// Fechada de propósito.
    Closed,
}

/// O erro de um pedido ao sistema de arquivos, em texto (o que a tela mostra).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FsError(pub String);

impl std::fmt::Display for FsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for FsError {}

/// O texto de um erro para a tela: a mensagem de `describe` (que já diz o caminho) ou, nos
/// erros de conexão, a do `tomni-ui-net`.
pub(crate) fn error_text(error: &io::Error) -> String {
    let unreachable = matches!(
        error.kind(),
        io::ErrorKind::ConnectionRefused
            | io::ErrorKind::HostUnreachable
            | io::ErrorKind::NetworkUnreachable
    );
    if unreachable || is_connection_error(error) {
        tomni_ui_net::error_message(error)
    } else {
        error.to_string()
    }
}

trait Task: Send {
    /// Executa o pedido. Devolve `Err` só nos erros de conexão (que merecem outra tentativa);
    /// os demais viram a resposta.
    fn attempt(&self, fs: &mut dyn Fs) -> io::Result<()>;
    /// Desiste: a resposta é o erro.
    fn fail(&self, error: &io::Error);
}

type JobFn<T> = Box<dyn Fn(&mut dyn Fs) -> io::Result<T> + Send>;

struct Job<T> {
    job: JobFn<T>,
    reply: Reply<T>,
}

impl<T: Send + 'static> Task for Job<T> {
    fn attempt(&self, fs: &mut dyn Fs) -> io::Result<()> {
        match (self.job)(fs) {
            Ok(value) => self.reply.set(Some(Ok(value))),
            Err(error) if is_connection_error(&error) => return Err(error),
            Err(error) => self.fail(&error),
        }
        Ok(())
    }

    fn fail(&self, error: &io::Error) {
        self.reply.set(Some(Err(error_text(error))));
    }
}

enum Msg {
    Task(Box<dyn Task>),
    Reconnect,
    Close,
}

struct Inner {
    opener: Opener,
    tx: Sender<Msg>,
    state: Live<RemoteState>,
    notice: Live<Option<String>>,
    caps: Arc<Mutex<Option<Caps>>>,
}

/// Uma conexão de arquivos. É barata de copiar; vive na thread da tela.
#[derive(Clone)]
pub struct Remote {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Remote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Remote").finish_non_exhaustive()
    }
}

impl Remote {
    /// Abre a conexão numa thread nova, com o [`KEEPALIVE_PADRAO`].
    pub fn spawn(name: impl Into<String>, opener: Opener) -> Remote {
        Self::spawn_with(name, opener, KEEPALIVE_PADRAO)
    }

    /// Como [`spawn`](Self::spawn), com o tempo de parada que dispara o `keepalive`.
    pub fn spawn_with(name: impl Into<String>, opener: Opener, keepalive: Duration) -> Remote {
        let (tx, rx) = mpsc::channel();
        let state = Live::new(RemoteState::Connecting);
        let notice = Live::new(None);
        let caps = Arc::new(Mutex::new(None));
        let worker = Worker {
            opener: Arc::clone(&opener),
            state: state.clone(),
            notice: notice.clone(),
            caps: Arc::clone(&caps),
            fs: None,
        };
        let spawned = thread::Builder::new()
            .name(format!("remote-{}", name.into()))
            .spawn(move || worker.run(rx, keepalive));
        if let Err(error) = spawned {
            state.set(RemoteState::Failed(error_text(&error)));
        }
        Remote {
            inner: Arc::new(Inner {
                opener,
                tx,
                state,
                notice,
                caps,
            }),
        }
    }

    /// O estado, para a tela mostrar "Conectando…" ou o erro.
    pub fn state(&self) -> Live<RemoteState> {
        self.inner.state.clone()
    }

    /// O último aviso que a conexão deixou (por exemplo, o FTP que passou para o TLS 1.2 porque
    /// o servidor não retoma sessões TLS 1.3). Quem mostra o aviso o apaga com `set(None)`.
    pub fn notice(&self) -> Live<Option<String>> {
        self.inner.notice.clone()
    }

    /// O que a conexão sabe fazer; `None` até conectar.
    pub fn caps(&self) -> Option<Caps> {
        *self
            .inner
            .caps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Quem abre conexões novas ao mesmo lugar (para as [`Transfers`](crate::Transfers)).
    pub fn opener(&self) -> Opener {
        Arc::clone(&self.inner.opener)
    }

    /// Manda um pedido para a thread. `job` roda uma vez, ou duas se a conexão cair no meio e
    /// reabrir; por isso é `Fn`.
    pub fn run<T: Send + 'static>(
        &self,
        job: impl Fn(&mut dyn Fs) -> io::Result<T> + Send + 'static,
    ) -> Reply<T> {
        let reply: Reply<T> = Live::detached(None);
        let task = Job {
            job: Box::new(job),
            reply: reply.clone(),
        };
        if self.inner.tx.send(Msg::Task(Box::new(task))).is_err() {
            reply.set(Some(Err("a conexão está fechada".to_owned())));
        }
        reply
    }

    /// A pasta inicial.
    pub fn home(&self) -> Reply<String> {
        self.run(|fs| fs.home())
    }

    /// O conteúdo de uma pasta.
    pub fn list(&self, dir: &str) -> Reply<Vec<Entry>> {
        let dir = dir.to_owned();
        self.run(move |fs| fs.list(&dir))
    }

    /// Os dados de um caminho (segue o link).
    pub fn stat(&self, path: &str) -> Reply<Entry> {
        let path = path.to_owned();
        self.run(move |fs| fs.stat(&path))
    }

    /// O caminho canônico de `path`, com os links resolvidos (`Unsupported` onde o protocolo
    /// não resolve).
    pub fn real_path(&self, path: &str) -> Reply<String> {
        let path = path.to_owned();
        self.run(move |fs| fs.real_path(&path))
    }

    /// Cria uma pasta.
    pub fn mkdir(&self, path: &str) -> Reply<()> {
        let path = path.to_owned();
        self.run(move |fs| fs.mkdir(&path))
    }

    /// Apaga um arquivo ou link.
    pub fn remove_file(&self, path: &str) -> Reply<()> {
        let path = path.to_owned();
        self.run(move |fs| fs.remove_file(&path))
    }

    /// Apaga uma pasta vazia.
    pub fn remove_dir(&self, path: &str) -> Reply<()> {
        let path = path.to_owned();
        self.run(move |fs| fs.remove_dir(&path))
    }

    /// Renomeia ou move.
    pub fn rename(&self, from: &str, to: &str, overwrite: bool) -> Reply<()> {
        let (from, to) = (from.to_owned(), to.to_owned());
        self.run(move |fs| fs.rename(&from, &to, overwrite))
    }

    /// Muda as permissões.
    pub fn set_permissions(&self, path: &str, mode: u32) -> Reply<()> {
        let path = path.to_owned();
        self.run(move |fs| fs.set_permissions(&path, mode))
    }

    /// Muda a data de modificação.
    pub fn set_modified(&self, path: &str, time: SystemTime) -> Reply<()> {
        let path = path.to_owned();
        self.run(move |fs| fs.set_modified(&path, time))
    }

    /// Reabre a conexão (depois de um `Failed`, ou quando o usuário pede).
    pub fn reconnect(&self) {
        let _ = self.inner.tx.send(Msg::Reconnect);
    }

    /// Fecha a conexão. Os pedidos que já estavam na fila ainda rodam.
    pub fn close(&self) {
        let _ = self.inner.tx.send(Msg::Close);
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Close);
    }
}

struct Worker {
    opener: Opener,
    state: Live<RemoteState>,
    notice: Live<Option<String>>,
    caps: Arc<Mutex<Option<Caps>>>,
    fs: Option<Box<dyn Fs>>,
}

impl Worker {
    fn run(mut self, rx: mpsc::Receiver<Msg>, keepalive: Duration) {
        self.connect();
        loop {
            match rx.recv_timeout(keepalive) {
                Ok(Msg::Task(task)) => self.task(task.as_ref()),
                Ok(Msg::Reconnect) => {
                    self.connect();
                }
                Ok(Msg::Close) | Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => self.keepalive(),
            }
        }
        self.fs = None;
        // Solta a fila antes de avisar: um pedido enviado depois do `Closed` precisa falhar no
        // envio (a resposta de "fechada"), e não ficar numa fila que ninguém mais lê.
        drop(rx);
        self.state.set(RemoteState::Closed);
    }

    /// Abre a conexão; devolve o erro quando não deu (e o estado vira `Failed`).
    fn connect(&mut self) -> Option<io::Error> {
        self.fs = None;
        self.state.set(RemoteState::Connecting);
        match (self.opener)() {
            Ok(fs) => {
                *self.caps.lock().unwrap_or_else(PoisonError::into_inner) = Some(fs.caps());
                self.fs = Some(fs);
                self.state.set(RemoteState::Ready);
                None
            }
            Err(error) => {
                self.state.set(RemoteState::Failed(error_text(&error)));
                Some(error)
            }
        }
    }

    /// Passa para a tela o aviso que a conexão deixou, se houver.
    fn collect_notice(&mut self) {
        if let Some(text) = self.fs.as_mut().and_then(|fs| fs.take_notice()) {
            self.notice.set(Some(text));
        }
    }

    fn task(&mut self, task: &dyn Task) {
        if self.fs.is_none()
            && let Some(error) = self.connect()
        {
            task.fail(&error);
            return;
        }
        let Some(fs) = self.fs.as_mut() else { return };
        let attempt = task.attempt(fs.as_mut());
        self.collect_notice();
        let Err(error) = attempt else {
            return;
        };
        // A conexão caiu: reabre uma vez e repete.
        self.state.set(RemoteState::Failed(error_text(&error)));
        if let Some(error) = self.connect() {
            task.fail(&error);
            return;
        }
        let Some(fs) = self.fs.as_mut() else { return };
        if let Err(error) = task.attempt(fs.as_mut()) {
            self.fs = None;
            self.state.set(RemoteState::Failed(error_text(&error)));
            task.fail(&error);
        }
    }

    fn keepalive(&mut self) {
        let Some(fs) = self.fs.as_mut() else { return };
        if let Err(error) = fs.keepalive() {
            self.fs = None;
            self.state.set(RemoteState::Failed(error_text(&error)));
        }
    }
}
