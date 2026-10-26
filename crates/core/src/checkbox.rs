//! `CheckBox`: caixa de seleção ligada a um `Signal<bool>`.

use std::fmt;

use tomni_ui_reactive::{Get, Signal, Update};

use crate::button::ButtonState;
use crate::cx::EventCx;
use crate::event::Event;
use crate::press::Pressable;
use crate::prop::{IntoProp, Prop};
use crate::widget::{Common, Look, Widget, common_builders};

/// Tudo o que o `Drawer` precisa saber da caixa de seleção para pintá-la.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CheckBoxVisual {
    /// Se está marcada.
    pub checked: bool,
    /// Estado, o mesmo do botão.
    pub state: ButtonState,
    /// Anel de foco: só com foco vindo do teclado, e nunca desabilitada.
    pub focus_ring: bool,
}

/// Caixa de seleção com rótulo, ligada a um `Signal<bool>`: marcar muda o signal, e mudar o
/// signal marca a caixa.
///
/// ```
/// use tomni_ui_core::CheckBox;
/// use tomni_ui_reactive::prelude::*;
///
/// # let scope = Scope::root();
/// # scope.run(|| {
/// let aceito = signal(false);
/// let termos = CheckBox::bind(aceito, "Li e aceito os termos").id("termos");
/// assert!(!termos.is_checked());
/// # });
/// # scope.clear();
/// ```
///
/// Alterna com o clique (na caixa ou no rótulo) e com Espaço; Enter não alterna, como no
/// WinUI. Não tem o estado indeterminado. Desabilitada, não recebe foco nem eventos.
pub struct CheckBox {
    common: Common,
    checked: Signal<bool>,
    label: Prop<String>,
    disabled: Prop<bool>,
    press: Pressable,
}

impl CheckBox {
    /// Caixa ligada a `checked`, com o rótulo `label` (fixo, closure ou signal).
    pub fn bind<M>(checked: Signal<bool>, label: impl IntoProp<String, M>) -> Self {
        Self {
            common: Common::default(),
            checked,
            label: label.into_prop(),
            disabled: Prop::Value(false),
            press: Pressable::default(),
        }
    }

    /// Id derivado do id do grupo que monta a caixa (a `PropertyGrid`).
    pub(crate) fn generated_id(mut self, id: Option<String>) -> Self {
        self.common = Common::generated(id);
        self
    }

    /// Desabilita a caixa (valor fixo, closure ou signal).
    pub fn disabled<M>(mut self, disabled: impl IntoProp<bool, M>) -> Self {
        self.disabled = disabled.into_prop();
        self
    }

    /// Se está marcada.
    pub fn is_checked(&self) -> bool {
        self.checked.get()
    }

    /// Rótulo atual.
    pub fn label(&self) -> String {
        self.label.get()
    }

    /// Se está desabilitada agora.
    pub fn is_disabled(&self) -> bool {
        self.disabled.get()
    }

    /// Estado para o `Drawer`.
    pub fn visual(&self) -> CheckBoxVisual {
        let checked = self.is_checked();
        if self.is_disabled() {
            return CheckBoxVisual {
                checked,
                state: ButtonState::Disabled,
                focus_ring: false,
            };
        }
        CheckBoxVisual {
            checked,
            state: self.press.state(),
            focus_ring: self.press.focus_ring(),
        }
    }

    /// Trata um evento. Devolve `true` se a caixa o consumiu.
    pub fn handle(&mut self, event: &Event) -> bool {
        if self.is_disabled() {
            self.press.reset();
            return false;
        }
        let (consumed, clicked) = self.press.handle(event, false);
        if clicked {
            self.checked.update(|c| *c = !*c);
        }
        consumed
    }
}

common_builders!(CheckBox);

impl Widget for CheckBox {
    fn kind(&self) -> &'static str {
        "checkbox"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::CheckBox {
            label: self.label(),
            visual: self.visual(),
        }
    }

    fn handle(&mut self, event: &Event, _: &mut EventCx) -> bool {
        CheckBox::handle(self, event)
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

impl fmt::Debug for CheckBox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CheckBox")
            .field("common", &self.common)
            .field("label", &self.label)
            .field("disabled", &self.disabled)
            .field("visual", &self.visual())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use tomni_ui_reactive::prelude::*;

    use super::*;
    use crate::event::{FocusSource, Key, PointerKind::Mouse};

    fn with(test: impl FnOnce(Signal<bool>)) {
        let scope = Scope::root();
        scope.run(|| test(signal(false)));
        scope.clear();
    }

    fn click(c: &mut CheckBox) {
        c.handle(&Event::PointerDown {
            kind: Mouse,
            clicks: 1,
        });
        c.handle(&Event::PointerUp {
            kind: Mouse,
            inside: true,
        });
    }

    #[test]
    fn clique_e_espaco_alternam() {
        with(|s| {
            let mut c = CheckBox::bind(s, "Termos");
            click(&mut c);
            assert!(s.get());
            assert!(c.visual().checked);
            c.handle(&Event::FocusIn(FocusSource::Keyboard));
            assert!(c.visual().focus_ring);
            c.handle(&Event::KeyDown(Key::Space));
            assert_eq!(c.visual().state, ButtonState::Pressed);
            assert!(s.get(), "alterna ao soltar");
            c.handle(&Event::KeyUp(Key::Space));
            assert!(!s.get());
            c.handle(&Event::KeyDown(Key::Enter));
            assert!(!s.get(), "Enter não alterna");
        });
    }

    #[test]
    fn soltar_fora_nao_alterna() {
        with(|s| {
            let mut c = CheckBox::bind(s, "Termos");
            c.handle(&Event::PointerDown {
                kind: Mouse,
                clicks: 1,
            });
            c.handle(&Event::PointerUp {
                kind: Mouse,
                inside: false,
            });
            assert!(!s.get());
        });
    }

    #[test]
    fn desabilitada_ignora_tudo() {
        with(|s| {
            let mut c = CheckBox::bind(s, "Termos").disabled(true);
            click(&mut c);
            assert!(!s.get());
            assert!(!c.focusable());
            assert_eq!(c.visual().state, ButtonState::Disabled);
            s.set(true);
            assert!(c.visual().checked, "o signal ainda marca a caixa");
        });
    }
}
