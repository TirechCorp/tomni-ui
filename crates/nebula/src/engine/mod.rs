//! O motor do Nebula: uma única tarefa do `tokio` que é dona de todo o estado (os túneis, os
//! apertos de mão em andamento, o que se sabe dos lighthouses) e reage ao UDP, aos pacotes IP
//! que a pilha manda, aos comandos e aos temporizadores.
//!
//! Portado do comportamento do Nebula (github.com/slackhq/nebula, licença MIT: `outside.go`,
//! `inside.go`, `handshake_manager.go`, `hostmap.go`, `lighthouse.go`, `connection_manager.go`,
//! `relay_manager.go` e `remote_list.go`). Um estado só numa tarefa dispensa as travas do Go e deixa a ordem dos
//! eventos previsível, o que os testes com relógio parado aproveitam.

mod data;
mod lighthouse;
mod relay;
mod session;

use std::collections::{HashMap, HashSet, VecDeque};
use std::future::poll_fn;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tomni_ui_netstack::{Cidr, Unreachable};

use crate::cert::{Certificate, Version};
use crate::config::{Config, Credentials, StaticMapNetwork};
use crate::handshake::Machine;
use crate::header::MessageType;
use crate::remotes::RemoteList;
use crate::timers::Timers;
use crate::tunnel::Tunnel;
use crate::underlay::Underlay;

/// Quantos pacotes IP o Nebula guarda por aperto de mão em andamento, esperando o túnel abrir
/// (`maxCachedPackets` do Nebula). O que passa disso é descartado e o TCP retransmite.
pub(crate) const MAX_CACHED_PACKETS: usize = 100;

/// Depois de trocar de endereço por causa de um pacote autêntico, o nó ignora por este tempo o
/// que o faria voltar ao endereço anterior (a proteção de roaming do Nebula).
pub(crate) const ROAMING_SUPPRESS: Duration = Duration::from_secs(2);

/// A profundidade das filas entre a pilha TCP e o motor, em pacotes. É o anel de uma placa de
/// rede: se enche, o pacote se perde, como na rede, e o TCP retransmite.
pub(crate) const PACKET_QUEUE: usize = 1024;

/// O que o motor diz sobre si.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// Abrindo o primeiro túnel com um lighthouse.
    Connecting,
    /// Há ao menos um lighthouse (ou nenhum configurado) e a rede funciona.
    Connected {
        /// O endereço do nó na rede Nebula.
        address: IpAddr,
        /// Quantos lighthouses têm túnel aberto.
        lighthouses: usize,
        /// Quantos túneis (de lighthouses e de pares) estão abertos.
        tunnels: usize,
    },
    /// Nenhum lighthouse respondeu; o motor tenta de novo, cada vez esperando mais. O texto diz
    /// o porquê.
    Retrying(String),
    /// O motor parou por um erro que não passa sozinho.
    Failed(String),
    /// Parou a pedido.
    Stopped,
}

/// Um túnel aberto com um par, para a interface mostrar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TunnelInfo {
    /// O endereço do par na rede Nebula.
    pub vpn_addr: IpAddr,
    /// O nome no certificado do par.
    pub name: String,
    /// O endereço UDP por onde o túnel passa agora. `None` quando ele passa por um relay.
    pub remote: Option<SocketAddr>,
    /// O relay por onde o túnel passa, quando não há caminho direto (os dois lados atrás de
    /// CGNAT, por exemplo).
    pub relay: Option<IpAddr>,
    /// Este nó iniciou o túnel.
    pub initiator: bool,
    /// O par é um lighthouse.
    pub lighthouse: bool,
}

/// Os comandos que o motor aceita de fora.
pub(crate) enum Command {
    /// Fecha os túneis e para.
    Stop(oneshot::Sender<()>),
    /// Lista os túneis.
    Tunnels(oneshot::Sender<Vec<TunnelInfo>>),
}

