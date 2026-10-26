//! A ponte SOCKS5 local: transforma a [`NetStack`] num proxy em `127.0.0.1`, para os clientes da
//! lib (SSH, SFTP, Telnet, FTP, S3 e rclone) usarem a rede do túnel pelo `Proxy::Socks5` que já
//! têm. Só `CONNECT`, com usuário e senha (RFC 1929) aleatórios: outro programa do computador
//! não consegue usar o túnel.

use std::fmt;
use std::future::Future;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener as StdTcpListener};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt, copy_bidirectional};
use tokio::net::TcpStream as LocalStream;
use tokio::sync::watch;
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::timeout;
use zeroize::Zeroizing;

use crate::NetStack;

/// Quanto o cliente tem para cumprir cada passo da negociação SOCKS (a mesma espera que o
/// cliente de `tomni-ui-net` usa para esperar o proxy).
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// O que o cliente precisa para usar a ponte.
#[derive(Clone)]
pub struct SocksAccess {
    /// O endereço da ponte (sempre `127.0.0.1` e uma porta livre).
    pub addr: SocketAddr,
    /// O usuário da RFC 1929, aleatório.
    pub user: String,
    /// A senha da RFC 1929, aleatória.
    pub password: Zeroizing<String>,
}

impl fmt::Debug for SocksAccess {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SocksAccess")
            .field("addr", &self.addr)
            .field("user", &self.user)
            .field("password", &"…")
            .finish()
    }
}

fn random_hex(bytes: usize) -> io::Result<String> {
    let mut raw = Zeroizing::new(vec![0u8; bytes]);
    getrandom::fill(&mut raw)
        .map_err(|e| io::Error::other(format!("sem números aleatórios: {e}")))?;
    Ok(raw.iter().map(|b| format!("{b:02x}")).collect())
}

/// O que um [`Resolver`] respondeu sobre um nome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    /// Os endereços do nome.
    Found(Vec<IpAddr>),
    /// O resolvedor cuida do nome e ele não existe (ou o servidor não respondeu): a ponte
    /// recusa o `CONNECT` e **não** pergunta ao resolvedor do computador, para o nome não vazar.
    NotFound,
    /// O nome não é do resolvedor (um domínio fora da lista da VPN): a ponte segue para o
    /// resolvedor do computador.
    Skip,
}

/// Quem resolve os nomes pedidos à ponte, depois de o próprio túnel
/// ([`PacketTunnel::resolve`](crate::PacketTunnel::resolve)) não saber. O [`DnsOverTcp`](crate::DnsOverTcp)
/// é o das VPNs que empurram servidores DNS.
pub trait Resolver: Send + Sync + 'static {
    /// Resolve `name` usando `stack` para falar com os servidores da rede.
    fn resolve<'a>(
        &'a self,
        stack: &'a NetStack,
        name: &'a str,
    ) -> Pin<Box<dyn Future<Output = Answer> + Send + 'a>>;
}

/// A porta da ponte, já aberta mas ainda sem pilha. O `bind` é síncrono e devolve a porta na hora:
/// o app precisa dela antes de o túnel subir.
pub struct SocksListener {
    listener: StdTcpListener,
    access: SocksAccess,
    connect_timeout: Duration,
    resolver: Option<Arc<dyn Resolver>>,
}

impl SocksListener {
    /// Abre `127.0.0.1:0` (nunca `0.0.0.0`), com usuário e senha aleatórios de 16 bytes cada,
    /// em hexadecimal.
    pub fn bind() -> io::Result<SocksListener> {
        let listener = StdTcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let access = SocksAccess {
            addr: listener.local_addr()?,
            user: random_hex(16)?,
            password: Zeroizing::new(random_hex(16)?),
        };
        Ok(SocksListener {
            listener,
            access,
            connect_timeout: tomni_ui_net::CONNECT_TIMEOUT,
            resolver: None,
        })
    }

    /// O endereço e a credencial da ponte.
    pub fn access(&self) -> &SocksAccess {
        &self.access
    }

    /// Quanto a ponte espera a pilha abrir cada conexão antes de responder "tempo esgotado"
    /// (0x06). O padrão é o `CONNECT_TIMEOUT` do `tomni-ui-net`.
    pub fn connect_timeout(mut self, limit: Duration) -> Self {
        self.connect_timeout = limit;
        self
    }

    /// Quem resolve os nomes que o túnel não sabe, antes do resolvedor do computador (o DNS que
    /// a VPN empurrou, por exemplo).
    pub fn resolver(mut self, resolver: Arc<dyn Resolver>) -> Self {
        self.resolver = Some(resolver);
        self
    }

    /// Começa a aceitar conexões sobre `stack`, no runtime atual.
    pub fn serve(self, stack: NetStack) -> io::Result<SocksBridge> {
        let bridge = self.serve_later()?;
        bridge.attach(stack);
        Ok(bridge)
    }

