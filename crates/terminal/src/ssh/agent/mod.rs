//! O agente de chaves do Tomni UI: um servidor do protocolo `ssh-agent` (draft-miller-ssh-agent)
//! que guarda as chaves **em memória** e atende pelo socket Unix (ou pelo pipe nomeado, no
//! Windows). O `ssh`, o `ssh-add` e o [`Ssh`](super::Ssh) falam com ele como com o `ssh-agent`.
//!
//! O agente do `russh` não serve de servidor para isto: o repositório de chaves dele é privado,
//! ele guarda o comentário vazio, ignora a confirmação e assina RSA sempre com SHA-1. Este atende
//! o conjunto que o OpenSSH usa de verdade: listar, assinar (com as variantes `rsa-sha2-*`),
//! adicionar (com validade e confirmação), remover, travar e as extensões.
//!
//! A segurança é de quem usa: o agente não pergunta nada por conta própria. A confirmação de uso
//! (`ssh-add -c`) só existe para a chave que pediu, e sem quem responda ([`Agent::on_confirm`])
//! ela é negada, como no `ssh-agent` sem o `ssh-askpass`. A pergunta diz **quem** pede: o pid do
//! programa, a cadeia de processos que o chamou (`codex ← git ← ssh`) e a chave do servidor para
//! onde o `ssh` vai (a extensão `session-bind@openssh.com`).
//!
//! Chaves de token FIDO (`sk-`) não entram: a privada fica no token e o agente não a tem.
//! Certificados (`*-cert.pub`) entram como uma identidade a mais da chave que os assina.
//!
//! ```no_run
//! use tomni_ui_terminal::ssh::agent::{Agent, Constraints};
//! use tomni_ui_terminal::ssh::keys::{KeyGen, KeyType};
//!
//! let agente = Agent::start(None).expect("subir o agente");
//! println!("export SSH_AUTH_SOCK={}", agente.path().display());
//! let chave = KeyGen::new(KeyType::Ed25519).generate().expect("gerar");
//! agente.add(chave.private_key().clone(), "ana@notebook", Constraints::default()).expect("guardar");
//! assert_eq!(agente.list().len(), 1);
//! ```

mod client;
pub mod pageant;
mod wire;

#[cfg(all(test, unix))]
#[path = "tests.rs"]
mod tests;

#[cfg(all(test, windows))]
#[path = "tests_windows.rs"]
mod tests_windows;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime};

use ssh_key::encoding::Decode;
use ssh_key::private::KeypairData;
use ssh_key::{Algorithm, Certificate, HashAlg, PrivateKey, PublicKey, Signature};
use subtle::ConstantTimeEq;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::runtime::{Builder, Handle};
use tokio::sync::Notify;

pub use client::AgentConnection;
use wire::{Reader, Writer};

/// Quanto o agente espera a resposta de uma confirmação de uso antes de negar.
const CONFIRM_TIMEOUT: Duration = Duration::from_secs(60);

/// O pipe em que o `ssh.exe` do Windows procura o agente quando não há `SSH_AUTH_SOCK`. Existe em
/// todos os sistemas para o app não repetir o texto.
pub const OPENSSH_PIPE: &str = r"\\.\pipe\openssh-ssh-agent";

/// Uma chave que está no agente.
#[derive(Clone, Debug)]
pub struct AgentKey {
    /// A chave pública (a privada nunca sai do agente).
    pub public: PublicKey,
    /// O comentário com que ela foi adicionada.
    pub comment: String,
    /// Quando foi adicionada.
    pub added: SystemTime,
    /// Quando o agente a esquece sozinho (`ssh-add -t`).
    pub expires: Option<SystemTime>,
    /// Se cada uso pede confirmação (`ssh-add -c`).
    pub confirm: bool,
    /// O certificado, quando esta entrada é um (`ssh-add` com o `-cert.pub` ao lado). `public` é
    /// a chave que ele certifica; a privada é a mesma da entrada sem certificado.
    pub certificate: Option<Certificate>,
}

/// As restrições ao adicionar uma chave.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Constraints {
    /// O agente esquece a chave depois deste tempo.
    pub lifetime: Option<Duration>,
    /// Cada assinatura pede confirmação a [`Agent::on_confirm`].
    pub confirm: bool,
}

/// Quem pede o uso de uma chave e a própria chave: o que [`Agent::on_confirm`] recebe.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ConfirmRequest {
    /// A chave que vai assinar.
    pub key: AgentKey,
    /// Quem pediu.
    pub peer: Peer,
}

/// O programa que fala com o agente.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct Peer {
    /// O pid do processo do outro lado do socket (ou do pipe); `None` onde o sistema não diz.
    pub pid: Option<u32>,
    /// A cadeia de processos do pedinte para cima, com no máximo 6 nomes (`["ssh", "git",
    /// "codex"]`), sem o `init`/`launchd`/`systemd`. Só é calculada quando há pergunta.
    pub chain: Vec<String>,
    /// A chave do servidor a que o `ssh` anunciou (`session-bind@openssh.com`) que a conexão vai.
    pub host_key: Option<PublicKey>,
}

/// Por que o agente não guardou uma chave.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AgentError {
    /// Chave de token FIDO (`sk-`): a privada fica no token.
    SecurityKey,
    /// A chave não pôde ser lida ou codificada.
    Invalid(String),
}

