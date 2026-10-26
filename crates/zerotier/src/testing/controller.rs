//! Um controlador de rede de mentira, só do formato das mensagens: responde ao
//! `NETWORK_CONFIG_REQUEST` de quem está autorizado com a configuração em pedaços assinados
//! (com certificado de pertinência por membro), recusa quem não está e empurra configuração
//! nova (`NETWORK_CONFIG`). Fala o VL1 de verdade, sobre um [`Underlay`].

use std::collections::{HashMap, HashSet};
use std::future::poll_fn;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{Instant, MissedTickBehavior, interval};
use tomni_ui_netstack::{Cidr, Underlay};

use crate::netconf::com::Com;
use crate::netconf::dns::Dns;
use crate::netconf::request::{build_chunks, parse_request};
use crate::netconf::route::ManagedRoute;
use crate::netconf::rules::Rule;
use crate::netconf::{FLAG_ENABLE_BROADCAST, NetworkConfig};
use crate::packet::{ErrorCode, Verb};
use crate::peer::RemoteVersion;
use crate::vl1::{Config, Effect, Vl1};
use crate::{Address, Identity, NetworkId, World};

/// Quanto dura a janela do certificado de pertinência que o controlador falso emite.
const JANELA_COM: u64 = 3_600_000;

/// O tamanho de cada pedaço da configuração, em bytes do dicionário.
const PEDACO: usize = 1000;

/// Como a rede de teste é configurada.
#[derive(Clone, Debug)]
pub struct RedeTeste {
    /// O nome da rede.
    pub nome: String,
    /// Rede privada (com certificado e autorização) ou pública.
    pub privada: bool,
    /// O bloco de onde saem os IPs, em ordem. O primeiro endereço (`.1`) fica de fora.
    pub pool: Cidr,
    /// As rotas além da do próprio bloco.
    pub rotas: Vec<ManagedRoute>,
    /// As regras. Sem nenhuma regra aceitando, o membro descarta tudo.
    pub regras: Vec<Rule>,
    /// O MTU.
    pub mtu: u16,
    /// O DNS.
    pub dns: Option<Dns>,
    /// O limite de multicast por grupo.
    pub multicast_limit: u32,
    /// As flags da configuração.
    pub flags: u64,
    /// Os especialistas.
    pub especialistas: Vec<(Address, u64)>,
    /// O tamanho de cada pedaço da configuração (para testar a montagem).
    pub pedaco: usize,
}

impl RedeTeste {
    /// A rede padrão: privada, `10.61.0.0/24`, aceita tudo, MTU 2800, broadcast ligado.
    pub fn padrao(nome: &str) -> Self {
        Self {
            nome: nome.to_string(),
            privada: true,
            pool: "10.61.0.0/24".parse().expect("bloco do teste"),
            rotas: Vec::new(),
            regras: vec![Rule::accept()],
            mtu: 2800,
            dns: None,
            multicast_limit: 32,
            flags: FLAG_ENABLE_BROADCAST,
            especialistas: Vec::new(),
            pedaco: PEDACO,
        }
    }
}

/// O que o controlador falso contou.
#[derive(Default)]
pub struct ControllerStats {
    /// Pedidos de configuração recebidos.
    pub pedidos: AtomicU64,
    /// Configurações enviadas (uma por pedido atendido ou empurrão).
    pub configuracoes: AtomicU64,
    /// Pedidos recusados por falta de autorização.
    pub recusas: AtomicU64,
    /// Pedidos de outra rede.
    pub desconhecidos: AtomicU64,
}

impl ControllerStats {
    /// Lê um contador.
    pub fn get(c: &AtomicU64) -> u64 {
        c.load(Ordering::Relaxed)
    }
}

struct Estado {
    rede: RedeTeste,
    id: NetworkId,
    autorizados: HashSet<Address>,
    fixos: HashMap<Address, Ipv4Addr>,
    /// O IP que cada membro recebeu (estável entre pedidos).
    dados: HashMap<Address, Ipv4Addr>,
    revisao: u64,
    ultimo_carimbo: u64,
    /// Responder sem assinar corretamente (para provar que o membro confere a assinatura).
    assinar_errado: bool,
}

