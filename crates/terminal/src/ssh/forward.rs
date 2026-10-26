//! Túneis da conexão SSH: o `-L`, o `-R`, o `-D` (um SOCKS5) e o `-L` para um socket Unix do
//! servidor. Tudo roda na thread da conexão, no runtime que a `session` já tem.

mod socks;

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::io;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};
use std::time::Duration;

use russh::client::{Handle, Msg};
use russh::{Channel, ChannelOpenFailure};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::mpsc::{UnboundedSender, WeakUnboundedSender};
use tokio::task::JoinHandle;

use super::link::ChannelGuard;
use super::x11::{X11State, X11Status};
use super::{CHANNEL_OPEN_TIMEOUT, Client, Command, Notice, Wire, ssh_error};

/// Quanto esperar o destino de um `-R` (nesta máquina) aceitar a conexão.
const LOCAL_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Quanto esperar a saudação e o pedido de um cliente do SOCKS5.
const SOCKS_TIMEOUT: Duration = Duration::from_secs(10);

/// Onde se escuta: um endereço e uma porta (a 0 é escolhida pelo sistema).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bind {
    /// O endereço (`127.0.0.1`, `0.0.0.0`, `localhost`).
    pub address: String,
    /// A porta; 0 deixa o sistema escolher.
    pub port: u16,
}

impl Bind {
    /// `127.0.0.1` na porta `port`.
    pub fn loopback(port: u16) -> Self {
        Self {
            address: "127.0.0.1".to_owned(),
            port,
        }
    }

    fn all_interfaces(port: u16) -> Self {
        Self {
            address: "0.0.0.0".to_owned(),
            port,
        }
    }
}

impl fmt::Display for Bind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.address, self.port)
    }
}

/// Onde o `-L` para um socket Unix do servidor escuta aqui.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocalSocket {
    /// Um socket Unix neste caminho (`0600`; o arquivo é apagado ao parar o túnel). No Windows,
    /// que não tem socket Unix no tokio, vira TCP em `127.0.0.1` numa porta escolhida pelo
    /// sistema, que aparece em [`ForwardStatus::listening`].
    Path(PathBuf),
    /// Uma porta TCP.
    Tcp(Bind),
}

/// Um encaminhamento de porta da conexão SSH.
///
/// ```
/// use tomni_ui_terminal::ssh::Forward;
///
/// // ssh -L 5433:db.interno:5432
/// let banco = Forward::local(5433, "db.interno", 5432);
/// // ssh -D 1080
/// let navegador = Forward::dynamic(1080);
/// // ssh -R 9000:localhost:3000
/// let aberto = Forward::remote(9000, "localhost", 3000);
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Forward {
    /// `-L`: escuta aqui e abre `host:port` a partir do servidor.
    Local {
        /// Onde escutar nesta máquina.
        bind: Bind,
        /// O destino, visto do servidor.
        host: String,
        /// A porta do destino.
        port: u16,
    },
    /// `-R`: o servidor escuta em `bind` e manda as conexões a `host:port` desta máquina.
    Remote {
        /// Onde o servidor escuta.
        bind: Bind,
        /// O destino, visto desta máquina.
        host: String,
        /// A porta do destino.
        port: u16,
    },
    /// `-D`: um SOCKS5 aqui (só CONNECT, sem autenticação); cada pedido abre o destino a partir
    /// do servidor.
    Dynamic {
        /// Onde escutar nesta máquina.
        bind: Bind,
    },
    /// `-L` para um socket Unix do servidor (`direct-streamlocal@openssh.com`).
    Unix {
        /// Onde escutar nesta máquina.
        local: LocalSocket,
        /// O caminho do socket no servidor.
        remote_path: String,
    },
}

impl Forward {
    /// `-L listen_port:host:port`, escutando em `127.0.0.1`.
    pub fn local(listen_port: u16, host: impl Into<String>, port: u16) -> Self {
        Self::Local {
            bind: Bind::loopback(listen_port),
            host: host.into(),
            port,
        }
    }

    /// `-R remote_port:host:port`: o servidor escuta em `localhost` (a porta 0 deixa o
    /// servidor escolher).
    pub fn remote(remote_port: u16, host: impl Into<String>, port: u16) -> Self {
        Self::Remote {
            bind: Bind {
                address: "localhost".to_owned(),
                port: remote_port,
            },
            host: host.into(),
            port,
        }
    }