impl std::fmt::Display for AgentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SecurityKey => f.write_str(
                "O agente não guarda chaves de token FIDO (sk-): a privada fica no token.",
            ),
            Self::Invalid(motivo) => write!(f, "A chave não pôde ser guardada: {motivo}"),
        }
    }
}

impl std::error::Error for AgentError {}

type ConfirmFn = dyn Fn(&ConfirmRequest) -> bool + Send + Sync;
type ChangeFn = dyn Fn() + Send + Sync;

struct Entry {
    id: u64,
    key: Arc<PrivateKey>,
    blob: Vec<u8>,
    info: AgentKey,
}

#[derive(Default)]
struct Store {
    entries: Vec<Entry>,
    /// A passphrase de `LOCK`; com ela o agente está travado.
    locked: Option<Vec<u8>>,
    next_id: u64,
}

/// O que a thread do agente e o `Agent` dividem.
struct Shared {
    store: Mutex<Store>,
    confirm: Mutex<Option<Arc<ConfirmFn>>>,
    change: Mutex<Option<Arc<ChangeFn>>>,
    /// A validade das chaves adicionadas sem uma (`ssh-agent -t`).
    default_lifetime: Mutex<Option<Duration>>,
    path: PathBuf,
    runtime: Handle,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

impl Shared {
    /// Avisa quem acompanha as chaves. Nunca chamar com o `store` travado.
    fn changed(&self) {
        let aviso = lock(&self.change).clone();
        if let Some(aviso) = aviso {
            aviso();
        }
    }

    fn add(
        self: &Arc<Self>,
        key: PrivateKey,
        comment: &str,
        c: Constraints,
    ) -> Result<(), AgentError> {
        if matches!(
            key.key_data(),
            KeypairData::SkEd25519(_) | KeypairData::SkEcdsaSha2NistP256(_)
        ) {
            return Err(AgentError::SecurityKey);
        }
        let public = key.public_key().clone();
        let blob = public
            .to_bytes()
            .map_err(|error| AgentError::Invalid(error.to_string()))?;
        let c = Constraints {
            lifetime: c.lifetime.or(*lock(&self.default_lifetime)),
            ..c
        };
        let agora = SystemTime::now();
        // Adicionar de novo a mesma chave troca a anterior (e os certificados dela, que o
        // `ssh-add` manda de novo), como no OpenSSH.
        let expires = c.lifetime.map(|v| agora + v);
        self.insert(
            Some(&public),
            Entry {
                id: 0,
                key: Arc::new(key),
                blob,
                info: AgentKey {
                    public: public.clone(),
                    comment: comment.to_owned(),
                    added: agora,
                    expires,
                    confirm: c.confirm,
                    certificate: None,
                },
            },
        );
        Ok(())
    }

    /// Guarda o certificado como uma identidade a mais da chave privada que ele certifica.
    /// `None` em `c` herda a validade e a confirmação da chave; `Some` só pode apertar. Falso se
    /// a privada não está no agente.
    fn add_certificate(
        self: &Arc<Self>,
        cert: Certificate,
        comment: &str,
        c: Option<Constraints>,
    ) -> bool {
        let Ok(blob) = cert.to_bytes() else {
            return false;
        };
        let public = PublicKey::from(cert.public_key().clone());
        let Ok(plain) = public.to_bytes() else {
            return false;
        };
        let agora = SystemTime::now();
        let achada = lock(&self.store)
            .entries
            .iter()
            .find(|e| e.blob == plain)
            .map(|e| (Arc::clone(&e.key), e.info.clone()));
        let Some((key, base)) = achada else {
            return false;
        };
        let c = c.unwrap_or_default();
        let expires = match (c.lifetime.map(|v| agora + v), base.expires) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        self.insert(
            None,
            Entry {
                id: 0,
                key,
                blob,
                info: AgentKey {
                    public,
                    comment: comment.to_owned(),
                    added: agora,
                    expires,
                    confirm: c.confirm || base.confirm,
                    certificate: Some(cert),
                },
            },
        );
        true
    }

    /// Põe `entry` na lista (tirando a que tem o mesmo blob e, se `troca` vier, a chave e os
    /// certificados dela) e agenda o fim da validade.
    fn insert(self: &Arc<Self>, troca: Option<&PublicKey>, mut entry: Entry) {
        let expires = entry.info.expires;
        let id = {
            let mut store = lock(&self.store);
            store.entries.retain(|e| {
                e.blob != entry.blob
                    && troca.is_none_or(|p| e.info.public.key_data() != p.key_data())
            });
            store.next_id += 1;
            entry.id = store.next_id;
            let id = entry.id;
            store.entries.push(entry);
            id
        };
        if let Some(fim) = expires {
            let vida = fim.duration_since(SystemTime::now()).unwrap_or_default();
            let agente = Arc::clone(self);
            self.runtime.spawn(async move {
                tokio::time::sleep(vida).await;
                agente.expire(id);
            });
        }
        self.changed();
    }

    /// Esquece a chave `id`, se ainda for a mesma (e não uma adicionada de novo).
    fn expire(&self, id: u64) {
        let tirou = {
            let mut store = lock(&self.store);
            let antes = store.entries.len();
            store.entries.retain(|e| e.id != id);
            store.entries.len() != antes
        };
        if tirou {
            self.changed();
        }
    }

