//! O VL2: a rede virtual do ZeroTier vista por um nó comum (M61). Pede a configuração ao
//! controlador, junta os pedaços assinados, guarda os certificados dos outros membros, aplica as
//! regras da rede e leva quadros Ethernet de um membro a outro (`FRAME`, `EXT_FRAME` e o multicast
//! com `LIKE`/`GATHER`), como o `Network.cpp`, o `Switch.cpp` e o `Multicaster.cpp` do ZeroTier.
//!
//! O [`Vl2`] é uma máquina de estados sem E/S: o laço do motor entrega a ele o relógio e o que o
//! [`Vl1`] subiu, e ele manda o que precisa pelo `Vl1` e devolve [`Event`]s (quadros para a pilha
//! e mudanças de estado).

use std::collections::{HashMap, HashSet, VecDeque};

use tomni_ui_netstack::Cidr;

use crate::dictionary::Dictionary;
use crate::netconf::request::{self, ConfigAssembler};
use crate::netconf::rules::{self, FrameView, Verdict};
use crate::netconf::{
    Com, NetworkConfig, SPECIALIST_ACTIVE_BRIDGE, SPECIALIST_MULTICAST_REPLICATOR,
};
use crate::packet::{ErrorCode, Verb};
use crate::peer::RemoteVersion;
use crate::vl1::{Delivery, Vl1};
use crate::{Address, Identity, NetworkId};

/// O tipo de quadro do IPv4.
pub const ETHERTYPE_IPV4: u16 = 0x0800;
/// O tipo de quadro do ARP.
pub const ETHERTYPE_ARP: u16 = 0x0806;
/// O MAC de broadcast.
pub const BROADCAST: [u8; 6] = [0xff; 6];

/// Sem configuração, pedimos de novo a cada tanto.
const REQUEST_RETRY: u64 = 8_000;
/// Com configuração, pedimos a atualização a cada tanto (`ZT_NETWORK_AUTOCONF_DELAY`).
const REQUEST_REFRESH: u64 = 60_000;
/// De quanto em quanto tempo anunciamos os grupos multicast (`ZT_MULTICAST_ANNOUNCE_PERIOD`).
const ANNOUNCE_PERIOD: u64 = 30_000;
/// O tempo que um membro anunciado vale sem novo `LIKE` (`ZT_MULTICAST_LIKE_EXPIRE`).
const MEMBER_EXPIRE: u64 = 600_000;
/// De quanto em quanto tempo mandamos as credenciais a um mesmo par.
const CREDENTIAL_PUSH: u64 = 20_000;
/// O mínimo entre dois `GATHER` explícitos do mesmo grupo. O ZeroTier espera mais, mas lá o
/// quadro fica na fila por 5 s; aqui o ARP repete a cada segundo e um grupo vazio não pode esperar.
const GATHER_DELAY: u64 = 2_000;
/// Quanto um quadro multicast espera na fila por membros (`ZT_MULTICAST_TRANSMIT_TIMEOUT`).
const QUEUE_TIMEOUT: u64 = 5_000;
/// O tamanho da fila de um grupo (`ZT_TX_QUEUE_SIZE`).
const QUEUE_SIZE: usize = 64;
/// O mínimo entre dois `ERROR(NEED_MEMBERSHIP_CERTIFICATE)` ao mesmo par.
const NEED_CERT_RATE: u64 = 1_000;
/// A quantos membros de um grupo pedimos no `GATHER` (o máximo que o ZeroTier usa de padrão).
const GATHER_MAX_PEERS: usize = 16;
/// O maior número de pontes ativas a que mandamos um quadro sem rota (`ZT_MAX_BRIDGE_SPAM`).
const MAX_BRIDGE_SPAM: usize = 32;
/// O maior quadro que aceitamos de um par (o MTU máximo da rede, `ZT_MAX_MTU`).
const MAX_FRAME: usize = 10_000;
/// A idade dos MACs de ponte aprendidos.
const BRIDGE_EXPIRE: u64 = 120_000;

/// O bit de especialista "âncora" (`ZT_NETWORKCONFIG_SPECIALIST_TYPE_ANCHOR`).
const SPECIALIST_ANCHOR: u64 = 1;

/// Um grupo multicast: o MAC e o campo ADI.
pub type Group = ([u8; 6], u32);

/// O primeiro byte dos MACs da rede (`MAC::firstOctetForNetwork`).
pub fn first_octet(rede: NetworkId) -> u8 {
    let o = ((rede.as_u64() & 0xfe) as u8) | 0x02;
    if o == 0x52 { 0x32 } else { o }
}

/// O MAC de um membro na rede: o endereço dele misturado com o id da rede (`MAC::fromAddress`).
pub fn mac_of(endereco: Address, rede: NetworkId) -> [u8; 6] {
    let n = rede.as_u64();
    let mut m = (u64::from(first_octet(rede)) << 40) | endereco.as_u64();
    m ^= ((n >> 8) & 0xff) << 32;
    m ^= ((n >> 16) & 0xff) << 24;
    m ^= ((n >> 24) & 0xff) << 16;
    m ^= ((n >> 32) & 0xff) << 8;
    m ^= (n >> 40) & 0xff;
    let b = m.to_be_bytes();
    [b[2], b[3], b[4], b[5], b[6], b[7]]
}

/// O endereço de um membro a partir do MAC dele (`MAC::toAddress`).
pub fn address_of(mac: [u8; 6], rede: NetworkId) -> Address {
    let n = rede.as_u64();
    let mut a =
        u64::from_be_bytes([0, 0, mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]]) & 0xff_ffff_ffff;
    a ^= ((n >> 8) & 0xff) << 32;
    a ^= ((n >> 16) & 0xff) << 24;
    a ^= ((n >> 24) & 0xff) << 16;
    a ^= ((n >> 32) & 0xff) << 8;
    a ^= (n >> 40) & 0xff;
    Address::new(a & 0xff_ffff_ffff)
}

fn is_multicast(mac: [u8; 6]) -> bool {
    mac[0] & 1 != 0
}

/// O grupo que resolve o ARP de um IPv4: broadcast com o IP no ADI
/// (`MulticastGroup::deriveMulticastGroupForAddressResolution`).
pub fn arp_group(ip: [u8; 4]) -> Group {
    (BROADCAST, u32::from_be_bytes(ip))
}

