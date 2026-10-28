//! Em que etapa a thread da interface está, e quanto tempo ficou em cada uma: a base do
//! registro de travamentos (`WindowOptions::stall_log` da plataforma).
//!
//! A plataforma marca as etapas grandes (um evento, o quadro, o desenho, a textura da janela),
//! a fachada e os widgets marcam as de dentro (os efeitos, o layout, os terminais). Uma thread
//! vigia lê [`current`] para dizer onde a interface parou e [`take_totals`] para dizer onde o
//! tempo foi. Desligado (o padrão), [`stage`] é o teste de um `bool`.
//!
//! ```
//! use tomni_ui_core::stages;
//!
//! stages::enable();
//! {
//!     let _quadro = stages::stage("quadro");
//!     let _layout = stages::stage("layout");
//!     assert_eq!(stages::current().unwrap().path, "quadro › layout");
//! }
//! assert!(stages::current().is_none());
//! ```

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

static ENABLED: AtomicBool = AtomicBool::new(false);
static STATE: Mutex<State> = Mutex::new(State::new());

struct State {
    /// As etapas abertas, da de fora para a de dentro.
    stack: Vec<&'static str>,
    /// Quando a etapa de fora começou.
    entered: Option<Instant>,
    /// A última troca de etapa: o tempo até a próxima vai para a de dentro.
    since: Option<Instant>,
    /// Quantas etapas de fora já começaram: diz à vigia se é a mesma parada.
    epoch: u64,
    totals: Vec<Total>,
    /// A etapa de fora mais demorada desde o último [`take_totals`].
    slowest: Option<(Duration, String)>,
}

impl State {
    const fn new() -> Self {
        Self {
            stack: Vec::new(),
            entered: None,
            since: None,
            epoch: 0,
            totals: Vec::new(),
            slowest: None,
        }
    }

    /// Soma o tempo desde a última troca à etapa de dentro.
    fn charge(&mut self, now: Instant) {
        let Some(since) = self.since.replace(now) else {
            return;
        };
        if self.stack.is_empty() {
            return;
        }
        let spent = now.saturating_duration_since(since);
        match self.totals.iter_mut().find(|t| t.stack == self.stack) {
            Some(total) => total.time += spent,
            None => self.totals.push(Total {
                stack: self.stack.clone(),
                time: spent,
                count: 0,
            }),
        }
    }

    fn count(&mut self) {
        if let Some(total) = self.totals.iter_mut().find(|t| t.stack == self.stack) {
            total.count += 1;
        } else {
            self.totals.push(Total {
                stack: self.stack.clone(),
                time: Duration::ZERO,
                count: 1,
            });
        }
    }
}

struct Total {
    stack: Vec<&'static str>,
    time: Duration,
    count: u32,
}

fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(PoisonError::into_inner)
}

fn path(stack: &[&'static str]) -> String {
    stack.join(" › ")
}

/// Liga a marcação das etapas, para o processo todo.
pub fn enable() {
    ENABLED.store(true, Ordering::Release);
}

/// Se a marcação está ligada.
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Acquire)
}

/// Entra na etapa `name` até o guarda sair de escopo. Dentro de outra, vira uma etapa dela
/// (`quadro › layout`). Só a thread da interface marca.
#[must_use = "a etapa acaba quando o guarda sai de escopo"]
pub fn stage(name: &'static str) -> StageGuard {
    if !enabled() {
        return StageGuard { open: false };
    }
    let now = Instant::now();
    let mut state = state();
    state.charge(now);
    if state.stack.is_empty() {
        state.entered = Some(now);
        state.epoch += 1;
    }
    state.stack.push(name);
    state.count();
    StageGuard { open: true }
}

/// O guarda de [`stage`]: sai da etapa no `drop`.
#[derive(Debug)]
pub struct StageGuard {
    open: bool,
}

impl Drop for StageGuard {
    fn drop(&mut self) {
        if !self.open {
            return;
        }
        let now = Instant::now();
        let mut state = state();
        state.charge(now);
        if state.stack.len() == 1 {
            let took = state.entered.take().map_or(Duration::ZERO, |entered| {
                now.saturating_duration_since(entered)
            });
            if state
                .slowest
                .as_ref()
                .is_none_or(|(slowest, _)| took > *slowest)
            {
                let name = path(&state.stack);
                state.slowest = Some((took, name));
            }
        }
        state.stack.pop();
    }
}

/// Onde a thread da interface está agora.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Current {
    /// As etapas abertas, separadas por ` › `.
    pub path: String,
    /// Há quanto tempo a etapa de fora começou.
    pub elapsed: Duration,
    /// Qual etapa de fora é esta: muda a cada uma que começa.
    pub epoch: u64,
}

/// A etapa aberta agora, ou `None` fora de todas (o loop esperando ou no sistema).
pub fn current() -> Option<Current> {
    let state = state();
    let entered = state.entered?;
    Some(Current {
        path: path(&state.stack),
        elapsed: entered.elapsed(),
        epoch: state.epoch,
    })
}

/// O tempo de uma etapa, sem o das de dentro dela.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Totals {
    /// Cada etapa, com o tempo e quantas vezes começou, da que mais gastou para a que menos.
    pub stages: Vec<(String, Duration, u32)>,
    /// A etapa de fora mais demorada, com quanto durou.
    pub slowest: Option<(String, Duration)>,
}

/// O tempo de cada etapa desde a última chamada, e zera a conta.
pub fn take_totals() -> Totals {
    let mut state = state();
    state.charge(Instant::now());
    let mut stages: Vec<_> = std::mem::take(&mut state.totals)
        .into_iter()
        .map(|t| (path(&t.stack), t.time, t.count))
        .collect();
    stages.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let slowest = state.slowest.take().map(|(took, name)| (name, took));
    Totals { stages, slowest }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soma_o_tempo_na_etapa_de_dentro_e_acha_a_mais_lenta() {
        enable();
        let _ = take_totals();
        {
            let _quadro = stage("quadro");
            std::thread::sleep(Duration::from_millis(20));
            let atual = current().unwrap();
            assert_eq!(atual.path, "quadro");
            assert!(atual.elapsed >= Duration::from_millis(20));
            {
                let _layout = stage("layout");
                assert_eq!(current().unwrap().path, "quadro › layout");
                assert_eq!(current().unwrap().epoch, atual.epoch);
                std::thread::sleep(Duration::from_millis(30));
            }
        }
        assert!(current().is_none());
        {
            let _evento = stage("evento");
        }
        let totais = take_totals();
        let achar = |nome: &str| {
            totais
                .stages
                .iter()
                .find(|(path, _, _)| path == nome)
                .cloned()
                .unwrap()
        };
        let (_, layout, vezes) = achar("quadro › layout");
        assert!(layout >= Duration::from_millis(30), "{layout:?}");
        assert_eq!(vezes, 1);
        let (_, quadro, _) = achar("quadro");
        assert!(quadro >= Duration::from_millis(20), "{quadro:?}");
        assert_eq!(achar("evento").2, 1);
        let (nome, durou) = totais.slowest.unwrap();
        assert_eq!(nome, "quadro");
        // O quadro conta sem o tempo do layout: os dois somam o quadro inteiro.
        assert!(
            quadro + layout <= durou,
            "{quadro:?} + {layout:?} > {durou:?}"
        );
        assert!(take_totals().stages.is_empty(), "zerou");
    }
}