    /// Tira as chaves vencidas (o `sleep` pode atrasar, a lista não pode mentir).
    fn purge(&self) {
        let agora = SystemTime::now();
        let tirou = {
            let mut store = lock(&self.store);
            let antes = store.entries.len();
            store
                .entries
                .retain(|e| e.info.expires.is_none_or(|fim| fim > agora));
            store.entries.len() != antes
        };
        if tirou {
            self.changed();
        }
    }

    /// Tira a identidade `blob`. Tirar a chave leva junto os certificados dela; tirar um
    /// certificado deixa a chave.
    fn remove_blob(&self, blob: &[u8]) -> bool {
        let tirou = {
            let mut store = lock(&self.store);
            let antes = store.entries.len();
            let chave = store
                .entries
                .iter()
                .find(|e| e.blob == blob && e.info.certificate.is_none())
                .map(|e| e.info.public.clone());
            store.entries.retain(|e| {
                e.blob != blob
                    && chave
                        .as_ref()
                        .is_none_or(|p| e.info.public.key_data() != p.key_data())
            });
            store.entries.len() != antes
        };
        if tirou {
            self.changed();
        }
        tirou
    }

    fn remove_all(&self) {
        let tinha = {
            let mut store = lock(&self.store);
            let tinha = !store.entries.is_empty();
            store.entries.clear();
            tinha
        };
        if tinha {
            self.changed();
        }
    }

    fn lock_with(&self, passphrase: &[u8]) -> bool {
        {
            let mut store = lock(&self.store);
            if store.locked.is_some() {
                return false;
            }
            store.locked = Some(passphrase.to_vec());
        }
        self.changed();
        true
    }

    fn unlock_with(&self, passphrase: &[u8]) -> bool {
        {
            let mut store = lock(&self.store);
            let Some(guardada) = store.locked.as_deref() else {
                return false;
            };
            // Em tempo constante: o tempo da resposta não conta quantos bytes acertaram.
            let igual =
                guardada.len() == passphrase.len() && bool::from(guardada.ct_eq(passphrase));
            if !igual {
                return false;
            }
            store.locked = None;
        }
        self.changed();
        true
    }

    fn list(&self) -> Vec<AgentKey> {
        self.purge();
        lock(&self.store)
            .entries
            .iter()
            .map(|e| e.info.clone())
            .collect()
    }
}

/// Para a thread do agente quando o último clone cai.
struct Guard {
    stop: Arc<Notify>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Guard {
    fn stop(&self) {
        self.stop.notify_one();
        if let Some(thread) = lock(&self.thread).take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Por que o [`Agent::start_pageant`] não ligou.
#[cfg(windows)]
#[derive(Debug)]
#[non_exhaustive]
pub enum PageantError {
    /// Já há um Pageant rodando (outro programa, ou outro agente deste processo).
    OutroPageant,
    /// O Windows recusou o pipe ou a janela.
    Io(io::Error),
}

#[cfg(windows)]
impl std::fmt::Display for PageantError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutroPageant => f.write_str("Já há um Pageant rodando"),
            Self::Io(e) => write!(f, "Não foi possível ligar o Pageant: {e}"),
        }
    }
}

#[cfg(windows)]
impl std::error::Error for PageantError {}

/// O Pageant ligado: soltar desliga o pipe e fecha a janela.
#[cfg(windows)]
pub struct PageantGuard {
    stop: Arc<Notify>,
    janela: Option<tomni_ui_win32::CopyDataWindow>,
}

#[cfg(windows)]
impl std::fmt::Debug for PageantGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PageantGuard").finish_non_exhaustive()
    }
}

#[cfg(windows)]
impl Drop for PageantGuard {
    fn drop(&mut self) {
        self.stop.notify_one();
        // Fecha a janela (e espera a thread dela) antes de o pipe sumir.
        self.janela.take();
    }
}

/// O agente do processo: guarda as chaves em memória e atende pelo socket (Unix) ou pelo pipe
/// nomeado (Windows). Clonar é barato; o último clone que cai fecha o socket e apaga o arquivo.
#[derive(Clone)]
pub struct Agent {
    shared: Arc<Shared>,
    guard: Arc<Guard>,
}

impl std::fmt::Debug for Agent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Agent")
            .field("path", &self.shared.path)
            .finish()
    }
}

impl Agent {
    /// Sobe o agente numa thread própria (runtime `current_thread` do tokio, como o
    /// [`Ssh`](super::Ssh)).
    ///
    /// `path` = `None` escolhe `$XDG_RUNTIME_DIR/tomni-ui/agent.<pid>`, ou
    /// `<temp>/tomni-ui-<usuário>/agent.<pid>`, com a pasta em 0700 e o socket em 0600. No
    /// Windows o padrão é o pipe `\\.\pipe\tomni-ui-agent-<usuário>`.
    ///
    /// Um arquivo de socket que sobrou de outro processo (ninguém responde nele) é apagado e
    /// refeito. Se alguém responde, o erro é `AddrInUse` ("Já há um agente em `<caminho>`").
    pub fn start(path: Option<PathBuf>) -> io::Result<Agent> {
        let padrao = path.is_none();
        let path = match path {
            Some(path) => path,
            None => sock::default_path()?,
        };
        let runtime = Builder::new_current_thread().enable_all().build()?;
        let listener = {
            let _contexto = runtime.enter();
            sock::bind(&path, padrao)?
        };
        let shared = Arc::new(Shared {
            store: Mutex::new(Store::default()),
            confirm: Mutex::new(None),
            change: Mutex::new(None),
            default_lifetime: Mutex::new(None),
            path: path.clone(),
            runtime: runtime.handle().clone(),
        });
        let stop = Arc::new(Notify::new());
        let thread = {
            let shared = Arc::clone(&shared);
            let stop = Arc::clone(&stop);
            std::thread::Builder::new()
                .name("tomni-ui-ssh-agent".into())
                .spawn(move || {
                    runtime.block_on(sock::serve(listener, shared, stop));
                    // Uma confirmação pendente não pode segurar o fim do agente.
                    runtime.shutdown_background();
                    sock::cleanup(&path);
                })?
        };
        Ok(Agent {
            shared,
            guard: Arc::new(Guard {
                stop,
                thread: Mutex::new(Some(thread)),
            }),
        })
    }

