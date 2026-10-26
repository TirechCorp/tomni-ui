//! O laço da pilha: uma tarefa só é dona da `Interface` e do `SocketSet` do `smoltcp`. As
//! [`TcpStream`](crate::TcpStream) falam com ela pelo estado compartilhado de `stream.rs` e pelos
//! comandos de `lib.rs`.

use std::collections::HashSet;
use std::future::poll_fn;
use std::io;
#[cfg(feature = "test-util")]
use std::net::SocketAddr;
use std::net::{IpAddr, Ipv4Addr, SocketAddrV4};
use std::sync::{Arc, PoisonError, RwLock};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use smoltcp::iface::{Config, Interface, PollResult, SocketHandle, SocketSet};
use smoltcp::socket::tcp::{self, CongestionControl, SocketBuffer, State};
use smoltcp::time::{Duration as SmolDuration, Instant as SmolInstant};
use smoltcp::wire::{HardwareAddress, IpAddress, IpCidr, IpEndpoint, IpListenEndpoint};
use tokio::sync::{Notify, mpsc, oneshot};
use tokio::time::Instant;

use crate::cidr::Cidr;
use crate::device::QueueDevice;
use crate::filter::{Filter, Flow};
use crate::stream::Shared;
#[cfg(feature = "test-util")]
use crate::stream::TcpStream;
use crate::{PacketTunnel, Unreachable};

/// A faixa das portas efêmeras, a da IANA (RFC 6335): de 49152 a 65535.
const EPHEMERAL_FIRST: u16 = 49152;
const EPHEMERAL_COUNT: u32 = 65536 - EPHEMERAL_FIRST as u32;
/// Quanto uma conexão largada espera o servidor fechar o lado dele antes de ser abortada: os 60 s
/// do `tcp_fin_timeout` do Linux para o estado FIN-WAIT-2, que o TCP não limita sozinho.
const LINGER: Duration = Duration::from_secs(60);
/// Quantos pacotes do túnel se lê de uma vez antes de rodar a pilha: um lote, não um limite de
/// tráfego (o resto fica para a volta seguinte do laço).
/// De quantos em quantos segundos de silêncio a conexão manda a sonda de keep-alive (veja
/// `new_socket`). Cinco segundos é a espera máxima de quem pegou a janela zero perdida.
const KEEP_ALIVE_SECS: u64 = 5;
const INGEST_BATCH: usize = 64;

/// O que a [`NetStack`](crate::NetStack) pede ao laço.
pub(crate) enum Command {
    Connect {
        addr: SocketAddrV4,
        shared: Arc<Shared>,
        reply: oneshot::Sender<io::Result<()>>,
    },
    #[cfg(feature = "test-util")]
    Listen {
        port: u16,
        reply: oneshot::Sender<io::Result<mpsc::UnboundedReceiver<TcpStream>>>,
    },
    Resolve {
        name: String,
        reply: oneshot::Sender<Option<Vec<IpAddr>>>,
    },
    Shutdown,
}

struct Conn {
    handle: SocketHandle,
    shared: Arc<Shared>,
    remote: SocketAddrV4,
    flow: Flow,
    /// A resposta do `connect` que ainda espera o handshake do TCP.
    connect_reply: Option<oneshot::Sender<io::Result<()>>>,
    /// O servidor fechou o lado dele (FIN visto).
    fin_seen: bool,
    /// Nós mandamos o FIN.
    fin_sent: bool,
    dropped_at: Option<Instant>,
}

#[cfg(feature = "test-util")]
struct ListenerState {
    port: u16,
    /// Os sockets em escuta da porta. O `smoltcp` atende um SYN por socket em escuta, então um
    /// só socket recusaria (RST) as conexões que chegam juntas: cada porta mantém uma fila
    /// deles (o `backlog`).
    handles: Vec<SocketHandle>,
    out: mpsc::UnboundedSender<TcpStream>,
}

/// Quantos sockets ficam em escuta por porta nos testes: o `backlog` do `listen`. É o valor
/// antigo do `SOMAXCONN` do Linux (128) pela metade; só o `test-util` escuta.
#[cfg(feature = "test-util")]
const LISTEN_BACKLOG: usize = 64;

