//! O motor: registra o nó, segue o mapa do controle e liga o WireGuard ([`wg::Device`]) ao UDP, ao
//! DERP ([`Regions`]) e à pilha TCP/IP em software ([`NetStack`]), com a ponte SOCKS5 interna
//! que os clientes do app usam.
//!
//! Nada aqui cria interface de rede, mexe em rota do sistema ou pede administrador, e **nada
//! da tailnet fica à mostra para outros programas**: o único acesso é a ponte SOCKS5 em
//! `127.0.0.1`, com porta e credencial aleatórias. A máquina se anuncia com `ShieldsUp` e sem
//! sub-redes (`RoutableIPs` vazio): nenhuma conexão que chega pela tailnet é aceita, e um
//! datagrama que não é de um par do mapa, nem disco de um par, nem a resposta de um pedido
//! nosso é descartado em silêncio.
//!
//! Para o WireGuard cada par ganha um endereço **virtual** (veja [`crate::magicsock`]): o
//! `Device` fala com esse endereço e o [`Magic`] decide se o datagrama vai direto pelo UDP ou
//! pelo DERP.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::future::{pending, poll_fn};
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::task::{Context, Poll};
use std::time::{Duration, Instant, SystemTime};

use jiff::Timestamp;
use jiff::tz::TimeZone;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tomni_ui_net::Proxy;
use tomni_ui_netstack::{
    Answer, Cidr as NetCidr, NetStack, PacketTunnel, Resolver, SocksAccess, SocksBridge,
    SocksListener, UdpUnderlay, Underlay, Unreachable,
};
use tomni_ui_wireguard as wg;
use zeroize::Zeroizing;

use crate::control::map::{self, MapAnnounce, MapConfig, MapEvent, MapHandle};
use crate::control::{Control, ControlConfig, RegisterOutcome, RegisterParams};
use crate::derp::Event as DerpEvent;
use crate::derp::regions::{Regions, pick_home};
use crate::key::{DiscoKey, MachineKey, MachinePublic, NodeKey, NodePublic};
use crate::magicsock::{Magic, Out, Path, PeerSpec, Recv};
use crate::netmap::{Change, NetMap};
use crate::stun::{self, TxId};
use crate::tailcfg::{DerpRegionId, EndpointType, Hostinfo, NetInfo, RegisterResponse};
use crate::upgrade::Roots;

/// De quanto em quanto tempo os relógios (WireGuard, disco, DERP) rodam.
const TIQUE: Duration = Duration::from_millis(100);

/// O MTU do túnel (o mesmo do Tailscale).
const MTU: usize = 1280;

/// Quantos pacotes IP esperam em cada sentido entre a pilha e o motor.
const FILA: usize = 1024;

/// De quanto em quanto tempo se pergunta ao STUN qual é o nosso endereço de fora.
const INTERVALO_STUN: Duration = Duration::from_secs(25);

/// Por quanto tempo um endereço visto pelo STUN continua valendo sem novo aviso.
const VALIDADE_STUN: Duration = Duration::from_secs(90);

/// Quantas regiões do DERP são consultadas por STUN a cada rodada.
const REGIOES_STUN: usize = 3;

/// De quanto em quanto tempo se tenta escolher a região de casa de novo, se falhou.
const REPETE_CASA: Duration = Duration::from_secs(10);

/// Quanto se espera entre dois pedidos de registro enquanto o administrador não aprova.
const ESPERA_APROVACAO: Duration = Duration::from_secs(5);

/// Quanto tempo o mesmo aviso de "inalcançável" não se repete.
const REPETE_AVISO: Duration = Duration::from_secs(2);

/// O que precisa sobreviver entre execuções; o app guarda no cofre.
///
/// Sem `machine_key`, o motor gera uma e chama `on_node_state` antes de tudo. Se o app não
/// gravar o estado, a próxima execução aparece como outra máquina no controle.
#[derive(Clone, Default, Debug)]
pub struct NodeState {
    /// A chave da máquina (identifica este programa para o controle); gerada no primeiro uso.
    pub machine_key: Option<MachineKey>,
    /// A chave do nó (a do WireGuard); gerada no primeiro uso.
    pub node_key: Option<NodeKey>,
    /// Quando a chave do nó vence, se o controle disse.
    pub node_key_expiry: Option<SystemTime>,
    /// O login que o controle conhece (o e-mail, em geral), da última vez.
    pub login: String,
    /// O domínio da tailnet (`minha-tailnet.ts.net`), da última vez.
    pub tailnet: String,
    /// O nome desta máquina na tailnet, da última vez.
    pub node_name: String,
    /// Os endereços desta máquina na tailnet, da última vez.
    pub addresses: Vec<IpAddr>,
}

type Assinatura = (
    Option<MachinePublic>,
    Option<NodePublic>,
    Option<SystemTime>,
    String,
    String,
    String,
    Vec<IpAddr>,
);

impl NodeState {
    fn assinatura(&self) -> Assinatura {
        (
            self.machine_key.as_ref().map(MachineKey::public),
            self.node_key.as_ref().map(NodeKey::public),
            self.node_key_expiry,
            self.login.clone(),
            self.tailnet.clone(),
            self.node_name.clone(),
            self.addresses.clone(),
        )
    }
}

/// O estado do nó.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// Subindo: falando com o controle.
    Connecting,
    /// Abra esta URL no navegador para entrar; o motor espera sozinho.
    NeedsLogin {
        /// A URL de login.
        url: String,
    },
    /// Login feito; a tailnet exige que o administrador aprove esta máquina.
    NeedsApproval,
    /// Tem o mapa da tailnet: o proxy já leva as conexões.
    Connected {
        /// O endereço IPv4 deste nó na tailnet.
        address: IpAddr,
        /// O domínio da tailnet.
        tailnet: String,
        /// Quantas outras máquinas estão ligadas agora.
        peers_online: usize,
        /// O nome da região do DERP de casa (vazio enquanto não escolhida).
        derp_home: String,
    },
    /// A chave do nó venceu e não há chave de autenticação para renovar: entre de novo (inicie
    /// com um `NodeState` sem `node_key`).
    Expired,
    /// O controle não responde; o motor tenta de novo sozinho. Os pares que já tinham sessão
    /// seguem funcionando.
    Retrying(String),
    /// Parou por um erro que não passa sozinho.
    Failed(String),
    /// Parado (por [`Tailscale::stop`]).
    Stopped,
}

impl State {
    /// O endereço do nó na tailnet, se está conectado.
    pub fn address(&self) -> Option<IpAddr> {
        match self {
            State::Connected { address, .. } => Some(*address),
            _ => None,
        }
    }
}

/// O que se sabe de uma máquina da tailnet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Peer {
    /// O nome curto (o primeiro rótulo do nome completo).
    pub name: String,
    /// O nome completo, sem o ponto final.
    pub dns_name: String,
    /// Os endereços na tailnet.
    pub addresses: Vec<IpAddr>,
    /// O sistema (`linux`, `windows`…), se a máquina contou.
    pub os: String,
    /// Se está ligada ao controle (`None`: o controle não disse).
    pub online: Option<bool>,
    /// Quando o controle a viu por último, se está desligada.
    pub last_seen: Option<SystemTime>,
    /// O login do dono.
    pub user: String,
    /// As etiquetas de ACL.
    pub tags: Vec<String>,
    /// Por onde vão os pacotes: `direto (192.0.2.7:41641)` ou `DERP (São Paulo)`; vazio quando
    /// ainda não há caminho.
    pub path: String,
}

