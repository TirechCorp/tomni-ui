//! SSH (RFC 4253 e 4254) pelo `russh`: uma sessão com pseudoterminal e shell, com salto por outros servidores (`ProxyJump`), proxy, agente de chaves e o
//! encaminhamento dele.

pub mod agent;
mod copy_id;
mod exec;
mod forward;
pub mod keys;
mod launch;
mod link;
mod pasta;
mod x11;

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use russh::client::{self, Handle};
use russh::keys::agent::AgentIdentity;
use russh::keys::agent::client::{AgentClient, AgentStream};
use russh::keys::{
    HashAlg, PrivateKey, PrivateKeyWithHashAlg, PublicKey, PublicKeyOrCertificate,
    check_known_hosts_path, known_hosts::learn_known_hosts_path, load_secret_key,
};
use russh::{Channel, ChannelMsg, Disconnect, Sig};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender, WeakUnboundedSender};

use tomni_ui_net::{CONNECT_TIMEOUT, connect_error_message, connect_tcp};

use crate::transport::{Inbox, Status, Transport, closed_error, error_message};

pub use copy_id::{CopyIdOptions, CopyIdOutcome, copy_id};
pub use exec::ExecOutput;
pub use forward::{
    Bind, Forward, ForwardId, ForwardState, ForwardStatus, LocalSocket, SshForwards,
};
use forward::{ForwardCommand, ForwardTable, Live, Tunnels};
pub use launch::{RemoteApp, RemoteAppStatus};
use link::{ChannelGuard, ExecChannel};
pub use link::{SshChannel, SshChannelOpener, SshLink};
pub use tomni_ui_net::Proxy;
use x11::X11State;
pub use x11::{X11, X11Mode, X11Status};

/// O `TERM` pedido por padrão no pseudoterminal remoto, o mesmo do `Process`.
const TERM: &str = "xterm-256color";
/// De quanto em quanto tempo o cliente confere se o servidor ainda responde.
const KEEPALIVE: Duration = Duration::from_secs(30);
/// Quanto se espera pelo motivo do fim da conexão, depois que o canal fechou sem aviso.
const END_REASON_WAIT: Duration = Duration::from_millis(200);

/// Uma sessão SSH, aberta numa thread: conecta, confere a chave do servidor, autentica, pede
/// um pseudoterminal e o shell. Um erro ("autenticação recusada para ana") aparece no fim da
/// tela, como os do `Telnet`.
///
/// A chave do servidor é conferida no `known_hosts` (o `~/.ssh/known_hosts`, ou o arquivo de
/// [`SshBuilder::known_hosts`]). Sem [`SshBuilder::check_host_key`], só uma chave já conhecida
/// passa, como o `StrictHostKeyChecking` do OpenSSH. Para perguntar à pessoa, o app usa o
/// `check_host_key` e grava a resposta com [`HostKey::remember`].
///
/// A autenticação tenta as chaves, na ordem do OpenSSH: primeiro a cópia no agente
/// ([`SshBuilder::agent_auth`]) das chaves da conexão ([`SshBuilder::key_file`] e
/// [`SshBuilder::key`]), depois as próprias chaves, as outras do agente e por fim a senha. Assim a
/// chave certa chega antes do limite de tentativas do servidor (o `MaxAuthTries`), mesmo com
/// muitas chaves no agente. A senha não é pedida no terminal: o app pede antes, num
/// `PasswordBox`.
///
/// Uma sessão encerrada reabre pelo [`Session::restart`](crate::Session::restart), e o
/// Alt+Shift+D de um [`TerminalPanes`](crate::TerminalPanes) abre uma conexão nova com a mesma
/// configuração.
///
/// ```no_run
/// use tomni_ui_terminal::{KnownHost, Ssh, Terminal};
///
/// let servidor = Ssh::builder("servidor.exemplo", 22, "ana")
///     .key_file("/home/ana/.ssh/id_ed25519", None)
///     .password("senha da ana")
///     .check_host_key(|chave, conhecida| match conhecida {
///         KnownHost::Known => true,
///         // Aqui o app perguntaria à pessoa, mostrando `chave.fingerprint()`.
///         KnownHost::Unknown => chave.remember().is_ok(),
///         KnownHost::Changed { .. } => false,
///     })
///     .connect();
/// let console = Terminal::new(servidor).id("console");
/// ```
///
/// Chegando por um servidor de salto (o `ProxyJump` do OpenSSH), com o agente:
///
/// ```no_run
/// use tomni_ui_terminal::Ssh;
///
/// let salto = Ssh::builder("bastiao.exemplo", 22, "ana").agent_auth();
/// let interno = Ssh::builder("interno.exemplo", 22, "ana")
///     .agent_auth()
///     .agent_forward(true)
///     .jump(salto)
///     .connect();
/// ```
#[derive(Debug)]
pub struct Ssh {
    inbox: Inbox,
    commands: UnboundedSender<Command>,
    size: Arc<Mutex<(u16, u16)>>,
    /// Os túneis e o X11 desta conexão (outra tabela a cada `reopen`).
    forwards: Arc<ForwardTable>,
    /// Como a conexão foi pedida, para reabrir e duplicar.
    builder: SshBuilder,
    /// Verdadeiro até o shell (ou o `exec`) abrir; uma flag nova a cada `reopen`.
    connecting: Arc<AtomicBool>,
}

/// O que a thread da conexão avisa à interface sem passar pela fila de bytes.
struct Flags {
    /// O `session` fecha a fila antes do fim quando o shell sai e ainda há canais extras;
    /// fechar de novo na thread, depois de um `reopen`, fecharia a conexão nova.
    closed: AtomicBool,
    /// Ver [`Transport::connecting`].
    connecting: Arc<AtomicBool>,
}

impl Flags {
    /// A conexão ficou pronta (ou acabou): acorda a interface para a aba perder o anel.
    fn ready(&self) {
        if self.connecting.swap(false, Ordering::AcqRel) {
            tomni_ui_core::wake();
        }
    }
}

#[derive(Debug)]
enum Command {
    Data(Vec<u8>),
    Resize(u16, u16),
    Close,
    /// Abre um canal com o subsistema `name` na conexão do terminal (o SFTP sobre a sessão).
    OpenSubsystem {
        name: String,
        reply: SyncSender<Result<SshChannel, String>>,
    },
    /// Roda `command` num canal `exec` novo da conexão do terminal (o `SshChannelOpener::exec`).
    OpenExec {
        command: String,
        reply: SyncSender<Result<ExecChannel, String>>,
    },
    /// Abre um canal `direct-tcpip` avulso até `host:port` (o `SshChannelOpener::open_direct`).
    OpenDirect {
        host: String,
        port: u16,
        reply: SyncSender<Result<SshChannel, String>>,
    },
    /// Um canal aberto por `OpenSubsystem`, `OpenExec` ou `OpenDirect` terminou: a conexão pode acabar se o
    /// shell já saiu.
    ChannelClosed,
    /// Um pedido da tela aos túneis ([`SshForwards`]).
    Forward(ForwardCommand),
}

/// Onde a conexão escreve um aviso no terminal (o fim da tela, como os do `before_connect`).
type Notice = Arc<dyn Fn(&str) + Send + Sync>;