/// O que chega das tarefas que resolvem nomes.
pub(crate) enum Dns {
    /// Os endereços de uma entrada do `static_host_map`.
    Static(IpAddr, Vec<SocketAddr>),
    /// Os endereços de `lighthouse.advertise_addrs` que eram nomes.
    Advertise(Vec<SocketAddr>),
}

/// Os temporizadores do motor.
#[derive(Clone, Debug)]
pub(crate) enum Key {
    /// A próxima tentativa do aperto de mão com um par.
    Handshake(IpAddr),
    /// A conferência de um túnel (pelo índice local).
    Traffic(u32),
    /// Um punch agendado por um lighthouse.
    Punch(SocketAddr),
    /// Responder ao punch com um teste.
    Respond(IpAddr),
    /// Avisar os lighthouses dos nossos endereços.
    LighthouseUpdate,
    /// Tentar de novo um lighthouse que não respondeu.
    LighthouseRetry(IpAddr),
    /// Resolver os nomes do `static_host_map` de novo.
    StaticRefresh,
}

/// Algo que espera o túnel abrir.
pub(crate) enum Cached {
    /// Um pacote IP da pilha.
    Ip(Vec<u8>),
    /// Uma mensagem do próprio Nebula (do lighthouse, de teste).
    Message {
        kind: MessageType,
        subtype: u8,
        payload: Vec<u8>,
    },
}

/// Um aperto de mão que este nó iniciou e ainda não terminou.
pub(crate) struct Pending {
    pub(crate) machine: Machine,
    pub(crate) local_index: u32,
    pub(crate) stage0: Vec<u8>,
    pub(crate) counter: u32,
    pub(crate) next_due: Instant,
    pub(crate) cache: VecDeque<Cached>,
    pub(crate) last_remotes: Vec<SocketAddr>,
    pub(crate) last_error: Option<String>,
    pub(crate) sent_any: bool,
}

/// O papel de um relay num túnel: este nó é uma das pontas, ou passa os pacotes adiante.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RelayKind {
    /// Este nó é a ponta: o pacote de dentro é para ele.
    Terminal,
    /// Este nó é o relay: o pacote segue para o outro par.
    Forwarding,
}

/// Em que pé está um relay (`relay_manager.go`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RelayState {
    /// Pedimos e esperamos a resposta.
    Requested,
    /// O par pediu e ainda falta o outro lado responder (só no relay).
    PeerRequested,
    /// Passa pacotes.
    Established,
    /// O túnel com o outro lado caiu; volta a valer se for pedido de novo.
    Disestablished,
}

/// Um relay com `peer`, guardado no túnel com o nó que faz o relay (ou, no relay, no túnel com
/// cada ponta).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Relay {
    pub(crate) kind: RelayKind,
    pub(crate) state: RelayState,
    /// O índice que o outro lado põe nos pacotes de relay para nós.
    pub(crate) local_index: u32,
    /// O índice que pomos nos pacotes de relay para o outro lado.
    pub(crate) remote_index: u32,
    /// A outra ponta.
    pub(crate) peer: IpAddr,
}

/// Por onde um pacote chegou.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Via {
    /// Direto pelo UDP.
    Udp(SocketAddr),
    /// Dentro de um pacote de relay.
    Relay {
        /// O túnel com o nó que faz o relay (índice local).
        host: u32,
        /// O índice local do relay nesse túnel.
        relay: u32,
        /// O endereço na VPN de quem faz o relay.
        through: IpAddr,
    },
}

impl std::fmt::Display for Via {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Via::Udp(de) => write!(f, "{de}"),
            Via::Relay { through, .. } => write!(f, "pelo relay {through}"),
        }
    }
}

