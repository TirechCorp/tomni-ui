//! O nó em execução (M61): liga o [`Vl1`] e o [`Vl2`] ao UDP, à pilha TCP/IP em software e à
//! ponte SOCKS5 interna que os clientes do app usam.
//!
//! Nada aqui cria interface de rede, mexe em rota do sistema ou pede administrador, e **nada da
//! rede virtual fica à mostra para outros programas**: o único acesso é a ponte SOCKS5 em
//! `127.0.0.1`, com porta e credencial aleatórias. O nó é invisível: só responde a quem ele
//! mesmo procurou, a um raiz ou ao controlador (veja [`crate::vl1`]).
//!
//! O estado que precisa sobreviver entre execuções (a identidade, o planet, a última
//! configuração assinada) sai pelo [`ZeroTierBuilder::on_node_state`]; quem grava é o app, no
//! cofre. O crate não grava nada em disco.

use std::fmt;
use std::future::poll_fn;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::task::{Context, Poll};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::{Instant, MissedTickBehavior, interval};
use tomni_ui_net::Proxy;
use tomni_ui_netstack::ethernet::{EthernetTunnel, FrameTunnel, Route};
use tomni_ui_netstack::{
    Answer, Cidr, DnsOverTcp, DnsScope, NetStack, Resolver, SocksAccess, SocksBridge,
    SocksListener, UdpUnderlay, Underlay,
};

use crate::netconf::NetworkConfig;
use crate::peer::{Peer, PeerRole, RemoteVersion};
use crate::vl1::{Config, Delivery, Effect, Vl1};
use crate::vl2::{Event, Status, Vl2};
use crate::{Address, Identity, NetworkId, World, WorldKind};

/// Com que frequência o relógio acorda o nó. O VL1 e o VL2 só agem nos seus próprios prazos;
/// acordar mais seguido só limita o atraso.
const TIQUE: Duration = Duration::from_secs(1);

/// O tamanho das filas de quadros entre a pilha e o nó. Fila cheia = quadro perdido, como numa
/// placa de rede; o TCP retransmite.
const FILA: usize = 1024;

/// Quantos caminhos diretos o `NodeState` guarda (os mais recentes).
const MEMORIZADOS: usize = 64;

/// O maior datagrama que o nó lê (o protocolo manda no máximo 1432 bytes; o que chega de fora
/// pode ser maior e é descartado pelo VL1).
const LEITURA: usize = 2048;

/// Quanto se espera por um raiz antes de dizer que nenhum respondeu.
const SEM_RAIZ: Duration = Duration::from_secs(30);

/// O MTU mínimo e o máximo que a pilha aceita (`ZT_MIN_MTU` e `ZT_MAX_MTU`).
const MTU_MINIMO: usize = 1280;
const MTU_MAXIMO: usize = 10_000;

/// A versão que o nó anuncia aos outros: a do ZeroTier 1.14.2, que fala o protocolo 13 (AES-GMAC-SIV
/// com quem anuncia 12 ou mais).
const VERSAO: RemoteVersion = RemoteVersion {
    protocol: 13,
    major: 1,
    minor: 14,
    revision: 2,
};

const MENSAGEM_SEM_IP: &str = "A rede ainda não atribuiu um IP a este membro. Peça ao administrador para atribuir um (ou informe um IP fixo nas opções da rede).";
const MENSAGEM_COLISAO: &str = "Outro membro está usando esta mesma identidade (o ZeroTier do sistema com a identidade importada?).";
const MENSAGEM_SSO: &str =
    "A rede exige login pelo navegador (SSO), que esta versão ainda não faz.";

/// Em que pé está o nó.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// Subindo: procurando um raiz.
    Connecting,
    /// Falando com os raiz; pedindo a configuração ao controlador.
    WaitingConfig,
    /// O controlador recusou: o administrador precisa autorizar este endereço.
    NotAuthorized {
        /// O endereço deste membro, o que o administrador autoriza.
        address: Address,
    },
    /// A rede não existe nesse controlador.
    NotFound,
    /// A rede exige login único pelo navegador (ainda não tratado).
    AuthenticationRequired,
    /// Conectado: a rede tem configuração e a pilha está de pé.
    Connected {
        /// O IP deste nó na rede.
        address: Ipv4Addr,
        /// O nome da rede.
        name: String,
        /// Avisos da configuração (por exemplo, regras que o nó não trata por inteiro).
        warnings: Vec<String>,
    },
    /// Tentando de novo; o texto diz o que falta.
    Retrying(String),
    /// Parou de vez por um erro.
    Failed(String),
    /// Parado por quem o iniciou.
    Stopped,
}

/// O que precisa sobreviver entre execuções; o app guarda no cofre.
#[derive(Clone, Default)]
pub struct NodeState {
    /// Gerada no primeiro uso; sem ela, cada execução seria um membro novo.
    pub identity: Option<Identity>,
    /// O último planet aceito (bytes do formato do mundo). Vazio = o embutido.
    pub planet: Vec<u8>,
    /// As moons recebidas (bytes).
    pub moons: Vec<Vec<u8>>,
    /// As moons que pedimos para orbitar: o id e o endereço de um raiz delas.
    pub orbits: Vec<(u64, Address)>,
    /// A última configuração assinada desta rede (os pedaços juntos, com a assinatura),
    /// conferida de novo ao carregar: com ela o nó sobe com o controlador fora do ar.
    pub network_config: Vec<u8>,
    /// O último caminho direto bom de cada par, para tentar primeiro.
    pub memorized_paths: Vec<(Address, SocketAddr)>,
    /// O nome da rede, para mostrar sem conectar.
    pub network_name: String,
    /// Os IPs da rede na última vez, para mostrar sem conectar.
    pub addresses: Vec<Cidr>,
}