/// Quanto esperar o servidor aceitar o canal de um subsistema.
const CHANNEL_OPEN_TIMEOUT: Duration = Duration::from_secs(20);

/// A chave que o servidor apresentou, para o app conferir em [`SshBuilder::check_host_key`].
#[derive(Clone, Debug)]
pub struct HostKey {
    host: String,
    port: u16,
    key: PublicKey,
    known_hosts: PathBuf,
}

impl HostKey {
    /// O servidor, como foi pedido em [`Ssh::builder`].
    pub fn host(&self) -> &str {
        &self.host
    }

    /// A porta.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// O algoritmo da chave (`ssh-ed25519`, `ecdsa-sha2-nistp256`, `ssh-rsa`).
    pub fn algorithm(&self) -> String {
        self.key.algorithm().as_str().to_owned()
    }

    /// A impressão digital SHA-256, no formato do OpenSSH e do PuTTY (`SHA256:…`).
    pub fn fingerprint(&self) -> String {
        self.key.fingerprint(HashAlg::Sha256).to_string()
    }

    /// A chave inteira no formato do OpenSSH (`ssh-ed25519 AAAA…`).
    pub fn openssh(&self) -> String {
        self.key.to_openssh().unwrap_or_default()
    }

    /// Grava a chave no `known_hosts` (o mesmo que foi conferido), para a próxima conexão já a
    /// conhecer.
    pub fn remember(&self) -> io::Result<()> {
        learn_known_hosts_path(&self.host, self.port, &self.key, &self.known_hosts)
            .map_err(|error| io::Error::other(error.to_string()))
    }

    fn describe(&self) -> String {
        format!("{} {}", self.algorithm(), self.fingerprint())
    }
}

/// O que o `known_hosts` diz da chave do servidor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KnownHost {
    /// A chave está gravada para este servidor.
    Known,
    /// O servidor não está no arquivo (a primeira conexão).
    Unknown,
    /// Há outra chave do mesmo tipo gravada para este servidor, na linha `line`: pode ser um
    /// ataque, ou o servidor foi reinstalado.
    Changed {
        /// A linha do `known_hosts`, a partir de 1.
        line: usize,
    },
}

/// Quem decide se a chave é aceita. Compartilhado entre as cópias do `SshBuilder` (o restart e
/// a duplicação reusam a mesma decisão do app), e por isso atrás de um `Mutex`.
type HostKeyCheck = Arc<Mutex<dyn FnMut(&HostKey, KnownHost) -> bool + Send>>;

/// O gancho de [`SshBuilder::before_connect`].
type BeforeConnect = Arc<Mutex<dyn FnMut(&mut dyn FnMut(&str)) -> Result<(), String> + Send>>;

/// Um fluxo de bytes qualquer sob o SSH: o `TcpStream` do primeiro servidor ou o canal
/// `direct-tcpip` aberto num salto.
trait Wire: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T: AsyncRead + AsyncWrite + Unpin + Send> Wire for T {}

/// De onde vem uma chave privada do [`SshBuilder`]: um arquivo (com a senha dele, se for
/// cifrado) ou a chave já lida, que o app guarda no cofre.
#[derive(Clone)]
enum KeySource {
    File(PathBuf, Option<String>),
    Memory(Arc<PrivateKey>),
}

impl KeySource {
    /// Para o `Debug` do builder: o caminho do arquivo, ou o aviso de que está na memória. Nunca
    /// a chave.
    fn describe(&self) -> String {
        match self {
            Self::File(path, _) => path.display().to_string(),
            Self::Memory(key) => format!("(na memória: {})", key.comment()),
        }
    }
}

/// O servidor, o usuário e a autenticação de um [`Ssh`], antes de conectar.
#[derive(Clone)]
pub struct SshBuilder {
    host: String,
    port: u16,
    user: String,
    password: Option<String>,
    keys: Vec<KeySource>,
    known_hosts: Option<PathBuf>,
    check: Option<HostKeyCheck>,
    before: Option<BeforeConnect>,
    jump: Option<Box<SshBuilder>>,
    proxy: Option<Proxy>,
    keepalive: Duration,
    compression: bool,
    agent_auth: bool,
    agent_path: Option<PathBuf>,
    agent_forward: bool,
    term: String,
    x11: Option<X11>,
    forwards: Vec<Forward>,
    exit_on_forward_failure: bool,
    no_shell: bool,
    command: Option<String>,
}

impl fmt::Debug for SshBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SshBuilder")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("password", &self.password.as_ref().map(|_| "…"))
            .field(
                "keys",
                &self
                    .keys
                    .iter()
                    .map(KeySource::describe)
                    .collect::<Vec<_>>(),
            )
            .field("known_hosts", &self.known_hosts)
            .field("before_connect", &self.before.as_ref().map(|_| "…"))
            .field("jump", &self.jump)
            .field("proxy", &self.proxy)
            .field("keepalive", &self.keepalive)
            .field("compression", &self.compression)
            .field("agent_auth", &self.agent_auth)
            .field("agent_forward", &self.agent_forward)
            .field("term", &self.term)
            .field("x11", &self.x11)
            .field("forwards", &self.forwards)
            .field("exit_on_forward_failure", &self.exit_on_forward_failure)
            .field("no_shell", &self.no_shell)
            .field("command", &self.command)
            .finish_non_exhaustive()
    }
}

