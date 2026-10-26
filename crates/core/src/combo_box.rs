//! `ComboBox`: escolhe um valor numa lista que abre embaixo da caixa.

use std::fmt;

use kurbo::Size;
use tomni_ui_reactive::{Get, GetUntracked, Set, Signal, With, WithUntracked, signal};

use crate::button::ButtonState;
use crate::containers::Column;
use crate::cx::EventCx;
use crate::density::Density;
use crate::event::{Event, Key};
use crate::press::Pressable;
use crate::prop::{IntoProp, Prop};
use crate::scroll_viewer::ScrollViewer;
use crate::widget::{Common, Flex, Insets, Layer, Look, PopupKind, Widget, common_builders};

/// Margem da lista dentro do popup (`ComboBoxDropdownContentMargin`), com a borda somada.
const DROPDOWN_PADDING: Insets = Insets::new(5.0, 1.0, 5.0, 1.0);
/// `ComboBoxPopupThemeMinWidth`.
const DROPDOWN_MIN_WIDTH: f64 = 80.0;
/// Itens que a lista mostra antes de rolar.
const DROPDOWN_ROWS: f64 = 15.0;
/// Altura de um item, com as margens: o alvo de toque do WinUI (`ComboBoxItemThemeTouchPadding`).
pub const COMBO_BOX_ITEM_HEIGHT: f64 = 48.0;
/// Altura de um item na densidade compacta, com as margens: o `ComboBoxItem` do WinUI para o
/// mouse (32 px mais a margem de 2 em cima e embaixo).
pub const COMBO_BOX_ITEM_HEIGHT_COMPACT: f64 = 36.0;

/// Altura de um item da lista na densidade `density`, com as margens.
pub(crate) fn item_height(density: Density) -> f64 {
    match density {
        Density::Standard => COMBO_BOX_ITEM_HEIGHT,
        Density::Compact => COMBO_BOX_ITEM_HEIGHT_COMPACT,
    }
}

/// Tudo o que o `Drawer` precisa saber da caixa fechada para pintá-la.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ComboBoxVisual {
    /// Valor escolhido; vazio, mostra o `placeholder`.
    pub text: String,
    /// Texto de quando nada foi escolhido.
    pub placeholder: String,
    /// Estado.
    pub state: ButtonState,
    /// Anel de foco: só com foco vindo do teclado, e nunca desabilitado.
    pub focus_ring: bool,
    /// Densidade: na compacta, a caixa tem 24 px e o `ComboBoxPadding` do `Compact.xaml`.
    pub density: Density,
}

/// Tudo o que o `Drawer` precisa saber de um item da lista para pintá-lo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ComboBoxItemVisual {
    /// Estado.
    pub state: ButtonState,
    /// Se é o valor escolhido (a pílula na cor accent, à esquerda).
    pub selected: bool,
    /// Anel de foco: só com foco vindo do teclado.
    pub focus_ring: bool,
    /// Densidade: na compacta, o item tem [`COMBO_BOX_ITEM_HEIGHT_COMPACT`] px.
    pub density: Density,
}

/// Caixa de seleção de um valor, ligada a um `Signal<String>`.
///
/// ```
/// use tomni_ui_core::ComboBox;
/// use tomni_ui_reactive::prelude::*;
///
/// # let scope = Scope::root();
/// # scope.run(|| {
/// let uf = signal(String::new());
/// let caixa = ComboBox::bind(uf, ["MG", "RJ", "SP"])
///     .id("uf")
///     .placeholder("Escolha a UF");
/// assert_eq!(caixa.value(), "");
/// # });
/// # scope.clear();
/// ```
///
/// Clique, Enter, Espaço ou Alt+↓ abrem a lista embaixo da caixa (em cima, se não couber), com
/// o foco no valor escolhido; ↑ e ↓ andam pela lista, Enter, Espaço ou um clique escolhem, e Esc
/// ou um clique fora fecham sem mudar nada. Com a caixa fechada e o foco nela, ↑ e ↓ (e Home e
/// End) trocam o valor direto.
///
/// Cada item tem o id `{id}-{opção}`, com a opção em minúsculas e o que não é letra ou número
/// trocado por `-` (`uf-sp`, `cidade-são-paulo`). Um valor que não está nas opções aparece na
/// caixa, mas nenhum item fica marcado.
pub struct ComboBox {
    common: Common,
    value: Signal<String>,
    options: Vec<String>,
    placeholder: Prop<String>,
    disabled: Prop<bool>,
    open: Signal<bool>,
    press: Pressable,
    density: Density,
}