    /// Liga o agente ao **Pageant**: o pipe `\\.\pipe\pageant.<usuário>.<hash>` e a janela
    /// oculta "Pageant", os dois jeitos de o `plink`, o `WinSCP`, o `git` do PuTTY e o resto
    /// falarem com ele. Só o usuário atual os alcança (o dono do pipe e do mapa de memória é
    /// conferido). Soltar o [`PageantGuard`] desliga os dois.
    ///
    /// Se já há um Pageant (de outro programa, ou de outro agente), devolve
    /// [`PageantError::OutroPageant`] sem ligar nada.
    #[cfg(windows)]
    pub fn start_pageant(&self) -> Result<PageantGuard, PageantError> {
        use self::pageant::{
            AGENT_COPYDATA_ID, copydata_exchange, pageant_obfuscation_input, pageant_pipe_name,
            pageant_pipe_suffix,
        };
        let usuario = tomni_ui_win32::pageant_user_name().map_err(PageantError::Io)?;
        // O PuTTY usa o texto sem cifra quando o `CryptProtectMemory` falha.
        let mut entrada = pageant_obfuscation_input();
        let _ = tomni_ui_win32::protect_memory_cross_process(&mut entrada);
        let nome = pageant_pipe_name(&usuario, &pageant_pipe_suffix(&entrada));
        if tomni_ui_win32::window_exists("Pageant", "Pageant")
            || std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&nome)
                .is_ok()
        {
            return Err(PageantError::OutroPageant);
        }
        let listener = {
            let _contexto = self.shared.runtime.enter();
            sock::bind_name(&nome).map_err(|e| {
                if e.kind() == io::ErrorKind::AddrInUse {
                    PageantError::OutroPageant
                } else {
                    PageantError::Io(e)
                }
            })?
        };
        let stop = Arc::new(Notify::new());
        self.shared.runtime.spawn(sock::serve_pipe(
            listener,
            Arc::clone(&self.shared),
            Arc::clone(&stop),
        ));
        let shared = Arc::clone(&self.shared);
        let janela = tomni_ui_win32::CopyDataWindow::start(
            "Pageant",
            "Pageant",
            AGENT_COPYDATA_ID,
            move |mapa| {
                copydata_exchange(mapa, |corpo| {
                    // O `WM_COPYDATA` não diz quem é o cliente: sem pid.
                    let pedido = corpo.to_vec();
                    let (envia, recebe) = std::sync::mpsc::channel();
                    let shared = Arc::clone(&shared);
                    let runtime = shared.runtime.clone();
                    runtime.spawn(async move {
                        let mut conn = Conn {
                            pid: None,
                            host_key: None,
                        };
                        let _ = envia.send(respond(&shared, &pedido, &mut conn).await);
                    });
                    recebe
                        .recv()
                        .unwrap_or_else(|_| wire::simple(wire::FAILURE))
                })
            },
        )
        .map_err(|e| {
            stop.notify_one();
            PageantError::Io(e)
        })?;
        Ok(PageantGuard {
            stop,
            janela: Some(janela),
        })
    }

    /// O que vai no `SSH_AUTH_SOCK`: o caminho do socket (ou do pipe).
    pub fn path(&self) -> &Path {
        &self.shared.path
    }

    /// Guarda a chave (a mesma chave de novo troca a anterior). Chave de token FIDO (`sk-`) é
    /// recusada: a privada não está aqui.
    pub fn add(&self, key: PrivateKey, comment: &str, c: Constraints) -> Result<(), AgentError> {
        self.shared.add(key, comment, c)
    }

    /// Guarda o certificado como uma identidade a mais da chave que ele certifica (a validade e a
    /// confirmação são as da chave). Falso se a chave privada ainda não está no agente.
    pub fn add_certificate(&self, cert: Certificate, comment: &str) -> bool {
        self.shared.add_certificate(cert, comment, None)
    }

    /// A validade das chaves adicionadas sem uma própria (o `-t` do `ssh-agent`). `None` as
    /// deixa valer até serem tiradas. Não muda as que já estão no agente.
    pub fn default_lifetime(&self, lifetime: Option<Duration>) {
        *lock(&self.shared.default_lifetime) = lifetime;
    }

    /// Tira a chave. Devolve se ela estava lá.
    pub fn remove(&self, public: &PublicKey) -> bool {
        public
            .to_bytes()
            .is_ok_and(|blob| self.shared.remove_blob(&blob))
    }

    /// Esquece todas as chaves.
    pub fn remove_all(&self) {
        self.shared.remove_all();
    }

    /// As chaves guardadas (sem as vencidas), na ordem em que entraram.
    pub fn list(&self) -> Vec<AgentKey> {
        self.shared.list()
    }

    /// Trava o agente: ele deixa de listar e de assinar até o `unlock` com a mesma passphrase.
    /// Devolve `false` se já estava travado.
    pub fn lock(&self, passphrase: &str) -> bool {
        self.shared.lock_with(passphrase.as_bytes())
    }

    /// Destrava. Devolve `false` se a passphrase não confere ou se não estava travado.
    pub fn unlock(&self, passphrase: &str) -> bool {
        self.shared.unlock_with(passphrase.as_bytes())
    }

    /// Se o agente está travado.
    pub fn is_locked(&self) -> bool {
        lock(&self.shared.store).locked.is_some()
    }

    /// Chamada fora da thread do agente quando uma chave com `confirm` vai assinar, com a chave e
    /// quem pede ([`ConfirmRequest`]). Devolve se permite. Sem ela (ou sem resposta em 60 s) o
    /// uso é negado, como no `ssh-agent` sem o `ssh-askpass`.
    pub fn on_confirm(&self, f: impl Fn(&ConfirmRequest) -> bool + Send + Sync + 'static) {
        *lock(&self.shared.confirm) = Some(Arc::new(f));
    }

    /// Chamada, na thread de quem mudou, quando as chaves ou o travamento mudam: um `ssh-add`
    /// pelo socket, uma chave que venceu, uma chamada deste `Agent`. O app a usa para acordar a
    /// tela.
    pub fn on_change(&self, f: impl Fn() + Send + Sync + 'static) {
        *lock(&self.shared.change) = Some(Arc::new(f));
    }

    /// Fecha o socket e apaga o arquivo. Os clones passam a não atender.
    pub fn stop(&self) {
        self.guard.stop();
    }
}