impl SshBuilder {
    /// `user` em `host`, na `port` (22 é a padrão do SSH).
    pub fn new(host: impl Into<String>, port: u16, user: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port,
            user: user.into(),
            password: None,
            keys: Vec::new(),
            known_hosts: None,
            check: None,
            before: None,
            jump: None,
            proxy: None,
            keepalive: KEEPALIVE,
            compression: false,
            agent_auth: false,
            agent_path: None,
            agent_forward: false,
            term: TERM.to_owned(),
            x11: None,
            forwards: Vec::new(),
            exit_on_forward_failure: false,
            no_shell: false,
            command: None,
        }
    }

    /// Autentica com a senha, se as chaves não bastarem.
    pub fn password(mut self, password: impl Into<String>) -> Self {
        self.password = Some(password.into());
        self
    }

    /// Autentica com a chave privada em `path` (o formato do OpenSSH, como o `id_ed25519`),
    /// com a senha da chave se ela for cifrada. Pode ser chamado mais de uma vez.
    pub fn key_file(mut self, path: impl AsRef<Path>, passphrase: Option<&str>) -> Self {
        self.keys.push(KeySource::File(
            path.as_ref().to_owned(),
            passphrase.map(str::to_owned),
        ));
        self
    }

    /// Autentica com uma chave privada que já está na memória (e já foi decifrada), como a que o
    /// app guarda no cofre. Não toca o disco. Pode ser chamado mais de uma vez, junto com
    /// [`key_file`](Self::key_file); as chaves são tentadas na ordem.
    pub fn key(mut self, key: PrivateKey) -> Self {
        self.keys.push(KeySource::Memory(Arc::new(key)));
        self
    }

    /// O `known_hosts` onde a chave do servidor é conferida e gravada (o padrão é o
    /// `~/.ssh/known_hosts`, o do OpenSSH).
    pub fn known_hosts(mut self, path: impl AsRef<Path>) -> Self {
        self.known_hosts = Some(path.as_ref().to_owned());
        self
    }

    /// Quem decide se a chave do servidor é aceita, com o que o `known_hosts` diz dela. Roda
    /// na thread da conexão e pode bloquear: para perguntar à pessoa, mande a chave para a
    /// interface e espere a resposta por um canal.
    ///
    /// A mesma função serve às cópias da conexão (o restart e a duplicação) e roda uma de
    /// cada vez.
    pub fn check_host_key(
        mut self,
        check: impl FnMut(&HostKey, KnownHost) -> bool + Send + 'static,
    ) -> Self {
        self.check = Some(Arc::new(Mutex::new(check)));
        self
    }

    /// Roda `f` na thread da conexão, antes de abrir o TCP (e antes do proxy e dos saltos): é
    /// onde o app sobe a rede que leva ao servidor (uma VPN num contêiner, por exemplo) e
    /// espera o proxy responder. As linhas passadas a `aviso` aparecem no terminal; um `Err`
    /// fecha a sessão com a mensagem, como um erro de conexão, e o TCP nem chega a abrir.
    ///
    /// Roda de novo a cada reconexão (o restart e a duplicação), uma de cada vez; por isso
    /// deve poder repetir o que faz. Nas conexões que não têm terminal (o `exec`, o SFTP
    /// próprio), os avisos são descartados. Só vale o gancho do servidor de destino, não o dos
    /// saltos.
    ///
    /// ```no_run
    /// use tomni_ui_terminal::Ssh;
    ///
    /// let ssh = Ssh::builder("10.0.0.5", 22, "ana")
    ///     .before_connect(|aviso| {
    ///         aviso("Subindo a rede do cliente…");
    ///         Ok(())
    ///     })
    ///     .connect();
    /// ```
    pub fn before_connect(
        mut self,
        f: impl FnMut(&mut dyn FnMut(&str)) -> Result<(), String> + Send + 'static,
    ) -> Self {
        self.before = Some(Arc::new(Mutex::new(f)));
        self
    }

    /// Chega ao servidor por `via`, como o `ProxyJump` do OpenSSH: o cliente conecta em `via`,
    /// pede a ele um túnel até este servidor (`direct-tcpip`) e fala SSH por dentro dele. Cada
    /// salto tem a sua autenticação, o seu `known_hosts` e o seu `check_host_key`. `via` pode
    /// ter outro `jump`, e a cadeia vai do mais fundo até este servidor.
    ///
    /// Um erro num salto (a chave que mudou, a senha recusada) começa com o `host:porta` dele.
    pub fn jump(mut self, via: SshBuilder) -> Self {
        self.jump = Some(Box::new(via));
        self
    }

    /// Passa a primeira conexão TCP (a do primeiro servidor da cadeia) por um proxy SOCKS5 ou
    /// HTTP CONNECT. Se o primeiro salto tem o seu proxy, vale o dele.
    pub fn proxy(mut self, proxy: Proxy) -> Self {
        self.proxy = Some(proxy);
        self
    }

    /// De quanto em quanto tempo o cliente pergunta ao servidor se ele ainda responde (o
    /// `ServerAliveInterval` do OpenSSH; o padrão é 30 s). Depois de três perguntas sem
    /// resposta, a sessão fecha com "tempo de conexão esgotado".
    pub fn keepalive(mut self, interval: Duration) -> Self {
        self.keepalive = interval;
        self
    }

    /// Pede compressão na conexão (o `ssh -C`): `zlib@openssh.com` e `zlib` antes de `none`.
    /// O servidor que não comprime aceita o `none`, e a conexão segue igual. Vale para tudo que
    /// passa pela conexão (terminal, SFTP, `exec`, túneis). Só o destino comprime: o que passa
    /// pelos saltos já vai cifrado e não comprime. Desligada por padrão.
    pub fn compression(mut self, on: bool) -> Self {
        self.compression = on;
        self
    }

    /// Autentica com as chaves do agente (o `ssh-agent`, ou o Pageant e o agente do OpenSSH no
    /// Windows). A cópia no agente das chaves da conexão vai primeiro; as outras, depois das
    /// chaves da conexão e antes da senha. Um agente que não existe não é erro: a autenticação
    /// segue para o resto.
    pub fn agent_auth(mut self) -> Self {
        self.agent_auth = true;
        self
    }

    /// Onde fica o agente: o caminho do socket (Unix) ou do pipe nomeado (Windows). Sem ele, o
    /// agente é o do ambiente (`SSH_AUTH_SOCK`, o Pageant).
    pub fn agent_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.agent_path = Some(path.into());
        self
    }

    /// Deixa o servidor usar o agente local (o `ForwardAgent` do OpenSSH, ou `ssh -A`). Só vale
    /// no servidor de destino, não nos saltos. Use só com servidores em que se confia: quem é
    /// administrador lá pode pedir ao agente que assine em nome da pessoa.
    pub fn agent_forward(mut self, on: bool) -> Self {
        self.agent_forward = on;
        self
    }

    /// O `TERM` pedido no pseudoterminal remoto (o padrão é `xterm-256color`).
    pub fn term(mut self, term: impl Into<String>) -> Self {
        self.term = term.into();
        self
    }

    /// Encaminha o X11 (o `ssh -X` e o `ssh -Y`): os programas gráficos rodados no servidor
    /// abrem a janela no servidor X desta máquina. Só vale no servidor de destino, não nos
    /// saltos. Sem servidor X aqui, a conexão abre igual, com um aviso no terminal.
    ///
    /// ```no_run
    /// use tomni_ui_terminal::ssh::{Ssh, X11};
    ///
    /// let ssh = Ssh::builder("servidor", 22, "ana").x11(X11::trusted()).connect();
    /// ```
    pub fn x11(mut self, x11: X11) -> Self {
        self.x11 = Some(x11);
        self
    }

    /// Acrescenta um túnel (`-L`, `-R`, `-D` ou um socket Unix do servidor), que sobe junto
    /// com a conexão. Pode ser chamado mais de uma vez. Um túnel que não sobe (a porta
    /// ocupada) vira um aviso no terminal e um [`ForwardState::Failed`]; a conexão abre igual,
    /// a menos que haja [`exit_on_forward_failure`](Self::exit_on_forward_failure).
    pub fn forward(mut self, forward: Forward) -> Self {
        self.forwards.push(forward);
        self
    }

    /// Como o `ExitOnForwardFailure` do OpenSSH: se algum túnel do começo não sobe, a conexão
    /// fecha com a mensagem dele.
    pub fn exit_on_forward_failure(mut self, on: bool) -> Self {
        self.exit_on_forward_failure = on;
        self
    }

    /// Não abre shell (o `ssh -N`): a conexão só mantém os túneis e o X11, e o terminal mostra
    /// os avisos deles. Acaba ao fechar a aba ou quando a rede cai.
    pub fn no_shell(mut self, on: bool) -> Self {
        self.no_shell = on;
        self
    }

    /// Roda `command` no lugar do shell, no pseudoterminal (o `ssh -t servidor comando`). A
    /// aba mostra a saída e fecha quando o comando termina.
    pub fn command(mut self, command: impl Into<String>) -> Self {
        self.command = Some(command.into());
        self
    }

    /// Conecta, sem bloquear.
    pub fn connect(self) -> Ssh {
        let inbox = Inbox::new();
        let size = Arc::new(Mutex::new((80, 24)));
        let (commands, forwards, connecting) = start(&self, &inbox, &size);
        Ssh {
            inbox,
            commands,
            size,
            forwards,
            builder: self,
            connecting,
        }
    }

    /// Os servidores da cadeia, do primeiro salto (o mais fundo) até este.
    fn chain(&self) -> Vec<&SshBuilder> {
        let mut hops = vec![self];
        let mut current = self;
        while let Some(via) = current.jump.as_deref() {
            hops.push(via);
            current = via;
        }
        hops.reverse();
        hops
    }
}

