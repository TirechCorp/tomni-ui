//! Túneis em memória para os testes: ligam duas pilhas (ou uma pilha e um lado "cru") por canais,
//! com perda, atraso e nomes configuráveis. Só existe com a feature `test-util`.

use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::sync::mpsc;

use crate::{Cidr, PacketTunnel, Unreachable};

/// Os ajustes do canal em memória.
#[derive(Clone, Debug)]
pub struct PipeOptions {
    /// A chance (de 0 a 1) de cada pacote ser perdido, sorteada de forma determinística pela
    /// `seed`.
    pub loss: f64,
    /// O atraso de cada pacote até o outro lado.
    pub delay: Duration,
    /// A semente do sorteio das perdas.
    pub seed: u64,
    /// O MTU que o túnel anuncia.
    pub mtu: usize,
}

impl Default for PipeOptions {
    fn default() -> Self {
        PipeOptions {
            loss: 0.0,
            delay: Duration::ZERO,
            seed: 1,
            // O MTU do túnel Nebula padrão (1300 bytes, o `tun.mtu` de fábrica do Nebula).
            mtu: 1300,
        }
    }
}

/// O que se observou passar pelo canal.
#[derive(Debug, Default)]
pub struct PipeStats {
    max_len: AtomicUsize,
    sent: AtomicUsize,
    dropped: AtomicUsize,
}

impl PipeStats {
    /// O maior pacote visto saindo.
    pub fn max_len(&self) -> usize {
        self.max_len.load(Ordering::Relaxed)
    }

    /// Quantos pacotes saíram (inclusive os perdidos).
    pub fn sent(&self) -> usize {
        self.sent.load(Ordering::Relaxed)
    }

    /// Quantos pacotes foram perdidos pelo sorteio.
    pub fn dropped(&self) -> usize {
        self.dropped.load(Ordering::Relaxed)
    }
}

/// Um gerador determinístico (xorshift64*), para as perdas.
struct Dice(u64);

