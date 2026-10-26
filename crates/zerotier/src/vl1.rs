//! O VL1: a camada de pares do ZeroTier, sem E/S. O motor entrega os datagramas que chegam
//! ([`Vl1::receive`]) e o relógio ([`Vl1::tick`]) e recolhe o que sai e o que sobe para o VL2
//! ([`Vl1::take_effects`]). Nada aqui abre socket, dorme ou lê o relógio: os tempos são
//! milissegundos que o chamador fornece, e por isso o que segue é testável sem esperar.
//!
//! **O app é invisível** (seção 1.3 do plano): não somos raiz e ninguém nos procura. HELLO e
//! ECHO só são respondidos a raiz, ao controlador e a pares que nós procuramos; WHOIS não é
//! respondido a ninguém; o que vem de um endereço desconhecido ou não decifra some em silêncio.

use std::collections::{BTreeMap, HashMap};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use crate::crypto::armor::{extended_armor, extended_dearmor};
use crate::crypto::{PeerKey, gmac_siv, salsa};
use crate::packet::{
    self, Cipher, ErrorCode, FLAG_EXTENDED_ARMOR, Fragment, MASK_CIPHER, MASK_HOPS, MASK_VERB,
    MIN_PACKET_LEN, Packet, UDP_PAYLOAD_MTU, Verb, idx,
};
use crate::peer::{Peer, PeerRole, RemoteVersion};
use crate::world::{Root, WorldKind};
use crate::{Address, Identity, World, inet};

/// O HELLO completo sai a cada minuto (`ZT_PEER_PING_PERIOD`, `Constants.hpp`).
pub const PING_PERIOD: u64 = 60_000;

/// A cada quanto o motor chama [`Vl1::tick`] (`ZT_PING_CHECK_INTERVAL`, `Constants.hpp`).
pub const PING_CHECK_INTERVAL: u64 = 5_000;

/// Um WHOIS do mesmo endereço sai no máximo a cada meio segundo (`ZT_WHOIS_RETRY_DELAY`).
const WHOIS_RETRY_DELAY: u64 = 500;

/// Quanto tempo um pacote espera a identidade de quem o mandou (`ZT_RECEIVE_QUEUE_TIMEOUT`).
const RECEIVE_QUEUE_TIMEOUT: u64 = 5_000;

/// Quanto tempo um pacote espera para sair, sem identidade ou sem caminho
/// (`ZT_TRANSMIT_QUEUE_TIMEOUT`).
const TRANSMIT_QUEUE_TIMEOUT: u64 = 5_000;

/// Um par sem tráfego por tanto tempo sai da tabela, menos raiz e controlador
/// (`ZT_PEER_ACTIVITY_TIMEOUT`).
const PEER_ACTIVITY_TIMEOUT: u64 = 500_000;

/// HELLO no mesmo caminho no máximo uma vez por segundo (`ZT_PATH_HELLO_RATE_LIMIT`).
const PATH_HELLO_RATE_LIMIT: u64 = 1_000;

/// PUSH_DIRECT_PATHS a um par sem caminho direto: a cada 15 s (`ZT_DIRECT_PATH_PUSH_INTERVAL`).
const DIRECT_PATH_PUSH_INTERVAL: u64 = 15_000;

/// PUSH_DIRECT_PATHS a um par que já tem caminho direto: a cada 120 s
/// (`ZT_DIRECT_PATH_PUSH_INTERVAL_HAVEPATH`).
const DIRECT_PATH_PUSH_INTERVAL_HAVEPATH: u64 = 120_000;

/// A janela em que se contam as levas de PUSH_DIRECT_PATHS recebidas de um par
/// (`ZT_PUSH_DIRECT_PATHS_CUTOFF_TIME`).
const PUSH_DIRECT_PATHS_CUTOFF_TIME: u64 = 30_000;

/// Quantas levas por janela aceitamos de um par (`ZT_PUSH_DIRECT_PATHS_MAX_PER_UNIT`).
const PUSH_DIRECT_PATHS_MAX_PER_WINDOW: u8 = 4;

/// Quantos endereços lemos de uma leva.
const PUSH_DIRECT_PATHS_MAX_ADDRESSES: usize = 16;

/// A cada quanto o caminho memorizado é tentado de novo (`ZT_TRY_MEMORIZED_PATH_INTERVAL`).
const TRY_MEMORIZED_PATH_INTERVAL: u64 = 30_000;

/// Quantas vezes o HELLO do furo de NAT sai para o mesmo endereço, e de quanto em quanto
/// tempo. O primeiro costuma morrer no NAT do outro lado; o seguinte já passa pelo furo que o
/// do outro lado abriu (`ZT_NAT_T_TACTICAL_ESCALATION_DELAY`, 1 s).
const PUNCH_ATTEMPTS: u8 = 3;
const PUNCH_RETRY_DELAY: u64 = 1_000;

/// Quanto tempo um furo espera a identidade do par antes de desistir.
const PUNCH_WAIT_IDENTITY: u64 = 5_000;

/// Quanto tempo uma resposta a um pedido nosso é aceita. Não é número do protocolo: o ZeroTier
/// usa baldes que giram; um minuto cobre a configuração da rede chegando em vários pedaços.
const REPLY_WINDOW: u64 = 60_000;

/// Quanto tempo os fragmentos esperam o resto do pacote.
const FRAGMENT_TIMEOUT: u64 = RECEIVE_QUEUE_TIMEOUT;

/// A versão mínima do protocolo que aceitamos de um par (`ZT_PROTO_VERSION_MIN`, `Packet.hpp`).
pub const PROTOCOL_MIN: u8 = 4;

/// O tamanho mínimo de um HELLO: protocolo, versão (3 bytes), hora (8) e a identidade (71).
const HELLO_FIXED: usize = 1 + 1 + 1 + 2 + 8;

/// O que o VL1 pede ao motor ou entrega ao VL2.
#[derive(Debug)]
pub enum Effect {
    /// Mandar este datagrama UDP.
    Send {
        /// O destino físico.
        to: SocketAddr,
        /// Os bytes.
        data: Vec<u8>,
    },
    /// Um verbo que o VL1 não trata chegou, já autenticado e descomprimido.
    Deliver(Delivery),
    /// O planet ou uma moon mudou: o app grava.
    WorldChanged,
    /// Um raiz nos viu neste endereço externo.
    SeenAddress(SocketAddr),
    /// A identidade de um endereço que procuramos chegou (pelo WHOIS).
    PeerKnown(Address),
    /// Um par ganhou um caminho direto novo: o app o memoriza para tentar primeiro na próxima vez.
    DirectPath(Address, SocketAddr),
}

/// Resolve o IP local que o sistema usaria para chegar a um endereço (`Underlay::local_ip_toward`).
pub type LocalResolver = Arc<dyn Fn(SocketAddr) -> Option<IpAddr> + Send + Sync>;

/// Um pacote autenticado que sobe para o VL2.
#[derive(Debug)]
pub struct Delivery {
    /// Quem mandou.
    pub from: Address,
    /// O verbo (`Ok` e `Error` trazem o que respondem em `in_re`).
    pub verb: Verb,
    /// O ID do pacote.
    pub id: u64,
    /// Os saltos que o pacote deu (0 = direto).
    pub hops: u8,
    /// Por onde chegou, fisicamente.
    pub via: SocketAddr,
    /// Só em `Ok` e `Error`: o verbo e o ID do pedido que respondem.
    pub in_re: Option<(Verb, u64)>,
    /// Só em `Error`: o código.
    pub error: Option<ErrorCode>,
    /// A carga do verbo, depois dos cabeçalhos de `Ok` e `Error`.
    pub payload: Vec<u8>,
}

/// O que o nó é, para o HELLO e o OK(HELLO).
#[derive(Clone)]
pub struct Config {
    /// A nossa identidade, com a parte secreta.
    pub identity: Identity,
    /// O planet.
    pub planet: World,
    /// As moons que já temos.
    pub moons: Vec<World>,
    /// As moons que queremos receber: o id e o endereço de um raiz delas.
    pub orbits: Vec<(u64, Address)>,
    /// A versão que anunciamos.
    pub version: RemoteVersion,
    /// Aprender caminhos diretos até os pares. Desligado, tudo passa pelos raiz.
    pub direct_paths: bool,
}