// ---- o protocolo ----

/// O que se sabe de uma conexão: quem está do outro lado e a que servidor o `ssh` disse que vai.
#[derive(Default)]
struct Conn {
    pid: Option<u32>,
    host_key: Option<PublicKey>,
}

/// Responde a um pedido (sem a moldura de tamanho).
async fn respond(shared: &Arc<Shared>, pedido: &[u8], conn: &mut Conn) -> Vec<u8> {
    let mut r = Reader(pedido);
    let Some(tipo) = r.u8() else {
        return wire::simple(wire::FAILURE);
    };
    let travado = lock(&shared.store).locked.is_some();
    match tipo {
        wire::REQUEST_IDENTITIES => {
            shared.purge();
            let store = lock(&shared.store);
            let mut w = Writer::new(wire::IDENTITIES_ANSWER);
            // Travado, a lista vem vazia (não `FAILURE`), como no OpenSSH.
            let visiveis: Vec<&Entry> = if travado {
                Vec::new()
            } else {
                store.entries.iter().collect()
            };
            w.u32(visiveis.len() as u32);
            for e in visiveis {
                w.string(&e.blob).string(e.info.comment.as_bytes());
            }
            w.0
        }
        wire::SIGN_REQUEST if !travado => sign(shared, r, conn).await,
        wire::ADD_IDENTITY if !travado => add_identity(shared, r, false),
        wire::ADD_ID_CONSTRAINED if !travado => add_identity(shared, r, true),
        wire::REMOVE_IDENTITY if !travado => match r.string() {
            Some(blob) if shared.remove_blob(blob) => wire::simple(wire::SUCCESS),
            _ => wire::simple(wire::FAILURE),
        },
        wire::REMOVE_ALL_IDENTITIES if !travado => {
            shared.remove_all();
            wire::simple(wire::SUCCESS)
        }
        wire::LOCK => ok(r.string().is_some_and(|p| shared.lock_with(p))),
        wire::UNLOCK => ok(r.string().is_some_and(|p| shared.unlock_with(p))),
        wire::EXTENSION => match r.string() {
            // O `query` anuncia as extensões que o agente atende: nenhuma.
            Some(b"query") => wire::simple(wire::SUCCESS),
            // O `ssh` avisa a que servidor a conexão serve: guarda a chave dele para a pergunta
            // de uso dizer "para github.com". Não confere a assinatura, é só informação.
            Some(b"session-bind@openssh.com") => match session_bind(r) {
                Some(host_key) => {
                    conn.host_key = host_key;
                    wire::simple(wire::SUCCESS)
                }
                None => wire::simple(wire::FAILURE),
            },
            _ => wire::simple(wire::FAILURE),
        },
        _ => wire::simple(wire::FAILURE),
    }
}

/// Lê `string hostkey, string session_id, string signature, bool forwarding`. `None` se vier
/// cortado; `Some(None)` se a chave do servidor for de um tipo que não conhecemos.
fn session_bind(mut r: Reader<'_>) -> Option<Option<PublicKey>> {
    let host_key = r.string()?;
    let _session_id = r.string()?;
    let _signature = r.string()?;
    let _forwarding = r.u8()?;
    Some(PublicKey::from_bytes(host_key).ok())
}

fn ok(deu_certo: bool) -> Vec<u8> {
    wire::simple(if deu_certo {
        wire::SUCCESS
    } else {
        wire::FAILURE
    })
}

