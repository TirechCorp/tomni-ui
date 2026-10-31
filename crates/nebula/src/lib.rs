//! A rede Nebula embutida do Tomni UI (M48): o app entra numa rede Nebula **sem placa virtual,
//! sem mexer nas rotas do sistema e sem administrador**, e só o tráfego dele passa por ela.
//!
//! O [Nebula](https://github.com/slackhq/nebula) (da Slack, licença MIT) é uma rede em malha
//! cifrada: cada nó tem um certificado assinado por uma CA, os nós se acham por um ou mais
//! lighthouses e conversam direto, furando o NAT. Aqui o protocolo é reimplementado em Rust (os
//! certificados v1 e v2, o aperto de mão Noise IX, o túnel, o lighthouse, o punch e os relays),
//! compatível com o `nebula` oficial, e fala com a pilha TCP/IP em software de
//! [`tomni_ui_netstack`].
//!
//! Quando o punch não passa (os dois nós atrás de CGNAT, comum nas operadoras), o tráfego vai por
//! um relay: o nó lista em `relay.relays` os nós com `relay.am_relay` pelos quais aceita ser
//! alcançado, avisa o lighthouse, e quem quer chegar a ele pede o relay e manda o aperto de mão
//! por dentro. O túnel continua cifrado de ponta a ponta; o relay só passa adiante. O app é só
//! cliente (`relay.use_relays`, ligado por padrão): não faz papel de relay.
//!
//! A [`Nebula`] devolve um [`Proxy`] SOCKS5 que só os clientes do próprio
//! app usam (SSH, SFTP, Telnet, FTP, S3, rclone): a porta fica em `127.0.0.1`, com porta e
//! credencial aleatórias, e não há como mostrá-la nem expô-la a outros programas.
//!
//! ```no_run
//! use tomni_ui_nebula::{Config, NebulaBuilder};
//! use std::time::Duration;
//!
//! async fn exemplo() -> Result<(), Box<dyn std::error::Error>> {
//!     let texto = std::fs::read_to_string("config.yml")?;
//!     let (config, _avisos) = Config::from_yaml(&texto, Some("pasta".as_ref()))?;
//!     let rede = NebulaBuilder::new(config).start().await?;
//!     rede.wait_connected(Duration::from_secs(20)).await?;
//!     // Os clientes do app usam este proxy; o resto do computador não o enxerga.
//!     let _proxy = rede.proxy();
//!     rede.stop().await;
//!     Ok(())
//! }
//! ```

mod cert;
mod config;
mod control;
mod der;
mod engine;
mod handshake;
mod header;
mod meta;
mod noise;
mod pem;
mod proto;
mod remotes;
mod replay;
#[cfg(test)]
mod testutil;
mod timers;
mod tunnel;
mod underlay;
mod util;

use std::fmt;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tomni_ui_net::Proxy;
use tomni_ui_netstack::{Cidr, NetStack, PacketTunnel, SocksBridge, Unreachable};

pub use cert::{CaPool, CertError, Certificate, Curve, PrivateKey, Version};
pub use config::{
    Cipher, Config, ConfigError, Credentials, Handshakes, LighthouseConfig, Listen, Pki, Problem,
    Punchy, RelayConfig, Severity, StaticMapNetwork, UnsafeRoute, Warning, parse_duration,
};
pub use engine::{State, TunnelInfo};
#[cfg(feature = "test-util")]
pub use underlay::{NatKind, SimEndpoint, SimNat, SimNet};
pub use underlay::{UdpUnderlay, Underlay};

use engine::{Command, Handles, Params};

/// O túnel de pacotes IP entre a pilha TCP e o motor.
struct NebulaTunnel {
    mtu: usize,
    addresses: Vec<Cidr>,
    routes: Vec<Cidr>,
    to_engine: mpsc::Sender<Vec<u8>>,
    from_engine: mpsc::Receiver<Vec<u8>>,
    unreachable: mpsc::UnboundedReceiver<Unreachable>,
}

impl PacketTunnel for NebulaTunnel {
    fn mtu(&self) -> usize {
        self.mtu
    }

    fn addresses(&self) -> Vec<Cidr> {
        self.addresses.clone()
    }

    fn routes(&self) -> Vec<Cidr> {
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
                "a rede Nebula parou",
            ))),
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_unreachable(&mut self, cx: &mut Context<'_>) -> Poll<Unreachable> {
        match self.unreachable.poll_recv(cx) {
            Poll::Ready(Some(aviso)) => Poll::Ready(aviso),
            // Sem motor não há mais avisos; o `poll_recv` acima já acaba a pilha.
            Poll::Ready(None) | Poll::Pending => Poll::Pending,
        }
    }
}

