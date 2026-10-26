//! `NumberBox`: campo de número, com botões de incremento, como o do WinUI.

use std::cell::Cell;
use std::fmt;
use std::rc::Rc;

use kurbo::{Size, Vec2};
use tomni_ui_reactive::{Get, GetUntracked, Set, Signal, With, WithUntracked, effect, signal};

use crate::cx::{EventCx, InputPurpose};
use crate::density::Density;
use crate::edit::{Edit, Part};
use crate::event::{Event, Key};
use crate::field_button::{FieldAction, FieldButton, FieldGlyph, FieldIndicator};
use crate::prop::{IntoProp, Prop};
use crate::widget::{Align, Common, Flex, Insets, Layer, Look, Placement, PopupKind, Widget};

/// Largura do X de limpar: o `MinWidth` do `DeleteButton` do `NumberBox` do WinUI, com a
/// margem de dentro (o fundo tem 36 px).
const CLEAR_WIDTH: f64 = 40.0;

/// Largura de um botão de incremento dentro do campo (`MinWidth` do `NumberBoxSpinButtonStyle`).
const SPIN_WIDTH: f64 = 32.0;

/// Margem do botão de aumentar dentro do campo (`Margin="4"`).
const SPIN_UP_MARGIN: Insets = Insets {
    top: 4.0,
    right: 4.0,
    bottom: 4.0,
    left: 4.0,
};

/// Margem do botão de diminuir dentro do campo (`Margin="0,4,4,4"`).
const SPIN_DOWN_MARGIN: Insets = Insets {
    top: 4.0,
    right: 4.0,
    bottom: 4.0,
    left: 0.0,
};

/// Ícone do indicador fixo (`PopupIndicator`, 12 px) com a `NumberBoxPopupIndicatorMargin`
/// (`0,0,8,0`).
const INDICATOR_WIDTH: f64 = 20.0;

/// Margem do indicador fixo.
const INDICATOR_MARGIN: Insets = Insets {
    top: 0.0,
    right: 8.0,
    bottom: 0.0,
    left: 0.0,
};

/// Lado de um botão do popup de incremento (`NumberBoxPopupSpinButtonStyle`).
const POPUP_BUTTON: f64 = 36.0;

/// Espaço entre os botões do popup (`Margin="0,0,0,4"` no de cima).
const POPUP_GAP: f64 = 4.0;

/// Padding do popup (6) mais a borda (1).
const POPUP_PADDING: f64 = 7.0;

/// Deslocamento do popup a partir do canto superior direito do campo
/// (`NumberBoxPopupHorizonalOffset` e `NumberBoxPopupVerticalOffset`).
const POPUP_OFFSET: Vec2 = Vec2::new(-21.0, -27.0);

/// Dígitos significativos do número mostrado sem `.precision` (o `SignificantDigits(10)` do
/// arredondador do WinUI).
const DISPLAY_DIGITS: i32 = 10;

/// Onde ficam os botões de aumentar e diminuir: o `SpinButtonPlacementMode` do WinUI.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SpinButtons {
    /// Sem botões: o valor muda pelo teclado e pela roda (padrão, como no WinUI).
    #[default]
    Hidden,
    /// Um indicador no fim do campo e os botões num popup sobre a ponta direita dele,
    /// enquanto o campo tem o foco.
    Compact,
    /// Dentro do campo, no fim, sempre à vista.
    Inline,
}

/// Limites, passos e casas decimais.
#[derive(Clone, Copy, Debug)]
struct Config {
    min: f64,
    max: f64,
    small: f64,
    large: f64,
    /// Casas decimais do valor; sem elas, o valor fica como veio e o campo mostra até 10
    /// dígitos significativos.
    precision: Option<usize>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            min: f64::NEG_INFINITY,
            max: f64::INFINITY,
            small: 1.0,
            large: 10.0,
            precision: None,
        }
    }
}

/// O valor ligado ao campo.
#[derive(Clone, Copy)]
enum Binding {
    /// Sempre tem número: vazio volta ao último valor.
    Required(Signal<f64>),
    /// Vazio vira `None` (o `NaN` do WinUI).
    Optional(Signal<Option<f64>>),
}