/// Um raiz em uso.
#[derive(Clone, Debug)]
pub struct RootInfo {
    /// O endereço.
    pub address: Address,
    /// Raiz do planet ou de uma moon.
    pub role: PeerRole,
    /// Onde achá-lo.
    pub endpoints: Vec<SocketAddr>,
}

/// Um HELLO que o NAT pede para repetir: o furo para um endereço que um raiz (RENDEZVOUS) ou o
/// próprio par (PUSH_DIRECT_PATHS) nos deu.
struct Punch {
    with: Address,
    addr: SocketAddr,
    next: u64,
    left: u8,
    since: u64,
}

/// Pacote que espera a identidade de quem mandou.
struct WaitingRx {
    from: SocketAddr,
    bytes: Vec<u8>,
    source: Address,
    since: u64,
}

/// Pacote nosso que espera a identidade do destino ou um caminho.
struct WaitingTx {
    dest: Address,
    verb: Verb,
    payload: Vec<u8>,
    compress: bool,
    since: u64,
}

/// O que se espera como resposta a um pacote que mandamos.
struct Expected {
    sent: u64,
    verb: Verb,
}

/// Um pacote fragmentado em montagem.
struct Reassembly {
    from: SocketAddr,
    head: Option<Vec<u8>>,
    pieces: BTreeMap<u8, Fragment>,
    total: u8,
    since: u64,
}

/// O VL1.
pub struct Vl1 {
    identity: Identity,
    version: RemoteVersion,
    direct_paths: bool,
    planet: World,
    moons: Vec<World>,
    orbits: Vec<(u64, Address)>,
    roots: HashMap<Address, RootInfo>,
    controller: Option<Address>,
    visible: bool,
    peers: HashMap<Address, Peer>,
    whois_sent: HashMap<Address, u64>,
    waiting_rx: Vec<WaitingRx>,
    waiting_tx: Vec<WaitingTx>,
    expecting: HashMap<u64, Expected>,
    reassembly: HashMap<u64, Reassembly>,
    last_hello_to: HashMap<SocketAddr, u64>,
    /// A porta do nosso socket e como achar o IP local voltado para um destino.
    local_port: u16,
    local_toward: Option<LocalResolver>,
    /// O endereço externo que cada raiz viu.
    external: HashMap<Address, SocketAddr>,
    /// IPs que nunca viram caminho físico: os da rede virtual e os das rotas gerenciadas.
    excluded: Vec<(Ipv4Addr, u8)>,
    push_paths: bool,
    /// Mandar o HELLO na armadura estendida a quem anuncia o protocolo 13 ou mais.
    extended_hello: bool,
    punches: Vec<Punch>,
    push_sent: HashMap<Address, u64>,
    push_received: HashMap<Address, (u64, u8)>,
    memorized: HashMap<Address, SocketAddr>,
    last_memorized: u64,
    next_id: u64,
    last_check: u64,
    effects: Vec<Effect>,
}

impl Vl1 {
    /// Um VL1 novo. Os raiz do planet e das moons viram pares na hora (a identidade deles vem
    /// do mundo, que é assinado).
    pub fn new(config: Config) -> Self {
        let mut seed = [0u8; 8];
        // Sem aleatoriedade o nó não consegue nem gerar a identidade; aqui ela já existe, e o
        // pior caso é um ID previsível, que o MAC e a chave por pacote tornam inofensivo.
        let _ = getrandom::fill(&mut seed);
        let mut vl1 = Self {
            identity: config.identity,
            version: config.version,
            direct_paths: config.direct_paths,
            planet: config.planet,
            moons: config.moons,
            orbits: config.orbits,
            roots: HashMap::new(),
            controller: None,
            visible: false,
            peers: HashMap::new(),
            whois_sent: HashMap::new(),
            waiting_rx: Vec::new(),
            waiting_tx: Vec::new(),
            expecting: HashMap::new(),
            reassembly: HashMap::new(),
            last_hello_to: HashMap::new(),
            local_port: 0,
            local_toward: None,
            external: HashMap::new(),
            excluded: Vec::new(),
            push_paths: true,
            extended_hello: false,
            punches: Vec::new(),
            push_sent: HashMap::new(),
            push_received: HashMap::new(),
            memorized: HashMap::new(),
            last_memorized: 0,
            // Os 3 bits baixos do ID não entram no `crypt_field`: andar de 8 em 8 garante um
            // IV novo a cada pacote.
            next_id: u64::from_be_bytes(seed) & !7,
            last_check: 0,
            effects: Vec::new(),
        };
        vl1.sync_roots(0);
        vl1
    }

    /// O nosso endereço.
    pub fn address(&self) -> Address {
        self.identity.address()
    }

    /// A nossa identidade.
    pub fn identity(&self) -> &Identity {
        &self.identity
    }

    /// O planet em uso.
    pub fn planet(&self) -> &World {
        &self.planet
    }

    /// As moons em uso.
    pub fn moons(&self) -> &[World] {
        &self.moons
    }

    /// O par com este endereço.
    pub fn peer(&self, endereco: Address) -> Option<&Peer> {
        self.peers.get(&endereco)
    }

    /// Todos os pares.
    pub fn peers(&self) -> impl Iterator<Item = &Peer> {
        self.peers.values()
    }

    /// Os raiz em uso.
    pub fn roots(&self) -> Vec<RootInfo> {
        let mut raizes: Vec<RootInfo> = self.roots.values().cloned().collect();
        raizes.sort_by_key(|r| r.address);
        raizes
    }

    /// Diz quem é o controlador da nossa rede: ele fala conosco sem ter sido procurado.
    pub fn set_controller(&mut self, endereco: Address) {
        self.controller = Some(endereco);
    }

    /// Diz a porta do nosso socket e como achar o IP local voltado para um destino; com isso o
    /// PUSH_DIRECT_PATHS anuncia o endereço local ao par.
    pub fn set_local(&mut self, port: u16, resolver: LocalResolver) {
        self.local_port = port;
        self.local_toward = Some(resolver);
    }

    /// Liga ou desliga o anúncio dos nossos endereços aos pares (PUSH_DIRECT_PATHS). Ligado por
    /// padrão; receber os dos outros não depende disto.
    pub fn set_push_direct_paths(&mut self, ligado: bool) {
        self.push_paths = ligado;
    }

    /// Liga ou desliga o HELLO na armadura estendida (a chave efêmera esconde a identidade de
    /// quem manda). Só vai a quem não é raiz e já anunciou o protocolo 13; o primeiro HELLO a
    /// um par, sem versão conhecida, sai em claro. Desligado por padrão, como no ZeroTier 1.14
    /// (`encryptedHelloEnabled`); receber não depende disto.
    pub fn set_extended_hello(&mut self, ligado: bool) {
        self.extended_hello = ligado;
    }

    /// Redes que não servem de caminho físico (a virtual e as das rotas gerenciadas).
    pub fn set_excluded(&mut self, redes: Vec<(Ipv4Addr, u8)>) {
        self.excluded = redes;
    }

    /// Os caminhos diretos memorizados de uma execução anterior: tentados quando o par aparece
    /// e a cada 30 s enquanto não houver outro.
    pub fn set_memorized(&mut self, caminhos: Vec<(Address, SocketAddr)>) {
        self.memorized = caminhos.into_iter().collect();
    }

    /// Liga a visibilidade: o nó responde ao HELLO de quem não conhecia, como um controlador ou
    /// um membro com IP público. O app fica invisível (o padrão); só os falsos de teste e um
    /// futuro papel de servidor ligam isto.
    pub fn set_visible(&mut self, visivel: bool) {
        self.visible = visivel;
    }

