//! A pilha TCP/IP em software do Tomni UI (M48).
//!
//! Uma VPN que entrega pacotes IP (o Nebula, e no futuro o WireGuard, o OpenVPN e o Tailscale)
//! vira, aqui, um [`PacketTunnel`]. Por cima dele roda uma pilha TCP/IP em software (o
//! `smoltcp`): [`NetStack::connect`] abre conexões TCP pela rede do túnel, **sem placa de rede
//! virtual, sem mexer nas rotas do sistema e sem administrador**. A [`SocksBridge`] transforma a
//! pilha num SOCKS5 local em `127.0.0.1`, e os clientes de SSH, SFTP, Telnet, FTP, S3 e rclone da
//! lib passam a usá-la pelo `Proxy::Socks5` que já tinham, sem mudar nada.
//!
//! O app só **abre** conexões. O filtro de saída descarta em silêncio tudo o que não é resposta
//! de uma conexão aberta por ele (SYN para outra porta, UDP, ICMP): para a rede, o app é
//! invisível, não responde nem ping.
//!
//! ```no_run
//! use tomni_ui_netstack::{NetStack, PacketTunnel, SocksBridge};
//!
//! async fn exemplo(tunel: impl PacketTunnel) -> std::io::Result<()> {
//!     let pilha = NetStack::spawn(tunel);
//!     // Direto, pela pilha…
//!     let _tcp = pilha.connect("10.42.0.10:22".parse().unwrap()).await?;
//!     // …ou por um SOCKS5 local, com usuário e senha aleatórios.
//!     let ponte = SocksBridge::start(pilha).await?;
//!     println!("proxy em {}", ponte.access().addr);
//!     Ok(())
//! }
//! ```

mod cidr;
mod device;
mod dns;
#[cfg(feature = "ethernet")]
pub mod ethernet;
mod filter;
#[cfg(feature = "test-util")]
pub mod pipe;
mod socks;
mod stack;
mod stream;
mod underlay;

use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::sync::{Notify, mpsc, oneshot};
use tokio::task::JoinHandle;

pub use cidr::Cidr;
pub use dns::{DnsOverTcp, DnsScope};
pub use socks::{Answer, Resolver, SocksAccess, SocksBridge, SocksListener};
pub use stream::TcpStream;
#[cfg(feature = "test-util")]
pub use underlay::{NatKind, SimEndpoint, SimNat, SimNet};
pub use underlay::{UdpUnderlay, Underlay};

use stack::{Command, SharedRoutes, Stack};
use stream::Shared;

/// Quanto cada buffer TCP (de envio e de recepção) guarda por conexão: 256 KiB, o produto banda
/// x atraso de uma ligação de 20 Mbit/s com 100 ms de ida e volta.
pub const DEFAULT_TCP_BUFFER: usize = 256 * 1024;

/// Um endereço que o túnel não consegue alcançar (por exemplo, o handshake com o par falhou).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unreachable {
    /// O endereço dentro da rede do túnel.
    pub addr: IpAddr,
    /// O porquê, em português, que vira a mensagem do erro de conexão.
    pub reason: String,
}

/// Um túnel que transporta pacotes IP: a VPN embaixo da pilha.
pub trait PacketTunnel: Send + 'static {
    /// O maior pacote IP que o túnel leva (o MTU interno).
    fn mtu(&self) -> usize;

    /// Os endereços desta ponta dentro da rede (o IP do certificado, no Nebula).
    fn addresses(&self) -> Vec<Cidr>;

    /// As redes alcançáveis pelo túnel. O resto é recusado na hora.
    fn routes(&self) -> Vec<Cidr>;

    /// Entrega um pacote IP que sai. Não bloqueia: se o túnel não pode levar agora, descarta (o
    /// TCP retransmite).
    fn send(&mut self, packet: &[u8]);

    /// Espera o próximo pacote IP que chega: limpa `buf` e o enche com ele. `Err` quer dizer que
    /// o túnel acabou, e a pilha para.
    fn poll_recv(&mut self, cx: &mut Context<'_>, buf: &mut Vec<u8>) -> Poll<io::Result<()>>;

    /// Avisa que as redes alcançáveis mudaram: devolve a lista nova **inteira** (no Tailscale as
    /// rotas vêm do mapa, que muda com uma máquina nova ou uma sub-rede aprovada). O padrão
    /// nunca avisa: as redes de [`routes`](PacketTunnel::routes) valem para sempre.
    fn poll_routes(&mut self, cx: &mut Context<'_>) -> Poll<Vec<Cidr>> {
        let _ = cx;
        Poll::Pending
    }

    /// Avisa que um endereço não pode ser alcançado: as conexões que esperam por ele falham com
    /// "host inalcançável". O padrão nunca avisa.
    fn poll_unreachable(&mut self, cx: &mut Context<'_>) -> Poll<Unreachable> {
        let _ = cx;
        Poll::Pending
    }

    /// Resolve um nome dentro da rede, quando o túnel sabe (o DNS do lighthouse, no futuro). O
    /// padrão é `None`: a ponte usa o resolvedor do sistema.
    fn resolve(&mut self, name: &str) -> Option<Vec<IpAddr>> {
        let _ = name;
        None
    }
}