impl fmt::Debug for NodeState {
    /// Sem segredo: a identidade aparece só pelo endereço.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NodeState")
            .field("identity", &self.identity)
            .field("planet", &self.planet.len())
            .field("moons", &self.moons.len())
            .field("orbits", &self.orbits)
            .field("network_config", &self.network_config.len())
            .field("memorized_paths", &self.memorized_paths.len())
            .field("network_name", &self.network_name)
            .field("addresses", &self.addresses)
            .finish()
    }
}

/// O que, no [`NodeState`], faz o app gravar de novo quando muda (tudo menos a identidade, que
/// só nasce no início).
#[derive(PartialEq)]
struct Assinatura {
    planet: Vec<u8>,
    moons: Vec<Vec<u8>>,
    orbits: Vec<(u64, Address)>,
    network_config: Vec<u8>,
    memorized_paths: Vec<(Address, SocketAddr)>,
    network_name: String,
    addresses: Vec<Cidr>,
}

impl NodeState {
    fn assinatura(&self) -> Assinatura {
        Assinatura {
            planet: self.planet.clone(),
            moons: self.moons.clone(),
            orbits: self.orbits.clone(),
            network_config: self.network_config.clone(),
            memorized_paths: self.memorized_paths.clone(),
            network_name: self.network_name.clone(),
            addresses: self.addresses.clone(),
        }
    }
}

/// Um par, como o nó o mostra.
#[derive(Clone, Debug)]
pub struct PeerInfo {
    /// O endereço.
    pub address: Address,
    /// O papel: raiz, moon, controlador ou membro.
    pub role: PeerRole,
    /// Por onde vão os pacotes: `direto (192.0.2.7:9993)` ou `pelo raiz`; vazio quando ainda
    /// não há caminho.
    pub path: String,
    /// A latência do melhor caminho vivo.
    pub latency: Option<Duration>,
    /// A versão anunciada (`1.14.2 (protocolo 13)`); vazio se o par ainda não se apresentou.
    pub version: String,
}

type OnState = Arc<dyn Fn(&State) + Send + Sync>;
type OnNode = Arc<dyn Fn(&NodeState) + Send + Sync>;

/// O que o nó, o túnel e o [`ZeroTier`] dividem.
struct Compartilhado {
    stack: Mutex<Option<NetStack>>,
    /// O DNS da rede, quando a configuração traz um e o app permitiu.
    dns: RwLock<Option<Arc<DnsOverTcp>>>,
}

/// Quem resolve os nomes para a ponte: os do domínio da rede vão aos servidores dela, por
/// dentro do túnel (só com `allow_dns`); o resto segue para o resolvedor do computador.
struct ZtDns(Arc<Compartilhado>);

impl Resolver for ZtDns {
    fn resolve<'a>(
        &'a self,
        stack: &'a NetStack,
        name: &'a str,
    ) -> Pin<Box<dyn Future<Output = Answer> + Send + 'a>> {
        let dns = self
            .0
            .dns
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        Box::pin(async move {
            match dns {
                Some(d) => d.resolve(stack, name).await,
                None => Answer::Skip,
            }
        })
    }
}

/// O túnel de quadros Ethernet entre a pilha e o nó.
struct ZtTunnel {
    mtu: usize,
    mac: [u8; 6],
    addresses: Vec<Cidr>,
    routes: Vec<Route>,
    to_engine: mpsc::Sender<Vec<u8>>,
    from_engine: mpsc::Receiver<Vec<u8>>,
    routes_rx: mpsc::UnboundedReceiver<Vec<Route>>,
}

impl FrameTunnel for ZtTunnel {
    fn mtu(&self) -> usize {
        self.mtu
    }

    fn mac(&self) -> [u8; 6] {
        self.mac
    }

    fn addresses(&self) -> Vec<Cidr> {
        self.addresses.clone()
    }

    fn routes(&self) -> Vec<Route> {
        self.routes.clone()
    }

    fn send_frame(&mut self, frame: &[u8]) {
        let _ = self.to_engine.try_send(frame.to_vec());
    }

