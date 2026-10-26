//! O meio Ethernet (M61): transforma um túnel que leva **quadros** Ethernet (o ZeroTier é uma
//! rede Ethernet virtual) num [`PacketTunnel`] de pacotes IP, que o [`NetStack`](crate::NetStack)
//! já sabe usar.
//!
//! O adaptador tira e põe o cabeçalho Ethernet, faz o ARP e escolhe o próximo salto pelas rotas
//! (a de máscara mais longa, com ou sem gateway). Em vez de ligar o `medium-ethernet` do
//! `smoltcp`, cujo cache de vizinhos e tabela de rotas têm tamanho fixo na compilação (8 e 2), o
//! cache de ARP aqui é um `HashMap` sem teto. A pilha, o filtro de saída e os outros túneis não
//! mudam.
//!
//! Para a rede o app continua invisível: um pedido ARP só é respondido quando vem de um IP que
//! nós mesmos procuramos (que está no cache ou esperando resposta). Quem nunca recebeu nada do
//! app não descobre o MAC dele.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::io;
use std::net::{IpAddr, Ipv4Addr};
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::time::{Instant, Sleep};

use crate::{Cidr, PacketTunnel, Unreachable};

/// O tamanho do cabeçalho Ethernet (destino, origem e tipo; sem FCS).
const ETH_HEADER: usize = 14;
/// O tipo de quadro do IPv4.
const ETHERTYPE_IPV4: u16 = 0x0800;
/// O tipo de quadro do ARP.
const ETHERTYPE_ARP: u16 = 0x0806;
/// O tamanho de um pacote ARP de Ethernet/IPv4 (RFC 826).
const ARP_LEN: usize = 28;
/// O MAC de broadcast.
const BROADCAST: [u8; 6] = [0xff; 6];

/// Quantos pedidos ARP saem antes de desistir de um vizinho (três, como o `mcast_solicit` do
/// Linux e o mínimo que o RFC 1122, seção 2.3.2.1, pede para as retransmissões).
const ARP_ATTEMPTS: u8 = 3;
/// O intervalo entre os pedidos ARP de um mesmo vizinho: 1 s, o `retrans_time` do Linux e o
/// intervalo máximo que o RFC 1122, seção 2.3.2.1, indica para as retransmissões.
const ARP_RETRY: Duration = Duration::from_secs(1);
/// Quanto uma entrada do cache vale: esquecida se ninguém a usa por 60 s e revalidada (pedido
/// unicast) se está em uso e ninguém a confirma por 60 s. O `base_reachable_time` do Linux é 30
/// s; o ZeroTier não define o tempo, e 60 s dá metade do tráfego de ARP.
const ARP_VALIDITY: Duration = Duration::from_secs(60);

/// Uma rota de um túnel de quadros: a rede e, se houver, o gateway (o próximo salto).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    /// A rede a que a rota se aplica.
    pub net: Cidr,
    /// O gateway. `None` quer dizer que a rede está no enlace (ARP direto para o destino).
    pub via: Option<Ipv4Addr>,
}

/// Um túnel que leva quadros Ethernet (14 bytes de cabeçalho, sem FCS). A forma é a do
/// [`PacketTunnel`].
pub trait FrameTunnel: Send + 'static {
    /// O MTU de IP (sem o cabeçalho Ethernet).
    fn mtu(&self) -> usize;

    /// O MAC desta ponta.
    fn mac(&self) -> [u8; 6];

    /// Os endereços desta ponta dentro da rede.
    fn addresses(&self) -> Vec<Cidr>;

    /// As rotas da rede, com ou sem gateway.
    fn routes(&self) -> Vec<Route>;

    /// Entrega um quadro que sai. Não bloqueia: se o túnel não pode levar agora, descarta.
    fn send_frame(&mut self, frame: &[u8]);

    /// Espera o próximo quadro que chega: limpa `frame` e o enche com ele. `Err` quer dizer que o
    /// túnel acabou.
    fn poll_recv_frame(
        &mut self,
        cx: &mut Context<'_>,
        frame: &mut Vec<u8>,
    ) -> Poll<io::Result<()>>;

    /// Avisa que as rotas mudaram: devolve a lista nova **inteira**. O padrão nunca avisa.
    fn poll_routes(&mut self, cx: &mut Context<'_>) -> Poll<Vec<Route>> {
        let _ = cx;
        Poll::Pending
    }

    /// Avisa que um endereço não pode ser alcançado. O padrão nunca avisa.
    fn poll_unreachable(&mut self, cx: &mut Context<'_>) -> Poll<Unreachable> {
        let _ = cx;
        Poll::Pending
    }

    /// Resolve um nome dentro da rede, quando o túnel sabe. O padrão é `None`.
    fn resolve(&mut self, name: &str) -> Option<Vec<IpAddr>> {
        let _ = name;
        None
    }
}

