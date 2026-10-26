//! O underlay: o UDP por onde os pacotes do Nebula viajam.
//!
//! O motor fala com um [`Underlay`], e não direto com o `tokio::net::UdpSocket`, para os testes
//! rodarem numa rede simulada (`sim`, com perda, atraso e os três tipos de NAT) sem abrir porta
//! nenhuma. Em produção o único underlay é o [`UdpUnderlay`].

use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::task::{Context, Poll};

use tokio::io::ReadBuf;
use tokio::net::UdpSocket;

/// A camada de baixo do motor: datagramas UDP IPv4.
///
/// O underlay é só IPv4: a pilha embutida não tem IPv6 e os endereços IPv6 dos pares são
/// ignorados.
pub trait Underlay: Send + Sync + 'static {
    /// O endereço local do socket (a porta é a que o Nebula anuncia).
    fn local_addr(&self) -> io::Result<SocketAddr>;

    /// Manda um datagrama sem esperar. Se o sistema não aceita agora (`WouldBlock`), o
    /// datagrama se perde, como em qualquer UDP: o protocolo retransmite o que importa.
    fn try_send_to(&self, buf: &[u8], to: SocketAddr) -> io::Result<()>;

    /// Espera o próximo datagrama e devolve seu tamanho e quem mandou.
    fn poll_recv_from(
        &self,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<(usize, SocketAddr)>>;

    /// O endereço IP local que o sistema usaria para chegar a `remote` (o do "socket
    /// conectado"; nada é enviado). É o endereço que o nó anuncia aos lighthouses no lugar de
    /// listar as placas de rede.
    fn local_ip_toward(&self, remote: SocketAddr) -> Option<IpAddr>;
}

/// O underlay de verdade: um `UdpSocket` IPv4.
pub struct UdpUnderlay {
    socket: UdpSocket,
}

impl UdpUnderlay {
    /// Abre o socket em `host:porta` (`0.0.0.0` e `::` valem como "todas as placas"). Se a
    /// porta está ocupada, usa outra e devolve o aviso em português.
    pub async fn bind(host: &str, port: u16) -> io::Result<(UdpUnderlay, Option<String>)> {
        let ip = match host.trim() {
            "" | "::" | "[::]" | "0.0.0.0" => Ipv4Addr::UNSPECIFIED,
            outro => outro.parse::<Ipv4Addr>().map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("listen.host \"{outro}\" não é um endereço IPv4"),
                )
            })?,
        };
        match UdpSocket::bind(SocketAddr::new(IpAddr::V4(ip), port)).await {
            Ok(socket) => Ok((UdpUnderlay { socket }, None)),
            Err(erro) if erro.kind() == io::ErrorKind::AddrInUse && port != 0 => {
                let socket = UdpSocket::bind(SocketAddr::new(IpAddr::V4(ip), 0)).await?;
                let nova = socket.local_addr()?.port();
                Ok((
                    UdpUnderlay { socket },
                    Some(format!(
                        "a porta UDP {port} está ocupada; usando a porta {nova}"
                    )),
                ))
            }
            Err(erro) => Err(io::Error::new(
                erro.kind(),
                format!("não foi possível abrir a porta UDP {port}: {erro}"),
            )),
        }
    }
}

impl Underlay for UdpUnderlay {
    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    fn try_send_to(&self, buf: &[u8], to: SocketAddr) -> io::Result<()> {
        match self.socket.try_send_to(buf, to) {
            Ok(_) => Ok(()),
            Err(erro) if erro.kind() == io::ErrorKind::WouldBlock => Ok(()),
            Err(erro) => Err(erro),
        }
    }