pub(crate) struct Stack {
    tunnel: Box<dyn PacketTunnel>,
    iface: Interface,
    device: QueueDevice,
    sockets: SocketSet<'static>,
    conns: Vec<Conn>,
    #[cfg(feature = "test-util")]
    listeners: Vec<ListenerState>,
    filter: Filter,
    locals: Vec<Ipv4Addr>,
    routes: Vec<Cidr>,
    /// As mesmas rotas, vistas de fora (`NetStack::routes`): trocadas quando o túnel avisa.
    shared_routes: SharedRoutes,
    wake: Arc<Notify>,
    capacity: usize,
    used_ports: HashSet<u16>,
    epoch: Instant,
}

/// As redes do túnel, compartilhadas entre a pilha (que as troca) e as alças `NetStack`.
pub(crate) type SharedRoutes = Arc<RwLock<Arc<[Cidr]>>>;

fn v4(addr: IpAddr) -> Option<Ipv4Addr> {
    match addr {
        IpAddr::V4(addr) => Some(addr),
        IpAddr::V6(_) => None,
    }
}

impl Stack {
    pub(crate) fn new(
        tunnel: Box<dyn PacketTunnel>,
        wake: Arc<Notify>,
        capacity: usize,
        shared_routes: SharedRoutes,
    ) -> Stack {
        let mut device = QueueDevice::new(tunnel.mtu());
        let epoch = Instant::now();
        let mut seed = [0u8; 8];
        let _ = getrandom::fill(&mut seed);
        let mut config = Config::new(HardwareAddress::Ip);
        config.random_seed = u64::from_le_bytes(seed);
        let mut iface = Interface::new(config, &mut device, SmolInstant::from_micros(0));
        let locals: Vec<Ipv4Addr> = tunnel
            .addresses()
            .iter()
            .filter_map(|cidr| v4(cidr.addr()))
            .collect();
        iface.update_ip_addrs(|addrs| {
            for cidr in tunnel.addresses() {
                if let Some(addr) = v4(cidr.addr()) {
                    // O limite de endereços do `smoltcp` é o dele (2 por padrão); o que passa
                    // disso fica de fora, e o primeiro (o do certificado) é o que o app usa.
                    let _ = addrs.push(IpCidr::new(IpAddress::Ipv4(addr), cidr.prefix()));
                }
            }
        });
        if let Some(first) = locals.first() {
            // Em `Medium::Ip` o próximo salto não importa (não há ARP): só precisa existir uma
            // rota para o destino fora da rede do endereço. Os destinos válidos já saíram do
            // filtro de `routes()`.
            let _ = iface.routes_mut().add_default_ipv4_route(*first);
        }
        let routes = tunnel.routes();
        Stack {
            tunnel,
            iface,
            device,
            sockets: SocketSet::new(Vec::new()),
            conns: Vec::new(),
            #[cfg(feature = "test-util")]
            listeners: Vec::new(),
            filter: Filter::default(),
            locals,
            routes,
            shared_routes,
            wake,
            capacity,
            used_ports: HashSet::new(),
            epoch,
        }
    }

    /// Liga o `any_ip` da interface (só os testes).
    #[cfg(feature = "test-util")]
    pub(crate) fn set_any_ip(&mut self, on: bool) {
        self.iface.set_any_ip(on);
        self.filter.set_any_destination(on);
    }

    fn now(&self) -> SmolInstant {
        let micros = self.epoch.elapsed().as_micros();
        SmolInstant::from_micros(i64::try_from(micros).unwrap_or(i64::MAX))
    }