    /// Começa a aceitar conexões **antes** de haver pilha (a VPN ainda está subindo): cada
    /// conexão faz a negociação SOCKS e espera a pilha chegar por [`SocksBridge::attach`], pelo
    /// mesmo tempo do `connect_timeout`; passado esse tempo, responde "rede inalcançável".
    pub fn serve_later(self) -> io::Result<SocksBridge> {
        let listener = tokio::net::TcpListener::from_std(self.listener)?;
        let access = Arc::new(self.access);
        let shared = access.clone();
        let limit = self.connect_timeout;
        let resolver = self.resolver;
        let (sender, receiver) = watch::channel(None::<NetStack>);
        let task = tokio::spawn(async move {
            // O `JoinSet` aborta as conexões em andamento quando a tarefa é abortada.
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((client, _)) = accepted else { continue };
                        let (stack, access, resolver) =
                            (receiver.clone(), shared.clone(), resolver.clone());
                        connections.spawn(async move {
                            let _ = serve_connection(client, stack, &access, limit, resolver)
                                .await;
                        });
                    }
                    Some(_) = connections.join_next(), if !connections.is_empty() => {}
                }
            }
        });
        Ok(SocksBridge {
            access: (*access).clone(),
            task,
            stack: sender,
        })
    }
}

/// A ponte SOCKS5 em funcionamento.
pub struct SocksBridge {
    access: SocksAccess,
    task: JoinHandle<()>,
    stack: watch::Sender<Option<NetStack>>,
}

impl SocksBridge {
    /// Abre a ponte em `127.0.0.1`, numa porta livre, com usuário e senha aleatórios.
    pub async fn start(stack: NetStack) -> io::Result<SocksBridge> {
        SocksListener::bind()?.serve(stack)
    }

    /// O endereço e a credencial.
    pub fn access(&self) -> &SocksAccess {
        &self.access
    }

    /// Passa a servir as conexões novas pela `stack` (a que a VPN acabou de montar). As
    /// conexões em andamento seguem na pilha antiga.
    pub fn attach(&self, stack: NetStack) {
        self.stack.send_replace(Some(stack));
    }

    /// Tira a pilha: as conexões novas esperam por outra (ou desistem pelo `connect_timeout`).
    pub fn detach(&self) {
        self.stack.send_replace(None);
    }

    /// Para de aceitar e derruba as conexões em andamento.
    pub async fn stop(self) {
        // O `Drop` aborta a tarefa; esperar o fim dela garante que a porta já foi liberada.
        self.task.abort();
        while !self.task.is_finished() {
            tokio::task::yield_now().await;
        }
    }
}

impl Drop for SocksBridge {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Compara sem parar no primeiro byte diferente.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Os códigos de resposta do SOCKS5 (RFC 1928, seção 6).
mod code {
    pub(super) const GENERAL_FAILURE: u8 = 0x01;
    pub(super) const NETWORK_UNREACHABLE: u8 = 0x03;
    pub(super) const HOST_UNREACHABLE: u8 = 0x04;
    pub(super) const REFUSED: u8 = 0x05;
    pub(super) const TIMED_OUT: u8 = 0x06;
    pub(super) const COMMAND_NOT_SUPPORTED: u8 = 0x07;
    pub(super) const ADDRESS_NOT_SUPPORTED: u8 = 0x08;
}

async fn reply(client: &mut LocalStream, code: u8) -> io::Result<()> {
    client.write_all(&[5, code, 0, 1, 0, 0, 0, 0, 0, 0]).await
}

fn protocol_error(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_owned())
}

enum Target {
    Ip(IpAddr),
    Name(String),
}

