//! O motor: liga o [`Device`] a um UDP ([`Underlay`]) e à pilha TCP/IP em software
//! ([`NetStack`]), e abre a ponte SOCKS5 interna que os clientes do app usam.
//!
//! Nada aqui cria interface de rede, mexe em rota do sistema ou pede administrador: os pacotes
//! IP saem da pilha, são cifrados pelo [`Device`] e viajam num datagrama UDP comum. A ponte
//! SOCKS5 fica em `127.0.0.1`, com porta e credencial aleatórias, e só os clientes do próprio
//! app a conhecem.

use std::collections::HashSet;
use std::fmt;
use std::future::poll_fn;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tomni_ui_net::Proxy;
use tomni_ui_netstack::{
    Cidr as NetCidr, NetStack, PacketTunnel, SocksAccess, SocksBridge, UdpUnderlay, Underlay,
    Unreachable,
};

use crate::conf::{Endpoint, InterfaceAddress, WgConfig};
use crate::device::{Device, Event, SendError, destino_ip};
use crate::key::PublicKey;
use crate::peer::{PeerConfig, PeerError};
use crate::router::Cidr;
use crate::timers::REKEY_ATTEMPT_TIME;

/// De quanto em quanto tempo os relógios do WireGuard rodam. É a menor resolução que os tempos
/// do protocolo pedem: a menor variação do `REKEY_TIMEOUT` é de 333 ms, então 100 ms basta.
const TIQUE: Duration = Duration::from_millis(100);

/// Quanto tempo se espera pela resolução de um nome de `Endpoint`.
const TEMPO_RESOLUCAO: Duration = Duration::from_secs(5);

/// De quanto em quanto tempo o nome de um `Endpoint` é resolvido de novo enquanto o handshake
/// não completa (acompanha DNS dinâmico).
const INTERVALO_RESOLUCAO: Duration = Duration::from_secs(10);

/// Quantos pacotes IP esperam em cada sentido entre a pilha e o motor.
const FILA: usize = 1024;

/// O estado do túnel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// Subindo: ainda nenhum par completou o handshake.
    Connecting,
    /// Pelo menos um par completou o handshake.
    Connected {
        /// O endereço IPv4 do nó dentro da rede.
        address: IpAddr,
        /// Quantos pares têm uma sessão agora.
        peers_up: usize,
        /// Quantos pares o `.conf` tem.
        peers: usize,
    },
    /// Sem sessão com ninguém e tentando de novo; o texto diz por quê.
    Retrying(String),
    /// Parou por um erro que não passa sozinho.
    Failed(String),
    /// Parado (por [`WireGuard::stop`]).
    Stopped,
}

impl State {
    /// O endereço do nó na rede, se está conectado.
    pub fn address(&self) -> Option<IpAddr> {
        match self {
            State::Connected { address, .. } => Some(*address),
            _ => None,
        }
    }
}

/// O que se sabe de um par.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerInfo {
    /// A chave pública do par.
    pub public_key: PublicKey,
    /// O endereço para onde os datagramas vão agora.
    pub endpoint: Option<SocketAddr>,
    /// Há quanto tempo o último handshake completou.
    pub last_handshake: Option<Duration>,
    /// Se há uma sessão que serve para mandar dados.
    pub connected: bool,
    /// Bytes de dados enviados ao par.
    pub tx_bytes: u64,
    /// Bytes de dados recebidos do par.
    pub rx_bytes: u64,
}

enum Command {
    Stop(oneshot::Sender<()>),
    Peers(oneshot::Sender<Vec<PeerInfo>>),
}

/// O túnel de pacotes IP entre a pilha TCP e o motor.
struct WireGuardTunnel {
    mtu: usize,
    addresses: Vec<NetCidr>,
    routes: Vec<NetCidr>,
    to_engine: mpsc::Sender<Vec<u8>>,
    from_engine: mpsc::Receiver<Vec<u8>>,
    unreachable: mpsc::UnboundedReceiver<Unreachable>,
}

impl PacketTunnel for WireGuardTunnel {
    fn mtu(&self) -> usize {
        self.mtu
    }