    fn new_socket(&self) -> tcp::Socket<'static> {
        // Buffers de `capacity` bytes (256 KiB por padrão, o produto banda x atraso de uma
        // ligação de 20 Mbit/s com 100 ms), com a escala de janela que o `smoltcp` liga quando o
        // buffer passa de 64 KiB.
        let mut socket = tcp::Socket::new(
            SocketBuffer::new(vec![0u8; self.capacity]),
            SocketBuffer::new(vec![0u8; self.capacity]),
        );
        socket.set_congestion_control(CongestionControl::Cubic);
        // O `smoltcp` não manda sonda de janela zero: se o aviso de que a janela reabriu se
        // perde, os dois lados esperam um pelo outro para sempre. O keep-alive é a sonda: o par
        // responde com a janela de agora e a conexão destrava.
        socket.set_keep_alive(Some(SmolDuration::from_secs(KEEP_ALIVE_SECS)));
        socket
    }

    fn pick_port(&self) -> Option<u16> {
        let mut seed = [0u8; 4];
        let _ = getrandom::fill(&mut seed);
        let start = u32::from_le_bytes(seed) % EPHEMERAL_COUNT;
        (0..EPHEMERAL_COUNT)
            .map(|i| EPHEMERAL_FIRST + ((start + i) % EPHEMERAL_COUNT) as u16)
            .find(|port| !self.used_ports.contains(port))
    }

    fn connect(
        &mut self,
        remote: SocketAddrV4,
        shared: Arc<Shared>,
        reply: oneshot::Sender<io::Result<()>>,
    ) {
        let Some(local) = self.locals.first().copied() else {
            let _ = reply.send(Err(io::Error::new(
                io::ErrorKind::NetworkUnreachable,
                "o túnel não tem endereço IPv4",
            )));
            return;
        };
        let Some(port) = self.pick_port() else {
            let _ = reply.send(Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                "todas as portas locais da rede estão em uso",
            )));
            return;
        };
        let mut socket = self.new_socket();
        let result = socket.connect(
            self.iface.context(),
            IpEndpoint::new(IpAddress::Ipv4(*remote.ip()), remote.port()),
            IpListenEndpoint {
                addr: Some(IpAddress::Ipv4(local)),
                port,
            },
        );
        if let Err(error) = result {
            let _ = reply.send(Err(io::Error::other(format!(
                "não foi possível abrir a conexão com {remote}: {error}"
            ))));
            return;
        }
        let flow = Flow {
            local_port: port,
            remote: *remote.ip(),
            remote_port: remote.port(),
        };
        self.filter.allow(flow);
        self.used_ports.insert(port);
        let handle = self.sockets.add(socket);
        self.conns.push(Conn {
            handle,
            shared,
            remote,
            flow,
            connect_reply: Some(reply),
            fin_seen: false,
            fin_sent: false,
            dropped_at: None,
        });
    }

    #[cfg(feature = "test-util")]
    fn listen(
        &mut self,
        port: u16,
        reply: oneshot::Sender<io::Result<mpsc::UnboundedReceiver<TcpStream>>>,
    ) {
        if self.listeners.iter().any(|l| l.port == port) {
            let _ = reply.send(Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                format!("a porta {port} já está em escuta"),
            )));
            return;
        }
        let mut handles = Vec::with_capacity(LISTEN_BACKLOG);
        for _ in 0..LISTEN_BACKLOG {
            let mut socket = self.new_socket();
            if let Err(error) = socket.listen(port) {
                for handle in handles {
                    self.sockets.remove(handle);
                }
                let _ = reply.send(Err(io::Error::other(format!(
                    "não foi possível escutar a porta {port}: {error}"
                ))));
                return;
            }
            handles.push(self.sockets.add(socket));
        }
        let (out, receiver) = mpsc::unbounded_channel();
        self.filter.listen(port);
        self.used_ports.insert(port);
        self.listeners.push(ListenerState { port, handles, out });
        let _ = reply.send(Ok(receiver));
    }

    /// Entrega ao `smoltcp` o pacote que chegou do túnel, se o filtro deixar.
    fn ingest(&mut self, packet: &mut Vec<u8>) {
        if self.filter.admit(packet, &self.locals) {
            self.device.rx.push_back(std::mem::take(packet));
        }
    }

    /// Troca as redes alcançáveis (o túnel avisou por `poll_routes`). As conexões que já estão
    /// abertas seguem: só o que sai daqui para frente passa pelo filtro novo.
    fn replace_routes(&mut self, routes: Vec<Cidr>) {
        *self
            .shared_routes
            .write()
            .unwrap_or_else(PoisonError::into_inner) = routes.as_slice().into();
        self.routes = routes;
    }

    /// Passa para o túnel os pacotes que a pilha montou (só para as `routes()`).
    fn flush(&mut self) {
        for packet in std::mem::take(&mut self.device.tx) {
            let to_route = packet
                .get(16..20)
                .map(|b| IpAddr::V4(Ipv4Addr::new(b[0], b[1], b[2], b[3])));
            if let Some(dst) = to_route
                && (self.routes.iter().any(|r| r.contains(dst))
                    || self
                        .locals
                        .contains(&v4(dst).unwrap_or(Ipv4Addr::UNSPECIFIED)))
            {
                self.tunnel.send(&packet);
            }
        }
    }

    /// Roda a pilha até ela parar de ter o que fazer: o `smoltcp`, a passagem de dados entre os
    /// sockets e as `TcpStream` e a saída para o túnel.
    pub(crate) fn run_iface(&mut self) {
        loop {
            let now = self.now();
            let changed = self.iface.poll(now, &mut self.device, &mut self.sockets);
            self.flush();
            let moved = self.pump();
            if changed == PollResult::None && !moved {
                break;
            }
        }
    }

    /// Move dados e estados entre os sockets e as `TcpStream`. `true` se mexeu em algum buffer.
    fn pump(&mut self) -> bool {
        let mut progress = false;
        let conns = std::mem::take(&mut self.conns);
        let mut kept = Vec::with_capacity(conns.len());
        for mut conn in conns {
            let socket = self.sockets.get_mut::<tcp::Socket>(conn.handle);
            if conn.pump(socket, &mut progress) {
                self.sockets.remove(conn.handle);
                self.filter.forget(&conn.flow);
                self.used_ports.remove(&conn.flow.local_port);
                progress = true;
            } else {
                kept.push(conn);
            }
        }
        self.conns = kept;
        #[cfg(feature = "test-util")]
        {
            progress |= self.pump_listeners();
        }
        progress
    }

    /// Entrega ao `accept` as conexões que chegaram nas portas em escuta e põe um socket novo em
    /// escuta no lugar de cada uma.
    #[cfg(feature = "test-util")]
    fn pump_listeners(&mut self) -> bool {
        let mut progress = false;
        let mut index = 0;
        while index < self.listeners.len() {
            let port = self.listeners[index].port;
            if self.listeners[index].out.is_closed() {
                for handle in std::mem::take(&mut self.listeners[index].handles) {
                    self.sockets.remove(handle);
                }
                self.filter.unlisten(port);
                self.used_ports.remove(&port);
                self.listeners.remove(index);
                continue;
            }
            for slot in 0..self.listeners[index].handles.len() {
                let handle = self.listeners[index].handles[slot];
                let socket = self.sockets.get_mut::<tcp::Socket>(handle);
                match socket.state() {
                    State::Listen => {}
                    State::Closed => {
                        let _ = socket.listen(port);
                    }
                    _ => {
                        let Some(remote) = socket.remote_endpoint() else {
                            continue;
                        };
                        // Só o IPv4 está compilado no `smoltcp` (o IPv6 é adiável).
                        let IpAddress::Ipv4(addr) = remote.addr;
                        let remote = SocketAddrV4::new(addr, remote.port);
                        let shared =
                            Shared::new(SocketAddr::V4(remote), self.capacity, self.wake.clone());
                        let stream = TcpStream::new(shared.clone());
                        let flow = Flow {
                            local_port: port,
                            remote: *remote.ip(),
                            remote_port: remote.port(),
                        };
                        // O socket que recebeu o SYN vira a conexão; um novo fica em escuta.
                        let mut fresh = self.new_socket();
                        let _ = fresh.listen(port);
                        self.listeners[index].handles[slot] = self.sockets.add(fresh);
                        self.conns.push(Conn {
                            handle,
                            shared,
                            remote,
                            flow,
                            connect_reply: None,
                            fin_seen: false,
                            fin_sent: false,
                            dropped_at: None,
                        });
                        let _ = self.listeners[index].out.send(stream);
                        progress = true;
                    }
                }
            }
            index += 1;
        }
        progress
    }

    /// Falha as conexões que esperam o handshake com `unreachable.addr`.
    fn unreachable(&mut self, event: Unreachable) {
        let IpAddr::V4(addr) = event.addr else {
            return;
        };
        let conns = std::mem::take(&mut self.conns);
        for mut conn in conns {
            if *conn.remote.ip() == addr && conn.connect_reply.is_some() {
                let error = io::Error::new(io::ErrorKind::HostUnreachable, event.reason.clone());
                if let Some(reply) = conn.connect_reply.take() {
                    let _ = reply.send(Err(error));
                }
                self.sockets.get_mut::<tcp::Socket>(conn.handle).abort();
                self.sockets.remove(conn.handle);
                self.filter.forget(&conn.flow);
                self.used_ports.remove(&conn.flow.local_port);
            } else {
                self.conns.push(conn);
            }
        }
    }

    /// Aborta tudo (RST para o outro lado) e avisa as `TcpStream`.
    fn abort_all(&mut self, message: &str) {
        for conn in &mut self.conns {
            self.sockets.get_mut::<tcp::Socket>(conn.handle).abort();
            if let Some(reply) = conn.connect_reply.take() {
                let _ = reply.send(Err(io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    message,
                )));
            }
        }
        let now = self.now();
        let _ = self.iface.poll(now, &mut self.device, &mut self.sockets);
        self.flush();
        for conn in &self.conns {
            conn.shared
                .lock()
                .fail(io::ErrorKind::ConnectionAborted, message);
        }
        self.conns.clear();
    }

    /// Quando a pilha precisa acordar sozinha: o timer do `smoltcp` e as conexões largadas.
    fn next_delay(&mut self) -> Option<Duration> {
        let now = self.now();
        let mut delay = self
            .iface
            .poll_delay(now, &self.sockets)
            .map(|d| Duration::from_micros(d.total_micros()));
        let lingering = self
            .conns
            .iter()
            .filter_map(|c| c.dropped_at)
            .map(|at| (at + LINGER).saturating_duration_since(Instant::now()))
            .min();
        if let Some(l) = lingering {
            delay = Some(delay.map_or(l, |d| d.min(l)));
        }
        delay
    }

    fn command(&mut self, command: Command) -> bool {
        match command {
            Command::Connect {
                addr,
                shared,
                reply,
            } => self.connect(addr, shared, reply),
            #[cfg(feature = "test-util")]
            Command::Listen { port, reply } => self.listen(port, reply),
            Command::Resolve { name, reply } => {
                let _ = reply.send(self.tunnel.resolve(&name));
            }
            Command::Shutdown => return false,
        }
        true
    }

    /// O laço. Termina no `Shutdown`, quando todos os `NetStack` somem ou quando o túnel acaba.
    pub(crate) async fn run(mut self, mut commands: mpsc::UnboundedReceiver<Command>) {
        let wake = self.wake.clone();
        let mut buffer = Vec::new();
        let message = loop {
            self.run_iface();
            let delay = self.next_delay();
            let timer = async {
                match delay {
                    Some(delay) => tokio::time::sleep(delay).await,
                    None => std::future::pending().await,
                }
            };
            let tunnel = &mut self.tunnel;
            let incoming = poll_fn(|cx| {
                // As rotas mudam raramente: vêm antes dos pacotes para o tráfego não as atrasar.
                if let Poll::Ready(routes) = tunnel.poll_routes(cx) {
                    return Poll::Ready(Event::Routes(routes));
                }
                if let Poll::Ready(result) = tunnel.poll_recv(cx, &mut buffer) {
                    return Poll::Ready(Event::Packet(result));
                }
                if let Poll::Ready(event) = tunnel.poll_unreachable(cx) {
                    return Poll::Ready(Event::Unreachable(event));
                }
                Poll::Pending
            });
            let event = tokio::select! {
                biased;
                event = incoming => event,
                command = commands.recv() => match command {
                    Some(command) => Event::Command(command),
                    None => Event::Command(Command::Shutdown),
                },
                () = wake.notified() => Event::Wake,
                () = timer => Event::Wake,
            };
            match event {
                Event::Packet(Ok(())) => {
                    self.ingest(&mut buffer);
                    let mut cx = Context::from_waker(Waker::noop());
                    for _ in 1..INGEST_BATCH {
                        match self.tunnel.poll_recv(&mut cx, &mut buffer) {
                            Poll::Ready(Ok(())) => self.ingest(&mut buffer),
                            _ => break,
                        }
                    }
                }
                Event::Packet(Err(error)) => {
                    break format!("o túnel da rede parou: {error}");
                }
                Event::Unreachable(event) => self.unreachable(event),
                Event::Routes(routes) => self.replace_routes(routes),
                Event::Command(command) => {
                    if !self.command(command) {
                        break "a pilha de rede foi desligada".to_owned();
                    }
                    // Os outros comandos que já esperam na fila.
                    while let Ok(command) = commands.try_recv() {
                        if !self.command(command) {
                            self.abort_all("a pilha de rede foi desligada");
                            return;
                        }
                    }
                }
                Event::Wake => {}
            }
        };
        self.abort_all(&message);
        // Quem mandar comando depois disto recebe o erro de canal fechado.
        commands.close();
        while let Ok(command) = commands.try_recv() {
            match command {
                Command::Connect { reply, .. } => {
                    let _ = reply.send(Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "a pilha de rede parou",
                    )));
                }
                Command::Resolve { reply, .. } => {
                    let _ = reply.send(None);
                }
                #[cfg(feature = "test-util")]
                Command::Listen { reply, .. } => {
                    let _ = reply.send(Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "a pilha de rede parou",
                    )));
                }
                Command::Shutdown => {}
            }
        }
    }
}