/// Abre a thread da conexão de `builder` e devolve por onde mandar comandos a ela. Se a thread
/// não abre, o `inbox` já fecha com o motivo.
fn start(
    builder: &SshBuilder,
    inbox: &Inbox,
    size: &Arc<Mutex<(u16, u16)>>,
) -> (UnboundedSender<Command>, Arc<ForwardTable>, Arc<AtomicBool>) {
    let (commands, receiver) = mpsc::unbounded_channel();
    let connecting = Arc::new(AtomicBool::new(true));
    let thread_connecting = Arc::clone(&connecting);
    let (builder, thread_inbox, thread_size) = (builder.clone(), inbox.clone(), Arc::clone(size));
    let table = ForwardTable::new(&builder.forwards);
    let thread_table = Arc::clone(&table);
    let weak = commands.downgrade();
    let spawned = thread::Builder::new()
        .name("tomni-ui-ssh".into())
        .spawn(move || {
            let flags = Flags {
                closed: AtomicBool::new(false),
                connecting: thread_connecting,
            };
            let reason = run(
                builder,
                receiver,
                weak,
                &thread_inbox,
                &thread_size,
                &thread_table,
                &flags,
            );
            thread_table.stop_all();
            if !flags.closed.load(Ordering::Acquire) {
                thread_inbox.close(reason);
            }
            flags.ready();
        });
    if let Err(error) = spawned {
        inbox.close(Some(error_message(&error)));
        connecting.store(false, Ordering::Release);
    }
    (commands, table, connecting)
}

impl Ssh {
    /// `user` em `host`, na `port`, para escolher a autenticação antes de conectar.
    pub fn builder(host: impl Into<String>, port: u16, user: impl Into<String>) -> SshBuilder {
        SshBuilder::new(host, port, user)
    }
}

impl Drop for Ssh {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Close);
    }
}

impl Transport for Ssh {
    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.commands
            .send(Command::Data(bytes.to_vec()))
            .map_err(|_| closed_error())
    }

    fn resize(&mut self, cols: u16, rows: u16) {
        *self.size.lock().unwrap_or_else(PoisonError::into_inner) = (cols, rows);
        let _ = self.commands.send(Command::Resize(cols, rows));
    }

    fn read(&mut self, buf: &mut Vec<u8>, max: usize) -> Status {
        self.inbox.read(buf, max)
    }

    fn pending(&self) -> bool {
        self.inbox.pending()
    }

    fn connecting(&self) -> bool {
        self.connecting.load(Ordering::Acquire)
    }

    fn reopen(&mut self) -> bool {
        // A thread antiga já terminou (a fila fechou); o `Close` só garante.
        let _ = self.commands.send(Command::Close);
        self.inbox.reopen();
        (self.commands, self.forwards, self.connecting) =
            start(&self.builder, &self.inbox, &self.size);
        true
    }

    fn duplicate(&self) -> Option<Box<dyn Transport>> {
        Some(Box::new(self.builder.clone().connect()))
    }

    fn duplicate_running(&self, command: Option<&str>) -> Option<Box<dyn Transport>> {
        let mut builder = self.builder.clone();
        builder.command = command.map(str::to_owned);
        Some(Box::new(builder.connect()))
    }

    fn ssh_channels(&self) -> Option<SshChannelOpener> {
        Some(SshChannelOpener::new(self.commands.clone()))
    }

    fn ssh_forwards(&self) -> Option<SshForwards> {
        Some(SshForwards::new(
            Arc::clone(&self.forwards),
            self.commands.clone(),
        ))
    }
}

/// O `Handler` do `russh`: confere a chave do servidor e, se o app pediu, deixa o servidor usar
/// o agente local.
struct Client {
    host: String,
    port: u16,
    known_hosts: PathBuf,
    check: Option<HostKeyCheck>,
    /// Por que a chave foi recusada, para a mensagem do fim da tela.
    rejected: Arc<Mutex<Option<String>>>,
    /// O servidor pode abrir canais para o agente local.
    forward: bool,
    agent_path: Option<PathBuf>,
    /// O X11 e os túneis remotos: só o servidor de destino tem, os saltos recusam esses canais.
    live: Option<Arc<Live>>,
}

impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let key = match key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key.clone(),
            PublicKeyOrCertificate::Certificate(certificate) => {
                PublicKey::from(certificate.public_key().clone())
            }
        };
        let key = HostKey {
            host: self.host.clone(),
            port: self.port,
            key,
            known_hosts: self.known_hosts.clone(),
        };
        let known = match check_known_hosts_path(&key.host, key.port, &key.key, &key.known_hosts) {
            Ok(true) => KnownHost::Known,
            Ok(false) => KnownHost::Unknown,
            Err(russh::keys::Error::KeyChanged { line }) => KnownHost::Changed { line },
            Err(_) => KnownHost::Unknown,
        };
        let accepted = match &self.check {
            Some(check) => {
                let mut check = check.lock().unwrap_or_else(PoisonError::into_inner);
                check(&key, known)
            }
            None => known == KnownHost::Known,
        };
        if !accepted {
            let message = match known {
                KnownHost::Known => format!("a chave do servidor foi recusada: {}", key.describe()),
                KnownHost::Unknown => {
                    format!("a chave do servidor não é conhecida: {}", key.describe())
                }
                KnownHost::Changed { line } => format!(
                    "a chave do servidor mudou (linha {line} do known_hosts): {}",
                    key.describe()
                ),
            };
            *self.rejected.lock().unwrap_or_else(PoisonError::into_inner) = Some(message);
        }
        Ok(accepted)
    }

    async fn server_channel_open_agent_forward(
        &mut self,
        channel: Channel<client::Msg>,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        // Sem a permissão do app, ou sem agente, `reply` cai e o canal é recusado.
        if !self.forward {
            return Ok(());
        }
        let Ok(agent) = agent_client(self.agent_path.as_deref()).await else {
            return Ok(());
        };
        reply.accept().await;
        let mut agent = agent.into_inner();
        tokio::spawn(async move {
            let mut channel = channel.into_stream();
            let _ = tokio::io::copy_bidirectional(&mut channel, &mut agent).await;
        });
        Ok(())
    }

    async fn server_channel_open_x11(
        &mut self,
        channel: Channel<client::Msg>,
        _originator_address: &str,
        _originator_port: u32,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        // Sem X11 pedido, `reply` cai e o canal é recusado.
        let Some(live) = &self.live else {
            return Ok(());
        };
        let Some(x11) = live.x11.clone() else {
            return Ok(());
        };
        tokio::spawn(x11.serve(channel, reply, live.guard(), Arc::clone(&live.notice)));
        Ok(())
    }

    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: Channel<client::Msg>,
        connected_address: &str,
        connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        let Some(live) = &self.live else {
            return Ok(());
        };
        tokio::spawn(forward::serve_remote(
            Arc::clone(live),
            channel,
            reply,
            connected_address.to_owned(),
            connected_port,
        ));
        Ok(())
    }

    // `forwarded-streamlocal` fica sem tratar: o padrão aceitaria o canal, e ninguém pediu
    // um `-R` para socket.
    async fn server_channel_open_forwarded_streamlocal(
        &mut self,
        _channel: Channel<client::Msg>,
        _socket_path: &str,
        _reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// Abre o TCP até o primeiro servidor da cadeia (pelo proxy, se houver), depois de rodar o
/// [`SshBuilder::before_connect`], que manda os avisos para `notice`. O erro já vem com o
/// servidor, numa cadeia de saltos.
fn open_tcp(
    builder: &SshBuilder,
    notice: &mut dyn FnMut(&str),
) -> Result<std::net::TcpStream, String> {
    if let Some(before) = &builder.before {
        let mut before = before.lock().unwrap_or_else(PoisonError::into_inner);
        before(notice)?;
    }
    let chain = builder.chain();
    let first = chain[0];
    let tcp = match first.proxy.as_ref().or(builder.proxy.as_ref()) {
        Some(proxy) => proxy.open(&first.host, first.port, CONNECT_TIMEOUT),
        None => connect_tcp(&first.host, first.port, CONNECT_TIMEOUT)
            .map_err(|error| connect_error_message(&error)),
    };
    tcp.map_err(|message| at_hop(first, chain.len(), message))
}

/// A thread da conexão. Devolve o motivo do fechamento.
fn run(
    builder: SshBuilder,
    commands: UnboundedReceiver<Command>,
    weak: WeakUnboundedSender<Command>,
    inbox: &Inbox,
    size: &Mutex<(u16, u16)>,
    table: &Arc<ForwardTable>,
    flags: &Flags,
) -> Option<String> {
    let mut notice = |line: &str| inbox.push(format!("{line}\r\n").as_bytes());
    let stream = match open_tcp(&builder, &mut notice) {
        Ok(stream) => stream,
        Err(message) => return Some(message),
    };
    let x11 = builder
        .x11
        .as_ref()
        .and_then(|x11| X11State::prepare(x11, &mut notice));
    table.set_x11(x11.clone());
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => return Some(error_message(&error)),
    };
    let notice: Notice = {
        let inbox = inbox.clone();
        Arc::new(move |line: &str| inbox.push(format!("{line}\r\n").as_bytes()))
    };
    let parts = Parts {
        weak,
        table: Arc::clone(table),
        x11,
        notice,
    };
    runtime
        .block_on(session(
            &builder, stream, commands, parts, inbox, size, flags,
        ))
        .unwrap_or_else(Some)
}

/// O que a `session` recebe da thread além do builder: a ponte para os túneis, o X11 e onde
/// escrever os avisos.
struct Parts {
    weak: WeakUnboundedSender<Command>,
    table: Arc<ForwardTable>,
    x11: Option<Arc<X11State>>,
    notice: Notice,
}

/// Numa cadeia de saltos, o erro começa com o servidor onde aconteceu.
fn at_hop(hop: &SshBuilder, hops: usize, message: String) -> String {
    if hops > 1 {
        format!("{}:{}: {message}", hop.host, hop.port)
    } else {
        message
    }
}

/// Os servidores já conectados e autenticados: o de destino e os saltos, que ficam vivos até o
/// fim (derrubar um salto fecha o túnel do seguinte).
struct Connected {
    handle: Arc<Handle<Client>>,
    jumps: Vec<Handle<Client>>,
}

impl Connected {
    async fn disconnect(self) {
        let _ = self
            .handle
            .disconnect(Disconnect::ByApplication, "", "pt-BR")
            .await;
        for jump in self.jumps.iter().rev() {
            let _ = jump
                .disconnect(Disconnect::ByApplication, "", "pt-BR")
                .await;
        }
    }
}

/// Conecta e autentica em toda a cadeia de saltos de `builder`, por cima do TCP `stream` até o
/// primeiro servidor.
async fn connect_chain(
    builder: &SshBuilder,
    stream: std::net::TcpStream,
    live: Option<Arc<Live>>,
) -> Result<Connected, String> {
    let chain = builder.chain();
    let hops = chain.len();
    stream
        .set_nonblocking(true)
        .map_err(|e| error_message(&e))?;
    let stream = tokio::net::TcpStream::from_std(stream).map_err(|e| error_message(&e))?;

    let mut jumps: Vec<Handle<Client>> = Vec::new();
    let mut wire: Option<Box<dyn Wire>> = Some(Box::new(stream));
    let mut last = None;
    for (index, hop) in chain.iter().enumerate() {
        let stream = wire.take().ok_or_else(|| "sem conexão".to_owned())?;
        let is_last = index + 1 == hops;
        let opened = open(
            hop,
            stream,
            is_last,
            is_last.then(|| live.clone()).flatten(),
        )
        .await
        .map_err(|message| at_hop(hop, hops, message))?;
        match chain.get(index + 1) {
            Some(next) => {
                let channel = opened
                    .channel_open_direct_tcpip(next.host.clone(), next.port.into(), "127.0.0.1", 0)
                    .await
                    .map_err(|e| at_hop(hop, hops, tunnel_error(&e, next)))?;
                wire = Some(Box::new(channel.into_stream()));
                jumps.push(opened);
            }
            None => last = Some(opened),
        }
    }
    let handle = Arc::new(last.ok_or_else(|| "sem conexão".to_owned())?);
    Ok(Connected { handle, jumps })
}