impl Binding {
    fn get(self) -> Option<f64> {
        match self {
            Self::Required(value) => Some(value.get()),
            Self::Optional(value) => value.get(),
        }
    }

    fn get_untracked(self) -> Option<f64> {
        match self {
            Self::Required(value) => Some(value.get_untracked()),
            Self::Optional(value) => value.get_untracked(),
        }
    }

    /// Grava `value` se mudou. `None` só vale no `Optional`.
    fn set(self, value: Option<f64>) {
        if self.get_untracked() == value {
            return;
        }
        match (self, value) {
            (Self::Required(signal), Some(value)) => signal.set(value),
            (Self::Optional(signal), value) => signal.set(value),
            (Self::Required(_), None) => {}
        }
    }
}

/// O que o campo e os botões compartilham.
#[derive(Clone)]
struct Shared {
    text: Signal<String>,
    value: Binding,
    config: Rc<Cell<Config>>,
}

impl Shared {
    fn config(&self) -> Config {
        self.config.get()
    }

    /// `value` no formato do campo, ou vazio.
    fn format(&self, value: Option<f64>) -> String {
        value.map_or_else(String::new, |v| match self.config().precision {
            Some(precision) => format_number(v, precision),
            None => format_display(v),
        })
    }

    fn set_text(&self, text: String) {
        if self.text.with_untracked(|t| *t == text) {
            return;
        }
        self.text.set(text);
    }

    /// Volta o texto ao valor (o `UpdateTextToValue` do WinUI).
    fn restore(&self) {
        self.set_text(self.format(self.value.get_untracked()));
    }

    /// Arredonda nas casas decimais, se houver `.precision`, e prende entre os limites (o
    /// `CoerceValue` do WinUI).
    fn fit(&self, value: f64) -> f64 {
        let config = self.config();
        let value = match config.precision {
            Some(precision) => {
                let scale = 10f64.powi(i32::try_from(precision).unwrap_or(i32::MAX));
                let rounded = (value * scale).round() / scale;
                if rounded.is_finite() { rounded } else { value }
            }
            None => value,
        };
        value.clamp(config.min, config.max)
    }

    /// Confere o texto (o foco saiu, Enter, antes de um passo), como o `ValidateInput` do
    /// WinUI: número vira o valor, preso nos limites; vazio vira `None` no
    /// `bind_optional`; o resto volta ao último valor.
    fn commit(&self) {
        let text = self.text.get_untracked();
        let value = match parse_number(&text) {
            Some(value) => Some(self.fit(value)),
            None if text.trim().is_empty() && matches!(self.value, Binding::Optional(_)) => None,
            None => self.value.get_untracked(),
        };
        self.value.set(value);
        self.set_text(self.format(value));
    }

    /// Confere o texto e soma `delta` ao valor (o `StepValue` do WinUI). Sem valor, não faz
    /// nada.
    fn step(&self, delta: f64) {
        self.commit();
        let Some(value) = self.value.get_untracked() else {
            return;
        };
        let value = self.fit(value + delta);
        self.value.set(Some(value));
        self.set_text(self.format(Some(value)));
    }
}

/// Lê um número escrito no campo: aceita vírgula ou ponto como separador decimal (`1,5` e
/// `1.5`) e, com vírgula, ponto como separador de milhar (`1.234,5`). `None` se não for um
/// número.
pub fn parse_number(text: &str) -> Option<f64> {
    let text: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if text.is_empty() {
        return None;
    }
    let text = if text.contains(',') {
        text.replace('.', "").replace(',', ".")
    } else {
        text
    };
    let valid = text
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+'));
    let value: f64 = text.parse().ok().filter(|_| valid)?;
    value.is_finite().then_some(value)
}

/// `value` com até `precision` casas, vírgula decimal e sem zeros no fim: `1,5`, `2`, `-0,25`.
pub fn format_number(value: f64, precision: usize) -> String {
    let text = format!("{value:.precision$}");
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.')
    } else {
        &text
    };
    let text = if text == "-0" { "0" } else { text };
    text.replace('.', ",")
}

