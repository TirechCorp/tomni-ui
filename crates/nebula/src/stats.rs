//! As métricas do Nebula (`stats`): os mesmos contadores, medidores e o histograma dos apertos
//! de mão que o Nebula oficial publica, com os mesmos nomes, entregues pelos mesmos dois meios:
//! um endereço HTTP que o Prometheus lê (`stats.type: prometheus`) ou o envio periódico ao
//! Graphite (`stats.type: graphite`).
//!
//! Portado de `stats.go`, `graphite.go`, `message_metrics.go` e do provedor
//! `go-metrics-prometheus` que o Nebula usa (github.com/slackhq/nebula, licença MIT). As
//! métricas do runtime do Go (`runtime.*`, `debug.*`) e as do firewall não existem aqui.

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::fmt::Write as _;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::engine::Command;
use crate::header::{self, MessageType};
use crate::meta::MetaType;
use crate::util::now_unix;

/// O `stats` do `config.yml`: para onde vão as métricas e quais existem.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stats {
    /// O `stats.interval`: de quanto em quanto tempo o Graphite recebe as métricas (obrigatório;
    /// o Prometheus lê na hora em que pede).
    pub interval: Duration,
    /// Para onde vão.
    pub export: StatsExport,
    /// O `stats.message_metrics`: conta cada tipo de mensagem do Nebula (sem ele, só os
    /// `recv_error`).
    pub message_metrics: bool,
    /// O `stats.lighthouse_metrics`: conta cada tipo de mensagem do lighthouse.
    pub lighthouse_metrics: bool,
}

/// Para onde vão as métricas.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StatsExport {
    /// `stats.type: graphite`: manda as linhas do Graphite por TCP a cada intervalo.
    Graphite {
        /// O `stats.protocol` (padrão `tcp`).
        protocol: GraphiteProtocol,
        /// O `stats.host`: o `host:porta` do Graphite.
        host: String,
        /// O `stats.prefix` (padrão `nebula`).
        prefix: String,
    },
    /// `stats.type: prometheus`: um endereço HTTP que entrega as métricas a quem pedir.
    Prometheus {
        /// O `stats.listen`: o `host:porta` em que o HTTP escuta (`:8080` é em todos os
        /// endereços).
        listen: String,
        /// O `stats.path`: o caminho das métricas (`/metrics`).
        path: String,
        /// O `stats.namespace`, o começo de cada nome.
        namespace: String,
        /// O `stats.subsystem`, depois do `namespace`.
        subsystem: String,
    },
}

/// A família de endereços do Graphite (`stats.protocol`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphiteProtocol {
    /// `tcp`: IPv4 ou IPv6, o que o nome resolver.
    Tcp,
    /// `tcp4`: só IPv4.
    Tcp4,
    /// `tcp6`: só IPv6.
    Tcp6,
}

// Os nomes que não dependem do tipo da mensagem.
pub(crate) const HANDSHAKE_INITIATED: &str = "handshake_manager.initiated";
pub(crate) const HANDSHAKE_TIMED_OUT: &str = "handshake_manager.timed_out";
pub(crate) const HANDSHAKE_BLOCKLISTED: &str = "handshake_manager.rejected.blocklisted";
pub(crate) const CACHED_SENT: &str = "hostinfo.cached_packets.sent";
pub(crate) const CACHED_DROPPED: &str = "hostinfo.cached_packets.dropped";
pub(crate) const UDP_TX_DROPPED: &str = "udp.tx.dropped";
pub(crate) const RX_INVALID: &str = "messages.rx.invalid";
pub(crate) const TX_EXHAUSTED: &str = "messages.tx.exhausted";
const LIGHTHOUSE_QUERY_DROPPED: &str = "lighthouse.query.dropped";
const HANDSHAKES: &str = "handshakes";

/// Quantos valores o histograma guarda (o tamanho da amostra do Nebula).
const HISTOGRAM_SAMPLE: usize = 1028;

/// Os percentis que vão ao Graphite, como no Nebula.
const GRAPHITE_PERCENTILES: [(f64, &str); 5] = [
    (0.5, "50"),
    (0.75, "75"),
    (0.95, "95"),
    (0.99, "99"),
    (0.999, "999"),
];

/// Quanto o Graphite espera para conectar e receber (o `graphiteTimeout` do Nebula).
const GRAPHITE_TIMEOUT: Duration = Duration::from_secs(30);

/// Quanto o HTTP do Prometheus espera o pedido chegar.
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