/// Quem resolve os nomes que o mapa não sabe: o nome sob o domínio da tailnet que não existe é
/// recusado (não vai ao DNS do computador); o resto segue para ele.
struct MagicDns(Arc<Compartilhado>);

impl Resolver for MagicDns {
    fn resolve<'a>(
        &'a self,
        _stack: &'a NetStack,
        name: &'a str,
    ) -> Pin<Box<dyn Future<Output = Answer> + Send + 'a>> {
        Box::pin(async move {
            let mapa = self.0.netmap();
            if let Some(ip) = mapa.resolve(name) {
                return Answer::Found(vec![IpAddr::V4(ip)]);
            }
            let dominio = mapa.domain().trim_end_matches('.').to_lowercase();
            let nome = name.trim().trim_end_matches('.').to_lowercase();
            if !dominio.is_empty() && nome.ends_with(&format!(".{dominio}")) {
                Answer::NotFound
            } else {
                Answer::Skip
            }
        })
    }
}

/// O que o motor, o túnel e o `Tailscale` dividem.
struct Compartilhado {
    netmap: RwLock<Arc<NetMap>>,
    stack: Mutex<Option<NetStack>>,
}

impl Compartilhado {
    fn netmap(&self) -> Arc<NetMap> {
        self.netmap
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// O túnel de pacotes IP entre a pilha TCP e o motor.
struct TailscaleTunnel {
    addresses: Vec<NetCidr>,
    routes: Vec<NetCidr>,
    to_engine: mpsc::Sender<Vec<u8>>,
    from_engine: mpsc::Receiver<Vec<u8>>,
    unreachable: mpsc::UnboundedReceiver<Unreachable>,
    routes_rx: mpsc::UnboundedReceiver<Vec<NetCidr>>,
    shared: Arc<Compartilhado>,
}

impl PacketTunnel for TailscaleTunnel {
    fn mtu(&self) -> usize {
        MTU
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
                "a rede Tailscale parou",
            ))),
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_routes(&mut self, cx: &mut Context<'_>) -> Poll<Vec<NetCidr>> {
        match self.routes_rx.poll_recv(cx) {
            Poll::Ready(Some(mut rotas)) => {
                // Só importa a última lista.
                while let Ok(mais) = self.routes_rx.try_recv() {
                    rotas = mais;
                }
                Poll::Ready(rotas)
            }
            Poll::Ready(None) | Poll::Pending => Poll::Pending,
        }
    }

    fn poll_unreachable(&mut self, cx: &mut Context<'_>) -> Poll<Unreachable> {
        match self.unreachable.poll_recv(cx) {
            Poll::Ready(Some(aviso)) => Poll::Ready(aviso),
            Poll::Ready(None) | Poll::Pending => Poll::Pending,
        }
    }

    /// O MagicDNS: responde só com o que o mapa sabe, sem rede.
    fn resolve(&mut self, name: &str) -> Option<Vec<IpAddr>> {
        self.shared
            .netmap()
            .resolve(name)
            .map(|ip| vec![IpAddr::V4(ip)])
    }
}

/// O que vai aos observadores: o estado e o `NodeState`.
struct Aviso {
    state: watch::Sender<State>,
    on_state: Option<OnState>,
    node: Mutex<NodeState>,
    on_node: Option<OnNode>,
}

impl Aviso {
    fn estado(&self, novo: State) {
        if *self.state.borrow() == novo {
            return;
        }
        self.state.send_replace(novo.clone());
        if let Some(f) = &self.on_state {
            f(&novo);
        }
    }

    /// Muda o `NodeState` e avisa o app se algo mudou.
    fn node(&self, f: impl FnOnce(&mut NodeState)) {
        let novo = {
            let mut guarda = self.node.lock().unwrap_or_else(PoisonError::into_inner);
            let antes = guarda.assinatura();
            f(&mut guarda);
            if guarda.assinatura() == antes {
                return;
            }
            guarda.clone()
        };
        if let Some(cb) = &self.on_node {
            cb(&novo);
        }
    }
}

/// O que o `Tailscale` pede ao motor.
enum Command {
    Stop(oneshot::Sender<()>),
    Peers(oneshot::Sender<Vec<Peer>>),
    Logout(oneshot::Sender<Result<(), String>>),
}

type OnState = Arc<dyn Fn(&State) + Send + Sync>;
type OnNode = Arc<dyn Fn(&NodeState) + Send + Sync>;

/// Monta e inicia um [`Tailscale`].
pub struct TailscaleBuilder {
    control: String,
    state: NodeState,
    hostname: String,
    auth_key: Option<Zeroizing<String>>,
    ephemeral: bool,
    accept_routes: bool,
    extra_roots: Option<String>,
    udp: bool,
    on_state: Option<OnState>,
    on_node: Option<OnNode>,
}

impl TailscaleBuilder {
    /// `control` é a URL do servidor (`https://controlplane.tailscale.com` ou a do Headscale).
    pub fn new(control: impl Into<String>, state: NodeState) -> Self {
        TailscaleBuilder {
            control: control.into(),
            state,
            hostname: "tirech-omni".to_string(),
            auth_key: None,
            ephemeral: false,
            accept_routes: true,
            extra_roots: None,
            udp: true,
            on_state: None,
            on_node: None,
        }
    }

    /// O nome da máquina na tailnet (o controle acrescenta um número se já houver outra igual).
    #[must_use]
    pub fn hostname(mut self, nome: impl Into<String>) -> Self {
        self.hostname = nome.into();
        self
    }

    /// A chave de autenticação (`tskey-auth-…` ou a do Headscale): entra sem abrir o navegador.
    #[must_use]
    pub fn auth_key(mut self, chave: Zeroizing<String>) -> Self {
        self.auth_key = Some(chave);
        self
    }

    /// Se a máquina é efêmera: o controle a esquece quando ela some.
    #[must_use]
    pub fn ephemeral(mut self, sim: bool) -> Self {
        self.ephemeral = sim;
        self
    }

    /// Usa as sub-redes que outras máquinas anunciam (padrão: sim).
    #[must_use]
    pub fn accept_routes(mut self, sim: bool) -> Self {
        self.accept_routes = sim;
        self
    }

    /// CAs extras em PEM para o TLS do controle e do DERP (um Headscale com CA própria).
    #[must_use]
    pub fn extra_roots(mut self, pem: impl Into<String>) -> Self {
        self.extra_roots = Some(pem.into());
        self
    }

    /// Sem UDP: só o DERP (redes que bloqueiam UDP; diagnóstico; testes). Padrão: com UDP.
    #[must_use]
    pub fn udp(mut self, sim: bool) -> Self {
        self.udp = sim;
        self
    }

    /// Chama `f` a cada mudança de [`State`], de qualquer thread. Não bloqueie dentro dela.
    #[must_use]
    pub fn on_state(mut self, f: impl Fn(&State) + Send + Sync + 'static) -> Self {
        self.on_state = Some(Arc::new(f));
        self
    }

    /// Chama `f` quando o [`NodeState`] muda (chave nova, login, endereços): o app grava no
    /// cofre. Nada neste crate grava em disco.
    #[must_use]
    pub fn on_node_state(mut self, f: impl Fn(&NodeState) + Send + Sync + 'static) -> Self {
        self.on_node = Some(Arc::new(f));
        self
    }