impl ComboBox {
    /// Caixa que mostra e altera `value`, com as opções `options`.
    pub fn bind(
        value: Signal<String>,
        options: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            common: Common::default(),
            value,
            options: options.into_iter().map(Into::into).collect(),
            placeholder: Prop::default(),
            disabled: Prop::Value(false),
            open: signal(false),
            press: Pressable::default(),
            density: Density::Standard,
        }
    }

    /// Id derivado do id do grupo que monta a caixa (a `PropertyGrid`).
    pub(crate) fn generated_id(mut self, id: Option<String>) -> Self {
        self.common = Common::generated(id);
        self
    }

    /// Usa `open` como o estado da lista, para quem monta a caixa abri-la por código.
    pub(crate) fn open_signal(mut self, open: Signal<bool>) -> Self {
        self.open = open;
        self
    }

    /// Texto de quando nada foi escolhido (fixo, closure ou signal).
    pub fn placeholder<M>(mut self, placeholder: impl IntoProp<String, M>) -> Self {
        self.placeholder = placeholder.into_prop();
        self
    }

    /// Desabilita a caixa (valor fixo, closure ou signal).
    pub fn disabled<M>(mut self, disabled: impl IntoProp<bool, M>) -> Self {
        self.disabled = disabled.into_prop();
        self
    }

    /// Valor escolhido; vazio se nada foi escolhido.
    pub fn value(&self) -> String {
        self.value.get()
    }

    /// Opções, na ordem.
    pub fn options(&self) -> &[String] {
        &self.options
    }

    /// Se está desabilitada agora.
    pub fn is_disabled(&self) -> bool {
        self.disabled.get()
    }

    /// Estado para o `Drawer`.
    pub fn visual(&self) -> ComboBoxVisual {
        let disabled = self.is_disabled();
        ComboBoxVisual {
            text: self.value.get(),
            placeholder: self.placeholder.get(),
            state: if disabled {
                ButtonState::Disabled
            } else {
                self.press.state()
            },
            focus_ring: !disabled && self.press.focus_ring(),
            density: self.density,
        }
    }

    /// Escolhe a opção `n` posições depois da atual (antes, se negativo), sem passar das
    /// pontas. Sem valor escolhido, ↓ vai para a primeira e ↑ para a última.
    fn step(&self, delta: isize) {
        let Some(last) = self.options.len().checked_sub(1) else {
            return;
        };
        let current = self.value.get_untracked();
        let next = match self.options.iter().position(|o| *o == current) {
            Some(i) => i.saturating_add_signed(delta).min(last),
            None if delta > 0 => 0,
            None => last,
        };
        if self.options[next] != current {
            self.value.set(self.options[next].clone());
        }
    }
}

common_builders!(ComboBox);

/// Parte do id de um item: a opção em minúsculas, com o que não é letra ou número trocado por
/// `-`.
pub(crate) fn option_slug(option: &str) -> String {
    option
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .flat_map(char::to_lowercase)
        .collect()
}

impl Widget for ComboBox {
    fn kind(&self) -> &'static str {
        "combo_box"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn names_children(&self) -> bool {
        true
    }

    fn look(&self) -> Look {
        Look::ComboBox(self.visual())
    }

    fn set_density(&mut self, density: Density) {
        self.density = density;
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        let id = self.common.id.clone();
        let mut list = Column::new();
        for option in &self.options {
            list = list.child(ComboBoxItem {
                common: Common::generated(
                    id.as_ref()
                        .map(|id| format!("{id}-{}", option_slug(option))),
                ),
                label: option.clone(),
                value: self.value,
                open: self.open,
                press: Pressable::default(),
                density: self.density,
            });
        }
        let mut scroll = ScrollViewer::new().child(list).generated();
        if let Some(id) = &id {
            scroll = scroll.id(format!("{id}-rolagem"));
        }
        vec![Box::new(ComboBoxDropDown {
            common: Common::generated(id.map(|id| format!("{id}-lista"))),
            open: self.open,
            child: Some(Box::new(scroll)),
            density: self.density,
        })]
    }

