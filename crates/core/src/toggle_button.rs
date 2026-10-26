//! `ToggleButton`: botão que fica ligado ou desligado, ligado a um `Signal<bool>`.

use std::fmt;

use tomni_ui_reactive::{Get, Signal, Update};

use crate::button::{ButtonState, ButtonVariant, ButtonVisual};
use crate::cx::EventCx;
use crate::density::Density;
use crate::event::Event;
use crate::press::Pressable;
use crate::prop::{IntoProp, Prop};
use crate::symbol::Symbol;
use crate::widget::{Common, Look, Widget, common_builders};

/// Tudo o que o `Drawer` precisa saber do `ToggleButton` para pintá-lo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ToggleButtonVisual {
    /// Se está ligado.
    pub checked: bool,
    /// Estado, o mesmo do botão.
    pub state: ButtonState,
    /// Anel de foco: só com foco vindo do teclado, e nunca desabilitado.
    pub focus_ring: bool,
    /// Ícone, como o do [`ButtonVisual`].
    pub icon: Option<Symbol>,
    /// Só de ícone, como o [`ButtonVisual`].
    pub icon_only: bool,
    /// Densidade, como a do [`ButtonVisual`].
    pub density: Density,
}

impl ToggleButtonVisual {
    /// O botão com o mesmo visual: o padrão desligado e o de destaque ligado. No WinUI, os
    /// pincéis `ToggleButton*Checked*` são os mesmos do `AccentButton*`, e os outros, os do
    /// `Button`.
    pub fn as_button(self) -> ButtonVisual {
        ButtonVisual {
            variant: if self.checked {
                ButtonVariant::Accent
            } else {
                ButtonVariant::Standard
            },
            state: self.state,
            focus_ring: self.focus_ring,
            icon: self.icon,
            icon_only: self.icon_only,
            density: self.density,
        }
    }
}

/// Botão que alterna entre ligado e desligado, ligado a um `Signal<bool>`: clicar muda o
/// signal, e mudar o signal liga o botão.
///
/// ```
/// use tomni_ui_core::ToggleButton;
/// use tomni_ui_reactive::prelude::*;
///
/// # let scope = Scope::root();
/// # scope.run(|| {
/// let negrito = signal(false);
/// let botao = ToggleButton::bind(negrito, "Negrito").id("negrito");
/// assert!(!botao.is_checked());
/// # });
/// # scope.clear();
/// ```
///
/// Como no WinUI: desligado tem o visual do botão padrão, e ligado o do botão de destaque
/// (`ToggleButtonBackgroundChecked` é o `AccentFillColorDefaultBrush`). Alterna com o clique,
/// com Enter (ao pressionar) e com Espaço (ao soltar), como o `Button`. Não tem o estado
/// indeterminado (`IsThreeState`). Desabilitado, não recebe foco nem eventos.
///
/// Como o `Button`, aceita um ícone antes do rótulo ([`ToggleButton::icon`]) ou só o ícone
/// ([`ToggleButton::icon_only`], num quadrado de 44 px, ou de 32×24 px no compacto, com o nome em [`ToggleButton::label`]).
pub struct ToggleButton {
    common: Common,
    checked: Signal<bool>,
    label: Prop<String>,
    icon: Option<Symbol>,
    icon_only: bool,
    disabled: Prop<bool>,
    press: Pressable,
    density: Density,
}

impl ToggleButton {
    /// Botão ligado a `checked`, com o rótulo `label` (fixo, closure ou signal).
    pub fn bind<M>(checked: Signal<bool>, label: impl IntoProp<String, M>) -> Self {
        Self {
            common: Common::default(),
            checked,
            label: label.into_prop(),
            icon: None,
            icon_only: false,
            disabled: Prop::Value(false),
            press: Pressable::default(),
            density: Density::Standard,
        }
    }

    /// Botão ligado a `checked` só com o ícone `symbol`, num quadrado de 44 px. Dê um nome com
    /// [`ToggleButton::label`]: ele vai para a árvore e para a dica.
    pub fn icon_only(checked: Signal<bool>, symbol: Symbol) -> Self {
        let mut button = Self::bind(checked, String::new());
        button.icon = Some(symbol);
        button.icon_only = true;
        button
    }

    /// Id gerado por um widget composto: o lint de ids não o cobra do app.
    pub(crate) fn generated_id(mut self, id: Option<String>) -> Self {
        self.common = Common::generated(id);
        self
    }

    /// Ícone de 16 px antes do rótulo.
    pub fn icon(mut self, symbol: Symbol) -> Self {
        self.icon = Some(symbol);
        self
    }