    fn addresses(&self) -> Vec<NetCidr> {
        self.addresses.clone()
    }

    fn routes(&self) -> Vec<NetCidr> {
        self.routes.clone()
    }

    fn send(&mut self, packet: &[u8]) {
        // Fila cheia = pacote perdido, como numa placa de rede; o TCP retransmite.
        let _ = self.to_engine.try_send(packet.to_vec());
    }

    fn poll_recv(&mut self, cx: &mut Context<'_>, buf: &mut Vec<u8>) -> Poll<io::Result<()>> {
        match self.from_engine.poll_recv(cx) {
            Poll::Ready(Some(pacote)) => {
                buf.clear();
                buf.extend_from_slice(&pacote);
                Poll::Ready(Ok(()))
            }
            Poll::Ready(None) => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "a rede WireGuard parou",
            ))),
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_unreachable(&mut self, cx: &mut Context<'_>) -> Poll<Unreachable> {
        match self.unreachable.poll_recv(cx) {
            Poll::Ready(Some(aviso)) => Poll::Ready(aviso),
            Poll::Ready(None) | Poll::Pending => Poll::Pending,
        }
    }
}

/// Monta e inicia um [`WireGuard`].
pub struct WireGuardBuilder {
    config: WgConfig,
    notifier: Option<Arc<dyn Fn(State) + Send + Sync>>,
}

impl WireGuardBuilder {
    /// Começa com o `.conf` já lido (veja [`WgConfig::from_conf`]).
    pub fn new(config: WgConfig) -> WireGuardBuilder {
        WireGuardBuilder {
            config,
            notifier: None,
        }
    }

    /// Chama `f` a cada mudança de [`State`], de qualquer thread. Não bloqueie dentro dela.
    #[must_use]
    pub fn on_state(mut self, f: impl Fn(State) + Send + Sync + 'static) -> WireGuardBuilder {
        self.notifier = Some(Arc::new(f));
        self
    }

    /// Abre um UDP numa porta aleatória (o `ListenPort` do `.conf` é ignorado), a ponte SOCKS5
    /// interna e inicia o túnel, no runtime atual.
    ///
    /// # Errors
    ///
    /// Se o `.conf` não passa em [`WgConfig::check`] ou o UDP não abre. A mensagem é em
    /// português.
    pub async fn start(self) -> io::Result<WireGuard> {
        let (under, aviso) = UdpUnderlay::bind("0.0.0.0", 0).await?;
        let mut wg = self.start_on(under).await?;
        wg.warnings.extend(aviso);
        Ok(wg)
    }

