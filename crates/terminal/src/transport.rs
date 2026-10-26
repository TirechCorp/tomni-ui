//! A conexão do terminal com o programa: o [`Transport`], a fila [`Inbox`] que as threads de
//! leitura enchem e o [`Loopback`] dos testes e exemplos.

use std::collections::VecDeque;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// Se a conexão continua aberta, depois de uma leitura.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Status {
    /// Aberta: podem chegar mais bytes.
    #[default]
    Open,
    /// Fechada, com o motivo quando é um erro ("conexão recusada"). Os bytes que chegaram antes
    /// já foram lidos.
    Closed(Option<String>),
}

/// A conexão do terminal com o programa: um processo local, um servidor Telnet ou SSH, uma
/// porta serial ou o que o app escrever (um WebSocket, uma conexão já aberta).
///
/// O terminal chama tudo da thread da interface, então nada aqui pode bloquear: quem lê da
/// rede ou de um processo faz isso numa thread própria, põe os bytes numa [`Inbox`] e ela
/// acorda a interface.
///
/// ```
/// use std::io;
/// use tomni_ui_terminal::{Inbox, Status, Transport};
///
/// /// Um transporte que responde "ok" a cada Enter.
/// struct Ok {
///     inbox: Inbox,
/// }
///
/// impl Transport for Ok {
///     fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
///         if bytes.contains(&b'\r') {
///             self.inbox.push(b"\r\nok\r\n");
///         }
///         Ok(())
///     }
///
///     fn read(&mut self, buf: &mut Vec<u8>, max: usize) -> Status {
///         self.inbox.read(buf, max)
///     }
///
///     fn pending(&self) -> bool {
///         self.inbox.pending()
///     }
/// }
/// ```
pub trait Transport {
    /// Manda `bytes` ao programa: as teclas, o texto colado e as respostas do terminal (a
    /// posição do cursor, o tipo de terminal). Um erro fecha o terminal com a mensagem.
    fn write(&mut self, bytes: &[u8]) -> io::Result<()>;

    /// A grade mudou para `cols` × `rows`: o `Process` avisa o pseudoterminal, o `Telnet`
    /// manda o NAWS e o `Ssh` o `window-change`.
    fn resize(&mut self, cols: u16, rows: u16) {
        let _ = (cols, rows);
    }

    /// Põe no fim de `buf` até `max` bytes que chegaram, sem bloquear.
    fn read(&mut self, buf: &mut Vec<u8>, max: usize) -> Status;

    /// Se há bytes esperando (ou o fechamento ainda não lido). Consultado a cada frame:
    /// precisa ser barato.
    fn pending(&self) -> bool;

    /// Se a conexão ainda está sendo aberta (o SSH autenticando e pedindo o shell, o Telnet
    /// conectando o TCP). Consultado a cada frame, junto do [`Transport::pending`]: precisa ser
    /// barato (um `AtomicBool`). O padrão é `false`: o transporte já nasce pronto para digitar.
    /// Quem passa de `true` a `false` deve acordar a interface com [`tomni_ui_core::wake`].
    fn connecting(&self) -> bool {
        false
    }

    /// Reabre a conexão no lugar, depois de encerrada, com a mesma configuração
    /// ([`Session::restart`](crate::Session::restart)). Devolve `false` se
    /// não sabe reabrir (o padrão), e então a sessão continua encerrada.
    fn reopen(&mut self) -> bool {
        false
    }

    /// Abre uma conexão nova com a mesma configuração, para duplicar a sessão (Alt+Shift+D num
    /// [`TerminalPanes`](crate::TerminalPanes)). `None` se não sabe (o padrão).
    fn duplicate(&self) -> Option<Box<dyn Transport>> {
        None
    }

    /// Como [`Transport::duplicate`], mas pedindo que a cópia comece em `dir`. Só o `Process`
    /// sabe começar numa pasta (se ela existir); os outros transportes fazem o mesmo que o
    /// `duplicate` (o padrão).
    fn duplicate_in(&self, dir: Option<&std::path::Path>) -> Option<Box<dyn Transport>> {
        let _ = dir;
        self.duplicate()
    }