impl Dice {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        let value = self.0.wrapping_mul(0x2545_F491_4F6C_DD1D);
        (value >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Uma ponta do canal em memória, que serve de [`PacketTunnel`].
pub struct PipeTunnel {
    options: PipeOptions,
    addresses: Vec<Cidr>,
    routes: Vec<Cidr>,
    outgoing: mpsc::UnboundedSender<Vec<u8>>,
    incoming: mpsc::UnboundedReceiver<Vec<u8>>,
    dice: Arc<Mutex<Dice>>,
    stats: Arc<PipeStats>,
    names: HashMap<String, Vec<IpAddr>>,
    dead_ends: Vec<IpAddr>,
    unreachable_tx: mpsc::UnboundedSender<Unreachable>,
    unreachable_rx: mpsc::UnboundedReceiver<Unreachable>,
}

/// O lado "cru" de um canal: injeta e lê pacotes IP sem pilha, para os testes do filtro.
pub struct RawEnd {
    outgoing: mpsc::UnboundedSender<Vec<u8>>,
    incoming: mpsc::UnboundedReceiver<Vec<u8>>,
}

impl RawEnd {
    /// Manda um pacote IP para a pilha.
    pub fn send(&self, packet: Vec<u8>) {
        let _ = self.outgoing.send(packet);
    }

    /// Espera o próximo pacote que a pilha mandou.
    pub async fn recv(&mut self) -> Option<Vec<u8>> {
        self.incoming.recv().await
    }

    /// O próximo pacote que a pilha já mandou, se houver.
    pub fn try_recv(&mut self) -> Option<Vec<u8>> {
        self.incoming.try_recv().ok()
    }
}

fn build(
    local: &str,
    remote_net: &str,
    options: &PipeOptions,
    outgoing: mpsc::UnboundedSender<Vec<u8>>,
    incoming: mpsc::UnboundedReceiver<Vec<u8>>,
    dice: Arc<Mutex<Dice>>,
    stats: Arc<PipeStats>,
) -> PipeTunnel {
    let (unreachable_tx, unreachable_rx) = mpsc::unbounded_channel();
    PipeTunnel {
        options: options.clone(),
        addresses: vec![local.parse().expect("endereço de teste")],
        routes: vec![remote_net.parse().expect("rede de teste")],
        outgoing,
        incoming,
        dice,
        stats,
        names: HashMap::new(),
        dead_ends: Vec::new(),
        unreachable_tx,
        unreachable_rx,
    }
}

impl PipeTunnel {
    /// Duas pontas ligadas entre si: `a_addr` (por exemplo `"10.42.0.1/24"`) e `b_addr`. Cada uma
    /// alcança a rede da outra. Os dois sentidos dividem as mesmas estatísticas e o mesmo sorteio.
    pub fn pair(a_addr: &str, b_addr: &str, options: PipeOptions) -> (PipeTunnel, PipeTunnel) {
        let (a_to_b, b_in) = mpsc::unbounded_channel();
        let (b_to_a, a_in) = mpsc::unbounded_channel();
        let dice = Arc::new(Mutex::new(Dice(options.seed.max(1))));
        let stats = Arc::new(PipeStats::default());
        let a = build(
            a_addr,
            b_addr,
            &options,
            a_to_b,
            a_in,
            dice.clone(),
            stats.clone(),
        );
        let b = build(b_addr, a_addr, &options, b_to_a, b_in, dice, stats);
        (a, b)
    }

    /// Uma ponta ligada a um [`RawEnd`], para injetar pacotes à mão.
    pub fn raw(addr: &str, options: PipeOptions) -> (PipeTunnel, RawEnd) {
        let (raw_to_tunnel, tunnel_in) = mpsc::unbounded_channel();
        let (tunnel_to_raw, raw_in) = mpsc::unbounded_channel();
        let dice = Arc::new(Mutex::new(Dice(options.seed.max(1))));
        let stats = Arc::new(PipeStats::default());
        let tunnel = build(addr, addr, &options, tunnel_to_raw, tunnel_in, dice, stats);
        (
            tunnel,
            RawEnd {
                outgoing: raw_to_tunnel,
                incoming: raw_in,
            },
        )
    }

    /// As estatísticas do canal.
    pub fn stats(&self) -> Arc<PipeStats> {
        self.stats.clone()
    }

    /// Ensina o `resolve` deste túnel: `nome` passa a valer `ip`.
    pub fn with_name(mut self, name: &str, ip: Ipv4Addr) -> Self {
        self.names.insert(name.to_owned(), vec![IpAddr::V4(ip)]);
        self
    }

    /// Faz `ip` ser inalcançável: um pacote para ele não sai, e a pilha recebe o aviso `reason`.
    pub fn with_dead_end(mut self, ip: Ipv4Addr) -> Self {
        self.dead_ends.push(IpAddr::V4(ip));
        self
    }
}

impl PacketTunnel for PipeTunnel {
    fn mtu(&self) -> usize {
        self.options.mtu
    }

    fn addresses(&self) -> Vec<Cidr> {
        self.addresses.clone()
    }

    fn routes(&self) -> Vec<Cidr> {
        self.routes.clone()
    }

    fn send(&mut self, packet: &[u8]) {
        self.stats.sent.fetch_add(1, Ordering::Relaxed);
        self.stats
            .max_len
            .fetch_max(packet.len(), Ordering::Relaxed);
        if packet.len() >= 20 {
            let dst = IpAddr::V4(Ipv4Addr::new(
                packet[16], packet[17], packet[18], packet[19],
            ));
            if self.dead_ends.contains(&dst) {
                let _ = self.unreachable_tx.send(Unreachable {
                    addr: dst,
                    reason: "o par não respondeu ao handshake".to_owned(),
                });
                return;
            }
        }
        let lost = self.options.loss > 0.0
            && self
                .dice
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .next()
                < self.options.loss;
        if lost {
            self.stats.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let packet = packet.to_vec();
        if self.options.delay.is_zero() {
            let _ = self.outgoing.send(packet);
        } else {
            let (to, delay) = (self.outgoing.clone(), self.options.delay);
            tokio::spawn(async move {
                tokio::time::sleep(delay).await;
                let _ = to.send(packet);
            });
        }
    }

    fn poll_recv(&mut self, cx: &mut Context<'_>, buf: &mut Vec<u8>) -> Poll<io::Result<()>> {
        match self.incoming.poll_recv(cx) {
            Poll::Ready(Some(packet)) => {
                buf.clear();
                buf.extend_from_slice(&packet);
                Poll::Ready(Ok(()))
            }
            Poll::Ready(None) => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "o outro lado do canal em memória acabou",
            ))),
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_unreachable(&mut self, cx: &mut Context<'_>) -> Poll<Unreachable> {
        match self.unreachable_rx.poll_recv(cx) {
            Poll::Ready(Some(event)) => Poll::Ready(event),
            _ => Poll::Pending,
        }
    }

    fn resolve(&mut self, name: &str) -> Option<Vec<IpAddr>> {
        self.names.get(name).cloned()
    }
}