    /// O melhor raiz: o de menor latência com caminho vivo.
    pub fn best_root(&self, now: u64) -> Option<Address> {
        self.best_root_except(now, None)
    }

    /// Recolhe o que o VL1 produziu desde a última chamada.
    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    // ---------------------------------------------------------------- raiz e mundo

    fn best_root_except(&self, now: u64, fora: Option<Address>) -> Option<Address> {
        self.roots
            .keys()
            .filter(|a| Some(**a) != fora)
            .filter_map(|a| {
                let c = self.peers.get(a)?.best_path(now)?;
                Some((c.score(now), *a))
            })
            .min_by_key(|(nota, a)| (*nota, *a))
            .map(|(_, a)| a)
    }

    /// Refaz a lista de raiz a partir do planet e das moons e cria os pares que faltam.
    fn sync_roots(&mut self, now: u64) {
        let mut raizes: HashMap<Address, RootInfo> = HashMap::new();
        let mundos = std::iter::once(&self.planet).chain(self.moons.iter());
        let mut identidades: Vec<(Identity, PeerRole, &Root)> = Vec::new();
        for mundo in mundos {
            let papel = match mundo.kind() {
                WorldKind::Planet => PeerRole::Root,
                WorldKind::Moon => PeerRole::Moon,
            };
            for raiz in mundo.roots() {
                identidades.push((raiz.identity.clone(), papel, raiz));
            }
        }
        for (identidade, papel, raiz) in identidades {
            let endereco = identidade.address();
            if endereco == self.identity.address() {
                continue;
            }
            let info = raizes.entry(endereco).or_insert_with(|| RootInfo {
                address: endereco,
                role: papel,
                endpoints: Vec::new(),
            });
            // O mesmo raiz em dois mundos: vale o planet e juntam-se os endereços.
            if papel == PeerRole::Root {
                info.role = PeerRole::Root;
            }
            for e in &raiz.endpoints {
                if !info.endpoints.contains(e) {
                    info.endpoints.push(*e);
                }
            }
            if !self.peers.contains_key(&endereco)
                && let Some(par) = Peer::new(&self.identity, identidade, now)
            {
                self.peers.insert(endereco, par);
            }
        }
        self.roots = raizes;
    }

    /// Considera uma atualização de mundo vinda de um raiz. Devolve se mudou algo.
    fn consider_world(&mut self, now: u64, novo: World) -> bool {
        let mudou = match novo.kind() {
            WorldKind::Planet => {
                if self.planet.should_be_replaced_by(&novo) {
                    self.planet = novo;
                    true
                } else {
                    false
                }
            }
            WorldKind::Moon => {
                if let Some(i) = self.moons.iter().position(|m| m.id() == novo.id()) {
                    if self.moons[i].should_be_replaced_by(&novo) {
                        self.moons[i] = novo;
                        true
                    } else {
                        false
                    }
                } else if self.orbits.iter().any(|(id, _)| *id == novo.id())
                    && novo.verify(novo.updates_signed_by())
                {
                    // Uma moon que pedimos para orbitar: vale se assinada pela chave que ela
                    // mesma declara (`Topology::addWorld`).
                    self.moons.push(novo);
                    true
                } else {
                    false
                }
            }
        };
        if mudou {
            self.sync_roots(now);
            self.effects.push(Effect::WorldChanged);
        }
        mudou
    }

    // ---------------------------------------------------------------- quem fala conosco

    /// Falamos AES-GMAC-SIV com o par: os dois anunciamos o protocolo 12 ou mais
    /// (`Peer::aesKeysIfSupported`).
    fn uses_aes(&self, par: &Peer) -> bool {
        self.version.protocol >= 12 && par.version().is_some_and(|v| v.protocol >= 12)
    }

    fn is_upstream(&self, endereco: Address) -> bool {
        self.roots.contains_key(&endereco)
    }

    /// Pode receber resposta: raiz, controlador ou par que procuramos.
    fn can_talk(&self, endereco: Address) -> bool {
        self.visible
            || self.is_upstream(endereco)
            || self.controller == Some(endereco)
            || self.peers.get(&endereco).is_some_and(Peer::contacted)
    }

    /// Um raiz nos apresentou este par (RENDEZVOUS) ou procuramos por ele: pode falar conosco.
    pub fn mark_contacted(&mut self, endereco: Address) {
        if let Some(par) = self.peers.get_mut(&endereco) {
            par.set_contacted();
        }
    }

    // ---------------------------------------------------------------- recepção

    /// Um datagrama UDP chegou de `from`.
    pub fn receive(&mut self, now: u64, from: SocketAddr, data: &[u8]) {
        // Cabeçalho de fragmento tem 16 bytes; o de pacote, 28.
        if data.len() < packet::FRAGMENT_HEADER_LEN {
            return;
        }
        if data[idx::SOURCE] == crate::address::RESERVED_PREFIX {
            if let Some(f) = Fragment::parse(data) {
                self.receive_fragment(now, from, f);
            }
            return;
        }
        if data.len() < MIN_PACKET_LEN {
            return;
        }
        if data[idx::FLAGS] & packet::FLAG_FRAGMENTED != 0 {
            self.receive_head(now, from, data.to_vec());
            return;
        }
        self.receive_packet(now, from, data.to_vec());
    }

    fn receive_head(&mut self, now: u64, from: SocketAddr, bytes: Vec<u8>) {
        if packet::dest_of(&bytes) != self.address() {
            return; // não repassamos: só os raiz fazem isso
        }
        let id = packet::id_of(&bytes);
        let entrada = self.reassembly.entry(id).or_insert_with(|| Reassembly {
            from,
            head: None,
            pieces: BTreeMap::new(),
            total: 0,
            since: now,
        });
        entrada.head = Some(bytes);
        self.try_complete(now, id);
    }

    fn receive_fragment(&mut self, now: u64, from: SocketAddr, fragmento: Fragment) {
        if fragmento.dest() != self.address() {
            return;
        }
        let id = fragmento.packet_id();
        let entrada = self.reassembly.entry(id).or_insert_with(|| Reassembly {
            from,
            head: None,
            pieces: BTreeMap::new(),
            total: 0,
            since: now,
        });
        entrada.total = fragmento.total();
        entrada.pieces.insert(fragmento.number(), fragmento);
        self.try_complete(now, id);
    }

    fn try_complete(&mut self, now: u64, id: u64) {
        let Some(entrada) = self.reassembly.get(&id) else {
            return;
        };
        let Some(head) = &entrada.head else { return };
        if entrada.total == 0 || entrada.pieces.len() + 1 < usize::from(entrada.total) {
            return;
        }
        let pecas: Vec<Fragment> = entrada.pieces.values().cloned().collect();
        let juntado = packet::join(head, &pecas);
        let from = entrada.from;
        self.reassembly.remove(&id);
        self.receive_packet(now, from, juntado);
    }

