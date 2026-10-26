//! `ProgressBar` e `ProgressRing`: andamento de uma tarefa, com valor ou indeterminado.

use std::fmt;
use std::time::Duration;

use crate::prop::{IntoProp, Prop};
use crate::slider::format_number;
use crate::widget::{Common, Look, Widget, common_builders};

/// O estado da barra ou do anel, que muda a cor (`ShowPaused` e `ShowError` do WinUI; no
/// anel é uma adaptação do M49, que o WinUI 3 não tem).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProgressStatus {
    /// Andando: a cor de destaque.
    #[default]
    Normal,
    /// Parada de propósito: a cor de alerta (`SystemFillColorCaution`).
    Paused,
    /// Deu errado: a cor de erro (`SystemFillColorCritical`).
    Error,
}

/// Tudo o que o `Drawer` precisa saber de uma barra ou anel de progresso para pintá-lo.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ProgressVisual {
    /// Andamento, de 0 a 1; `None` é indeterminado.
    pub value: Option<f64>,
    /// Segundos desde a montagem, para a animação do indeterminado. Fica em zero nos
    /// screenshots e nos testes que não avançam o tempo.
    pub phase: f64,
    /// Normal, pausada ou com erro.
    pub status: ProgressStatus,
}

/// Valor, estado e fase comuns às duas formas.
#[derive(Default)]
struct Progress {
    value: Option<Prop<f64>>,
    paused: Option<Prop<bool>>,
    error: Option<Prop<bool>>,
    phase: f64,
}

impl Progress {
    fn value(&self) -> Option<f64> {
        let value = self.value.as_ref()?.get();
        Some(if value.is_nan() {
            0.0
        } else {
            value.clamp(0.0, 1.0)
        })
    }

    /// O erro vale mais que a pausa.
    fn status(&self) -> ProgressStatus {
        if self.error.as_ref().is_some_and(Prop::get) {
            ProgressStatus::Error
        } else if self.paused.as_ref().is_some_and(Prop::get) {
            ProgressStatus::Paused
        } else {
            ProgressStatus::Normal
        }
    }

    fn visual(&self) -> ProgressVisual {
        let value = self.value();
        let status = self.status();
        ProgressVisual {
            value,
            // Com valor, ou parada, a fase não importa: zerada, ela não repinta à toa.
            phase: if value.is_some() || status != ProgressStatus::Normal {
                0.0
            } else {
                self.phase
            },
            status,
        }
    }

    fn tick(&mut self, dt: Duration) {
        self.phase += dt.as_secs_f64();
    }
}

