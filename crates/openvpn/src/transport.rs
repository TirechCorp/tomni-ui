//! O transporte do motor: o enlace com o servidor ([`Link`]) sobre UDP ou TCP e quem o abre
//! ([`Connector`]). O UDP leva um datagrama por mensagem; o TCP leva o prefixo de 2 bytes do
//! OpenVPN em cada uma e junta os pedaços na leitura.
//!
//! O enlace é *poll*: o motor o espera junto com os timers num `select!`. Escrever nunca
//! bloqueia: no TCP, o que não coube no socket fica num buffer limitado (o que passar do limite
//! é descartado, e o TCP de dentro do túnel retransmite); no UDP, o datagrama que não saiu na
//! hora se perde, como numa rede com perda.

use std::future::Future;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::task::{Context, Poll, ready};
use std::time::Duration;

use tokio::io::{AsyncWrite, ReadBuf};
use tokio::net::{TcpStream, UdpSocket};
use tomni_ui_net::Proxy;

use crate::config::{Family, Proto, Remote};
use crate::packet::{TcpDeframer, tcp_frame};

/// O quanto o buffer de escrita do TCP guarda antes de descartar pacotes: 1 MiB.
const TCP_WRITE_LIMIT: usize = 1 << 20;
/// O maior datagrama UDP que se lê.
const MAX_DATAGRAM: usize = 65_535;

/// O enlace com o servidor: leva e traz os datagramas do OpenVPN (já sem o prefixo do TCP).
pub trait Link: Send + 'static {
    /// Enfileira um datagrama para o servidor. Não bloqueia. `Err` quer dizer que o enlace
    /// acabou.
    fn send(&mut self, datagram: &[u8]) -> io::Result<()>;

    /// Espera o próximo datagrama que chega: limpa `buf` e o enche com ele. Também empurra para
    /// o socket o que [`send`](Link::send) enfileirou. `Err` quer dizer que o enlace acabou.
    fn poll_recv(&mut self, cx: &mut Context<'_>, buf: &mut Vec<u8>) -> Poll<io::Result<()>>;

    /// Termina de escrever o que está na fila (a saída do motor espera isto por pouco tempo).
    fn poll_flush(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>>;
}

/// O que o [`Connector`] precisa para abrir o enlace com um servidor.
#[derive(Debug, Clone, Copy)]
pub struct Target<'a> {
    /// O servidor (nome ou IP, porta, protocolo e família).
    pub remote: &'a Remote,
    /// A porta local do UDP (`lport`).
    pub local_port: Option<u16>,
    /// Quanto esperar para o TCP (e o proxy) chegarem ao servidor.
    pub timeout: Duration,
    /// O proxy do transporte TCP (o "Salto/Proxy" do perfil). O UDP não usa proxy.
    pub proxy: Option<&'a Proxy>,
}

/// O que o [`Connector::connect`] devolve: o enlace aberto, ou o erro.
pub type Connecting<'a> = Pin<Box<dyn Future<Output = io::Result<Box<dyn Link>>> + Send + 'a>>;

/// Abre o enlace com um servidor. O motor chama a cada tentativa, inclusive nas reconexões.
pub trait Connector: Send + 'static {
    /// Abre o enlace. O erro vai em português e entra no estado "Reconectando".
    fn connect<'a>(&'a mut self, target: Target<'a>) -> Connecting<'a>;
}

/// O conector de verdade: resolve o nome pelo sistema (o servidor está fora da VPN) e abre o
/// socket UDP ou TCP.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemConnector;

impl Connector for SystemConnector {
    fn connect<'a>(&'a mut self, target: Target<'a>) -> Connecting<'a> {
        Box::pin(async move {
            match target.remote.proto {
                Proto::Udp => {
                    let link = UdpLink::open(target).await?;
                    Ok(Box::new(link) as Box<dyn Link>)
                }
                Proto::Tcp => {
                    let link = TcpLink::open(target).await?;
                    Ok(Box::new(link) as Box<dyn Link>)
                }
            }
        })
    }
}