    fn handle(&mut self, event: &Event, cx: &mut EventCx) -> bool {
        if self.is_disabled() {
            self.press.reset();
            return false;
        }
        match event {
            Event::KeyDown(Key::Down) if cx.modifiers().alt => {
                self.open.set(true);
                return true;
            }
            Event::KeyDown(Key::F(4)) if self.press.is_focused() && !cx.modifiers().ctrl => {
                self.open.set(true);
                return true;
            }
            Event::KeyDown(key @ (Key::Up | Key::Down | Key::Home | Key::End))
                if self.press.is_focused() =>
            {
                let n = self.options.len() as isize;
                self.step(match key {
                    Key::Up => -1,
                    Key::Down => 1,
                    Key::Home => -n,
                    _ => n,
                });
                return true;
            }
            _ => {}
        }
        let (consumed, clicked) = self.press.handle(event, true);
        if clicked {
            self.open.set(true);
        }
        consumed
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
        Some(self.value.get())
    }
}

impl fmt::Debug for ComboBox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ComboBox")
            .field("common", &self.common)
            .field("options", &self.options)
            .field("visual", &self.visual())
            .finish_non_exhaustive()
    }
}

/// O popup com a lista de um [`ComboBox`].
struct ComboBoxDropDown {
    common: Common,
    open: Signal<bool>,
    child: Option<Box<dyn Widget>>,
    density: Density,
}

impl Widget for ComboBoxDropDown {
    fn kind(&self) -> &'static str {
        "combo_box_dropdown"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn visible(&self) -> bool {
        self.open.get()
    }

    fn look(&self) -> Look {
        Look::Popup(PopupKind::ComboBox)
    }

    fn flex(&self) -> Option<Flex> {
        Some(Flex {
            padding: DROPDOWN_PADDING,
            ..Flex::default()
        })
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        self.child.take().into_iter().collect()
    }

    fn layer(&self) -> Option<Layer> {
        Some(Layer {
            arrows: true,
            match_width: true,
            min_size: Size::new(DROPDOWN_MIN_WIDTH, 0.0),
            max_size: Size::new(
                f64::INFINITY,
                DROPDOWN_ROWS * item_height(self.density)
                    + DROPDOWN_PADDING.top
                    + DROPDOWN_PADDING.bottom,
            ),
            ..Layer::default()
        })
    }

    fn set_open(&mut self, open: bool) {
        self.open.set(open);
    }
}

/// Item da lista de um [`ComboBox`].
struct ComboBoxItem {
    common: Common,
    label: String,
    value: Signal<String>,
    open: Signal<bool>,
    press: Pressable,
    density: Density,
}

impl ComboBoxItem {
    fn selected(&self) -> bool {
        self.value.with(|v| *v == self.label)
    }

    fn selected_untracked(&self) -> bool {
        self.value.with_untracked(|v| *v == self.label)
    }
}

impl Widget for ComboBoxItem {
    fn kind(&self) -> &'static str {
        "combo_box_item"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::ComboBoxItem {
            label: self.label.clone(),
            visual: ComboBoxItemVisual {
                state: self.press.state(),
                selected: self.selected(),
                focus_ring: self.press.focus_ring(),
                density: self.density,
            },
        }
    }

    fn handle(&mut self, event: &Event, _: &mut EventCx) -> bool {
        let (consumed, clicked) = self.press.handle(event, true);
        if clicked {
            if !self.selected_untracked() {
                self.value.set(self.label.clone());
            }
            self.open.set(false);
        }
        consumed
    }

    fn interactive(&self) -> bool {
        true
    }

    fn focusable(&self) -> bool {
        true
    }

    fn text(&self) -> Option<String> {
        Some(self.label.clone())
    }

    fn autofocus(&self) -> bool {
        self.selected_untracked()
    }
}