/// Um vizinho conhecido.
struct Neighbor {
    mac: [u8; 6],
    /// A última vez que o vizinho se confirmou (resposta ou pedido ARP dele).
    confirmed: Instant,
    /// A última vez que mandamos algo a ele.
    used: Instant,
}

/// A revalidação (pedidos unicast) de um vizinho em uso cuja confirmação venceu.
struct Probe {
    sent: u8,
    next: Instant,
}

/// Os pacotes que esperam o MAC de um próximo salto.
struct Search {
    queue: VecDeque<Vec<u8>>,
    sent: u8,
    next: Instant,
}

/// Transforma um [`FrameTunnel`] num [`PacketTunnel`] (o que o `NetStackBuilder::spawn` aceita).
pub struct EthernetTunnel<T: FrameTunnel> {
    inner: T,
    mac: [u8; 6],
    addresses: Vec<Cidr>,
    routes: Vec<Route>,
    cache: HashMap<Ipv4Addr, Neighbor>,
    probes: HashMap<Ipv4Addr, Probe>,
    // Sem teto de pacotes: a fila de um salto só cresce o que a janela do TCP deixa sair em 3 s,
    // e é descartada inteira se o vizinho não responde.
    searches: HashMap<Ipv4Addr, Search>,
    unreachable: VecDeque<Unreachable>,
    timer: Option<Pin<Box<Sleep>>>,
    last_purge: Option<Instant>,
    frame: Vec<u8>,
    out: Vec<u8>,
    #[cfg(feature = "test-util")]
    answer_all: bool,
}

impl<T: FrameTunnel> EthernetTunnel<T> {
    /// Envolve o túnel de quadros.
    pub fn new(tunnel: T) -> Self {
        Self {
            mac: tunnel.mac(),
            addresses: tunnel.addresses(),
            routes: tunnel.routes(),
            inner: tunnel,
            cache: HashMap::new(),
            probes: HashMap::new(),
            searches: HashMap::new(),
            unreachable: VecDeque::new(),
            timer: None,
            last_purge: None,
            frame: Vec::new(),
            out: Vec::new(),
            #[cfg(feature = "test-util")]
            answer_all: false,
        }
    }

    /// Com `sim`, responde ao ARP de qualquer um (e aprende o MAC dele). Só os testes, para o
    /// lado que escuta: o app de verdade não faz isso.
    #[cfg(feature = "test-util")]
    pub fn answer_all_arp(mut self, sim: bool) -> Self {
        self.answer_all = sim;
        self
    }

    /// O próximo salto para `dst`: a rota de máscara mais longa (a primeira que aparece, no
    /// empate); sem `via`, o próprio destino. Sem rota, mas na rede de um endereço nosso, o
    /// próprio destino.
    fn next_hop(&self, dst: Ipv4Addr) -> Option<Ipv4Addr> {
        let mut best: Option<&Route> = None;
        for route in &self.routes {
            if route.net.contains(IpAddr::V4(dst))
                && best.is_none_or(|b| route.net.prefix() > b.net.prefix())
            {
                best = Some(route);
            }
        }
        match best {
            Some(route) => Some(route.via.unwrap_or(dst)),
            None => self
                .addresses
                .iter()
                .any(|a| a.contains(IpAddr::V4(dst)))
                .then_some(dst),
        }
    }

