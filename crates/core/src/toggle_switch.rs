//! `ToggleSwitch`: chave liga/desliga ligada a um `Signal<bool>`.

use std::fmt;

use tomni_ui_reactive::{Get, Signal, Update};

use crate::button::ButtonState;
use crate::cx::EventCx;
use crate::event::Event;
use crate::press::Pressable;
use crate::prop::{IntoProp, Prop};
use crate::widget::{Common, Look, Widget, common_builders};

/// Tudo o que o `Drawer` precisa saber da chave para pintá-la.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ToggleSwitchVisual {
    /// Se está ligada.
    pub on: bool,
    /// Estado, o mesmo do botão.
    pub state: ButtonState,
    /// Anel de foco: só com foco vindo do teclado, e nunca desabilitada.
    pub focus_ring: bool,
}

/// Chave liga/desliga, ligada a um `Signal<bool>`, com "Ligado" ou "Desligado" ao lado e um
/// rótulo opcional acima.
///
/// ```
/// use tomni_ui_core::ToggleSwitch;
/// use tomni_ui_reactive::prelude::*;
///
/// # let scope = Scope::root();
/// # scope.run(|| {
/// let novidades = signal(true);
/// let chave = ToggleSwitch::bind(novidades)
///     .header("Receber novidades por e-mail")
///     .id("novidades");
/// assert!(chave.is_on());
/// # });
/// # scope.clear();
/// ```
///
/// Alterna com o clique em qualquer ponto do controle e com Espaço. O polegar troca de lado
/// na hora: não desliza nem se arrasta. Desabilitada, não recebe foco nem eventos.
pub struct ToggleSwitch {
    common: Common,
    on: Signal<bool>,
    header: Prop<String>,
    disabled: Prop<bool>,
    press: Pressable,
}

impl ToggleSwitch {
    /// Chave ligada a `on`, sem rótulo.
    pub fn bind(on: Signal<bool>) -> Self {
        Self {
            common: Common::default(),
            on,
            header: Prop::default(),
            disabled: Prop::Value(false),
            press: Pressable::default(),
        }
    }

    /// Rótulo acima da chave (fixo, closure ou signal).
    pub fn header<M>(mut self, header: impl IntoProp<String, M>) -> Self {
        self.header = header.into_prop();
        self
    }

    /// Desabilita a chave (valor fixo, closure ou signal).
    pub fn disabled<M>(mut self, disabled: impl IntoProp<bool, M>) -> Self {
        self.disabled = disabled.into_prop();
        self
    }

    /// Se está ligada.
    pub fn is_on(&self) -> bool {
        self.on.get()
    }

    /// Rótulo atual; vazio, sem rótulo.
    pub fn header_text(&self) -> String {
        self.header.get()
    }

    /// Se está desabilitada agora.
    pub fn is_disabled(&self) -> bool {
        self.disabled.get()
    }

    /// Estado para o `Drawer`.
    pub fn visual(&self) -> ToggleSwitchVisual {
        let on = self.is_on();
        if self.is_disabled() {
            return ToggleSwitchVisual {
                on,
                state: ButtonState::Disabled,
                focus_ring: false,
            };
        }
        ToggleSwitchVisual {
            on,
            state: self.press.state(),
            focus_ring: self.press.focus_ring(),
        }
    }

    /// Trata um evento. Devolve `true` se a chave o consumiu.
    pub fn handle(&mut self, event: &Event) -> bool {
        if self.is_disabled() {
            self.press.reset();
            return false;
        }
        let (consumed, clicked) = self.press.handle(event, false);
        if clicked {
            self.on.update(|on| *on = !*on);
        }
        consumed
    }
}

common_builders!(ToggleSwitch);

impl Widget for ToggleSwitch {
    fn kind(&self) -> &'static str {
        "toggle_switch"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::ToggleSwitch {
            header: self.header_text(),
            visual: self.visual(),
        }
    }

    fn handle(&mut self, event: &Event, _: &mut EventCx) -> bool {
        ToggleSwitch::handle(self, event)
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
}

impl fmt::Debug for ToggleSwitch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToggleSwitch")
            .field("common", &self.common)
            .field("header", &self.header)
            .field("disabled", &self.disabled)
            .field("visual", &self.visual())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use tomni_ui_reactive::prelude::*;

    use super::*;
    use crate::event::{FocusSource, Key, PointerKind::Touch};

    #[test]
    fn toque_e_espaco_alternam() {
        let scope = Scope::root();
        scope.run(|| {
            let s = signal(false);
            let mut t = ToggleSwitch::bind(s).header("Wi-Fi");
            assert_eq!(t.header_text(), "Wi-Fi");
            t.handle(&Event::PointerDown {
                kind: Touch,
                clicks: 1,
            });
            assert_eq!(t.visual().state, ButtonState::Pressed);
            t.handle(&Event::PointerUp {
                kind: Touch,
                inside: true,
            });
            assert!(s.get());
            assert_eq!(t.visual().state, ButtonState::Rest, "sem hover no toque");
            t.handle(&Event::FocusIn(FocusSource::Keyboard));
            t.handle(&Event::KeyDown(Key::Space));
            t.handle(&Event::KeyUp(Key::Space));
            assert!(!t.is_on());
        });
        scope.clear();
    }

    #[test]
    fn desabilitada_nao_alterna() {
        let scope = Scope::root();
        scope.run(|| {
            let s = signal(true);
            let mut t = ToggleSwitch::bind(s).disabled(true);
            t.handle(&Event::FocusIn(FocusSource::Keyboard));
            t.handle(&Event::KeyDown(Key::Space));
            t.handle(&Event::KeyUp(Key::Space));
            assert!(s.get());
            assert_eq!(
                t.visual(),
                ToggleSwitchVisual {
                    on: true,
                    state: ButtonState::Disabled,
                    focus_ring: false,
                }
            );
        });
        scope.clear();
    }
}