    fn poll_recv_from(
        &self,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<(usize, SocketAddr)>> {
        let mut leitura = ReadBuf::new(buf);
        match self.socket.poll_recv_from(cx, &mut leitura) {
            Poll::Ready(Ok(origem)) => Poll::Ready(Ok((leitura.filled().len(), origem))),
            Poll::Ready(Err(erro)) => Poll::Ready(Err(erro)),
            Poll::Pending => Poll::Pending,
        }
    }

    fn local_ip_toward(&self, remote: SocketAddr) -> Option<IpAddr> {
        // Um socket UDP "conectado" só escolhe a rota: nenhum pacote sai.
        let teste = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
        teste.connect(remote).ok()?;
        let ip = teste.local_addr().ok()?.ip();
        (!ip.is_unspecified()).then_some(ip)
    }
}

#[cfg(feature = "test-util")]
pub use sim::{NatKind, SimEndpoint, SimNat, SimNet};

/// A rede simulada dos testes: hosts públicos, NATs de três tipos, perda e atraso.
#[cfg(feature = "test-util")]
mod sim {
    use std::collections::{HashMap, HashSet};
    use std::io;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::sync::{Arc, Mutex, PoisonError};
    use std::task::{Context, Poll};
    use std::time::Duration;

    use tokio::sync::mpsc;

    use super::Underlay;

    type Datagram = (Vec<u8>, SocketAddr);

    /// Como o NAT trata o que entra.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum NatKind {
        /// Qualquer um pode escrever na porta aberta (a mais permissiva).
        FullCone,
        /// Só entra o que vem de um endereço e porta para os quais o nó já escreveu; a porta
        /// externa é a mesma para todos os destinos.
        Restricted,
        /// Uma porta externa nova para cada destino, e só ele entra (o que o furo de NAT não
        /// consegue passar sem ajuda).
        Symmetric,
    }

    struct Mapping {
        private: SocketAddr,
        open_to_all: bool,
        allowed: HashSet<SocketAddr>,
    }

    struct Nat {
        kind: NatKind,
        lan: HashMap<SocketAddr, mpsc::UnboundedSender<Datagram>>,
        /// (privado, destino) → porta externa. Fora do simétrico o destino é `None`.
        out: HashMap<(SocketAddr, Option<SocketAddr>), u16>,
        inbound: HashMap<u16, Mapping>,
        next_port: u16,
    }

    struct Inner {
        publics: HashMap<SocketAddr, mpsc::UnboundedSender<Datagram>>,
        nats: HashMap<Ipv4Addr, Nat>,
        loss_percent: u64,
        delay: Duration,
        rng: u64,
        /// Quantos datagramas foram entregues ou perdidos, para os testes.
        delivered: u64,
        lost: u64,
    }

    impl Inner {
        fn random(&mut self) -> u64 {
            // xorshift64: determinístico, para os testes não variarem.
            let mut x = self.rng;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.rng = x;
            x
        }
    }

    /// A rede simulada. Clonar dá outra alça para a mesma rede.
    #[derive(Clone)]
    pub struct SimNet {
        inner: Arc<Mutex<Inner>>,
    }

    impl Default for SimNet {
        fn default() -> Self {
            SimNet::new()
        }
    }

    impl SimNet {
        /// Uma rede vazia, sem perda e sem atraso.
        pub fn new() -> SimNet {
            SimNet {
                inner: Arc::new(Mutex::new(Inner {
                    publics: HashMap::new(),
                    nats: HashMap::new(),
                    loss_percent: 0,
                    delay: Duration::ZERO,
                    rng: 0x9e37_79b9_7f4a_7c15,
                    delivered: 0,
                    lost: 0,
                })),
            }
        }

        fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
            self.inner.lock().unwrap_or_else(PoisonError::into_inner)
        }

        /// Perde esta porcentagem dos datagramas (0 a 100).
        pub fn set_loss(&self, percent: u64) {
            self.lock().loss_percent = percent.min(100);
        }

        /// Atrasa cada datagrama por este tempo.
        pub fn set_delay(&self, delay: Duration) {
            self.lock().delay = delay;
        }

        /// Quantos datagramas a rede entregou até agora.
        pub fn delivered(&self) -> u64 {
            self.lock().delivered
        }

        /// Um host com endereço público (sem NAT).
        pub fn host(&self, ip: &str, port: u16) -> SimEndpoint {
            let addr = SocketAddr::new(ip.parse().expect("IP de teste válido"), port);
            let (tx, rx) = mpsc::unbounded_channel();
            self.lock().publics.insert(addr, tx.clone());
            SimEndpoint {
                net: self.clone(),
                addr,
                nat: None,
                tx,
                rx: Mutex::new(rx),
            }
        }