macro_rules! progress_widget {
    ($name:ident, $kind:literal, $look:ident, $doc:literal, $example:literal) => {
        #[doc = $doc]
        ///
        /// ```
        #[doc = concat!("use tomni_ui_core::", stringify!($name), ";")]
        /// use tomni_ui_reactive::prelude::*;
        ///
        /// # let scope = Scope::root();
        /// # scope.run(|| {
        /// let andamento = signal(0.25);
        #[doc = $example]
        /// # });
        /// # scope.clear();
        /// ```
        ///
        /// `.value()` vai de 0 (nada) a 1 (pronto); o que passa disso fica na ponta. Sem
        /// `.value()`, é indeterminado: uma animação contínua, para quando não se sabe quanto
        /// falta.
        pub struct $name {
            common: Common,
            progress: Progress,
        }

        impl $name {
            /// Indeterminado até ganhar um `.value()`.
            pub fn new() -> Self {
                Self {
                    common: Common::default(),
                    progress: Progress::default(),
                }
            }

            /// Andamento, de 0 a 1 (valor fixo, closure ou signal).
            pub fn value<M>(mut self, value: impl IntoProp<f64, M>) -> Self {
                self.progress.value = Some(value.into_prop());
                self
            }

            /// Andamento de 0 a 1, ou `None` se indeterminado.
            pub fn current(&self) -> Option<f64> {
                self.progress.value()
            }

            /// Estado para o `Drawer`.
            pub fn visual(&self) -> ProgressVisual {
                self.progress.visual()
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        common_builders!($name);

        impl Widget for $name {
            fn kind(&self) -> &'static str {
                $kind
            }

            fn common(&self) -> &Common {
                &self.common
            }

            fn look(&self) -> Look {
                Look::$look(self.visual())
            }

            fn text(&self) -> Option<String> {
                Some(self.current().map(format_number).unwrap_or_default())
            }

            fn animating(&self) -> bool {
                self.progress.value.is_none() && self.progress.status() == ProgressStatus::Normal
            }

            fn tick(&mut self, dt: Duration) {
                self.progress.tick(dt);
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_struct(stringify!($name))
                    .field("common", &self.common)
                    .field("visual", &self.visual())
                    .finish()
            }
        }
    };
}

progress_widget!(
    ProgressBar,
    "progress_bar",
    ProgressBar,
    "Barra fina que enche da esquerda para a direita conforme a tarefa anda.",
    "let barra = ProgressBar::new().id(\"envio\").value(andamento);\n\
     assert_eq!(barra.current(), Some(0.25));\n\
     let sem_valor = ProgressBar::new().id(\"sincronizando\");\n\
     assert_eq!(sem_valor.current(), None);"
);

impl ProgressBar {
    /// Pinta a barra com a cor de alerta (`ShowPaused` do WinUI). Sem valor, a barra fica cheia
    /// e parada. Vale `true` ou `false`, um closure ou um signal.
    pub fn paused<M>(mut self, on: impl IntoProp<bool, M>) -> Self {
        self.progress.paused = Some(on.into_prop());
        self
    }

    /// Pinta a barra com a cor de erro (`ShowError` do WinUI). Vale mais que
    /// [`paused`](Self::paused). Sem valor, a barra fica cheia e parada.
    pub fn error<M>(mut self, on: impl IntoProp<bool, M>) -> Self {
        self.progress.error = Some(on.into_prop());
        self
    }

    /// O estado da barra (normal, pausada ou com erro).
    pub fn status(&self) -> ProgressStatus {
        self.progress.status()
    }
}

progress_widget!(
    ProgressRing,
    "progress_ring",
    ProgressRing,
    "Anel que se fecha conforme a tarefa anda; indeterminado, um arco girando.",
    "let anel = ProgressRing::new().id(\"envio\").value(andamento);\n\
     assert_eq!(anel.current(), Some(0.25));"
);

impl ProgressRing {
    /// Pinta o anel com a cor de alerta (`SystemFillColorCaution`). Sem valor, o anel fica
    /// inteiro e parado. O WinUI 3 não tem esse estado no anel: é uma adaptação do M49, para a
    /// aba de terminal mostrar "pausado" em 16 px. Vale `true` ou `false`, um closure ou um
    /// signal.
    pub fn paused<M>(mut self, on: impl IntoProp<bool, M>) -> Self {
        self.progress.paused = Some(on.into_prop());
        self
    }

    /// Pinta o anel com a cor de erro (`SystemFillColorCritical`). Vale mais que
    /// [`paused`](Self::paused). Sem valor, o anel fica inteiro e parado. Adaptação do M49,
    /// como a pausa.
    pub fn error<M>(mut self, on: impl IntoProp<bool, M>) -> Self {
        self.progress.error = Some(on.into_prop());
        self
    }

    /// O estado do anel (normal, pausado ou com erro).
    pub fn status(&self) -> ProgressStatus {
        self.progress.status()
    }
}

#[cfg(test)]
mod tests {
    use tomni_ui_reactive::prelude::*;

    use super::*;

    #[test]
    fn valor_fica_entre_zero_e_um() {
        let scope = Scope::root();
        scope.run(|| {
            let v = signal(1.5);
            let barra = ProgressBar::new().value(v);
            assert_eq!(barra.current(), Some(1.0));
            v.set(-1.0);
            assert_eq!(barra.current(), Some(0.0));
            v.set(f64::NAN);
            assert_eq!(barra.current(), Some(0.0));
            v.set(0.5);
            assert_eq!(barra.text().as_deref(), Some("0.5"));
            assert!(!barra.animating());
        });
        scope.clear();
    }

    #[test]
    fn indeterminado_anima() {
        let mut anel = ProgressRing::new();
        assert!(anel.animating());
        assert_eq!(anel.text().as_deref(), Some(""));
        anel.tick(Duration::from_millis(250));
        anel.tick(Duration::from_millis(250));
        assert_eq!(
            anel.look(),
            Look::ProgressRing(ProgressVisual {
                value: None,
                phase: 0.5,
                status: ProgressStatus::Normal,
            })
        );
    }

    #[test]
    fn pausada_e_com_erro_mudam_o_status() {
        let normal = ProgressBar::new().value(0.5);
        assert_eq!(normal.visual().status, ProgressStatus::Normal);
        let pausada = ProgressBar::new().value(0.5).paused(true);
        assert_eq!(pausada.visual().status, ProgressStatus::Paused);
        assert_eq!(pausada.status(), ProgressStatus::Paused);
        let com_erro = ProgressBar::new().value(0.5).error(true);
        assert_eq!(com_erro.visual().status, ProgressStatus::Error);
        // O erro vale mais que a pausa, na ordem que for.
        let os_dois = ProgressBar::new().value(0.5).paused(true).error(true);
        assert_eq!(os_dois.visual().status, ProgressStatus::Error);
        let ao_contrario = ProgressBar::new().value(0.5).error(true).paused(true);
        assert_eq!(ao_contrario.visual().status, ProgressStatus::Error);
        let desligada = ProgressBar::new().value(0.5).paused(false).error(false);
        assert_eq!(desligada.visual().status, ProgressStatus::Normal);
    }

    #[test]
    fn com_status_o_indeterminado_nao_anima() {
        let mut barra = ProgressBar::new();
        assert!(barra.animating());
        let mut pausada = ProgressBar::new().paused(true);
        assert!(!pausada.animating());
        pausada.tick(Duration::from_secs(1));
        assert_eq!(pausada.visual().phase, 0.0, "parada, a fase não anda");
        assert!(!ProgressBar::new().error(true).animating());
        barra.tick(Duration::from_secs(1));
        assert!(barra.visual().phase > 0.0);
    }

    #[test]
    fn o_anel_tambem_tem_pausa_e_erro() {
        let mut anel = ProgressRing::new().value(0.4).paused(true);
        assert_eq!(anel.status(), ProgressStatus::Paused);
        assert_eq!(
            anel.look(),
            Look::ProgressRing(ProgressVisual {
                value: Some(0.4),
                phase: 0.0,
                status: ProgressStatus::Paused,
            })
        );
        assert_eq!(
            ProgressRing::new().paused(true).error(true).status(),
            ProgressStatus::Error
        );
        let parado = ProgressRing::new().error(true);
        assert!(!parado.animating(), "sem valor e com erro, o anel não gira");
        anel.tick(Duration::from_secs(1));
        assert_eq!(anel.visual().phase, 0.0);
    }

    #[test]
    fn um_signal_liga_e_desliga_o_status() {
        let scope = Scope::root();
        scope.run(|| {
            let pausa = signal(false);
            let falha = signal(false);
            let barra = ProgressBar::new().value(0.4).paused(pausa).error(falha);
            assert_eq!(barra.status(), ProgressStatus::Normal);
            pausa.set(true);
            assert_eq!(barra.status(), ProgressStatus::Paused);
            falha.set(true);
            assert_eq!(barra.status(), ProgressStatus::Error);
            falha.set(false);
            pausa.set(false);
            assert_eq!(barra.status(), ProgressStatus::Normal);
        });
        scope.clear();
    }
}