    /// Como [`Transport::duplicate`], mas a cópia roda `command` no lugar do programa da
    /// original (`None`: o shell). Serve para a cópia de uma aba que roda um comando que não
    /// pode ser repetido igual, como o `tmux new-session -A -s nome`. `None` se não sabe (o
    /// padrão); o SSH sabe.
    fn duplicate_running(&self, command: Option<&str>) -> Option<Box<dyn Transport>> {
        let _ = command;
        None
    }

    /// O número do processo local por trás da conexão, para quem quer olhar o sistema
    /// (`/proc/<pid>`). `None` se não há processo local (o padrão: Telnet, SSH, serial).
    fn process_id(&self) -> Option<u32> {
        None
    }

    /// Por onde abrir canais de subsistema (o SFTP) na mesma conexão, sem autenticar de novo.
    /// `None` se a conexão não é SSH (o padrão).
    #[cfg(feature = "ssh")]
    fn ssh_channels(&self) -> Option<crate::ssh::SshChannelOpener> {
        None
    }

    /// Os túneis e o X11 da conexão SSH. `None` se a conexão não é SSH (o padrão).
    #[cfg(feature = "ssh")]
    fn ssh_forwards(&self) -> Option<crate::ssh::SshForwards> {
        None
    }
}

impl<T: Transport + ?Sized> Transport for Box<T> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        (**self).write(bytes)
    }

    fn resize(&mut self, cols: u16, rows: u16) {
        (**self).resize(cols, rows);
    }

    fn read(&mut self, buf: &mut Vec<u8>, max: usize) -> Status {
        (**self).read(buf, max)
    }

    fn pending(&self) -> bool {
        (**self).pending()
    }

    fn connecting(&self) -> bool {
        (**self).connecting()
    }

    fn reopen(&mut self) -> bool {
        (**self).reopen()
    }

    fn duplicate(&self) -> Option<Box<dyn Transport>> {
        (**self).duplicate()
    }

    fn duplicate_in(&self, dir: Option<&std::path::Path>) -> Option<Box<dyn Transport>> {
        (**self).duplicate_in(dir)
    }

    fn duplicate_running(&self, command: Option<&str>) -> Option<Box<dyn Transport>> {
        (**self).duplicate_running(command)
    }

    fn process_id(&self) -> Option<u32> {
        (**self).process_id()
    }

    #[cfg(feature = "ssh")]
    fn ssh_channels(&self) -> Option<crate::ssh::SshChannelOpener> {
        (**self).ssh_channels()
    }

    #[cfg(feature = "ssh")]
    fn ssh_forwards(&self) -> Option<crate::ssh::SshForwards> {
        (**self).ssh_forwards()
    }
}

#[derive(Debug, Default)]
struct Queue {
    bytes: VecDeque<u8>,
    /// `Some` depois do [`Inbox::close`], com o motivo.
    closed: Option<Option<String>>,
}

#[derive(Debug, Default)]
struct Shared {
    queue: Mutex<Queue>,
    pending: AtomicBool,
}

/// Fila de bytes que chegam, de uma thread de leitura para a interface. Clonar dá outra ponta
/// da mesma fila.
///
/// [`Inbox::push`] e [`Inbox::close`] acordam a interface
/// ([`tomni_ui_core::wake`]): o terminal lê no frame seguinte.
#[derive(Clone, Debug, Default)]
pub struct Inbox {
    shared: Arc<Shared>,
}

impl Inbox {
    /// Fila vazia e aberta.
    pub fn new() -> Self {
        Self::default()
    }

    fn queue(&self) -> MutexGuard<'_, Queue> {
        self.shared
            .queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Põe bytes na fila, de qualquer thread.
    pub fn push(&self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.queue().bytes.extend(bytes);
        self.shared.pending.store(true, Ordering::Release);
        tomni_ui_core::wake();
    }

    /// Fecha a fila, com o motivo se foi um erro. Os bytes já postos continuam para ler.
    pub fn close(&self, reason: Option<String>) {
        let mut queue = self.queue();
        if queue.closed.is_none() {
            queue.closed = Some(reason);
        }
        drop(queue);
        self.shared.pending.store(true, Ordering::Release);
        tomni_ui_core::wake();
    }