    /// Abre um UDP numa porta aleatória (se `udp` não foi desligado), a ponte SOCKS5 interna e
    /// inicia o nó, no runtime atual. Volta já: o registro e o mapa acontecem em segundo plano
    /// (veja [`Tailscale::state`] e [`Tailscale::wait_connected`]).
    ///
    /// # Errors
    ///
    /// Se a URL do controle ou a CA extra não servem, ou o UDP não abre. A mensagem é em
    /// português.
    pub async fn start(self) -> io::Result<Tailscale> {
        let under: Option<Arc<dyn Underlay>> = if self.udp {
            let (u, _aviso) = UdpUnderlay::bind("0.0.0.0", 0).await?;
            Some(Arc::new(u))
        } else {
            None
        };
        self.iniciar(under).await
    }

    /// Como [`start`](TailscaleBuilder::start), mas sobre um UDP dado (os testes usam a rede
    /// simulada do `tomni-ui-netstack`). A opção [`udp`](TailscaleBuilder::udp) vale.
    ///
    /// # Errors
    ///
    /// Os mesmos de [`start`](TailscaleBuilder::start).
    pub async fn start_on(self, under: impl Underlay) -> io::Result<Tailscale> {
        let under: Option<Arc<dyn Underlay>> = if self.udp {
            Some(Arc::new(under))
        } else {
            None
        };
        self.iniciar(under).await
    }

    async fn iniciar(self, under: Option<Arc<dyn Underlay>>) -> io::Result<Tailscale> {
        let invalido = |e: String| io::Error::new(io::ErrorKind::InvalidInput, e);
        let mut roots = Roots::system();
        if let Some(pem) = &self.extra_roots {
            roots = roots.with_pem(pem).map_err(|e| invalido(e.to_string()))?;
        }
        let mut estado = self.state.clone();
        let gerou = estado.machine_key.is_none() || estado.node_key.is_none();
        let machine_key = estado
            .machine_key
            .clone()
            .unwrap_or_else(MachineKey::generate);
        let node_key = estado.node_key.clone().unwrap_or_else(NodeKey::generate);
        estado.machine_key = Some(machine_key.clone());
        estado.node_key = Some(node_key.clone());
        let mut config = ControlConfig::new(self.control.clone());
        config.roots = roots.clone();
        let control = Control::new(config, machine_key).map_err(|e| invalido(e.to_string()))?;

        let (state_tx, state_rx) = watch::channel(State::Connecting);
        let aviso = Arc::new(Aviso {
            state: state_tx,
            on_state: self.on_state,
            node: Mutex::new(estado.clone()),
            on_node: self.on_node,
        });
        if gerou && let Some(cb) = &aviso.on_node {
            cb(&estado);
        }

        let shared = Arc::new(Compartilhado {
            netmap: RwLock::new(Arc::new(NetMap::new(self.accept_routes))),
            stack: Mutex::new(None),
        });
        let bridge = SocksListener::bind()?
            .resolver(Arc::new(MagicDns(shared.clone())))
            .serve_later()?;
        let access = bridge.access().clone();

        let disco = DiscoKey::generate();
        let magic = Magic::new(node_key.public(), disco, under.is_some());
        let (derp_tx, derp_rx) = mpsc::unbounded_channel();
        let regions = Regions::new(node_key.clone(), roots, derp_tx);
        let private = wg::PrivateKey::from_bytes(node_key.to_bytes());
        let device = wg::Device::new(private, Instant::now())
            .map_err(|e| invalido(format!("a chave do nó não serve: {e}")))?;
        let local_addr = match &under {
            Some(u) => Some(u.local_addr()?),
            None => None,
        };

        let (to_engine_tx, to_engine_rx) = mpsc::channel(FILA);
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (casa_tx, casa_rx) = mpsc::unbounded_channel();
        let motor = Motor {
            cfg: Config {
                hostname: self.hostname,
                auth_key: self.auth_key,
                ephemeral: self.ephemeral,
                accept_routes: self.accept_routes,
            },
            aviso: aviso.clone(),
            shared: shared.clone(),
            under,
            node_key: node_key.clone(),
            magic,
            regions,
            device,
            vagas: HashMap::new(),
            netmap: Arc::new(NetMap::new(self.accept_routes)),
            tem_mapa: false,
            desconectado: None,
            bridge,
            to_engine: to_engine_tx,
            to_stack: None,
            unreachable: None,
            routes_tx: None,
            stack: None,
            enderecos: Vec::new(),
            rotas: Vec::new(),
            pendentes: HashMap::new(),
            avisados: HashMap::new(),
            stun: Stun::default(),
            anunciado: MapAnnounce::default(),
            casa_tx,
            escolhendo_casa: false,
            ultima_escolha: None,
        };
        let task = tokio::spawn(motor.run(control, to_engine_rx, cmd_rx, derp_rx, casa_rx));
        Ok(Tailscale {
            commands: cmd_tx,
            state: state_rx,
            task: Some(task),
            access,
            shared,
            public_key: node_key.public(),
            local_addr,
        })
    }
}

/// Um nó da tailnet em funcionamento, só para o tráfego do app.
///
/// Ao ser largado (ou em [`stop`](Tailscale::stop)), para o motor.
pub struct Tailscale {
    commands: mpsc::UnboundedSender<Command>,
    state: watch::Receiver<State>,
    task: Option<JoinHandle<()>>,
    access: SocksAccess,
    shared: Arc<Compartilhado>,
    public_key: NodePublic,
    local_addr: Option<SocketAddr>,
}

impl fmt::Debug for Tailscale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tailscale")
            .field("state", &*self.state.borrow())
            .finish_non_exhaustive()
    }
}

impl Tailscale {
    /// O proxy SOCKS5 que leva as conexões pela tailnet: entregue-o ao cliente de SSH, SFTP,
    /// Telnet, FTP, S3 ou rclone do app. É só para os clientes do próprio app; a porta e a
    /// credencial são aleatórias e não devem aparecer na interface.
    pub fn proxy(&self) -> Proxy {
        Proxy::Socks5 {
            host: self.access.addr.ip().to_string(),
            port: self.access.addr.port(),
            user: Some(self.access.user.clone()),
            password: Some((*self.access.password).clone()),
        }
    }

    /// O acesso à ponte SOCKS5 interna (endereço, usuário e senha). Nunca vai para a
    /// interface.
    pub fn socks(&self) -> &SocksAccess {
        &self.access
    }

