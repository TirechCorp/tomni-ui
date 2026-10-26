//! `RadioButtons`: grupo de opções exclusivas ligado a um `Signal<String>`.

use std::fmt;
use std::rc::Rc;

use tomni_ui_reactive::{Get, Set, Signal, With, WithUntracked};

use crate::button::ButtonState;
use crate::combo_box::option_slug;
use crate::containers::Column;
use crate::cx::{EventCx, FocusMove};
use crate::density::Density;
use crate::event::{Event, Key};
use crate::press::Pressable;
use crate::prop::{IntoProp, Prop};
use crate::text::Text;
use crate::widget::{Common, Flex, Insets, Look, Widget, common_builders};

/// Altura de cada opção: o alvo de toque mínimo do lint, no lugar dos 32 px do WinUI
/// (`RadioButton.MinHeight`).
pub const RADIO_BUTTON_HEIGHT: f64 = 44.0;
/// Altura de cada opção na densidade compacta: os 32 px do WinUI (`RadioButton.MinHeight`).
pub const RADIO_BUTTON_HEIGHT_COMPACT: f64 = 32.0;
/// Espaço entre o cabeçalho e as opções (`RadioButtonsHeaderMargin`, embaixo).
const HEADER_SPACING: f64 = 8.0;

/// Tudo o que o `Drawer` precisa saber de uma opção para pintá-la.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct RadioButtonVisual {
    /// Se é a opção escolhida.
    pub checked: bool,
    /// Estado, o mesmo do botão.
    pub state: ButtonState,
    /// Anel de foco: só com foco vindo do teclado, e nunca desabilitada.
    pub focus_ring: bool,
}

/// Grupo de opções exclusivas, ligado a um `Signal<String>` com a opção escolhida.
///
/// ```
/// use tomni_ui_core::RadioButtons;
/// use tomni_ui_reactive::prelude::*;
///
/// # let scope = Scope::root();
/// # scope.run(|| {
/// let tema = signal("Claro".to_string());
/// let grupo = RadioButtons::bind(tema, ["Claro", "Escuro", "Do sistema"])
///     .id("tema")
///     .header("Tema");
/// assert_eq!(grupo.value(), "Claro");
/// # });
/// # scope.clear();
/// ```
///
/// Um clique ou Espaço numa opção a escolhe. O grupo é uma parada só de Tab: o foco entra na
/// opção escolhida (ou, sem nenhuma, na primeira), e ↑/↓ (ou ←/→) movem o foco e a escolha
/// juntos, sem dar a volta. Um valor que não está nas opções deixa todas desmarcadas.
///
/// As opções ficam uma embaixo da outra, com 44 px de altura cada (32 na densidade compacta),
/// e têm o id `{id}-{opção}`,
/// com a opção em minúsculas e o que não é letra ou número trocado por `-` (`tema-escuro`,
/// `tema-do-sistema`).
pub struct RadioButtons {
    common: Common,
    value: Signal<String>,
    options: Rc<[String]>,
    header: Prop<String>,
    disabled: Prop<bool>,
    density: Density,
}

impl RadioButtons {
    /// Grupo que mostra e altera `value`, com as opções `options`.
    pub fn bind(
        value: Signal<String>,
        options: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            common: Common::default(),
            value,
            options: options.into_iter().map(Into::into).collect(),
            header: Prop::default(),
            disabled: Prop::Value(false),
            density: Density::Standard,
        }
    }

    /// Cabeçalho acima das opções (fixo, closure ou signal); vazio, sem cabeçalho.
    pub fn header<M>(mut self, header: impl IntoProp<String, M>) -> Self {
        self.header = header.into_prop();
        self
    }

    /// Desabilita o grupo inteiro (valor fixo, closure ou signal).
    pub fn disabled<M>(mut self, disabled: impl IntoProp<bool, M>) -> Self {
        self.disabled = disabled.into_prop();
        self
    }

    /// Opção escolhida; vazio se nenhuma.
    pub fn value(&self) -> String {
        self.value.get()
    }

    /// Opções, na ordem.
    pub fn options(&self) -> &[String] {
        &self.options
    }
}

common_builders!(RadioButtons);

impl Widget for RadioButtons {
    fn kind(&self) -> &'static str {
        "radio_buttons"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn names_children(&self) -> bool {
        true
    }