fn add_identity(shared: &Arc<Shared>, mut r: Reader<'_>, com_restricoes: bool) -> Vec<u8> {
    let tipo = Reader(r.0).string().unwrap_or_default();
    if tipo.ends_with(b"-cert-v01@openssh.com") {
        return add_certificate(shared, r, com_restricoes);
    }
    // O corpo é o mesmo da chave privada do arquivo OpenSSH: algoritmo e campos.
    let mut resto: &[u8] = r.0;
    let Ok(dados) = KeypairData::decode(&mut resto) else {
        return wire::simple(wire::FAILURE);
    };
    r.0 = resto;
    let Some(comentario) = r.string() else {
        return wire::simple(wire::FAILURE);
    };
    let Some(restricoes) = constraints(&mut r, com_restricoes) else {
        return wire::simple(wire::FAILURE);
    };
    let Ok(chave) = PrivateKey::new(dados, String::from_utf8_lossy(comentario).into_owned()) else {
        return wire::simple(wire::FAILURE);
    };
    ok(shared
        .add(chave, &String::from_utf8_lossy(comentario), restricoes)
        .is_ok())
}

/// O `ADD_IDENTITY` de um certificado: `string tipo, string certificado, <campos privados>,
/// string comentário`. A privada já está no agente (o `ssh-add` manda a chave antes), então os
/// campos privados só são pulados: quantos são depende do algoritmo.
fn add_certificate(shared: &Arc<Shared>, mut r: Reader<'_>, com_restricoes: bool) -> Vec<u8> {
    let (Some(_tipo), Some(blob)) = (r.string(), r.string()) else {
        return wire::simple(wire::FAILURE);
    };
    let Ok(cert) = Certificate::from_bytes(blob) else {
        return wire::simple(wire::FAILURE);
    };
    let privados = match cert.public_key().algorithm() {
        Algorithm::Ed25519 => 2,
        Algorithm::Ecdsa { .. } => 1,
        Algorithm::Rsa { .. } => 4,
        _ => return wire::simple(wire::FAILURE),
    };
    for _ in 0..privados {
        if r.string().is_none() {
            return wire::simple(wire::FAILURE);
        }
    }
    let Some(comentario) = r.string() else {
        return wire::simple(wire::FAILURE);
    };
    let Some(restricoes) = constraints(&mut r, com_restricoes) else {
        return wire::simple(wire::FAILURE);
    };
    ok(shared.add_certificate(cert, &String::from_utf8_lossy(comentario), Some(restricoes)))
}

/// As restrições que vêm depois do comentário. `None` se algo não se entende: o agente recusa em
/// vez de adivinhar (`EXTENSION`, 255, e o que não conhecemos).
fn constraints(r: &mut Reader<'_>, com_restricoes: bool) -> Option<Constraints> {
    let mut restricoes = Constraints::default();
    if com_restricoes {
        while !r.is_empty() {
            match r.u8() {
                Some(wire::CONSTRAIN_LIFETIME) => {
                    restricoes.lifetime = Some(Duration::from_secs(r.u32()?.into()));
                }
                Some(wire::CONSTRAIN_CONFIRM) => restricoes.confirm = true,
                _ => return None,
            }
        }
    }
    Some(restricoes)
}

async fn sign(shared: &Arc<Shared>, mut r: Reader<'_>, conn: &Conn) -> Vec<u8> {
    let (Some(blob), Some(dados)) = (r.string(), r.string()) else {
        return wire::simple(wire::FAILURE);
    };
    let flags = r.u32().unwrap_or(0);
    shared.purge();
    let achada = lock(&shared.store)
        .entries
        .iter()
        .find(|e| e.blob == blob)
        .map(|e| (Arc::clone(&e.key), e.info.clone()));
    let Some((chave, info)) = achada else {
        return wire::simple(wire::FAILURE);
    };
    if info.confirm && !confirm(shared, info, conn).await {
        return wire::simple(wire::FAILURE);
    }
    match sign_with(&chave, dados, flags) {
        Some(assinatura) => {
            let mut w = Writer::new(wire::SIGN_RESPONSE);
            w.string(&assinatura);
            w.0
        }
        None => wire::simple(wire::FAILURE),
    }
}

/// Pergunta a quem registrou o `on_confirm`, sem travar o agente (a pergunta espera uma pessoa).
/// A cadeia de processos só é montada aqui, quando há pergunta.
async fn confirm(shared: &Arc<Shared>, info: AgentKey, conn: &Conn) -> bool {
    let Some(pergunta) = lock(&shared.confirm).clone() else {
        return false;
    };
    let (pid, host_key) = (conn.pid, conn.host_key.clone());
    let resposta = tokio::task::spawn_blocking(move || {
        let chain = pid.map(process_chain).unwrap_or_default();
        pergunta(&ConfirmRequest {
            key: info,
            peer: Peer {
                pid,
                chain,
                host_key,
            },
        })
    });
    matches!(
        tokio::time::timeout(CONFIRM_TIMEOUT, resposta).await,
        Ok(Ok(true))
    )
}