/// Um túnel aberto.
pub(crate) struct Host {
    pub(crate) vpn: IpAddr,
    pub(crate) addrs: Vec<IpAddr>,
    pub(crate) unsafe_nets: Vec<Cidr>,
    pub(crate) local_index: u32,
    pub(crate) remote_index: u32,
    pub(crate) tunnel: Tunnel,
    pub(crate) remote: Option<SocketAddr>,
    pub(crate) last_roam: Option<(SocketAddr, Instant)>,
    pub(crate) peer_cert: Certificate,
    pub(crate) my_cert_version: Version,
    pub(crate) initiator: bool,
    pub(crate) time: u64,
    pub(crate) in_traffic: bool,
    pub(crate) out_traffic: bool,
    pub(crate) pending_deletion: bool,
    pub(crate) last_activity: Instant,
    pub(crate) stage0: Option<Vec<u8>>,
    pub(crate) stage2: Option<Vec<u8>>,
    pub(crate) rehandshaked: bool,
    /// Os relays que passam por este túnel, pela outra ponta.
    pub(crate) relay_for: HashMap<IpAddr, Relay>,
    /// Os nós que fazem relay para chegar a este par, quando não há `remote`.
    pub(crate) relays: Vec<IpAddr>,
}

/// O que o motor precisa para começar.
pub(crate) struct Params<U> {
    pub(crate) cfg: Config,
    pub(crate) creds: Arc<Credentials>,
    pub(crate) under: U,
    pub(crate) am_lighthouse: bool,
    pub(crate) am_relay: bool,
    pub(crate) notifier: Option<Arc<dyn Fn(State) + Send + Sync>>,
}

/// As pontas de fora do motor.
pub(crate) struct Handles {
    pub(crate) commands: mpsc::UnboundedSender<Command>,
    pub(crate) from_net: mpsc::Sender<Vec<u8>>,
    pub(crate) to_net: mpsc::Receiver<Vec<u8>>,
    pub(crate) unreachable: mpsc::UnboundedReceiver<Unreachable>,
    pub(crate) state: watch::Receiver<State>,
    pub(crate) task: JoinHandle<()>,
    pub(crate) local_addr: SocketAddr,
}

struct Receivers {
    commands: mpsc::UnboundedReceiver<Command>,
    from_net: mpsc::Receiver<Vec<u8>>,
    dns: mpsc::UnboundedReceiver<Dns>,
}

pub(crate) struct Engine<U: Underlay> {
    cfg: Config,
    creds: Arc<Credentials>,
    under: U,
    am_lighthouse: bool,
    /// Este nó passa pacotes de um par a outro (`relay.am_relay`; só nos testes).
    am_relay: bool,
    local_port: u16,
    /// Os endereços do nó na rede e as redes a que pertence (de todos os certificados).
    my_addrs: Vec<IpAddr>,
    my_nets: Vec<Cidr>,
    advertise: Vec<SocketAddr>,
    hosts: HashMap<u32, Host>,
    primary: HashMap<IpAddr, u32>,
    pending: HashMap<IpAddr, Pending>,
    pending_by_index: HashMap<u32, IpAddr>,
    remotes: HashMap<IpAddr, RemoteList>,
    /// O índice local de cada relay e o túnel onde ele está (o `Relays` do `hostmap.go`).
    relay_index: HashMap<u32, u32>,
    lighthouses: HashSet<IpAddr>,
    lighthouse_errors: HashMap<IpAddr, String>,
    lighthouse_backoff: HashMap<IpAddr, Duration>,
    timers: Timers<Key>,
    net_tx: mpsc::Sender<Vec<u8>>,
    unreachable_tx: mpsc::UnboundedSender<Unreachable>,
    dns_tx: mpsc::UnboundedSender<Dns>,
    state_tx: watch::Sender<State>,
    state: State,
    notifier: Option<Arc<dyn Fn(State) + Send + Sync>>,
}