    /// `-D listen_port`, escutando em `127.0.0.1`.
    pub fn dynamic(listen_port: u16) -> Self {
        Self::Dynamic {
            bind: Bind::loopback(listen_port),
        }
    }

    /// `-L` de `local` para o socket Unix `remote_path` do servidor.
    pub fn unix(local: LocalSocket, remote_path: impl Into<String>) -> Self {
        Self::Unix {
            local,
            remote_path: remote_path.into(),
        }
    }

    /// Escuta em todas as interfaces (`0.0.0.0`) e não só no `127.0.0.1`: outras máquinas da
    /// rede entram no túnel. No [`Forward::Remote`], o pedido ao servidor vai com `0.0.0.0` e
    /// só vale se o `sshd_config` dele tem `GatewayPorts clientspecified` ou `yes`. Num
    /// [`Forward::Unix`] com socket em arquivo, não muda nada.
    pub fn open_to_network(mut self) -> Self {
        match &mut self {
            Self::Local { bind, .. } | Self::Remote { bind, .. } | Self::Dynamic { bind } => {
                *bind = Bind::all_interfaces(bind.port);
            }
            Self::Unix {
                local: LocalSocket::Tcp(bind),
                ..
            } => *bind = Bind::all_interfaces(bind.port),
            Self::Unix { .. } => {}
        }
        self
    }

    /// Uma linha que diz o que o túnel faz (`local 127.0.0.1:8080 → db:5432`).
    pub fn describe(&self) -> String {
        match self {
            Self::Local { bind, host, port } => format!("local {bind} → {host}:{port}"),
            Self::Remote { bind, host, port } => format!("remoto {bind} → {host}:{port}"),
            Self::Dynamic { bind } => format!("SOCKS5 em {bind}"),
            Self::Unix { local, remote_path } => match local {
                LocalSocket::Path(path) => format!("socket {} → {remote_path}", path.display()),
                LocalSocket::Tcp(bind) => format!("local {bind} → socket {remote_path}"),
            },
        }
    }

    /// "o túnel local para db:5432", para as mensagens de erro.
    fn what(&self) -> String {
        match self {
            Self::Local { host, port, .. } => format!("o túnel local para {host}:{port}"),
            Self::Remote { bind, .. } => format!("o túnel remoto da porta {}", bind.port),
            Self::Dynamic { .. } => "o túnel SOCKS5".to_owned(),
            Self::Unix { remote_path, .. } => format!("o túnel do socket {remote_path}"),
        }
    }
}

/// O número de um túnel, estável na conexão (um [`SshForwards::restart`] mantém o número).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ForwardId(u64);

impl ForwardId {
    /// O número como inteiro, para guardar em tabelas.
    pub fn get(self) -> u64 {
        self.0
    }
}

/// Em que pé o túnel está.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ForwardState {
    /// Ainda subindo (a conexão não terminou de abrir).
    Starting,
    /// Escutando.
    Listening,
    /// Parado por [`SshForwards::stop`] ou pelo fim da conexão.
    Stopped,
    /// Não subiu; o motivo já vem escrito para a pessoa.
    Failed(String),
}

/// O estado de um túnel, para a tela.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForwardStatus {
    /// O número do túnel.
    pub id: ForwardId,
    /// O que ele faz.
    pub forward: Forward,
    /// Onde escuta de verdade (`127.0.0.1:8080`, a porta que o sistema escolheu para a 0, o
    /// caminho do socket); `None` se não escuta.
    pub listening: Option<String>,
    /// O estado.
    pub state: ForwardState,
    /// Quantas conexões estão abertas agora.
    pub active: usize,
    /// Quantas conexões já passaram.
    pub total: u64,
    /// Bytes daqui para o servidor.
    pub sent: u64,
    /// Bytes do servidor para cá.
    pub received: u64,
    /// O último canal que o servidor não abriu, com o motivo.
    pub last_error: Option<String>,
}

#[derive(Debug)]
struct Info {
    state: ForwardState,
    listening: Option<String>,
    last_error: Option<String>,
    /// No `-R` que subiu: o endereço e a porta que o servidor escuta.
    remote: Option<(String, u32)>,
}