    /// Um pacote inteiro, para nós, ainda armado.
    fn receive_packet(&mut self, now: u64, from: SocketAddr, mut bytes: Vec<u8>) {
        if bytes.len() < MIN_PACKET_LEN || packet::dest_of(&bytes) != self.address() {
            return;
        }
        let Some(source) = Address::from_bytes(&bytes[idx::SOURCE..]) else {
            return;
        };
        if source == self.address() || source.is_reserved() {
            return;
        }
        let flags = bytes[idx::FLAGS];
        let cifra = Cipher::from_bits((flags & MASK_CIPHER) >> 3);
        let saltos = flags & MASK_HOPS;
        match cifra {
            Cipher::None if flags & FLAG_EXTENDED_ARMOR != 0 => {
                // Só o HELLO usa a armadura estendida com a suíte 0.
                if extended_dearmor(&mut bytes, &self.identity).is_err() {
                    return;
                }
                // A armadura estendida só esconde: quem garante que o HELLO é de quem diz é o MAC
                // da Salsa que vem por dentro, conferido em `handle_hello`.
                if Verb::from_u8(bytes[idx::VERB] & MASK_VERB) == Verb::Hello {
                    self.handle_hello(now, from, bytes, source, saltos, false);
                }
            }
            Cipher::None | Cipher::Salsa2012 => {
                if cifra == Cipher::None
                    && Verb::from_u8(bytes[idx::VERB] & MASK_VERB) == Verb::Hello
                {
                    self.handle_hello(now, from, bytes, source, saltos, false);
                    return;
                }
                let original = bytes.clone();
                let Some(par) = self.peers.get(&source) else {
                    self.wait_for_identity(now, from, original, source);
                    return;
                };
                if salsa::dearmor(&par.key().salsa, &mut bytes).is_err() {
                    return;
                }
                self.dispatch(now, from, source, saltos, bytes);
            }
            Cipher::AesGmacSiv => {
                // Só com o protocolo 12 ou mais conversamos em AES; antes, o pacote não vale.
                if self.version.protocol < 12 {
                    return;
                }
                let original = bytes.clone();
                let Some(par) = self.peers.get(&source) else {
                    self.wait_for_identity(now, from, original, source);
                    return;
                };
                if gmac_siv::dearmor(&par.key().aes, &mut bytes).is_err() {
                    return;
                }
                self.dispatch(now, from, source, saltos, bytes);
            }
            // O caminho confiável não existe aqui: descartado.
            _ => {}
        }
    }

    fn wait_for_identity(&mut self, now: u64, from: SocketAddr, bytes: Vec<u8>, source: Address) {
        self.waiting_rx.push(WaitingRx {
            from,
            bytes,
            source,
            since: now,
        });
        self.request_whois(now, source);
    }

    /// O pacote já passou na autenticação: descomprime e despacha pelo verbo.
    fn dispatch(&mut self, now: u64, from: SocketAddr, source: Address, hops: u8, bytes: Vec<u8>) {
        let Ok(mut pacote) = Packet::from_bytes(bytes) else {
            return;
        };
        if pacote.uncompress().is_err() {
            return;
        }
        let verbo = pacote.verb();
        match verbo {
            Verb::Hello => {
                let bytes = pacote.into_bytes();
                self.handle_hello(now, from, bytes, source, hops, true);
            }
            Verb::Ok => self.handle_ok(now, from, source, hops, &pacote),
            Verb::Error => self.handle_error(now, from, source, hops, &pacote),
            Verb::Echo => self.handle_echo(now, from, source, hops, &pacote),
            // Não somos raiz: não respondemos a WHOIS de ninguém.
            Verb::Whois | Verb::Nop => self.touch(now, from, source, hops, verbo),
            Verb::Rendezvous => {
                self.touch(now, from, source, hops, verbo);
                self.handle_rendezvous(now, source, &pacote);
            }
            Verb::PushDirectPaths => {
                self.touch(now, from, source, hops, verbo);
                self.handle_push_direct_paths(now, source, &pacote);
            }
            Verb::Ack
            | Verb::QosMeasurement
            | Verb::UserMessage
            | Verb::RemoteTrace
            | Verb::PathNegotiationRequest => self.touch(now, from, source, hops, verbo),
            _ => {
                self.touch(now, from, source, hops, verbo);
                self.effects.push(Effect::Deliver(Delivery {
                    from: source,
                    verb: verbo,
                    id: pacote.id(),
                    hops,
                    via: from,
                    in_re: None,
                    error: None,
                    payload: pacote.payload().to_vec(),
                }));
            }
        }
    }

    /// O par mandou algo autenticado: renova o que sabemos dele e o caminho.
    fn touch(&mut self, now: u64, from: SocketAddr, source: Address, hops: u8, verbo: Verb) {
        let upstream = self.is_upstream(source);
        let direct = self.direct_paths || upstream;
        let Some(par) = self.peers.get_mut(&source) else {
            return;
        };
        par.received(now);
        if !upstream {
            self.maybe_push_direct_paths(now, source);
        }
        let Some(par) = self.peers.get_mut(&source) else {
            return;
        };
        if hops != 0 || !direct {
            return;
        }
        if let Some(c) = par.path_mut(from) {
            c.received(now);
            return;
        }
        if verbo == Verb::Ok {
            // Só uma resposta a algo nosso prova que o endereço é do par.
            par.learn_path(from, now);
            if !upstream {
                self.memorized.insert(source, from);
                self.effects.push(Effect::DirectPath(source, from));
            }
        } else {
            // Qualquer outra coisa vinda de um endereço novo: confirma com um HELLO antes de
            // confiar nele (`Peer::received`, `Peer.cpp`).
            self.confirm_path(now, source, from);
        }
    }

    fn confirm_path(&mut self, now: u64, par: Address, em: SocketAddr) {
        let ultimo = self.last_hello_to.get(&em).copied().unwrap_or(0);
        if ultimo != 0 && now.saturating_sub(ultimo) < PATH_HELLO_RATE_LIMIT {
            return;
        }
        self.send_hello(now, par, Some(em));
    }

    // ---------------------------------------------------------------- o NAT

    /// RENDEZVOUS: um raiz diz onde está um par com quem ele nos viu falar. Só vale de raiz.
    fn handle_rendezvous(&mut self, now: u64, source: Address, pacote: &Packet) {
        if !self.direct_paths || !self.is_upstream(source) {
            return;
        }
        // `flags (1) ‖ endereço (5) ‖ porta (2) ‖ tamanho do IP (1) ‖ IP`.
        let p = pacote.payload();
        if p.len() < 9 {
            return;
        }
        let Some(com) = Address::from_bytes(&p[1..6]) else {
            return;
        };
        let porta = u16::from_be_bytes([p[6], p[7]]);
        let tamanho = usize::from(p[8]);
        if com == self.address() || porta == 0 || tamanho != 4 || p.len() < 9 + tamanho {
            return; // IPv6 não é tratado
        }
        let ip = Ipv4Addr::new(p[9], p[10], p[11], p[12]);
        self.schedule_punch(now, com, SocketAddr::new(IpAddr::V4(ip), porta));
    }

    /// PUSH_DIRECT_PATHS: o par nos conta por onde também é alcançável. Tentamos cada endereço.
    fn handle_push_direct_paths(&mut self, now: u64, source: Address, pacote: &Packet) {
        if !self.direct_paths || self.is_upstream(source) {
            return;
        }
        if !self.peers.get(&source).is_some_and(Peer::contacted) {
            return;
        }
        let leva = self.push_received.entry(source).or_insert((now, 0));
        if now.saturating_sub(leva.0) > PUSH_DIRECT_PATHS_CUTOFF_TIME {
            *leva = (now, 0);
        }
        leva.1 = leva.1.saturating_add(1);
        if leva.1 > PUSH_DIRECT_PATHS_MAX_PER_WINDOW {
            return;
        }
        let p = pacote.payload();
        if p.len() < 2 {
            return;
        }
        let quantos =
            usize::from(u16::from_be_bytes([p[0], p[1]])).min(PUSH_DIRECT_PATHS_MAX_ADDRESSES);
        let mut pos = 2;
        let mut enderecos = Vec::new();
        for _ in 0..quantos {
            // `flags (1) ‖ tamanho da extensão (2) ‖ extensão ‖ tipo (1) ‖ tamanho (1) ‖ endereço`.
            if pos + 3 > p.len() {
                break;
            }
            let esquecer = p[pos] & 0x01 != 0;
            let extensao = usize::from(u16::from_be_bytes([p[pos + 1], p[pos + 2]]));
            pos += 3 + extensao;
            if pos + 2 > p.len() {
                break;
            }
            let tipo = p[pos];
            let tamanho = usize::from(p[pos + 1]);
            pos += 2;
            if pos + tamanho > p.len() {
                break;
            }
            if tipo == 4 && tamanho == 6 && !esquecer {
                let ip = Ipv4Addr::new(p[pos], p[pos + 1], p[pos + 2], p[pos + 3]);
                let porta = u16::from_be_bytes([p[pos + 4], p[pos + 5]]);
                enderecos.push(SocketAddr::new(IpAddr::V4(ip), porta));
            }
            pos += tamanho;
        }
        for endereco in enderecos {
            self.schedule_punch(now, source, endereco);
        }
    }