enum Comando {
    Empurrar(Address),
}

/// Um controlador do ZeroTier de mentira.
pub struct ControllerFake {
    estado: Arc<Mutex<Estado>>,
    stats: Arc<ControllerStats>,
    address: Address,
    id: NetworkId,
    comandos: mpsc::UnboundedSender<Comando>,
    task: JoinHandle<()>,
}

impl ControllerFake {
    /// Sobe o controlador. `numero` completa o id da rede (`endereço << 24 | numero`). O
    /// controlador faz HELLO ao raiz do `planet` para poder ser alcançado por ele.
    pub fn start<U: Underlay>(
        identity: Identity,
        planet: World,
        numero: u32,
        rede: RedeTeste,
        under: U,
    ) -> ControllerFake {
        let address = identity.address();
        let id = NetworkId::new((address.as_u64() << 24) | u64::from(numero & 0x00ff_ffff));
        let estado = Arc::new(Mutex::new(Estado {
            rede,
            id,
            autorizados: HashSet::new(),
            fixos: HashMap::new(),
            dados: HashMap::new(),
            revisao: 1,
            ultimo_carimbo: 0,
            assinar_errado: false,
        }));
        let stats = Arc::new(ControllerStats::default());
        let (comandos, recebe) = mpsc::unbounded_channel();
        let mut vl1 = Vl1::new(Config {
            identity,
            planet,
            moons: Vec::new(),
            orbits: Vec::new(),
            version: RemoteVersion {
                protocol: 11,
                major: 1,
                minor: 4,
                revision: 7,
            },
            direct_paths: true,
        });
        vl1.set_visible(true);
        let task = tokio::spawn(rodar(
            Arc::new(under),
            vl1,
            estado.clone(),
            stats.clone(),
            recebe,
        ));
        ControllerFake {
            estado,
            stats,
            address,
            id,
            comandos,
            task,
        }
    }

    /// O endereço ZeroTier do controlador.
    pub fn address(&self) -> Address {
        self.address
    }

    /// O id da rede que ele controla.
    pub fn network(&self) -> NetworkId {
        self.id
    }

    /// Os contadores.
    pub fn stats(&self) -> &ControllerStats {
        &self.stats
    }

    fn estado(&self) -> std::sync::MutexGuard<'_, Estado> {
        self.estado.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Autoriza o membro (na rede privada, quem não é autorizado leva `NETWORK_ACCESS_DENIED`).
    pub fn authorize(&self, membro: Address) {
        self.estado().autorizados.insert(membro);
    }

    /// Tira a autorização do membro. O próximo pedido (ou empurrão) é recusado.
    pub fn deauthorize(&self, membro: Address) {
        self.estado().autorizados.remove(&membro);
    }

    /// Fixa o IP do membro (vale a partir da próxima configuração).
    pub fn assign_ip(&self, membro: Address, ip: Ipv4Addr) {
        let mut e = self.estado();
        e.fixos.insert(membro, ip);
        e.dados.remove(&membro);
        e.revisao += 1;
    }

    /// Troca a configuração da rede (sobe a revisão).
    pub fn set_rede(&self, mudanca: impl FnOnce(&mut RedeTeste)) {
        let mut e = self.estado();
        mudanca(&mut e.rede);
        e.revisao += 1;
    }

    /// Passa a assinar as respostas com uma chave que não é a do controlador.
    pub fn assinar_errado(&self, errado: bool) {
        self.estado().assinar_errado = errado;
    }

    /// Empurra a configuração atual ao membro (`NETWORK_CONFIG`), como o controlador faz quando
    /// algo muda. Sem autorização, o membro recebe o erro.
    pub fn push(&self, membro: Address) {
        let _ = self.comandos.send(Comando::Empurrar(membro));
    }
}

impl Drop for ControllerFake {
    fn drop(&mut self) {
        self.task.abort();
    }
}

enum Acordou {
    Datagrama(std::io::Result<(usize, std::net::SocketAddr)>),
    Relogio,
    Comando(Option<Comando>),
}