    fn poll_recv_frame(
        &mut self,
        cx: &mut Context<'_>,
        frame: &mut Vec<u8>,
    ) -> Poll<io::Result<()>> {
        match self.from_engine.poll_recv(cx) {
            Poll::Ready(Some(quadro)) => {
                frame.clear();
                frame.extend_from_slice(&quadro);
                Poll::Ready(Ok(()))
            }
            Poll::Ready(None) => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "a rede ZeroTier parou",
            ))),
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_routes(&mut self, cx: &mut Context<'_>) -> Poll<Vec<Route>> {
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

/// O que o [`ZeroTier`] pede ao nó.
enum Command {
    Stop(oneshot::Sender<()>),
    Peers(oneshot::Sender<Vec<PeerInfo>>),
    Roots(oneshot::Sender<Vec<PeerInfo>>),
}

/// As opções de quem usa.
struct Opcoes {
    allow_global: bool,
    allow_default: bool,
    allow_dns: bool,
    static_address: Option<Cidr>,
}

/// O monta o nó. Veja [`ZeroTierBuilder::start`].
pub struct ZeroTierBuilder {
    network: NetworkId,
    state: NodeState,
    opcoes: Opcoes,
    port: u16,
    planet: Option<Vec<u8>>,
    tcp_buffer: Option<usize>,
    answer_all_arp: bool,
    any_ip: bool,
    direct_paths: bool,
    push_paths: bool,
    extended_hello: bool,
    protocol: Option<u8>,
    on_state: Option<OnState>,
    on_node: Option<OnNode>,
}

impl ZeroTierBuilder {
    /// Um nó da rede `network`, com o estado que o app guardou (vazio na primeira vez).
    pub fn new(network: NetworkId, state: NodeState) -> Self {
        Self {
            network,
            state,
            opcoes: Opcoes {
                allow_global: false,
                allow_default: false,
                allow_dns: false,
                static_address: None,
            },
            port: 0,
            planet: None,
            tcp_buffer: None,
            answer_all_arp: false,
            any_ip: false,
            direct_paths: true,
            push_paths: true,
            extended_hello: false,
            protocol: None,
            on_state: None,
            on_node: None,
        }
    }

    /// Aceitar IPs e rotas gerenciados em faixas públicas (padrão: não).
    pub fn allow_global(mut self, sim: bool) -> Self {
        self.opcoes.allow_global = sim;
        self
    }

    /// Aceitar a rota padrão da rede (padrão: não).
    pub fn allow_default(mut self, sim: bool) -> Self {
        self.opcoes.allow_default = sim;
        self
    }

    /// Usar o DNS da rede para os nomes do domínio dela (padrão: não).
    pub fn allow_dns(mut self, sim: bool) -> Self {
        self.opcoes.allow_dns = sim;
        self
    }

    /// Um IP fixo para redes que não distribuem endereço (o administrador diz qual). Só vale
    /// se o controlador não atribuir nenhum.
    pub fn static_address(mut self, ip: Cidr) -> Self {
        self.opcoes.static_address = Some(ip);
        self
    }

    /// A porta UDP local; 0 = aleatória (o padrão: a 9993 costuma ser do ZeroTier do
    /// sistema).
    pub fn port(mut self, porta: u16) -> Self {
        self.port = porta;
        self
    }

    /// Outro planet (testes; redes privadas com raiz próprios), no formato do arquivo `planet`.
    pub fn planet(mut self, bytes: Vec<u8>) -> Self {
        self.planet = Some(bytes);
        self
    }

    /// O tamanho do buffer de cada conexão TCP da pilha.
    pub fn tcp_buffer(mut self, bytes: usize) -> Self {
        self.tcp_buffer = Some(bytes);
        self
    }

    /// Manda o HELLO na armadura estendida (chave efêmera) aos pares que já anunciaram o
    /// protocolo 13, como faz o ZeroTier com `encryptedHelloEnabled`. Desligado por padrão, como
    /// no 1.14.2; o ZeroTier de verdade aceita receber nos dois casos.
    pub fn extended_hello(mut self, sim: bool) -> Self {
        self.extended_hello = sim;
        self
    }

    /// Sem caminho direto: tudo pelos raiz (diagnóstico e testes).
    pub fn direct_paths(mut self, sim: bool) -> Self {
        self.direct_paths = sim;
        self
    }

    /// Não anuncia os nossos endereços aos pares (PUSH_DIRECT_PATHS): o furo de NAT depende só
    /// do RENDEZVOUS do raiz. Para os testes isolarem um dos dois mecanismos.
    #[cfg(feature = "test-util")]
    pub fn push_direct_paths(mut self, sim: bool) -> Self {
        self.push_paths = sim;
        self
    }

    /// Responde ao ARP de qualquer um (o lado que escuta nos testes; o app de verdade não faz isso).
    #[cfg(feature = "test-util")]
    pub fn answer_all_arp(mut self, sim: bool) -> Self {
        self.answer_all_arp = sim;
        self
    }

    /// Aceita pacotes para qualquer IP: faz o papel de gateway nos testes.
    #[cfg(feature = "test-util")]
    pub fn any_ip(mut self, sim: bool) -> Self {
        self.any_ip = sim;
        self
    }

    /// A versão de protocolo anunciada (testes das duas cifras).
    #[cfg(feature = "test-util")]
    pub fn protocol_version(mut self, versao: u8) -> Self {
        self.protocol = Some(versao);
        self
    }

    /// Chamado a cada mudança de estado.
    pub fn on_state(mut self, f: impl Fn(&State) + Send + Sync + 'static) -> Self {
        self.on_state = Some(Arc::new(f));
        self
    }

    /// Chamado quando o [`NodeState`] muda (identidade nova, planet, configuração, caminhos):
    /// o app grava no cofre.
    pub fn on_node_state(mut self, f: impl Fn(&NodeState) + Send + Sync + 'static) -> Self {
        self.on_node = Some(Arc::new(f));
        self
    }

    /// Orbita uma moon: o id e o endereço de um raiz dela.
    pub fn orbit(mut self, moon: u64, semente: Address) -> Self {
        if !self.state.orbits.iter().any(|(id, _)| *id == moon) {
            self.state.orbits.push((moon, semente));
        }
        self
    }

    /// Abre o UDP, a ponte SOCKS5 e inicia o nó, no runtime atual. Volta quando a identidade
    /// existe (se não havia, criar uma leva um instante); a conexão acontece em segundo plano
    /// (veja [`ZeroTier::state`] e [`ZeroTier::wait_connected`]).
    ///
    /// # Errors
    ///
    /// Se a porta UDP não abre, a identidade não se cria ou o planet não serve. A mensagem é em
    /// português.
    pub async fn start(self) -> io::Result<ZeroTier> {
        let porta = self.port;
        let (under, _aviso) = UdpUnderlay::bind("0.0.0.0", porta).await.map_err(|e| {
            io::Error::new(
                e.kind(),
                format!(
                    "Não foi possível abrir a porta UDP {porta}: {e}. Deixe a porta em 0 (aleatória)."
                ),
            )
        })?;
        self.iniciar(Arc::new(under)).await
    }

    /// Como [`start`](ZeroTierBuilder::start), mas sobre um UDP dado (os testes usam a rede
    /// simulada do `tomni-ui-netstack`).
    ///
    /// # Errors
    ///
    /// Os mesmos de [`start`](ZeroTierBuilder::start), menos a porta.
    pub async fn start_on(self, under: impl Underlay) -> io::Result<ZeroTier> {
        self.iniciar(Arc::new(under)).await
    }

    async fn iniciar(self, under: Arc<dyn Underlay>) -> io::Result<ZeroTier> {
        let invalido = |e: String| io::Error::new(io::ErrorKind::InvalidInput, e);
        let mut estado = self.state;

        // O planet de partida: o do builder (ou o oficial) e, se o que o app guardou for uma
        // versão mais nova do mesmo planet, ele.
        let base = match &self.planet {
            Some(bytes) => {
                World::parse(bytes).map_err(|e| invalido(format!("O planet não serve: {e}.")))?
            }
            None => World::terra(),
        };
        let planet = match World::parse(&estado.planet) {
            Ok(guardado)
                if guardado.kind() == WorldKind::Planet
                    && guardado.id() == base.id()
                    && guardado.timestamp() > base.timestamp() =>
            {
                guardado
            }
            _ => base,
        };
        let moons: Vec<World> = estado
            .moons
            .iter()
            .filter_map(|b| World::parse(b).ok())
            .filter(|m| m.kind() == WorldKind::Moon)
            .collect();

        let gerou = estado.identity.is_none();
        let identity = match estado.identity.clone() {
            Some(i) => i,
            None => tokio::task::spawn_blocking(Identity::generate)
                .await
                .map_err(|e| {
                    io::Error::other(format!("Não foi possível criar a identidade: {e}."))
                })?
                .map_err(|e| {
                    io::Error::other(format!("Não foi possível criar a identidade: {e}."))
                })?,
        };
        estado.identity = Some(identity.clone());
        let ours = identity.address();

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
            stack: Mutex::new(None),
            dns: RwLock::new(None),
        });
        let bridge = SocksListener::bind()?
            .resolver(Arc::new(ZtDns(shared.clone())))
            .serve_later()?;
        let access = bridge.access().clone();
        let local_addr = under.local_addr().ok();

        let mut version = VERSAO;
        if let Some(p) = self.protocol {
            version.protocol = p;
        }
        let mut vl1 = Vl1::new(Config {
            identity,
            planet,
            moons,
            orbits: estado.orbits.clone(),
            version,
            direct_paths: self.direct_paths,
        });
        vl1.set_controller(self.network.controller());
        vl1.set_memorized(estado.memorized_paths.clone());
        vl1.set_push_direct_paths(self.push_paths);
        vl1.set_extended_hello(self.extended_hello);
        if let Some(local) = local_addr {
            let resolve = under.clone();
            vl1.set_local(
                local.port(),
                Arc::new(move |alvo| resolve.local_ip_toward(alvo)),
            );
        }
        let mut vl2 = Vl2::new(
            self.network,
            ours,
            version,
            std::env::consts::OS,
            estado.network_config.clone(),
        );
        vl2.set_static_addresses(self.opcoes.static_address.into_iter().collect());

        let (to_engine_tx, to_engine_rx) = mpsc::channel(FILA);
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let agora = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
        let motor = Motor {
            network: self.network,
            ours,
            under,
            vl1,
            vl2,
            aviso: aviso.clone(),
            shared: shared.clone(),
            bridge,
            opcoes: self.opcoes,
            tcp_buffer: self.tcp_buffer,
            answer_all_arp: self.answer_all_arp,
            any_ip: self.any_ip,
            to_engine: to_engine_tx,
            to_stack: None,
            routes_tx: None,
            stack: None,
            enderecos: Vec::new(),
            rotas: Vec::new(),
            mtu: 0,
            nome: String::new(),
            avisos: Vec::new(),
            rejeitada: None,
            vl2_ligado: false,
            inicio: Instant::now(),
            base: agora,
        };
        // As moons orbitadas e o planet em uso vão para o `NodeState` já no começo.
        motor.gravar_mundos();
        let task = tokio::spawn(motor.run(to_engine_rx, cmd_rx));
        Ok(ZeroTier {
            commands: cmd_tx,
            state: state_rx,
            task: Some(task),
            access,
            shared,
            address: ours,
            network: self.network,
            local_addr,
        })
    }
}

