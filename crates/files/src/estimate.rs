//! A estimativa de tempo da fila de transferências (M40): um custo fixo por arquivo mais os
//! bytes pela vazão, medidos **por rota** (tipo e pontas), cada um com média e variância
//! exponenciais, e o resultado como uma faixa provável (p10, p50 e p90).
//!
//! O [`Estimator`] não lê relógio nenhum: quem o chama passa os intervalos medidos (e o
//! instante, no [`Estimator::tick`]). Por isso ele é determinístico nos testes. A [`Transfers`](crate::Transfers)
//! (`crate::Transfers`) o alimenta com o [`Clock`] dela.
//!
//! ```
//! use tomni_ui_files::TransferKind;
//! use tomni_ui_files::estimate::{Estimator, Route, Work};
//!
//! let rota = Route { kind: TransferKind::Upload, from: 0, to: 1 };
//! let mut estimador = Estimator::new();
//! // 3 s a 10 MiB/s, em trechos de 0,1 s.
//! for _ in 0..30 {
//!     estimador.bytes(rota, 1 << 20, 0.1);
//! }
//! // Falta um arquivo de 100 MiB, já na fase dos bytes.
//! let trabalho = [Work { route: rota, files: 0, bytes: 100 << 20 }];
//! let eta = estimador.estimate(&trabalho, 1).unwrap();
//! assert!((eta.likely.as_secs_f64() - 10.0).abs() < 0.5);
//! assert!(eta.low < eta.likely && eta.likely < eta.high);
//! ```

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::transfers::TransferKind;

/// O tempo da fila: um instante monotônico desde uma origem qualquer.
pub trait Clock: Send + Sync + 'static {
    /// Quanto tempo se passou desde a origem do relógio.
    fn now(&self) -> Duration;
}

/// O relógio de verdade, sobre o [`Instant`].
#[derive(Debug)]
pub struct SystemClock {
    origin: Instant,
}

impl SystemClock {
    /// Um relógio que começa em zero agora.
    pub fn new() -> SystemClock {
        SystemClock {
            origin: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        SystemClock::new()
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }
}

/// Um relógio que só anda quando o teste manda ([`advance`](Self::advance), [`set`](Self::set)).
#[derive(Debug, Default)]
pub struct ManualClock {
    now: Mutex<Duration>,
}

impl ManualClock {
    /// Um relógio parado em zero.
    pub fn new() -> ManualClock {
        ManualClock::default()
    }

    /// Anda `by` para a frente.
    pub fn advance(&self, by: Duration) {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner) += by;
    }

    /// Põe o relógio em `to`.
    pub fn set(&self, to: Duration) {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner) = to;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Duration {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A constante de tempo da vazão por rota, em segundos: um trecho longo pesa mais que um curto.
const TAU_VAZAO: f64 = 10.0;
/// Quantos arquivos formam a "memória" do custo fixo.
const N_CUSTO: f64 = 20.0;
/// A incerteza do modelo ("o futuro não é o passado"): a faixa nunca fica mais estreita que isso.
const KAPPA: f64 = 0.10;
/// O z do percentil 90 da normal.
const Z90: f64 = 1.2816;
/// Quantos segundos de bytes a rota precisa ter medido para a vazão valer.
const MINIMO_MEDIDO: f64 = 2.0;
/// Quantas amostras de custo fixo a rota precisa ter para valer.
const MINIMO_CUSTOS: u32 = 3;
/// Até quantos arquivos por começar se estima sem nenhuma amostra de custo fixo.
const ARQUIVOS_SEM_CUSTO: u64 = 3;
/// A constante de tempo da velocidade da fila inteira, em segundos.
const TAU_FILA: f64 = 3.0;

/// Uma média e uma variância com esquecimento exponencial (Finch, 2009).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Ewma {
    /// A média.
    pub mean: f64,
    /// A variância.
    pub var: f64,
    /// Quantas amostras entraram.
    pub n: u32,
}

impl Ewma {
    /// Junta a amostra `x` com o peso `a`, de 0 (nada) a 1 (só ela).
    pub fn push(&mut self, x: f64, a: f64) {
        if self.n == 0 {
            self.mean = x;
            self.var = 0.0;
        } else {
            let d = x - self.mean;
            self.mean += a * d;
            self.var = (1.0 - a) * (self.var + a * d * d);
        }
        self.n = self.n.saturating_add(1);
    }