/// Os nanossegundos desde 1970, que o aperto de mão carrega.
pub(crate) fn now_nanos() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// Põe o motor para rodar.
pub(crate) fn spawn<U: Underlay>(params: Params<U>) -> io::Result<Handles> {
    let local_addr = params.under.local_addr()?;
    let (commands_tx, commands_rx) = mpsc::unbounded_channel();
    let (from_net_tx, from_net_rx) = mpsc::channel(PACKET_QUEUE);
    let (to_net_tx, to_net_rx) = mpsc::channel(PACKET_QUEUE);
    let (unreachable_tx, unreachable_rx) = mpsc::unbounded_channel();
    let (dns_tx, dns_rx) = mpsc::unbounded_channel();
    let (state_tx, state_rx) = watch::channel(State::Connecting);

    let mut my_addrs = Vec::new();
    let mut my_nets = Vec::new();
    for cert in [&params.creds.v1, &params.creds.v2].into_iter().flatten() {
        for rede in cert.networks() {
            if !my_addrs.contains(&rede.addr()) {
                my_addrs.push(rede.addr());
            }
            if !my_nets.contains(rede) {
                my_nets.push(*rede);
            }
        }
    }
    let lighthouses: HashSet<IpAddr> = params.cfg.lighthouse.hosts.iter().copied().collect();
    let engine = Engine {
        cfg: params.cfg,
        creds: params.creds,
        under: params.under,
        am_lighthouse: params.am_lighthouse,
        am_relay: params.am_relay,
        local_port: local_addr.port(),
        my_addrs,
        my_nets,
        advertise: Vec::new(),
        hosts: HashMap::new(),
        primary: HashMap::new(),
        pending: HashMap::new(),
        pending_by_index: HashMap::new(),
        remotes: HashMap::new(),
        relay_index: HashMap::new(),
        lighthouses,
        lighthouse_errors: HashMap::new(),
        lighthouse_backoff: HashMap::new(),
        timers: Timers::new(),
        net_tx: to_net_tx,
        unreachable_tx,
        dns_tx,
        state_tx,
        state: State::Connecting,
        notifier: params.notifier,
    };
    let receivers = Receivers {
        commands: commands_rx,
        from_net: from_net_rx,
        dns: dns_rx,
    };
    let task = tokio::spawn(engine.run(receivers));
    Ok(Handles {
        commands: commands_tx,
        from_net: from_net_tx,
        to_net: to_net_rx,
        unreachable: unreachable_rx,
        state: state_rx,
        task,
        local_addr,
    })
}