/// Um nó de uma rede ZeroTier em funcionamento, só para o tráfego do app.
///
/// Ao ser largado (ou em [`stop`](ZeroTier::stop)), para o nó.
pub struct ZeroTier {
    commands: mpsc::UnboundedSender<Command>,
    state: watch::Receiver<State>,
    task: Option<JoinHandle<()>>,
    access: SocksAccess,
    shared: Arc<Compartilhado>,
    address: Address,
    network: NetworkId,
    local_addr: Option<SocketAddr>,
}

impl fmt::Debug for ZeroTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ZeroTier")
            .field("address", &self.address)
            .field("state", &*self.state.borrow())
            .finish_non_exhaustive()
    }
}

impl ZeroTier {
    /// O proxy SOCKS5 que leva as conexões pela rede: entregue-o ao cliente de SSH, SFTP,
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

    /// A pilha TCP, para abrir conexões direto, sem passar pelo SOCKS5. `None` até a
    /// configuração chegar (é quando se sabe o IP do nó).
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

    /// O endereço deste nó, o que o administrador da rede autoriza.
    pub fn address(&self) -> Address {
        self.address
    }

    /// A rede.
    pub fn network(&self) -> NetworkId {
        self.network
    }

    /// A porta UDP local (para os testes e o diagnóstico).
    pub fn udp_addr(&self) -> Option<SocketAddr> {
        self.local_addr
    }