fn not_found(host: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        format!("servidor não encontrado: {host}"),
    )
}

/// O enlace UDP: um socket ligado ao servidor (só vêm dele os datagramas).
pub struct UdpLink {
    socket: UdpSocket,
}

impl UdpLink {
    async fn open(target: Target<'_>) -> io::Result<UdpLink> {
        let remote = target.remote;
        let wants = |ip: &IpAddr| match remote.family {
            Family::Any => true,
            Family::V4 => ip.is_ipv4(),
            Family::V6 => ip.is_ipv6(),
        };
        let mut addresses: Vec<SocketAddr> =
            tokio::net::lookup_host((remote.host.as_str(), remote.port))
                .await
                .map_err(|_| not_found(&remote.host))?
                .filter(|addr| wants(&addr.ip()))
                .collect();
        // IPv4 primeiro: é o que os servidores OpenVPN escutam na maioria dos casos.
        addresses.sort_by_key(|addr| addr.is_ipv6());
        let mut last = None;
        for address in addresses {
            let local: SocketAddr = if address.is_ipv4() {
                (
                    std::net::Ipv4Addr::UNSPECIFIED,
                    target.local_port.unwrap_or(0),
                )
                    .into()
            } else {
                (
                    std::net::Ipv6Addr::UNSPECIFIED,
                    target.local_port.unwrap_or(0),
                )
                    .into()
            };
            let attempt = async {
                let socket = UdpSocket::bind(local).await?;
                socket.connect(address).await?;
                io::Result::Ok(socket)
            };
            match attempt.await {
                Ok(socket) => return Ok(UdpLink { socket }),
                Err(error) => last = Some(error),
            }
        }
        Err(last.unwrap_or_else(|| not_found(&remote.host)))
    }
}

impl Link for UdpLink {
    fn send(&mut self, datagram: &[u8]) -> io::Result<()> {
        match self.socket.try_send(datagram) {
            Ok(_) => Ok(()),
            // Sem lugar no socket, ou o ICMP "porta inalcançável" da tentativa anterior: o
            // datagrama se perde, e a retransmissão e o `ping-restart` cuidam do resto.
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::ConnectionRefused
                ) =>
            {
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    fn poll_recv(&mut self, cx: &mut Context<'_>, buf: &mut Vec<u8>) -> Poll<io::Result<()>> {
        buf.clear();
        buf.resize(MAX_DATAGRAM, 0);
        loop {
            let mut read = ReadBuf::new(buf);
            match self.socket.poll_recv(cx, &mut read) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Ok(())) => {
                    let len = read.filled().len();
                    buf.truncate(len);
                    return Poll::Ready(Ok(()));
                }
                Poll::Ready(Err(error)) if error.kind() == io::ErrorKind::ConnectionRefused => {
                    continue;
                }
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
            }
        }
    }