/// A assinatura no formato do protocolo (nome do algoritmo e bytes), com o hash que o cliente
/// pediu nas `flags` (RSA: 2 = SHA-256, 4 = SHA-512, 0 = `ssh-rsa`, SHA-1).
fn sign_with(chave: &PrivateKey, dados: &[u8], flags: u32) -> Option<Vec<u8>> {
    use russh::keys::signature::Signer;
    use ssh_key::encoding::Encode;

    let assinatura: Signature = match chave.key_data() {
        KeypairData::Rsa(rsa) => {
            let hash = if flags & 2 != 0 {
                Some(HashAlg::Sha256)
            } else if flags & 4 != 0 {
                Some(HashAlg::Sha512)
            } else {
                None
            };
            Signer::try_sign(&(rsa, hash), dados).ok()?
        }
        outra => Signer::try_sign(outra, dados).ok()?,
    };
    debug_assert!(!matches!(chave.algorithm(), Algorithm::Other(_)));
    assinatura.encode_vec().ok()
}

/// Atende uma conexão até o cliente sair ou mandar uma moldura inválida.
async fn serve_connection<S: AsyncRead + AsyncWrite + Unpin>(
    shared: Arc<Shared>,
    mut s: S,
    pid: Option<u32>,
) {
    let mut conn = Conn {
        pid,
        host_key: None,
    };
    loop {
        let mut tamanho = [0u8; 4];
        if s.read_exact(&mut tamanho).await.is_err() {
            return;
        }
        let tamanho = u32::from_be_bytes(tamanho) as usize;
        if tamanho == 0 || tamanho > wire::MAX_FRAME {
            return;
        }
        let mut pedido = vec![0u8; tamanho];
        if s.read_exact(&mut pedido).await.is_err() {
            return;
        }
        let resposta = respond(&shared, &pedido, &mut conn).await;
        if s.write_all(&wire::framed(&resposta)).await.is_err() || s.flush().await.is_err() {
            return;
        }
    }
}

/// No máximo quantos nomes a cadeia de processos tem.
const CHAIN_MAX: usize = 6;

/// Os nomes dos processos de `pid` para cima: o próprio programa, quem o chamou e assim por
/// diante (`["ssh", "git", "codex"]`), parando em `CHAIN_MAX`, no `init`, `launchd` ou `systemd`
/// (que não entram) ou onde o sistema não diz mais. Linux por `/proc`, macOS por `ps`, Windows
/// pela tabela de processos; nos outros sistemas, vazia.
pub fn process_chain(pid: u32) -> Vec<String> {
    chain_with(pid, process_info)
}

/// A cadeia, dado como achar o pai e o nome de um processo.
fn chain_with(pid: u32, info: impl Fn(u32) -> Option<(u32, String)>) -> Vec<String> {
    let mut chain = Vec::new();
    let mut atual = pid;
    let mut vistos = Vec::new();
    while chain.len() < CHAIN_MAX && atual > 1 && !vistos.contains(&atual) {
        vistos.push(atual);
        let Some((pai, nome)) = info(atual) else {
            break;
        };
        if matches!(nome.as_str(), "init" | "launchd" | "systemd") {
            break;
        }
        chain.push(nome);
        atual = pai;
    }
    chain
}

/// O pai e o nome de `pid`.
#[cfg(target_os = "linux")]
fn process_info(pid: u32) -> Option<(u32, String)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (pai, comm) = parse_proc_stat(&stat)?;
    let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
    let nome = script_name(&comm, &cmdline).unwrap_or(comm);
    Some((pai, nome))
}