/// O maior pedido HTTP aceito (o cabeçalho; o Prometheus manda poucas centenas de bytes).
const HTTP_MAX_REQUEST: usize = 16 * 1024;

/// O valor de uma métrica num instante.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Value {
    /// Um contador, que só cresce.
    Counter(u64),
    /// Um medidor, o valor de agora.
    Gauge(i64),
    /// A amostra de um histograma.
    Histogram(HistogramSnapshot),
}

/// Uma métrica com o nome do Nebula (`messages.tx.lighthouse`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Sample {
    pub(crate) name: &'static str,
    pub(crate) value: Value,
}

/// A amostra de um histograma: quantos valores entraram desde o começo e os últimos
/// [`HISTOGRAM_SAMPLE`], em ordem.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct HistogramSnapshot {
    count: u64,
    sorted: Vec<i64>,
}

impl HistogramSnapshot {
    fn sum(&self) -> i64 {
        self.sorted
            .iter()
            .fold(0i64, |soma, v| soma.saturating_add(*v))
    }

    fn min(&self) -> i64 {
        self.sorted.first().copied().unwrap_or(0)
    }

    fn max(&self) -> i64 {
        self.sorted.last().copied().unwrap_or(0)
    }

    #[allow(clippy::cast_precision_loss)]
    fn mean(&self) -> f64 {
        if self.sorted.is_empty() {
            return 0.0;
        }
        self.sum() as f64 / self.sorted.len() as f64
    }

    /// A variância da amostra (a da população, como o `SampleVariance` do go-metrics).
    #[allow(clippy::cast_precision_loss)]
    fn variance(&self) -> f64 {
        if self.sorted.is_empty() {
            return 0.0;
        }
        let media = self.mean();
        let soma: f64 = self
            .sorted
            .iter()
            .map(|v| {
                let d = *v as f64 - media;
                d * d
            })
            .sum();
        soma / self.sorted.len() as f64
    }

    fn std_dev(&self) -> f64 {
        self.variance().sqrt()
    }

    /// O percentil `p` (de 0 a 1), interpolado como o `SamplePercentiles` do go-metrics.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    fn percentile(&self, p: f64) -> f64 {
        let tamanho = self.sorted.len();
        if tamanho == 0 {
            return 0.0;
        }
        let pos = p * (tamanho + 1) as f64;
        if pos < 1.0 {
            self.sorted[0] as f64
        } else if pos >= tamanho as f64 {
            self.sorted[tamanho - 1] as f64
        } else {
            let baixo = self.sorted[pos as usize - 1] as f64;
            let alto = self.sorted[pos as usize] as f64;
            baixo + (pos - pos.floor()) * (alto - baixo)
        }
    }
}

/// Os contadores do motor. Só existem os nomes que o `stats` liga; contar um nome que não
/// existe não faz nada, então o motor conta sem perguntar. Sem `stats`, nada é guardado.
pub(crate) struct Metrics {
    counters: RefCell<BTreeMap<&'static str, u64>>,
    handshakes: RefCell<Option<(u64, VecDeque<i64>)>>,
    lighthouse: bool,
}

impl Metrics {
    pub(crate) fn new(stats: Option<&Stats>) -> Metrics {
        let mut counters = BTreeMap::new();
        let Some(stats) = stats else {
            return Metrics {
                counters: RefCell::new(counters),
                handshakes: RefCell::new(None),
                lighthouse: false,
            };
        };
        let mut nomes = vec![
            HANDSHAKE_INITIATED,
            HANDSHAKE_TIMED_OUT,
            HANDSHAKE_BLOCKLISTED,
            CACHED_SENT,
            CACHED_DROPPED,
            UDP_TX_DROPPED,
            LIGHTHOUSE_QUERY_DROPPED,
            "messages.rx.recv_error",
            "messages.tx.recv_error",
        ];
        if stats.message_metrics {
            nomes.extend([
                "messages.rx.handshake_ixpsk0",
                "messages.tx.handshake_ixpsk0",
                "messages.rx.lighthouse",
                "messages.tx.lighthouse",
                "messages.rx.test_request",
                "messages.tx.test_request",
                "messages.rx.test_response",
                "messages.tx.test_response",
                "messages.rx.close_tunnel",
                "messages.tx.close_tunnel",
                "messages.rx.other",
                "messages.tx.other",
                RX_INVALID,
                TX_EXHAUSTED,
            ]);
        }
        if stats.lighthouse_metrics {
            nomes.extend([
                "lighthouse.rx.HostQuery",
                "lighthouse.tx.HostQuery",
                "lighthouse.rx.HostQueryReply",
                "lighthouse.tx.HostQueryReply",
                "lighthouse.rx.HostUpdateNotification",
                "lighthouse.tx.HostUpdateNotification",
                "lighthouse.rx.HostPunchNotification",
                "lighthouse.tx.HostPunchNotification",
                "lighthouse.rx.HostUpdateNotificationAck",
                "lighthouse.tx.HostUpdateNotificationAck",
                "lighthouse.rx.other",
                "lighthouse.tx.other",
            ]);
        }
        for nome in nomes {
            counters.insert(nome, 0);
        }
        Metrics {
            counters: RefCell::new(counters),
            handshakes: RefCell::new(Some((0, VecDeque::new()))),
            lighthouse: stats.lighthouse_metrics,
        }
    }