    /// Manda os nossos endereços a um par em quem confiamos, a cada 15 s sem caminho direto e a
    /// cada 120 s com um.
    fn maybe_push_direct_paths(&mut self, now: u64, dest: Address) {
        if !self.direct_paths || !self.push_paths {
            return;
        }
        let Some(par) = self.peers.get(&dest) else {
            return;
        };
        if !par.contacted() {
            return;
        }
        let intervalo = if par.has_alive_path(now) {
            DIRECT_PATH_PUSH_INTERVAL_HAVEPATH
        } else {
            DIRECT_PATH_PUSH_INTERVAL
        };
        if self
            .push_sent
            .get(&dest)
            .is_some_and(|t| now.saturating_sub(*t) < intervalo)
        {
            return;
        }
        // O IP local voltado para o par (se já temos um caminho dele) ou para o raiz.
        let mut alvos: Vec<SocketAddr> = par.paths().iter().map(|c| c.addr()).collect();
        if alvos.is_empty()
            && let Some(raiz) = self.best_root(now)
            && let Some(c) = self.peers.get(&raiz).and_then(|r| r.best_path(now))
        {
            alvos.push(c.addr());
        }
        let mut meus: Vec<SocketAddr> = Vec::new();
        if let Some(resolve) = &self.local_toward
            && self.local_port != 0
        {
            for alvo in alvos {
                if let Some(ip) = resolve(alvo) {
                    meus.push(SocketAddr::new(ip, self.local_port));
                }
            }
        }
        meus.extend(self.external.values().copied());
        meus.sort();
        meus.dedup();
        let mut carga: Vec<u8> = Vec::new();
        let mut quantos: u16 = 0;
        for meu in meus {
            let SocketAddr::V4(v4) = meu else { continue };
            if !self.usable_address(&meu) {
                continue;
            }
            carga.push(0); // flags
            carga.extend_from_slice(&0u16.to_be_bytes()); // sem extensão
            carga.push(4);
            carga.push(6);
            carga.extend_from_slice(&v4.ip().octets());
            carga.extend_from_slice(&v4.port().to_be_bytes());
            quantos += 1;
        }
        self.push_sent.insert(dest, now);
        if quantos == 0 {
            return;
        }
        let mut pacote = quantos.to_be_bytes().to_vec();
        pacote.extend_from_slice(&carga);
        self.send_packet(now, dest, Verb::PushDirectPaths, &pacote, false);
    }

    /// O endereço pode virar caminho físico: IPv4, com porta, fora da rede virtual e das rotas.
    fn usable_address(&self, endereco: &SocketAddr) -> bool {
        let IpAddr::V4(ip) = endereco.ip() else {
            return false;
        };
        if endereco.port() == 0 || ip.is_unspecified() || ip.is_multicast() || ip.is_broadcast() {
            return false;
        }
        !self.excluded.iter().any(|(rede, bits)| {
            let mascara = if *bits == 0 {
                0
            } else {
                u32::MAX << (32 - u32::from(*bits).min(32))
            };
            u32::from(ip) & mascara == u32::from(*rede) & mascara
        })
    }

    /// Agenda o HELLO que fura o NAT até `endereco`. O primeiro sai já.
    fn schedule_punch(&mut self, now: u64, com: Address, endereco: SocketAddr) {
        if !self.direct_paths || com == self.address() || !self.usable_address(&endereco) {
            return;
        }
        if self
            .punches
            .iter()
            .any(|f| f.with == com && f.addr == endereco)
        {
            return;
        }
        match self.peers.get(&com) {
            Some(par) => {
                if par
                    .paths()
                    .iter()
                    .any(|c| c.addr() == endereco && c.alive(now))
                {
                    return;
                }
                self.mark_contacted(com);
            }
            None => self.request_whois(now, com),
        }
        self.punches.push(Punch {
            with: com,
            addr: endereco,
            next: now,
            left: PUNCH_ATTEMPTS,
            since: now,
        });
        self.run_punches(now);
    }

    /// Manda os HELLO do furo que já venceram a espera; larga os que deram certo ou cansaram.
    fn run_punches(&mut self, now: u64) {
        if self.punches.is_empty() {
            return;
        }
        let fila = std::mem::take(&mut self.punches);
        for mut furo in fila {
            let Some(par) = self.peers.get(&furo.with) else {
                if now.saturating_sub(furo.since) < PUNCH_WAIT_IDENTITY {
                    self.punches.push(furo);
                }
                continue;
            };
            if par
                .paths()
                .iter()
                .any(|c| c.addr() == furo.addr && c.alive(now))
            {
                continue;
            }
            if now >= furo.next {
                self.send_hello(now, furo.with, Some(furo.addr));
                furo.left -= 1;
                furo.next = now + PUNCH_RETRY_DELAY;
            }
            if furo.left > 0 {
                self.punches.push(furo);
            }
        }
    }

    /// Os caminhos memorizados que ainda não viraram caminho vivo são tentados de novo.
    fn try_memorized(&mut self, now: u64) {
        if self.last_memorized != 0
            && now.saturating_sub(self.last_memorized) < TRY_MEMORIZED_PATH_INTERVAL
        {
            return;
        }
        self.last_memorized = now;
        let memorizados: Vec<(Address, SocketAddr)> =
            self.memorized.iter().map(|(a, c)| (*a, *c)).collect();
        for (par, caminho) in memorizados {
            if self.peers.get(&par).is_some_and(|p| !p.has_alive_path(now)) {
                self.schedule_punch(now, par, caminho);
            }
        }
    }

    // ---------------------------------------------------------------- HELLO