async fn rodar<U: Underlay>(
    under: Arc<U>,
    mut vl1: Vl1,
    estado: Arc<Mutex<Estado>>,
    stats: Arc<ControllerStats>,
    mut comandos: mpsc::UnboundedReceiver<Comando>,
) {
    let base = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    let inicio = Instant::now();
    let agora = move || base + u64::try_from(inicio.elapsed().as_millis()).unwrap_or(u64::MAX);
    let mut relogio = interval(Duration::from_secs(1));
    relogio.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut leitura = vec![0u8; 2048];
    loop {
        let acordou = tokio::select! {
            r = poll_fn(|cx| under.poll_recv_from(cx, &mut leitura)) => Acordou::Datagrama(r),
            _ = relogio.tick() => Acordou::Relogio,
            c = comandos.recv() => Acordou::Comando(c),
        };
        let agora = agora();
        match acordou {
            Acordou::Datagrama(Ok((n, de))) => vl1.receive(agora, de, &leitura[..n]),
            Acordou::Datagrama(Err(_)) => {}
            Acordou::Relogio => vl1.tick(agora),
            Acordou::Comando(None) => break,
            Acordou::Comando(Some(Comando::Empurrar(membro))) => {
                empurrar(&mut vl1, agora, &estado, &stats, membro);
            }
        }
        // Atender o que subiu pode gerar mais envios, mas nunca mais eventos.
        for efeito in vl1.take_effects() {
            match efeito {
                Effect::Send { to, data } => {
                    let _ = under.try_send_to(&data, to);
                }
                Effect::Deliver(d) if d.verb == Verb::NetworkConfigRequest => {
                    atender(&mut vl1, agora, &estado, &stats, d.from, d.id, &d.payload);
                }
                _ => {}
            }
        }
        for efeito in vl1.take_effects() {
            if let Effect::Send { to, data } = efeito {
                let _ = under.try_send_to(&data, to);
            }
        }
    }
}

/// Monta a configuração do membro. `None` se ele não está autorizado numa rede privada.
fn montar(
    e: &mut Estado,
    controlador: &Identity,
    membro: &Identity,
    agora: u64,
) -> Option<NetworkConfig> {
    if e.rede.privada && !e.autorizados.contains(&membro.address()) {
        return None;
    }
    let ip = match e
        .fixos
        .get(&membro.address())
        .or(e.dados.get(&membro.address()))
    {
        Some(ip) => *ip,
        None => {
            let ip = proximo_ip(e);
            e.dados.insert(membro.address(), ip);
            ip
        }
    };
    let prefixo = e.rede.pool.prefix();
    let mut rotas = vec![ManagedRoute {
        target: Cidr::new(e.rede.pool.network(), prefixo).expect("bloco do teste"),
        via: None,
        flags: 0,
        metric: 0,
    }];
    rotas.extend(e.rede.rotas.iter().cloned());
    // O carimbo só cresce: o membro ignora configuração mais velha que a que já tem.
    e.ultimo_carimbo = agora.max(e.ultimo_carimbo + 1);
    let com = if e.rede.privada {
        Com::issue(controlador, e.id, membro, e.ultimo_carimbo, JANELA_COM)
    } else {
        None
    };
    Some(NetworkConfig {
        network: e.id,
        timestamp: e.ultimo_carimbo,
        revision: e.revisao,
        issued_to: membro.address(),
        flags: e.rede.flags,
        multicast_limit: e.rede.multicast_limit,
        public: !e.rede.privada,
        name: e.rede.nome.clone(),
        mtu: e.rede.mtu,
        com,
        specialists: e.rede.especialistas.clone(),
        routes: rotas,
        addresses: vec![Cidr::new(IpAddr::V4(ip), prefixo).expect("IP do teste")],
        rules: e.rede.regras.clone(),
        dns: e.rede.dns.clone(),
        warnings: Vec::new(),
    })
}