    /// Soma `n` ao contador `nome`, se ele existe.
    pub(crate) fn add(&self, nome: &'static str, n: u64) {
        if let Some(valor) = self.counters.borrow_mut().get_mut(nome) {
            *valor = valor.saturating_add(n);
        }
    }

    /// Soma um ao contador `nome`, se ele existe.
    pub(crate) fn inc(&self, nome: &'static str) {
        self.add(nome, 1);
    }

    /// Uma mensagem do Nebula chegou (as de dados não contam, como no Nebula).
    pub(crate) fn rx(&self, tipo: MessageType, subtipo: u8) {
        if let Some(nome) = message_name(true, tipo, subtipo) {
            self.inc(nome);
        }
    }

    /// Uma mensagem do Nebula saiu (as de dados não contam, como no Nebula).
    pub(crate) fn tx(&self, tipo: MessageType, subtipo: u8) {
        if let Some(nome) = message_name(false, tipo, subtipo) {
            self.inc(nome);
        }
    }

    /// Uma mensagem do lighthouse chegou.
    pub(crate) fn lighthouse_rx(&self, tipo: MetaType) {
        if self.lighthouse {
            self.inc(lighthouse_name(true, tipo));
        }
    }

    /// `n` mensagens do lighthouse saíram.
    pub(crate) fn lighthouse_tx(&self, tipo: MetaType, n: u64) {
        if self.lighthouse {
            self.add(lighthouse_name(false, tipo), n);
        }
    }

    /// Um aperto de mão que este nó iniciou terminou, depois de `duracao`.
    pub(crate) fn handshake_done(&self, duracao: Duration) {
        if let Some((total, valores)) = self.handshakes.borrow_mut().as_mut() {
            *total += 1;
            if valores.len() == HISTOGRAM_SAMPLE {
                valores.pop_front();
            }
            valores.push_back(i64::try_from(duracao.as_nanos()).unwrap_or(i64::MAX));
        }
    }

    /// Os contadores e o histograma agora (os medidores vêm do motor).
    pub(crate) fn snapshot(&self) -> Vec<Sample> {
        let mut lista: Vec<Sample> = self
            .counters
            .borrow()
            .iter()
            .map(|(nome, valor)| Sample {
                name: nome,
                value: Value::Counter(*valor),
            })
            .collect();
        if let Some((total, valores)) = self.handshakes.borrow().as_ref() {
            let mut sorted: Vec<i64> = valores.iter().copied().collect();
            sorted.sort_unstable();
            lista.push(Sample {
                name: HANDSHAKES,
                value: Value::Histogram(HistogramSnapshot {
                    count: *total,
                    sorted,
                }),
            });
        }
        lista
    }
}

/// O nome do contador de uma mensagem, pela tabela do `message_metrics.go`: o que não está
/// na tabela vai para `other`; as de dados (`Message`) não contam.
fn message_name(rx: bool, tipo: MessageType, subtipo: u8) -> Option<&'static str> {
    let par = match (tipo, subtipo) {
        (MessageType::Message, _) => return None,
        (MessageType::Handshake, header::HANDSHAKE_IX_PSK0) => (
            "messages.rx.handshake_ixpsk0",
            "messages.tx.handshake_ixpsk0",
        ),
        (MessageType::RecvError, 0) => ("messages.rx.recv_error", "messages.tx.recv_error"),
        (MessageType::LightHouse, 0) => ("messages.rx.lighthouse", "messages.tx.lighthouse"),
        (MessageType::Test, header::TEST_REQUEST) => {
            ("messages.rx.test_request", "messages.tx.test_request")
        }
        (MessageType::Test, header::TEST_REPLY) => {
            ("messages.rx.test_response", "messages.tx.test_response")
        }
        (MessageType::CloseTunnel, 0) => ("messages.rx.close_tunnel", "messages.tx.close_tunnel"),
        _ => ("messages.rx.other", "messages.tx.other"),
    };
    Some(if rx { par.0 } else { par.1 })
}