    /// Como [`start`](WireGuardBuilder::start), mas sobre um UDP dado (os testes usam a rede
    /// simulada do `tomni-ui-netstack`).
    ///
    /// # Errors
    ///
    /// Os mesmos de [`start`](WireGuardBuilder::start).
    pub async fn start_on<U: Underlay>(self, under: U) -> io::Result<WireGuard> {
        let config = self.config;
        config
            .check()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;
        let agora = Instant::now();
        let mut device = Device::new(config.interface.private_key.clone(), agora)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;
        let public_key = device.public_key();
        let mtu = usize::from(config.interface.mtu);

        let mut pares = Vec::new();
        let mut rotas_wg: Vec<Cidr> = Vec::new();
        let mut avisos = Vec::new();
        for par in &config.peers {
            let mut cfg = PeerConfig::new(par.public_key);
            cfg.mtu = mtu;
            if let Some(psk) = &par.preshared_key {
                cfg = cfg.with_preshared_key(psk.clone());
            }
            if let Some(k) = par.persistent_keepalive {
                cfg = cfg.with_persistent_keepalive_secs(k);
            }
            let mut endpoint = None;
            if let Some(Endpoint::Addr(a)) = &par.endpoint {
                endpoint = Some(*a);
            }
            device
                .add_peer(cfg, endpoint, &par.allowed_ips, agora)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;
            rotas_wg.extend(par.allowed_ips.iter().copied());
            pares.push(Vaga {
                chave: par.public_key,
                nome: par.endpoint.clone(),
                ultima_resolucao: None,
                resolvendo: false,
                pendentes: HashSet::new(),
            });
        }

        // Os nomes dos Endpoints são resolvidos já na partida; uma falha não impede de subir
        // (o DNS pode voltar), vira o estado `Retrying` e é tentada de novo.
        let mut falha: Option<String> = None;
        for vaga in &mut pares {
            if let Some(Endpoint::Host { host, port }) = &vaga.nome {
                vaga.ultima_resolucao = Some(agora);
                match resolver(host, *port).await {
                    Ok(addr) => device.set_endpoint(&vaga.chave, Some(addr)),
                    Err(e) => {
                        avisos.push(e.clone());
                        falha = Some(e);
                    }
                }
            }
            if device.endpoint(&vaga.chave).is_some()
                && let Some(p) = device.peer_mut(&vaga.chave)
            {
                p.connect();
            }
        }

        // O que a pilha enxerga: só IPv4 (a pilha ainda não tem IPv6).
        let addresses: Vec<NetCidr> = config
            .interface
            .addresses
            .iter()
            .filter(|a| a.ip.is_ipv4())
            .filter_map(|a| NetCidr::new(a.ip, a.prefix))
            .collect();
        let mut routes: Vec<NetCidr> = Vec::new();
        for faixa in &rotas_wg {
            if let Some(c) = NetCidr::new(faixa.addr(), faixa.prefix())
                && faixa.addr().is_ipv4()
                && !routes.contains(&c)
            {
                routes.push(c);
            }
        }
        let address = config
            .interface
            .addresses
            .iter()
            .map(|a| a.ip)
            .find(IpAddr::is_ipv4)
            .expect("o `check` garante um endereço IPv4");

        let local_addr = under.local_addr()?;
        let (to_engine_tx, to_engine_rx) = mpsc::channel(FILA);
        let (from_engine_tx, from_engine_rx) = mpsc::channel(FILA);
        let (unreachable_tx, unreachable_rx) = mpsc::unbounded_channel();
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (state_tx, state_rx) = watch::channel(State::Connecting);

        let mut motor = Motor {
            device,
            under: Arc::new(under),
            pares,
            to_stack: from_engine_tx,
            unreachable: unreachable_tx,
            state: state_tx,
            notifier: self.notifier,
            address,
            total: config.peers.len(),
            ja_conectou: false,
            falha,
            resolvidos: None,
        };
        if let Some(f) = motor.falha.clone() {
            motor.mudar_estado(State::Retrying(f));
        }
        let task = tokio::spawn(motor.run(to_engine_rx, cmd_rx));

        let tunnel = WireGuardTunnel {
            mtu,
            addresses,
            routes,
            to_engine: to_engine_tx,
            from_engine: from_engine_rx,
            unreachable: unreachable_rx,
        };
        let stack = NetStack::spawn(tunnel);
        let bridge = SocksBridge::start(stack.clone()).await?;
        Ok(WireGuard {
            commands: cmd_tx,
            state: state_rx,
            task: Some(task),
            stack,
            bridge: Some(bridge),
            local_addr,
            public_key,
            warnings: avisos,
            routes: rotas_wg,
            addresses: config.interface.addresses.clone(),
        })
    }
}

/// Um túnel WireGuard em funcionamento, só para o tráfego do app.
///
/// Ao ser largado (ou em [`stop`](WireGuard::stop)), para o motor.
pub struct WireGuard {
    commands: mpsc::UnboundedSender<Command>,
    state: watch::Receiver<State>,
    task: Option<JoinHandle<()>>,
    stack: NetStack,
    bridge: Option<SocksBridge>,
    local_addr: SocketAddr,
    public_key: PublicKey,
    warnings: Vec<String>,
    routes: Vec<Cidr>,
    addresses: Vec<InterfaceAddress>,
}

impl fmt::Debug for WireGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WireGuard")
            .field("state", &*self.state.borrow())
            .finish_non_exhaustive()
    }
}

