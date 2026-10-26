//! `Slider`: escolhe um número numa faixa arrastando um polegar.

use std::fmt;
use std::ops::RangeInclusive;

use tomni_ui_reactive::{Get, GetUntracked, Set, Signal};

use crate::button::ButtonState;
use crate::cx::EventCx;
use crate::event::{Event, FocusSource, Key};
use crate::prop::{IntoProp, Prop};
use crate::widget::{Common, Look, Widget, common_builders};

/// Diâmetro do polegar (`SliderHorizontalThumbWidth`): o centro dele anda de `x0 + 9` a
/// `x1 - 9`.
pub const SLIDER_THUMB_SIZE: f64 = 18.0;

/// Tudo o que o `Drawer` precisa saber de um `Slider` para pintá-lo.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SliderVisual {
    /// Posição do valor na faixa, de 0 a 1.
    pub fraction: f64,
    /// Estado: `Hover` com o ponteiro em cima, `Pressed` arrastando.
    pub state: ButtonState,
    /// Anel de foco: só com foco vindo do teclado, e nunca desabilitado.
    pub focus_ring: bool,
}

/// Escolhe um número entre duas pontas, ligado a um `Signal<f64>`.
///
/// ```
/// use tomni_ui_core::Slider;
/// use tomni_ui_reactive::prelude::*;
///
/// # let scope = Scope::root();
/// # scope.run(|| {
/// let volume = signal(50.0);
/// let slider = Slider::bind(volume, 0..=100).id("volume").header("Volume").step(5.0);
/// assert_eq!(slider.value(), 50.0);
/// # });
/// # scope.clear();
/// ```
///
/// Um clique na trilha leva o valor até ali, e arrastar o acompanha. Com o foco, ← e ↓ descem
/// um passo, → e ↑ sobem, PageDown e PageUp andam um décimo da faixa e Home e End vão às
/// pontas. O valor sempre cai num múltiplo do passo (contado da ponta de baixo) e fica dentro
/// da faixa.
pub struct Slider {
    common: Common,
    value: Signal<f64>,
    min: f64,
    max: f64,
    step: f64,
    header: Prop<String>,
    disabled: Prop<bool>,
    hovered: bool,
    dragging: bool,
    focus: Option<FocusSource>,
}

impl Slider {
    /// Slider que mostra e altera `value`, entre as pontas de `range` (`0..=100`, `0.0..=1.0`).
    pub fn bind<T: Into<f64>>(value: Signal<f64>, range: RangeInclusive<T>) -> Self {
        let (min, max) = range.into_inner();
        let (min, max) = (min.into(), max.into());
        Self {
            common: Common::default(),
            value,
            min: min.min(max),
            max: max.max(min),
            step: 1.0,
            header: Prop::default(),
            disabled: Prop::Value(false),
            hovered: false,
            dragging: false,
            focus: None,
        }
    }

    /// Passo do valor (padrão 1). Um passo de zero ou negativo deixa o valor contínuo.
    pub fn step(mut self, step: f64) -> Self {
        self.step = step;
        self
    }

    /// Rótulo acima da trilha (fixo, closure ou signal); vazio, sem rótulo.
    pub fn header<M>(mut self, header: impl IntoProp<String, M>) -> Self {
        self.header = header.into_prop();
        self
    }

    /// Desabilita o slider (valor fixo, closure ou signal).
    pub fn disabled<M>(mut self, disabled: impl IntoProp<bool, M>) -> Self {
        self.disabled = disabled.into_prop();
        self
    }

    /// Valor atual.
    pub fn value(&self) -> f64 {
        self.value.get()
    }

    /// Se está desabilitado agora.
    pub fn is_disabled(&self) -> bool {
        self.disabled.get()
    }

    /// Estado para o `Drawer`.
    pub fn visual(&self) -> SliderVisual {
        let span = self.max - self.min;
        let fraction = if span > 0.0 {
            ((self.value.get() - self.min) / span).clamp(0.0, 1.0)
        } else {
            0.0
        };
        if self.is_disabled() {
            return SliderVisual {
                fraction,
                state: ButtonState::Disabled,
                focus_ring: false,
            };
        }
        let state = if self.dragging {
            ButtonState::Pressed
        } else if self.hovered {
            ButtonState::Hover
        } else {
            ButtonState::Rest
        };
        SliderVisual {
            fraction,
            state,
            focus_ring: self.focus == Some(FocusSource::Keyboard),
        }
    }