    /// A pilha TCP, para abrir conexões direto, sem passar pelo SOCKS5. `None` até o primeiro
    /// mapa (é quando se sabe o endereço do nó).
    pub fn stack(&self) -> Option<NetStack> {
        self.shared
            .stack
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// O estado agora.
    pub fn state(&self) -> State {
        self.state.borrow().clone()
    }

    /// A chave pública deste nó (a que o controle conhece).
    pub fn public_key(&self) -> NodePublic {
        self.public_key
    }

    /// A porta UDP local (para os testes e o diagnóstico); `None` sem UDP.
    pub fn udp_addr(&self) -> Option<SocketAddr> {
        self.local_addr
    }

    /// Espera o estado `Connected` e devolve o endereço do nó na tailnet.
    ///
    /// # Errors
    ///
    /// Se passa `limit` sem conectar (a mensagem diz o último estado, por exemplo a URL de
    /// login), o motor falhou ou parou.
    pub async fn wait_connected(&self, limit: Duration) -> io::Result<IpAddr> {
        let mut rx = self.state.clone();
        let espera = async {
            loop {
                let atual = rx.borrow_and_update().clone();
                match atual {
                    State::Connected { address, .. } => return Ok(address),
                    State::Failed(motivo) => {
                        return Err(io::Error::other(format!(
                            "a rede Tailscale falhou: {motivo}"
                        )));
                    }
                    State::Expired => {
                        return Err(io::Error::other(
                            "a chave desta máquina venceu; entre de novo",
                        ));
                    }
                    State::Stopped => {
                        return Err(io::Error::new(
                            io::ErrorKind::BrokenPipe,
                            "a rede Tailscale parou",
                        ));
                    }
                    State::Connecting
                    | State::NeedsLogin { .. }
                    | State::NeedsApproval
                    | State::Retrying(_) => {}
                }
                if rx.changed().await.is_err() {
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "a rede Tailscale parou",
                    ));
                }
            }
        };
        match tokio::time::timeout(limit, espera).await {
            Ok(resultado) => resultado,
            Err(_) => {
                let detalhe = match self.state() {
                    State::NeedsLogin { url } => mensagem_login(&url),
                    State::NeedsApproval => MENSAGEM_APROVACAO.to_string(),
                    State::Retrying(motivo) => motivo,
                    outro => format!("estado: {outro:?}"),
                };
                Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!(
                        "a rede Tailscale não conectou em {} s ({detalhe})",
                        limit.as_secs()
                    ),
                ))
            }
        }
    }

    /// As máquinas da tailnet agora, com o caminho de cada uma.
    pub async fn peers(&self) -> Vec<Peer> {
        let (resposta, espera) = oneshot::channel();
        if self.commands.send(Command::Peers(resposta)).is_err() {
            return Vec::new();
        }
        espera.await.unwrap_or_default()
    }

    /// Sai da tailnet: pede ao controle que a chave do nó vença e para. Depois disso o
    /// `NodeState` não serve mais (comece com um sem `node_key`).
    ///
    /// # Errors
    ///
    /// Se o controle não respondeu; o nó para do mesmo jeito.
    pub async fn logout(mut self) -> Result<(), String> {
        let (resposta, espera) = oneshot::channel();
        if self.commands.send(Command::Logout(resposta)).is_err() {
            return Err("a rede Tailscale já parou".to_string());
        }
        let resultado = espera
            .await
            .unwrap_or_else(|_| Err("a rede Tailscale parou".to_string()));
        if let Some(task) = self.task.take() {
            let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
        }
        resultado
    }

    /// Fecha as conexões e para. `espera` limita quanto tempo se espera o motor acabar.
    pub async fn stop(mut self, espera: Duration) {
        let (ack, pronto) = oneshot::channel();
        if self.commands.send(Command::Stop(ack)).is_ok() {
            let _ = tokio::time::timeout(espera, pronto).await;
        }
        if let Some(task) = self.task.take() {
            let _ = tokio::time::timeout(espera, task).await;
        }
    }
}

impl Drop for Tailscale {
    fn drop(&mut self) {
        // Largado sem `stop`: ainda assim para o motor, sem esperar.
        let (ack, _) = oneshot::channel();
        let _ = self.commands.send(Command::Stop(ack));
    }
}

const MENSAGEM_APROVACAO: &str =
    "login feito; esperando o administrador da tailnet aprovar esta máquina";

fn mensagem_login(url: &str) -> String {
    format!("abra {url} no navegador para entrar na tailnet")
}

struct Config {
    hostname: String,
    auth_key: Option<Zeroizing<String>>,
    ephemeral: bool,
    accept_routes: bool,
}

/// Um par no `Device`.
struct Vaga {
    wg: wg::PublicKey,
    permitidos: Vec<wg::Cidr>,
}

/// O que o motor sabe do STUN.
#[derive(Default)]
struct Stun {
    /// Os pedidos esperando resposta: o servidor consultado.
    pendentes: HashMap<TxId, (SocketAddr, Instant)>,
    /// O que cada servidor viu de nós, e quando.
    vistos: HashMap<SocketAddr, (SocketAddr, Instant)>,
    proxima: Option<Instant>,
}

/// Por que o motor acabou.
enum Fim {
    Parar(Option<oneshot::Sender<()>>),
    Estado(State),
    Sair,
}

struct Motor {
    cfg: Config,
    aviso: Arc<Aviso>,
    shared: Arc<Compartilhado>,
    under: Option<Arc<dyn Underlay>>,
    node_key: NodeKey,
    magic: Magic,
    regions: Regions,
    device: wg::Device,
    vagas: HashMap<NodePublic, Vaga>,
    netmap: Arc<NetMap>,
    tem_mapa: bool,
    /// O motivo de o controle estar fora do ar (o estado é `Retrying`).
    desconectado: Option<String>,
    bridge: SocksBridge,
    to_engine: mpsc::Sender<Vec<u8>>,
    to_stack: Option<mpsc::Sender<Vec<u8>>>,
    unreachable: Option<mpsc::UnboundedSender<Unreachable>>,
    routes_tx: Option<mpsc::UnboundedSender<Vec<NetCidr>>>,
    stack: Option<NetStack>,
    enderecos: Vec<Ipv4Addr>,
    rotas: Vec<NetCidr>,
    /// Os destinos que tentaram passar por um par sem sessão.
    pendentes: HashMap<NodePublic, HashSet<IpAddr>>,
    avisados: HashMap<IpAddr, Instant>,
    stun: Stun,
    anunciado: MapAnnounce,
    casa_tx: mpsc::UnboundedSender<Option<DerpRegionId>>,
    escolhendo_casa: bool,
    ultima_escolha: Option<Instant>,
}

/// O endereço de destino de um pacote IP inteiro.
fn destino_ip(p: &[u8]) -> Option<IpAddr> {
    match p.first()? >> 4 {
        4 if p.len() >= 20 => Some(IpAddr::V4(Ipv4Addr::new(p[16], p[17], p[18], p[19]))),
        6 if p.len() >= 40 => {
            let mut b = [0u8; 16];
            b.copy_from_slice(&p[24..40]);
            Some(IpAddr::V6(b.into()))
        }
        _ => None,
    }
}

fn so_ip(texto: &str) -> Option<IpAddr> {
    texto.split('/').next()?.parse().ok()
}

fn nome_curto(fqdn: &str) -> String {
    fqdn.trim_end_matches('.')
        .split('.')
        .next()
        .unwrap_or_default()
        .to_string()
}

fn data_legivel(t: Timestamp) -> String {
    t.to_zoned(TimeZone::UTC)
        .strftime("%d/%m/%Y %H:%M UTC")
        .to_string()
}

fn cidr_wg(c: &NetCidr) -> Option<wg::Cidr> {
    wg::Cidr::new(c.addr(), c.prefix()).ok()
}

impl Motor {
    fn hostinfo(&self) -> Hostinfo {
        let os = match std::env::consts::OS {
            "macos" => "macOS",
            outro => outro,
        };
        Hostinfo {
            ipn_version: concat!("tomni-ui-tailscale/", env!("CARGO_PKG_VERSION")).to_string(),
            os: os.to_string(),
            hostname: self.cfg.hostname.clone(),
            shields_up: true,
            routable_ips: Vec::new(),
            net_info: Some(NetInfo {
                working_udp: Some(self.under.is_some()),
                working_ipv6: Some(false),
                preferred_derp: self.regions.home(),
            }),
            userspace: Some(true),
            ..Hostinfo::default()
        }
    }