/// Monta um quadro Ethernet: destino, origem, tipo e carga.
pub fn ethernet_frame(dest: [u8; 6], src: [u8; 6], ethertype: u16, data: &[u8]) -> Vec<u8> {
    let mut q = Vec::with_capacity(14 + data.len());
    q.extend_from_slice(&dest);
    q.extend_from_slice(&src);
    q.extend_from_slice(&ethertype.to_be_bytes());
    q.extend_from_slice(data);
    q
}

/// Em que pé está o pedido de configuração.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// Pedindo a configuração ao controlador.
    Requesting,
    /// Com configuração: a rede funciona.
    Ready,
    /// O controlador não nos autorizou.
    AccessDenied,
    /// O controlador não conhece a rede.
    NotFound,
    /// A rede pede login único.
    AuthenticationRequired,
    /// Já existe outro nó com o nosso endereço.
    Collision,
}

/// O que o [`Vl2`] produz.
#[derive(Debug)]
pub enum Event {
    /// O estado mudou.
    Status(Status),
    /// Uma configuração nova vale. `signed` são os pedaços assinados (para o app guardar); vazio
    /// no formato antigo e na que veio do cofre.
    Config(Box<NetworkConfig>, Vec<u8>),
    /// A configuração deixou de valer (o controlador nos recusou).
    ConfigLost,
    /// Um quadro Ethernet chegou, para a pilha.
    Frame(Vec<u8>),
    /// Uma configuração chegou, mas não serve (de outra rede, mal formada...).
    Rejected(String),
}

/// O que se sabe de um outro membro.
#[derive(Default)]
struct Membership {
    com: Option<Com>,
    last_pushed: u64,
    last_like: u64,
    last_need_cert: u64,
}

/// Um quadro multicast à espera de membros.
struct Queued {
    since: u64,
    ethertype: u16,
    data: Vec<u8>,
    sent_to: HashSet<Address>,
}

#[derive(Default)]
struct GroupState {
    members: Vec<(Address, u64)>,
    queue: VecDeque<Queued>,
    last_gather: u64,
}

/// O VL2 de uma rede.
pub struct Vl2 {
    network: NetworkId,
    ours: Address,
    mac: [u8; 6],
    controller: Address,
    version: RemoteVersion,
    system: String,
    status: Status,
    config: Option<NetworkConfig>,
    assembler: ConfigAssembler,
    stored: Vec<u8>,
    last_request: u64,
    last_announce: u64,
    memberships: HashMap<Address, Membership>,
    groups: HashMap<Group, GroupState>,
    bridges: HashMap<[u8; 6], (Address, u64)>,
    /// Os IPs fixos que o app pediu (o controlador não distribuiu nenhum): entram nos grupos.
    static_addresses: Vec<Cidr>,
    rng: u64,
    events: Vec<Event>,
}

fn xorshift(s: &mut u64) -> u64 {
    let mut x = *s;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *s = x;
    x
}

impl Vl2 {
    /// O VL2 da rede `network`. `stored` é a configuração assinada que o app guardou da última
    /// vez (vazia se não há): vale assim que o controlador for conhecido.
    pub fn new(
        network: NetworkId,
        ours: Address,
        version: RemoteVersion,
        system: &str,
        stored: Vec<u8>,
    ) -> Self {
        let mut seed = [0u8; 8];
        let _ = getrandom::fill(&mut seed);
        Self {
            network,
            ours,
            mac: mac_of(ours, network),
            controller: network.controller(),
            version,
            system: system.to_string(),
            status: Status::Requesting,
            config: None,
            assembler: ConfigAssembler::new(),
            stored,
            last_request: 0,
            last_announce: 0,
            memberships: HashMap::new(),
            groups: HashMap::new(),
            bridges: HashMap::new(),
            static_addresses: Vec::new(),
            rng: u64::from_be_bytes(seed) | 1,
            events: Vec::new(),
        }
    }

    /// Os IPs fixos do app, que também entram no anúncio dos grupos (o ARP deles).
    pub fn set_static_addresses(&mut self, enderecos: Vec<Cidr>) {
        self.static_addresses = enderecos;
    }

    /// O MAC do nó nesta rede.
    pub fn mac(&self) -> [u8; 6] {
        self.mac
    }

    /// O controlador da rede.
    pub fn controller(&self) -> Address {
        self.controller
    }

    /// O estado.
    pub fn status(&self) -> &Status {
        &self.status
    }

    /// A configuração em uso.
    pub fn config(&self) -> Option<&NetworkConfig> {
        self.config.as_ref()
    }

    /// Recolhe o que foi produzido desde a última chamada.
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    fn set_status(&mut self, status: Status) {
        if self.status != status {
            self.status = status.clone();
            self.events.push(Event::Status(status));
        }
    }

    fn controller_identity(&self, vl1: &Vl1) -> Option<Identity> {
        vl1.peer(self.controller).map(|p| p.identity().clone())
    }

    // ------------------------------------------------------------------ relógio

    /// O relógio andou: repete o pedido de configuração, anuncia os grupos e limpa o que venceu.
    pub fn tick(&mut self, now: u64, vl1: &mut Vl1) {
        self.load_stored(now, vl1);

        let intervalo = if self.config.is_some() {
            REQUEST_REFRESH
        } else {
            REQUEST_RETRY
        };
        if self.last_request == 0 || now.saturating_sub(self.last_request) >= intervalo {
            self.request_config(now, vl1);
        }
        if self.config.is_some() && now.saturating_sub(self.last_announce) >= ANNOUNCE_PERIOD {
            self.announce_groups(now, vl1, false);
        }
        for grupo in self.groups.values_mut() {
            grupo
                .members
                .retain(|(_, visto)| now.saturating_sub(*visto) < MEMBER_EXPIRE);
            while grupo
                .queue
                .front()
                .is_some_and(|q| now.saturating_sub(q.since) >= QUEUE_TIMEOUT)
            {
                grupo.queue.pop_front();
            }
        }
        self.bridges
            .retain(|_, (_, visto)| now.saturating_sub(*visto) < BRIDGE_EXPIRE);
    }