/// Monta uma [`NetStack`] com ajustes.
#[derive(Clone, Debug)]
pub struct NetStackBuilder {
    tcp_buffer: usize,
    #[cfg(feature = "test-util")]
    any_ip: bool,
}

impl NetStackBuilder {
    /// O tamanho, em bytes, de cada buffer TCP por conexão (o padrão é [`DEFAULT_TCP_BUFFER`]).
    pub fn tcp_buffer(mut self, bytes: usize) -> Self {
        self.tcp_buffer = bytes.max(1);
        self
    }

    /// Com `sim`, a pilha aceita pacotes para qualquer IP (o `any_ip` do `smoltcp`): faz o papel
    /// de gateway de uma rede nos testes. O app de verdade nunca liga isto.
    #[cfg(feature = "test-util")]
    pub fn any_ip(mut self, sim: bool) -> Self {
        self.any_ip = sim;
        self
    }

    /// Põe a pilha para rodar sobre o túnel, no runtime atual do `tokio`.
    pub fn spawn(self, tunnel: impl PacketTunnel) -> NetStack {
        let routes: SharedRoutes = Arc::new(RwLock::new(tunnel.routes().into()));
        let addresses: Arc<[Cidr]> = tunnel.addresses().into();
        let wake = Arc::new(Notify::new());
        let (commands, receiver) = mpsc::unbounded_channel();
        #[allow(unused_mut)]
        let mut stack = Stack::new(
            Box::new(tunnel),
            wake.clone(),
            self.tcp_buffer,
            routes.clone(),
        );
        #[cfg(feature = "test-util")]
        stack.set_any_ip(self.any_ip);
        let task = tokio::spawn(stack.run(receiver));
        NetStack {
            commands,
            wake,
            routes,
            addresses,
            tcp_buffer: self.tcp_buffer,
            task: Arc::new(Mutex::new(Some(task))),
        }
    }
}

/// A pilha, rodando numa tarefa do `tokio`. Clonar dá outra alça para a mesma pilha; ela para
/// quando alguém chama [`shutdown`](NetStack::shutdown) ou quando a última alça some.
#[derive(Clone)]
pub struct NetStack {
    commands: mpsc::UnboundedSender<Command>,
    wake: Arc<Notify>,
    routes: SharedRoutes,
    addresses: Arc<[Cidr]>,
    tcp_buffer: usize,
    task: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl std::fmt::Debug for NetStack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NetStack")
            .field("addresses", &self.addresses)
            .field("routes", &self.routes())
            .finish()
    }
}

fn stopped() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "a pilha de rede parou")
}

impl NetStack {
    /// Os ajustes da pilha: `NetStack::builder().tcp_buffer(…).spawn(túnel)`.
    pub fn builder() -> NetStackBuilder {
        NetStackBuilder {
            tcp_buffer: DEFAULT_TCP_BUFFER,
            #[cfg(feature = "test-util")]
            any_ip: false,
        }
    }

    /// Põe a pilha para rodar sobre o túnel, no runtime atual, com os ajustes padrão.
    pub fn spawn(tunnel: impl PacketTunnel) -> NetStack {
        Self::builder().spawn(tunnel)
    }

    /// Os endereços desta ponta dentro da rede do túnel.
    pub fn addresses(&self) -> &[Cidr] {
        &self.addresses
    }