    /// `value` encaixado no passo e preso entre as pontas.
    fn snap(&self, value: f64) -> f64 {
        let value = value.clamp(self.min, self.max);
        if self.step <= 0.0 {
            return value;
        }
        let steps = ((value - self.min) / self.step).round();
        let snapped = self.min + steps * self.step;
        // O último passo pode passar da ponta de cima quando a faixa não é múltipla dele.
        if snapped > self.max {
            snapped - self.step
        } else {
            snapped
        }
    }

    fn set(&self, value: f64) {
        let value = self.snap(value);
        if self.value.get_untracked() != value {
            self.value.set(value);
        }
    }

    /// Leva o valor até a posição do ponteiro.
    fn follow(&self, cx: &EventCx) {
        let rect = cx.rect();
        let half = SLIDER_THUMB_SIZE / 2.0;
        let width = rect.width() - SLIDER_THUMB_SIZE;
        let fraction = if width > 0.0 {
            ((cx.pointer().x - rect.x0 - half) / width).clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.set(self.min + fraction * (self.max - self.min));
    }
}

common_builders!(Slider);

impl Widget for Slider {
    fn kind(&self) -> &'static str {
        "slider"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::Slider {
            header: self.header.get(),
            visual: self.visual(),
        }
    }

    fn handle(&mut self, event: &Event, cx: &mut EventCx) -> bool {
        if self.is_disabled() {
            self.hovered = false;
            self.dragging = false;
            self.focus = None;
            return false;
        }
        match *event {
            Event::PointerMove { kind, inside } => {
                self.hovered = inside && kind.hovers();
                if self.dragging {
                    self.follow(cx);
                }
                true
            }
            Event::PointerDown { kind, .. } => {
                self.hovered = kind.hovers();
                self.dragging = true;
                self.follow(cx);
                true
            }
            Event::PointerUp { kind, inside } => {
                self.hovered = inside && kind.hovers();
                self.dragging = false;
                true
            }
            Event::PointerCancel => {
                self.hovered = false;
                self.dragging = false;
                true
            }
            Event::FocusIn(source) => {
                self.focus = Some(source);
                true
            }
            Event::FocusOut => {
                self.focus = None;
                true
            }
            Event::KeyDown(key) if self.focus.is_some() && !cx.modifiers().command() => {
                let current = self.value.get_untracked();
                let step = if self.step > 0.0 { self.step } else { 1.0 };
                let page = ((self.max - self.min) / 10.0).max(step);
                let target = match key {
                    Key::Left | Key::Down => current - step,
                    Key::Right | Key::Up => current + step,
                    Key::PageDown => current - page,
                    Key::PageUp => current + page,
                    Key::Home => self.min,
                    Key::End => self.max,
                    _ => return false,
                };
                self.set(target);
                true
            }
            _ => false,
        }
    }

    fn interactive(&self) -> bool {
        true
    }

    fn focusable(&self) -> bool {
        !self.is_disabled()
    }

    fn enabled(&self) -> bool {
        !self.is_disabled()
    }

    fn text(&self) -> Option<String> {
        Some(format_number(self.value.get()))
    }

    fn range(&self) -> Option<RangeInclusive<f64>> {
        Some(self.min..=self.max)
    }
}

/// O número sem casas decimais inúteis: `50`, `0.25`.
pub(crate) fn format_number(value: f64) -> String {
    let rounded = (value * 1e6).round() / 1e6;
    let mut text = format!("{rounded}");
    if text == "-0" {
        text = "0".into();
    }
    text
}