/// Monta e inicia uma [`Nebula`].
pub struct NebulaBuilder {
    config: Config,
    notifier: Option<Arc<dyn Fn(State) + Send + Sync>>,
    am_lighthouse: bool,
    am_relay: bool,
}

impl NebulaBuilder {
    /// Começa com o `config` já lido (veja [`Config::from_yaml`]).
    pub fn new(config: Config) -> NebulaBuilder {
        NebulaBuilder {
            config,
            notifier: None,
            am_lighthouse: false,
            am_relay: false,
        }
    }

    /// Chama `f` a cada mudança de [`State`], de qualquer thread. Não bloqueie dentro dela.
    #[must_use]
    pub fn on_state(mut self, f: impl Fn(State) + Send + Sync + 'static) -> NebulaBuilder {
        self.notifier = Some(Arc::new(f));
        self
    }

    /// Faz o nó atuar como lighthouse (guarda e conta os endereços dos outros). O app é só
    /// cliente; isto existe para os testes do motor.
    #[cfg(feature = "test-util")]
    #[must_use]
    pub fn as_lighthouse(mut self) -> NebulaBuilder {
        self.am_lighthouse = true;
        self
    }

    /// Faz o nó passar os pacotes de um par a outro (`relay.am_relay`), para os nós que não se
    /// alcançam direto. O app é só cliente (usa os relays da rede); isto existe para os testes
    /// do motor.
    #[cfg(feature = "test-util")]
    #[must_use]
    pub fn as_relay(mut self) -> NebulaBuilder {
        self.am_relay = true;
        self
    }

    /// Abre o UDP (em `listen.host` e `listen.port`) e inicia a rede, no runtime atual.
    ///
    /// # Errors
    ///
    /// Se o config tem erro (certificado vencido, chave que não casa, CA desconhecida…) ou o
    /// UDP não abre. A mensagem é em português.
    pub async fn start(self) -> io::Result<Nebula> {
        let (under, aviso) =
            UdpUnderlay::bind(&self.config.listen.host, self.config.listen.port).await?;
        let mut nebula = self.start_on(under).await?;
        nebula.warnings.extend(aviso);
        Ok(nebula)
    }

    /// Como [`start`](NebulaBuilder::start), mas sobre um UDP dado (os testes usam a rede
    /// simulada).
    ///
    /// # Errors
    ///
    /// Os mesmos de [`start`](NebulaBuilder::start).
    pub async fn start_on<U: Underlay>(self, under: U) -> io::Result<Nebula> {
        let agora = i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
        )
        .unwrap_or(i64::MAX);
        // Um lighthouse (só nos testes) não procura ninguém, então não precisa de endereços.
        let erros: Vec<String> = self
            .config
            .check(agora)
            .into_iter()
            .filter(|p| p.severity == Severity::Error && !self.am_lighthouse)
            .map(|p| p.message)
            .collect();
        if !erros.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                erros.join("; "),
            ));
        }
        let creds = self
            .config
            .credentials()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;
        let mut addresses = Vec::new();
        for cert in [&creds.v1, &creds.v2].into_iter().flatten() {
            for rede in cert.networks() {
                let rede = Cidr::new(rede.addr(), rede.prefix()).unwrap_or(*rede);
                if rede.addr().is_ipv4() && !addresses.contains(&rede) {
                    addresses.push(rede);
                }
            }
        }
        let mut routes: Vec<Cidr> = addresses
            .iter()
            .filter_map(|c| Cidr::new(c.network(), c.prefix()))
            .collect();
        for rota in &self.config.unsafe_routes {
            if rota.route.addr().is_ipv4() && !routes.contains(&rota.route) {
                routes.push(rota.route);
            }
        }
        let mtu = usize::from(self.config.mtu);
        let Handles {
            commands,
            from_net,
            to_net,
            unreachable,
            state,
            task,
            local_addr,
        } = engine::spawn(Params {
            cfg: self.config,
            creds: Arc::new(creds),
            under,
            am_lighthouse: self.am_lighthouse,
            am_relay: self.am_relay,
            notifier: self.notifier,
        })?;
        let tunnel = NebulaTunnel {
            mtu,
            addresses,
            routes,
            to_engine: from_net,
            from_engine: to_net,
            unreachable,
        };
        let stack = NetStack::spawn(tunnel);
        let bridge = SocksBridge::start(stack.clone()).await?;
        Ok(Nebula {
            commands,
            state,
            task: Some(task),
            stack,
            bridge: Some(bridge),
            local_addr,
            warnings: Vec::new(),
        })
    }
}