    /// Espera o estado `Connected` e devolve o IP do nó na rede.
    ///
    /// # Errors
    ///
    /// Se passa `limit` sem conectar (a mensagem diz o que falta, por exemplo a autorização),
    /// a rede não existe, exige SSO, o nó falhou ou parou.
    pub async fn wait_connected(&self, limit: Duration) -> io::Result<Ipv4Addr> {
        let mut rx = self.state.clone();
        let espera = async {
            loop {
                let atual = rx.borrow_and_update().clone();
                match atual {
                    State::Connected { address, .. } => return Ok(address),
                    State::Failed(motivo) => {
                        return Err(io::Error::other(format!(
                            "a rede ZeroTier falhou: {motivo}"
                        )));
                    }
                    State::NotFound => {
                        return Err(io::Error::other(format!(
                            "A rede {} não existe no controlador {}.",
                            self.network,
                            self.network.controller()
                        )));
                    }
                    State::AuthenticationRequired => {
                        return Err(io::Error::other(MENSAGEM_SSO));
                    }
                    State::Stopped => {
                        return Err(io::Error::new(
                            io::ErrorKind::BrokenPipe,
                            "a rede ZeroTier parou",
                        ));
                    }
                    State::Connecting
                    | State::WaitingConfig
                    | State::NotAuthorized { .. }
                    | State::Retrying(_) => {}
                }
                if rx.changed().await.is_err() {
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "a rede ZeroTier parou",
                    ));
                }
            }
        };
        match tokio::time::timeout(limit, espera).await {
            Ok(resultado) => resultado,
            Err(_) => {
                let detalhe = match self.state() {
                    State::NotAuthorized { address } => mensagem_autorizacao(address),
                    State::Retrying(motivo) => motivo,
                    outro => format!("estado: {outro:?}"),
                };
                Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!(
                        "a rede ZeroTier não conectou em {} s ({detalhe})",
                        limit.as_secs()
                    ),
                ))
            }
        }
    }

    /// Os pares com quem houve tráfego (sem os raiz), com o caminho de cada um.
    pub async fn peers(&self) -> Vec<PeerInfo> {
        let (resposta, espera) = oneshot::channel();
        if self.commands.send(Command::Peers(resposta)).is_err() {
            return Vec::new();
        }
        espera.await.unwrap_or_default()
    }

    /// Os raiz em uso (planet e moons), com a latência de cada um.
    pub async fn roots(&self) -> Vec<PeerInfo> {
        let (resposta, espera) = oneshot::channel();
        if self.commands.send(Command::Roots(resposta)).is_err() {
            return Vec::new();
        }
        espera.await.unwrap_or_default()
    }

    /// Pede para parar sem esperar e sem largar o motor: serve quando há outros donos do
    /// `Arc` (as conexões abertas por ele) e o motor não pode mais ficar ligado. Depois disso,
    /// o estado vai a `Stopped`, como no [`ZeroTier::stop`].
    pub fn close(&self) {
        let (ack, _) = oneshot::channel();
        let _ = self.commands.send(Command::Stop(ack));
    }

    /// Fecha as conexões e para. `espera` limita quanto tempo se espera o nó acabar.
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

impl Drop for ZeroTier {
    fn drop(&mut self) {
        // Largado sem `stop`: ainda assim para o nó, sem esperar.
        let (ack, _) = oneshot::channel();
        let _ = self.commands.send(Command::Stop(ack));
    }
}

fn mensagem_autorizacao(address: Address) -> String {
    format!(
        "Aguardando autorização: peça ao administrador da rede para autorizar o membro {address}."
    )
}