impl fmt::Debug for Slider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Slider")
            .field("common", &self.common)
            .field("min", &self.min)
            .field("max", &self.max)
            .field("step", &self.step)
            .field("visual", &self.visual())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use kurbo::{Point, Rect};
    use tomni_ui_reactive::prelude::*;

    use super::*;
    use crate::cx::{FixedMetrics, MemoryClipboard};
    use crate::event::{Modifiers, PointerKind::Mouse};

    fn with(test: impl FnOnce(Signal<f64>)) {
        let scope = Scope::root();
        scope.run(|| test(signal(50.0)));
        scope.clear();
    }

    /// Manda `event` com o slider em x de 0 a 218 (a trilha útil vai de 9 a 209) e o ponteiro
    /// em `x`.
    fn send(slider: &mut Slider, event: Event, x: f64) -> bool {
        let mut metrics = FixedMetrics::default();
        let mut clipboard = MemoryClipboard::default();
        let mut cx = EventCx::new(
            Rect::new(0.0, 0.0, 218.0, 32.0),
            Point::new(x, 16.0),
            Modifiers::NONE,
            &mut metrics,
            &mut clipboard,
        );
        slider.handle(&event, &mut cx)
    }

    fn key(slider: &mut Slider, key: Key) {
        send(slider, Event::KeyDown(key), 0.0);
    }

    #[test]
    fn clique_e_arrasto_levam_o_valor() {
        with(|s| {
            let mut slider = Slider::bind(s, 0..=100);
            let down = Event::PointerDown {
                kind: Mouse,
                clicks: 1,
            };
            send(&mut slider, down, 59.0);
            assert_eq!(s.get(), 25.0);
            assert_eq!(slider.visual().state, ButtonState::Pressed);
            let move_ = Event::PointerMove {
                kind: Mouse,
                inside: false,
            };
            send(&mut slider, move_.clone(), 500.0);
            assert_eq!(s.get(), 100.0, "fora da trilha, fica na ponta");
            send(
                &mut slider,
                Event::PointerUp {
                    kind: Mouse,
                    inside: false,
                },
                500.0,
            );
            send(&mut slider, move_, 0.0);
            assert_eq!(s.get(), 100.0, "sem arrastar, mover não muda");
            assert_eq!(slider.visual().fraction, 1.0);
        });
    }

    #[test]
    fn teclas_andam_no_passo() {
        with(|s| {
            let mut slider = Slider::bind(s, 0..=100).step(5.0);
            key(&mut slider, Key::Right);
            assert_eq!(s.get(), 50.0, "sem foco, nada");
            send(&mut slider, Event::FocusIn(FocusSource::Keyboard), 0.0);
            assert!(slider.visual().focus_ring);
            key(&mut slider, Key::Right);
            assert_eq!(s.get(), 55.0);
            key(&mut slider, Key::Down);
            key(&mut slider, Key::Down);
            assert_eq!(s.get(), 45.0);
            key(&mut slider, Key::PageUp);
            assert_eq!(s.get(), 55.0);
            key(&mut slider, Key::End);
            key(&mut slider, Key::Up);
            assert_eq!(s.get(), 100.0);
            key(&mut slider, Key::Home);
            assert_eq!(s.get(), 0.0);
        });
    }

    #[test]
    fn valor_encaixa_no_passo_e_na_faixa() {
        with(|s| {
            let slider = Slider::bind(s, 1..=10).step(4.0);
            assert_eq!(slider.snap(3.2), 5.0);
            assert_eq!(slider.snap(10.0), 9.0, "13 passaria da ponta");
            assert_eq!(slider.snap(-3.0), 1.0);
            let continuo = Slider::bind(s, 0.0..=1.0).step(0.0);
            assert_eq!(continuo.snap(0.123), 0.123);
        });
    }

    #[test]
    fn texto_sem_casas_inuteis() {
        assert_eq!(format_number(50.0), "50");
        assert_eq!(format_number(0.25), "0.25");
        assert_eq!(format_number(0.1 + 0.2), "0.3");
        assert_eq!(format_number(-0.0), "0");
    }

    #[test]
    fn desabilitado_ignora_tudo() {
        with(|s| {
            let mut slider = Slider::bind(s, 0..=100).disabled(true);
            assert!(!send(
                &mut slider,
                Event::PointerDown {
                    kind: Mouse,
                    clicks: 1
                },
                9.0
            ));
            assert_eq!(s.get(), 50.0);
            assert_eq!(slider.visual().state, ButtonState::Disabled);
        });
    }
}