/// `value` como o campo mostra sem `.precision`, como o `NumberBox` do WinUI: arredondado em
/// 10 dígitos significativos, com vírgula decimal e sem zeros no fim (`0,1 + 0,2` aparece
/// como `0,3`). O valor em si não muda.
pub(crate) fn format_display(value: f64) -> String {
    let rounded = if value == 0.0 || !value.is_finite() {
        value
    } else {
        let magnitude = value.abs().log10().floor();
        let scale = 10f64.powf(f64::from(DISPLAY_DIGITS - 1) - magnitude);
        let rounded = (value * scale).round() / scale;
        if rounded.is_finite() { rounded } else { value }
    };
    let text = format!("{rounded}");
    let text = if text == "-0" { "0" } else { &text };
    text.replace('.', ",")
}

/// Campo de número, ligado a um `Signal<f64>` (ou `Signal<Option<f64>>`).
///
/// ```
/// use tomni_ui_core::NumberBox;
/// use tomni_ui_reactive::prelude::*;
///
/// # let scope = Scope::root();
/// # scope.run(|| {
/// let idade = signal(30.0);
/// let campo = NumberBox::bind(idade)
///     .id("idade")
///     .min(0)
///     .max(130)
///     .precision(0);
/// assert_eq!(campo.value(), "30");
/// # });
/// # scope.clear();
/// ```
///
/// Regras (as do `NumberBox` do WinUI):
///
/// - Digita-se livremente; o número vale quando o foco sai ou com Enter, preso entre
///   `.min()` e `.max()`. O campo mostra o número com vírgula, sem zeros no fim e com até 10
///   dígitos significativos (`1,5`); com `.precision(casas)`, o valor também é arredondado
///   nessas casas. Texto que não é número volta ao último valor; vazio também, a não ser no
///   [`NumberBox::bind_optional`], em que vira `None`. Esc volta o texto ao valor.
/// - Ao receber o foco, o texto fica todo selecionado.
/// - ↑ e ↓ somam e subtraem o `.small_change()` (1); PageUp e PageDown, o `.large_change()`
///   (10). A roda do mouse sobre o campo com foco também muda o valor. Antes do passo, o
///   texto digitado é conferido; sem valor (`None`), o passo não faz nada.
/// - Os botões de incremento ([`SpinButtons`]) não aparecem por padrão. Com
///   `SpinButtons::Compact`, um indicador fica no fim do campo e os botões abrem num popup
///   sobre a ponta direita dele enquanto ele tem o foco (id `{id}-incremento`); com
///   `SpinButtons::Inline`, ficam dentro do campo. Os ids são `{id}-aumentar` e
///   `{id}-diminuir`; segurados, repetem a cada [`SPIN_REPEAT`](crate::SPIN_REPEAT) e não
///   tiram o foco do campo. O de aumentar desabilita no máximo, o de diminuir no mínimo, e
///   os dois sem valor.
/// - Com foco e texto, um botão `{id}-limpar` apaga o campo (`.clear_button(false)` tira).
pub struct NumberBox {
    edit: Edit,
    shared: Shared,
    clear_button: bool,
    spin_buttons: SpinButtons,
}

impl NumberBox {
    /// Campo que mostra e altera `value`. Vazio ou com texto que não é número, volta ao último
    /// valor.
    pub fn bind(value: Signal<f64>) -> Self {
        Self::with_binding(Binding::Required(value))
    }

    /// Campo que mostra e altera `value`, que fica `None` com o campo vazio.
    pub fn bind_optional(value: Signal<Option<f64>>) -> Self {
        Self::with_binding(Binding::Optional(value))
    }

    fn with_binding(value: Binding) -> Self {
        let config = Rc::new(Cell::new(Config::default()));
        let text = signal(String::new());
        let shared = Shared {
            text,
            value,
            config,
        };
        shared.set_text(shared.format(value.get_untracked()));
        // Valor mudado por fora (o app, outro widget): o campo mostra o novo.
        let sync = shared.clone();
        effect(move || {
            let value = sync.value.get();
            let current = parse_number(&sync.text.get_untracked());
            if current != value {
                sync.set_text(sync.format(value));
            }
        });
        Self {
            edit: Edit::bind(text),
            shared,
            clear_button: true,
            spin_buttons: SpinButtons::Hidden,
        }
    }