/// O IP está numa faixa que não é da internet (privada, compartilhada de operadora ou local do
/// enlace)? Os IPs e rotas fora dela só valem com `allow_global`.
fn nao_global(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    ip.is_private()
        || ip.is_link_local()
        || ip.is_loopback()
        // 100.64.0.0/10, o espaço compartilhado de operadora (RFC 6598).
        || (o[0] == 100 && (o[1] & 0xc0) == 0x40)
}

/// O nó em si: o que roda na tarefa.
struct Motor {
    network: NetworkId,
    ours: Address,
    under: Arc<dyn Underlay>,
    vl1: Vl1,
    vl2: Vl2,
    aviso: Arc<Aviso>,
    shared: Arc<Compartilhado>,
    bridge: SocksBridge,
    opcoes: Opcoes,
    tcp_buffer: Option<usize>,
    #[cfg_attr(not(feature = "test-util"), allow(dead_code))]
    answer_all_arp: bool,
    #[cfg_attr(not(feature = "test-util"), allow(dead_code))]
    any_ip: bool,
    to_engine: mpsc::Sender<Vec<u8>>,
    to_stack: Option<mpsc::Sender<Vec<u8>>>,
    routes_tx: Option<mpsc::UnboundedSender<Vec<Route>>>,
    stack: Option<NetStack>,
    enderecos: Vec<Cidr>,
    rotas: Vec<Route>,
    mtu: usize,
    nome: String,
    avisos: Vec<String>,
    /// Por que a última configuração recebida não serve, enquanto não vier outra.
    rejeitada: Option<String>,
    /// O VL2 já recebeu o primeiro tique (com um raiz vivo).
    vl2_ligado: bool,
    inicio: Instant,
    base: u64,
}