    fn poll_flush(&mut self, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

/// O enlace TCP: cada datagrama leva o prefixo de 2 bytes.
pub struct TcpLink {
    stream: TcpStream,
    deframer: TcpDeframer,
    outgoing: Vec<u8>,
    written: usize,
}

impl TcpLink {
    async fn open(target: Target<'_>) -> io::Result<TcpLink> {
        let host = target.remote.host.clone();
        let port = target.remote.port;
        let proxy = target.proxy.cloned();
        let timeout = target.timeout;
        // A conexão da lib (com o proxy e as mensagens em português) é síncrona.
        let std_stream = tokio::task::spawn_blocking(move || {
            tomni_ui_net::connect(&host, port, proxy.as_ref(), timeout)
        })
        .await
        .map_err(|_| io::Error::other("a conexão foi interrompida"))?
        .map_err(|error| {
            io::Error::new(error.kind(), tomni_ui_net::connect_error_message(&error))
        })?;
        std_stream.set_nonblocking(true)?;
        let _ = std_stream.set_nodelay(true);
        Ok(TcpLink::new(TcpStream::from_std(std_stream)?))
    }

    /// Um enlace sobre uma conexão TCP já aberta (os testes).
    pub fn new(stream: TcpStream) -> TcpLink {
        TcpLink {
            stream,
            deframer: TcpDeframer::new(),
            outgoing: Vec::new(),
            written: 0,
        }
    }

    fn poll_write_out(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        while self.written < self.outgoing.len() {
            let n =
                ready!(Pin::new(&mut self.stream).poll_write(cx, &self.outgoing[self.written..]))?;
            if n == 0 {
                return Poll::Ready(Err(io::ErrorKind::WriteZero.into()));
            }
            self.written += n;
        }
        self.outgoing.clear();
        self.written = 0;
        Poll::Ready(Ok(()))
    }
}

impl Link for TcpLink {
    fn send(&mut self, datagram: &[u8]) -> io::Result<()> {
        if self.outgoing.len() - self.written > TCP_WRITE_LIMIT {
            return Ok(());
        }
        if let Some(framed) = tcp_frame(datagram) {
            self.outgoing.extend_from_slice(&framed);
        }
        Ok(())
    }

    fn poll_recv(&mut self, cx: &mut Context<'_>, buf: &mut Vec<u8>) -> Poll<io::Result<()>> {
        if let Poll::Ready(Err(error)) = self.poll_write_out(cx) {
            return Poll::Ready(Err(error));
        }
        let mut chunk = [0u8; 16 * 1024];
        loop {
            if let Some(packet) = self.deframer.next_packet() {
                *buf = packet;
                return Poll::Ready(Ok(()));
            }
            let mut read = ReadBuf::new(&mut chunk);
            ready!(tokio::io::AsyncRead::poll_read(
                Pin::new(&mut self.stream),
                cx,
                &mut read
            ))?;
            if read.filled().is_empty() {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "o servidor fechou a conexão TCP",
                )));
            }
            self.deframer.push(read.filled());
        }
    }

    fn poll_flush(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.poll_write_out(cx)
    }
}

/// Um enlace em memória, que liga duas pontas por canais (os testes do motor).
#[cfg(feature = "test-util")]
pub struct MemoryLink {
    to_peer: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
    from_peer: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
}

#[cfg(feature = "test-util")]
impl MemoryLink {
    /// As duas pontas de um enlace.
    pub fn pair() -> (MemoryLink, MemoryLink) {
        let (a_tx, a_rx) = tokio::sync::mpsc::unbounded_channel();
        let (b_tx, b_rx) = tokio::sync::mpsc::unbounded_channel();
        (
            MemoryLink {
                to_peer: a_tx,
                from_peer: b_rx,
            },
            MemoryLink {
                to_peer: b_tx,
                from_peer: a_rx,
            },
        )
    }

    /// Espera o próximo datagrama (`None` quando a outra ponta foi largada).
    pub async fn recv(&mut self) -> Option<Vec<u8>> {
        self.from_peer.recv().await
    }

    /// Envia um datagrama (sem efeito se a outra ponta foi largada).
    pub fn push(&self, datagram: Vec<u8>) {
        let _ = self.to_peer.send(datagram);
    }
}

#[cfg(feature = "test-util")]
impl Link for MemoryLink {
    fn send(&mut self, datagram: &[u8]) -> io::Result<()> {
        self.to_peer
            .send(datagram.to_vec())
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "a outra ponta fechou"))
    }

    fn poll_recv(&mut self, cx: &mut Context<'_>, buf: &mut Vec<u8>) -> Poll<io::Result<()>> {
        match ready!(self.from_peer.poll_recv(cx)) {
            Some(packet) => {
                *buf = packet;
                Poll::Ready(Ok(()))
            }
            None => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "a outra ponta fechou",
            ))),
        }
    }

    fn poll_flush(&mut self, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}
