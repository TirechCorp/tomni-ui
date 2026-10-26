//! Os parceiros de teste: um raiz falso que fala o VL1 de verdade sobre um [`Underlay`], para
//! os testes do motor rodarem sem rede e sem o `zerotier-one`.
//!
//! O raiz falso faz o que um raiz faz com o app: responde a HELLO de qualquer um, diz quem é
//! quem num WHOIS, responde ECHO e repassa pacotes de um par a outro. Não valida a prova de
//! trabalho das identidades que recebe: nos testes elas vêm das fixtures.

pub mod controller;

use std::collections::HashMap;
use std::future::poll_fn;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use tokio::task::JoinHandle;
use tomni_ui_netstack::Underlay;

use crate::crypto::PeerKey;
use crate::crypto::salsa;
use crate::packet::{
    Cipher, FLAG_FRAGMENTED, Fragment, MASK_CIPHER, MASK_HOPS, MASK_VERB, MIN_PACKET_LEN, Packet,
    Verb, dest_of, id_of, idx,
};
use crate::{Address, Identity, World, inet};

/// O que o raiz falso contou, para os testes conferirem.
#[derive(Default)]
pub struct RootStats {
    /// HELLO respondidos.
    pub hellos: AtomicU64,
    /// WHOIS respondidos.
    pub whois: AtomicU64,
    /// ECHO respondidos.
    pub echoes: AtomicU64,
    /// Pacotes repassados de um par a outro.
    pub relayed: AtomicU64,
    /// Datagramas descartados (destino desconhecido, MAC inválido).
    pub dropped: AtomicU64,
    /// RENDEZVOUS mandados (dois por par apresentado).
    pub rendezvous: AtomicU64,
}

impl RootStats {
    /// Lê um contador.
    pub fn get(c: &AtomicU64) -> u64 {
        c.load(Ordering::Relaxed)
    }
}

struct Known {
    identity: Identity,
    key: PeerKey,
    addr: SocketAddr,
}

struct State {
    identity: Identity,
    world: World,
    /// As moons que ele serve a quem as pede no HELLO.
    moons: Vec<World>,
    known: HashMap<Address, Known>,
    /// Quem anunciou (`MULTICAST_LIKE`) cada grupo: `(rede, mac, adi)`.
    grupos: HashMap<(u64, [u8; 6], u32), Vec<Address>>,
    /// Quando cada par de nós foi apresentado (RENDEZVOUS), para não repetir antes de 30 s.
    unidos: HashMap<(Address, Address), tokio::time::Instant>,
    /// Se o raiz apresenta os nós que ele repassa (o ZeroTier de verdade apresenta).
    apresentar: bool,
    next_id: u64,
}

/// De quanto em quanto tempo o raiz apresenta o mesmo par de nós (`ZT_MIN_UNITE_INTERVAL`).
const MIN_UNITE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

/// Um raiz do ZeroTier de mentira.
pub struct RootFake {
    state: Arc<Mutex<State>>,
    stats: Arc<RootStats>,
    address: Address,
    addr: SocketAddr,
    task: JoinHandle<()>,
}

impl RootFake {
    /// Sobe o raiz sobre `under`. `world` é o planet que ele oferece a quem tem um mais velho.
    pub fn start<U: Underlay>(identity: Identity, world: World, under: U) -> RootFake {
        let address = identity.address();
        let addr = under.local_addr().expect("endereço local do underlay");
        let state = Arc::new(Mutex::new(State {
            identity,
            world,
            moons: Vec::new(),
            known: HashMap::new(),
            grupos: HashMap::new(),
            unidos: HashMap::new(),
            apresentar: true,
            next_id: 0x0100_0000_0000_0000,
        }));
        let stats = Arc::new(RootStats::default());
        let task = tokio::spawn(run(Arc::new(under), state.clone(), stats.clone()));
        RootFake {
            state,
            stats,
            address,
            addr,
            task,
        }
    }

    /// O endereço ZeroTier do raiz.
    pub fn address(&self) -> Address {
        self.address
    }