    /// Abre a fila de novo depois de fechada, para a mesma conexão ser refeita (o
    /// [`Transport::reopen`]). Os bytes que sobraram continuam para ler.
    pub fn reopen(&self) {
        self.queue().closed = None;
    }

    /// Se foi fechada (mesmo com bytes ainda para ler).
    pub fn is_closed(&self) -> bool {
        self.queue().closed.is_some()
    }

    /// Tira até `max` bytes para o fim de `buf`. Dá [`Status::Closed`] quando a fila foi
    /// fechada e não sobrou nada.
    pub fn read(&self, buf: &mut Vec<u8>, max: usize) -> Status {
        let mut queue = self.queue();
        let n = max.min(queue.bytes.len());
        buf.extend(queue.bytes.drain(..n));
        let empty = queue.bytes.is_empty();
        if empty {
            self.shared.pending.store(false, Ordering::Release);
        }
        match &queue.closed {
            Some(reason) if empty => Status::Closed(reason.clone()),
            _ => Status::Open,
        }
    }

    /// Se há bytes esperando ou o fechamento ainda não foi lido.
    pub fn pending(&self) -> bool {
        self.shared.pending.load(Ordering::Acquire)
    }
}

#[derive(Debug, Default)]
struct LoopbackState {
    written: Vec<u8>,
    size: Option<(u16, u16)>,
    echo: bool,
    /// Quantas vezes a conexão foi reaberta.
    reopened: usize,
    /// O comando que a conexão diz rodar ([`Loopback::running`]).
    command: Option<String>,
}

/// Transporte sem programa do outro lado, para testes e exemplos: os bytes chegam por
/// [`LoopbackHandle::feed`] (ou pelo `feed` do `TestApp`), e o que o terminal escreve fica
/// em [`LoopbackHandle::take_written`].
///
/// ```
/// use tomni_ui_terminal::{Loopback, Status, Transport};
///
/// let mut eco = Loopback::new().echo();
/// let handle = eco.handle();
/// eco.write(b"ls\r").unwrap();
/// let mut lido = Vec::new();
/// assert_eq!(eco.read(&mut lido, 1024), Status::Open);
/// assert_eq!(lido, b"ls\r\n");
/// assert_eq!(handle.take_written(), b"ls\r");
/// ```
#[derive(Debug, Default)]
pub struct Loopback {
    inbox: Inbox,
    state: Arc<Mutex<LoopbackState>>,
}

impl Loopback {
    /// Loopback mudo: o que o terminal escreve não volta.
    pub fn new() -> Self {
        Self::default()
    }

    /// Devolve o que o terminal escreve, como um eco local de linha: o Enter vira CR LF e o
    /// Backspace apaga o caractere anterior.
    pub fn echo(self) -> Self {
        self.lock().echo = true;
        self
    }

    /// Anota que a conexão roda `command` no lugar do shell, como um `SshBuilder::command`.
    /// Nada é rodado: o teste lê o comando em [`LoopbackHandle::command`].
    pub fn running(self, command: impl Into<String>) -> Self {
        self.lock().command = Some(command.into());
        self
    }

    fn lock(&self) -> MutexGuard<'_, LoopbackState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A outra ponta, para injetar bytes e ver o que o terminal escreveu. Pode ser clonada e
    /// mandada para outra thread.
    pub fn handle(&self) -> LoopbackHandle {
        LoopbackHandle {
            inbox: self.inbox.clone(),
            state: Arc::clone(&self.state),
        }
    }
}

