//! `Border` e `Card`: um filho dentro de um fundo com borda e cantos arredondados.

use std::fmt;

use crate::button::ButtonState;
use crate::color::{Brush, ThemeColor, Token};
use crate::cx::EventCx;
use crate::event::Event;
use crate::press::Pressable;
use crate::prop::{IntoProp, Prop};
use crate::widget::{Common, Flex, Insets, Look, View, Widget, common_builders, density_builder};

/// Espaço entre a borda do `Card` e o conteúdo, como o `Padding` dos cards da Gallery do
/// WinUI.
pub const CARD_PADDING: f64 = 16.0;

/// Raio dos cantos do `Card`: o `OverlayCornerRadius` do WinUI.
pub const CARD_CORNER_RADIUS: f64 = 8.0;

/// Espessura da borda do `Card`.
pub const CARD_BORDER: f64 = 1.0;

/// Tudo o que o `Drawer` precisa saber do `Border` para pintá-lo.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BorderVisual {
    /// Fundo, por baixo do conteúdo. `None` é transparente.
    pub background: Option<Brush>,
    /// Borda: o pincel e a espessura, em pixels, por dentro do retângulo.
    pub border: Option<(Brush, f64)>,
    /// Raio dos cantos, em pixels.
    pub corner_radius: f64,
}

/// Um filho dentro de um fundo, uma borda e cantos arredondados, como o `Border` do WinUI.
///
/// O fundo e a borda aceitam uma cor fixa, um token do tema (que acompanha a troca de tema) ou
/// um gradiente linear:
///
/// ```
/// use tomni_ui_core::{Border, Brush, Color, Text, Token};
///
/// let faixa = Border::new()
///     .background(Brush::linear(
///         (0.0, 0.0),
///         (1.0, 1.0),
///         [(0.0, Color::hex(0x003E92)), (1.0, Color::hex(0x0078D4))],
///     ))
///     .border(Token::CardStrokeColorDefault, 1)
///     .corner_radius(8)
///     .padding(24)
///     .child(Text::new("Novidades").title().color(Color::WHITE));
/// ```
///
/// A borda fica por dentro do retângulo, e o `padding` conta a partir dela. O conteúdo não é
/// cortado nos cantos. Chamar `.child` de novo troca o filho anterior.
pub struct Border {
    common: Common,
    visual: BorderVisual,
    padding: Insets,
    child: Option<Box<dyn Widget>>,
}

impl Border {
    /// Borda vazia: sem fundo, sem borda, sem raio e sem padding.
    pub fn new() -> Self {
        Self {
            common: Common::default(),
            visual: BorderVisual::default(),
            padding: Insets::ZERO,
            child: None,
        }
    }

    /// Fundo: uma [`Color`](crate::Color), um [`Token`] ou um [`Brush`].
    pub fn background(mut self, brush: impl Into<Brush>) -> Self {
        self.visual.background = Some(brush.into());
        self
    }

    /// Borda com o pincel `brush` e `thickness` pixels de espessura.
    pub fn border(mut self, brush: impl Into<Brush>, thickness: impl Into<f64>) -> Self {
        self.visual.border = Some((brush.into(), thickness.into()));
        self
    }

    /// Raio dos cantos, em pixels.
    pub fn corner_radius(mut self, radius: impl Into<f64>) -> Self {
        self.visual.corner_radius = radius.into();
        self
    }

    /// Espaço entre a borda e o filho: um número para os quatro lados ou [`Insets`].
    pub fn padding(mut self, padding: impl Into<Insets>) -> Self {
        self.padding = padding.into();
        self
    }

    /// Conteúdo. Chamar de novo troca o anterior.
    pub fn child(mut self, child: impl View) -> Self {
        self.child = Some(Box::new(child));
        self
    }

    /// Fundo, borda e raio.
    pub fn visual(&self) -> &BorderVisual {
        &self.visual
    }
}

impl Default for Border {
    fn default() -> Self {
        Self::new()
    }
}

common_builders!(Border);

impl Widget for Border {
    fn kind(&self) -> &'static str {
        "border"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::Border(self.visual.clone())
    }

    fn flex(&self) -> Option<Flex> {
        let border = self.visual.border.as_ref().map_or(0.0, |(_, t)| *t);
        Some(Flex {
            padding: self.padding + Insets::all(border),
            ..Flex::default()
        })
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        self.child.take().into_iter().collect()
    }
}