    /// As redes que o túnel alcança agora (uma cópia do momento: o túnel pode trocá-las, veja
    /// [`PacketTunnel::poll_routes`]).
    pub fn routes(&self) -> Arc<[Cidr]> {
        self.routes
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// `true` se `ip` está numa das redes do túnel.
    pub fn routes_to(&self, ip: IpAddr) -> bool {
        self.routes
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .any(|route| route.contains(ip))
    }

    /// Abre uma conexão TCP pela rede do túnel, esperando o quanto for preciso. Para limitar a
    /// espera, use [`connect_timeout`](Self::connect_timeout).
    ///
    /// Os erros vêm em português: "{addr} está fora das redes do túnel" (de tipo
    /// `NetworkUnreachable`, sem mandar pacote nenhum), "a rede recusou a conexão com {addr}"
    /// (`ConnectionRefused`, o RST do servidor) e o motivo do túnel quando o par não é
    /// alcançável (`HostUnreachable`).
    pub async fn connect(&self, addr: SocketAddr) -> io::Result<TcpStream> {
        let SocketAddr::V4(v4) = addr else {
            return Err(io::Error::new(
                io::ErrorKind::NetworkUnreachable,
                format!("{addr} está fora das redes do túnel (o IPv6 ainda não é suportado)"),
            ));
        };
        if !self.routes_to(addr.ip()) {
            return Err(io::Error::new(
                io::ErrorKind::NetworkUnreachable,
                format!("{addr} está fora das redes do túnel"),
            ));
        }
        let shared = Shared::new(addr, self.tcp_buffer, self.wake.clone());
        // A `TcpStream` nasce antes do handshake: se quem espera desistir (o tempo acabou), ela
        // é largada, e o laço aborta a conexão pendente.
        let stream = TcpStream::new(shared.clone());
        let (reply, answer) = oneshot::channel();
        self.commands
            .send(Command::Connect {
                addr: v4,
                shared,
                reply,
            })
            .map_err(|_| stopped())?;
        self.wake.notify_one();
        answer.await.map_err(|_| stopped())??;
        Ok(stream)
    }

    /// Como [`connect`](Self::connect), mas desiste depois de `limit` com "{addr} não respondeu
    /// em {s} s" (`TimedOut`).
    pub async fn connect_timeout(
        &self,
        addr: SocketAddr,
        limit: Duration,
    ) -> io::Result<TcpStream> {
        match tokio::time::timeout(limit, self.connect(addr)).await {
            Ok(result) => result,
            Err(_) => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("{addr} não respondeu em {} s", limit.as_secs().max(1)),
            )),
        }
    }

    /// Pergunta ao túnel o endereço de um nome dentro da rede. `None` se ele não sabe.
    pub async fn resolve(&self, name: &str) -> Option<Vec<IpAddr>> {
        let (reply, answer) = oneshot::channel();
        self.commands
            .send(Command::Resolve {
                name: name.to_owned(),
                reply,
            })
            .ok()?;
        self.wake.notify_one();
        answer.await.ok().flatten()
    }

    /// Fecha todas as conexões (RST) e para a pilha.
    pub async fn shutdown(self) {
        let _ = self.commands.send(Command::Shutdown);
        self.wake.notify_one();
        let task = self
            .task
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(task) = task {
            let _ = task.await;
        }
    }

    /// Aceita conexões numa porta (só nos testes: o app não escuta nada).
    #[cfg(feature = "test-util")]
    pub async fn listen(&self, port: u16) -> io::Result<Listener> {
        let (reply, answer) = oneshot::channel();
        self.commands
            .send(Command::Listen { port, reply })
            .map_err(|_| stopped())?;
        self.wake.notify_one();
        let accepted = answer.await.map_err(|_| stopped())??;
        Ok(Listener { port, accepted })
    }
}

/// Uma porta em escuta da pilha (só no `test-util`).
#[cfg(feature = "test-util")]
pub struct Listener {
    port: u16,
    accepted: mpsc::UnboundedReceiver<TcpStream>,
}

#[cfg(feature = "test-util")]
impl Listener {
    /// A porta em escuta.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Espera a próxima conexão.
    pub async fn accept(&mut self) -> io::Result<TcpStream> {
        self.accepted.recv().await.ok_or_else(stopped)
    }
}