        /// Um NAT com este endereço público.
        pub fn nat(&self, public_ip: &str, kind: NatKind) -> SimNat {
            let ip: Ipv4Addr = public_ip.parse().expect("IP de teste válido");
            self.lock().nats.insert(
                ip,
                Nat {
                    kind,
                    lan: HashMap::new(),
                    out: HashMap::new(),
                    inbound: HashMap::new(),
                    next_port: 40_000,
                },
            );
            SimNat {
                net: self.clone(),
                ip,
            }
        }

        fn deliver(&self, from: &SimEndpoint, data: &[u8], to: SocketAddr) {
            let (destino, origem, atraso) = {
                let mut dentro = self.lock();
                let sorte = dentro.random() % 100;
                if sorte < dentro.loss_percent {
                    dentro.lost += 1;
                    return;
                }
                let atraso = dentro.delay;
                let Some((origem, destino)) = Self::route(&mut dentro, from, to) else {
                    dentro.lost += 1;
                    return;
                };
                dentro.delivered += 1;
                (destino, origem, atraso)
            };
            let datagrama = (data.to_vec(), origem);
            if atraso.is_zero() {
                let _ = destino.send(datagrama);
            } else {
                tokio::spawn(async move {
                    tokio::time::sleep(atraso).await;
                    let _ = destino.send(datagrama);
                });
            }
        }