    fn params(&self, node_key: &NodeKey, antiga: Option<NodePublic>) -> RegisterParams {
        let mut hostinfo = self.hostinfo();
        hostinfo.net_info = None;
        RegisterParams {
            node_key: node_key.public(),
            old_node_key: antiga,
            auth_key: self.cfg.auth_key.clone(),
            hostinfo,
            ephemeral: self.cfg.ephemeral,
        }
    }

    /// Quanto cada comando pode fazer enquanto não há mapa.
    fn comando_sem_mapa(comando: Option<Command>) -> Option<Fim> {
        match comando {
            Some(Command::Stop(ack)) => Some(Fim::Parar(Some(ack))),
            None => Some(Fim::Sair),
            Some(Command::Peers(r)) => {
                let _ = r.send(Vec::new());
                None
            }
            Some(Command::Logout(r)) => {
                let _ = r.send(Err("esta máquina ainda não entrou na tailnet".to_string()));
                None
            }
        }
    }

    /// Registra o nó. Devolve a resposta do registro ou o motivo de não seguir.
    async fn registrar(
        &mut self,
        control: &mut Control,
        cmds: &mut mpsc::UnboundedReceiver<Command>,
    ) -> Result<RegisterResponse, Fim> {
        let aviso = self.aviso.clone();
        let mut troquei_chave = false;
        loop {
            let atual = self.node_key.clone();
            let params = self.params(&atual, None);
            let resultado = {
                let mut ao_pedir_login = |url: &str| {
                    aviso.estado(State::NeedsLogin {
                        url: url.to_string(),
                    });
                };
                let fluxo = control.register_flow(&params, &mut ao_pedir_login);
                tokio::pin!(fluxo);
                loop {
                    tokio::select! {
                        r = &mut fluxo => break r,
                        c = cmds.recv() => {
                            if let Some(fim) = Self::comando_sem_mapa(c) {
                                return Err(fim);
                            }
                        }
                    }
                }
            };
            match resultado {
                Ok(RegisterOutcome::Authorized(resposta)) => return Ok(resposta),
                Ok(RegisterOutcome::NeedsApproval(_)) => {
                    self.aviso.estado(State::NeedsApproval);
                    let espera = tokio::time::sleep(ESPERA_APROVACAO);
                    tokio::pin!(espera);
                    loop {
                        tokio::select! {
                            () = &mut espera => break,
                            c = cmds.recv() => {
                                if let Some(fim) = Self::comando_sem_mapa(c) {
                                    return Err(fim);
                                }
                            }
                        }
                    }
                }
                Ok(RegisterOutcome::KeyExpired) => {
                    if self.cfg.auth_key.is_none() || troquei_chave {
                        return Err(Fim::Estado(State::Expired));
                    }
                    // Com chave de autenticação a renovação é automática: chave de nó nova.
                    troquei_chave = true;
                    self.trocar_chave(NodeKey::generate());
                }
                Ok(RegisterOutcome::NeedsLogin { url }) => {
                    // O `register_flow` já trata o login; chegar aqui é raro.
                    self.aviso.estado(State::NeedsLogin { url });
                }
                Err(e) => return Err(Fim::Estado(State::Failed(e.to_string()))),
            }
        }
    }

    /// Troca a chave do nó (depois de vencer): o `Device` e o `Regions` passam a usar a nova.
    fn trocar_chave(&mut self, nova: NodeKey) {
        let privada = wg::PrivateKey::from_bytes(nova.to_bytes());
        if let Ok(device) = wg::Device::new(privada, Instant::now()) {
            self.device = device;
            self.vagas.clear();
        }
        let disco = DiscoKey::generate();
        self.magic = Magic::new(nova.public(), disco, self.under.is_some());
        let (tx, _rx) = mpsc::unbounded_channel();
        // O DERP reconecta com a chave nova assim que o mapa chegar.
        self.regions.close();
        self.regions = Regions::new(nova.clone(), Roots::system(), tx);
        self.node_key = nova.clone();
        self.aviso.node(|n| n.node_key = Some(nova));
    }

    // ----- a saída -----

    /// Põe na rede o que o `Magic` mandou.
    fn executar(&mut self, saida: Vec<Out>) {
        let agora = Instant::now();
        for o in saida {
            match o {
                Out::Udp { to, data } => {
                    if let Some(under) = &self.under {
                        // O UDP perde pacotes: um erro de envio é um pacote perdido.
                        let _ = under.try_send_to(&data, to);
                    }
                }
                Out::Derp { region, to, data } => {
                    let _ = self.regions.send(region, to, data, agora);
                }
            }
        }
    }

    /// Roda o `Device` (com um datagrama ou só os relógios) até não haver mais o que fazer, e
    /// põe a saída no UDP, no DERP e na pilha.
    fn bombear(&mut self, datagrama: Option<(&[u8], SocketAddr)>) {
        let agora = Instant::now();
        let mut saida: Vec<Out> = Vec::new();
        let mut falhas: Vec<(wg::PublicKey, wg::PeerError)> = Vec::new();
        {
            let Motor {
                device,
                magic,
                to_stack,
                ..
            } = self;
            let mut evento = match datagrama {
                Some((dados, de)) => device.receive(dados, de, agora),
                None => device.poll(agora),
            };
            loop {
                match evento {
                    wg::Event::Nothing => break,
                    wg::Event::Transmit { to, data } => {
                        magic.route(to, data.to_vec(), agora, &mut saida);
                    }
                    wg::Event::Ip { data, .. } => {
                        if let Some(tx) = to_stack {
                            let _ = tx.try_send(data.to_vec());
                        }
                    }
                    wg::Event::PeerError { peer, error } => falhas.push((peer, error)),
                }
                evento = device.poll(agora);
            }
        }
        self.executar(saida);
        for (chave, erro) in falhas {
            self.falhou(NodePublic::from_bytes(*chave.as_bytes()), erro);
        }
    }

    /// Um par não respondeu ao handshake: as conexões que esperavam por ele falham.
    fn falhou(&mut self, chave: NodePublic, erro: wg::PeerError) {
        let motivo = match erro {
            wg::PeerError::HandshakeTimeout => self.motivo_sem_resposta(&chave),
            outro => outro.to_string(),
        };
        if let Some(destinos) = self.pendentes.remove(&chave)
            && let Some(tx) = &self.unreachable
        {
            for addr in destinos {
                let _ = tx.send(Unreachable {
                    addr,
                    reason: motivo.clone(),
                });
            }
        }
    }

    /// Por que a máquina não respondeu, na linguagem de quem usa.
    fn motivo_sem_resposta(&self, chave: &NodePublic) -> String {
        let Some(no) = self.netmap.peer_by_key(chave) else {
            return "a máquina não respondeu".to_string();
        };
        let nome = nome_curto(&no.name);
        if no.online == Some(false) {
            return desligada(&nome, no.last_seen);
        }
        format!(
            "{nome} não respondeu; confira se as regras de acesso (ACL) da tailnet deixam esta máquina chegar a ela"
        )
    }