impl<U: Underlay> Engine<U> {
    async fn run(mut self, mut rx: Receivers) {
        self.start();
        let mut buf = vec![0u8; 65_535];
        loop {
            let deadline = self.timers.next();
            let wait = async {
                match deadline {
                    Some(quando) => tokio::time::sleep_until(quando).await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                recebido = poll_fn(|cx| self.under.poll_recv_from(cx, &mut buf)) => {
                    match recebido {
                        Ok((n, de)) => self.on_udp(&buf[..n], de),
                        Err(erro) if matches!(
                            erro.kind(),
                            io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionRefused
                        ) => {
                            // O Windows avisa assim que um destino devolveu "porta inalcançável".
                        }
                        Err(erro) => {
                            self.set_state(State::Failed(format!("o UDP parou: {erro}")));
                            return;
                        }
                    }
                }
                pacote = rx.from_net.recv() => match pacote {
                    Some(pacote) => self.on_inside(pacote),
                    // A pilha acabou: ninguém mais usa a rede.
                    None => {
                        self.stop();
                        return;
                    }
                },
                comando = rx.commands.recv() => match comando {
                    Some(Command::Stop(ack)) => {
                        // O que a pilha já mandou (os RST do desligamento) sai antes dos CloseTunnel.
                        while let Ok(pacote) = rx.from_net.try_recv() {
                            self.on_inside(pacote);
                        }
                        self.stop();
                        let _ = ack.send(());
                        return;
                    }
                    Some(Command::Tunnels(resposta)) => {
                        let _ = resposta.send(self.tunnels());
                    }
                    None => {
                        self.stop();
                        return;
                    }
                },
                Some(dns) = rx.dns.recv() => self.on_dns(dns),
                () = wait => {
                    let agora = Instant::now();
                    while let Some((quando, chave)) = self.timers.pop_due(agora) {
                        self.on_timer(quando, chave);
                    }
                }
            }
        }
    }

    /// O que acontece ao começar: resolve os nomes e abre o aperto de mão com os lighthouses.
    fn start(&mut self) {
        let mut entradas = Vec::new();
        for (vpn, nomes) in &self.cfg.static_host_map {
            if nomes.is_empty() {
                continue;
            }
            self.remotes.entry(*vpn).or_default().resolving = true;
            entradas.push((*vpn, nomes.clone()));
        }
        if !entradas.is_empty() {
            spawn_resolver(entradas, self.cfg.static_map_network, self.dns_tx.clone());
            let cadencia = self.cfg.static_map_cadence;
            self.timers
                .add(Instant::now() + cadencia, Key::StaticRefresh);
        }
        let mut nomes = Vec::new();
        for texto in &self.cfg.lighthouse.advertise_addrs {
            match texto.parse::<SocketAddr>() {
                Ok(addr) => self.advertise.push(addr),
                Err(_) => nomes.push(texto.clone()),
            }
        }
        if !nomes.is_empty() {
            let tx = self.dns_tx.clone();
            tokio::spawn(async move {
                let mut achados = Vec::new();
                for nome in nomes {
                    if let Ok(enderecos) = tokio::net::lookup_host(nome).await {
                        achados.extend(enderecos.filter(SocketAddr::is_ipv4));
                    }
                }
                let _ = tx.send(Dns::Advertise(achados));
            });
        }
        if !self.am_lighthouse {
            let lighthouses: Vec<IpAddr> = self.cfg.lighthouse.hosts.clone();
            for lh in lighthouses {
                self.start_handshake(lh);
            }
            let intervalo = self.cfg.lighthouse.interval;
            self.timers
                .add(Instant::now() + intervalo, Key::LighthouseUpdate);
        }
        self.connect_relays();
        self.recompute_state();
    }

    fn on_dns(&mut self, dns: Dns) {
        match dns {
            Dns::Static(vpn, enderecos) => {
                let lista = self.remotes.entry(vpn).or_default();
                lista.resolving = false;
                let mudou = lista.statics != enderecos;
                lista.statics = enderecos;
                if mudou && self.pending.contains_key(&vpn) {
                    self.retrigger(vpn);
                }
            }
            Dns::Advertise(enderecos) => {
                for addr in enderecos {
                    if !self.advertise.contains(&addr) {
                        self.advertise.push(addr);
                    }
                }
            }
        }
    }

    fn on_timer(&mut self, quando: Instant, chave: Key) {
        match chave {
            Key::Handshake(vpn) => self.handshake_tick(vpn, quando),
            Key::Traffic(indice) => self.traffic_tick(indice),
            Key::Punch(alvo) => self.raw_send(&[0], alvo),
            Key::Respond(vpn) => self.respond_to_punch(vpn),
            Key::LighthouseUpdate => self.lighthouse_update_tick(),
            Key::LighthouseRetry(vpn) => self.lighthouse_retry(vpn),
            Key::StaticRefresh => {
                let entradas: Vec<(IpAddr, Vec<String>)> = self
                    .cfg
                    .static_host_map
                    .iter()
                    .filter(|(_, nomes)| !nomes.is_empty())
                    .map(|(vpn, nomes)| (*vpn, nomes.clone()))
                    .collect();
                spawn_resolver(entradas, self.cfg.static_map_network, self.dns_tx.clone());
                let cadencia = self.cfg.static_map_cadence;
                self.timers
                    .add(Instant::now() + cadencia, Key::StaticRefresh);
            }
        }
    }

    /// Fecha todos os túneis avisando os pares e para.
    fn stop(&mut self) {
        let indices: Vec<u32> = self.hosts.keys().copied().collect();
        for indice in indices {
            self.send_close(indice);
        }
        self.set_state(State::Stopped);
    }

    fn tunnels(&self) -> Vec<TunnelInfo> {
        let mut lista: Vec<TunnelInfo> = self
            .hosts
            .values()
            .filter(|h| self.primary.get(&h.vpn) == Some(&h.local_index))
            .map(|h| TunnelInfo {
                vpn_addr: h.vpn,
                name: h.peer_cert.name().to_owned(),
                remote: h.remote,
                relay: if h.remote.is_some() {
                    None
                } else {
                    h.relays.first().copied()
                },
                initiator: h.initiator,
                lighthouse: self.lighthouses.contains(&h.vpn),
            })
            .collect();
        lista.sort_by_key(|t| t.vpn_addr);
        lista
    }

    pub(crate) fn raw_send(&self, bytes: &[u8], to: SocketAddr) {
        // Um erro do sistema aqui (rede fora do ar) é igual a um pacote perdido.
        let _ = self.under.try_send_to(bytes, to);
    }

    fn unreachable(&self, addr: IpAddr, reason: String) {
        let _ = self.unreachable_tx.send(Unreachable { addr, reason });
    }

    fn set_state(&mut self, novo: State) {
        if novo == self.state {
            return;
        }
        self.state = novo.clone();
        let _ = self.state_tx.send(novo.clone());
        if let Some(aviso) = &self.notifier {
            aviso(novo);
        }
    }

    /// Recalcula o estado a partir dos túneis e das falhas dos lighthouses.
    pub(crate) fn recompute_state(&mut self) {
        if matches!(self.state, State::Stopped | State::Failed(_)) {
            return;
        }
        let ligados = self
            .cfg
            .lighthouse
            .hosts
            .iter()
            .filter(|lh| self.primary.contains_key(lh))
            .count();
        let tuneis = self
            .hosts
            .values()
            .filter(|h| self.primary.get(&h.vpn) == Some(&h.local_index))
            .count();
        let novo = if self.am_lighthouse || self.cfg.lighthouse.hosts.is_empty() || ligados > 0 {
            match self.my_addrs.first() {
                Some(address) => State::Connected {
                    address: *address,
                    lighthouses: ligados,
                    tunnels: tuneis,
                },
                None => State::Connecting,
            }
        } else if self.lighthouse_errors.len() >= self.cfg.lighthouse.hosts.len() {
            let mut motivos: Vec<String> = self.lighthouse_errors.values().cloned().collect();
            motivos.sort();
            State::Retrying(format!(
                "nenhum lighthouse respondeu: {}",
                motivos.join("; ")
            ))
        } else {
            State::Connecting
        };
        self.set_state(novo);
    }

    /// As redes do nó contêm o endereço.
    pub(crate) fn in_my_networks(&self, ip: IpAddr) -> bool {
        self.my_nets.iter().any(|rede| rede.contains(ip))
    }
}

/// Resolve os nomes do `static_host_map` em tarefas e manda o resultado de cada entrada.
fn spawn_resolver(
    entradas: Vec<(IpAddr, Vec<String>)>,
    rede: StaticMapNetwork,
    tx: mpsc::UnboundedSender<Dns>,
) {
    for (vpn, nomes) in entradas {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut achados: Vec<SocketAddr> = Vec::new();
            for nome in nomes {
                let Ok(enderecos) = tokio::net::lookup_host(nome).await else {
                    continue;
                };
                for addr in enderecos {
                    let vale = match rede {
                        StaticMapNetwork::Ip4 => addr.is_ipv4(),
                        StaticMapNetwork::Ip6 => addr.is_ipv6(),
                        StaticMapNetwork::Ip => true,
                    };
                    if vale && !achados.contains(&addr) {
                        achados.push(addr);
                    }
                }
            }
            let _ = tx.send(Dns::Static(vpn, achados));
        });
    }
}