async fn session(
    builder: &SshBuilder,
    stream: std::net::TcpStream,
    mut commands: UnboundedReceiver<Command>,
    parts: Parts,
    inbox: &Inbox,
    size: &Mutex<(u16, u16)>,
    flags: &Flags,
) -> Result<Option<String>, String> {
    let Parts {
        weak,
        table,
        x11,
        notice,
    } = parts;
    // Os canais extras (o SFTP, as conexões dos túneis, o X11) que ainda dependem desta
    // conexão, e se o shell já acabou: a aba mostra o fim, mas a conexão só cai quando o último
    // canal extra fecha.
    let extras = Arc::new(AtomicUsize::new(0));
    let live = Arc::new(Live {
        x11: x11.clone(),
        table: Arc::clone(&table),
        extras: Arc::clone(&extras),
        commands: weak.clone(),
        notice: Arc::clone(&notice),
    });
    let Connected { handle, jumps } =
        connect_chain(builder, stream, Some(Arc::clone(&live))).await?;

    let mut tunnels = Tunnels::new(Arc::clone(&handle), live);
    // Com o `?`, o `Tunnels` solto já para os ouvintes que subiram.
    tunnels.start_all(builder.exit_on_forward_failure).await?;

    let mut channel = if builder.no_shell {
        None
    } else {
        Some(open_shell(builder, &handle, x11.as_deref(), &notice, size).await?)
    };
    flags.ready();

    let mut exit = None;
    // O canal acabou sem o servidor avisar (a rede caiu, o keepalive esgotou): o motivo está no
    // resultado do `Handle`.
    let mut lost = false;
    let mut shell_done = false;
    // Sem shell, só a queda da conexão ou o `Close` encerram: confere a conexão de tempos em
    // tempos, já que não há canal para avisar.
    let mut watch = tokio::time::interval(Duration::from_millis(250));
    watch.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            command = commands.recv() => match command {
                Some(Command::Data(bytes)) => {
                    if let Some(channel) = channel.as_ref().filter(|_| !shell_done) {
                        channel.data_bytes(bytes).await.map_err(|e| ssh_error(&e))?;
                    }
                }
                Some(Command::Resize(cols, rows)) => {
                    if let Some(channel) = channel.as_ref().filter(|_| !shell_done) {
                        channel
                            .window_change(cols.into(), rows.into(), 0, 0)
                            .await
                            .map_err(|e| ssh_error(&e))?;
                    }
                }
                Some(Command::OpenSubsystem { name, reply }) => {
                    let opened = open_extra(&handle, &name, &extras, &weak).await;
                    let _ = reply.send(opened);
                }
                Some(Command::OpenExec { command, reply }) => {
                    let opened = open_extra_exec(&handle, &command, &extras, &weak).await;
                    let _ = reply.send(opened);
                }
                Some(Command::OpenDirect { host, port, reply }) => {
                    let opened = open_extra_direct(&handle, &host, port, &extras, &weak).await;
                    let _ = reply.send(opened);
                }
                Some(Command::ChannelClosed) => {
                    if shell_done && extras.load(Ordering::Acquire) == 0 {
                        break;
                    }
                }
                Some(Command::Forward(command)) => tunnels.command(command).await,
                Some(Command::Close) | None => {
                    if let Some(channel) = channel.as_ref().filter(|_| !shell_done) {
                        let _ = channel.close().await;
                    }
                    break;
                }
            },
            message = async {
                match channel.as_mut() {
                    Some(channel) => channel.wait().await,
                    None => std::future::pending().await,
                }
            }, if !shell_done => match message {
                Some(ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. }) => {
                    inbox.push(&data);
                }
                Some(ChannelMsg::ExitStatus { exit_status }) => {
                    exit = (exit_status != 0)
                        .then(|| format!("o programa remoto saiu com o código {exit_status}"));
                }
                Some(ChannelMsg::ExitSignal { signal_name, .. }) => {
                    let name = match signal_name {
                        Sig::Custom(name) => name,
                        other => format!("{other:?}"),
                    };
                    exit = Some(format!("o programa remoto terminou com o sinal SIG{name}"));
                }
                Some(ChannelMsg::Close) => {
                    let waiting = extras.load(Ordering::Acquire);
                    if waiting == 0 {
                        break;
                    }
                    shell_done = true;
                    flags.closed.store(true, Ordering::Release);
                    let busy = table.busy();
                    if busy > 0 {
                        inbox.push(
                            waiting_notice(busy).as_bytes(),
                        );
                    }
                    inbox.close(exit.clone());
                }
                None => {
                    lost = true;
                    break;
                }
                Some(_) => {}
            },
            // Com o shell encerrado, ou sem shell, só a queda da conexão acorda o laço além
            // dos comandos.
            _ = watch.tick(), if shell_done || channel.is_none() => {
                if handle.is_closed() {
                    lost = channel.is_none();
                    break;
                }
            }
        }
    }
    tunnels.shutdown();
    let handle = if lost {
        match unique(handle).await {
            Ok(mut owned) => {
                if let Ok(Err(error)) = tokio::time::timeout(END_REASON_WAIT, &mut owned).await {
                    return Err(ssh_error(&error));
                }
                Arc::new(owned)
            }
            Err(shared) => shared,
        }
    } else {
        handle
    };
    Connected { handle, jumps }.disconnect().await;
    Ok(exit)
}

/// "O shell terminou; esperando 2 conexões dos túneis (feche a aba para encerrar)."
fn waiting_notice(busy: usize) -> String {
    let what = if busy == 1 {
        "1 conexão dos túneis".to_owned()
    } else {
        format!("{busy} conexões dos túneis")
    };
    format!("\r\nO shell terminou; esperando {what} (feche a aba para encerrar).\r\n")
}