impl Transport for Loopback {
    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        let mut state = self.lock();
        state.written.extend_from_slice(bytes);
        if state.echo {
            let mut echo = Vec::with_capacity(bytes.len());
            for &b in bytes {
                match b {
                    b'\r' => echo.extend_from_slice(b"\r\n"),
                    0x7f | 0x08 => echo.extend_from_slice(b"\x08 \x08"),
                    _ => echo.push(b),
                }
            }
            drop(state);
            self.inbox.push(&echo);
        }
        Ok(())
    }

    fn resize(&mut self, cols: u16, rows: u16) {
        self.lock().size = Some((cols, rows));
    }

    fn read(&mut self, buf: &mut Vec<u8>, max: usize) -> Status {
        self.inbox.read(buf, max)
    }

    fn pending(&self) -> bool {
        self.inbox.pending()
    }

    fn reopen(&mut self) -> bool {
        self.inbox.reopen();
        self.lock().reopened += 1;
        true
    }

    fn duplicate(&self) -> Option<Box<dyn Transport>> {
        let copy = Loopback::new();
        let state = self.lock();
        let mut copied = copy.lock();
        copied.echo = state.echo;
        copied.command.clone_from(&state.command);
        drop(copied);
        Some(Box::new(copy))
    }

    fn duplicate_running(&self, command: Option<&str>) -> Option<Box<dyn Transport>> {
        let copy = Loopback::new();
        let mut copied = copy.lock();
        copied.echo = self.lock().echo;
        copied.command = command.map(str::to_owned);
        drop(copied);
        Some(Box::new(copy))
    }
}

/// A outra ponta de um [`Loopback`].
#[derive(Clone, Debug)]
pub struct LoopbackHandle {
    inbox: Inbox,
    state: Arc<Mutex<LoopbackState>>,
}

impl LoopbackHandle {
    /// Bytes que chegam ao terminal, como se viessem do programa.
    pub fn feed(&self, bytes: impl AsRef<[u8]>) {
        self.inbox.push(bytes.as_ref());
    }

    /// Fecha a conexão, com o motivo se foi um erro.
    pub fn close(&self, reason: Option<String>) {
        self.inbox.close(reason);
    }

    /// Tira o que o terminal escreveu até agora (teclas, texto colado, respostas).
    pub fn take_written(&self) -> Vec<u8> {
        std::mem::take(&mut self.lock().written)
    }

    /// Quantas vezes a conexão foi reaberta ([`Session::restart`](crate::Session::restart)).
    pub fn reopened(&self) -> usize {
        self.lock().reopened
    }

    /// O comando que a conexão diz rodar ([`Loopback::running`]), que a cópia de
    /// [`Session::duplicate_running`](crate::Session::duplicate_running) troca.
    pub fn command(&self) -> Option<String> {
        self.lock().command.clone()
    }

    /// O último tamanho avisado pelo terminal, em colunas e linhas.
    pub fn size(&self) -> Option<(u16, u16)> {
        self.lock().size
    }

    fn lock(&self) -> MutexGuard<'_, LoopbackState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Mensagem em português para um erro de conexão, mostrada no fim do terminal.
pub(crate) use tomni_ui_net::error_message;

/// Erro do `write` depois que a thread da conexão terminou.
pub(crate) fn closed_error() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "a conexão foi encerrada")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fila_le_em_partes_e_fecha_depois_de_esvaziar() {
        let inbox = Inbox::new();
        assert!(!inbox.pending());
        inbox.push(b"abcdef");
        inbox.close(Some("fim".into()));
        let mut buf = Vec::new();
        assert_eq!(inbox.read(&mut buf, 4), Status::Open);
        assert!(inbox.pending());
        assert_eq!(inbox.read(&mut buf, 4), Status::Closed(Some("fim".into())));
        assert_eq!(buf, b"abcdef");
        assert!(!inbox.pending());
    }

    #[test]
    fn eco_apaga_com_backspace() {
        let mut eco = Loopback::new().echo();
        eco.write(b"ab\x7f").unwrap();
        let mut buf = Vec::new();
        eco.read(&mut buf, 100);
        assert_eq!(buf, b"ab\x08 \x08");
    }

    #[test]
    fn de_outra_thread() {
        let mut lb = Loopback::new();
        let handle = lb.handle();
        std::thread::spawn(move || handle.feed("olá"))
            .join()
            .unwrap();
        let mut buf = Vec::new();
        lb.read(&mut buf, 100);
        assert_eq!(buf, "olá".as_bytes());
        lb.resize(100, 30);
        assert_eq!(lb.handle().size(), Some((100, 30)));
    }
}