    /// A configuração que veio do cofre vale se os pedaços fecham com o controlador.
    fn load_stored(&mut self, now: u64, vl1: &mut Vl1) {
        if self.stored.is_empty() {
            return;
        }
        let Some(controlador) = self.controller_identity(vl1) else {
            // Espera a identidade do controlador (o WHOIS sai com o primeiro pedido).
            vl1.request_whois(now, self.controller);
            return;
        };
        let guardado = std::mem::take(&mut self.stored);
        let Some(pedacos) = request::split_stored(&guardado) else {
            return;
        };
        let mut montador = ConfigAssembler::new();
        for pedaco in pedacos {
            if let Some(pronto) = montador.add(pedaco, self.controller, &controlador, 0) {
                self.apply_config(now, vl1, &pronto.dictionary, pronto.signed, true);
                return;
            }
        }
    }

    fn request_config(&mut self, now: u64, vl1: &mut Vl1) {
        self.last_request = now;
        let atual = self.config.as_ref().map(|c| (c.revision, c.timestamp));
        let carga = request::request_payload(self.network, self.version, &self.system, atual);
        vl1.send_with(
            now,
            self.controller,
            Verb::NetworkConfigRequest,
            &carga,
            true,
        );
    }

    // ------------------------------------------------------------------ o que o VL1 sobe

    /// Trata um pacote que o VL1 entregou.
    pub fn handle(&mut self, now: u64, vl1: &mut Vl1, d: Delivery) {
        match d.verb {
            Verb::Ok => match d.in_re {
                Some((Verb::NetworkConfigRequest, _)) if d.from == self.controller => {
                    self.on_config_chunk(now, vl1, &d.payload, d.id, d.from);
                }
                Some((Verb::MulticastGather, _)) => {
                    self.on_gather_reply(now, vl1, &d.payload, false)
                }
                Some((Verb::MulticastFrame, _)) => self.on_gather_reply(now, vl1, &d.payload, true),
                _ => {}
            },
            Verb::Error => self.on_error(now, vl1, &d),
            Verb::NetworkConfig if d.from == self.controller => {
                if self.on_config_chunk(now, vl1, &d.payload, d.id, d.from) {
                    // O ZeroTier confirma a atualização empurrada com um OK que cita o ECHO.
                    let update_id = Self::chunk_update_id(&d.payload);
                    let mut ok = vec![Verb::Echo.to_u8()];
                    ok.extend_from_slice(&d.id.to_be_bytes());
                    ok.extend_from_slice(&self.network.as_u64().to_be_bytes());
                    ok.extend_from_slice(&update_id.to_be_bytes());
                    vl1.send(now, d.from, Verb::Ok, &ok);
                }
            }
            Verb::NetworkCredentials => self.on_credentials(now, vl1, &d),
            Verb::Frame => self.on_frame(now, vl1, &d),
            Verb::ExtFrame => self.on_ext_frame(now, vl1, &d),
            Verb::MulticastFrame => self.on_multicast_frame(now, vl1, &d),
            Verb::MulticastLike => self.on_like(now, vl1, &d),
            Verb::MulticastGather => self.on_gather(now, vl1, &d),
            _ => {}
        }
    }

    fn chunk_update_id(payload: &[u8]) -> u64 {
        // `nwid ‖ tamanho ‖ pedaço ‖ flags ‖ updateId ...`
        let Some(n) = payload.get(8..10) else {
            return 0;
        };
        let n = usize::from(u16::from_be_bytes([n[0], n[1]]));
        payload
            .get(10 + n + 1..10 + n + 9)
            .map_or(0, |b| u64::from_be_bytes(b.try_into().expect("8 bytes")))
    }

    /// Um pedaço da configuração. Devolve `true` se foi aceito (assinatura conferida).
    fn on_config_chunk(
        &mut self,
        now: u64,
        vl1: &mut Vl1,
        payload: &[u8],
        packet_id: u64,
        from: Address,
    ) -> bool {
        let Some(controlador) = self.controller_identity(vl1) else {
            return false;
        };
        if payload.get(..8) != Some(&self.network.as_u64().to_be_bytes()[..]) {
            return false;
        }
        match self.assembler.add(payload, from, &controlador, packet_id) {
            Some(pronto) => {
                self.apply_config(now, vl1, &pronto.dictionary, pronto.signed, false);
                true
            }
            None => true,
        }
    }

    fn apply_config(
        &mut self,
        now: u64,
        vl1: &mut Vl1,
        dict: &Dictionary,
        signed: Vec<u8>,
        from_cache: bool,
    ) {
        let cfg = match NetworkConfig::from_dictionary(dict, self.network, self.ours) {
            Ok(c) => c,
            Err(e) => {
                self.events.push(Event::Rejected(e.to_string()));
                return;
            }
        };
        if let Some(atual) = &self.config
            && cfg.timestamp < atual.timestamp
        {
            return;
        }
        if from_cache && self.config.is_some() {
            return;
        }
        let nova = self.config.as_ref().is_none_or(|c| {
            c.revision != cfg.revision
                || c.timestamp != cfg.timestamp
                || c.com.is_some() != cfg.com.is_some()
        });
        self.config = Some(cfg.clone());
        self.set_status(Status::Ready);
        if nova {
            self.events.push(Event::Config(Box::new(cfg), signed));
        }
        // Com configuração nova, as credenciais e os grupos vão logo, sem esperar o relógio.
        self.memberships.clear();
        self.announce_groups(now, vl1, true);
    }