#[derive(Debug)]
pub(super) struct Stats {
    info: Mutex<Info>,
    active: AtomicUsize,
    total: AtomicU64,
    sent: AtomicU64,
    received: AtomicU64,
}

impl Stats {
    fn new() -> Self {
        Self {
            info: Mutex::new(Info {
                state: ForwardState::Starting,
                listening: None,
                last_error: None,
                remote: None,
            }),
            active: AtomicUsize::new(0),
            total: AtomicU64::new(0),
            sent: AtomicU64::new(0),
            received: AtomicU64::new(0),
        }
    }

    fn info(&self) -> std::sync::MutexGuard<'_, Info> {
        self.info.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn set_state(&self, state: ForwardState) {
        self.info().state = state;
    }

    fn state(&self) -> ForwardState {
        self.info().state.clone()
    }

    fn fail(&self, message: &str) {
        let mut info = self.info();
        info.state = ForwardState::Failed(message.to_owned());
        info.listening = None;
        info.remote = None;
    }

    fn last_error(&self, message: String) {
        self.info().last_error = Some(message);
    }
}

/// Uma conexão do túnel aberta: conta enquanto existir.
struct Active(Arc<Stats>);

impl Active {
    fn new(stats: &Arc<Stats>) -> Self {
        stats.active.fetch_add(1, Ordering::AcqRel);
        stats.total.fetch_add(1, Ordering::AcqRel);
        Self(Arc::clone(stats))
    }
}

impl Drop for Active {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Debug)]
struct Entry {
    id: ForwardId,
    forward: Forward,
    stats: Arc<Stats>,
}

/// Os túneis e o X11 de uma conexão, lidos da tela sem passar pela thread da conexão.
#[derive(Debug, Default)]
pub(super) struct ForwardTable {
    entries: Mutex<Vec<Entry>>,
    next: AtomicU64,
    x11: Mutex<Option<Arc<X11State>>>,
}

impl ForwardTable {
    pub(super) fn new(forwards: &[Forward]) -> Arc<Self> {
        let table = Arc::new(Self::default());
        for forward in forwards {
            table.add(forward.clone());
        }
        table
    }

    fn entries(&self) -> std::sync::MutexGuard<'_, Vec<Entry>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn add(&self, forward: Forward) -> ForwardId {
        let id = ForwardId(self.next.fetch_add(1, Ordering::AcqRel));
        self.entries().push(Entry {
            id,
            forward,
            stats: Arc::new(Stats::new()),
        });
        id
    }

    pub(super) fn set_x11(&self, state: Option<Arc<X11State>>) {
        *self.x11.lock().unwrap_or_else(PoisonError::into_inner) = state;
    }

    fn x11(&self) -> Option<Arc<X11State>> {
        self.x11
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn get(&self, id: ForwardId) -> Option<(Forward, Arc<Stats>)> {
        self.entries()
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| (entry.forward.clone(), Arc::clone(&entry.stats)))
    }

    fn ids(&self) -> Vec<ForwardId> {
        self.entries().iter().map(|entry| entry.id).collect()
    }

    /// Quantas conexões de túnel e canais X11 estão abertos agora.
    pub(super) fn busy(&self) -> usize {
        let tunnels: usize = self
            .entries()
            .iter()
            .map(|entry| entry.stats.active.load(Ordering::Acquire))
            .sum();
        tunnels + self.x11().map_or(0, |x11| x11.active())
    }

    /// O `-R` que o servidor escuta em `port`, vindo em `address`.
    fn find_remote(&self, address: &str, port: u32) -> Option<(Forward, Arc<Stats>)> {
        let entries = self.entries();
        let mut found = None;
        for entry in entries.iter() {
            let info = entry.stats.info();
            if let Some((bound, bound_port)) = &info.remote
                && *bound_port == port
            {
                let candidate = (entry.forward.clone(), Arc::clone(&entry.stats));
                if bound == address {
                    return Some(candidate);
                }
                found.get_or_insert(candidate);
            }
        }
        found
    }

    fn status(&self, entry: &Entry) -> ForwardStatus {
        let info = entry.stats.info();
        ForwardStatus {
            id: entry.id,
            forward: entry.forward.clone(),
            listening: info.listening.clone(),
            state: info.state.clone(),
            active: entry.stats.active.load(Ordering::Acquire),
            total: entry.stats.total.load(Ordering::Acquire),
            sent: entry.stats.sent.load(Ordering::Acquire),
            received: entry.stats.received.load(Ordering::Acquire),
            last_error: info.last_error.clone(),
        }
    }

    /// Marca como parados os que escutavam, no fim da conexão.
    pub(super) fn stop_all(&self) {
        for entry in self.entries().iter() {
            let mut info = entry.stats.info();
            if matches!(info.state, ForwardState::Listening | ForwardState::Starting) {
                info.state = ForwardState::Stopped;
                info.listening = None;
                info.remote = None;
            }
        }
    }
}