async fn serve_connection(
    mut client: LocalStream,
    mut stacks: watch::Receiver<Option<NetStack>>,
    access: &SocksAccess,
    connect_timeout: Duration,
    resolver: Option<Arc<dyn Resolver>>,
) -> io::Result<()> {
    let _ = client.set_nodelay(true);
    let (target, port) = timeout(HANDSHAKE_TIMEOUT, negotiate(&mut client, access))
        .await
        .map_err(|_| protocol_error("o cliente SOCKS demorou a negociar"))??;
    let stack = match timeout(connect_timeout, stacks.wait_for(Option::is_some)).await {
        Ok(Ok(current)) => (*current).clone(),
        _ => None,
    };
    let Some(stack) = stack else {
        reply(&mut client, code::NETWORK_UNREACHABLE).await?;
        return Ok(());
    };

    let candidates: Vec<IpAddr> = match target {
        Target::Ip(ip) => vec![ip],
        Target::Name(name) => match name.parse::<IpAddr>() {
            Ok(ip) => vec![ip],
            Err(_) => {
                // Primeiro o túnel; depois o `Resolver` (o DNS que a VPN empurrou); por fim o
                // resolvedor do computador, que só serve para nomes que a pessoa já resolve (um
                // `hosts` ou um DNS que aponte para o IP da rede).
                let found = match stack.resolve(&name).await {
                    Some(list) => Some(list),
                    None => {
                        let answer = match &resolver {
                            Some(resolver) => resolver.resolve(&stack, &name).await,
                            None => Answer::Skip,
                        };
                        match answer {
                            Answer::Found(list) => Some(list),
                            Answer::NotFound => None,
                            Answer::Skip => tokio::net::lookup_host((name.as_str(), port))
                                .await
                                .ok()
                                .map(|list| list.map(|addr| addr.ip()).collect()),
                        }
                    }
                };
                match found {
                    Some(list) if !list.is_empty() => list,
                    _ => {
                        reply(&mut client, code::HOST_UNREACHABLE).await?;
                        return Ok(());
                    }
                }
            }
        },
    };
    let Some(ip) = candidates.into_iter().find(|ip| stack.routes_to(*ip)) else {
        reply(&mut client, code::NETWORK_UNREACHABLE).await?;
        return Ok(());
    };
    match stack
        .connect_timeout(SocketAddr::new(ip, port), connect_timeout)
        .await
    {
        Ok(mut remote) => {
            reply(&mut client, 0).await?;
            let _ = copy_bidirectional(&mut client, &mut remote).await;
            let _ = remote.shutdown().await;
            Ok(())
        }
        Err(error) => {
            let answer = match error.kind() {
                io::ErrorKind::ConnectionRefused => code::REFUSED,
                io::ErrorKind::TimedOut => code::TIMED_OUT,
                io::ErrorKind::HostUnreachable => code::HOST_UNREACHABLE,
                io::ErrorKind::NetworkUnreachable => code::NETWORK_UNREACHABLE,
                _ => code::GENERAL_FAILURE,
            };
            reply(&mut client, answer).await
        }
    }
}

/// A saudação, a autenticação por usuário e senha e o pedido. Responde os erros do protocolo e
/// devolve o destino do `CONNECT`.
async fn negotiate(client: &mut LocalStream, access: &SocksAccess) -> io::Result<(Target, u16)> {
    let mut head = [0u8; 2];
    client.read_exact(&mut head).await?;
    if head[0] != 5 {
        return Err(protocol_error("não é SOCKS5"));
    }
    let mut methods = vec![0u8; usize::from(head[1])];
    client.read_exact(&mut methods).await?;
    // Só o método 0x02 (usuário e senha); sem autenticação (0x00) é recusado.
    if !methods.contains(&2) {
        client.write_all(&[5, 0xFF]).await?;
        return Err(protocol_error("o cliente não ofereceu usuário e senha"));
    }
    client.write_all(&[5, 2]).await?;

    let mut version = [0u8; 2];
    client.read_exact(&mut version).await?;
    if version[0] != 1 {
        return Err(protocol_error("autenticação fora da RFC 1929"));
    }
    let mut user = vec![0u8; usize::from(version[1])];
    client.read_exact(&mut user).await?;
    let mut password_len = [0u8; 1];
    client.read_exact(&mut password_len).await?;
    let mut password = Zeroizing::new(vec![0u8; usize::from(password_len[0])]);
    client.read_exact(&mut password).await?;
    let user_ok = same(&user, access.user.as_bytes());
    let password_ok = same(&password, access.password.as_bytes());
    if !(user_ok & password_ok) {
        client.write_all(&[1, 1]).await?;
        return Err(protocol_error("usuário ou senha errados"));
    }
    client.write_all(&[1, 0]).await?;

    let mut request = [0u8; 4];
    client.read_exact(&mut request).await?;
    if request[0] != 5 {
        return Err(protocol_error("pedido fora do SOCKS5"));
    }
    let target = match request[3] {
        1 => {
            let mut raw = [0u8; 4];
            client.read_exact(&mut raw).await?;
            Target::Ip(IpAddr::from(raw))
        }
        4 => {
            let mut raw = [0u8; 16];
            client.read_exact(&mut raw).await?;
            Target::Ip(IpAddr::from(raw))
        }
        3 => {
            let mut len = [0u8; 1];
            client.read_exact(&mut len).await?;
            let mut raw = vec![0u8; usize::from(len[0])];
            client.read_exact(&mut raw).await?;
            Target::Name(String::from_utf8_lossy(&raw).into_owned())
        }
        _ => {
            reply(client, code::ADDRESS_NOT_SUPPORTED).await?;
            return Err(protocol_error("tipo de endereço desconhecido"));
        }
    };
    let mut port = [0u8; 2];
    client.read_exact(&mut port).await?;
    // Só CONNECT (0x01); BIND e UDP ASSOCIATE respondem 0x07.
    if request[1] != 1 {
        reply(client, code::COMMAND_NOT_SUPPORTED).await?;
        return Err(protocol_error("só o CONNECT é suportado"));
    }
    Ok((target, u16::from_be_bytes(port)))
}