#[cfg(target_os = "macos")]
fn process_info(pid: u32) -> Option<(u32, String)> {
    let saida = std::process::Command::new("ps")
        .args(["-o", "ppid=,comm=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    let texto = String::from_utf8_lossy(&saida.stdout);
    let texto = texto.trim();
    let (pai, comm) = texto.split_once(char::is_whitespace)?;
    let nome = comm.trim().rsplit('/').next()?.to_owned();
    Some((pai.parse().ok()?, nome))
}

/// No Windows, pela tabela de processos (`CreateToolhelp32Snapshot`), sem o `.exe` no nome.
#[cfg(windows)]
fn process_info(pid: u32) -> Option<(u32, String)> {
    let entrada = tomni_ui_win32::process_table()
        .ok()?
        .into_iter()
        .find(|e| e.pid == pid)?;
    let nome = entrada
        .name
        .strip_suffix(".exe")
        .unwrap_or(&entrada.name)
        .to_owned();
    Some((entrada.parent_pid, nome))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn process_info(_pid: u32) -> Option<(u32, String)> {
    None
}

/// O `ppid` e o nome do `/proc/<pid>/stat`: `pid (nome) S ppid …`. O nome pode ter espaços e
/// parênteses, então vale o último `)`.
#[cfg(any(target_os = "linux", all(test, unix)))]
fn parse_proc_stat(stat: &str) -> Option<(u32, String)> {
    let abre = stat.find('(')?;
    let fecha = stat.rfind(')')?;
    let nome = stat.get(abre + 1..fecha)?.to_owned();
    let mut campos = stat.get(fecha + 1..)?.split_whitespace();
    let _estado = campos.next()?;
    Some((campos.next()?.parse().ok()?, nome))
}

/// Para `node`, `python` e parecidos, o nome do script em vez do interpretador: o `codex` roda
/// como `node /…/bin/codex`. `cmdline` separa os argumentos por `\0`.
#[cfg(any(target_os = "linux", all(test, unix)))]
fn script_name(comm: &str, cmdline: &[u8]) -> Option<String> {
    let interpretador = comm == "node"
        || comm == "bun"
        || comm == "deno"
        || comm == "ruby"
        || comm == "perl"
        || comm.starts_with("python");
    if !interpretador {
        return None;
    }
    let script = cmdline
        .split(|&b| b == 0)
        .skip(1)
        .find(|arg| !arg.is_empty() && !arg.starts_with(b"-"))?;
    let script = String::from_utf8_lossy(script);
    let base = script.rsplit('/').next()?;
    (!base.is_empty()).then(|| base.to_owned())
}

#[cfg(unix)]
mod sock {
    use super::*;
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    use tokio::net::UnixListener;

    pub(super) fn default_path() -> io::Result<PathBuf> {
        let pasta = match std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) {
            Some(base) => PathBuf::from(base).join("tomni-ui"),
            None => {
                let usuario = std::env::var("USER")
                    .or_else(|_| std::env::var("LOGNAME"))
                    .unwrap_or_else(|_| "usuario".into());
                std::env::temp_dir().join(format!("tomni-ui-{usuario}"))
            }
        };
        Ok(pasta.join(format!("agent.{}", std::process::id())))
    }

    pub(super) fn bind(path: &Path, padrao: bool) -> io::Result<UnixListener> {
        // O campo `sun_path` tem 108 bytes (104 no macOS), com o zero do fim: um caminho maior
        // não tem como virar socket. Dizê-lo aqui evita o erro da std e as pastas criadas à toa.
        let limite = if cfg!(target_os = "macos") { 103 } else { 107 };
        let tamanho = path.as_os_str().len();
        if tamanho > limite {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "O caminho do socket tem {tamanho} bytes, e o sistema aceita até {limite}: {}",
                    path.display()
                ),
            ));
        }
        if let Some(pasta) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(pasta)?;
            if padrao {
                // A pasta do agente é só de quem a criou, ainda que já existisse.
                std::fs::set_permissions(pasta, std::fs::Permissions::from_mode(0o700))?;
            }
        }
        if path.exists() {
            // Um socket que sobrou: se ninguém responde, é lixo; se alguém responde, é outro agente.
            if std::os::unix::net::UnixStream::connect(path).is_ok() {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    format!("Já há um agente em {}", path.display()),
                ));
            }
            std::fs::remove_file(path)?;
        }
        let listener = UnixListener::bind(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        Ok(listener)
    }

    pub(super) async fn serve(listener: UnixListener, shared: Arc<Shared>, stop: Arc<Notify>) {
        loop {
            tokio::select! {
                biased;
                _ = stop.notified() => return,
                aceito = listener.accept() => {
                    let Ok((conexao, _)) = aceito else { continue };
                    // Quem está do outro lado (`SO_PEERCRED` no Linux, `LOCAL_PEERPID` no macOS).
                    let pid = conexao
                        .peer_cred()
                        .ok()
                        .and_then(|c| c.pid())
                        .and_then(|pid| u32::try_from(pid).ok());
                    tokio::spawn(serve_connection(Arc::clone(&shared), conexao, pid));
                }
            }
        }
    }

    pub(super) fn cleanup(path: &Path) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(windows)]
mod sock {
    use super::*;
    use tokio::net::windows::named_pipe::NamedPipeServer;

    pub(super) fn default_path() -> io::Result<PathBuf> {
        let usuario = std::env::var("USERNAME").unwrap_or_else(|_| "usuario".into());
        Ok(PathBuf::from(format!(r"\\.\pipe\tomni-ui-agent-{usuario}")))
    }

    pub(super) struct Listener {
        name: String,
        next: NamedPipeServer,
    }

    pub(super) fn bind(path: &Path, _padrao: bool) -> io::Result<Listener> {
        let name = path.to_str().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "O nome do pipe não é Unicode")
        })?;
        bind_name(name)
    }

    /// O primeiro pipe do nome, do usuário e só dele (e do SYSTEM). Quem já tem o nome faz o
    /// erro ser `AddrInUse`.
    pub(super) fn bind_name(name: &str) -> io::Result<Listener> {
        let next = tomni_ui_win32::private_pipe(name, true).map_err(|e| {
            if e.kind() == io::ErrorKind::PermissionDenied {
                io::Error::new(
                    io::ErrorKind::AddrInUse,
                    format!("Já há um agente em {name}"),
                )
            } else {
                e
            }
        })?;
        Ok(Listener {
            name: name.to_owned(),
            next,
        })
    }

    pub(super) async fn serve(listener: Listener, shared: Arc<Shared>, stop: Arc<Notify>) {
        serve_pipe(listener, shared, stop).await;
    }

    /// Atende o pipe até `stop`. Uma instância nova a cada conexão aceita; o pid de quem
    /// conectou vem do próprio pipe.
    pub(super) async fn serve_pipe(mut listener: Listener, shared: Arc<Shared>, stop: Arc<Notify>) {
        loop {
            tokio::select! {
                biased;
                _ = stop.notified() => return,
                ligou = listener.next.connect() => {
                    if ligou.is_err() { continue }
                    let Ok(proxima) = tomni_ui_win32::private_pipe(&listener.name, false) else {
                        return;
                    };
                    let conexao = std::mem::replace(&mut listener.next, proxima);
                    let pid = tomni_ui_win32::named_pipe_client_pid(&conexao).ok();
                    tokio::spawn(serve_connection(Arc::clone(&shared), conexao, pid));
                }
            }
        }
    }

    pub(super) fn cleanup(_path: &Path) {}
}