impl fmt::Debug for Border {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Border")
            .field("common", &self.common)
            .field("visual", &self.visual)
            .field("padding", &self.padding)
            .finish_non_exhaustive()
    }
}

/// Tudo o que o `Drawer` precisa saber do `Card` para pintá-lo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CardVisual {
    /// Se tem `on_click`: só então tem hover e pressionado.
    pub clickable: bool,
    /// Estado, o mesmo do botão. Sem clique, fica em `Rest` (ou `Disabled`).
    pub state: ButtonState,
    /// Anel de foco: só com foco vindo do teclado, e nunca desabilitado.
    pub focus_ring: bool,
}

/// Um card, como os da Gallery do WinUI: um [`Border`] com fundo
/// `CardBackgroundFillColorDefault`, borda `CardStrokeColorDefault` de 1 px, raio 8 e padding
/// 16.
///
/// ```
/// use tomni_ui_core::{Card, Column, Text};
///
/// let card = Card::new()
///     .id("card-tema")
///     .label("Tema escuro")
///     .on_click(|| println!("abrir"))
///     .child(
///         Column::new()
///             .gap(4)
///             .child(Text::new("Tema escuro").body_strong())
///             .child(Text::new("Menos luz à noite.").secondary().wrap().max_lines(2)),
///     );
/// ```
///
/// Com `.on_click`, o card fica clicável: tem hover, pressionado e foco pelo teclado, e Enter e
/// Espaço o ativam, como o botão. As cores dos estados são as do `SettingsCard` do Windows
/// Community Toolkit, o card clicável das Configurações do Windows 11 (o WinUI não tem um
/// controle de card). Pressionado e desabilitado, os `Text` de dentro sem cor própria ficam
/// com a cor secundária e a desabilitada, como o `Foreground` do WinUI.
///
/// O conteúdo de um card clicável é livre, então ele pede um `.label` para a árvore, os testes
/// e o leitor de tela; sem ele, o lint dá o aviso `missing-label`.
pub struct Card {
    common: Common,
    padding: Insets,
    label: Option<Prop<String>>,
    disabled: Prop<bool>,
    on_click: Option<Box<dyn FnMut()>>,
    press: Pressable,
    child: Option<Box<dyn Widget>>,
}

impl Card {
    /// Card vazio, sem clique.
    pub fn new() -> Self {
        Self {
            common: Common::default(),
            padding: Insets::all(CARD_PADDING),
            label: None,
            disabled: Prop::Value(false),
            on_click: None,
            press: Pressable::default(),
            child: None,
        }
    }

    /// Conteúdo. Chamar de novo troca o anterior.
    pub fn child(mut self, child: impl View) -> Self {
        self.child = Some(Box::new(child));
        self
    }

    /// Espaço entre a borda e o conteúdo. O padrão é 16.
    pub fn padding(mut self, padding: impl Into<Insets>) -> Self {
        self.padding = padding.into();
        self
    }

    /// Torna o card clicável: `f` roda no clique, no Enter e no Espaço.
    pub fn on_click(mut self, f: impl FnMut() + 'static) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }

    /// Nome do card na árvore, nos testes e no leitor de tela (fixo, closure ou signal).
    pub fn label<M>(mut self, label: impl IntoProp<String, M>) -> Self {
        self.label = Some(label.into_prop());
        self
    }

    /// Desabilita o card (valor fixo, closure ou signal): não recebe foco nem cliques, e o
    /// texto de dentro fica com a cor desabilitada.
    pub fn disabled<M>(mut self, disabled: impl IntoProp<bool, M>) -> Self {
        self.disabled = disabled.into_prop();
        self
    }

    /// Se tem `on_click`.
    pub fn is_clickable(&self) -> bool {
        self.on_click.is_some()
    }

    /// Se está desabilitado agora.
    pub fn is_disabled(&self) -> bool {
        self.disabled.get()
    }

    /// Estado para o `Drawer`.
    pub fn visual(&self) -> CardVisual {
        let clickable = self.is_clickable();
        if self.is_disabled() {
            return CardVisual {
                clickable,
                state: ButtonState::Disabled,
                focus_ring: false,
            };
        }
        if !clickable {
            return CardVisual::default();
        }
        CardVisual {
            clickable,
            state: self.press.state(),
            focus_ring: self.press.focus_ring(),
        }
    }

    /// Trata um evento. Devolve `true` se o card o consumiu.
    pub fn handle(&mut self, event: &Event) -> bool {
        if self.is_disabled() || !self.is_clickable() {
            self.press.reset();
            return false;
        }
        let (consumed, clicked) = self.press.handle(event, true);
        if clicked && let Some(f) = &mut self.on_click {
            f();
        }
        consumed
    }
}

impl Default for Card {
    fn default() -> Self {
        Self::new()
    }
}

common_builders!(Card);
density_builder!(Card);

impl Widget for Card {
    fn kind(&self) -> &'static str {
        "card"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::Card(self.visual())
    }