    /// O endereço UDP do raiz.
    pub fn socket_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Os contadores.
    pub fn stats(&self) -> &RootStats {
        &self.stats
    }

    /// Troca o mundo que o raiz oferece (para testar a atualização do planet).
    pub fn set_world(&self, world: World) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .world = world;
    }

    /// Passa a servir uma moon: quem a pede no HELLO (ou tem uma mais velha) recebe o mundo
    /// no OK(HELLO).
    pub fn serve_moon(&self, moon: World) {
        let mut s = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        s.moons.retain(|m| m.id() != moon.id());
        s.moons.push(moon);
    }

    /// Passa a conhecer `identity` em `addr` sem esperar o HELLO dela, como um raiz de
    /// verdade que já viu todo mundo: serve para WHOIS e repasse.
    pub fn know(&self, identity: Identity, addr: SocketAddr) {
        let mut s = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(acordo) = s.identity.agree(&identity) {
            s.known.insert(
                identity.address(),
                Known {
                    key: PeerKey::from_agreement(&acordo),
                    identity,
                    addr,
                },
            );
        }
    }

    /// Liga ou desliga o RENDEZVOUS que o raiz manda aos dois lados de um repasse (ligado por
    /// padrão, como no ZeroTier de verdade).
    pub fn introduce(&self, ligado: bool) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .apresentar = ligado;
    }

    /// Quem o raiz conhece agora.
    pub fn known(&self) -> Vec<Address> {
        let mut v: Vec<Address> = self
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .known
            .keys()
            .copied()
            .collect();
        v.sort();
        v
    }
}