    fn handle_hello(
        &mut self,
        now: u64,
        from: SocketAddr,
        mut bytes: Vec<u8>,
        source: Address,
        hops: u8,
        authenticated: bool,
    ) {
        let corpo = &bytes[idx::PAYLOAD..];
        if corpo.len() < HELLO_FIXED {
            return;
        }
        let protocolo = corpo[0];
        let versao = RemoteVersion {
            protocol: protocolo,
            major: corpo[1],
            minor: corpo[2],
            revision: u16::from_be_bytes([corpo[3], corpo[4]]),
        };
        let hora = u64::from_be_bytes(corpo[5..13].try_into().expect("8 bytes"));
        let Ok((id, usados)) = Identity::from_wire(&corpo[HELLO_FIXED..]) else {
            return;
        };
        if protocolo < PROTOCOL_MIN || id.address() != source {
            return;
        }
        // Um HELLO de quem não conhecemos nem procuramos não tem resposta: o app é invisível.
        // Visível, o desconhecido vira par (se a identidade é válida) e some de novo se o MAC
        // do HELLO não fecha.
        let mut criado = false;
        if self.visible && !self.peers.contains_key(&source) {
            if !id.validate() {
                return;
            }
            let Some(novo) = Peer::new(&self.identity, id.clone(), now) else {
                return;
            };
            self.peers.insert(source, novo);
            criado = true;
        }
        let Some(par) = self.peers.get(&source) else {
            return;
        };
        if par.identity().public_bytes() != id.public_bytes() || !self.can_talk(source) {
            return;
        }
        let chave = par.key().salsa.clone();
        if !authenticated && salsa::dearmor(&chave, &mut bytes).is_err() {
            if criado {
                self.peers.remove(&source);
            }
            return;
        }
        let mut pos = HELLO_FIXED + usados;
        let corpo_inicio = idx::PAYLOAD;
        // O endereço pelo qual o outro lado nos vê.
        if pos < bytes.len() - corpo_inicio
            && let Ok((visto, n)) = inet::read(&bytes[corpo_inicio + pos..])
        {
            pos += n;
            if let Some(visto) = visto
                && hops == 0
                && self.is_upstream(source)
            {
                self.external.insert(source, visto);
                self.effects.push(Effect::SeenAddress(visto));
            }
        }
        // O planet que o outro lado tem (id e hora): 16 bytes.
        let mut mundo_dele: Option<(u64, u64)> = None;
        if corpo_inicio + pos + 16 <= bytes.len() {
            let a = corpo_inicio + pos;
            mundo_dele = Some((
                u64::from_be_bytes(bytes[a..a + 8].try_into().expect("8 bytes")),
                u64::from_be_bytes(bytes[a + 8..a + 16].try_into().expect("8 bytes")),
            ));
            pos += 16;
        }
        // O resto vem cifrado: as moons que o outro lado tem ou quer.
        let mut luas: Vec<(u64, u64)> = Vec::new();
        if corpo_inicio + pos < bytes.len() {
            salsa::crypt_field(&chave, &mut bytes, corpo_inicio + pos);
            let r = &bytes[corpo_inicio + pos..];
            if r.len() >= 2 {
                let n = usize::from(u16::from_be_bytes([r[0], r[1]]));
                for i in 0..n {
                    let a = 2 + i * 17;
                    if a + 17 > r.len() {
                        break;
                    }
                    if r[a] == 127 {
                        luas.push((
                            u64::from_be_bytes(r[a + 1..a + 9].try_into().expect("8 bytes")),
                            u64::from_be_bytes(r[a + 9..a + 17].try_into().expect("8 bytes")),
                        ));
                    }
                }
            }
        }
        // O OK(HELLO): eco da hora, a nossa versão, o endereço de onde o vimos e as
        // atualizações de mundo que ele não tem.
        let mut carga = Vec::new();
        carga.push(Verb::Hello.to_u8());
        carga.extend_from_slice(&packet::id_of(&bytes).to_be_bytes());
        carga.extend_from_slice(&hora.to_be_bytes());
        carga.push(self.version.protocol);
        carga.push(self.version.major);
        carga.push(self.version.minor);
        carga.extend_from_slice(&self.version.revision.to_be_bytes());
        inet::write(&mut carga, Some(from));
        let mut mundos = Vec::new();
        if let Some((id_planet, hora_planet)) = mundo_dele
            && id_planet != 0
            && id_planet == self.planet.id()
            && self.planet.timestamp() > hora_planet
        {
            mundos.extend_from_slice(&self.planet.to_bytes());
        }
        for lua in &self.moons {
            if let Some((_, hora_lua)) = luas.iter().find(|(id, _)| *id == lua.id())
                && lua.timestamp() > *hora_lua
            {
                mundos.extend_from_slice(&lua.to_bytes());
            }
        }
        carga.extend_from_slice(&(mundos.len() as u16).to_be_bytes());
        carga.extend_from_slice(&mundos);
        self.send_via(now, source, Verb::Ok, &carga, from, false);
        if let Some(par) = self.peers.get_mut(&source) {
            par.set_version(versao);
        }
        self.touch(now, from, source, hops, Verb::Hello);
    }

    /// Manda um HELLO a `dest`. Com `at`, direto a esse endereço; sem, pelo melhor raiz.
    pub fn send_hello(&mut self, now: u64, dest: Address, at: Option<SocketAddr>) {
        let Some(par) = self.peers.get(&dest) else {
            return;
        };
        let chave = par.key().salsa.clone();
        let estendido = self.extended_hello
            && self.version.protocol >= 13
            && !self.is_upstream(dest)
            && par.version().is_some_and(|v| v.protocol >= 13);
        let identidade_par = estendido.then(|| par.identity().clone());
        let id = self.take_id();
        let mut p = Packet::new(id, dest, self.address(), Verb::Hello);
        // A marca entra na mistura da chave da Salsa: liga antes de armar.
        p.set_extended_armor(estendido);
        p.append(&[
            self.version.protocol,
            self.version.major,
            self.version.minor,
        ]);
        p.append(&self.version.revision.to_be_bytes());
        p.append(&now.to_be_bytes());
        p.append(&self.identity.to_wire());
        let mut fim = Vec::new();
        inet::write(&mut fim, at);
        p.append(&fim);
        p.append(&self.planet.id().to_be_bytes());
        p.append(&self.planet.timestamp().to_be_bytes());
        let inicio_cifrado = p.len();
        let mut luas: Vec<u8> = Vec::new();
        let mut quantas: u16 = 0;
        for lua in &self.moons {
            luas.push(127);
            luas.extend_from_slice(&lua.id().to_be_bytes());
            luas.extend_from_slice(&lua.timestamp().to_be_bytes());
            quantas += 1;
        }
        for (id_lua, _) in &self.orbits {
            if !self.moons.iter().any(|m| m.id() == *id_lua) {
                // Queremos esta moon e ainda não temos: hora 0 pede a versão inteira.
                luas.push(127);
                luas.extend_from_slice(&id_lua.to_be_bytes());
                luas.extend_from_slice(&0u64.to_be_bytes());
                quantas += 1;
            }
        }
        p.append(&quantas.to_be_bytes());
        p.append(&luas);
        let mut bytes = p.into_bytes();
        salsa::crypt_field(&chave, &mut bytes, inicio_cifrado);
        salsa::armor(&chave, &mut bytes, false);
        if let Some(para) = identidade_par
            && extended_armor(&mut bytes, &para).is_err()
        {
            return;
        }
        let rota = match at {
            Some(a) => Some(a),
            None => self.route(now, dest),
        };
        let Some(rota) = rota else { return };
        self.expecting.insert(
            id,
            Expected {
                sent: now,
                verb: Verb::Hello,
            },
        );
        self.last_hello_to.insert(rota, now);
        self.mark_sent(now, dest, rota);
        self.emit(rota, &bytes);
    }

    // ---------------------------------------------------------------- OK, ERROR, ECHO