/// O nome do contador de uma mensagem do lighthouse (`lighthouse.rx.HostQuery`).
fn lighthouse_name(rx: bool, tipo: MetaType) -> &'static str {
    let par = match tipo {
        MetaType::HostQuery => ("lighthouse.rx.HostQuery", "lighthouse.tx.HostQuery"),
        MetaType::HostQueryReply => (
            "lighthouse.rx.HostQueryReply",
            "lighthouse.tx.HostQueryReply",
        ),
        MetaType::HostUpdateNotification => (
            "lighthouse.rx.HostUpdateNotification",
            "lighthouse.tx.HostUpdateNotification",
        ),
        MetaType::HostPunchNotification => (
            "lighthouse.rx.HostPunchNotification",
            "lighthouse.tx.HostPunchNotification",
        ),
        MetaType::HostUpdateNotificationAck => (
            "lighthouse.rx.HostUpdateNotificationAck",
            "lighthouse.tx.HostUpdateNotificationAck",
        ),
        MetaType::HostMovedNotification | MetaType::Other(_) => {
            ("lighthouse.rx.other", "lighthouse.tx.other")
        }
    };
    if rx { par.0 } else { par.1 }
}

/// As linhas do Graphite (`prefixo.nome.sufixo valor instante`), como o `graphiteOnce` do
/// Nebula.
#[allow(clippy::cast_precision_loss)]
fn graphite_lines(prefixo: &str, intervalo: Duration, agora: i64, amostras: &[Sample]) -> String {
    let segundos = intervalo.as_secs_f64();
    let mut texto = String::new();
    for amostra in amostras {
        let nome = amostra.name;
        match &amostra.value {
            Value::Counter(n) => {
                let _ = writeln!(texto, "{prefixo}.{nome}.count {n} {agora}");
                let por_segundo = *n as f64 / segundos;
                let _ = writeln!(texto, "{prefixo}.{nome}.count_ps {por_segundo:.2} {agora}");
            }
            Value::Gauge(v) => {
                let _ = writeln!(texto, "{prefixo}.{nome}.value {v} {agora}");
            }
            Value::Histogram(h) => {
                let _ = writeln!(texto, "{prefixo}.{nome}.count {} {agora}", h.count);
                let _ = writeln!(texto, "{prefixo}.{nome}.min {} {agora}", h.min());
                let _ = writeln!(texto, "{prefixo}.{nome}.max {} {agora}", h.max());
                let _ = writeln!(texto, "{prefixo}.{nome}.mean {:.2} {agora}", h.mean());
                let _ = writeln!(texto, "{prefixo}.{nome}.std-dev {:.2} {agora}", h.std_dev());
                for (p, chave) in GRAPHITE_PERCENTILES {
                    let _ = writeln!(
                        texto,
                        "{prefixo}.{nome}.{chave}-percentile {:.2} {agora}",
                        h.percentile(p)
                    );
                }
            }
        }
    }
    texto
}

/// O nome do Prometheus: `namespace_subsystem_nome`, sem as partes vazias, com ` `, `.`, `-`
/// e `=` trocados por `_` (o `flattenKey` e o `BuildFQName`).
fn prometheus_name(namespace: &str, subsystem: &str, nome: &str) -> String {
    let achatar = |s: &str| s.replace([' ', '.', '-', '='], "_");
    [namespace, subsystem, nome]
        .into_iter()
        .filter(|parte| !parte.is_empty())
        .map(achatar)
        .collect::<Vec<_>>()
        .join("_")
}