/// Os túneis e o X11 da conexão SSH de uma sessão: o estado, ao vivo, e os controles. Vem de
/// [`Session::ssh_forwards`](crate::Session::ssh_forwards); é `Clone + Send + Sync`.
///
/// Depois de um [`restart`](crate::Session::restart) da sessão, peça outro: o antigo fala com a
/// conexão que acabou.
#[derive(Clone)]
pub struct SshForwards {
    table: Arc<ForwardTable>,
    commands: UnboundedSender<Command>,
}

impl fmt::Debug for SshForwards {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SshForwards").finish_non_exhaustive()
    }
}

impl SshForwards {
    pub(super) fn new(table: Arc<ForwardTable>, commands: UnboundedSender<Command>) -> Self {
        Self { table, commands }
    }

    /// Os túneis, na ordem em que foram pedidos, com o estado e os contadores de agora.
    pub fn list(&self) -> Vec<ForwardStatus> {
        let entries = self.table.entries();
        entries
            .iter()
            .map(|entry| self.table.status(entry))
            .collect()
    }

    /// O X11 da conexão; `None` se não foi pedido ou ficou desligado (o terminal diz por quê).
    pub fn x11(&self) -> Option<X11Status> {
        self.table.x11().map(|state| state.status())
    }

    /// Sobe um túnel na conexão aberta (ou o enfileira, se ela ainda está abrindo). Volta na
    /// hora; o resultado aparece no [`list`](Self::list).
    pub fn add(&self, forward: Forward) -> ForwardId {
        let id = self.table.add(forward);
        let _ = self
            .commands
            .send(Command::Forward(ForwardCommand::Start(id)));
        id
    }

    /// Para de escutar. As conexões que já passam pelo túnel seguem até acabar.
    pub fn stop(&self, id: ForwardId) {
        let _ = self
            .commands
            .send(Command::Forward(ForwardCommand::Stop(id)));
    }

    /// Tenta subir de novo o túnel parado ou que falhou (a porta que liberou, por exemplo).
    pub fn restart(&self, id: ForwardId) {
        let _ = self
            .commands
            .send(Command::Forward(ForwardCommand::Restart(id)));
    }
}

/// Um pedido da tela ao laço da sessão.
#[derive(Debug)]
pub(super) enum ForwardCommand {
    Start(ForwardId),
    Stop(ForwardId),
    Restart(ForwardId),
}

/// O que a conexão oferece aos túneis, ao X11 e aos canais que o servidor abre: só o servidor
/// de destino tem (os saltos recusam).
pub(super) struct Live {
    pub(super) x11: Option<Arc<X11State>>,
    pub(super) table: Arc<ForwardTable>,
    /// Os canais que seguram a conexão depois do fim do shell (o SFTP, os túneis, o X11).
    pub(super) extras: Arc<AtomicUsize>,
    pub(super) commands: WeakUnboundedSender<Command>,
    pub(super) notice: Notice,
}

impl Live {
    /// Conta mais um canal que segura a conexão, até soltar o guarda.
    pub(super) fn guard(&self) -> ChannelGuard {
        self.extras.fetch_add(1, Ordering::AcqRel);
        ChannelGuard {
            extras: Arc::clone(&self.extras),
            commands: self.commands.clone(),
        }
    }
}

/// Soma os bytes que passam por `inner` nos contadores do túnel: o que se lê dele vai ao
/// servidor (`sent`), o que se escreve nele veio do servidor (`received`).
struct Counted<S> {
    inner: S,
    stats: Arc<Stats>,
}