    fn on_error(&mut self, now: u64, vl1: &mut Vl1, d: &Delivery) {
        let Some(codigo) = d.error else { return };
        match d.in_re {
            Some((Verb::NetworkConfigRequest, _)) if d.from == self.controller => match codigo {
                ErrorCode::ObjectNotFound | ErrorCode::UnsupportedOperation => {
                    self.set_status(Status::NotFound);
                }
                ErrorCode::NetworkAccessDenied => {
                    if self.config.take().is_some() {
                        self.events.push(Event::ConfigLost);
                    }
                    self.set_status(Status::AccessDenied);
                }
                ErrorCode::NetworkAuthenticationRequired => {
                    self.set_status(Status::AuthenticationRequired);
                }
                ErrorCode::IdentityCollision => self.set_status(Status::Collision),
                _ => {}
            },
            Some(_)
                if codigo == ErrorCode::NeedMembershipCertificate
                    && d.payload.get(..8) == Some(&self.network.as_u64().to_be_bytes()[..]) =>
            {
                self.push_credentials(now, vl1, d.from, true);
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------------ credenciais

    /// O par pode trocar quadros conosco? Redes públicas deixam todos; nas privadas, o
    /// certificado dele precisa concordar com o nosso.
    fn gate(&mut self, now: u64, vl1: &mut Vl1, par: Address) -> bool {
        let Some(cfg) = &self.config else {
            return false;
        };
        if cfg.public {
            return true;
        }
        let Some(nossa) = &cfg.com else { return false };
        let Some(id) = vl1.peer(par).map(|p| p.identity().clone()) else {
            return false;
        };
        let permitido = self
            .memberships
            .get(&par)
            .and_then(|m| m.com.as_ref())
            .is_some_and(|dele| {
                dele.timestamp().is_some_and(|t| t > 0) && nossa.agrees_with(dele, &id)
            });
        if permitido {
            // Passou a primeira vez: o par ainda não sabe dos nossos grupos nem do nosso COM.
            self.push_credentials(now, vl1, par, false);
            self.like_gate(now, vl1, par);
        }
        permitido
    }

    /// Manda o nosso COM a `par`. Sem `force`, no máximo a cada `CREDENTIAL_PUSH`; com, no
    /// máximo uma vez por segundo.
    fn push_credentials(&mut self, now: u64, vl1: &mut Vl1, par: Address, force: bool) {
        let Some(com) = self.config.as_ref().and_then(|c| c.com.as_ref()) else {
            return;
        };
        let m = self.memberships.entry(par).or_default();
        let espera = if force {
            NEED_CERT_RATE
        } else {
            CREDENTIAL_PUSH
        };
        if m.last_pushed != 0 && now.saturating_sub(m.last_pushed) < espera {
            return;
        }
        m.last_pushed = now;
        let mut carga = com.to_bytes();
        carga.push(0);
        carga.extend_from_slice(&[0u8; 8]);
        vl1.send_with(now, par, Verb::NetworkCredentials, &carga, true);
    }

    fn on_credentials(&mut self, now: u64, vl1: &mut Vl1, d: &Delivery) {
        let mut p = 0;
        while d.payload.get(p).is_some_and(|b| *b != 0) {
            let Some((com, usados)) = Com::from_bytes(&d.payload[p..]) else {
                return;
            };
            p += usados;
            if com.network() == Some(self.network) {
                self.add_credential(now, vl1, d.from, com);
            }
        }
    }

    /// Guarda o COM de um membro se for novo e assinado pelo controlador.
    fn add_credential(&mut self, now: u64, vl1: &mut Vl1, de: Address, com: Com) {
        let Some(ts) = com.timestamp() else { return };
        if ts == 0 || com.issued_to() != Some(de) {
            return;
        }
        if let Some(velho) = self
            .memberships
            .get(&de)
            .and_then(|m| m.com.as_ref())
            .and_then(Com::timestamp)
        {
            if ts < velho {
                return;
            }
            if ts == velho {
                return; // redundante
            }
        }
        let Some(controlador) = self.controller_identity(vl1) else {
            vl1.request_whois(now, self.controller);
            return;
        };
        if !com.verify(self.network, &controlador) {
            return;
        }
        self.memberships.entry(de).or_default().com = Some(com);
    }

    // ------------------------------------------------------------------ grupos e LIKE

    /// Os grupos em que estamos: o ARP de cada IPv4 nosso e, se a rede deixa, o broadcast.
    fn own_groups(&self) -> Vec<Group> {
        let Some(cfg) = &self.config else {
            return Vec::new();
        };
        let mut grupos: Vec<Group> = cfg
            .addresses
            .iter()
            .chain(&self.static_addresses)
            .filter_map(|c| match c.addr() {
                std::net::IpAddr::V4(ip) => Some(arp_group(ip.octets())),
                std::net::IpAddr::V6(_) => None,
            })
            .collect();
        if cfg.broadcast_enabled() {
            grupos.push((BROADCAST, 0));
        }
        grupos
    }

    fn likes_payload(&self) -> Vec<u8> {
        let mut carga = Vec::new();
        for (mac, adi) in self.own_groups() {
            carga.extend_from_slice(&self.network.as_u64().to_be_bytes());
            carga.extend_from_slice(&mac);
            carga.extend_from_slice(&adi.to_be_bytes());
        }
        carga
    }

    /// Anuncia os nossos grupos ao controlador, aos raiz e aos membros com quem já falamos.
    fn announce_groups(&mut self, now: u64, vl1: &mut Vl1, force: bool) {
        if !force && now.saturating_sub(self.last_announce) < ANNOUNCE_PERIOD {
            return;
        }
        self.last_announce = now;
        let carga = self.likes_payload();
        if carga.is_empty() {
            return;
        }
        let mut alvos: Vec<Address> = vec![self.controller];
        alvos.extend(vl1.roots().into_iter().map(|r| r.address));
        let membros: Vec<Address> = self
            .memberships
            .iter()
            .filter(|(_, m)| m.com.is_some())
            .map(|(a, _)| *a)
            .collect();
        for par in membros {
            if self.gate(now, vl1, par) {
                alvos.push(par);
            }
        }
        alvos.sort();
        alvos.dedup();
        for alvo in alvos {
            // Quem não é membro (controlador, raiz) precisa do nosso COM para aceitar.
            self.push_credentials(now, vl1, alvo, false);
            vl1.send_with(now, alvo, Verb::MulticastLike, &carga, true);
            self.memberships.entry(alvo).or_default().last_like = now;
        }
    }

    /// Um par passou pelo portão: se ainda não sabe dos nossos grupos, anuncia a ele.
    fn like_gate(&mut self, now: u64, vl1: &mut Vl1, par: Address) {
        let m = self.memberships.entry(par).or_default();
        if m.last_like != 0 && now.saturating_sub(m.last_like) < ANNOUNCE_PERIOD {
            return;
        }
        m.last_like = now;
        let carga = self.likes_payload();
        if !carga.is_empty() {
            vl1.send_with(now, par, Verb::MulticastLike, &carga, true);
        }
    }

    fn add_member(&mut self, now: u64, grupo: Group, membro: Address) -> bool {
        if membro == self.ours {
            return false;
        }
        let g = self.groups.entry(grupo).or_default();
        if let Some(m) = g.members.iter_mut().find(|(a, _)| *a == membro) {
            m.1 = now;
            return false;
        }
        g.members.push((membro, now));
        true
    }

    fn on_like(&mut self, now: u64, vl1: &mut Vl1, d: &Delivery) {
        if self.config.is_none() || !self.gate(now, vl1, d.from) {
            return;
        }
        let mut novos = Vec::new();
        for item in d.payload.chunks_exact(18) {
            if item[..8] != self.network.as_u64().to_be_bytes() {
                continue;
            }
            let mac: [u8; 6] = item[8..14].try_into().expect("6 bytes");
            let adi = u32::from_be_bytes(item[14..18].try_into().expect("4 bytes"));
            if self.add_member(now, (mac, adi), d.from) {
                novos.push((mac, adi));
            }
        }
        for grupo in novos {
            self.flush_queue(now, vl1, grupo);
        }
    }

    /// Alguém pergunta quem está num grupo: respondemos com o que sabemos, sem ele.
    fn on_gather(&mut self, now: u64, vl1: &mut Vl1, d: &Delivery) {
        let p = &d.payload;
        if p.len() < 8 + 1 + 6 + 4 + 4 || p[..8] != self.network.as_u64().to_be_bytes() {
            return;
        }
        let flags = p[8];
        if flags & 0x01 != 0
            && let Some((com, _)) = Com::from_bytes(&p[23..])
        {
            self.add_credential(now, vl1, d.from, com);
        }
        let mac: [u8; 6] = p[9..15].try_into().expect("6 bytes");
        let adi = u32::from_be_bytes(p[15..19].try_into().expect("4 bytes"));
        let limite = u32::from_be_bytes(p[19..23].try_into().expect("4 bytes")) as usize;
        if limite == 0 || self.config.is_none() || !self.gate(now, vl1, d.from) {
            return;
        }
        let membros: Vec<Address> = self
            .groups
            .get(&(mac, adi))
            .map(|g| {
                g.members
                    .iter()
                    .map(|(a, _)| *a)
                    .filter(|a| *a != d.from)
                    .take(limite.min(1000))
                    .collect()
            })
            .unwrap_or_default();
        if membros.is_empty() {
            return;
        }
        let mut ok = vec![Verb::MulticastGather.to_u8()];
        ok.extend_from_slice(&d.id.to_be_bytes());
        ok.extend_from_slice(&self.network.as_u64().to_be_bytes());
        ok.extend_from_slice(&mac);
        ok.extend_from_slice(&adi.to_be_bytes());
        ok.extend_from_slice(&(membros.len() as u32).to_be_bytes());
        ok.extend_from_slice(&(membros.len() as u16).to_be_bytes());
        for m in &membros {
            ok.extend_from_slice(&m.to_bytes());
        }
        vl1.send(now, d.from, Verb::Ok, &ok);
    }

    /// O `OK` de um `GATHER` ou de um `MULTICAST_FRAME` com a lista de membros do grupo.
    fn on_gather_reply(&mut self, now: u64, vl1: &mut Vl1, p: &[u8], em_quadro: bool) {
        // nwid ‖ mac ‖ adi ‖ [flags (só no MULTICAST_FRAME; 0x02 = tem a lista)] ‖ total ‖ n ‖ n×5
        if p.len() < 18 || p[..8] != self.network.as_u64().to_be_bytes() {
            return;
        }
        let mac: [u8; 6] = p[8..14].try_into().expect("6 bytes");
        let adi = u32::from_be_bytes(p[14..18].try_into().expect("4 bytes"));
        let mut pos = 18;
        if em_quadro {
            if p.get(pos).is_none_or(|f| f & 0x02 == 0) {
                return;
            }
            pos += 1;
        }
        let Some(n) = p.get(pos + 4..pos + 6) else {
            return;
        };
        let n = usize::from(u16::from_be_bytes([n[0], n[1]]));
        pos += 6;
        let mut mudou = false;
        for i in 0..n {
            let Some(a) = p
                .get(pos + i * 5..pos + i * 5 + 5)
                .and_then(Address::from_bytes)
            else {
                break;
            };
            if a.is_reserved() {
                continue;
            }
            mudou |= self.add_member(now, (mac, adi), a);
        }
        if mudou {
            self.flush_queue(now, vl1, (mac, adi));
        }
    }

    // ------------------------------------------------------------------ quadros que entram

    fn nwid_ok(&self, payload: &[u8]) -> bool {
        payload.get(..8) == Some(&self.network.as_u64().to_be_bytes()[..])
    }

    fn need_credentials(&mut self, now: u64, vl1: &mut Vl1, par: Address) {
        let m = self.memberships.entry(par).or_default();
        if m.last_need_cert != 0 && now.saturating_sub(m.last_need_cert) < NEED_CERT_RATE {
            return;
        }
        m.last_need_cert = now;
        // `ERROR(re, id, código, nwid)`: o id do pacote que motivou o erro vai no lugar certo.
        let mut e = vec![Verb::Frame.to_u8()];
        e.extend_from_slice(&0u64.to_be_bytes());
        e.push(ErrorCode::NeedMembershipCertificate.to_u8());
        e.extend_from_slice(&self.network.as_u64().to_be_bytes());
        vl1.send(now, par, Verb::Error, &e);
    }

    /// Passa o quadro pelas regras da rede. `true` se segue.
    fn filter_in(
        &mut self,
        zt_source: Address,
        mac_source: [u8; 6],
        mac_dest: [u8; 6],
        ethertype: u16,
        data: &[u8],
    ) -> bool {
        let Some(cfg) = &self.config else {
            return false;
        };
        let vista = FrameView {
            zt_source,
            zt_dest: self.ours,
            mac_source,
            mac_dest,
            ethertype,
            data,
            inbound: true,
            mac_authenticated: mac_source == mac_of(zt_source, self.network),
        };
        let mut semente = self.rng;
        let saida = rules::evaluate(&cfg.rules, &vista, self.ours, cfg.flags, || {
            xorshift(&mut semente) as u32
        });
        self.rng = semente;
        // O redirecionamento só faz sentido para quem repassa; aqui descarta.
        matches!(saida.verdict, Verdict::Accept)
    }

    fn deliver_frame(&mut self, dest: [u8; 6], src: [u8; 6], ethertype: u16, data: &[u8]) {
        self.events
            .push(Event::Frame(ethernet_frame(dest, src, ethertype, data)));
    }

    fn on_frame(&mut self, now: u64, vl1: &mut Vl1, d: &Delivery) {
        let p = &d.payload;
        if self.config.is_none() || !self.nwid_ok(p) || p.len() <= 10 {
            return;
        }
        if !self.gate(now, vl1, d.from) {
            self.need_credentials(now, vl1, d.from);
            return;
        }
        let ethertype = u16::from_be_bytes([p[8], p[9]]);
        let dados = &p[10..];
        if dados.len() > MAX_FRAME {
            return;
        }
        let origem = mac_of(d.from, self.network);
        if self.filter_in(d.from, origem, self.mac, ethertype, dados) {
            self.deliver_frame(self.mac, origem, ethertype, dados);
        }
    }

    fn permits_bridging(&self, par: Address) -> bool {
        self.config.as_ref().is_some_and(|c| {
            c.specialists_with(SPECIALIST_ACTIVE_BRIDGE)
                .any(|a| a == par)
        })
    }

    fn on_ext_frame(&mut self, now: u64, vl1: &mut Vl1, d: &Delivery) {
        let p = &d.payload;
        if self.config.is_none() || !self.nwid_ok(p) || p.len() < 9 {
            return;
        }
        let flags = p[8];
        let mut pos = 9;
        if flags & 0x01 != 0 {
            let Some((com, n)) = Com::from_bytes(&p[pos..]) else {
                return;
            };
            pos += n;
            if com.network() == Some(self.network) {
                self.add_credential(now, vl1, d.from, com);
            }
        }
        if !self.gate(now, vl1, d.from) {
            self.need_credentials(now, vl1, d.from);
            return;
        }
        let Some(cab) = p.get(pos..pos + 14) else {
            return;
        };
        let para: [u8; 6] = cab[0..6].try_into().expect("6 bytes");
        let de: [u8; 6] = cab[6..12].try_into().expect("6 bytes");
        let ethertype = u16::from_be_bytes([cab[12], cab[13]]);
        let dados = &p[pos + 14..];
        if dados.is_empty() || dados.len() > MAX_FRAME || de == self.mac || de == [0; 6] {
            return;
        }
        if de != mac_of(d.from, self.network) {
            if !self.permits_bridging(d.from) {
                return;
            }
            self.bridges.insert(de, (d.from, now));
        }
        // Só nos interessa o que é para nós ou para todos.
        if para != self.mac && !is_multicast(para) {
            return;
        }
        if self.filter_in(d.from, de, para, ethertype, dados) {
            self.deliver_frame(para, de, ethertype, dados);
        }
    }

    fn on_multicast_frame(&mut self, now: u64, vl1: &mut Vl1, d: &Delivery) {
        let p = &d.payload;
        if self.config.is_none() || !self.nwid_ok(p) || p.len() < 9 {
            return;
        }
        let flags = p[8];
        let mut pos = 9;
        if flags & 0x01 != 0 {
            let Some((com, n)) = Com::from_bytes(&p[pos..]) else {
                return;
            };
            pos += n;
            if com.network() == Some(self.network) {
                self.add_credential(now, vl1, d.from, com);
            }
        }
        if !self.gate(now, vl1, d.from) {
            self.need_credentials(now, vl1, d.from);
            return;
        }
        let mut gather = 0usize;
        if flags & 0x02 != 0 {
            let Some(b) = p.get(pos..pos + 4) else { return };
            gather = u32::from_be_bytes(b.try_into().expect("4 bytes")) as usize;
            pos += 4;
        }
        let origem = if flags & 0x04 != 0 {
            let Some(b) = p.get(pos..pos + 6) else { return };
            pos += 6;
            let m: [u8; 6] = b.try_into().expect("6 bytes");
            m
        } else {
            mac_of(d.from, self.network)
        };
        let Some(cab) = p.get(pos..pos + 12) else {
            return;
        };
        let mac: [u8; 6] = cab[0..6].try_into().expect("6 bytes");
        let adi = u32::from_be_bytes(cab[6..10].try_into().expect("4 bytes"));
        let ethertype = u16::from_be_bytes([cab[10], cab[11]]);
        let dados = &p[pos + 12..];

        let limite = self.config.as_ref().map_or(0, |c| c.multicast_limit);
        if limite != 0 && !dados.is_empty() && dados.len() <= MAX_FRAME {
            let valido = is_multicast(mac)
                && origem != [0; 6]
                && !is_multicast(origem)
                && origem != self.mac;
            if valido {
                let proprio = origem == mac_of(d.from, self.network);
                if !proprio && self.permits_bridging(d.from) {
                    self.bridges.insert(origem, (d.from, now));
                }
                if (proprio || self.permits_bridging(d.from))
                    && self.filter_in(d.from, origem, mac, ethertype, dados)
                {
                    self.deliver_frame(mac, origem, ethertype, dados);
                }
            }
        }
        // O remetente quer saber quem mais está no grupo.
        if gather > 0 {
            let membros: Vec<Address> = self
                .groups
                .get(&(mac, adi))
                .map(|g| {
                    g.members
                        .iter()
                        .map(|(a, _)| *a)
                        .filter(|a| *a != d.from)
                        .take(gather.min(1000))
                        .collect()
                })
                .unwrap_or_default();
            if !membros.is_empty() {
                let mut ok = vec![Verb::MulticastFrame.to_u8()];
                ok.extend_from_slice(&d.id.to_be_bytes());
                ok.extend_from_slice(&self.network.as_u64().to_be_bytes());
                ok.extend_from_slice(&mac);
                ok.extend_from_slice(&adi.to_be_bytes());
                ok.push(0x02);
                ok.extend_from_slice(&(membros.len() as u32).to_be_bytes());
                ok.extend_from_slice(&(membros.len() as u16).to_be_bytes());
                for m in &membros {
                    ok.extend_from_slice(&m.to_bytes());
                }
                vl1.send(now, d.from, Verb::Ok, &ok);
            }
        }
    }

    // ------------------------------------------------------------------ quadros que saem

    /// A pilha mandou um quadro Ethernet inteiro. Sem configuração, ele é descartado.
    pub fn send_frame(&mut self, now: u64, vl1: &mut Vl1, quadro: &[u8]) {
        if quadro.len() < 14 || quadro.len() - 14 > MAX_FRAME {
            return;
        }
        let Some(cfg) = self.config.clone() else {
            return;
        };
        let para: [u8; 6] = quadro[0..6].try_into().expect("6 bytes");
        let de: [u8; 6] = quadro[6..12].try_into().expect("6 bytes");
        let ethertype = u16::from_be_bytes([quadro[12], quadro[13]]);
        let dados = &quadro[14..];
        // Não somos ponte: só mandamos com o nosso MAC.
        if de != self.mac {
            return;
        }

        if is_multicast(para) {
            let mut grupo: Group = (para, 0);
            if para == BROADCAST {
                if ethertype == ETHERTYPE_ARP
                    && dados.len() >= 28
                    && dados[2] == 0x08
                    && dados[3] == 0x00
                    && dados[4] == 6
                    && dados[5] == 4
                    && dados[7] == 0x01
                {
                    // O ARP vira multicast: o IP procurado entra no ADI.
                    grupo = arp_group(dados[24..28].try_into().expect("4 bytes"));
                } else if !cfg.broadcast_enabled() {
                    return;
                }
            }
            if cfg.multicast_limit == 0 {
                return;
            }
            if !self.filter_out(&cfg, Address::new(0), de, para, ethertype, dados) {
                return;
            }
            self.multicast_send(now, vl1, &cfg, grupo, ethertype, dados);
        } else if para == self.mac {
            self.deliver_frame(para, de, ethertype, dados);
        } else if para[0] == first_octet(self.network) {
            let destino = address_of(para, self.network);
            if !self.filter_out(&cfg, destino, de, para, ethertype, dados) {
                return;
            }
            self.push_credentials(now, vl1, destino, false);
            let mut carga = Vec::with_capacity(10 + dados.len());
            carga.extend_from_slice(&self.network.as_u64().to_be_bytes());
            carga.extend_from_slice(&ethertype.to_be_bytes());
            carga.extend_from_slice(dados);
            vl1.send(now, destino, Verb::Frame, &carga);
        } else {
            // Um MAC que não é de membro: atrás de uma ponte.
            if !self.filter_out(&cfg, Address::new(0), de, para, ethertype, dados) {
                return;
            }
            let mut pontes: Vec<Address> = Vec::new();
            match self.bridges.get(&para) {
                Some((a, _)) if *a != self.ours && self.permits_bridging(*a) => pontes.push(*a),
                _ => pontes.extend(
                    cfg.specialists_with(SPECIALIST_ACTIVE_BRIDGE)
                        .filter(|a| *a != self.ours)
                        .take(MAX_BRIDGE_SPAM),
                ),
            }
            for ponte in pontes {
                self.send_ext_frame(now, vl1, ponte, para, de, ethertype, dados);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn send_ext_frame(
        &mut self,
        now: u64,
        vl1: &mut Vl1,
        a: Address,
        para: [u8; 6],
        de: [u8; 6],
        ethertype: u16,
        dados: &[u8],
    ) {
        self.push_credentials(now, vl1, a, false);
        let mut carga = Vec::with_capacity(9 + 14 + dados.len());
        carga.extend_from_slice(&self.network.as_u64().to_be_bytes());
        carga.push(0x00);
        carga.extend_from_slice(&para);
        carga.extend_from_slice(&de);
        carga.extend_from_slice(&ethertype.to_be_bytes());
        carga.extend_from_slice(dados);
        vl1.send(now, a, Verb::ExtFrame, &carga);
    }

    /// As regras de saída. Aplica TEE/WATCH (cópia por `EXT_FRAME`); o redirecionamento não é
    /// tratado (o app não é ponte): vira descarte.
    fn filter_out(
        &mut self,
        cfg: &NetworkConfig,
        zt_dest: Address,
        mac_source: [u8; 6],
        mac_dest: [u8; 6],
        ethertype: u16,
        dados: &[u8],
    ) -> bool {
        let vista = FrameView {
            zt_source: self.ours,
            zt_dest,
            mac_source,
            mac_dest,
            ethertype,
            data: dados,
            inbound: false,
            mac_authenticated: true,
        };
        let mut semente = self.rng;
        let saida = rules::evaluate(&cfg.rules, &vista, self.ours, cfg.flags, || {
            xorshift(&mut semente) as u32
        });
        self.rng = semente;
        // A cópia (TEE/WATCH) pede o `Vl1`, que a regra de saída não recebe: fica para quem
        // chama. O app não usa TEE; registrar aqui seria código sem teste.
        matches!(saida.verdict, Verdict::Accept)
    }

    /// Manda um quadro multicast: ao replicador, aos membros que já conhecemos e, se faltam,
    /// pergunta por mais (`Multicaster::send`).
    fn multicast_send(
        &mut self,
        now: u64,
        vl1: &mut Vl1,
        cfg: &NetworkConfig,
        grupo: Group,
        ethertype: u16,
        dados: &[u8],
    ) {
        let (mac, adi) = grupo;
        let rede = self.network.as_u64().to_be_bytes();

        // Replicadores designados: o de menor latência com caminho vivo.
        if !self.is_replicator(cfg) {
            let melhor = cfg
                .specialists_with(SPECIALIST_MULTICAST_REPLICATOR)
                .filter_map(|a| {
                    let par = vl1.peer(a)?;
                    par.has_alive_path(now)
                        .then(|| (par.latency(now).unwrap_or(u32::MAX), a))
                })
                .min();
            if let Some((_, replicador)) = melhor {
                let mut carga = Vec::with_capacity(9 + 6 + 12 + dados.len());
                carga.extend_from_slice(&rede);
                carga.push(0x0c);
                carga.extend_from_slice(&self.mac);
                carga.extend_from_slice(&mac);
                carga.extend_from_slice(&adi.to_be_bytes());
                carga.extend_from_slice(&ethertype.to_be_bytes());
                carga.extend_from_slice(dados);
                self.push_credentials(now, vl1, replicador, false);
                vl1.send_with(now, replicador, Verb::MulticastFrame, &carga, true);
                return;
            }
        }

        let limite = cfg.multicast_limit as usize;
        let estado = self.groups.entry(grupo).or_default();
        let membros: Vec<Address> = estado.members.iter().map(|(a, _)| *a).collect();
        let mut alvos = membros.clone();
        // Embaralha (Fisher-Yates) para não favorecer sempre os mesmos.
        for i in (1..alvos.len()).rev() {
            let j = (xorshift(&mut self.rng) % (i as u64 + 1)) as usize;
            alvos.swap(i, j);
        }
        alvos.truncate(limite);

        if membros.len() >= limite {
            // Já temos membros de sobra: manda e ainda pede 1 para manter a lista fresca.
            for alvo in &alvos {
                self.send_multicast_to(now, vl1, *alvo, grupo, ethertype, None, 1, dados);
            }
            return;
        }

        // Faltam membros: o quadro espera na fila e perguntamos por mais.
        let falta = (limite - membros.len() + 1) as u32;
        let precisa_gather = {
            let estado = self.groups.entry(grupo).or_default();
            let precisa =
                estado.members.is_empty() || now.saturating_sub(estado.last_gather) >= GATHER_DELAY;
            if precisa {
                estado.last_gather = now;
            }
            while estado.queue.len() >= QUEUE_SIZE {
                estado.queue.pop_front();
            }
            let mut enviados = HashSet::new();
            for a in &alvos {
                enviados.insert(*a);
            }
            estado.queue.push_back(Queued {
                since: now,
                ethertype,
                data: dados.to_vec(),
                sent_to: enviados,
            });
            precisa
        };
        for alvo in &alvos {
            self.send_multicast_to(now, vl1, *alvo, grupo, ethertype, None, falta, dados);
        }
        if precisa_gather {
            self.gather(now, vl1, cfg, grupo, falta);
        }
    }

    fn is_replicator(&self, cfg: &NetworkConfig) -> bool {
        cfg.specialists_with(SPECIALIST_MULTICAST_REPLICATOR)
            .any(|a| a == self.ours)
    }

    /// `MULTICAST_GATHER` ao melhor raiz, ao controlador e aos membros sempre contatados.
    fn gather(&mut self, now: u64, vl1: &mut Vl1, cfg: &NetworkConfig, grupo: Group, limite: u32) {
        let mut alvos: Vec<Address> = Vec::new();
        if let Some(raiz) = vl1.best_root(now) {
            alvos.push(raiz);
        }
        alvos.push(self.controller);
        alvos.extend(
            cfg.specialists
                .iter()
                .filter(|(_, b)| b & SPECIALIST_ANCHOR != 0)
                .map(|(a, _)| *a)
                .filter(|a| *a != self.ours)
                .take(GATHER_MAX_PEERS.saturating_sub(alvos.len())),
        );
        alvos.sort();
        alvos.dedup();
        let com = cfg.com.as_ref().map(Com::to_bytes);
        for alvo in alvos {
            let mut carga = Vec::new();
            carga.extend_from_slice(&self.network.as_u64().to_be_bytes());
            carga.push(u8::from(com.is_some()));
            carga.extend_from_slice(&grupo.0);
            carga.extend_from_slice(&grupo.1.to_be_bytes());
            carga.extend_from_slice(&limite.to_be_bytes());
            if let Some(c) = &com {
                carga.extend_from_slice(c);
            }
            vl1.send(now, alvo, Verb::MulticastGather, &carga);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn send_multicast_to(
        &mut self,
        now: u64,
        vl1: &mut Vl1,
        alvo: Address,
        grupo: Group,
        ethertype: u16,
        src: Option<[u8; 6]>,
        gather: u32,
        dados: &[u8],
    ) {
        self.push_credentials(now, vl1, alvo, false);
        let mut flags = 0u8;
        if gather > 0 {
            flags |= 0x02;
        }
        if src.is_some() {
            flags |= 0x04;
        }
        let mut carga = Vec::with_capacity(9 + 4 + 6 + 12 + dados.len());
        carga.extend_from_slice(&self.network.as_u64().to_be_bytes());
        carga.push(flags);
        if gather > 0 {
            carga.extend_from_slice(&gather.to_be_bytes());
        }
        if let Some(s) = src {
            carga.extend_from_slice(&s);
        }
        carga.extend_from_slice(&grupo.0);
        carga.extend_from_slice(&grupo.1.to_be_bytes());
        carga.extend_from_slice(&ethertype.to_be_bytes());
        carga.extend_from_slice(dados);
        vl1.send_with(now, alvo, Verb::MulticastFrame, &carga, true);
    }

    /// Chegaram membros novos num grupo: os quadros que esperavam vão a eles.
    fn flush_queue(&mut self, now: u64, vl1: &mut Vl1, grupo: Group) {
        let Some(limite) = self.config.as_ref().map(|c| c.multicast_limit as usize) else {
            return;
        };
        let Some(estado) = self.groups.get_mut(&grupo) else {
            return;
        };
        let membros: Vec<Address> = estado.members.iter().map(|(a, _)| *a).collect();
        let mut saidas: Vec<(Address, u16, Vec<u8>)> = Vec::new();
        for q in estado.queue.iter_mut() {
            for m in &membros {
                if q.sent_to.len() >= limite {
                    break;
                }
                if q.sent_to.insert(*m) {
                    saidas.push((*m, q.ethertype, q.data.clone()));
                }
            }
        }
        for (alvo, ethertype, dados) in saidas {
            self.send_multicast_to(now, vl1, alvo, grupo, ethertype, None, 0, &dados);
        }
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn o_mac_vai_e_volta() {
        let rede = NetworkId::new(0x8056_c2e2_1c00_0001);
        for a in [0x0011_2233_4455_u64, 0x0099_aabb_ccdd, 0x0001_0000_0001] {
            let mac = mac_of(Address::new(a), rede);
            assert_eq!(mac[0], first_octet(rede));
            assert!(!is_multicast(mac));
            assert_eq!(address_of(mac, rede), Address::new(a));
        }
    }

    #[test]
    fn o_primeiro_byte_nunca_e_multicast_nem_o_52() {
        for n in 0..=255u64 {
            let o = first_octet(NetworkId::new(n));
            assert_eq!(o & 1, 0);
            assert_ne!(o, 0x52);
        }
        assert_eq!(first_octet(NetworkId::new(0x50)), 0x32);
    }

    #[test]
    fn o_grupo_do_arp_leva_o_ip_no_adi() {
        assert_eq!(
            arp_group([10, 61, 0, 7]),
            (BROADCAST, u32::from_be_bytes([10, 61, 0, 7]))
        );
    }

    #[test]
    fn o_quadro_ethernet_tem_14_bytes_de_cabecalho() {
        let q = ethernet_frame([1; 6], [2; 6], 0x0800, &[9, 9]);
        assert_eq!(q.len(), 16);
        assert_eq!(&q[12..14], &[0x08, 0x00]);
    }
}