    fn configure(self, f: impl FnOnce(&mut Config)) -> Self {
        let mut config = self.shared.config();
        f(&mut config);
        self.shared.config.set(config);
        self
    }

    /// Menor valor aceito.
    pub fn min(self, min: impl Into<f64>) -> Self {
        let min = min.into();
        self.configure(|c| c.min = min)
    }

    /// Maior valor aceito.
    pub fn max(self, max: impl Into<f64>) -> Self {
        let max = max.into();
        self.configure(|c| c.max = max)
    }

    /// Passo de ↑, ↓, da roda e dos botões de incremento (1, por padrão).
    pub fn small_change(self, step: impl Into<f64>) -> Self {
        let step = step.into();
        self.configure(|c| c.small = step)
    }

    /// Passo de PageUp e PageDown (10, por padrão).
    pub fn large_change(self, step: impl Into<f64>) -> Self {
        let step = step.into();
        self.configure(|c| c.large = step)
    }

    /// Casas decimais do valor (0 para inteiros): o valor é arredondado nelas e o campo
    /// mostra no máximo essas casas. Sem `.precision`, o valor fica como veio e o campo mostra
    /// até 10 dígitos significativos, como no WinUI.
    pub fn precision(self, digits: usize) -> Self {
        let this = self.configure(|c| c.precision = Some(digits));
        let text = this.shared.format(this.shared.value.get_untracked());
        this.shared.set_text(text);
        this
    }

    /// Texto mostrado enquanto o campo está vazio (fixo, closure ou signal).
    pub fn placeholder<M>(mut self, placeholder: impl IntoProp<String, M>) -> Self {
        self.edit = self.edit.placeholder(placeholder);
        self
    }

    /// Id derivado do id do grupo que monta o campo (a `PropertyGrid`).
    pub(crate) fn generated_id(mut self, id: Option<String>) -> Self {
        self.edit = self.edit.generated_id(id);
        self
    }

    /// Desabilita o campo (valor fixo, closure ou signal).
    pub fn disabled<M>(mut self, disabled: impl IntoProp<bool, M>) -> Self {
        self.edit = self.edit.disabled(disabled);
        self
    }

    /// Mostra o botão de limpar, com foco e texto (padrão: sim).
    pub fn clear_button(mut self, show: bool) -> Self {
        self.clear_button = show;
        self
    }

    /// Onde ficam os botões de incremento: [`SpinButtons`].
    pub fn spin_buttons(mut self, placement: SpinButtons) -> Self {
        self.spin_buttons = placement;
        self
    }

    /// Texto do campo agora.
    pub fn value(&self) -> String {
        self.edit.value()
    }

    /// Se os botões de incremento sobre o campo estão à vista.
    pub fn is_open(&self) -> bool {
        self.spin_buttons == SpinButtons::Compact
            && self.edit.focused.get()
            && !self.edit.is_disabled()
    }

    /// Um botão de incremento, que repete enquanto está pressionado. Habilitado com valor e
    /// abaixo do máximo (aumentar) ou acima do mínimo (diminuir), como no WinUI.
    fn spin(&self, edit: &Edit, up: bool, width: f64, shown: Rc<dyn Fn() -> bool>) -> FieldButton {
        let shared = self.shared.clone();
        let strings = crate::strings();
        let (suffix, label, glyph) = if up {
            ("aumentar", strings.increase.to_string(), FieldGlyph::Up)
        } else {
            ("diminuir", strings.decrease.to_string(), FieldGlyph::Down)
        };
        let disabled = edit.disabled.clone();
        let value = self.shared.value;
        let config = Rc::clone(&self.shared.config);
        let enabled = move || {
            let config = config.get();
            !disabled.get()
                && value
                    .get()
                    .is_some_and(|v| if up { v < config.max } else { v > config.min })
        };
        FieldButton::new(
            edit.part_id(suffix),
            label,
            glyph,
            width,
            shown,
            FieldAction::Repeat(Rc::new(move || {
                let small = shared.config().small;
                shared.step(if up { small } else { -small });
            })),
        )
        .enabled(Prop::Dynamic(Rc::new(enabled)))
    }
}