impl Motor {
    fn agora(&self) -> u64 {
        self.base + u64::try_from(self.inicio.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// Grava no `NodeState` o planet, as moons e as órbitas em uso.
    fn gravar_mundos(&self) {
        let planet = self.vl1.planet().to_bytes();
        let moons: Vec<Vec<u8>> = self.vl1.moons().iter().map(World::to_bytes).collect();
        self.aviso.node(|n| {
            n.planet = planet;
            n.moons = moons;
        });
    }

    /// Guarda o último caminho direto bom do par no `NodeState` (no máximo [`MEMORIZADOS`]).
    fn memorizar_caminho(&self, par: Address, caminho: SocketAddr) {
        self.aviso.node(|n| {
            n.memorized_paths.retain(|(a, _)| *a != par);
            n.memorized_paths.push((par, caminho));
            let sobra = n.memorized_paths.len().saturating_sub(MEMORIZADOS);
            n.memorized_paths.drain(..sobra);
        });
    }

    // ----- o que sobe do VL1 e do VL2 -----

    /// Escoa o que o VL1 e o VL2 produziram: datagramas para o UDP, entregas para o VL2,
    /// quadros para a pilha, mudanças para o app. Repete até não sobrar nada, porque cada
    /// entrega pode gerar envios.
    fn drenar(&mut self, agora: u64) {
        loop {
            let efeitos = self.vl1.take_effects();
            let eventos = self.vl2.take_events();
            if efeitos.is_empty() && eventos.is_empty() {
                break;
            }
            for efeito in efeitos {
                match efeito {
                    // Datagrama perdido não é erro: o protocolo retransmite o que importa.
                    Effect::Send { to, data } => {
                        let _ = self.under.try_send_to(&data, to);
                    }
                    Effect::Deliver(entrega) => self.entregar(agora, entrega),
                    Effect::WorldChanged => self.gravar_mundos(),
                    Effect::DirectPath(par, caminho) => self.memorizar_caminho(par, caminho),
                    Effect::SeenAddress(_) | Effect::PeerKnown(_) => {}
                }
            }
            for evento in eventos {
                self.evento(evento);
            }
        }
    }

    fn entregar(&mut self, agora: u64, entrega: Delivery) {
        self.vl2.handle(agora, &mut self.vl1, entrega);
    }

    fn evento(&mut self, evento: Event) {
        match evento {
            // O estado é recalculado no fim de cada passada.
            Event::Status(_) => {}
            Event::Config(config, assinada) => {
                self.rejeitada = None;
                if !assinada.is_empty() {
                    self.aviso.node(|n| n.network_config = assinada);
                }
                self.aplicar_config(&config);
            }
            Event::ConfigLost => {
                self.aviso.node(|n| n.network_config.clear());
                self.desmontar_pilha();
            }
            Event::Frame(quadro) => {
                if let Some(tx) = &self.to_stack {
                    let _ = tx.try_send(quadro);
                }
            }
            Event::Rejected(motivo) => self.rejeitada = Some(motivo),
        }
    }

    // ----- a configuração e a pilha -----

    /// Aplica a configuração: filtra os IPs e as rotas pelas permissões do app e refaz a pilha
    /// se os IPs ou o MTU mudaram (as conexões abertas caem, como num IP novo de verdade); se
    /// só as rotas mudaram, avisa a pilha.
    fn aplicar_config(&mut self, config: &NetworkConfig) {
        let mut enderecos: Vec<Cidr> = config
            .addresses
            .iter()
            .filter(|c| match c.addr() {
                IpAddr::V4(ip) => self.opcoes.allow_global || nao_global(ip),
                IpAddr::V6(_) => false,
            })
            .copied()
            .collect();
        if enderecos.is_empty()
            && let Some(fixo) = self.opcoes.static_address
        {
            enderecos.push(fixo);
        }

        let mut rotas: Vec<Route> = Vec::new();
        for r in &config.routes {
            let IpAddr::V4(destino) = r.target.addr() else {
                continue;
            };
            let padrao = r.target.prefix() == 0;
            if padrao {
                if !self.opcoes.allow_default {
                    continue;
                }
            } else if !self.opcoes.allow_global && !nao_global(destino) {
                continue;
            }
            let via = match r.via {
                None => None,
                Some(IpAddr::V4(v)) => Some(v),
                Some(IpAddr::V6(_)) => continue,
            };
            rotas.push(Route { net: r.target, via });
        }

        let mtu = usize::from(config.mtu).clamp(MTU_MINIMO, MTU_MAXIMO);
        // Os IPs da rede virtual e das rotas gerenciadas nunca viram caminho físico: seria um
        // túnel dentro do túnel. A rota padrão não conta, senão nada serviria.
        let excluidas: Vec<(Ipv4Addr, u8)> = config
            .addresses
            .iter()
            .copied()
            .chain(config.routes.iter().map(|r| r.target))
            .filter(|c| c.prefix() > 0)
            .filter_map(|c| match c.addr() {
                IpAddr::V4(ip) => Some((ip, c.prefix())),
                IpAddr::V6(_) => None,
            })
            .collect();
        self.vl1.set_excluded(excluidas);
        self.nome.clone_from(&config.name);
        self.avisos = config.warnings.iter().map(ToString::to_string).collect();
        self.aviso.node(|n| {
            n.network_name.clone_from(&config.name);
            n.addresses.clone_from(&enderecos);
        });
        self.vl2
            .set_static_addresses(self.opcoes.static_address.into_iter().collect());
        self.aplicar_dns(config);

        if enderecos.is_empty() {
            // Sem IP não há o que ligar: o estado diz o que falta.
            self.desmontar_pilha();
            self.enderecos = enderecos;
            return;
        }
        if self.stack.is_none() || enderecos != self.enderecos || mtu != self.mtu {
            self.criar_pilha(enderecos, rotas, mtu);
        } else if rotas != self.rotas {
            self.rotas.clone_from(&rotas);
            if let Some(tx) = &self.routes_tx {
                let _ = tx.send(rotas);
            }
        }
    }

    /// O DNS da rede, quando ela traz um e o app permitiu: só os servidores IPv4 (o IPv6 ainda
    /// não entra) e só para os nomes do domínio dela.
    fn aplicar_dns(&self, config: &NetworkConfig) {
        let dns = config
            .dns
            .as_ref()
            .filter(|_| self.opcoes.allow_dns)
            .and_then(|d| {
                let servidores: Vec<Ipv4Addr> = d
                    .servers
                    .iter()
                    .filter_map(|ip| match ip {
                        IpAddr::V4(v) => Some(*v),
                        IpAddr::V6(_) => None,
                    })
                    .collect();
                if servidores.is_empty() || d.domain.is_empty() {
                    return None;
                }
                Some(Arc::new(DnsOverTcp::new(
                    servidores,
                    vec![d.domain.clone()],
                    DnsScope::Domains(vec![d.domain.clone()]),
                )))
            });
        *self
            .shared
            .dns
            .write()
            .unwrap_or_else(PoisonError::into_inner) = dns;
    }

    fn criar_pilha(&mut self, enderecos: Vec<Cidr>, rotas: Vec<Route>, mtu: usize) {
        let (to_stack, from_engine) = mpsc::channel(FILA);
        let (routes_tx, routes_rx) = mpsc::unbounded_channel();
        let tunnel = EthernetTunnel::new(ZtTunnel {
            mtu,
            mac: self.vl2.mac(),
            addresses: enderecos.clone(),
            routes: rotas.clone(),
            to_engine: self.to_engine.clone(),
            from_engine,
            routes_rx,
        });
        #[cfg(feature = "test-util")]
        let tunnel = tunnel.answer_all_arp(self.answer_all_arp);
        let mut construtor = NetStack::builder();
        if let Some(bytes) = self.tcp_buffer {
            construtor = construtor.tcp_buffer(bytes);
        }
        #[cfg(feature = "test-util")]
        {
            construtor = construtor.any_ip(self.any_ip);
        }
        let nova = construtor.spawn(tunnel);
        self.to_stack = Some(to_stack);
        self.routes_tx = Some(routes_tx);
        self.enderecos = enderecos;
        self.rotas = rotas;
        self.mtu = mtu;
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

    /// Derruba a pilha (a rede nos recusou ou não nos deu IP).
    fn desmontar_pilha(&mut self) {
        self.bridge.detach();
        self.to_stack = None;
        self.routes_tx = None;
        *self
            .shared
            .stack
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;
        if let Some(velha) = self.stack.take() {
            tokio::spawn(async move { velha.shutdown().await });
        }
    }

    // ----- o estado -----

    /// Traduz o que o VL2 e a pilha dizem no estado que o app mostra.
    fn recalcular_estado(&self, agora: u64) {
        let estado = match self.vl2.status() {
            Status::Ready => match (&self.stack, self.enderecos.first()) {
                (Some(_), Some(c)) => match c.addr() {
                    IpAddr::V4(address) => State::Connected {
                        address,
                        name: self.nome.clone(),
                        warnings: self.avisos.clone(),
                    },
                    IpAddr::V6(_) => State::Retrying(MENSAGEM_SEM_IP.to_string()),
                },
                _ => State::Retrying(MENSAGEM_SEM_IP.to_string()),
            },
            Status::AccessDenied => State::NotAuthorized { address: self.ours },
            Status::NotFound => State::NotFound,
            Status::AuthenticationRequired => State::AuthenticationRequired,
            Status::Collision => State::Failed(MENSAGEM_COLISAO.to_string()),
            Status::Requesting => {
                if let Some(motivo) = &self.rejeitada {
                    State::Retrying(motivo.clone())
                } else if self.vl1.best_root(agora).is_some() {
                    State::WaitingConfig
                } else if self.inicio.elapsed() >= SEM_RAIZ {
                    State::Retrying(format!(
                        "Nenhum raiz respondeu em {} s. A rede bloqueia UDP? Esta versão ainda não faz o relay por TCP.",
                        SEM_RAIZ.as_secs()
                    ))
                } else {
                    State::Connecting
                }
            }
        };
        self.aviso.estado(estado);
    }

    // ----- os pares -----

    fn info(&self, par: &Peer, agora: u64) -> PeerInfo {
        let raizes = self.vl1.roots();
        let role = raizes
            .iter()
            .find(|r| r.address == par.address())
            .map_or_else(
                || {
                    if par.address() == self.network.controller() {
                        PeerRole::Controller
                    } else {
                        PeerRole::Member
                    }
                },
                |r| r.role,
            );
        let path = match par.best_path(agora) {
            Some(c) => format!("direto ({})", c.addr()),
            None if matches!(role, PeerRole::Root | PeerRole::Moon) => String::new(),
            None => "pelo raiz".to_string(),
        };
        PeerInfo {
            address: par.address(),
            role,
            path,
            latency: par
                .latency(agora)
                .map(|ms| Duration::from_millis(u64::from(ms))),
            version: par.version().map(|v| v.to_string()).unwrap_or_default(),
        }
    }

    fn pares(&self, agora: u64, raizes: bool) -> Vec<PeerInfo> {
        let mut lista: Vec<PeerInfo> = self
            .vl1
            .peers()
            .filter(|p| p.address() != self.ours)
            .map(|p| self.info(p, agora))
            .filter(|i| matches!(i.role, PeerRole::Root | PeerRole::Moon) == raizes)
            .collect();
        lista.sort_by_key(|i| i.address);
        lista
    }

    // ----- o laço -----

    async fn encerrar(mut self) {
        self.aviso.estado(State::Stopped);
        self.bridge.detach();
        self.to_stack = None;
        if let Some(pilha) = self.stack.take() {
            pilha.shutdown().await;
        }
        self.bridge.stop().await;
    }

    async fn run(
        mut self,
        mut da_pilha: mpsc::Receiver<Vec<u8>>,
        mut cmds: mpsc::UnboundedReceiver<Command>,
    ) {
        enum Acordou {
            Datagrama(io::Result<(usize, SocketAddr)>),
            Relogio,
            Quadro(Option<Vec<u8>>),
            Comando(Option<Command>),
        }
        self.aviso.estado(State::Connecting);
        let under = self.under.clone();
        let mut relogio = interval(TIQUE);
        relogio.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut leitura = vec![0u8; LEITURA];
        let ack = loop {
            let acordou = tokio::select! {
                r = poll_fn(|cx| under.poll_recv_from(cx, &mut leitura)) => Acordou::Datagrama(r),
                _ = relogio.tick() => Acordou::Relogio,
                q = da_pilha.recv() => Acordou::Quadro(q),
                c = cmds.recv() => Acordou::Comando(c),
            };
            let agora = self.agora();
            let era_relogio = matches!(acordou, Acordou::Relogio);
            match acordou {
                Acordou::Datagrama(Ok((n, de))) => {
                    self.vl1.receive(agora, de, &leitura[..n]);
                }
                // Erro de leitura do socket (por exemplo, ICMP de porta fechada): segue.
                Acordou::Datagrama(Err(_)) => {}
                Acordou::Relogio => self.vl1.tick(agora),
                Acordou::Quadro(Some(quadro)) => {
                    self.vl2.send_frame(agora, &mut self.vl1, &quadro);
                }
                Acordou::Quadro(None) => {}
                Acordou::Comando(None) => break None,
                Acordou::Comando(Some(Command::Stop(ack))) => break Some(ack),
                Acordou::Comando(Some(Command::Peers(r))) => {
                    let _ = r.send(self.pares(agora, false));
                }
                Acordou::Comando(Some(Command::Roots(r))) => {
                    let _ = r.send(self.pares(agora, true));
                }
            }
            // O VL2 só fala quando há um raiz vivo: antes disso o pedido não teria por onde
            // ir. O primeiro tique sai assim que o raiz responde, sem esperar o relógio.
            if self.vl1.best_root(agora).is_some() {
                let primeiro = !self.vl2_ligado;
                self.vl2_ligado = true;
                if primeiro || era_relogio {
                    self.vl2.tick(agora, &mut self.vl1);
                }
            }
            self.drenar(agora);
            self.recalcular_estado(agora);
        };
        self.encerrar().await;
        if let Some(ack) = ack {
            let _ = ack.send(());
        }
    }
}