impl<S: AsyncRead + Unpin> AsyncRead for Counted<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let result = Pin::new(&mut self.inner).poll_read(cx, buf);
        if let Poll::Ready(Ok(())) = &result {
            let read = buf.filled().len() - before;
            self.stats.sent.fetch_add(read as u64, Ordering::AcqRel);
        }
        result
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Counted<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let result = Pin::new(&mut self.inner).poll_write(cx, buf);
        if let Poll::Ready(Ok(written)) = &result {
            self.stats
                .received
                .fetch_add(*written as u64, Ordering::AcqRel);
        }
        result
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// Liga `local` ao canal, contando os bytes.
async fn bridge(local: Box<dyn Wire>, channel: Channel<Msg>, stats: &Arc<Stats>) {
    let mut local = Counted {
        inner: local,
        stats: Arc::clone(stats),
    };
    let mut remote = channel.into_stream();
    let _ = tokio::io::copy_bidirectional(&mut local, &mut remote).await;
}

// ----- quem escuta -----

enum Listener {
    Tcp(tokio::net::TcpListener),
    #[cfg(unix)]
    Unix(tokio::net::UnixListener, PathBuf),
}

impl Listener {
    /// A próxima conexão, com a origem (o endereço e a porta, para o `direct-tcpip`).
    async fn accept(&self) -> io::Result<(Box<dyn Wire>, String, u32)> {
        match self {
            Self::Tcp(listener) => {
                let (stream, peer) = listener.accept().await?;
                let _ = stream.set_nodelay(true);
                Ok((
                    Box::new(stream),
                    peer.ip().to_string(),
                    u32::from(peer.port()),
                ))
            }
            #[cfg(unix)]
            Self::Unix(listener, _) => {
                let (stream, _) = listener.accept().await?;
                Ok((Box::new(stream), "127.0.0.1".to_owned(), 0))
            }
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Self::Unix(_, path) = self {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// A mensagem de uma porta que não deu para escutar.
fn bind_error(forward: &Forward, address: &str, error: &io::Error) -> String {
    let what = forward.what();
    match error.kind() {
        io::ErrorKind::AddrInUse => {
            format!("A porta {address} já está em uso por outro programa; {what} não subiu.")
        }
        io::ErrorKind::PermissionDenied => {
            format!("Sem permissão para escutar em {address}; {what} não subiu.")
        }
        _ => format!(
            "Não foi possível escutar em {address} ({}); {what} não subiu.",
            crate::transport::error_message(error)
        ),
    }
}

/// Escuta onde o túnel pede. Devolve o ouvinte e o texto de onde ele escuta de verdade.
async fn bind(forward: &Forward) -> Result<(Listener, String), String> {
    let tcp = |bind: &Bind| {
        let bind = bind.clone();
        async move {
            let address = bind.to_string();
            let listener = tokio::net::TcpListener::bind((bind.address.as_str(), bind.port))
                .await
                .map_err(|error| bind_error(forward, &address, &error))?;
            let real = listener
                .local_addr()
                .map_or(address, |address| address.to_string());
            Ok::<_, String>((Listener::Tcp(listener), real))
        }
    };
    match forward {
        Forward::Local { bind, .. } | Forward::Dynamic { bind } => tcp(bind).await,
        Forward::Unix {
            local: LocalSocket::Tcp(bind),
            ..
        } => tcp(bind).await,
        #[cfg(unix)]
        Forward::Unix {
            local: LocalSocket::Path(path),
            ..
        } => bind_unix(forward, path).await,
        #[cfg(not(unix))]
        Forward::Unix {
            local: LocalSocket::Path(_),
            ..
        } => tcp(&Bind::loopback(0)).await,
        Forward::Remote { .. } => Err("um túnel remoto não escuta aqui".to_owned()),
    }
}

#[cfg(unix)]
async fn bind_unix(forward: &Forward, path: &PathBuf) -> Result<(Listener, String), String> {
    use std::os::unix::fs::PermissionsExt;
    let shown = path.display().to_string();
    // Um arquivo que sobrou de outra execução: se ninguém atende nele, apaga; se atende, está em
    // uso.
    if path.exists() {
        if tokio::net::UnixStream::connect(path).await.is_ok() {
            return Err(format!(
                "O socket {shown} já está em uso por outro programa; {} não subiu.",
                forward.what()
            ));
        }
        let _ = std::fs::remove_file(path);
    }
    let listener = tokio::net::UnixListener::bind(path)
        .map_err(|error| bind_error(forward, &shown, &error))?;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    Ok((Listener::Unix(listener, path.clone()), shown))
}

// ----- os canais -----

/// A mensagem de um canal que o servidor não abriu.
pub(super) fn open_error(error: &russh::Error, destination: &str) -> String {
    let reason = match error {
        russh::Error::ChannelOpenFailure(ChannelOpenFailure::AdministrativelyProhibited) => {
            "o servidor proíbe túneis (AllowTcpForwarding)".to_owned()
        }
        russh::Error::ChannelOpenFailure(ChannelOpenFailure::ConnectFailed) => {
            "conexão recusada".to_owned()
        }
        other => ssh_error(other),
    };
    format!("O servidor não abriu {destination}: {reason}")
}

/// Espera o servidor abrir o canal, com o tempo limite.
async fn open_channel(
    open: impl Future<Output = Result<Channel<Msg>, russh::Error>>,
    destination: &str,
) -> Result<Channel<Msg>, (String, u8)> {
    match tokio::time::timeout(CHANNEL_OPEN_TIMEOUT, open).await {
        Err(_) => Err((
            format!("O servidor não abriu {destination}: não respondeu ao pedido do canal"),
            socks::HOST_UNREACHABLE,
        )),
        Ok(Err(error)) => {
            let code = match error {
                russh::Error::ChannelOpenFailure(_) => socks::REFUSED,
                _ => socks::GENERAL_FAILURE,
            };
            Err((open_error(&error, destination), code))
        }
        Ok(Ok(channel)) => Ok(channel),
    }
}

/// Uma conexão que chegou a um túnel local: abre o canal e liga os dois lados.
async fn connection(
    mut local: Box<dyn Wire>,
    origin: (String, u32),
    forward: Forward,
    handle: Arc<Handle<Client>>,
    stats: Arc<Stats>,
    live: Arc<Live>,
) {
    let _active = Active::new(&stats);
    let _guard = live.guard();
    let channel = match &forward {
        Forward::Local { host, port, .. } => {
            let destination = format!("{host}:{port}");
            let open = handle.channel_open_direct_tcpip(
                host.clone(),
                u32::from(*port),
                origin.0,
                origin.1,
            );
            open_channel(open, &destination).await
        }
        Forward::Unix { remote_path, .. } => {
            let open = handle.channel_open_direct_streamlocal(remote_path.clone());
            open_channel(open, remote_path).await
        }
        Forward::Dynamic { .. } => {
            let target = tokio::time::timeout(SOCKS_TIMEOUT, socks::handshake(&mut local)).await;
            let Ok(Ok((host, port))) = target else {
                return;
            };
            let destination = format!("{host}:{port}");
            let open = handle.channel_open_direct_tcpip(host, u32::from(port), origin.0, origin.1);
            let opened = open_channel(open, &destination).await;
            socks::reply(
                &mut local,
                match &opened {
                    Ok(_) => socks::SUCCEEDED,
                    Err((_, code)) => *code,
                },
            )
            .await;
            opened
        }
        Forward::Remote { .. } => return,
    };
    // Daqui em diante o `Handle` não faz falta: quem abriu o canal solta a sua parte.
    drop(handle);
    match channel {
        Ok(channel) => bridge(local, channel, &stats).await,
        Err((message, _)) => stats.last_error(message),
    }
}

/// O laço de um túnel local: aceita e entrega cada conexão a uma tarefa.
async fn serve(
    listener: Listener,
    forward: Forward,
    handle: Arc<Handle<Client>>,
    stats: Arc<Stats>,
    live: Arc<Live>,
) {
    loop {
        match listener.accept().await {
            Ok((stream, address, port)) => {
                tokio::spawn(connection(
                    stream,
                    (address, port),
                    forward.clone(),
                    Arc::clone(&handle),
                    Arc::clone(&stats),
                    Arc::clone(&live),
                ));
            }
            // Um erro passageiro (descritores demais): respira e tenta de novo.
            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
}

/// O servidor abriu um `forwarded-tcpip`: liga ao destino do `-R` correspondente, nesta
/// máquina. Roda numa tarefa própria.
pub(super) async fn serve_remote(
    live: Arc<Live>,
    channel: Channel<Msg>,
    reply: russh::client::ChannelOpenHandle,
    address: String,
    port: u32,
) {
    let Some((
        Forward::Remote {
            host,
            port: local_port,
            ..
        },
        stats,
    )) = live.table.find_remote(&address, port)
    else {
        return;
    };
    let _guard = live.guard();
    let connect = tokio::net::TcpStream::connect((host.as_str(), local_port));
    let local = match tokio::time::timeout(LOCAL_CONNECT_TIMEOUT, connect).await {
        Ok(Ok(stream)) => stream,
        Ok(Err(error)) => {
            stats.last_error(format!(
                "Não foi possível abrir {host}:{local_port} nesta máquina: {}",
                crate::transport::error_message(&error)
            ));
            return;
        }
        Err(_) => {
            stats.last_error(format!(
                "Não foi possível abrir {host}:{local_port} nesta máquina: tempo esgotado"
            ));
            return;
        }
    };
    let _ = local.set_nodelay(true);
    reply.accept().await;
    let _active = Active::new(&stats);
    bridge(Box::new(local), channel, &stats).await;
}

// ----- quem sobe, para e reinicia -----

/// Os túneis da sessão: sobe, para e reinicia, e guarda a tarefa de cada ouvinte.
pub(super) struct Tunnels {
    handle: Arc<Handle<Client>>,
    live: Arc<Live>,
    tasks: HashMap<ForwardId, JoinHandle<()>>,
}

impl Tunnels {
    pub(super) fn new(handle: Arc<Handle<Client>>, live: Arc<Live>) -> Self {
        Self {
            handle,
            live,
            tasks: HashMap::new(),
        }
    }

    /// Sobe os túneis do começo da conexão: os remotos primeiro, depois os locais. Com
    /// `exit_on_failure`, o primeiro que não sobe devolve o erro.
    pub(super) async fn start_all(&mut self, exit_on_failure: bool) -> Result<(), String> {
        let mut ids = self.live.table.ids();
        ids.sort_by_key(|id| {
            let remote = matches!(self.live.table.get(*id), Some((Forward::Remote { .. }, _)));
            (!remote, *id)
        });
        let mut first_error = None;
        for id in ids {
            if let Err(message) = self.start(id).await
                && exit_on_failure
                && first_error.is_none()
            {
                first_error = Some(message);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Um pedido da tela.
    pub(super) async fn command(&mut self, command: ForwardCommand) {
        match command {
            ForwardCommand::Start(id) => {
                let waiting = self
                    .live
                    .table
                    .get(id)
                    .is_some_and(|(_, stats)| stats.state() == ForwardState::Starting);
                if waiting && !self.tasks.contains_key(&id) {
                    let _ = self.start(id).await;
                }
            }
            ForwardCommand::Stop(id) => self.stop(id).await,
            ForwardCommand::Restart(id) => {
                self.stop(id).await;
                let _ = self.start(id).await;
            }
        }
    }

    fn notice(&self, line: &str) {
        (self.live.notice)(line);
    }

    /// Sobe o túnel `id`. O erro já vem escrito, e já foi mostrado no terminal.
    async fn start(&mut self, id: ForwardId) -> Result<(), String> {
        let Some((forward, stats)) = self.live.table.get(id) else {
            return Ok(());
        };
        if self.tasks.contains_key(&id) || stats.state() == ForwardState::Listening {
            return Ok(());
        }
        stats.set_state(ForwardState::Starting);
        let result = match &forward {
            Forward::Remote { bind, .. } => self.start_remote(&forward, bind, &stats).await,
            _ => self.start_local(id, &forward, &stats).await,
        };
        match result {
            Ok(line) => {
                self.notice(&line);
                Ok(())
            }
            Err(message) => {
                stats.fail(&message);
                self.notice(&message);
                Err(message)
            }
        }
    }

    async fn start_remote(
        &self,
        forward: &Forward,
        bind: &Bind,
        stats: &Arc<Stats>,
    ) -> Result<String, String> {
        let asked = self
            .handle
            .tcpip_forward(bind.address.clone(), u32::from(bind.port))
            .await
            .map_err(|_| {
                format!(
                    "O servidor não abriu a porta {} (o sshd_config tem AllowTcpForwarding no, ou a porta está em uso lá).",
                    bind.port
                )
            })?;
        let port = if bind.port == 0 {
            asked
        } else {
            u32::from(bind.port)
        };
        let mut info = stats.info();
        info.state = ForwardState::Listening;
        info.listening = Some(format!("{}:{port}", bind.address));
        info.remote = Some((bind.address.clone(), port));
        drop(info);
        let Forward::Remote {
            host, port: local, ..
        } = forward
        else {
            return Ok(String::new());
        };
        Ok(format!(
            "Túnel remoto {}:{port} → {host}:{local}",
            bind.address
        ))
    }

    async fn start_local(
        &mut self,
        id: ForwardId,
        forward: &Forward,
        stats: &Arc<Stats>,
    ) -> Result<String, String> {
        let (listener, listening) = bind(forward).await?;
        let line = match forward {
            Forward::Local { host, port, .. } => {
                format!("Túnel local {listening} → {host}:{port}")
            }
            Forward::Dynamic { .. } => format!("SOCKS5 em {listening}"),
            Forward::Unix { remote_path, .. } => {
                format!("Túnel do socket {remote_path} em {listening}")
            }
            Forward::Remote { .. } => String::new(),
        };
        {
            let mut info = stats.info();
            info.state = ForwardState::Listening;
            info.listening = Some(listening);
        }
        let task = tokio::spawn(serve(
            listener,
            forward.clone(),
            Arc::clone(&self.handle),
            Arc::clone(stats),
            Arc::clone(&self.live),
        ));
        self.tasks.insert(id, task);
        Ok(line)
    }

    /// Para de escutar o túnel `id`; as conexões abertas seguem.
    async fn stop(&mut self, id: ForwardId) {
        if let Some(task) = self.tasks.remove(&id) {
            task.abort();
        }
        let Some((forward, stats)) = self.live.table.get(id) else {
            return;
        };
        let remote = stats.info().remote.take();
        if let (Forward::Remote { .. }, Some((address, port))) = (&forward, remote) {
            let _ = self.handle.cancel_tcpip_forward(address, port).await;
        }
        let mut info = stats.info();
        if info.state != ForwardState::Stopped {
            info.state = ForwardState::Stopped;
        }
        info.listening = None;
    }

    /// Fim da conexão: para os ouvintes e solta o `Handle`.
    pub(super) fn shutdown(self) {
        drop(self);
    }
}

impl Drop for Tunnels {
    fn drop(&mut self) {
        for task in self.tasks.values() {
            task.abort();
        }
        self.live.table.stop_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_construtores_escutam_so_no_loopback() {
        assert_eq!(
            Forward::local(8080, "db", 5432),
            Forward::Local {
                bind: Bind::loopback(8080),
                host: "db".into(),
                port: 5432
            }
        );
        assert_eq!(
            Forward::dynamic(1080),
            Forward::Dynamic {
                bind: Bind::loopback(1080)
            }
        );
        assert_eq!(
            Forward::local(1, "h", 2).open_to_network(),
            Forward::Local {
                bind: Bind::all_interfaces(1),
                host: "h".into(),
                port: 2
            }
        );
    }

    #[test]
    fn o_remoto_pede_localhost_ou_todas_as_interfaces() {
        let Forward::Remote { bind, .. } = Forward::remote(9000, "localhost", 3000) else {
            unreachable!()
        };
        assert_eq!(bind.address, "localhost");
        let Forward::Remote { bind, .. } =
            Forward::remote(9000, "localhost", 3000).open_to_network()
        else {
            unreachable!()
        };
        assert_eq!(bind, Bind::all_interfaces(9000));
    }

    #[test]
    fn a_descricao_diz_o_que_o_tunel_faz() {
        assert_eq!(
            Forward::local(8080, "db", 5432).describe(),
            "local 127.0.0.1:8080 → db:5432"
        );
        assert_eq!(
            Forward::dynamic(1080).describe(),
            "SOCKS5 em 127.0.0.1:1080"
        );
    }

    #[test]
    fn a_tabela_numera_e_acha_o_remoto_pela_porta() {
        let tabela = ForwardTable::new(&[
            Forward::local(1, "a", 2),
            Forward::remote(0, "localhost", 3),
        ]);
        let ids = tabela.ids();
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1]);
        assert!(tabela.find_remote("localhost", 4000).is_none());
        let (_, stats) = tabela.get(ids[1]).unwrap();
        stats.info().remote = Some(("localhost".to_owned(), 4000));
        assert!(tabela.find_remote("outro", 4000).is_some());
        assert!(tabela.find_remote("localhost", 4001).is_none());
    }
}