impl NumberBox {
    /// Id estável, usado em testes, no MCP e no lint.
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.edit = self.edit.id(id);
        self
    }

    /// Fator de crescimento no eixo principal do contêiner.
    pub fn grow(mut self, grow: impl Into<f64>) -> Self {
        self.edit = self.edit.grow(grow);
        self
    }

    /// Largura fixa, em pixels.
    pub fn width(mut self, width: impl Into<f64>) -> Self {
        self.edit = self.edit.width(width);
        self
    }

    /// Altura fixa, em pixels.
    pub fn height(mut self, height: impl Into<f64>) -> Self {
        self.edit = self.edit.height(height);
        self
    }

    /// Mostra ou esconde (valor fixo, closure ou signal).
    pub fn visible<M>(mut self, visible: impl IntoProp<bool, M>) -> Self {
        self.edit = self.edit.visible(visible);
        self
    }

    /// Dica que aparece com o ponteiro parado em cima.
    pub fn tooltip<M>(mut self, text: impl IntoProp<String, M>) -> Self {
        self.edit = self.edit.tooltip(text);
        self
    }
}

impl Widget for NumberBox {
    fn kind(&self) -> &'static str {
        "number_box"
    }

    fn common(&self) -> &Common {
        self.edit.common()
    }

    fn look(&self) -> Look {
        self.edit.look()
    }

    fn flex(&self) -> Option<Flex> {
        self.edit.flex()
    }

    fn measured(&self) -> bool {
        self.edit.measured()
    }

    fn set_density(&mut self, density: Density) {
        self.edit.set_density(density);
    }

    fn names_children(&self) -> bool {
        true
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        let focused = self.edit.focused;
        let disabled = self.edit.disabled.clone();
        let text = self.shared.text;
        if self.clear_button {
            let shared = self.shared.clone();
            let disabled = disabled.clone();
            self.edit.push_part(Box::new(move |edit: &Edit| {
                let shown: Rc<dyn Fn() -> bool> = Rc::new(move || {
                    focused.get() && !disabled.get() && text.with(|t| !t.is_empty())
                });
                Part {
                    widget: Box::new(FieldButton::new(
                        edit.part_id("limpar"),
                        crate::strings().clear.to_string(),
                        FieldGlyph::Clear,
                        CLEAR_WIDTH,
                        Rc::clone(&shown),
                        FieldAction::Click(Rc::new(move || shared.set_text(String::new()))),
                    )),
                    width: CLEAR_WIDTH,
                    shown,
                }
            }));
        }
        let mut layer = None;
        let always: Rc<dyn Fn() -> bool> = Rc::new(|| true);
        match self.spin_buttons {
            SpinButtons::Compact => {
                let shown = Rc::clone(&always);
                self.edit.push_part(Box::new(move |_: &Edit| Part {
                    widget: Box::new(FieldIndicator::new(
                        FieldGlyph::UpDown,
                        INDICATOR_WIDTH,
                        INDICATOR_MARGIN,
                    )),
                    width: INDICATOR_WIDTH,
                    shown,
                }));
                let open: Rc<dyn Fn() -> bool> = {
                    let disabled = disabled.clone();
                    Rc::new(move || focused.get() && !disabled.get())
                };
                let up = self
                    .spin(&self.edit, true, POPUP_BUTTON, Rc::clone(&open))
                    .large(POPUP_BUTTON);
                let down = self
                    .spin(&self.edit, false, POPUP_BUTTON, Rc::clone(&open))
                    .large(POPUP_BUTTON);
                layer = Some(SpinPopup {
                    common: Common::generated(self.edit.part_id("incremento")),
                    open,
                    children: vec![Box::new(up), Box::new(down)],
                });
            }
            SpinButtons::Inline => {
                let up = self
                    .spin(&self.edit, true, SPIN_WIDTH + 8.0, Rc::clone(&always))
                    .margin(SPIN_UP_MARGIN);
                let down = self
                    .spin(&self.edit, false, SPIN_WIDTH + 4.0, Rc::clone(&always))
                    .margin(SPIN_DOWN_MARGIN);
                for (button, width) in [(up, SPIN_WIDTH + 8.0), (down, SPIN_WIDTH + 4.0)] {
                    let shown = Rc::clone(&always);
                    self.edit.push_part(Box::new(move |_: &Edit| Part {
                        widget: Box::new(button),
                        width,
                        shown,
                    }));
                }
            }
            SpinButtons::Hidden => {}
        }
        let mut children = Widget::take_children(&mut self.edit);
        children.extend(layer.map(|l| Box::new(l) as Box<dyn Widget>));
        children
    }

    fn handle(&mut self, event: &Event, cx: &mut EventCx) -> bool {
        if self.edit.is_disabled() {
            return Widget::handle(&mut self.edit, event, cx);
        }
        let focused = self.edit.is_focused();
        let plain = !cx.modifiers().command() && !cx.modifiers().alt;
        let config = self.shared.config();
        let step = match event {
            Event::KeyDown(Key::Up) if focused && plain => Some(config.small),
            Event::KeyDown(Key::Down) if focused && plain => Some(-config.small),
            Event::KeyDown(Key::PageUp) if focused && plain => Some(config.large),
            Event::KeyDown(Key::PageDown) if focused && plain => Some(-config.large),
            Event::Wheel { up } if focused => Some(if *up { config.small } else { -config.small }),
            _ => None,
        };
        if let Some(delta) = step {
            self.shared.step(delta);
            return true;
        }
        match event {
            // Enter confere o número e segue adiante (o botão padrão do formulário).
            Event::KeyDown(Key::Enter) if focused => self.shared.commit(),
            // Esc volta ao valor; sem nada a desfazer, segue adiante (fecha o dialog).
            Event::KeyDown(Key::Escape) if focused && plain => {
                let before = self.shared.text.get_untracked();
                self.shared.restore();
                if self.shared.text.with_untracked(|t| *t != before) {
                    return true;
                }
            }
            Event::FocusOut => self.shared.commit(),
            // Como o `GotFocus` do WinUI. No clique, o `PointerDown` vem depois e põe o cursor.
            Event::FocusIn(_) => {
                let handled = Widget::handle(&mut self.edit, event, cx);
                self.edit.select_all();
                return handled;
            }
            _ => {}
        }
        Widget::handle(&mut self.edit, event, cx)
    }

    fn interactive(&self) -> bool {
        true
    }

    fn focusable(&self) -> bool {
        self.edit.focusable()
    }

    fn enabled(&self) -> bool {
        self.edit.enabled()
    }

    fn text(&self) -> Option<String> {
        Some(self.edit.value())
    }

    fn text_input(&self) -> Option<InputPurpose> {
        self.edit.text_input()
    }
}