/// Um número no formato do texto do Prometheus.
#[allow(clippy::cast_possible_truncation)]
fn prometheus_number(v: f64) -> String {
    if v.is_nan() {
        "NaN".to_owned()
    } else if v.is_infinite() {
        if v > 0.0 { "+Inf" } else { "-Inf" }.to_owned()
    } else if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

/// Uma família do Prometheus: o nome, a ajuda e os valores (com o rótulo `type`, se há).
type Familia = (String, String, Vec<(Option<&'static str>, f64)>);

/// O texto que o Prometheus lê (formato 0.0.4): uma família por métrica, todas `gauge`, em
/// ordem de nome, como o registro do Prometheus do Nebula entrega. O histograma vira um
/// medidor com o rótulo `type`; os percentis são os de verdade (o provedor do Go pede 75 em
/// vez de 0,75 e devolve sempre o máximo).
#[allow(clippy::cast_precision_loss)]
fn prometheus_text(namespace: &str, subsystem: &str, amostras: &[Sample]) -> String {
    let mut familias: Vec<Familia> = Vec::new();
    familias.push((
        prometheus_name(namespace, subsystem, "info"),
        "Version information for the Nebula binary".to_owned(),
        vec![(None, 1.0)],
    ));
    for amostra in amostras {
        let valores = match &amostra.value {
            Value::Counter(n) => vec![(None, *n as f64)],
            Value::Gauge(v) => vec![(None, *v as f64)],
            Value::Histogram(h) => vec![
                (Some("count"), h.count as f64),
                (Some("max"), h.max() as f64),
                (Some("mean"), h.mean()),
                (Some("min"), h.min() as f64),
                (Some("perc75"), h.percentile(0.75)),
                (Some("perc95"), h.percentile(0.95)),
                (Some("perc99"), h.percentile(0.99)),
                (Some("perc999"), h.percentile(0.999)),
                (Some("stddev"), h.std_dev()),
                (Some("sum"), h.sum() as f64),
                (Some("variance"), h.variance()),
            ],
        };
        familias.push((
            prometheus_name(namespace, subsystem, amostra.name),
            amostra.name.to_owned(),
            valores,
        ));
    }
    familias.sort_by(|a, b| a.0.cmp(&b.0));
    let versao = env!("CARGO_PKG_VERSION");
    let info = prometheus_name(namespace, subsystem, "info");
    let mut texto = String::new();
    for (nome, ajuda, valores) in familias {
        let ajuda = ajuda.replace('\\', "\\\\").replace('\n', "\\n");
        let _ = writeln!(texto, "# HELP {nome} {ajuda}");
        let _ = writeln!(texto, "# TYPE {nome} gauge");
        for (rotulo, valor) in valores {
            let valor = prometheus_number(valor);
            match rotulo {
                Some(tipo) => {
                    let _ = writeln!(texto, "{nome}{{type=\"{tipo}\"}} {valor}");
                }
                None if nome == info => {
                    let _ = writeln!(texto, "{nome}{{version=\"{versao}\"}} {valor}");
                }
                None => {
                    let _ = writeln!(texto, "{nome} {valor}");
                }
            }
        }
    }
    texto
}

/// Pede as métricas ao motor. `None` se ele parou.
async fn ask(commands: &mpsc::UnboundedSender<Command>) -> Option<Vec<Sample>> {
    let (resposta, espera) = oneshot::channel();
    commands.send(Command::Metrics(resposta)).ok()?;
    espera.await.ok()
}

/// Abre o HTTP do Prometheus. Um `listen` sem host (`:8080`) escuta em todos os endereços,
/// como no Go: no IPv6 e IPv4 juntos, ou só no IPv4 onde não há IPv6.
pub(crate) async fn bind_prometheus(listen: &str) -> io::Result<TcpListener> {
    if let Some(porta) = listen.strip_prefix(':') {
        let porta: u16 = porta.parse().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("stats.listen: a porta de {listen} não é válida"),
            )
        })?;
        return match TcpListener::bind(SocketAddr::from(([0u16; 8], porta))).await {
            Ok(escuta) => Ok(escuta),
            Err(_) => TcpListener::bind(SocketAddr::from(([0u8; 4], porta))).await,
        };
    }
    TcpListener::bind(listen).await
}

/// Põe para rodar o que entrega as métricas: o envio periódico ao Graphite, ou o HTTP do
/// Prometheus em `escuta` (já aberto; sem ele, não há o que rodar). Acaba quando o motor
/// para.
pub(crate) fn spawn_exporter(
    stats: Stats,
    escuta: Option<TcpListener>,
    commands: mpsc::UnboundedSender<Command>,
) -> Option<JoinHandle<()>> {
    match (stats.export, escuta) {
        (
            StatsExport::Prometheus {
                path,
                namespace,
                subsystem,
                ..
            },
            Some(escuta),
        ) => {
            let contexto = Arc::new(Prometheus {
                path,
                namespace,
                subsystem,
                commands,
            });
            Some(tokio::spawn(serve_prometheus(escuta, contexto)))
        }
        (
            StatsExport::Graphite {
                protocol,
                host,
                prefix,
            },
            _,
        ) => Some(tokio::spawn(graphite_loop(
            protocol,
            host,
            prefix,
            stats.interval,
            commands,
        ))),
        (StatsExport::Prometheus { .. }, None) => None,
    }
}