    /// O nosso IP nos pedidos ARP para `hop`: o da rede dele, ou o primeiro IPv4.
    fn source_for(&self, hop: Ipv4Addr) -> Ipv4Addr {
        let v4 = |a: &Cidr| match a.addr() {
            IpAddr::V4(ip) => Some(ip),
            IpAddr::V6(_) => None,
        };
        self.addresses
            .iter()
            .find(|a| a.contains(IpAddr::V4(hop)))
            .and_then(v4)
            .or_else(|| self.addresses.iter().find_map(v4))
            .unwrap_or(Ipv4Addr::UNSPECIFIED)
    }

    fn is_ours(&self, ip: Ipv4Addr) -> bool {
        self.addresses.iter().any(|a| a.addr() == IpAddr::V4(ip))
    }

    /// `ip` é um vizinho que nós procuramos (está no cache ou esperando resposta)?
    fn searched(&self, ip: Ipv4Addr) -> bool {
        self.cache.contains_key(&ip) || self.searches.contains_key(&ip)
    }

    fn emit_ip(&mut self, dst_mac: [u8; 6], packet: &[u8]) {
        self.out.clear();
        self.out.extend_from_slice(&dst_mac);
        self.out.extend_from_slice(&self.mac);
        self.out.extend_from_slice(&ETHERTYPE_IPV4.to_be_bytes());
        self.out.extend_from_slice(packet);
        self.inner.send_frame(&self.out);
    }

    fn emit_arp(&mut self, op: u16, dst_mac: [u8; 6], tha: [u8; 6], spa: Ipv4Addr, tpa: Ipv4Addr) {
        self.out.clear();
        self.out.extend_from_slice(&dst_mac);
        self.out.extend_from_slice(&self.mac);
        self.out.extend_from_slice(&ETHERTYPE_ARP.to_be_bytes());
        self.out.extend_from_slice(&[0, 1, 8, 0, 6, 4]);
        self.out.extend_from_slice(&op.to_be_bytes());
        self.out.extend_from_slice(&self.mac);
        self.out.extend_from_slice(&spa.octets());
        self.out.extend_from_slice(&tha);
        self.out.extend_from_slice(&tpa.octets());
        self.inner.send_frame(&self.out);
    }

    /// Um pedido ARP: em broadcast para um vizinho novo, unicast para revalidar um conhecido.
    fn request(&mut self, hop: Ipv4Addr, to: Option<[u8; 6]>) {
        let spa = self.source_for(hop);
        self.emit_arp(1, to.unwrap_or(BROADCAST), [0; 6], spa, hop);
    }

    /// Aprende (ou renova) o MAC de `ip` e solta a fila que esperava por ele.
    fn learn(&mut self, ip: Ipv4Addr, mac: [u8; 6], now: Instant) {
        let used = self.cache.get(&ip).map_or(now, |n| n.used);
        self.cache.insert(
            ip,
            Neighbor {
                mac,
                confirmed: now,
                used,
            },
        );
        self.probes.remove(&ip);
        if let Some(search) = self.searches.remove(&ip) {
            for packet in search.queue {
                self.emit_ip(mac, &packet);
            }
        }
    }

    /// Esquece o que ficou velho: vizinhos sem uso há mais de `ARP_VALIDITY`.
    fn purge(&mut self, now: Instant) {
        if self
            .last_purge
            .is_some_and(|at| now.saturating_duration_since(at) < ARP_VALIDITY)
        {
            return;
        }
        self.last_purge = Some(now);
        self.cache
            .retain(|_, n| now.saturating_duration_since(n.used) < ARP_VALIDITY);
        let cache = &self.cache;
        self.probes.retain(|ip, _| cache.contains_key(ip));
    }