impl WireGuard {
    /// O proxy SOCKS5 que leva as conexões pelo túnel: entregue-o ao cliente de SSH, SFTP,
    /// Telnet, FTP, S3 ou rclone do app. É só para os clientes do próprio app; a porta e a
    /// credencial são aleatórias e não devem aparecer na interface.
    pub fn proxy(&self) -> Proxy {
        let acesso = self.socks();
        Proxy::Socks5 {
            host: acesso.addr.ip().to_string(),
            port: acesso.addr.port(),
            user: Some(acesso.user.clone()),
            password: Some((*acesso.password).clone()),
        }
    }

    /// O acesso à ponte SOCKS5 interna (endereço, usuário e senha). Nunca vai para a
    /// interface.
    pub fn socks(&self) -> &SocksAccess {
        self.bridge
            .as_ref()
            .map(SocksBridge::access)
            .expect("a ponte só some em `stop`, que consome o WireGuard")
    }

    /// A pilha TCP, para abrir conexões direto, sem passar pelo SOCKS5.
    pub fn stack(&self) -> &NetStack {
        &self.stack
    }

    /// O estado agora.
    pub fn state(&self) -> State {
        self.state.borrow().clone()
    }

    /// A chave pública deste nó (a que o outro lado põe no `[Peer]`).
    pub fn public_key(&self) -> PublicKey {
        self.public_key
    }

    /// A porta UDP local (para os testes e o diagnóstico).
    pub fn udp_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Os avisos de quando o túnel subiu (por exemplo, um nome de `Endpoint` que não resolveu).
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// As redes alcançáveis pelo túnel: os `AllowedIPs` de todos os pares.
    pub fn routes(&self) -> &[Cidr] {
        &self.routes
    }

    /// Os endereços deste nó dentro da rede (o `Address` do `.conf`).
    pub fn addresses(&self) -> &[InterfaceAddress] {
        &self.addresses
    }

    /// Espera o estado `Connected`.
    ///
    /// # Errors
    ///
    /// Se passa `limit` sem conectar (a mensagem diz o último estado) ou o motor parou.
    pub async fn wait_connected(&self, limit: Duration) -> io::Result<()> {
        let mut rx = self.state.clone();
        let espera = async {
            loop {
                let atual = rx.borrow_and_update().clone();
                match atual {
                    State::Connected { .. } => return Ok(()),
                    State::Failed(motivo) => {
                        return Err(io::Error::other(format!(
                            "a rede WireGuard falhou: {motivo}"
                        )));
                    }
                    State::Stopped => {
                        return Err(io::Error::new(
                            io::ErrorKind::BrokenPipe,
                            "a rede WireGuard parou",
                        ));
                    }
                    State::Connecting | State::Retrying(_) => {}
                }
                if rx.changed().await.is_err() {
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "a rede WireGuard parou",
                    ));
                }
            }
        };
        match tokio::time::timeout(limit, espera).await {
            Ok(resultado) => resultado,
            Err(_) => {
                let detalhe = match self.state() {
                    State::Retrying(motivo) => motivo,
                    outro => format!("estado: {outro:?}"),
                };
                Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!(
                        "a rede WireGuard não conectou em {} s ({detalhe})",
                        limit.as_secs()
                    ),
                ))
            }
        }
    }

    /// O que se sabe de cada par agora.
    pub async fn peers(&self) -> Vec<PeerInfo> {
        let (resposta, espera) = oneshot::channel();
        if self.commands.send(Command::Peers(resposta)).is_err() {
            return Vec::new();
        }
        espera.await.unwrap_or_default()
    }

    /// Pede para parar sem esperar e sem largar o motor: serve quando há outros donos do
    /// `Arc` (as conexões abertas por ele) e o motor não pode mais ficar ligado. Depois disso,
    /// o estado vai a `Stopped`, como no [`WireGuard::stop`].
    pub fn close(&self) {
        let (ack, _) = oneshot::channel();
        let _ = self.commands.send(Command::Stop(ack));
    }

    /// Fecha as conexões e para. O WireGuard não tem "fechar": não há nada a avisar aos pares.
    /// `espera` limita quanto tempo se espera o motor acabar.
    pub async fn stop(mut self, espera: Duration) {
        if let Some(ponte) = self.bridge.take() {
            ponte.stop().await;
        }
        self.stack.clone().shutdown().await;
        let (ack, pronto) = oneshot::channel();
        if self.commands.send(Command::Stop(ack)).is_ok() {
            let _ = tokio::time::timeout(espera, pronto).await;
        }
        if let Some(task) = self.task.take() {
            let _ = tokio::time::timeout(espera, task).await;
        }
    }
}