/// Manda as métricas ao Graphite ao começar e a cada `intervalo`. Uma falha (o Graphite fora
/// do ar) perde só aquele envio, como no Nebula.
async fn graphite_loop(
    protocolo: GraphiteProtocol,
    host: String,
    prefixo: String,
    intervalo: Duration,
    commands: mpsc::UnboundedSender<Command>,
) {
    let mut relogio = tokio::time::interval(intervalo);
    relogio.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        relogio.tick().await;
        let Some(amostras) = ask(&commands).await else {
            return;
        };
        let linhas = graphite_lines(&prefixo, intervalo, now_unix(), &amostras);
        let _ = tokio::time::timeout(
            GRAPHITE_TIMEOUT,
            send_graphite(protocolo, &host, linhas.as_bytes()),
        )
        .await;
    }
}

async fn send_graphite(protocolo: GraphiteProtocol, host: &str, linhas: &[u8]) -> io::Result<()> {
    let enderecos = tokio::net::lookup_host(host)
        .await?
        .filter(|a| match protocolo {
            GraphiteProtocol::Tcp => true,
            GraphiteProtocol::Tcp4 => a.is_ipv4(),
            GraphiteProtocol::Tcp6 => a.is_ipv6(),
        });
    let mut ultimo = io::Error::new(
        io::ErrorKind::NotFound,
        format!("{host} não resolveu para nenhum endereço"),
    );
    for addr in enderecos {
        match TcpStream::connect(addr).await {
            Ok(mut conexao) => {
                conexao.write_all(linhas).await?;
                conexao.shutdown().await?;
                return Ok(());
            }
            Err(erro) => ultimo = erro,
        }
    }
    Err(ultimo)
}

/// O que o HTTP do Prometheus precisa para responder.
struct Prometheus {
    path: String,
    namespace: String,
    subsystem: String,
    commands: mpsc::UnboundedSender<Command>,
}

async fn serve_prometheus(escuta: TcpListener, contexto: Arc<Prometheus>) {
    loop {
        let Ok((conexao, _)) = escuta.accept().await else {
            // Falta de descritores e afins: espera um pouco e segue.
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
        };
        if contexto.commands.is_closed() {
            return;
        }
        let contexto = contexto.clone();
        tokio::spawn(async move {
            let _ = tokio::time::timeout(HTTP_TIMEOUT, answer(conexao, &contexto)).await;
        });
    }
}

/// Lê um pedido HTTP/1.x e responde com as métricas (no caminho certo) ou 404, e fecha.
async fn answer(mut conexao: TcpStream, contexto: &Prometheus) -> io::Result<()> {
    let mut pedido = Vec::new();
    let mut buf = [0u8; 2048];
    loop {
        let n = conexao.read(&mut buf).await?;
        if n == 0 {
            return Ok(());
        }
        pedido.extend_from_slice(&buf[..n]);
        if pedido.windows(4).any(|j| j == b"\r\n\r\n") || pedido.windows(2).any(|j| j == b"\n\n") {
            break;
        }
        if pedido.len() > HTTP_MAX_REQUEST {
            return respond(
                &mut conexao,
                false,
                "431 Request Header Fields Too Large",
                "",
            )
            .await;
        }
    }
    let linha = String::from_utf8_lossy(&pedido);
    let mut partes = linha.lines().next().unwrap_or_default().split_whitespace();
    let (Some(metodo), Some(alvo)) = (partes.next(), partes.next()) else {
        return respond(&mut conexao, false, "400 Bad Request", "").await;
    };
    let cabeca = metodo == "HEAD";
    let caminho = alvo.split(['?', '#']).next().unwrap_or_default();
    if !path_matches(&contexto.path, caminho) {
        return respond(
            &mut conexao,
            cabeca,
            "404 Not Found",
            "404 page not found\n",
        )
        .await;
    }
    let Some(amostras) = ask(&contexto.commands).await else {
        return respond(&mut conexao, cabeca, "503 Service Unavailable", "").await;
    };
    let texto = prometheus_text(&contexto.namespace, &contexto.subsystem, &amostras);
    let corpo = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain; version=0.0.4; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        texto.len()
    );
    conexao.write_all(corpo.as_bytes()).await?;
    if !cabeca {
        conexao.write_all(texto.as_bytes()).await?;
    }
    conexao.shutdown().await
}