    fn flex(&self) -> Option<Flex> {
        Some(Flex {
            padding: self.padding + Insets::all(CARD_BORDER),
            ..Flex::default()
        })
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        self.child.take().into_iter().collect()
    }

    fn handle(&mut self, event: &Event, _: &mut EventCx) -> bool {
        Card::handle(self, event)
    }

    fn interactive(&self) -> bool {
        self.is_clickable()
    }

    fn focusable(&self) -> bool {
        self.is_clickable() && !self.is_disabled()
    }

    fn enabled(&self) -> bool {
        !self.is_disabled()
    }

    fn text(&self) -> Option<String> {
        self.label.as_ref().map(Prop::get)
    }

    fn foreground(&self) -> Option<ThemeColor> {
        // `SettingsCardForegroundPressed` e `SettingsCardForegroundDisabled`.
        match self.visual().state {
            ButtonState::Rest | ButtonState::Hover => None,
            ButtonState::Pressed => Some(Token::TextFillColorSecondary.into()),
            ButtonState::Disabled => Some(Token::TextFillColorDisabled.into()),
        }
    }

    fn passes_foreground(&self) -> bool {
        true
    }
}

impl fmt::Debug for Card {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Card")
            .field("common", &self.common)
            .field("label", &self.label)
            .field("disabled", &self.disabled)
            .field("visual", &self.visual())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;
    use crate::Text;
    use crate::color::Color;
    use crate::event::{FocusSource, Key, PointerKind::Mouse};

    fn click(card: &mut Card) {
        card.handle(&Event::PointerDown {
            kind: Mouse,
            clicks: 1,
        });
        card.handle(&Event::PointerUp {
            kind: Mouse,
            inside: true,
        });
    }

    #[test]
    fn border_soma_a_espessura_ao_padding() {
        let mut border = Border::new()
            .background(Color::WHITE)
            .border(Token::CardStrokeColorDefault, 2)
            .corner_radius(8)
            .padding(10)
            .child(Text::new("a"))
            .child(Text::new("b"));
        assert_eq!(border.flex().unwrap().padding, Insets::all(12.0));
        assert_eq!(border.visual().corner_radius, 8.0);
        assert_eq!(
            border.take_children().len(),
            1,
            "o segundo troca o primeiro"
        );
    }

    #[test]
    fn card_clicavel_responde_ao_clique_enter_e_espaco() {
        let cliques = Rc::new(Cell::new(0));
        let c = Rc::clone(&cliques);
        let mut card = Card::new().on_click(move || c.set(c.get() + 1));
        assert!(card.focusable() && card.interactive());
        click(&mut card);
        card.handle(&Event::FocusIn(FocusSource::Keyboard));
        assert!(card.visual().focus_ring);
        card.handle(&Event::KeyDown(Key::Enter));
        card.handle(&Event::KeyDown(Key::Space));
        assert_eq!(card.visual().state, ButtonState::Pressed);
        assert_eq!(
            card.foreground(),
            Some(Token::TextFillColorSecondary.into())
        );
        card.handle(&Event::KeyUp(Key::Space));
        assert_eq!(cliques.get(), 3);
        assert_eq!(card.foreground(), None);
    }

    #[test]
    fn card_sem_clique_nao_tem_estados() {
        let mut card = Card::new().child(Text::new("a"));
        assert!(!card.focusable() && !card.interactive());
        click(&mut card);
        assert_eq!(card.visual(), CardVisual::default());
        assert_eq!(card.flex().unwrap().padding, Insets::all(17.0));
        let card = Card::new().disabled(true);
        assert_eq!(card.visual().state, ButtonState::Disabled);
        assert_eq!(card.foreground(), Some(Token::TextFillColorDisabled.into()));
    }
}