/// Uma rede Nebula em funcionamento, só para o tráfego do app.
///
/// Ao ser largada (ou em [`stop`](Nebula::stop)), fecha os túneis e para.
pub struct Nebula {
    commands: mpsc::UnboundedSender<Command>,
    state: watch::Receiver<State>,
    task: Option<JoinHandle<()>>,
    stack: NetStack,
    bridge: Option<SocksBridge>,
    local_addr: SocketAddr,
    warnings: Vec<String>,
}

impl fmt::Debug for Nebula {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Nebula")
            .field("state", &*self.state.borrow())
            .finish_non_exhaustive()
    }
}

impl Nebula {
    /// O proxy SOCKS5 que leva as conexões pela rede Nebula: entregue-o ao cliente de SSH, SFTP,
    /// Telnet, FTP, S3 ou rclone do app. É só para os clientes do próprio app; a porta e a
    /// credencial são aleatórias e não devem aparecer na interface.
    pub fn proxy(&self) -> Proxy {
        let acesso = self
            .bridge
            .as_ref()
            .map(SocksBridge::access)
            .expect("a ponte só some em `stop`, que consome a Nebula");
        Proxy::Socks5 {
            host: acesso.addr.ip().to_string(),
            port: acesso.addr.port(),
            user: Some(acesso.user.clone()),
            password: Some((*acesso.password).clone()),
        }
    }

    /// A pilha TCP, para abrir conexões direto, sem passar pelo SOCKS5.
    pub fn stack(&self) -> &NetStack {
        &self.stack
    }

    /// O estado agora.
    pub fn state(&self) -> State {
        self.state.borrow().clone()
    }

    /// A porta UDP local (para os testes e o diagnóstico).
    pub fn udp_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Os avisos de quando a rede subiu (por exemplo, a porta UDP que estava ocupada).
    pub fn warnings(&self) -> &[String] {
        &self.warnings
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
                        return Err(io::Error::other(format!("a rede Nebula falhou: {motivo}")));
                    }
                    State::Stopped => {
                        return Err(io::Error::new(
                            io::ErrorKind::BrokenPipe,
                            "a rede Nebula parou",
                        ));
                    }
                    State::Connecting | State::Retrying(_) => {}
                }
                if rx.changed().await.is_err() {
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "a rede Nebula parou",
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
                        "a rede Nebula não conectou em {} s ({detalhe})",
                        limit.as_secs()
                    ),
                ))
            }
        }
    }

    /// Os túneis abertos agora (os lighthouses e os pares).
    pub async fn tunnels(&self) -> Vec<TunnelInfo> {
        let (resposta, espera) = oneshot::channel();
        if self.commands.send(Command::Tunnels(resposta)).is_err() {
            return Vec::new();
        }
        espera.await.unwrap_or_default()
    }

    /// Pede para parar sem esperar e sem largar o motor: serve quando há outros donos do
    /// `Arc` (as conexões abertas por ele) e o motor não pode mais ficar ligado. Avisa os
    /// pares como o [`Nebula::stop`]; depois disso, o estado vai a `Stopped`.
    pub fn close(&self) {
        let (ack, _) = oneshot::channel();
        let _ = self.commands.send(Command::Stop(ack));
    }

    /// Fecha as conexões, avisa os pares (`CloseTunnel`) e para.
    pub async fn stop(mut self) {
        if let Some(ponte) = self.bridge.take() {
            ponte.stop().await;
        }
        self.stack.clone().shutdown().await;
        // Espera o fim da tarefa, e não o `ack`: se a pilha já fechou, o motor sai sozinho, e o
        // tokio pode deixar na fila (sem largar) um pedido enviado no instante em que o receptor
        // fechou, então o `ack` nunca responderia nem seria largado.
        let (ack, _) = oneshot::channel();
        let _ = self.commands.send(Command::Stop(ack));
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for Nebula {
    fn drop(&mut self) {
        // Largada sem `stop`: ainda assim avisa os pares, sem esperar.
        let (ack, _) = oneshot::channel();
        let _ = self.commands.send(Command::Stop(ack));
    }
}

/// O endereço Nebula do nó, quando há (para a interface).
impl State {
    /// O endereço do nó na rede, se está conectado.
    pub fn address(&self) -> Option<IpAddr> {
        match self {
            State::Connected { address, .. } => Some(*address),
            _ => None,
        }
    }
}