    /// O desvio padrão.
    pub fn sd(&self) -> f64 {
        self.var.max(0.0).sqrt()
    }
}

/// De onde para onde uma transferência anda: a vazão e o custo de uma rota não se misturam
/// com os de outra. `from` e `to` são a identidade das pontas (0 é o disco desta máquina).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Route {
    /// O tipo da transferência.
    pub kind: TransferKind,
    /// A identidade da origem.
    pub from: u64,
    /// A identidade do destino.
    pub to: u64,
}

/// O que falta fazer de um item: quantos arquivos por começar e quantos bytes por mover.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Work {
    /// A rota do item.
    pub route: Route,
    /// 1 se o item ainda não chegou à fase dos bytes (ou é uma exclusão), 0 se já chegou.
    pub files: u64,
    /// Os bytes que faltam.
    pub bytes: u64,
}

/// O tempo restante como uma faixa provável.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Eta {
    /// O percentil 10: com 90% de chance leva mais que isso.
    pub low: Duration,
    /// A mediana.
    pub likely: Duration,
    /// O percentil 90: com 90% de chance leva menos que isso.
    pub high: Duration,
}

#[derive(Clone, Copy, Debug, Default)]
struct RouteStats {
    /// A vazão de um trabalhador, em bytes por segundo.
    speed: Ewma,
    /// Os segundos de bytes que a rota já mediu.
    measured: f64,
    /// O custo fixo de um arquivo, em segundos.
    overhead: Ewma,
}

/// O que a fila aprendeu das transferências, por rota.
#[derive(Debug, Default)]
pub struct Estimator {
    routes: HashMap<Route, RouteStats>,
    queue_speed: Ewma,
    queue_base: Option<(Duration, u64)>,
    /// Os segundos que a velocidade da fila já mediu desde o começo da leva.
    queue_seconds: f64,
}

impl Estimator {
    /// Um estimador que ainda não sabe nada.
    pub fn new() -> Estimator {
        Estimator::default()
    }

    /// Uma medida de vazão: `bytes` movidos em `seconds` de trabalho (sem pausas nem esperas).
    /// Com `seconds == 0` não há amostra.
    pub fn bytes(&mut self, route: Route, bytes: u64, seconds: f64) {
        if seconds.is_nan() || seconds <= 0.0 {
            return;
        }
        let stats = self.routes.entry(route).or_default();
        let a = 1.0 - (-seconds / TAU_VAZAO).exp();
        stats.speed.push(bytes as f64 / seconds, a);
        stats.measured += seconds;
    }

    /// Um arquivo terminou: `cost` segundos de custo fixo (o tempo ativo menos o dos bytes).
    /// Só se chama com os arquivos que terminaram bem.
    pub fn file_done(&mut self, route: Route, cost: f64) {
        let stats = self.routes.entry(route).or_default();
        let a = 1.0 - (-1.0 / N_CUSTO).exp();
        stats.overhead.push(cost.max(0.0), a);
    }

    /// A velocidade da fila inteira: a cada chamada entra a amostra dos bytes feitos desde a
    /// anterior, por tempo de relógio. Com `active == false` (fila pausada ou esperando) a base
    /// é zerada e não há amostra, então a pausa não puxa a velocidade para baixo.
    pub fn tick(&mut self, now: Duration, bytes_done: u64, active: bool) -> f64 {
        if !active {
            self.queue_base = None;
            return self.queue_speed.mean;
        }
        if let Some((before, bytes_before)) = self.queue_base {
            let seconds = now.saturating_sub(before).as_secs_f64();
            if seconds > 0.0 {
                let moved = bytes_done.saturating_sub(bytes_before);
                // Nos primeiros segundos a média é a de tudo o que se mediu (peso pelo tempo),
                // e não a de uma exponencial que ainda parte do zero.
                let a = (1.0 - (-seconds / TAU_FILA).exp())
                    .max(seconds / (self.queue_seconds + seconds));
                self.queue_seconds += seconds;
                self.queue_speed.push(moved as f64 / seconds, a);
            } else {
                return self.queue_speed.mean;
            }
        }
        self.queue_base = Some((now, bytes_done));
        self.queue_speed.mean
    }

    /// Esquece a velocidade da fila (uma leva nova começou).
    pub fn reset_queue_speed(&mut self) {
        self.queue_speed = Ewma::default();
        self.queue_base = None;
        self.queue_seconds = 0.0;
    }

    /// A velocidade da fila inteira, em bytes por segundo.
    pub fn queue_speed(&self) -> f64 {
        self.queue_speed.mean
    }