impl Drop for WireGuard {
    fn drop(&mut self) {
        // Largado sem `stop`: ainda assim para o motor, sem esperar.
        let (ack, _) = oneshot::channel();
        let _ = self.commands.send(Command::Stop(ack));
    }
}

/// O que o motor guarda de cada par além do que o [`Device`] sabe.
struct Vaga {
    chave: PublicKey,
    /// O `Endpoint` do `.conf` (para resolver o nome de novo).
    nome: Option<Endpoint>,
    ultima_resolucao: Option<Instant>,
    resolvendo: bool,
    /// Os destinos que tentaram passar por este par sem sessão: se o handshake não completa,
    /// as conexões que esperam por eles falham.
    pendentes: HashSet<IpAddr>,
}

type Resolvido = (PublicKey, Result<SocketAddr, String>);

struct Motor<U> {
    device: Device,
    under: Arc<U>,
    pares: Vec<Vaga>,
    to_stack: mpsc::Sender<Vec<u8>>,
    unreachable: mpsc::UnboundedSender<Unreachable>,
    state: watch::Sender<State>,
    notifier: Option<Arc<dyn Fn(State) + Send + Sync>>,
    address: IpAddr,
    total: usize,
    ja_conectou: bool,
    falha: Option<String>,
    resolvidos: Option<mpsc::UnboundedSender<Resolvido>>,
}

/// Resolve o nome de um `Endpoint` para um endereço IPv4 (o UDP de baixo é IPv4).
async fn resolver(host: &str, porta: u16) -> Result<SocketAddr, String> {
    let busca = tokio::net::lookup_host((host, porta));
    let enderecos = match tokio::time::timeout(TEMPO_RESOLUCAO, busca).await {
        Ok(Ok(it)) => it.collect::<Vec<_>>(),
        Ok(Err(e)) => return Err(format!("não foi possível resolver o Endpoint {host}: {e}")),
        Err(_) => {
            return Err(format!(
                "não foi possível resolver o Endpoint {host}: sem resposta em {} s",
                TEMPO_RESOLUCAO.as_secs()
            ));
        }
    };
    enderecos.into_iter().find(SocketAddr::is_ipv4).ok_or_else(|| {
        format!("não foi possível resolver o Endpoint {host}: só há endereços IPv6 (ainda não suportado)")
    })
}

impl<U: Underlay> Motor<U> {
    fn mudar_estado(&mut self, novo: State) {
        if *self.state.borrow() == novo {
            return;
        }
        self.state.send_replace(novo.clone());
        if let Some(f) = &self.notifier {
            f(novo);
        }
    }

    /// Roda o `Device` (com um datagrama ou só os relógios) até não haver mais o que fazer, e
    /// põe a saída no UDP e na pilha.
    fn bombear(&mut self, datagrama: Option<(&[u8], SocketAddr)>) {
        let agora = Instant::now();
        let mut falhas: Vec<(PublicKey, PeerError)> = Vec::new();
        {
            let Motor {
                device,
                under,
                to_stack,
                ..
            } = self;
            let mut evento = match datagrama {
                Some((dados, de)) => device.receive(dados, de, agora),
                None => device.poll(agora),
            };
            loop {
                match evento {
                    Event::Nothing => break,
                    Event::Transmit { to, data } => {
                        // O UDP perde pacotes: um erro de envio é um pacote perdido.
                        let _ = under.try_send_to(data, to);
                    }
                    Event::Ip { data, .. } => {
                        let _ = to_stack.try_send(data.to_vec());
                    }
                    Event::PeerError { peer, error } => falhas.push((peer, error)),
                }
                evento = device.poll(agora);
            }
        }
        for (chave, erro) in falhas {
            self.falhou(chave, erro);
        }
        self.atualizar_estado(agora);
    }

