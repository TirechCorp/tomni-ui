//! Máquina de clique comum aos controles que se ativam com ponteiro e teclado (`Button`,
//! `CheckBox`, `ToggleSwitch` e o botão de fechar da `InfoBar`).

use crate::button::ButtonState;
use crate::event::{Event, FocusSource, Key};

/// O que está segurando o controle pressionado.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Press {
    /// Ponteiro pressionado sobre o controle; `inside` acompanha se ainda está sobre ele.
    Pointer { inside: bool },
    /// Barra de espaço, com foco.
    Space,
}

/// Hover, press e foco de um controle clicável.
///
/// O clique dispara ao soltar sobre o controle, desde que o press tenha começado nele; Espaço
/// dispara ao soltar e, se o controle aceitar, Enter ao pressionar; no toque não há hover.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Pressable {
    hovered: bool,
    press: Option<Press>,
    focus: Option<FocusSource>,
}

impl Pressable {
    /// Esquece hover, press e foco (o controle ficou desabilitado).
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    /// Se está com o foco.
    pub(crate) fn is_focused(&self) -> bool {
        self.focus.is_some()
    }

    /// Se o ponteiro está sobre o controle (o mouse; o toque não tem hover).
    pub(crate) fn hovered(&self) -> bool {
        self.hovered
    }

    /// Anel de foco: só com foco vindo do teclado.
    pub(crate) fn focus_ring(&self) -> bool {
        self.focus == Some(FocusSource::Keyboard)
    }

    /// Estado visual, com o controle habilitado.
    pub(crate) fn state(&self) -> ButtonState {
        match self.press {
            Some(Press::Pointer { inside: true } | Press::Space) => ButtonState::Pressed,
            _ if self.hovered => ButtonState::Hover,
            _ => ButtonState::Rest,
        }
    }

    /// Trata um evento: devolve se o consumiu e se o controle foi ativado (clicado). `enter`
    /// diz se Enter ativa, como no `Button`.
    pub(crate) fn handle(&mut self, event: &Event, enter: bool) -> (bool, bool) {
        match *event {
            Event::PointerMove { kind, inside } => {
                self.hovered = inside && kind.hovers();
                if let Some(Press::Pointer { inside: was }) = &mut self.press {
                    *was = inside;
                }
                (true, false)
            }
            Event::PointerDown { kind, .. } => {
                self.hovered = kind.hovers();
                self.press = Some(Press::Pointer { inside: true });
                (true, false)
            }
            Event::PointerUp { kind, inside } => {
                self.hovered = inside && kind.hovers();
                if !matches!(self.press, Some(Press::Pointer { .. })) {
                    return (false, false);
                }
                self.press = None;
                (true, inside)
            }
            Event::PointerCancel => {
                self.hovered = false;
                if matches!(self.press, Some(Press::Pointer { .. })) {
                    self.press = None;
                }
                (true, false)
            }
            Event::KeyDown(Key::Enter) if enter && self.is_focused() => (true, true),
            Event::KeyDown(Key::Space) if self.is_focused() => {
                self.press = Some(Press::Space);
                (true, false)
            }
            Event::KeyUp(Key::Space) if self.press == Some(Press::Space) => {
                self.press = None;
                (true, true)
            }
            Event::FocusIn(source) => {
                self.focus = Some(source);
                (true, false)
            }
            Event::FocusOut => {
                self.focus = None;
                // Espaço segurado quando o foco sai não vira clique.
                if self.press == Some(Press::Space) {
                    self.press = None;
                }
                (true, false)
            }
            Event::KeyDown(_)
            | Event::KeyUp(_)
            | Event::Text(_)
            | Event::Ime(_)
            | Event::Wheel { .. }
            | Event::Scroll { .. }
            | Event::SecondaryDown
            | Event::SecondaryUp
            | Event::ContextOpening
            | Event::MiddleClick
            | Event::MiddleDown
            | Event::MiddleUp
            | Event::DragOver(_)
            | Event::DragLeave
            | Event::Drop(_)
            | Event::DropOutside { .. }
            | Event::FilesDropped { .. } => (false, false),
        }
    }
}