enum Event {
    Packet(io::Result<()>),
    Unreachable(Unreachable),
    Routes(Vec<Cidr>),
    Command(Command),
    Wake,
}

impl Conn {
    /// Passa dados e estados entre o socket e a `TcpStream`. `true` se a conexão acabou e o
    /// socket pode ser removido.
    fn pump(&mut self, socket: &mut tcp::Socket<'static>, progress: &mut bool) -> bool {
        let state = socket.state();
        if self.connect_reply.is_some() {
            match state {
                State::SynSent | State::SynReceived => {}
                State::Closed => {
                    let message = format!("a rede recusou a conexão com {}", self.remote);
                    if let Some(reply) = self.connect_reply.take() {
                        let _ = reply.send(Err(io::Error::new(
                            io::ErrorKind::ConnectionRefused,
                            message.clone(),
                        )));
                    }
                    self.shared
                        .lock()
                        .fail(io::ErrorKind::ConnectionRefused, message);
                    return true;
                }
                _ => {
                    if let Some(reply) = self.connect_reply.take() {
                        let _ = reply.send(Ok(()));
                    }
                }
            }
        }
        let mut inner = self.shared.lock();
        if inner.dropped && self.dropped_at.is_none() {
            self.dropped_at = Some(Instant::now());
        }
        if inner.dropped && matches!(state, State::SynSent | State::SynReceived) {
            socket.abort();
            return true;
        }

        // Escrita: do buffer de preparo para o socket.
        if !inner.tx.is_empty() {
            let before = inner.tx.len();
            while socket.can_send() && !inner.tx.is_empty() {
                let sent = socket.send_slice(inner.tx.as_slices().0);
                match sent {
                    Ok(n) if n > 0 => {
                        inner.tx.drain(..n);
                        *progress = true;
                    }
                    _ => break,
                }
            }
            if inner.tx.len() < before {
                inner.wake_writer();
            }
        }
        // O FIN, depois de mandar tudo.
        // O FIN só sai com tudo enviado **e confirmado** (`send_queue() == 0`): o `smoltcp` 0.12
        // desliga o timer de retransmissão a cada ACK em LAST-ACK, CLOSING e CLOSE-WAIT, e dado
        // perdido depois do FIN nunca seria reenviado (a conexão pararia para sempre).
        if (inner.write_shutdown || inner.dropped)
            && inner.tx.is_empty()
            && socket.send_queue() == 0
            && !self.fin_sent
            && matches!(state, State::Established | State::CloseWait)
        {
            socket.close();
            self.fin_sent = true;
            inner.write_closed = true;
            inner.wake_writer();
            *progress = true;
        }
        // Leitura: do socket para o buffer de preparo (ou para o lixo, se largada).
        if inner.dropped {
            while socket.can_recv() {
                let _ = socket.recv(|data| (data.len(), ()));
                *progress = true;
            }
        } else {
            let room = self.shared.capacity().saturating_sub(inner.rx.len());
            if room > 0 && socket.can_recv() {
                let received = socket.recv(|data| {
                    let n = data.len().min(room);
                    inner.rx.extend(&data[..n]);
                    (n, n)
                });
                if let Ok(n) = received
                    && n > 0
                {
                    *progress = true;
                    inner.wake_reader();
                }
            }
        }
        if matches!(
            state,
            State::CloseWait | State::LastAck | State::Closing | State::TimeWait
        ) {
            self.fin_seen = true;
        }
        let socket_done = !matches!(state, State::SynSent | State::SynReceived | State::Listen)
            && !socket.may_recv()
            && socket.recv_queue() == 0;
        if socket_done {
            if state == State::Closed && !self.fin_seen && !self.fin_sent {
                inner.fail(
                    io::ErrorKind::ConnectionReset,
                    "conexão interrompida pelo outro lado",
                );
            } else if !inner.rx_eof {
                inner.rx_eof = true;
                inner.wake_reader();
            }
        }
        let finished = matches!(state, State::Closed | State::TimeWait)
            && (socket.recv_queue() == 0 || inner.dropped);
        if finished {
            inner.closed = true;
            inner.rx_eof = true;
            inner.wake_reader();
            inner.wake_writer();
            return true;
        }
        if let Some(at) = self.dropped_at
            && at.elapsed() >= LINGER
        {
            socket.abort();
            return true;
        }
        false
    }
}