    /// Um pacote IP que a pilha mandou.
    fn pacote_da_pilha(&mut self, pacote: &[u8]) {
        let agora = Instant::now();
        match self.device.send(pacote) {
            Ok(chave) => {
                let chave_no = NodePublic::from_bytes(*chave.as_bytes());
                let sem_sessao = !self
                    .device
                    .peer(&chave)
                    .is_some_and(|p| p.has_session(agora));
                if sem_sessao && let Some(dest) = destino_ip(pacote) {
                    self.pendentes.entry(chave_no).or_default().insert(dest);
                    // Uma máquina que o controle diz estar desligada: falha já, sem esperar o
                    // handshake estourar (e o pacote segue, caso o aviso esteja velho).
                    if let Some(no) = self.netmap.peer_by_key(&chave_no)
                        && no.online == Some(false)
                    {
                        let msg = desligada(&nome_curto(&no.name), no.last_seen);
                        self.avisar(dest, msg);
                    }
                }
            }
            Err(wg::SendError::NoRoute(ip)) => {
                let msg = self.motivo_sem_rota(ip);
                self.avisar(ip, msg);
            }
            Err(wg::SendError::NotIp) => {}
        }
        self.bombear(None);
    }

    fn motivo_sem_rota(&self, ip: IpAddr) -> String {
        if let Some(no) = self.netmap.peer_with_address(ip) {
            let nome = nome_curto(&no.name);
            if no.expired {
                return format!("a chave de {nome} venceu; ela precisa entrar de novo na tailnet");
            }
        }
        format!("{ip} não pertence a nenhuma máquina nem sub-rede da tailnet")
    }

    fn avisar(&mut self, addr: IpAddr, reason: String) {
        let agora = Instant::now();
        if self
            .avisados
            .get(&addr)
            .is_some_and(|t| agora.duration_since(*t) < REPETE_AVISO)
        {
            return;
        }
        self.avisados.insert(addr, agora);
        if let Some(tx) = &self.unreachable {
            let _ = tx.send(Unreachable { addr, reason });
        }
    }

    // ----- a entrada -----

    /// Chegou um datagrama UDP.
    fn udp_chegou(&mut self, dados: &[u8], de: SocketAddr) {
        let agora = Instant::now();
        if stun::looks_like_stun(dados) {
            self.stun_chegou(dados, agora);
            return;
        }
        let mut saida = Vec::new();
        let r = self.magic.on_udp(de, dados, agora, &mut saida);
        self.executar(saida);
        if let Recv::Wireguard(virtual_addr) = r {
            self.bombear(Some((dados, virtual_addr)));
        }
    }

    /// Chegou um evento de uma região do DERP.
    fn derp_chegou(&mut self, regiao: DerpRegionId, evento: DerpEvent) {
        self.regions.note_event(regiao, &evento);
        let agora = Instant::now();
        match evento {
            DerpEvent::Packet { from, data } => {
                let mut saida = Vec::new();
                let r = self.magic.on_derp(regiao, from, &data, agora, &mut saida);
                self.executar(saida);
                if let Recv::Wireguard(virtual_addr) = r {
                    self.bombear(Some((&data, virtual_addr)));
                }
            }
            DerpEvent::Connected | DerpEvent::Disconnected(_) => self.recalcular_estado(),
            DerpEvent::PeerGone(_) | DerpEvent::Health(_) => {}
        }
    }

    // ----- o mapa -----

    /// Aplica um mapa novo do controle.
    fn aplicar_mapa(&mut self, mapa: Arc<NetMap>, mudanca: &Change, primeiro: bool) {
        *self
            .shared
            .netmap
            .write()
            .unwrap_or_else(PoisonError::into_inner) = mapa.clone();
        self.netmap = mapa.clone();
        self.tem_mapa = true;
        self.desconectado = None;

        if let Some(eu) = mapa.me() {
            let enderecos: Vec<IpAddr> = mapa.addresses().into_iter().map(IpAddr::V4).collect();
            let nome = eu.name.trim_end_matches('.').to_string();
            let dominio = mapa.domain().trim_end_matches('.').to_string();
            let validade = eu.key_expiry.map(SystemTime::from);
            self.aviso.node(|n| {
                n.tailnet = dominio;
                n.node_name = nome;
                n.addresses = enderecos;
                n.node_key_expiry = validade;
            });
        }
        if primeiro || mudanca.derp {
            self.regions.set_map(mapa.derp_map().clone());
            let casa = self.regions.home();
            let existe = casa != 0
                && mapa
                    .derp_map()
                    .regions
                    .get(&casa)
                    .is_some_and(Option::is_some);
            if !existe {
                self.escolher_casa();
            }
        }
        self.sincronizar_pares();
        self.sincronizar_pilha();
        self.recalcular_estado();
    }

    /// Faz o `Device` e o `Magic` terem os mesmos pares do mapa.
    fn sincronizar_pares(&mut self) {
        let agora = Instant::now();
        let mapa = self.netmap.clone();
        let mut vistos: HashSet<NodePublic> = HashSet::new();
        for no in mapa.peers() {
            if !NetMap::usavel(no) || no.key == self.node_key.public() {
                continue;
            }
            vistos.insert(no.key);
            let virtual_addr = self.magic.set_peer(PeerSpec {
                key: no.key,
                disco: no.disco_key,
                endpoints: no.endpoints.clone(),
                home_derp: no.home_derp,
            });
            let permitidos: Vec<wg::Cidr> =
                mapa.prefixes_of(no).iter().filter_map(cidr_wg).collect();
            let wg_key = wg::PublicKey::from_bytes(*no.key.as_bytes());
            let mudou = self
                .vagas
                .get(&no.key)
                .is_some_and(|v| v.permitidos != permitidos);
            if mudou {
                self.device.remove_peer(&wg_key);
                self.vagas.remove(&no.key);
            }
            if !self.vagas.contains_key(&no.key) {
                let mut cfg = wg::PeerConfig::new(wg_key);
                cfg.mtu = MTU;
                if self
                    .device
                    .add_peer(cfg, Some(virtual_addr), &permitidos, agora)
                    .is_ok()
                {
                    self.vagas.insert(
                        no.key,
                        Vaga {
                            wg: wg_key,
                            permitidos,
                        },
                    );
                }
            }
        }
        let sumiram: Vec<NodePublic> = self
            .vagas
            .keys()
            .filter(|k| !vistos.contains(k))
            .copied()
            .collect();
        for chave in sumiram {
            if let Some(v) = self.vagas.remove(&chave) {
                self.device.remove_peer(&v.wg);
            }
            self.magic.remove_peer(&chave);
            self.pendentes.remove(&chave);
        }
    }

    /// Cria a pilha (a primeira vez que se sabe o nosso endereço) ou atualiza as rotas.
    fn sincronizar_pilha(&mut self) {
        let enderecos = self.netmap.addresses();
        if enderecos.is_empty() {
            return;
        }
        let rotas = self.netmap.routes();
        if self.stack.is_none() || enderecos != self.enderecos {
            self.criar_pilha(enderecos, rotas);
        } else if rotas != self.rotas {
            self.rotas.clone_from(&rotas);
            if let Some(tx) = &self.routes_tx {
                let _ = tx.send(rotas);
            }
        }
    }

    fn criar_pilha(&mut self, enderecos: Vec<Ipv4Addr>, rotas: Vec<NetCidr>) {
        let (to_stack, from_engine) = mpsc::channel(FILA);
        let (unreachable_tx, unreachable_rx) = mpsc::unbounded_channel();
        let (routes_tx, routes_rx) = mpsc::unbounded_channel();
        let tunnel = TailscaleTunnel {
            addresses: enderecos
                .iter()
                .filter_map(|a| NetCidr::new(IpAddr::V4(*a), 32))
                .collect(),
            routes: rotas.clone(),
            to_engine: self.to_engine.clone(),
            from_engine,
            unreachable: unreachable_rx,
            routes_rx,
            shared: self.shared.clone(),
        };
        let nova = NetStack::spawn(tunnel);
        self.to_stack = Some(to_stack);
        self.unreachable = Some(unreachable_tx);
        self.routes_tx = Some(routes_tx);
        self.enderecos = enderecos;
        self.rotas = rotas;
        *self
            .shared
            .stack
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(nova.clone());
        self.bridge.attach(nova.clone());
        if let Some(velha) = self.stack.replace(nova) {
            tokio::spawn(async move { velha.shutdown().await });
        }
    }