async fn respond(
    conexao: &mut TcpStream,
    cabeca: bool,
    status: &str,
    corpo: &str,
) -> io::Result<()> {
    let topo = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        corpo.len()
    );
    conexao.write_all(topo.as_bytes()).await?;
    if !cabeca {
        conexao.write_all(corpo.as_bytes()).await?;
    }
    conexao.shutdown().await
}

/// O caminho do pedido serve ao `stats.path`, como no `ServeMux` do Go: igual, ou dentro dele
/// quando ele termina em `/`.
fn path_matches(padrao: &str, caminho: &str) -> bool {
    caminho == padrao || (padrao.ends_with('/') && caminho.starts_with(padrao))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(message: bool, lighthouse: bool) -> Stats {
        Stats {
            interval: Duration::from_secs(10),
            export: StatsExport::Graphite {
                protocol: GraphiteProtocol::Tcp,
                host: "127.0.0.1:2003".to_owned(),
                prefix: "nebula".to_owned(),
            },
            message_metrics: message,
            lighthouse_metrics: lighthouse,
        }
    }

    fn contador(m: &Metrics, nome: &str) -> Option<u64> {
        m.snapshot().into_iter().find_map(|a| match a.value {
            Value::Counter(n) if a.name == nome => Some(n),
            _ => None,
        })
    }

    #[test]
    fn sem_stats_nada_e_guardado() {
        let m = Metrics::new(None);
        m.inc(HANDSHAKE_INITIATED);
        m.tx(MessageType::RecvError, 0);
        m.handshake_done(Duration::from_millis(5));
        assert!(m.snapshot().is_empty());
    }

    #[test]
    fn sem_message_metrics_so_o_recv_error_conta() {
        let m = Metrics::new(Some(&stats(false, false)));
        m.tx(MessageType::RecvError, 0);
        m.rx(MessageType::RecvError, 0);
        m.rx(MessageType::RecvError, 0);
        m.tx(MessageType::LightHouse, 0);
        m.rx(MessageType::Control, 0);
        m.inc(RX_INVALID);
        m.lighthouse_tx(MetaType::HostQuery, 1);
        assert_eq!(contador(&m, "messages.tx.recv_error"), Some(1));
        assert_eq!(contador(&m, "messages.rx.recv_error"), Some(2));
        assert_eq!(contador(&m, "messages.tx.lighthouse"), None);
        assert_eq!(contador(&m, "messages.rx.other"), None);
        assert_eq!(contador(&m, RX_INVALID), None);
        assert_eq!(contador(&m, "lighthouse.tx.HostQuery"), None);
        assert_eq!(contador(&m, HANDSHAKE_INITIATED), Some(0));
    }

    #[test]
    fn com_message_metrics_cada_tipo_tem_o_seu_nome() {
        let m = Metrics::new(Some(&stats(true, true)));
        m.tx(MessageType::Handshake, header::HANDSHAKE_IX_PSK0);
        m.tx(MessageType::Test, header::TEST_REQUEST);
        m.rx(MessageType::Test, header::TEST_REPLY);
        m.rx(MessageType::Control, 0);
        m.tx(MessageType::Message, 0);
        m.rx(MessageType::Message, header::MESSAGE_RELAY);
        m.lighthouse_tx(MetaType::HostQuery, 2);
        m.lighthouse_rx(MetaType::HostMovedNotification);
        assert_eq!(contador(&m, "messages.tx.handshake_ixpsk0"), Some(1));
        assert_eq!(contador(&m, "messages.tx.test_request"), Some(1));
        assert_eq!(contador(&m, "messages.rx.test_response"), Some(1));
        assert_eq!(contador(&m, "messages.rx.other"), Some(1));
        assert_eq!(contador(&m, "messages.tx.other"), Some(0));
        assert_eq!(contador(&m, "lighthouse.tx.HostQuery"), Some(2));
        assert_eq!(contador(&m, "lighthouse.rx.other"), Some(1));
    }

    #[test]
    fn o_histograma_guarda_as_ultimas_e_conta_todas() {
        let m = Metrics::new(Some(&stats(false, false)));
        for i in 0..(HISTOGRAM_SAMPLE as u64 + 10) {
            m.handshake_done(Duration::from_nanos(i));
        }
        let h = m
            .snapshot()
            .into_iter()
            .find_map(|a| match a.value {
                Value::Histogram(h) => Some(h),
                _ => None,
            })
            .unwrap();
        assert_eq!(h.count, HISTOGRAM_SAMPLE as u64 + 10);
        assert_eq!(h.sorted.len(), HISTOGRAM_SAMPLE);
        assert_eq!(h.min(), 10);
    }

    #[test]
    fn percentis_e_desvio_como_no_go_metrics() {
        let h = HistogramSnapshot {
            count: 4,
            sorted: vec![10, 20, 30, 40],
        };
        // pos = p * (n + 1): 0,5 * 5 = 2,5 → entre o 2º e o 3º.
        assert!((h.percentile(0.5) - 25.0).abs() < 1e-9);
        assert!((h.percentile(0.1) - 10.0).abs() < 1e-9);
        assert!((h.percentile(0.999) - 40.0).abs() < 1e-9);
        assert!((h.mean() - 25.0).abs() < 1e-9);
        assert!((h.variance() - 125.0).abs() < 1e-9);
        assert_eq!(h.sum(), 100);
        let vazio = HistogramSnapshot::default();
        assert!(vazio.percentile(0.5).abs() < f64::EPSILON);
        assert!(vazio.std_dev().abs() < f64::EPSILON);
    }

    #[test]
    fn linhas_do_graphite() {
        let amostras = vec![
            Sample {
                name: "handshake_manager.initiated",
                value: Value::Counter(5),
            },
            Sample {
                name: "hostmap.main.hosts",
                value: Value::Gauge(2),
            },
            Sample {
                name: "handshakes",
                value: Value::Histogram(HistogramSnapshot {
                    count: 2,
                    sorted: vec![100, 300],
                }),
            },
        ];
        let texto = graphite_lines("nebula", Duration::from_secs(10), 1_700_000_000, &amostras);
        let linhas: Vec<&str> = texto.lines().collect();
        assert_eq!(
            linhas[..3],
            [
                "nebula.handshake_manager.initiated.count 5 1700000000",
                "nebula.handshake_manager.initiated.count_ps 0.50 1700000000",
                "nebula.hostmap.main.hosts.value 2 1700000000",
            ]
        );
        assert!(linhas.contains(&"nebula.handshakes.count 2 1700000000"));
        assert!(linhas.contains(&"nebula.handshakes.mean 200.00 1700000000"));
        assert!(linhas.contains(&"nebula.handshakes.std-dev 100.00 1700000000"));
        assert!(linhas.contains(&"nebula.handshakes.999-percentile 300.00 1700000000"));
        assert!(linhas.contains(&"nebula.handshakes.50-percentile 200.00 1700000000"));
        assert_eq!(linhas.len(), 3 + 10);
    }

    #[test]
    fn texto_do_prometheus() {
        let amostras = vec![
            Sample {
                name: "messages.tx.recv_error",
                value: Value::Counter(3),
            },
            Sample {
                name: "handshakes",
                value: Value::Histogram(HistogramSnapshot {
                    count: 1,
                    sorted: vec![1500],
                }),
            },
        ];
        let texto = prometheus_text("nebula", "", &amostras);
        let versao = env!("CARGO_PKG_VERSION");
        assert!(texto.starts_with(
            "# HELP nebula_handshakes handshakes\n# TYPE nebula_handshakes gauge\n\
             nebula_handshakes{type=\"count\"} 1\nnebula_handshakes{type=\"max\"} 1500\n"
        ));
        assert!(texto.contains(&format!(
            "# HELP nebula_info Version information for the Nebula binary\n\
             # TYPE nebula_info gauge\nnebula_info{{version=\"{versao}\"}} 1\n"
        )));
        assert!(texto.ends_with(
            "# HELP nebula_messages_tx_recv_error messages.tx.recv_error\n\
             # TYPE nebula_messages_tx_recv_error gauge\nnebula_messages_tx_recv_error 3\n"
        ));
        assert_eq!(prometheus_name("", "sub-x", "a.b c=d"), "sub_x_a_b_c_d");
        assert_eq!(prometheus_number(0.5), "0.5");
        assert_eq!(prometheus_number(f64::INFINITY), "+Inf");
    }

    #[test]
    fn caminho_como_o_servemux() {
        assert!(path_matches("/metrics", "/metrics"));
        assert!(!path_matches("/metrics", "/metrics/x"));
        assert!(!path_matches("/metrics", "/"));
        assert!(path_matches("/", "/qualquer"));
        assert!(path_matches("/m/", "/m/x"));
    }
}