impl fmt::Debug for NumberBox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NumberBox")
            .field("edit", &self.edit)
            .field("config", &self.shared.config())
            .field("spin_buttons", &self.spin_buttons)
            .finish_non_exhaustive()
    }
}

/// Os botões de incremento sobre o campo: não tiram o foco dele.
struct SpinPopup {
    common: Common,
    open: Rc<dyn Fn() -> bool>,
    children: Vec<Box<dyn Widget>>,
}

impl Widget for SpinPopup {
    fn kind(&self) -> &'static str {
        "number_box_spin"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn names_children(&self) -> bool {
        true
    }

    fn visible(&self) -> bool {
        (self.open)()
    }

    fn look(&self) -> Look {
        Look::Popup(PopupKind::NumberBox)
    }

    fn flex(&self) -> Option<Flex> {
        Some(Flex {
            align: Align::Center,
            gap: POPUP_GAP,
            padding: Insets::all(POPUP_PADDING),
            ..Flex::default()
        })
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        std::mem::take(&mut self.children)
    }

    fn layer(&self) -> Option<Layer> {
        let size = Size::new(
            POPUP_BUTTON + 2.0 * POPUP_PADDING,
            2.0 * POPUP_BUTTON + POPUP_GAP + 2.0 * POPUP_PADDING,
        );
        Some(Layer {
            input: false,
            pointer: true,
            light_dismiss: false,
            placement: Placement::End,
            offset: POPUP_OFFSET,
            gap: 0.0,
            min_size: size,
            max_size: size,
            ..Layer::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use kurbo::{Point, Rect};
    use tomni_ui_reactive::prelude::*;

    use super::*;
    use crate::cx::{FixedMetrics, MemoryClipboard};
    use crate::event::{FocusSource, Modifiers};

    fn send(w: &mut dyn Widget, event: Event) -> bool {
        let mut metrics = FixedMetrics::default();
        let mut clipboard = MemoryClipboard::default();
        let mut cx = EventCx::new(
            Rect::new(0.0, 0.0, 200.0, 32.0),
            Point::ZERO,
            Modifiers::NONE,
            &mut metrics,
            &mut clipboard,
        );
        w.handle(&event, &mut cx)
    }

    fn with(test: impl FnOnce()) {
        let scope = Scope::root();
        scope.run(test);
        scope.clear();
    }

    #[test]
    fn formata_com_virgula_e_sem_zeros() {
        assert_eq!(format_number(1.5, 2), "1,5");
        assert_eq!(format_number(2.0, 2), "2");
        assert_eq!(format_number(1.005, 0), "1");
        assert_eq!(format_number(-0.001, 2), "0");
        assert_eq!(format_number(-0.25, 2), "-0,25");
        assert_eq!(format_number(1234.5, 2), "1234,5");
    }

    #[test]
    fn mostra_dez_digitos_significativos() {
        assert_eq!(format_display(0.1 + 0.2), "0,3");
        assert_eq!(format_display(2.0), "2");
        assert_eq!(format_display(-0.0), "0");
        assert_eq!(format_display(1.0 / 3.0), "0,3333333333");
        assert_eq!(format_display(1234.5), "1234,5");
    }

    #[test]
    fn le_virgula_ou_ponto() {
        assert_eq!(parse_number("1,5"), Some(1.5));
        assert_eq!(parse_number(" 1.5 "), Some(1.5));
        assert_eq!(parse_number("1.234,5"), Some(1234.5));
        assert_eq!(parse_number("-3"), Some(-3.0));
        assert_eq!(parse_number(""), None);
        assert_eq!(parse_number("abc"), None);
        assert_eq!(parse_number("inf"), None);
        assert_eq!(parse_number("1e3"), None);
    }

    #[test]
    fn confere_ao_perder_o_foco() {
        with(|| {
            let v = signal(5.0);
            let mut campo = NumberBox::bind(v).min(0).max(10);
            assert_eq!(campo.value(), "5");
            send(&mut campo, Event::FocusIn(FocusSource::Keyboard));
            send(&mut campo, Event::Text("50".into()));
            assert_eq!(campo.value(), "50", "o foco seleciona o texto todo");
            assert_eq!(v.get(), 5.0, "só vale quando o foco sai");
            send(&mut campo, Event::FocusOut);
            assert_eq!(v.get(), 10.0, "recortado no máximo");
            assert_eq!(campo.value(), "10");

            send(&mut campo, Event::FocusIn(FocusSource::Keyboard));
            send(&mut campo, Event::Text("x".into()));
            send(&mut campo, Event::KeyDown(Key::Enter));
            assert_eq!(v.get(), 10.0);
            assert_eq!(campo.value(), "10", "texto inválido volta ao valor");
        });
    }

    #[test]
    fn opcional_aceita_vazio() {
        with(|| {
            let v = signal(Some(2.5));
            let mut campo = NumberBox::bind_optional(v);
            assert_eq!(campo.value(), "2,5");
            send(&mut campo, Event::FocusIn(FocusSource::Keyboard));
            for _ in 0..3 {
                send(&mut campo, Event::KeyDown(Key::Backspace));
            }
            send(&mut campo, Event::FocusOut);
            assert_eq!(v.get(), None);
            assert_eq!(campo.value(), "");
        });
    }

    #[test]
    fn setas_paginas_e_roda() {
        with(|| {
            let v = signal(0.0);
            let mut campo = NumberBox::bind(v).max(15);
            send(&mut campo, Event::FocusIn(FocusSource::Keyboard));
            send(&mut campo, Event::KeyDown(Key::Up));
            assert_eq!(v.get(), 1.0);
            send(&mut campo, Event::KeyDown(Key::PageUp));
            assert_eq!(v.get(), 11.0);
            send(&mut campo, Event::KeyDown(Key::PageUp));
            assert_eq!(v.get(), 15.0, "para no máximo");
            send(&mut campo, Event::Wheel { up: false });
            assert_eq!(v.get(), 14.0);
            assert_eq!(campo.value(), "14");
            send(&mut campo, Event::KeyDown(Key::PageDown));
            send(&mut campo, Event::KeyDown(Key::Down));
            assert_eq!(v.get(), 3.0);
        });
    }

    #[test]
    fn passo_parte_do_texto_digitado() {
        with(|| {
            let v = signal(1.0);
            let mut campo = NumberBox::bind(v).small_change(0.1);
            send(&mut campo, Event::FocusIn(FocusSource::Keyboard));
            send(&mut campo, Event::Text("0,2".into()));
            send(&mut campo, Event::KeyDown(Key::Up));
            assert_eq!(
                v.get(),
                0.2 + 0.1,
                "sem precision, o valor não é arredondado"
            );
            assert_eq!(
                campo.value(),
                "0,3",
                "o campo mostra 10 dígitos significativos"
            );

            let w = signal(1.0);
            let mut campo = NumberBox::bind(w).small_change(0.1).precision(1);
            send(&mut campo, Event::FocusIn(FocusSource::Keyboard));
            send(&mut campo, Event::Text("0,2".into()));
            send(&mut campo, Event::KeyDown(Key::Up));
            assert_eq!(w.get(), 0.3, "com precision, arredonda nas casas");
        });
    }

    #[test]
    fn esc_volta_ao_valor() {
        with(|| {
            let v = signal(5.0);
            let mut campo = NumberBox::bind(v);
            send(&mut campo, Event::FocusIn(FocusSource::Keyboard));
            send(&mut campo, Event::Text("8".into()));
            assert!(send(&mut campo, Event::KeyDown(Key::Escape)));
            assert_eq!(campo.value(), "5");
            assert_eq!(v.get(), 5.0);
            assert!(
                !send(&mut campo, Event::KeyDown(Key::Escape)),
                "sem mudança, o Esc passa adiante"
            );
        });
    }

    #[test]
    fn sem_foco_a_roda_passa() {
        with(|| {
            let v = signal(0.0);
            let mut campo = NumberBox::bind(v);
            assert!(!send(&mut campo, Event::Wheel { up: true }));
            assert_eq!(v.get(), 0.0);
        });
    }

    #[test]
    fn partes_com_ids_derivados() {
        with(|| {
            let v = signal(0.0);
            let ids = |campo: &mut NumberBox| -> Vec<_> {
                campo
                    .take_children()
                    .iter()
                    .map(|w| w.common().id.clone())
                    .collect()
            };
            let mut campo = NumberBox::bind(v).id("idade");
            assert_eq!(ids(&mut campo), [Some("idade-limpar".to_owned())]);

            let mut campo = NumberBox::bind(v)
                .id("idade")
                .spin_buttons(SpinButtons::Compact);
            assert_eq!(
                ids(&mut campo),
                [
                    Some("idade-limpar".to_owned()),
                    None,
                    Some("idade-incremento".to_owned()),
                ]
            );
            assert!(!campo.is_open());
            send(&mut campo, Event::FocusIn(FocusSource::Pointer));
            assert!(campo.is_open());
        });
    }
}