    fn look(&self) -> Look {
        Look::Column
    }

    fn set_density(&mut self, density: Density) {
        self.density = density;
    }

    fn flex(&self) -> Option<Flex> {
        Some(Flex::default())
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        let header = self.header.clone();
        let shown = self.header.clone();
        let mut children: Vec<Box<dyn Widget>> = vec![Box::new(
            Column::new()
                .padding(Insets::new(0.0, 0.0, HEADER_SPACING, 0.0))
                .visible(move || !shown.get().is_empty())
                .child(Text::new(move || header.get())),
        )];
        for (index, option) in self.options.iter().enumerate() {
            children.push(Box::new(RadioButton {
                common: Common {
                    id: self
                        .common
                        .id
                        .as_ref()
                        .map(|id| format!("{id}-{}", option_slug(option))),
                    height: Some(match self.density {
                        Density::Standard => RADIO_BUTTON_HEIGHT,
                        Density::Compact => RADIO_BUTTON_HEIGHT_COMPACT,
                    }),
                    generated: true,
                    ..Common::default()
                },
                value: self.value,
                options: Rc::clone(&self.options),
                index,
                disabled: self.disabled.clone(),
                press: Pressable::default(),
            }));
        }
        children
    }

    fn enabled(&self) -> bool {
        !self.disabled.get()
    }

    fn text(&self) -> Option<String> {
        Some(self.value.get())
    }
}

impl fmt::Debug for RadioButtons {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RadioButtons")
            .field("common", &self.common)
            .field("options", &self.options)
            .field("header", &self.header)
            .field("disabled", &self.disabled)
            .finish_non_exhaustive()
    }
}

/// Uma opção de um [`RadioButtons`].
struct RadioButton {
    common: Common,
    value: Signal<String>,
    options: Rc<[String]>,
    index: usize,
    disabled: Prop<bool>,
    press: Pressable,
}

impl RadioButton {
    fn label(&self) -> &str {
        &self.options[self.index]
    }

    fn checked(&self) -> bool {
        self.value.with(|v| v == self.label())
    }

    fn is_disabled(&self) -> bool {
        self.disabled.get()
    }

    fn visual(&self) -> RadioButtonVisual {
        let checked = self.checked();
        if self.is_disabled() {
            return RadioButtonVisual {
                checked,
                state: ButtonState::Disabled,
                focus_ring: false,
            };
        }
        RadioButtonVisual {
            checked,
            state: self.press.state(),
            focus_ring: self.press.focus_ring(),
        }
    }

    /// Escolhe a opção `index`.
    fn choose(&self, index: usize) {
        let option = &self.options[index];
        if self.value.with_untracked(|v| v != option) {
            self.value.set(option.clone());
        }
    }
}

impl Widget for RadioButton {
    fn kind(&self) -> &'static str {
        "radio_button"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::RadioButton {
            label: self.label().to_owned(),
            visual: self.visual(),
        }
    }

    fn handle(&mut self, event: &Event, cx: &mut EventCx) -> bool {
        if self.is_disabled() {
            self.press.reset();
            return false;
        }
        if let Event::KeyDown(key @ (Key::Up | Key::Down | Key::Left | Key::Right)) = event
            && self.press.is_focused()
            && !cx.modifiers().command()
            && !cx.modifiers().alt
        {
            let back = matches!(key, Key::Up | Key::Left);
            let next = if back {
                self.index.checked_sub(1)
            } else {
                Some(self.index + 1).filter(|&i| i < self.options.len())
            };
            if let Some(next) = next {
                self.choose(next);
                cx.move_focus(if back {
                    FocusMove::Prev
                } else {
                    FocusMove::Next
                });
            }
            return true;
        }
        let (consumed, clicked) = self.press.handle(event, false);
        if clicked {
            self.choose(self.index);
        }
        consumed
    }

    fn interactive(&self) -> bool {
        true
    }

    fn focusable(&self) -> bool {
        !self.is_disabled()
    }

    fn tab_stop(&self) -> bool {
        let value = self.value.get();
        if value == self.label() {
            return true;
        }
        self.index == 0 && !self.options.contains(&value)
    }

    fn enabled(&self) -> bool {
        !self.is_disabled()
    }

    fn text(&self) -> Option<String> {
        Some(self.label().to_owned())
    }
}