        /// Quem recebe e com que endereço de origem; `None` se o datagrama não chega.
        fn route(
            dentro: &mut Inner,
            from: &SimEndpoint,
            to: SocketAddr,
        ) -> Option<(SocketAddr, mpsc::UnboundedSender<Datagram>)> {
            let src = match from.nat {
                None => from.addr,
                Some(nat_ip) => {
                    // Na própria rede local o endereço é o privado.
                    let nat = dentro.nats.get_mut(&nat_ip)?;
                    if let Some(destino) = nat.lan.get(&to) {
                        return Some((from.addr, destino.clone()));
                    }
                    let chave = match nat.kind {
                        NatKind::Symmetric => (from.addr, Some(to)),
                        _ => (from.addr, None),
                    };
                    let porta = match nat.out.get(&chave) {
                        Some(porta) => *porta,
                        None => {
                            let porta = nat.next_port;
                            nat.next_port += 1;
                            nat.out.insert(chave, porta);
                            nat.inbound.insert(
                                porta,
                                Mapping {
                                    private: from.addr,
                                    open_to_all: nat.kind == NatKind::FullCone,
                                    allowed: HashSet::new(),
                                },
                            );
                            porta
                        }
                    };
                    if let Some(mapa) = nat.inbound.get_mut(&porta) {
                        mapa.allowed.insert(to);
                    }
                    SocketAddr::new(IpAddr::V4(nat_ip), porta)
                }
            };
            if let Some(destino) = dentro.publics.get(&to) {
                return Some((src, destino.clone()));
            }
            let IpAddr::V4(ip_destino) = to.ip() else {
                return None;
            };
            let nat = dentro.nats.get(&ip_destino)?;
            let mapa = nat.inbound.get(&to.port())?;
            if !mapa.open_to_all && !mapa.allowed.contains(&src) {
                return None;
            }
            Some((src, nat.lan.get(&mapa.private)?.clone()))
        }
    }

    /// Um NAT da rede simulada.
    pub struct SimNat {
        net: SimNet,
        ip: Ipv4Addr,
    }

    impl SimNat {
        /// Um nó atrás do NAT, com endereço privado.
        pub fn endpoint(&self, private_ip: &str, port: u16) -> SimEndpoint {
            let addr = SocketAddr::new(private_ip.parse().expect("IP de teste válido"), port);
            let (tx, rx) = mpsc::unbounded_channel();
            if let Some(nat) = self.net.lock().nats.get_mut(&self.ip) {
                nat.lan.insert(addr, tx.clone());
            }
            SimEndpoint {
                net: self.net.clone(),
                addr,
                nat: Some(self.ip),
                tx,
                rx: Mutex::new(rx),
            }
        }

        /// Esquece todos os mapeamentos: os nós passam a sair por portas novas (o que acontece
        /// quando o roteador reinicia ou o telefone troca de rede).
        pub fn rebind(&self) {
            if let Some(nat) = self.net.lock().nats.get_mut(&self.ip) {
                nat.out.clear();
                nat.inbound.clear();
                nat.next_port += 1000;
            }
        }
    }

    /// Uma ponta da rede simulada: o que o motor usa no lugar do socket.
    pub struct SimEndpoint {
        net: SimNet,
        addr: SocketAddr,
        nat: Option<Ipv4Addr>,
        /// A entrada deste endereço no mapa da rede: quem fecha só a retira se ainda for a dele.
        tx: mpsc::UnboundedSender<Datagram>,
        rx: Mutex<mpsc::UnboundedReceiver<Datagram>>,
    }

    impl Drop for SimEndpoint {
        fn drop(&mut self) {
            let mut dentro = self.net.lock();
            match self.nat {
                // Um nó que cai e volta no mesmo endereço cria a ponta nova antes de a antiga
                // acabar de fechar (o motor a larga só ao terminar): a antiga não pode levar
                // embora a entrada da nova.
                None => {
                    if dentro
                        .publics
                        .get(&self.addr)
                        .is_some_and(|atual| atual.same_channel(&self.tx))
                    {
                        dentro.publics.remove(&self.addr);
                    }
                }
                Some(ip) => {
                    if let Some(nat) = dentro.nats.get_mut(&ip)
                        && nat
                            .lan
                            .get(&self.addr)
                            .is_some_and(|atual| atual.same_channel(&self.tx))
                    {
                        nat.lan.remove(&self.addr);
                    }
                }
            }
        }
    }

    impl Underlay for SimEndpoint {
        fn local_addr(&self) -> io::Result<SocketAddr> {
            Ok(self.addr)
        }

        fn try_send_to(&self, buf: &[u8], to: SocketAddr) -> io::Result<()> {
            self.net.deliver(self, buf, to);
            Ok(())
        }

        fn poll_recv_from(
            &self,
            cx: &mut Context<'_>,
            buf: &mut [u8],
        ) -> Poll<io::Result<(usize, SocketAddr)>> {
            let mut rx = self.rx.lock().unwrap_or_else(PoisonError::into_inner);
            match rx.poll_recv(cx) {
                Poll::Ready(Some((dados, origem))) => {
                    let n = dados.len().min(buf.len());
                    buf[..n].copy_from_slice(&dados[..n]);
                    Poll::Ready(Ok((n, origem)))
                }
                Poll::Ready(None) => Poll::Pending,
                Poll::Pending => Poll::Pending,
            }
        }

        fn local_ip_toward(&self, _remote: SocketAddr) -> Option<IpAddr> {
            Some(self.addr.ip())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::future::poll_fn;

        async fn recebe(ponta: &SimEndpoint) -> (Vec<u8>, SocketAddr) {
            let mut buf = [0u8; 64];
            let (n, de) = poll_fn(|cx| ponta.poll_recv_from(cx, &mut buf))
                .await
                .unwrap();
            (buf[..n].to_vec(), de)
        }

        async fn nada_chega(ponta: &SimEndpoint) -> bool {
            tokio::time::timeout(Duration::from_millis(50), recebe(ponta))
                .await
                .is_err()
        }

        #[tokio::test(start_paused = true)]
        async fn hosts_publicos_se_falam() {
            let rede = SimNet::new();
            let a = rede.host("203.0.113.1", 4242);
            let b = rede.host("203.0.113.2", 4242);
            a.try_send_to(b"oi", b.addr).unwrap();
            assert_eq!(recebe(&b).await, (b"oi".to_vec(), a.addr));
        }

        #[tokio::test(start_paused = true)]
        async fn cone_completo_aceita_de_qualquer_um() {
            let rede = SimNet::new();
            let nat = rede.nat("198.51.100.1", NatKind::FullCone);
            let dentro = nat.endpoint("192.168.1.10", 4242);
            let servidor = rede.host("203.0.113.1", 4242);
            let outro = rede.host("203.0.113.9", 1);
            dentro.try_send_to(b"a", servidor.addr).unwrap();
            let (_, visto) = recebe(&servidor).await;
            assert_eq!(visto.ip(), IpAddr::V4("198.51.100.1".parse().unwrap()));
            outro.try_send_to(b"b", visto).unwrap();
            assert_eq!(recebe(&dentro).await.0, b"b");
        }

        #[tokio::test(start_paused = true)]
        async fn cone_restrito_so_aceita_de_quem_recebeu() {
            let rede = SimNet::new();
            let nat = rede.nat("198.51.100.1", NatKind::Restricted);
            let dentro = nat.endpoint("192.168.1.10", 4242);
            let servidor = rede.host("203.0.113.1", 4242);
            let outro = rede.host("203.0.113.9", 1);
            dentro.try_send_to(b"a", servidor.addr).unwrap();
            let (_, visto) = recebe(&servidor).await;
            outro.try_send_to(b"b", visto).unwrap();
            assert!(nada_chega(&dentro).await);
            servidor.try_send_to(b"c", visto).unwrap();
            assert_eq!(recebe(&dentro).await.0, b"c");
        }

        #[tokio::test(start_paused = true)]
        async fn nat_simetrico_usa_uma_porta_por_destino() {
            let rede = SimNet::new();
            let nat = rede.nat("198.51.100.1", NatKind::Symmetric);
            let dentro = nat.endpoint("192.168.1.10", 4242);
            let a = rede.host("203.0.113.1", 4242);
            let b = rede.host("203.0.113.2", 4242);
            dentro.try_send_to(b"1", a.addr).unwrap();
            dentro.try_send_to(b"2", b.addr).unwrap();
            let (_, de_a) = recebe(&a).await;
            let (_, de_b) = recebe(&b).await;
            assert_ne!(de_a.port(), de_b.port());
            // O endereço que `a` viu não aceita `b`.
            b.try_send_to(b"x", de_a).unwrap();
            assert!(nada_chega(&dentro).await);
        }

        #[tokio::test(start_paused = true)]
        async fn rebind_troca_a_porta_externa() {
            let rede = SimNet::new();
            let nat = rede.nat("198.51.100.1", NatKind::FullCone);
            let dentro = nat.endpoint("192.168.1.10", 4242);
            let servidor = rede.host("203.0.113.1", 4242);
            dentro.try_send_to(b"1", servidor.addr).unwrap();
            let (_, antes) = recebe(&servidor).await;
            nat.rebind();
            dentro.try_send_to(b"2", servidor.addr).unwrap();
            let (_, depois) = recebe(&servidor).await;
            assert_ne!(antes, depois);
        }

        #[tokio::test(start_paused = true)]
        async fn vizinhos_da_mesma_rede_local_se_falam_pelo_endereco_privado() {
            let rede = SimNet::new();
            let nat = rede.nat("198.51.100.1", NatKind::Symmetric);
            let a = nat.endpoint("192.168.1.10", 4242);
            let b = nat.endpoint("192.168.1.11", 4242);
            a.try_send_to(b"oi", b.addr).unwrap();
            assert_eq!(recebe(&b).await, (b"oi".to_vec(), a.addr));
        }

        #[tokio::test(start_paused = true)]
        async fn perda_e_atraso() {
            let rede = SimNet::new();
            let a = rede.host("203.0.113.1", 1);
            let b = rede.host("203.0.113.2", 1);
            rede.set_delay(Duration::from_millis(30));
            let antes = tokio::time::Instant::now();
            a.try_send_to(b"x", b.addr).unwrap();
            recebe(&b).await;
            assert!(antes.elapsed() >= Duration::from_millis(30));
            rede.set_delay(Duration::ZERO);
            rede.set_loss(100);
            a.try_send_to(b"y", b.addr).unwrap();
            assert!(nada_chega(&b).await);
        }
    }
}