/// O próximo IP livre do bloco (do `.2` em diante).
fn proximo_ip(e: &Estado) -> Ipv4Addr {
    let IpAddr::V4(base) = e.rede.pool.network() else {
        panic!("o controlador falso só faz IPv4");
    };
    let usados: HashSet<Ipv4Addr> = e.fixos.values().chain(e.dados.values()).copied().collect();
    let base = u32::from(base);
    (2u32..)
        .map(|i| Ipv4Addr::from(base + i))
        .find(|ip| !usados.contains(ip))
        .expect("o bloco tem espaço")
}

/// Os pedaços assinados da configuração.
fn pedacos(
    e: &Estado,
    controlador: &Identity,
    cfg: &NetworkConfig,
    update_id: u64,
) -> Option<Vec<Vec<u8>>> {
    let dicionario = cfg.to_dictionary().to_bytes();
    if e.assinar_errado {
        // Assina com uma identidade qualquer, que não é a do controlador.
        let outra = Identity::generate().ok()?;
        return build_chunks(&outra, e.id, &dicionario, update_id, e.rede.pedaco);
    }
    build_chunks(controlador, e.id, &dicionario, update_id, e.rede.pedaco)
}

fn erro(vl1: &mut Vl1, agora: u64, para: Address, re_id: u64, codigo: ErrorCode, rede: NetworkId) {
    let mut carga = vec![Verb::NetworkConfigRequest.to_u8()];
    carga.extend_from_slice(&re_id.to_be_bytes());
    carga.push(codigo.to_u8());
    carga.extend_from_slice(&rede.as_u64().to_be_bytes());
    vl1.send(agora, para, Verb::Error, &carga);
}

fn atender(
    vl1: &mut Vl1,
    agora: u64,
    estado: &Mutex<Estado>,
    stats: &ControllerStats,
    de: Address,
    id: u64,
    payload: &[u8],
) {
    stats.pedidos.fetch_add(1, Ordering::Relaxed);
    let Some((rede, _metadados, _atual)) = parse_request(payload) else {
        return;
    };
    let mut e = estado.lock().unwrap_or_else(PoisonError::into_inner);
    if rede != e.id {
        stats.desconhecidos.fetch_add(1, Ordering::Relaxed);
        erro(vl1, agora, de, id, ErrorCode::ObjectNotFound, rede);
        return;
    }
    let Some(membro) = vl1.peer(de).map(|p| p.identity().clone()) else {
        return;
    };
    let controlador = vl1.identity().clone();
    let Some(cfg) = montar(&mut e, &controlador, &membro, agora) else {
        stats.recusas.fetch_add(1, Ordering::Relaxed);
        erro(vl1, agora, de, id, ErrorCode::NetworkAccessDenied, rede);
        return;
    };
    let Some(lista) = pedacos(&e, &controlador, &cfg, agora) else {
        return;
    };
    for pedaco in lista {
        let mut carga = vec![Verb::NetworkConfigRequest.to_u8()];
        carga.extend_from_slice(&id.to_be_bytes());
        carga.extend_from_slice(&pedaco);
        vl1.send(agora, de, Verb::Ok, &carga);
    }
    stats.configuracoes.fetch_add(1, Ordering::Relaxed);
}

fn empurrar(
    vl1: &mut Vl1,
    agora: u64,
    estado: &Mutex<Estado>,
    stats: &ControllerStats,
    para: Address,
) {
    let mut e = estado.lock().unwrap_or_else(PoisonError::into_inner);
    let Some(membro) = vl1.peer(para).map(|p| p.identity().clone()) else {
        return;
    };
    let controlador = vl1.identity().clone();
    let rede = e.id;
    let Some(cfg) = montar(&mut e, &controlador, &membro, agora) else {
        stats.recusas.fetch_add(1, Ordering::Relaxed);
        erro(vl1, agora, para, 0, ErrorCode::NetworkAccessDenied, rede);
        return;
    };
    let Some(lista) = pedacos(&e, &controlador, &cfg, agora) else {
        return;
    };
    for pedaco in lista {
        vl1.send(agora, para, Verb::NetworkConfig, &pedaco);
    }
    stats.configuracoes.fetch_add(1, Ordering::Relaxed);
}