    /// O tempo restante de `work` (um item por entrada) com até `parallel` trabalhadores.
    /// `None` quando ainda não se mediu o bastante ou não falta nada.
    pub fn estimate(&self, work: &[Work], parallel: usize) -> Option<Eta> {
        if work.is_empty() {
            return None;
        }
        let mut routes: HashMap<Route, (u64, u64)> = HashMap::new();
        for item in work {
            let total = routes.entry(item.route).or_default();
            total.0 += item.files;
            total.1 += item.bytes;
        }
        let mut seconds = 0.0;
        let mut variance = 0.0;
        for (route, (files, bytes)) in &routes {
            let stats = self.routes.get(route).copied().unwrap_or_default();
            let cost = stats.overhead.mean;
            if *files > ARQUIVOS_SEM_CUSTO && stats.overhead.n < MINIMO_CUSTOS {
                return None;
            }
            let fixed = *files as f64 * cost;
            let var_fixed = *files as f64 * stats.overhead.var + (KAPPA * fixed).powi(2);
            let (moved, var_moved) = if *bytes > 0 {
                if stats.speed.n == 0 || stats.speed.mean <= 0.0 {
                    return None;
                }
                if stats.measured < MINIMO_MEDIDO {
                    return None;
                }
                let moved = *bytes as f64 / stats.speed.mean;
                let k = (moved / TAU_VAZAO).max(1.0);
                let relative = stats.speed.sd() / stats.speed.mean;
                (
                    moved,
                    moved * moved * (relative * relative / k + KAPPA * KAPPA),
                )
            } else {
                (0.0, 0.0)
            };
            seconds += fixed + moved;
            variance += var_fixed + var_moved;
        }
        let workers = parallel.min(work.len()).max(1) as f64;
        let longest = work
            .iter()
            .map(|item| self.item_seconds(item))
            .fold(0.0, f64::max);
        let likely = (seconds / workers).max(longest);
        if likely.is_nan() || likely <= 0.0 {
            return None;
        }
        let sd = variance.sqrt() / workers;
        let ratio = sd / likely;
        let sigma = (1.0 + ratio * ratio).ln().sqrt();
        Some(Eta {
            low: seconds_to_duration(likely * (-Z90 * sigma).exp()),
            likely: seconds_to_duration(likely),
            high: seconds_to_duration(likely * (Z90 * sigma).exp()),
        })
    }

    /// Os segundos de um item sozinho, com as médias da rota dele.
    fn item_seconds(&self, item: &Work) -> f64 {
        let stats = self.routes.get(&item.route).copied().unwrap_or_default();
        let moved = if item.bytes > 0 && stats.speed.mean > 0.0 {
            item.bytes as f64 / stats.speed.mean
        } else {
            0.0
        };
        item.files as f64 * stats.overhead.mean + moved
    }
}

fn seconds_to_duration(seconds: f64) -> Duration {
    Duration::try_from_secs_f64(seconds.max(0.0)).unwrap_or(Duration::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_media_de_uma_constante_e_ela_mesma() {
        let mut ewma = Ewma::default();
        for _ in 0..50 {
            ewma.push(7.0, 0.2);
        }
        assert!((ewma.mean - 7.0).abs() < 1e-12);
        assert!(ewma.var.abs() < 1e-12);
        assert_eq!(ewma.n, 50);
    }

    #[test]
    fn a_variancia_de_amostras_alternadas_converge_perto_de_d_ao_quadrado() {
        let mut ewma = Ewma::default();
        for i in 0..2000 {
            ewma.push(if i % 2 == 0 { 10.0 + 3.0 } else { 10.0 - 3.0 }, 0.05);
        }
        assert!((ewma.mean - 10.0).abs() < 0.5, "média {}", ewma.mean);
        assert!(ewma.var > 6.0 && ewma.var < 10.0, "variância {}", ewma.var);
    }

    #[test]
    fn o_relogio_manual_anda_e_pula() {
        let relogio = ManualClock::new();
        assert_eq!(relogio.now(), Duration::ZERO);
        relogio.advance(Duration::from_secs(3));
        relogio.advance(Duration::from_millis(500));
        assert_eq!(relogio.now(), Duration::from_millis(3500));
        relogio.set(Duration::from_secs(1));
        assert_eq!(relogio.now(), Duration::from_secs(1));
    }

    #[test]
    fn o_relogio_do_sistema_so_anda_para_a_frente() {
        let relogio = SystemClock::new();
        let antes = relogio.now();
        std::thread::sleep(Duration::from_millis(5));
        assert!(relogio.now() > antes);
    }

    #[test]
    fn sem_amostra_o_intervalo_zero_nao_conta() {
        let rota = Route {
            kind: TransferKind::Copy,
            from: 0,
            to: 0,
        };
        let mut estimador = Estimator::new();
        estimador.bytes(rota, 1000, 0.0);
        assert!(estimador.routes.is_empty());
    }
}
