//! O motor: uma tarefa do `tokio` que liga o [`Vl1`] a um [`Underlay`] (o socket UDP de verdade
//! ou a rede simulada dos testes). Ela recebe os datagramas, chama o relógio, aceita pedidos
//! de envio e consulta, e entrega o que sobe para o VL2 por um canal.

use std::future::poll_fn;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::{Instant, MissedTickBehavior, interval};
use tomni_ui_netstack::Underlay;

use crate::packet::Verb;
use crate::peer::{PeerRole, RemoteVersion};
use crate::vl1::{Config, Effect, Vl1};
use crate::{Address, World};

/// Com que frequência o relógio do motor acorda o VL1. O VL1 só age a cada
/// [`crate::vl1::PING_CHECK_INTERVAL`]; acordar mais seguido só limita o atraso.
const ACORDAR_A_CADA: Duration = Duration::from_secs(1);

/// O maior datagrama que o motor lê: o do UDP (o protocolo manda no máximo 1432 bytes, mas o
/// que chega de fora pode ser maior e é descartado pelo VL1).
const LEITURA: usize = 2048;

/// Um par, como o motor o mostra.
#[derive(Clone, Debug)]
pub struct PeerInfo {
    /// O endereço.
    pub address: Address,
    /// O papel.
    pub role: PeerRole,
    /// A versão anunciada, se já se apresentou.
    pub version: Option<RemoteVersion>,
    /// A menor latência entre os caminhos vivos, em milissegundos.
    pub latency: Option<u32>,
    /// Os caminhos físicos conhecidos.
    pub paths: Vec<SocketAddr>,
    /// Há caminho vivo agora.
    pub alive: bool,
}

/// O estado do nó num instante.
#[derive(Clone, Debug)]
pub struct Snapshot {
    /// O nosso endereço.
    pub address: Address,
    /// Os pares.
    pub peers: Vec<PeerInfo>,
    /// O id e a data do planet em uso.
    pub planet: (u64, u64),
    /// O id e a data de cada moon em uso.
    pub moons: Vec<(u64, u64)>,
    /// Os mundos por inteiro, para o app gravar.
    pub worlds: Vec<World>,
}

enum Command {
    Send {
        dest: Address,
        verb: Verb,
        payload: Vec<u8>,
        compress: bool,
    },
    Controller(Address),
    Snapshot(oneshot::Sender<Snapshot>),
}

/// O que o motor entrega a quem o usa: o que sobe do VL1 (sem o que é só do socket).
pub type Events = mpsc::UnboundedReceiver<Effect>;

/// O motor em execução. Soltar o `Engine` encerra a tarefa.
pub struct Engine {
    comandos: mpsc::UnboundedSender<Command>,
    tarefa: JoinHandle<()>,
}

impl Engine {
    /// Liga o VL1 ao `under` e começa a trabalhar. Precisa de um runtime do `tokio`.
    pub fn start<U: Underlay>(under: U, config: Config) -> (Engine, Events) {
        let (comandos, recebe) = mpsc::unbounded_channel();
        let (eventos, saida) = mpsc::unbounded_channel();
        let base = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
        let under = Arc::new(under);
        let mut vl1 = Vl1::new(config);
        if let Ok(local) = under.local_addr() {
            let resolve = under.clone();
            vl1.set_local(
                local.port(),
                Arc::new(move |alvo| resolve.local_ip_toward(alvo)),
            );
        }
        let tarefa = tokio::spawn(rodar(under, vl1, recebe, eventos, base));
        (Engine { comandos, tarefa }, saida)
    }

    /// Manda `verb` com `payload` a `dest`.
    pub fn send(&self, dest: Address, verb: Verb, payload: Vec<u8>) {
        self.send_with(dest, verb, payload, false);
    }

    /// Como [`Engine::send`], comprimindo a carga se isso a diminuir.
    pub fn send_with(&self, dest: Address, verb: Verb, payload: Vec<u8>, compress: bool) {
        let _ = self.comandos.send(Command::Send {
            dest,
            verb,
            payload,
            compress,
        });
    }

    /// Diz quem é o controlador da nossa rede.
    pub fn set_controller(&self, endereco: Address) {
        let _ = self.comandos.send(Command::Controller(endereco));
    }

    /// O estado do nó agora. `None` se o motor já parou.
    pub async fn snapshot(&self) -> Option<Snapshot> {
        let (tx, rx) = oneshot::channel();
        self.comandos.send(Command::Snapshot(tx)).ok()?;
        rx.await.ok()
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.tarefa.abort();
    }
}

/// O que acordou o laço.
enum Acordou {
    Datagrama(std::io::Result<(usize, SocketAddr)>),
    Relogio,
    Comando(Option<Command>),
}

async fn rodar<U: Underlay>(
    under: Arc<U>,
    mut vl1: Vl1,
    mut comandos: mpsc::UnboundedReceiver<Command>,
    eventos: mpsc::UnboundedSender<Effect>,
    base: u64,
) {
    let inicio = Instant::now();
    let agora = move || base + u64::try_from(inicio.elapsed().as_millis()).unwrap_or(u64::MAX);
    let mut relogio = interval(ACORDAR_A_CADA);
    relogio.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut leitura = vec![0u8; LEITURA];
    loop {
        let acordou = tokio::select! {
            r = poll_fn(|cx| under.poll_recv_from(cx, &mut leitura)) => Acordou::Datagrama(r),
            _ = relogio.tick() => Acordou::Relogio,
            c = comandos.recv() => Acordou::Comando(c),
        };
        let agora = agora();
        match acordou {
            Acordou::Datagrama(Ok((n, de))) => vl1.receive(agora, de, &leitura[..n]),
            // Erro de leitura do socket (por exemplo, ICMP de porta fechada): segue.
            Acordou::Datagrama(Err(_)) => {}
            Acordou::Relogio => vl1.tick(agora),
            Acordou::Comando(None) => break,
            Acordou::Comando(Some(Command::Send {
                dest,
                verb,
                payload,
                compress,
            })) => vl1.send_with(agora, dest, verb, &payload, compress),
            Acordou::Comando(Some(Command::Controller(a))) => vl1.set_controller(a),
            Acordou::Comando(Some(Command::Snapshot(tx))) => {
                let _ = tx.send(tirar_retrato(&vl1, agora));
            }
        }
        for efeito in vl1.take_effects() {
            match efeito {
                // Datagrama perdido não é erro: o protocolo retransmite o que importa.
                Effect::Send { to, data } => {
                    let _ = under.try_send_to(&data, to);
                }
                outro => {
                    let _ = eventos.send(outro);
                }
            }
        }
    }
}

fn tirar_retrato(vl1: &Vl1, agora: u64) -> Snapshot {
    let raizes = vl1.roots();
    let mut pares: Vec<PeerInfo> = vl1
        .peers()
        .map(|p| PeerInfo {
            address: p.address(),
            role: raizes
                .iter()
                .find(|r| r.address == p.address())
                .map_or(PeerRole::Member, |r| r.role),
            version: p.version(),
            latency: p.latency(agora),
            paths: p.paths().iter().map(|c| c.addr()).collect(),
            alive: p.has_alive_path(agora),
        })
        .collect();
    pares.sort_by_key(|p| p.address);
    Snapshot {
        address: vl1.address(),
        peers: pares,
        planet: (vl1.planet().id(), vl1.planet().timestamp()),
        moons: vl1
            .moons()
            .iter()
            .map(|m| (m.id(), m.timestamp()))
            .collect(),
        worlds: std::iter::once(vl1.planet().clone())
            .chain(vl1.moons().iter().cloned())
            .collect(),
    }
}