    /// Um par não respondeu: as conexões que esperavam por ele falham com o motivo.
    fn falhou(&mut self, chave: PublicKey, erro: PeerError) {
        let endpoint = self
            .device
            .endpoint(&chave)
            .map(|e| e.to_string())
            .or_else(|| {
                self.pares
                    .iter()
                    .find(|v| v.chave == chave)
                    .and_then(|v| v.nome.as_ref().map(ToString::to_string))
            })
            .unwrap_or_else(|| "sem endpoint".into());
        let motivo = match erro {
            PeerError::HandshakeTimeout => format!(
                "{} não respondeu ao handshake em {} s ({endpoint})",
                chave.short(),
                REKEY_ATTEMPT_TIME.as_secs()
            ),
            PeerError::Random => erro.to_string(),
        };
        if let Some(vaga) = self.pares.iter_mut().find(|v| v.chave == chave) {
            for addr in vaga.pendentes.drain() {
                let _ = self.unreachable.send(Unreachable {
                    addr,
                    reason: motivo.clone(),
                });
            }
            // O nome pode ter mudado de endereço: tenta de novo já.
            vaga.ultima_resolucao = None;
        }
        self.falha = Some(motivo);
    }

    /// Recalcula o estado a partir das sessões.
    fn atualizar_estado(&mut self, agora: Instant) {
        let mut ativos = 0;
        for vaga in &mut self.pares {
            if self
                .device
                .peer(&vaga.chave)
                .is_some_and(|p| p.has_session(agora))
            {
                ativos += 1;
                vaga.pendentes.clear();
            }
        }
        if ativos > 0 {
            self.ja_conectou = true;
            self.falha = None;
            let novo = State::Connected {
                address: self.address,
                peers_up: ativos,
                peers: self.total,
            };
            self.mudar_estado(novo);
        } else if let Some(f) = self.falha.clone() {
            self.mudar_estado(State::Retrying(f));
        }
        // Sem sessão e sem falha: se já conectou, a sessão só expirou por falta de uso e o
        // estado fica como está; senão, continua `Connecting`.
    }

    /// Um pacote IP que a pilha mandou.
    fn pacote_da_pilha(&mut self, pacote: &[u8]) {
        let agora = Instant::now();
        match self.device.send(pacote) {
            Ok(chave) => {
                if let Some(dest) = destino_ip(pacote)
                    && !self
                        .device
                        .peer(&chave)
                        .is_some_and(|p| p.has_session(agora))
                    && let Some(vaga) = self.pares.iter_mut().find(|v| v.chave == chave)
                {
                    vaga.pendentes.insert(dest);
                }
            }
            Err(SendError::NoRoute(ip)) => {
                let _ = self.unreachable.send(Unreachable {
                    addr: ip,
                    reason: format!("{ip} não está nos AllowedIPs de nenhum par"),
                });
            }
            Err(SendError::NotIp) => {}
        }
        self.bombear(None);
    }

    /// Resolve de novo os nomes dos `Endpoint` dos pares que ainda não têm sessão.
    fn resolver_pendentes(&mut self) {
        let Some(canal) = self.resolvidos.clone() else {
            return;
        };
        let agora = Instant::now();
        for vaga in &mut self.pares {
            let Some(Endpoint::Host { host, port }) = &vaga.nome else {
                continue;
            };
            let sem_sessao = !self
                .device
                .peer(&vaga.chave)
                .is_some_and(|p| p.has_session(agora));
            let tentando = self
                .device
                .peer(&vaga.chave)
                .is_some_and(|p| p.handshake_in_flight())
                || self.device.endpoint(&vaga.chave).is_none();
            let vencido = vaga
                .ultima_resolucao
                .is_none_or(|t| agora.duration_since(t) >= INTERVALO_RESOLUCAO);
            if vaga.resolvendo || !sem_sessao || !tentando || !vencido {
                continue;
            }
            vaga.resolvendo = true;
            vaga.ultima_resolucao = Some(agora);
            let (chave, host, porta, canal) = (vaga.chave, host.clone(), *port, canal.clone());
            tokio::spawn(async move {
                let _ = canal.send((chave, resolver(&host, porta).await));
            });
        }
    }