    fn handle_ok(&mut self, now: u64, from: SocketAddr, source: Address, hops: u8, p: &Packet) {
        let carga = p.payload();
        if carga.len() < 9 {
            return;
        }
        let re_verbo = Verb::from_u8(carga[0] & MASK_VERB);
        let re_id = u64::from_be_bytes(carga[1..9].try_into().expect("8 bytes"));
        let Some(esperado) = self.expecting.get(&re_id) else {
            return; // não pedimos: some em silêncio
        };
        if esperado.verb != re_verbo {
            return; // a resposta não bate com o que perguntamos
        }
        let enviado_em = esperado.sent;
        let resto = &carga[9..];
        let upstream = self.is_upstream(source);
        match re_verbo {
            Verb::Hello => {
                // eco da hora (8) ‖ protocolo ‖ maior ‖ menor ‖ revisão (2) ‖ endereço visto ‖
                // tamanho dos mundos (2) ‖ mundos
                if resto.len() < 13 {
                    return;
                }
                let eco = u64::from_be_bytes(resto[0..8].try_into().expect("8 bytes"));
                let versao = RemoteVersion {
                    protocol: resto[8],
                    major: resto[9],
                    minor: resto[10],
                    revision: u16::from_be_bytes([resto[11], resto[12]]),
                };
                if versao.protocol < PROTOCOL_MIN {
                    return;
                }
                let mut pos = 13;
                let mut visto = None;
                if pos < resto.len()
                    && let Ok((v, n)) = inet::read(&resto[pos..])
                {
                    visto = v;
                    pos += n;
                }
                let mut mundos: Vec<World> = Vec::new();
                if pos + 2 <= resto.len() {
                    let tamanho = usize::from(u16::from_be_bytes([resto[pos], resto[pos + 1]]));
                    pos += 2;
                    let fim = (pos + tamanho).min(resto.len());
                    let mut a = pos;
                    // Mundo novo só de raiz ou da semente de uma moon que pedimos.
                    let aceita_mundos =
                        upstream || self.orbits.iter().any(|(_, semente)| *semente == source);
                    while aceita_mundos && a < fim {
                        match World::parse_prefix(&resto[a..fim]) {
                            Ok((w, n)) => {
                                mundos.push(w);
                                a += n;
                            }
                            Err(_) => break,
                        }
                    }
                }
                if let Some(par) = self.peers.get_mut(&source) {
                    par.set_version(versao);
                }
                self.touch(now, from, source, hops, Verb::Ok);
                if hops == 0 {
                    let ms = u32::try_from(now.saturating_sub(eco)).unwrap_or(u32::MAX);
                    if let Some(c) = self.peers.get_mut(&source).and_then(|p| p.path_mut(from)) {
                        c.set_latency(ms);
                    }
                    if let Some(visto) = visto
                        && upstream
                    {
                        self.external.insert(source, visto);
                        self.effects.push(Effect::SeenAddress(visto));
                    }
                }
                for w in mundos {
                    self.consider_world(now, w);
                }
            }
            Verb::Echo => {
                self.touch(now, from, source, hops, Verb::Ok);
                if hops == 0 {
                    let ms = u32::try_from(now.saturating_sub(enviado_em)).unwrap_or(u32::MAX);
                    if let Some(c) = self.peers.get_mut(&source).and_then(|p| p.path_mut(from)) {
                        c.set_latency(ms);
                    }
                }
            }
            Verb::Whois => {
                // Só raiz sabe responder a um WHOIS nosso.
                if !upstream {
                    return;
                }
                self.touch(now, from, source, hops, Verb::Ok);
                let mut pos = 0;
                while pos < resto.len() {
                    let Ok((id, n)) = Identity::from_wire(&resto[pos..]) else {
                        break;
                    };
                    pos += n;
                    let endereco = id.address();
                    // Só aceita o que foi pedido e ainda não temos.
                    if self.whois_sent.remove(&endereco).is_none()
                        || self.peers.contains_key(&endereco)
                    {
                        continue;
                    }
                    if let Some(mut novo) = Peer::new(&self.identity, id, now) {
                        // Se havia algo nosso esperando por ele, fomos nós que o procuramos.
                        if self.waiting_tx.iter().any(|w| w.dest == endereco)
                            || self.punches.iter().any(|f| f.with == endereco)
                        {
                            novo.set_contacted();
                        }
                        self.peers.insert(endereco, novo);
                        self.effects.push(Effect::PeerKnown(endereco));
                        self.flush_waiting(now, endereco);
                        if let Some(caminho) = self.memorized.get(&endereco).copied() {
                            self.schedule_punch(now, endereco, caminho);
                        }
                    }
                }
            }
            _ => {
                self.touch(now, from, source, hops, Verb::Ok);
                self.effects.push(Effect::Deliver(Delivery {
                    from: source,
                    verb: Verb::Ok,
                    id: p.id(),
                    hops,
                    via: from,
                    in_re: Some((re_verbo, re_id)),
                    error: None,
                    payload: resto.to_vec(),
                }));
            }
        }
    }

    fn handle_error(&mut self, now: u64, from: SocketAddr, source: Address, hops: u8, p: &Packet) {
        let carga = p.payload();
        if carga.len() < 10 {
            return;
        }
        let re_verbo = Verb::from_u8(carga[0] & MASK_VERB);
        let re_id = u64::from_be_bytes(carga[1..9].try_into().expect("8 bytes"));
        if !self.expecting.contains_key(&re_id) {
            return;
        }
        let codigo = ErrorCode::from_u8(carga[9]);
        self.touch(now, from, source, hops, Verb::Error);
        if re_verbo == Verb::Hello && codigo == ErrorCode::IdentityCollision {
            // O outro lado já conhece outra identidade com o nosso endereço: o VL2 mostra.
        }
        self.effects.push(Effect::Deliver(Delivery {
            from: source,
            verb: Verb::Error,
            id: p.id(),
            hops,
            via: from,
            in_re: Some((re_verbo, re_id)),
            error: Some(codigo),
            payload: carga[10..].to_vec(),
        }));
    }

    fn handle_echo(&mut self, now: u64, from: SocketAddr, source: Address, hops: u8, p: &Packet) {
        if !self.can_talk(source) {
            return;
        }
        if hops == 0 {
            let liberado = self
                .peers
                .get_mut(&source)
                .and_then(|par| par.path_mut(from))
                .is_none_or(|c| c.allow_echo_reply(now));
            if !liberado {
                return;
            }
        }
        let mut carga = Vec::with_capacity(9 + p.payload().len());
        carga.push(Verb::Echo.to_u8());
        carga.extend_from_slice(&p.id().to_be_bytes());
        carga.extend_from_slice(p.payload());
        self.send_via(now, source, Verb::Ok, &carga, from, false);
        self.touch(now, from, source, hops, Verb::Echo);
    }

    // ---------------------------------------------------------------- WHOIS e as filas

    /// Pergunta ao melhor raiz quem é este endereço (no máximo um pedido por meio segundo).
    pub fn request_whois(&mut self, now: u64, endereco: Address) {
        if endereco == self.address() {
            return;
        }
        if let Some(ultimo) = self.whois_sent.get(&endereco)
            && now.saturating_sub(*ultimo) < WHOIS_RETRY_DELAY
        {
            return;
        }
        let Some(raiz) = self.best_root_except(now, Some(endereco)) else {
            return;
        };
        self.whois_sent.insert(endereco, now);
        self.send_packet(now, raiz, Verb::Whois, &endereco.to_bytes(), false);
    }

    /// A identidade de `endereco` chegou: despacha os pacotes que esperavam por ela.
    fn flush_waiting(&mut self, now: u64, endereco: Address) {
        let (prontos, resto): (Vec<_>, Vec<_>) = std::mem::take(&mut self.waiting_rx)
            .into_iter()
            .partition(|w| w.source == endereco);
        self.waiting_rx = resto;
        for w in prontos {
            self.receive_packet(now, w.from, w.bytes);
        }
        let (prontos, resto): (Vec<_>, Vec<_>) = std::mem::take(&mut self.waiting_tx)
            .into_iter()
            .partition(|w| w.dest == endereco);
        self.waiting_tx = resto;
        for w in prontos {
            self.send_with(now, w.dest, w.verb, &w.payload, w.compress);
        }
    }

    // ---------------------------------------------------------------- envio

    /// Manda `verbo` com `carga` a `dest`: direto se há caminho vivo, senão pelo melhor raiz.
    /// Sem a identidade do destino (WHOIS) ou sem rota, espera na fila até cinco segundos.
    /// Procurar um par o torna alguém que pode falar conosco.
    pub fn send(&mut self, now: u64, dest: Address, verbo: Verb, carga: &[u8]) {
        self.send_with(now, dest, verbo, carga, false);
    }

    /// Como [`Vl1::send`], comprimindo a carga com LZ4 se isso a diminuir.
    pub fn send_with(
        &mut self,
        now: u64,
        dest: Address,
        verbo: Verb,
        carga: &[u8],
        comprimir: bool,
    ) {
        if dest == self.address() {
            return;
        }
        if !self.is_upstream(dest) {
            self.mark_contacted(dest);
        }
        if self.send_packet(now, dest, verbo, carga, comprimir) {
            return;
        }
        self.waiting_tx.push(WaitingTx {
            dest,
            verb: verbo,
            payload: carga.to_vec(),
            compress: comprimir,
            since: now,
        });
        if !self.peers.contains_key(&dest) {
            self.request_whois(now, dest);
        }
    }

    fn take_id(&mut self) -> u64 {
        self.next_id = self.next_id.wrapping_add(8);
        self.next_id
    }

    /// A rota física até `dest`: o melhor caminho direto vivo ou o do melhor raiz.
    fn route(&self, now: u64, dest: Address) -> Option<SocketAddr> {
        let par = self.peers.get(&dest)?;
        if (self.direct_paths || self.is_upstream(dest))
            && let Some(c) = par.best_path(now)
        {
            return Some(c.addr());
        }
        let raiz = self.best_root_except(now, Some(dest))?;
        self.peers.get(&raiz)?.best_path(now).map(|c| c.addr())
    }

    /// Registra que saiu algo por `rota`: o caminho é do par `dest` ou do raiz que repassa.
    fn mark_sent(&mut self, now: u64, _dest: Address, rota: SocketAddr) {
        for par in self.peers.values_mut() {
            if let Some(c) = par.path_mut(rota) {
                c.sent(now);
            }
        }
    }