/// Tira o `Handle` do `Arc` quando as tarefas dos túneis já soltaram a parte delas. Devolve o
/// `Arc` de volta se alguém ainda o segura depois de uns instantes.
async fn unique(mut handle: Arc<Handle<Client>>) -> Result<Handle<Client>, Arc<Handle<Client>>> {
    for _ in 0..20 {
        match Arc::try_unwrap(handle) {
            Ok(owned) => return Ok(owned),
            Err(shared) => handle = shared,
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    Err(handle)
}

/// Abre o canal de sessão do terminal: o pseudoterminal, o X11, o agente e o shell (ou o
/// `command`).
async fn open_shell(
    builder: &SshBuilder,
    handle: &Handle<Client>,
    x11: Option<&X11State>,
    notice: &Notice,
    size: &Mutex<(u16, u16)>,
) -> Result<Channel<client::Msg>, String> {
    let mut channel = handle
        .channel_open_session()
        .await
        .map_err(|e| ssh_error(&e))?;
    let (cols, rows) = *size.lock().unwrap_or_else(PoisonError::into_inner);
    channel
        .request_pty(false, &builder.term, cols.into(), rows.into(), 0, 0, &[])
        .await
        .map_err(|e| ssh_error(&e))?;
    if let Some(x11) = x11 {
        request_x11(&mut channel, x11, notice).await?;
    }
    if builder.agent_forward {
        channel
            .agent_forward(false)
            .await
            .map_err(|e| ssh_error(&e))?;
    }
    match &builder.command {
        Some(command) => channel.exec(false, command.as_bytes()).await,
        None => channel.request_shell(false).await,
    }
    .map_err(|e| ssh_error(&e))?;
    Ok(channel)
}

/// Pede o X11 no canal e espera a resposta do servidor. Uma recusa vira um aviso no terminal e
/// a conexão segue sem X11.
async fn request_x11(
    channel: &mut Channel<client::Msg>,
    x11: &X11State,
    notice: &Notice,
) -> Result<(), String> {
    channel
        .request_x11(
            true,
            false,
            "MIT-MAGIC-COOKIE-1",
            x11.fake_hex(),
            x11.screen(),
        )
        .await
        .map_err(|e| ssh_error(&e))?;
    let answer = tokio::time::timeout(x11::REQUEST_TIMEOUT, async {
        loop {
            match channel.wait().await {
                Some(ChannelMsg::Success) => return true,
                Some(ChannelMsg::Failure) | None => return false,
                Some(_) => {}
            }
        }
    })
    .await;
    if !matches!(answer, Ok(true)) {
        notice(
            "O servidor recusou o encaminhamento do X11 (confira o X11Forwarding no sshd_config).",
        );
    }
    Ok(())
}

/// Abre o canal de um subsistema para fora do terminal e conta nos `extras`.
async fn open_extra(
    handle: &Handle<Client>,
    name: &str,
    extras: &Arc<AtomicUsize>,
    commands: &WeakUnboundedSender<Command>,
) -> Result<SshChannel, String> {
    let open = async {
        let channel = handle
            .channel_open_session()
            .await
            .map_err(|e| ssh_error(&e))?;
        channel
            .request_subsystem(true, name)
            .await
            .map_err(|e| ssh_error(&e))?;
        Ok::<_, String>(channel.into_stream())
    };
    let stream = tokio::time::timeout(CHANNEL_OPEN_TIMEOUT, open)
        .await
        .map_err(|_| "o servidor não respondeu ao pedido do canal".to_owned())??;
    extras.fetch_add(1, Ordering::AcqRel);
    Ok(SshChannel::counted(
        stream,
        ChannelGuard {
            extras: Arc::clone(extras),
            commands: commands.clone(),
        },
    ))
}

/// O `client::Config` de `hop`. A compressão só vale no servidor de destino (`last`): o russh
/// põe `none` na frente da lista, e o cliente fica com o primeiro que o servidor aceita, então
/// sem mexer na lista nunca há compressão.
pub(crate) fn client_config(hop: &SshBuilder, last: bool) -> client::Config {
    let mut config = client::Config {
        keepalive_interval: Some(hop.keepalive),
        nodelay: true,
        ..Default::default()
    };
    if last && hop.compression {
        config.preferred = russh::Preferred {
            compression: std::borrow::Cow::Borrowed(&[
                russh::compression::ZLIB_LEGACY,
                russh::compression::ZLIB,
                russh::compression::NONE,
            ]),
            ..russh::Preferred::default()
        };
    }
    config
}

/// Abre um canal `direct-tcpip` até `host:port`, visto do servidor, contado como canal extra.
async fn open_extra_direct(
    handle: &Handle<Client>,
    host: &str,
    port: u16,
    extras: &Arc<AtomicUsize>,
    commands: &WeakUnboundedSender<Command>,
) -> Result<SshChannel, String> {
    let destination = format!("{host}:{port}");
    let open = handle.channel_open_direct_tcpip(host, u32::from(port), "127.0.0.1", 0);
    let channel = match tokio::time::timeout(CHANNEL_OPEN_TIMEOUT, open).await {
        Err(_) => {
            return Err(format!(
                "O servidor não abriu {destination}: não respondeu ao pedido do canal"
            ));
        }
        Ok(Err(russh::Error::ChannelOpenFailure(russh::ChannelOpenFailure::ConnectFailed))) => {
            return Err(format!("Nada escuta em {destination} no servidor."));
        }
        Ok(Err(error)) => return Err(forward::open_error(&error, &destination)),
        Ok(Ok(channel)) => channel,
    };
    extras.fetch_add(1, Ordering::AcqRel);
    Ok(SshChannel::counted(
        channel.into_stream(),
        ChannelGuard {
            extras: Arc::clone(extras),
            commands: commands.clone(),
        },
    ))
}

/// Abre um canal `exec` com `command` na conexão do terminal, contado como canal extra.
async fn open_extra_exec(
    handle: &Handle<Client>,
    command: &str,
    extras: &Arc<AtomicUsize>,
    commands: &WeakUnboundedSender<Command>,
) -> Result<ExecChannel, String> {
    let open = async {
        let channel = handle
            .channel_open_session()
            .await
            .map_err(|e| ssh_error(&e))?;
        channel
            .exec(true, command.as_bytes())
            .await
            .map_err(|e| ssh_error(&e))?;
        Ok::<_, String>(channel)
    };
    let channel = tokio::time::timeout(CHANNEL_OPEN_TIMEOUT, open)
        .await
        .map_err(|_| "o servidor não respondeu ao pedido do canal".to_owned())??;
    extras.fetch_add(1, Ordering::AcqRel);
    Ok(ExecChannel::new(
        channel,
        ChannelGuard {
            extras: Arc::clone(extras),
            commands: commands.clone(),
        },
    ))
}

/// Conecta o SSH de `hop` por cima de `stream`, confere a chave do servidor e autentica.
/// `last` é o servidor de destino, o único que pode encaminhar o agente.
async fn open(
    hop: &SshBuilder,
    stream: Box<dyn Wire>,
    last: bool,
    live: Option<Arc<Live>>,
) -> Result<Handle<Client>, String> {
    let known_hosts = match &hop.known_hosts {
        Some(path) => path.clone(),
        None => std::env::home_dir()
            .unwrap_or_default()
            .join(".ssh")
            .join("known_hosts"),
    };
    let config = Arc::new(client_config(hop, last));
    let rejected = Arc::new(Mutex::new(None));
    let handler = Client {
        host: hop.host.clone(),
        port: hop.port,
        known_hosts,
        check: hop.check.clone(),
        rejected: Arc::clone(&rejected),
        forward: last && hop.agent_forward,
        agent_path: hop.agent_path.clone(),
        live,
    };
    let mut handle = client::connect_stream(config, stream, handler)
        .await
        .map_err(|error| {
            rejected
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take()
                .unwrap_or_else(|| ssh_error(&error))
        })?;
    authenticate(&mut handle, hop).await?;
    Ok(handle)
}

/// Tenta as chaves e depois a senha (ou o método `none`, sem nenhuma das duas), na ordem do
/// OpenSSH: primeiro a cópia no agente das chaves da conexão ([`SshBuilder::key_file`] e
/// [`SshBuilder::key`]), depois as próprias chaves e só então as outras do agente. O servidor
/// derruba quem erra demais (o `MaxAuthTries`, 6 no OpenSSH): com muitas chaves no agente, a
/// chave certa ofertada por último nunca chegaria.
async fn authenticate(handle: &mut Handle<Client>, hop: &SshBuilder) -> Result<(), String> {
    let user = hop.user.as_str();
    // Um arquivo que não abre (não existe, falta a senha) não interrompe: o agente pode ter a
    // chave. O erro dele só aparece se nada mais der certo.
    let mut keys = Vec::new();
    let mut preferred = Vec::new();
    let mut key_error = None;
    for source in &hop.keys {
        match open_key(source) {
            Ok(key) => {
                preferred.push(key.public_key().clone());
                keys.push(key);
            }
            Err(error) => {
                if let KeySource::File(path, _) = source
                    && let Some(public) = public_of_file(path)
                {
                    preferred.push(public);
                }
                key_error.get_or_insert(error);
            }
        }
    }
    let mut tries = 0;
    if hop.agent_auth
        && !preferred.is_empty()
        && authenticate_with_agent(
            handle,
            user,
            hop.agent_path.as_deref(),
            AgentKeys::Only(&preferred),
            &mut tries,
        )
        .await?
    {
        return Ok(());
    }
    for key in keys {
        let hash = if key.algorithm().is_rsa() {
            handle
                .best_supported_rsa_hash()
                .await
                .map_err(|e| ssh_error(&e))?
                .flatten()
        } else {
            None
        };
        let key = PrivateKeyWithHashAlg::new(Arc::new(key), hash);
        let result = handle
            .authenticate_publickey(user, key)
            .await
            .map_err(|e| ssh_error(&e))?;
        if result.success() {
            return Ok(());
        }
        tries += 1;
    }
    if hop.agent_auth
        && authenticate_with_agent(
            handle,
            user,
            hop.agent_path.as_deref(),
            AgentKeys::Except(&preferred),
            &mut tries,
        )
        .await?
    {
        return Ok(());
    }
    let result = match &hop.password {
        Some(password) => handle.authenticate_password(user, password.clone()).await,
        None if hop.keys.is_empty() => handle.authenticate_none(user).await,
        None => {
            return Err(key_error.unwrap_or_else(|| format!("autenticação recusada para {user}")));
        }
    };
    if result.map_err(|e| ssh_error(&e))?.success() {
        Ok(())
    } else {
        Err(key_error.unwrap_or_else(|| format!("autenticação recusada para {user}")))
    }
}

/// Abre a chave de `source`: lê o arquivo (com a senha, se veio) ou copia a da memória.
fn open_key(source: &KeySource) -> Result<PrivateKey, String> {
    match source {
        KeySource::File(path, passphrase) => {
            load_secret_key(path, passphrase.as_deref()).map_err(|error| {
                let path = path.display();
                match error {
                    russh::keys::Error::KeyIsEncrypted => {
                        format!("a chave {path} precisa da senha")
                    }
                    russh::keys::Error::IO(error) => {
                        format!(
                            "não foi possível ler a chave {path}: {}",
                            error_message(&error)
                        )
                    }
                    _ => format!("não foi possível ler a chave {path}: {error}"),
                }
            })
        }
        KeySource::Memory(key) => Ok((**key).clone()),
    }
}

/// A chave pública de um arquivo de chave que não abriu (cifrado e sem a senha): a do `.pub` ao
/// lado ou, no formato do OpenSSH, a que vai às claras dentro do próprio arquivo.
fn public_of_file(path: &Path) -> Option<PublicKey> {
    let mut pub_path = path.as_os_str().to_owned();
    pub_path.push(".pub");
    if let Ok(public) = PublicKey::read_openssh_file(Path::new(&pub_path)) {
        return Some(public);
    }
    PrivateKey::read_openssh_file(path)
        .ok()
        .map(|key| key.public_key().clone())
}

/// Quais chaves do agente oferecer.
#[derive(Clone, Copy)]
enum AgentKeys<'a> {
    /// Só as que estão na lista (as da conexão).
    Only(&'a [PublicKey]),
    /// Todas, menos as da lista (já oferecidas).
    Except(&'a [PublicKey]),
}

impl AgentKeys<'_> {
    fn offers(self, key: &PublicKey) -> bool {
        let listed = |list: &[PublicKey]| list.iter().any(|k| k.key_data() == key.key_data());
        match self {
            Self::Only(list) => listed(list),
            Self::Except(list) => !listed(list),
        }
    }
}

/// Oferece ao servidor as chaves do agente que `which` escolhe, e o agente assina o desafio.
/// `tries` conta as chaves recusadas (as de antes também), para a mensagem. Um agente que não
/// existe, que não responde ou que não tem a chave do servidor não é erro: devolve `false` e a
/// autenticação segue. Já um agente que **recusa assinar** (a chave pede confirmação e foi
/// negada) é erro: o servidor ficou esperando a assinatura e a conexão não serve mais. O
/// servidor que derruba a conexão depois de recusas demais também.
async fn authenticate_with_agent(
    handle: &mut Handle<Client>,
    user: &str,
    path: Option<&Path>,
    which: AgentKeys<'_>,
    tries: &mut usize,
) -> Result<bool, String> {
    let Ok(mut agent) = agent_client(path).await else {
        return Ok(false);
    };
    let Ok(identities) = agent.request_identities().await else {
        return Ok(false);
    };
    for identity in identities {
        // Os certificados do agente ficam para depois: só as chaves simples.
        let AgentIdentity::PublicKey { key, .. } = identity else {
            continue;
        };
        if !which.offers(&key) {
            continue;
        }
        let hash = if key.algorithm().is_rsa() {
            handle
                .best_supported_rsa_hash()
                .await
                .ok()
                .flatten()
                .flatten()
        } else {
            None
        };
        match handle
            .authenticate_publickey_with(user, key, hash, &mut agent)
            .await
        {
            Ok(result) if result.success() => return Ok(true),
            Ok(_) => *tries += 1,
            Err(russh::AgentAuthError::Send(_)) => {
                return Err(format!(
                    "o servidor encerrou a conexão depois de {tries} chave(s) recusada(s) (o \
                     limite MaxAuthTries dele); ligue à conexão a chave que ele aceita",
                    tries = *tries
                ));
            }
            Err(russh::AgentAuthError::Key(_)) => {
                return Err("o agente recusou usar a chave".into());
            }
        }
    }
    Ok(false)
}

/// O cliente do agente de chaves: o socket em `path` (ou o do ambiente, `SSH_AUTH_SOCK`) no
/// Unix; no Windows, o pipe nomeado em `path`, ou o Pageant e depois o agente do OpenSSH.
async fn agent_client(
    path: Option<&Path>,
) -> Result<AgentClient<Box<dyn AgentStream + Send + Unpin>>, russh::keys::Error> {
    #[cfg(unix)]
    {
        let client = match path {
            Some(path) => AgentClient::connect_uds(path).await?,
            None => AgentClient::connect_env().await?,
        };
        Ok(client.dynamic())
    }
    #[cfg(windows)]
    {
        if let Some(path) = path {
            return Ok(AgentClient::connect_named_pipe(path).await?.dynamic());
        }
        match AgentClient::connect_pageant().await {
            Ok(client) => Ok(client.dynamic()),
            Err(_) => Ok(AgentClient::connect_named_pipe(agent::OPENSSH_PIPE)
                .await?
                .dynamic()),
        }
    }
}

/// O erro de abrir o túnel até o próximo salto.
fn tunnel_error(error: &russh::Error, next: &SshBuilder) -> String {
    match error {
        russh::Error::ChannelOpenFailure(_) => {
            format!("o servidor recusou o túnel até {}:{}", next.host, next.port)
        }
        other => ssh_error(other),
    }
}

fn ssh_error(error: &russh::Error) -> String {
    match error {
        russh::Error::IO(error) => error_message(error),
        russh::Error::ConnectionTimeout
        | russh::Error::KeepaliveTimeout
        | russh::Error::InactivityTimeout
        | russh::Error::Elapsed(_) => "tempo de conexão esgotado".to_owned(),
        russh::Error::HUP | russh::Error::Disconnect => "a conexão foi encerrada".to_owned(),
        russh::Error::NoCommonAlgo { .. } => {
            "o servidor não aceita nenhum dos algoritmos do cliente".to_owned()
        }
        other => format!("erro do SSH: {other}"),
    }
}

#[cfg(test)]
mod tests;