    // ----- o DERP de casa -----

    /// Escolhe a região de casa (em segundo plano, medindo a latência de cada uma).
    fn escolher_casa(&mut self) {
        if self.escolhendo_casa {
            return;
        }
        self.escolhendo_casa = true;
        self.ultima_escolha = Some(Instant::now());
        let mapa = self.netmap.derp_map().clone();
        let tx = self.casa_tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(pick_home(&mapa).await);
        });
    }

    fn casa_escolhida(&mut self, regiao: Option<DerpRegionId>, map: &MapHandle) {
        self.escolhendo_casa = false;
        if let Some(r) = regiao {
            self.regions.set_home(r);
            self.anunciar(map);
            self.recalcular_estado();
        }
    }

    // ----- o STUN e os endpoints -----

    /// Os servidores STUN do mapa do DERP (os de casa primeiro).
    fn servidores_stun(&self) -> Vec<SocketAddr> {
        let casa = self.regions.home();
        let mut regioes: Vec<_> = self
            .netmap
            .derp_map()
            .regions
            .iter()
            .filter_map(|(id, r)| r.as_ref().map(|r| (*id, r)))
            .filter(|(_, r)| !r.avoid)
            .collect();
        regioes.sort_by_key(|(id, _)| (*id != casa, *id));
        regioes
            .into_iter()
            .filter_map(|(_, r)| {
                r.nodes.iter().find_map(|n| {
                    if n.stun_port < 0 {
                        return None;
                    }
                    let ip: IpAddr = n.ipv4.parse().ok().or_else(|| n.host_name.parse().ok())?;
                    let porta = if n.stun_port == 0 {
                        stun::PORTA_PADRAO
                    } else {
                        u16::try_from(n.stun_port).ok()?
                    };
                    Some(SocketAddr::new(ip, porta))
                })
            })
            .take(REGIOES_STUN)
            .collect()
    }

    fn pedir_stun(&mut self, agora: Instant) {
        let Some(under) = self.under.clone() else {
            return;
        };
        self.stun
            .pendentes
            .retain(|_, (_, quando)| agora.duration_since(*quando) < Duration::from_secs(10));
        for servidor in self.servidores_stun() {
            let mut tx_id: TxId = [0; 12];
            if getrandom::fill(&mut tx_id).is_err() {
                return;
            }
            self.stun.pendentes.insert(tx_id, (servidor, agora));
            let _ = under.try_send_to(&stun::request(&tx_id), servidor);
        }
    }

    fn stun_chegou(&mut self, dados: &[u8], agora: Instant) {
        let Some((tx_id, visto)) = stun::parse_response(dados) else {
            return;
        };
        // Só vale a resposta a um pedido nosso.
        let Some((servidor, _)) = self.stun.pendentes.remove(&tx_id) else {
            return;
        };
        self.stun.vistos.insert(servidor, (visto, agora));
    }

    /// Onde nos alcançar: o endereço local e o que o STUN viu.
    fn endpoints(&self, agora: Instant) -> Vec<(SocketAddr, EndpointType)> {
        let Some(under) = &self.under else {
            return Vec::new();
        };
        let mut saida: Vec<(SocketAddr, EndpointType)> = Vec::new();
        let mut poe = |a: SocketAddr, t: EndpointType| {
            if !saida.iter().any(|(x, _)| *x == a) {
                saida.push((a, t));
            }
        };
        let Ok(local) = under.local_addr() else {
            return saida;
        };
        let alvo = self
            .servidores_stun()
            .first()
            .copied()
            .unwrap_or_else(|| SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)), 9));
        let ip_local = if local.ip().is_unspecified() {
            under.local_ip_toward(alvo)
        } else {
            Some(local.ip())
        };
        if let Some(ip) = ip_local.filter(|ip| !ip.is_unspecified()) {
            poe(SocketAddr::new(ip, local.port()), EndpointType::LOCAL);
        }
        let mut vistos: Vec<_> = self
            .stun
            .vistos
            .iter()
            .filter(|(_, (_, quando))| agora.duration_since(*quando) < VALIDADE_STUN)
            .map(|(_, (a, _))| *a)
            .collect();
        vistos.sort();
        for a in vistos {
            let tipo = if a.port() == local.port() {
                EndpointType::STUN4_LOCAL_PORT
            } else {
                EndpointType::STUN
            };
            poe(a, tipo);
        }
        saida
    }

    /// Conta ao controle o `Hostinfo` e os endpoints, se mudaram, e passa os endpoints ao
    /// `Magic` (para o `CallMeMaybe`).
    fn anunciar(&mut self, map: &MapHandle) {
        let agora = Instant::now();
        let endpoints = self.endpoints(agora);
        self.magic
            .set_our_endpoints(endpoints.iter().map(|(a, _)| *a).collect());
        let novo = MapAnnounce {
            endpoints,
            hostinfo: self.hostinfo(),
        };
        if novo != self.anunciado {
            self.anunciado = novo.clone();
            map.announce(novo);
        }
    }

    // ----- o estado e os pares -----

    fn recalcular_estado(&mut self) {
        if !self.tem_mapa {
            return;
        }
        if let Some(motivo) = &self.desconectado {
            self.aviso.estado(State::Retrying(motivo.clone()));
            return;
        }
        if self.netmap.me().is_some_and(|eu| eu.expired) {
            self.aviso.estado(State::Expired);
            return;
        }
        let Some(address) = self.netmap.addresses().first().copied() else {
            return;
        };
        let peers_online = self
            .netmap
            .peers()
            .filter(|p| p.online == Some(true))
            .count();
        let derp_home = self.nome_regiao(self.regions.home());
        self.aviso.estado(State::Connected {
            address: IpAddr::V4(address),
            tailnet: self.netmap.domain().trim_end_matches('.').to_string(),
            peers_online,
            derp_home,
        });
    }

    fn nome_regiao(&self, regiao: DerpRegionId) -> String {
        if regiao == 0 {
            return String::new();
        }
        self.netmap
            .derp_map()
            .regions
            .get(&regiao)
            .and_then(Option::as_ref)
            .map(|r| {
                if r.region_name.is_empty() {
                    r.region_code.clone()
                } else {
                    r.region_name.clone()
                }
            })
            .unwrap_or_else(|| format!("região {regiao}"))
    }

    fn caminho(&self, chave: &NodePublic) -> String {
        match self.magic.path(chave, Instant::now()) {
            Path::Direct(a) => format!("direto ({a})"),
            Path::Derp(r) => format!("DERP ({})", self.nome_regiao(r)),
            Path::None => String::new(),
        }
    }

    fn informacoes(&self) -> Vec<Peer> {
        self.netmap
            .peers()
            .map(|no| Peer {
                name: nome_curto(&no.name),
                dns_name: no.name.trim_end_matches('.').to_string(),
                addresses: no.addresses.iter().filter_map(|a| so_ip(a)).collect(),
                os: no
                    .hostinfo
                    .as_ref()
                    .map(|h| h.os.clone())
                    .unwrap_or_default(),
                online: no.online,
                last_seen: no.last_seen.map(SystemTime::from),
                user: self
                    .netmap
                    .user(no.user)
                    .map(|u| u.login_name.clone())
                    .unwrap_or_default(),
                tags: no.tags.clone(),
                path: if self.vagas.contains_key(&no.key) {
                    self.caminho(&no.key)
                } else {
                    String::new()
                },
            })
            .collect()
    }

    // ----- o laço -----

    /// Espera o motor ser parado, sem fazer mais nada (depois de uma falha).
    async fn esperar_parar(
        cmds: &mut mpsc::UnboundedReceiver<Command>,
    ) -> Option<oneshot::Sender<()>> {
        loop {
            match cmds.recv().await {
                Some(Command::Stop(ack)) => return Some(ack),
                None => return None,
                Some(Command::Peers(r)) => {
                    let _ = r.send(Vec::new());
                }
                Some(Command::Logout(r)) => {
                    let _ = r.send(Err("esta máquina não está na tailnet".to_string()));
                }
            }
        }
    }

    async fn encerrar(mut self, mapa: Option<MapHandle>) {
        if let Some(m) = mapa {
            let _ = m.stop().await;
        }
        self.regions.close();
        self.bridge.detach();
        if let Some(pilha) = self.stack.take() {
            pilha.shutdown().await;
        }
        self.bridge.stop().await;
    }

    async fn run(
        mut self,
        mut control: Control,
        mut da_pilha: mpsc::Receiver<Vec<u8>>,
        mut cmds: mpsc::UnboundedReceiver<Command>,
        mut derp_rx: mpsc::UnboundedReceiver<(DerpRegionId, DerpEvent)>,
        mut casa_rx: mpsc::UnboundedReceiver<Option<DerpRegionId>>,
    ) {
        self.aviso.estado(State::Connecting);
        let registro = match self.registrar(&mut control, &mut cmds).await {
            Ok(r) => r,
            Err(fim) => {
                let ack = match fim {
                    Fim::Parar(ack) => ack,
                    Fim::Estado(estado) => {
                        self.aviso.estado(estado);
                        Self::esperar_parar(&mut cmds).await
                    }
                    Fim::Sair => None,
                };
                self.aviso.estado(State::Stopped);
                self.encerrar(None).await;
                if let Some(a) = ack {
                    let _ = a.send(());
                }
                return;
            }
        };
        let login = if registro.login.login_name.is_empty() {
            registro.user.display_name.clone()
        } else {
            registro.login.login_name.clone()
        };
        self.aviso.node(|n| n.login = login);
        self.aviso.estado(State::Connecting);

        let mut config = MapConfig::new(self.node_key.public(), self.magic.disco_public());
        config.accept_routes = self.cfg.accept_routes;
        config.announce = MapAnnounce {
            endpoints: self.endpoints(Instant::now()),
            hostinfo: self.hostinfo(),
        };
        self.anunciado = config.announce.clone();
        self.magic
            .set_our_endpoints(config.announce.endpoints.iter().map(|(a, _)| *a).collect());
        let mut map = map::spawn(control, config);

        let under = self.under.clone();
        let mut buf = vec![0u8; 65_535];
        let mut tique = tokio::time::interval(TIQUE);
        tique.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut ack: Option<oneshot::Sender<()>> = None;
        let mut logout: Option<oneshot::Sender<Result<(), String>>> = None;
        let mut falhou = false;
        loop {
            tokio::select! {
                recebido = async {
                    match &under {
                        Some(u) => poll_fn(|cx| u.poll_recv_from(cx, &mut buf)).await,
                        None => pending().await,
                    }
                } => match recebido {
                    Ok((n, de)) => self.udp_chegou(&buf[..n], de),
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
                        self.aviso.estado(State::Failed(format!("o UDP do nó parou: {e}")));
                        falhou = true;
                        break;
                    }
                },
                evento = derp_rx.recv() => {
                    if let Some((regiao, e)) = evento {
                        self.derp_chegou(regiao, e);
                    }
                }
                casa = casa_rx.recv() => {
                    if let Some(r) = casa {
                        self.casa_escolhida(r, &map);
                    }
                }
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
                    None => break,
                },
                evento = map.recv() => match evento {
                    Some(MapEvent::Connected) => {}
                    Some(MapEvent::Update { netmap, change, first }) => {
                        self.aplicar_mapa(netmap, &change, first);
                        self.anunciar(&map);
                    }
                    Some(MapEvent::Disconnected(e)) => {
                        let motivo = e.to_string();
                        self.desconectado = Some(motivo.clone());
                        if self.tem_mapa {
                            self.aviso.estado(State::Retrying(motivo));
                        }
                    }
                    Some(MapEvent::Failed(e)) => {
                        self.aviso.estado(State::Failed(e.to_string()));
                        falhou = true;
                        break;
                    }
                    None => break,
                },
                _ = tique.tick() => {
                    self.tique(&map);
                }
                comando = cmds.recv() => match comando {
                    Some(Command::Peers(resposta)) => {
                        let _ = resposta.send(self.informacoes());
                    }
                    Some(Command::Logout(resposta)) => {
                        logout = Some(resposta);
                        break;
                    }
                    Some(Command::Stop(a)) => {
                        ack = Some(a);
                        break;
                    }
                    None => break,
                },
            }
        }
        if falhou {
            // O estado `Failed` fica à vista até alguém parar o nó.
            let a = Self::esperar_parar(&mut cmds).await;
            self.aviso.estado(State::Stopped);
            self.encerrar(Some(map)).await;
            if let Some(a) = a {
                let _ = a.send(());
            }
            return;
        }
        if let Some(resposta) = logout {
            let params = self.params(&self.node_key.clone(), None);
            let resultado = match map.stop().await {
                Some(mut c) => c.logout(&params).await.map_err(|e| e.to_string()),
                None => Err("a conexão com o controle já tinha acabado".to_string()),
            };
            self.aviso.estado(State::Stopped);
            self.encerrar(None).await;
            let _ = resposta.send(resultado);
            return;
        }
        self.aviso.estado(State::Stopped);
        self.encerrar(Some(map)).await;
        if let Some(a) = ack {
            let _ = a.send(());
        }
    }

    /// Os relógios: WireGuard, disco, DERP, STUN e a casa.
    fn tique(&mut self, map: &MapHandle) {
        let agora = Instant::now();
        self.bombear(None);
        let mut saida = Vec::new();
        self.magic.tick(agora, &mut saida);
        self.executar(saida);
        self.regions.tick(agora);
        if self.tem_mapa {
            if self.stun.proxima.is_none_or(|t| agora >= t) {
                self.stun.proxima = Some(agora + INTERVALO_STUN);
                self.pedir_stun(agora);
            }
            if self.regions.home() == 0
                && !self.escolhendo_casa
                && self
                    .ultima_escolha
                    .is_none_or(|t| agora.duration_since(t) >= REPETE_CASA)
            {
                self.escolher_casa();
            }
            self.anunciar(map);
        }
        self.avisados
            .retain(|_, t| agora.duration_since(*t) < REPETE_AVISO * 10);
    }
}

fn desligada(nome: &str, visto: Option<jiff::Timestamp>) -> String {
    match visto {
        Some(t) => format!(
            "{nome} está desligada (visto por último em {})",
            data_legivel(t)
        ),
        None => format!("{nome} está desligada"),
    }
}