    /// Escreve o pacote já armado em pedaços do tamanho do UDP.
    fn emit(&mut self, para: SocketAddr, bytes: &[u8]) {
        if let Ok(pedacos) = packet::split(bytes) {
            for pedaco in pedacos {
                self.effects.push(Effect::Send {
                    to: para,
                    data: pedaco,
                });
            }
        }
    }

    /// Arma e manda um pacote ao par. `false` se não há como mandar agora.
    fn send_packet(
        &mut self,
        now: u64,
        dest: Address,
        verbo: Verb,
        carga: &[u8],
        comprimir: bool,
    ) -> bool {
        let Some(rota) = self.route(now, dest) else {
            return false;
        };
        self.send_via(now, dest, verbo, carga, rota, comprimir);
        true
    }

    /// Arma (Salsa20/12 + Poly1305, cifrando) e manda por `rota`.
    fn send_via(
        &mut self,
        now: u64,
        dest: Address,
        verbo: Verb,
        carga: &[u8],
        rota: SocketAddr,
        comprimir: bool,
    ) {
        let Some(par) = self.peers.get(&dest) else {
            return;
        };
        let aes = self.uses_aes(par);
        let chave = par.key().clone();
        let id = self.take_id();
        let mut p = Packet::new(id, dest, self.address(), verbo);
        p.append(carga);
        if comprimir {
            p.compress();
        }
        // A marca de fragmentado entra na chave do MAC: tem de estar ligada antes de armar.
        if p.len() > UDP_PAYLOAD_MTU {
            p.set_fragmented(true);
        }
        let mut bytes = p.into_bytes();
        let id = armor_encrypted(&chave, aes, &mut bytes);
        if !matches!(verbo, Verb::Ok | Verb::Error) {
            self.expecting.insert(
                id,
                Expected {
                    sent: now,
                    verb: verbo,
                },
            );
        }
        self.mark_sent(now, dest, rota);
        self.emit(rota, &bytes);
    }

    // ---------------------------------------------------------------- relógio

    /// O relógio andou: HELLO e heartbeat nos caminhos, WHOIS repetido, limpeza das filas.
    pub fn tick(&mut self, now: u64) {
        // Os furos de NAT repetem de segundo em segundo: não esperam a checagem de 5 s.
        self.run_punches(now);
        if self.last_check != 0 && now.saturating_sub(self.last_check) < PING_CHECK_INTERVAL {
            return;
        }
        self.last_check = now;
        self.prune(now);
        if self.direct_paths {
            self.try_memorized(now);
        }

        // Os raiz: HELLO em todos os endereços enquanto não há caminho vivo; depois, o
        // mesmo ritmo dos outros pares.
        let raizes: Vec<RootInfo> = self.roots.values().cloned().collect();
        for raiz in raizes {
            let vivo = self
                .peers
                .get(&raiz.address)
                .is_some_and(|p| p.has_alive_path(now));
            if !vivo {
                for endereco in raiz.endpoints.iter().filter(|e| e.is_ipv4()) {
                    self.send_hello(now, raiz.address, Some(*endereco));
                }
            }
        }
        let enderecos: Vec<Address> = self.peers.keys().copied().collect();
        for endereco in enderecos {
            self.ping_and_keepalive(now, endereco);
        }

        // As moons que queremos e ainda não temos: acha a semente e manda HELLO a ela, que
        // responde com o mundo no OK(HELLO).
        let sementes: Vec<Address> = self
            .orbits
            .iter()
            .filter(|(id, _)| !self.moons.iter().any(|m| m.id() == *id))
            .map(|(_, semente)| *semente)
            .collect();
        for semente in sementes {
            if self.peers.contains_key(&semente) {
                self.send_hello(now, semente, None);
            } else {
                self.request_whois(now, semente);
            }
        }
        // WHOIS que ainda não foi respondido.
        let mut procurados: Vec<Address> = self
            .waiting_rx
            .iter()
            .map(|w| w.source)
            .chain(self.waiting_tx.iter().map(|w| w.dest))
            .filter(|a| !self.peers.contains_key(a))
            .collect();
        procurados.sort();
        procurados.dedup();
        for a in procurados {
            self.request_whois(now, a);
        }
        // Pacotes nossos que esperavam uma rota.
        let esperando = std::mem::take(&mut self.waiting_tx);
        for w in esperando {
            if self.peers.contains_key(&w.dest)
                && self.send_packet(now, w.dest, w.verb, &w.payload, w.compress)
            {
                continue;
            }
            self.waiting_tx.push(w);
        }
    }

    /// HELLO completo a cada minuto e ECHO a cada heartbeat, nos caminhos vivos de um par.
    fn ping_and_keepalive(&mut self, now: u64, endereco: Address) {
        let Some(par) = self.peers.get_mut(&endereco) else {
            return;
        };
        par.expire_paths(now);
        let completo = par.take_full_hello(now);
        let usa_eco = par.version().is_some_and(|v| v.protocol >= 5);
        let alvos: Vec<(SocketAddr, bool)> = par
            .paths()
            .iter()
            .map(|c| (c.addr(), c.needs_heartbeat(now)))
            .collect();
        for (alvo, precisa) in alvos {
            if completo || !usa_eco {
                if completo || precisa {
                    self.confirm_path(now, endereco, alvo);
                }
            } else if precisa {
                self.send_echo(now, endereco, alvo);
            }
        }
    }

    fn send_echo(&mut self, now: u64, dest: Address, em: SocketAddr) {
        let Some(par) = self.peers.get(&dest) else {
            return;
        };
        let aes = self.uses_aes(par);
        let chave = par.key().clone();
        let id = self.take_id();
        let p = Packet::new(id, dest, self.address(), Verb::Echo);
        let mut bytes = p.into_bytes();
        let id = armor_encrypted(&chave, aes, &mut bytes);
        self.expecting.insert(
            id,
            Expected {
                sent: now,
                verb: Verb::Echo,
            },
        );
        self.mark_sent(now, dest, em);
        self.emit(em, &bytes);
    }

    fn prune(&mut self, now: u64) {
        self.expecting
            .retain(|_, e| now.saturating_sub(e.sent) < REPLY_WINDOW);
        self.reassembly
            .retain(|_, r| now.saturating_sub(r.since) < FRAGMENT_TIMEOUT);
        self.waiting_rx
            .retain(|w| now.saturating_sub(w.since) < RECEIVE_QUEUE_TIMEOUT);
        self.waiting_tx
            .retain(|w| now.saturating_sub(w.since) < TRANSMIT_QUEUE_TIMEOUT);
        let controlador = self.controller;
        let raizes = &self.roots;
        self.peers.retain(|a, p| {
            raizes.contains_key(a)
                || controlador == Some(*a)
                || now.saturating_sub(p.last_receive()) < PEER_ACTIVITY_TIMEOUT
        });
        self.whois_sent
            .retain(|_, t| now.saturating_sub(*t) < RECEIVE_QUEUE_TIMEOUT);
        self.last_hello_to
            .retain(|_, t| now.saturating_sub(*t) < PING_PERIOD);
        self.push_sent
            .retain(|_, t| now.saturating_sub(*t) < DIRECT_PATH_PUSH_INTERVAL_HAVEPATH);
        self.push_received
            .retain(|_, (t, _)| now.saturating_sub(*t) < PUSH_DIRECT_PATHS_CUTOFF_TIME);
    }
}

/// Arma um pacote cifrado (AES-GMAC-SIV ou Salsa20/12) e devolve o ID que viaja: no AES é a
/// metade da etiqueta, e é a ele que as respostas se referem.
fn armor_encrypted(chave: &PeerKey, aes: bool, bytes: &mut [u8]) -> u64 {
    if aes {
        gmac_siv::armor(&chave.aes, bytes);
    } else {
        salsa::armor(&chave.salsa, bytes, true);
    }
    packet::id_of(bytes)
}