    /// Manda o pacote IP que a pilha montou.
    fn send_packet(&mut self, packet: &[u8]) {
        if packet.len() < 20 || packet[0] >> 4 != 4 {
            return;
        }
        let dst = Ipv4Addr::new(packet[16], packet[17], packet[18], packet[19]);
        let Some(hop) = self.next_hop(dst) else {
            return;
        };
        let now = Instant::now();
        self.purge(now);
        if let Some(neighbor) = self.cache.get_mut(&hop) {
            neighbor.used = now;
            let mac = neighbor.mac;
            let stale = now.saturating_duration_since(neighbor.confirmed) >= ARP_VALIDITY;
            if stale && !self.probes.contains_key(&hop) {
                self.probes.insert(
                    hop,
                    Probe {
                        sent: 1,
                        next: now + ARP_RETRY,
                    },
                );
                self.request(hop, Some(mac));
            }
            self.emit_ip(mac, packet);
            return;
        }
        if let Some(search) = self.searches.get_mut(&hop) {
            search.queue.push_back(packet.to_vec());
            return;
        }
        self.searches.insert(
            hop,
            Search {
                queue: VecDeque::from([packet.to_vec()]),
                sent: 1,
                next: now + ARP_RETRY,
            },
        );
        self.request(hop, None);
    }

    /// Trata o quadro que chegou (em `self.frame`). `true` se é um pacote IP para a pilha (a
    /// partir do byte 14 do quadro).
    fn on_frame(&mut self, now: Instant) -> bool {
        if self.frame.len() < ETH_HEADER {
            return false;
        }
        let ethertype = u16::from_be_bytes([self.frame[12], self.frame[13]]);
        match ethertype {
            // Só o que é para o nosso MAC: o broadcast de IP não deve vir, e se vier não é para nós.
            ETHERTYPE_IPV4 => self.frame[0..6] == self.mac && self.frame.len() > ETH_HEADER,
            ETHERTYPE_ARP => {
                self.on_arp(now);
                false
            }
            _ => false,
        }
    }

    fn on_arp(&mut self, now: Instant) {
        let Some(arp) = self.frame.get(ETH_HEADER..ETH_HEADER + ARP_LEN) else {
            return;
        };
        // Ethernet (1), IPv4 (0x0800), MAC de 6 bytes e IP de 4.
        if arp[..6] != [0, 1, 8, 0, 6, 4] {
            return;
        }
        let op = u16::from_be_bytes([arp[6], arp[7]]);
        let sha: [u8; 6] = arp[8..14].try_into().unwrap_or_default();
        let spa = Ipv4Addr::new(arp[14], arp[15], arp[16], arp[17]);
        let tpa = Ipv4Addr::new(arp[24], arp[25], arp[26], arp[27]);
        if sha == self.mac || spa.is_unspecified() {
            return;
        }
        match op {
            // Resposta: só vale de quem procuramos.
            2 => {
                if self.searched(spa) {
                    self.learn(spa, sha, now);
                }
            }
            // Pedido pelo nosso IP: só responde a quem procuramos antes.
            1 if self.is_ours(tpa) => {
                #[cfg(feature = "test-util")]
                let any = self.answer_all;
                #[cfg(not(feature = "test-util"))]
                let any = false;
                if any || self.searched(spa) {
                    self.learn(spa, sha, now);
                    self.emit_arp(2, sha, sha, tpa, spa);
                }
            }
            // ARP gratuito de um vizinho que já conhecemos: o MAC pode ter mudado.
            1 if spa == tpa && self.cache.contains_key(&spa) => self.learn(spa, sha, now),
            _ => {}
        }
    }