impl Drop for RootFake {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn run<U: Underlay>(under: Arc<U>, state: Arc<Mutex<State>>, stats: Arc<RootStats>) {
    let mut buf = vec![0u8; 2048];
    loop {
        let Ok((n, from)) = poll_fn(|cx| under.poll_recv_from(cx, &mut buf)).await else {
            continue;
        };
        let saidas = {
            let mut s = state.lock().unwrap_or_else(PoisonError::into_inner);
            handle(&mut s, &stats, from, &buf[..n])
        };
        for (to, data) in saidas {
            let _ = under.try_send_to(&data, to);
        }
    }
}

type Saidas = Vec<(SocketAddr, Vec<u8>)>;

fn handle(s: &mut State, stats: &RootStats, from: SocketAddr, data: &[u8]) -> Saidas {
    let drop = |stats: &RootStats| {
        stats.dropped.fetch_add(1, Ordering::Relaxed);
        Vec::new()
    };
    if data.len() < 16 {
        return drop(stats);
    }
    let dest = dest_of(data);
    if dest != s.identity.address() {
        return relay(s, stats, data, dest);
    }
    if data.len() < MIN_PACKET_LEN {
        return drop(stats);
    }
    let Some(source) = Address::from_bytes(&data[idx::SOURCE..]) else {
        return drop(stats);
    };
    let cifra = Cipher::from_bits((data[idx::FLAGS] & MASK_CIPHER) >> 3);
    let mut bytes = data.to_vec();
    // O verbo só está em claro no HELLO (suíte 0): nos outros pacotes o byte é cifrado e pode
    // parecer qualquer verbo.
    let verbo = Verb::from_u8(bytes[idx::VERB] & MASK_VERB);
    if cifra == Cipher::None && verbo == Verb::Hello {
        return hello(s, stats, from, bytes, source);
    }
    let Some(par) = s.known.get_mut(&source) else {
        return drop(stats);
    };
    if !matches!(cifra, Cipher::None | Cipher::Salsa2012)
        || salsa::dearmor(&par.key.salsa, &mut bytes).is_err()
    {
        return drop(stats);
    }
    par.addr = from;
    let Ok(mut pacote) = Packet::from_bytes(bytes) else {
        return drop(stats);
    };
    if pacote.uncompress().is_err() {
        return drop(stats);
    }
    match pacote.verb() {
        Verb::Whois => {
            stats.whois.fetch_add(1, Ordering::Relaxed);
            let mut carga = vec![Verb::Whois.to_u8()];
            carga.extend_from_slice(&pacote.id().to_be_bytes());
            for chunk in pacote.payload().chunks_exact(5) {
                if let Some(a) = Address::from_bytes(chunk)
                    && let Some(k) = s.known.get(&a)
                {
                    carga.extend_from_slice(&k.identity.to_wire());
                }
            }
            reply(s, source, from, &carga)
        }
        Verb::Echo => {
            stats.echoes.fetch_add(1, Ordering::Relaxed);
            let mut carga = vec![Verb::Echo.to_u8()];
            carga.extend_from_slice(&pacote.id().to_be_bytes());
            carga.extend_from_slice(pacote.payload());
            reply(s, source, from, &carga)
        }
        Verb::MulticastLike => {
            // Cada item: `rede ‖ mac ‖ adi`.
            for item in pacote.payload().chunks_exact(18) {
                let rede = u64::from_be_bytes(item[..8].try_into().expect("8 bytes"));
                let mac: [u8; 6] = item[8..14].try_into().expect("6 bytes");
                let adi = u32::from_be_bytes(item[14..18].try_into().expect("4 bytes"));
                let membros = s.grupos.entry((rede, mac, adi)).or_default();
                if !membros.contains(&source) {
                    membros.push(source);
                }
            }
            Vec::new()
        }
        Verb::MulticastGather => {
            // `rede ‖ flags ‖ mac ‖ adi ‖ limite [‖ COM]`.
            let p = pacote.payload();
            if p.len() < 23 {
                return drop(stats);
            }
            let rede = u64::from_be_bytes(p[..8].try_into().expect("8 bytes"));
            let mac: [u8; 6] = p[9..15].try_into().expect("6 bytes");
            let adi = u32::from_be_bytes(p[15..19].try_into().expect("4 bytes"));
            let limite = u32::from_be_bytes(p[19..23].try_into().expect("4 bytes")) as usize;
            let membros: Vec<Address> = s
                .grupos
                .get(&(rede, mac, adi))
                .map(|g| {
                    g.iter()
                        .copied()
                        .filter(|a| *a != source)
                        .take(limite.min(1000))
                        .collect()
                })
                .unwrap_or_default();
            if membros.is_empty() {
                return Vec::new();
            }
            let mut carga = vec![Verb::MulticastGather.to_u8()];
            carga.extend_from_slice(&pacote.id().to_be_bytes());
            carga.extend_from_slice(&rede.to_be_bytes());
            carga.extend_from_slice(&mac);
            carga.extend_from_slice(&adi.to_be_bytes());
            carga.extend_from_slice(&(membros.len() as u32).to_be_bytes());
            carga.extend_from_slice(&(membros.len() as u16).to_be_bytes());
            for m in &membros {
                carga.extend_from_slice(&m.to_bytes());
            }
            reply(s, source, from, &carga)
        }
        _ => drop(stats),
    }
}

/// Repassa o pacote ao destino, se o raiz o conhece: soma um salto e manda como veio. Quem
/// repassa apresenta os dois lados com um RENDEZVOUS a cada um (`ZT_MIN_UNITE_INTERVAL`).
fn relay(s: &mut State, stats: &RootStats, data: &[u8], dest: Address) -> Saidas {
    let Some(alvo) = s.known.get(&dest).map(|k| k.addr) else {
        stats.dropped.fetch_add(1, Ordering::Relaxed);
        return Vec::new();
    };
    let mut bytes = data.to_vec();
    let mut origem = None;
    if let Some(mut f) = Fragment::parse(&bytes) {
        if !f.increment_hops() {
            return Vec::new();
        }
        bytes = f.as_bytes().to_vec();
    } else {
        let saltos = bytes[idx::FLAGS] & MASK_HOPS;
        if saltos >= crate::packet::MAX_HOPS {
            return Vec::new();
        }
        bytes[idx::FLAGS] = (bytes[idx::FLAGS] & !MASK_HOPS) | (saltos + 1);
        origem = Address::from_bytes(&data[idx::SOURCE..]);
    }
    stats.relayed.fetch_add(1, Ordering::Relaxed);
    let mut saidas = vec![(alvo, bytes)];
    if s.apresentar
        && saltos_zero(data)
        && let Some(origem) = origem
    {
        saidas.extend(apresentar(s, stats, origem, dest));
    }
    saidas
}

/// O pacote ainda não passou por nenhum raiz.
fn saltos_zero(data: &[u8]) -> bool {
    data[idx::FLAGS] & MASK_HOPS == 0
}

/// Manda a `a` o endereço de `b` e a `b` o de `a`, se os dois são conhecidos e faz 30 s ou mais
/// da última vez.
fn apresentar(s: &mut State, stats: &RootStats, a: Address, b: Address) -> Saidas {
    let (Some(end_a), Some(end_b)) = (
        s.known.get(&a).map(|k| k.addr),
        s.known.get(&b).map(|k| k.addr),
    ) else {
        return Vec::new();
    };
    let chave = if a < b { (a, b) } else { (b, a) };
    let agora = tokio::time::Instant::now();
    if s.unidos
        .get(&chave)
        .is_some_and(|antes| agora.duration_since(*antes) < MIN_UNITE_INTERVAL)
    {
        return Vec::new();
    }
    s.unidos.insert(chave, agora);
    let mut saidas = Vec::new();
    for (para, onde, outro, endereco) in [(a, end_a, b, end_b), (b, end_b, a, end_a)] {
        let SocketAddr::V4(v4) = endereco else {
            continue;
        };
        let mut carga = vec![0u8];
        carga.extend_from_slice(&outro.to_bytes());
        carga.extend_from_slice(&v4.port().to_be_bytes());
        carga.push(4);
        carga.extend_from_slice(&v4.ip().octets());
        saidas.extend(send(s, para, onde, Verb::Rendezvous, &carga));
        stats.rendezvous.fetch_add(1, Ordering::Relaxed);
    }
    saidas
}

/// Armazena o par que se apresentou e responde o OK(HELLO).
fn hello(
    s: &mut State,
    stats: &RootStats,
    from: SocketAddr,
    mut bytes: Vec<u8>,
    source: Address,
) -> Saidas {
    let corpo = &bytes[idx::PAYLOAD..];
    let Some((id, usados)) = (corpo.len() > 13)
        .then(|| Identity::from_wire(&corpo[13..]).ok())
        .flatten()
    else {
        return Vec::new();
    };
    if id.address() != source {
        return Vec::new();
    }
    let Some(acordo) = s.identity.agree(&id) else {
        return Vec::new();
    };
    let key = PeerKey::from_agreement(&acordo);
    if salsa::dearmor(&key.salsa, &mut bytes).is_err() {
        stats.dropped.fetch_add(1, Ordering::Relaxed);
        return Vec::new();
    }
    let corpo = &bytes[idx::PAYLOAD..];
    let hora: [u8; 8] = corpo[5..13].try_into().expect("8 bytes");
    // Depois da identidade: o endereço visto (inet), o id e a data do planet dele.
    let mut mundo_dele = None;
    if let Ok((_, n)) = inet::read(&corpo[13 + usados..]) {
        let a = 13 + usados + n;
        if corpo.len() >= a + 16 {
            mundo_dele = Some((
                u64::from_be_bytes(corpo[a..a + 8].try_into().expect("8 bytes")),
                u64::from_be_bytes(corpo[a + 8..a + 16].try_into().expect("8 bytes")),
            ));
        }
    }
    // O resto do HELLO vem cifrado: as moons que o par tem ou quer (tipo, id, data).
    let mut luas: Vec<(u64, u64)> = Vec::new();
    if let Ok((_, n)) = inet::read(&bytes[idx::PAYLOAD + 13 + usados..]) {
        let inicio = idx::PAYLOAD + 13 + usados + n + 16;
        if inicio < bytes.len() {
            salsa::crypt_field(&key.salsa, &mut bytes, inicio);
            let r = &bytes[inicio..];
            if r.len() >= 2 {
                let n = usize::from(u16::from_be_bytes([r[0], r[1]]));
                for i in 0..n {
                    let a = 2 + i * 17;
                    if a + 17 > r.len() {
                        break;
                    }
                    luas.push((
                        u64::from_be_bytes(r[a + 1..a + 9].try_into().expect("8 bytes")),
                        u64::from_be_bytes(r[a + 9..a + 17].try_into().expect("8 bytes")),
                    ));
                }
            }
        }
    }
    let id_pacote = id_of(&bytes);
    let mut carga = vec![Verb::Hello.to_u8()];
    carga.extend_from_slice(&id_pacote.to_be_bytes());
    carga.extend_from_slice(&hora);
    carga.extend_from_slice(&[11, 1, 4]);
    carga.extend_from_slice(&7u16.to_be_bytes());
    inet::write(&mut carga, Some(from));
    let mut mundos = Vec::new();
    if let Some((mundo_id, mundo_ts)) = mundo_dele
        && mundo_id == s.world.id()
        && mundo_ts < s.world.timestamp()
    {
        mundos = s.world.to_bytes();
    }
    for lua in &s.moons {
        if luas
            .iter()
            .any(|(id, hora)| *id == lua.id() && *hora < lua.timestamp())
        {
            mundos.extend_from_slice(&lua.to_bytes());
        }
    }
    carga.extend_from_slice(&(mundos.len() as u16).to_be_bytes());
    carga.extend_from_slice(&mundos);
    stats.hellos.fetch_add(1, Ordering::Relaxed);
    s.known.insert(
        source,
        Known {
            identity: id,
            key,
            addr: from,
        },
    );
    reply(s, source, from, &carga)
}

/// Monta um OK armado com a chave do par e endereçado a ele.
fn reply(s: &mut State, para: Address, a: SocketAddr, carga: &[u8]) -> Saidas {
    // A carga do OK já começa pelo verbo a que responde.
    send(s, para, a, Verb::Ok, carga)
}

/// Monta um pacote de `verbo` armado com a chave do par e endereçado a ele.
fn send(s: &mut State, para: Address, a: SocketAddr, verbo: Verb, carga: &[u8]) -> Saidas {
    let Some(par) = s.known.get(&para) else {
        return Vec::new();
    };
    s.next_id = s.next_id.wrapping_add(8);
    let mut p = Packet::new(s.next_id, para, s.identity.address(), verbo);
    p.append(carga);
    let mut bytes = p.into_bytes();
    debug_assert_eq!(bytes[idx::FLAGS] & FLAG_FRAGMENTED, 0);
    salsa::armor(&par.key.salsa, &mut bytes, true);
    vec![(a, bytes)]
}

/// Manda um HELLO válido de `intruso` ao nó em `alvo` e diz se ficou sem resposta por cinco
/// segundos. Serve para provar que o app é invisível a quem ele não conhece.
pub async fn hello_de_intruso<U: Underlay>(
    under: &U,
    intruso: Identity,
    alvo: Identity,
    onde: SocketAddr,
) -> bool {
    let Some(acordo) = intruso.agree(&alvo) else {
        return false;
    };
    let chave = PeerKey::from_agreement(&acordo);
    let mut p = Packet::new(0x42, alvo.address(), intruso.address(), Verb::Hello);
    p.append(&[11, 1, 4]);
    p.append(&7u16.to_be_bytes());
    p.append(&0u64.to_be_bytes());
    p.append(&intruso.to_wire());
    let mut fim = Vec::new();
    inet::write(&mut fim, Some(onde));
    p.append(&fim);
    p.append(&0u64.to_be_bytes());
    p.append(&0u64.to_be_bytes());
    let mut bytes = p.into_bytes();
    salsa::armor(&chave.salsa, &mut bytes, false);
    if under.try_send_to(&bytes, onde).is_err() {
        return false;
    }
    let mut buf = [0u8; 2048];
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        poll_fn(|cx| under.poll_recv_from(cx, &mut buf)),
    )
    .await
    .is_err()
}