    fn resolvido(&mut self, chave: PublicKey, resultado: Result<SocketAddr, String>) {
        let agora = Instant::now();
        if let Some(vaga) = self.pares.iter_mut().find(|v| v.chave == chave) {
            vaga.resolvendo = false;
        }
        match resultado {
            Ok(addr) => {
                let tem_sessao = self
                    .device
                    .peer(&chave)
                    .is_some_and(|p| p.has_session(agora));
                // Com sessão, o endereço que vale é o de onde o par fala (roaming).
                if !tem_sessao && self.device.endpoint(&chave) != Some(addr) {
                    self.device.set_endpoint(&chave, Some(addr));
                }
            }
            Err(e) => {
                if !self.ja_conectou {
                    self.falha = Some(e);
                    self.atualizar_estado(agora);
                }
            }
        }
    }

    fn informacoes(&self) -> Vec<PeerInfo> {
        let agora = Instant::now();
        self.pares
            .iter()
            .filter_map(|vaga| {
                let par = self.device.peer(&vaga.chave)?;
                let stats = par.stats();
                Some(PeerInfo {
                    public_key: vaga.chave,
                    endpoint: self.device.endpoint(&vaga.chave),
                    last_handshake: stats
                        .last_handshake
                        .map(|t| agora.saturating_duration_since(t)),
                    connected: par.has_session(agora),
                    tx_bytes: stats.tx_bytes,
                    rx_bytes: stats.rx_bytes,
                })
            })
            .collect()
    }

    async fn run(
        mut self,
        mut da_pilha: mpsc::Receiver<Vec<u8>>,
        mut comandos: mpsc::UnboundedReceiver<Command>,
    ) {
        let (resolvidos_tx, mut resolvidos_rx) = mpsc::unbounded_channel();
        self.resolvidos = Some(resolvidos_tx);
        let under = self.under.clone();
        let mut buf = vec![0u8; 65_535];
        let mut tique = tokio::time::interval(TIQUE);
        tique.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // Já manda os handshakes dos pares que têm endereço.
        self.bombear(None);
        let mut ack: Option<oneshot::Sender<()>> = None;
        loop {
            tokio::select! {
                recebido = poll_fn(|cx| under.poll_recv_from(cx, &mut buf)) => match recebido {
                    Ok((n, de)) => self.bombear(Some((&buf[..n], de))),
                    Err(e) if matches!(
                        e.kind(),
                        io::ErrorKind::ConnectionReset
                            | io::ErrorKind::ConnectionRefused
                            | io::ErrorKind::ConnectionAborted
                            | io::ErrorKind::WouldBlock
                            | io::ErrorKind::Interrupted
                    ) => {
                        // O Windows avisa assim que um datagrama anterior voltou como ICMP
                        // "porta inalcançável": não é erro do soquete.
                    }
                    Err(e) => {
                        self.mudar_estado(State::Failed(format!(
                            "o UDP do túnel parou: {e}"
                        )));
                        return;
                    }
                },
                pacote = da_pilha.recv() => match pacote {
                    Some(p) => {
                        self.pacote_da_pilha(&p);
                        // Aproveita a viagem: tira o que mais já esperava na fila.
                        for _ in 0..63 {
                            match da_pilha.try_recv() {
                                Ok(p) => self.pacote_da_pilha(&p),
                                Err(_) => break,
                            }
                        }
                    }
                    // A pilha acabou (o `stop` já a encerrou).
                    None => break,
                },
                _ = tique.tick() => {
                    self.bombear(None);
                    self.resolver_pendentes();
                }
                Some((chave, resultado)) = resolvidos_rx.recv() => {
                    self.resolvido(chave, resultado);
                    self.bombear(None);
                }
                comando = comandos.recv() => match comando {
                    Some(Command::Peers(resposta)) => {
                        let _ = resposta.send(self.informacoes());
                    }
                    Some(Command::Stop(a)) => {
                        ack = Some(a);
                        break;
                    }
                    None => break,
                },
            }
        }
        self.mudar_estado(State::Stopped);
        if let Some(a) = ack {
            let _ = a.send(());
        }
    }
}