    /// Roda o que venceu: os pedidos ARP a repetir ou desistir, e as revalidações.
    fn tick(&mut self, now: Instant) {
        let due: Vec<Ipv4Addr> = self
            .searches
            .iter()
            .filter(|(_, s)| s.next <= now)
            .map(|(hop, _)| *hop)
            .collect();
        for hop in due {
            let Some(search) = self.searches.get_mut(&hop) else {
                continue;
            };
            if search.sent < ARP_ATTEMPTS {
                search.sent += 1;
                search.next = now + ARP_RETRY;
                self.request(hop, None);
                continue;
            }
            if let Some(search) = self.searches.remove(&hop) {
                let mut seen = Vec::new();
                for packet in &search.queue {
                    if let Some(b) = packet.get(16..20) {
                        let dst = Ipv4Addr::new(b[0], b[1], b[2], b[3]);
                        if !seen.contains(&dst) {
                            seen.push(dst);
                        }
                    }
                }
                for dst in seen {
                    self.unreachable.push_back(Unreachable {
                        addr: IpAddr::V4(dst),
                        reason: format!("{hop} não respondeu ao ARP"),
                    });
                }
            }
        }
        let due: Vec<Ipv4Addr> = self
            .probes
            .iter()
            .filter(|(_, p)| p.next <= now)
            .map(|(ip, _)| *ip)
            .collect();
        for ip in due {
            let Some(probe) = self.probes.get_mut(&ip) else {
                continue;
            };
            if probe.sent < ARP_ATTEMPTS {
                probe.sent += 1;
                probe.next = now + ARP_RETRY;
                let mac = self.cache.get(&ip).map(|n| n.mac);
                self.request(ip, mac);
            } else {
                // Não confirmou: o próximo pacote para ele volta a procurá-lo em broadcast.
                self.probes.remove(&ip);
                self.cache.remove(&ip);
            }
        }
    }

    /// O próximo instante em que `tick` tem o que fazer.
    fn next_deadline(&self) -> Option<Instant> {
        self.searches
            .values()
            .map(|s| s.next)
            .chain(self.probes.values().map(|p| p.next))
            .min()
    }
}

impl<T: FrameTunnel> PacketTunnel for EthernetTunnel<T> {
    fn mtu(&self) -> usize {
        self.inner.mtu()
    }

    fn addresses(&self) -> Vec<Cidr> {
        self.addresses.clone()
    }

    fn routes(&self) -> Vec<Cidr> {
        self.routes
            .iter()
            .map(|r| r.net)
            .chain(self.addresses.iter().copied())
            .collect()
    }

    fn send(&mut self, packet: &[u8]) {
        self.send_packet(packet);
    }

    fn poll_recv(&mut self, cx: &mut Context<'_>, buf: &mut Vec<u8>) -> Poll<io::Result<()>> {
        loop {
            match self.inner.poll_recv_frame(cx, &mut self.frame) {
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(())) => {
                    if self.on_frame(Instant::now()) {
                        buf.clear();
                        buf.extend_from_slice(&self.frame[ETH_HEADER..]);
                        return Poll::Ready(Ok(()));
                    }
                    continue;
                }
                Poll::Pending => {}
            }
            self.tick(Instant::now());
            let Some(at) = self.next_deadline() else {
                return Poll::Pending;
            };
            let timer = self
                .timer
                .get_or_insert_with(|| Box::pin(tokio::time::sleep_until(at)));
            timer.as_mut().reset(at);
            if timer.as_mut().poll(cx).is_pending() {
                return Poll::Pending;
            }
        }
    }

    fn poll_routes(&mut self, cx: &mut Context<'_>) -> Poll<Vec<Cidr>> {
        match self.inner.poll_routes(cx) {
            Poll::Ready(routes) => {
                self.routes = routes;
                Poll::Ready(PacketTunnel::routes(self))
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_unreachable(&mut self, cx: &mut Context<'_>) -> Poll<Unreachable> {
        if let Some(event) = self.unreachable.pop_front() {
            return Poll::Ready(event);
        }
        self.inner.poll_unreachable(cx)
    }

    fn resolve(&mut self, name: &str) -> Option<Vec<IpAddr>> {
        self.inner.resolve(name)
    }
}