#[cfg(test)]
mod tests {
    use kurbo::{Point, Rect};
    use tomni_ui_reactive::prelude::*;

    use super::*;
    use crate::cx::{FixedMetrics, MemoryClipboard};
    use crate::event::{FocusSource, Modifiers, PointerKind::Mouse};

    fn with(test: impl FnOnce(Signal<String>)) {
        let scope = Scope::root();
        scope.run(|| test(signal(String::new())));
        scope.clear();
    }

    /// As opções de um grupo `a`, `b`, `c` ligado a `s`, como a montagem as cria.
    fn items(s: Signal<String>) -> Vec<RadioButton> {
        let options: Rc<[String]> = ["a", "b", "c"].map(String::from).into();
        (0..options.len())
            .map(|index| RadioButton {
                common: Common {
                    id: Some(format!("g-{}", options[index])),
                    ..Common::default()
                },
                value: s,
                options: Rc::clone(&options),
                index,
                disabled: Prop::Value(false),
                press: Pressable::default(),
            })
            .collect()
    }

    fn send(item: &mut RadioButton, event: Event) -> (bool, Option<FocusMove>) {
        let mut metrics = FixedMetrics::default();
        let mut clipboard = MemoryClipboard::default();
        let mut cx = EventCx::new(
            Rect::ZERO,
            Point::ZERO,
            Modifiers::NONE,
            &mut metrics,
            &mut clipboard,
        );
        let consumed = item.handle(&event, &mut cx);
        (consumed, cx.focus_request())
    }

    #[test]
    fn clique_escolhe() {
        with(|s| {
            let mut items = items(s);
            send(
                &mut items[1],
                Event::PointerDown {
                    kind: Mouse,
                    clicks: 1,
                },
            );
            send(
                &mut items[1],
                Event::PointerUp {
                    kind: Mouse,
                    inside: true,
                },
            );
            assert_eq!(s.get(), "b");
            assert!(items[1].visual().checked);
            assert!(!items[0].visual().checked);
        });
    }

    #[test]
    fn setas_escolhem_e_pedem_o_foco_sem_dar_a_volta() {
        with(|s| {
            let mut items = items(s);
            assert!(items[0].tab_stop(), "sem escolha, a primeira é a parada");
            assert!(!items[1].tab_stop());
            send(&mut items[0], Event::FocusIn(FocusSource::Keyboard));
            assert_eq!(
                send(&mut items[0], Event::KeyDown(Key::Down)),
                (true, Some(FocusMove::Next))
            );
            assert_eq!(s.get(), "b");
            assert!(items[1].tab_stop() && !items[0].tab_stop());
            assert_eq!(
                send(&mut items[0], Event::KeyDown(Key::Up)),
                (true, None),
                "a primeira não volta para a última"
            );
            send(&mut items[2], Event::FocusIn(FocusSource::Keyboard));
            assert_eq!(
                send(&mut items[2], Event::KeyDown(Key::Left)),
                (true, Some(FocusMove::Prev))
            );
            assert_eq!(s.get(), "b");
        });
    }

    #[test]
    fn montagem_da_os_ids_e_a_altura() {
        with(|s| {
            let mut group = RadioButtons::bind(s, ["Claro", "Do sistema"]).id("tema");
            let children = group.take_children();
            assert_eq!(children.len(), 3, "cabeçalho e duas opções");
            assert_eq!(children[2].common().id.as_deref(), Some("tema-do-sistema"));
            assert_eq!(children[2].common().height, Some(RADIO_BUTTON_HEIGHT));
            assert_eq!(children[1].kind(), "radio_button");
        });
    }

    #[test]
    fn desabilitado_ignora_tudo() {
        with(|s| {
            let mut group = RadioButtons::bind(s, ["a"]).disabled(true);
            let children = group.take_children();
            let item = &children[1];
            assert!(!item.focusable());
            assert_eq!(
                item.look(),
                Look::RadioButton {
                    label: "a".into(),
                    visual: RadioButtonVisual {
                        state: ButtonState::Disabled,
                        ..RadioButtonVisual::default()
                    },
                }
            );
        });
    }
}