    /// Rótulo (fixo, closure ou signal). No [`ToggleButton::icon_only`], é o nome do botão:
    /// não aparece, mas vai para a árvore e, sem `.tooltip`, vira a dica.
    pub fn label<M>(mut self, label: impl IntoProp<String, M>) -> Self {
        self.label = label.into_prop();
        if self.icon_only && self.common.tooltip.is_none() {
            self.common.tooltip = Some(self.label.clone());
        }
        self
    }

    /// Desabilita o botão (valor fixo, closure ou signal).
    pub fn disabled<M>(mut self, disabled: impl IntoProp<bool, M>) -> Self {
        self.disabled = disabled.into_prop();
        self
    }

    /// Se está ligado.
    pub fn is_checked(&self) -> bool {
        self.checked.get()
    }

    /// Rótulo atual (no botão só de ícone, o nome).
    pub fn current_label(&self) -> String {
        self.label.get()
    }

    /// Se está desabilitado agora.
    pub fn is_disabled(&self) -> bool {
        self.disabled.get()
    }

    /// Estado para o `Drawer`.
    pub fn visual(&self) -> ToggleButtonVisual {
        let (state, focus_ring) = if self.is_disabled() {
            (ButtonState::Disabled, false)
        } else {
            (self.press.state(), self.press.focus_ring())
        };
        ToggleButtonVisual {
            checked: self.is_checked(),
            state,
            focus_ring,
            icon: self.icon,
            icon_only: self.icon_only,
            density: self.density,
        }
    }

    /// Trata um evento. Devolve `true` se o botão o consumiu.
    pub fn handle(&mut self, event: &Event) -> bool {
        if self.is_disabled() {
            self.press.reset();
            return false;
        }
        let (consumed, clicked) = self.press.handle(event, true);
        if clicked {
            self.checked.update(|c| *c = !*c);
        }
        consumed
    }
}

common_builders!(ToggleButton);

impl Widget for ToggleButton {
    fn kind(&self) -> &'static str {
        "toggle_button"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn set_density(&mut self, density: Density) {
        self.density = density;
    }

    fn look(&self) -> Look {
        Look::ToggleButton {
            label: self.current_label(),
            visual: self.visual(),
        }
    }

    fn handle(&mut self, event: &Event, _: &mut EventCx) -> bool {
        ToggleButton::handle(self, event)
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

impl fmt::Debug for ToggleButton {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToggleButton")
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

    fn click(b: &mut ToggleButton) {
        b.handle(&Event::PointerDown {
            kind: Mouse,
            clicks: 1,
        });
        b.handle(&Event::PointerUp {
            kind: Mouse,
            inside: true,
        });
    }

    #[test]
    fn clique_alterna() {
        with(|s| {
            let mut b = ToggleButton::bind(s, "Negrito");
            click(&mut b);
            assert!(s.get());
            assert!(b.visual().checked);
            click(&mut b);
            assert!(!s.get());
        });
    }

    #[test]
    fn enter_alterna_ao_pressionar_e_espaco_ao_soltar() {
        with(|s| {
            let mut b = ToggleButton::bind(s, "Negrito");
            b.handle(&Event::FocusIn(FocusSource::Keyboard));
            assert!(b.visual().focus_ring);
            b.handle(&Event::KeyDown(Key::Enter));
            assert!(s.get(), "Enter alterna ao pressionar, como no botão");
            b.handle(&Event::KeyUp(Key::Enter));
            b.handle(&Event::KeyDown(Key::Space));
            assert_eq!(b.visual().state, ButtonState::Pressed);
            assert!(s.get(), "Espaço alterna só ao soltar");
            b.handle(&Event::KeyUp(Key::Space));
            assert!(!s.get());
        });
    }

    #[test]
    fn soltar_fora_nao_alterna() {
        with(|s| {
            let mut b = ToggleButton::bind(s, "Negrito");
            b.handle(&Event::PointerDown {
                kind: Mouse,
                clicks: 1,
            });
            b.handle(&Event::PointerUp {
                kind: Mouse,
                inside: false,
            });
            assert!(!s.get());
        });
    }

    #[test]
    fn signal_liga_o_botao() {
        with(|s| {
            let b = ToggleButton::bind(s, "Negrito");
            s.set(true);
            assert!(b.visual().checked);
            assert!(b.is_checked());
        });
    }

    #[test]
    fn desabilitado_ignora_tudo() {
        with(|s| {
            let mut b = ToggleButton::bind(s, "Negrito").disabled(true);
            click(&mut b);
            b.handle(&Event::KeyDown(Key::Enter));
            assert!(!s.get());
            assert!(!b.focusable());
            assert_eq!(b.visual().state, ButtonState::Disabled);
            s.set(true);
            assert!(b.visual().checked, "o signal ainda liga o botão");
        });
    }
}
