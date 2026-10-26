//! O crachá com o número de um painel do `TerminalPanes` (M36): uma sobreposição
//! ([`Placement::Over`]) no centro do painel, que o app liga para o usuário ver qual painel é
//! qual.

use std::fmt;

use tomni_ui_core::kurbo::Size;
use tomni_ui_core::{
    Align, BorderVisual, Brush, Common, Direction, Flex, Insets, Justify, Layer, Look, Placement,
    Prop, Text, Token, Widget,
};

/// Lado mínimo do crachá, em px: o número cabe com folga e dá para ler de longe.
pub(crate) const BADGE_SIZE: f64 = 120.0;
/// Folga entre o crachá e a borda do painel, de cada lado: um painel menor que o crachá mais
/// essa folga não mostra o crachá.
const BADGE_MARGIN: f64 = 8.0;
/// Raio dos cantos: o `OverlayCornerRadius` do WinUI.
const BADGE_RADIUS: f64 = 8.0;

/// O crachá de um painel: um quadrado arredondado com o número em rampa Display.
///
/// Normal, tem o fundo sólido da janela e uma borda fina; realçado, o fundo é a cor de
/// destaque, o texto vai em cima dela e a borda some.
pub(crate) struct PaneBadge {
    common: Common,
    number: usize,
    highlighted: Prop<bool>,
    children: Vec<Box<dyn Widget>>,
}

impl PaneBadge {
    /// O crachá do painel `pane` (a partir de 0). `id` é o da camada; `shown` e `highlighted`
    /// são lidos a cada pintura.
    pub(crate) fn new(
        id: Option<String>,
        pane: usize,
        shown: Prop<bool>,
        highlighted: Prop<bool>,
    ) -> Self {
        let number = pane + 1;
        let on = {
            let highlighted = highlighted.clone();
            move || highlighted.get()
        };
        let off = {
            let highlighted = highlighted.clone();
            move || !highlighted.get()
        };
        let mut plain = Text::new(number.to_string())
            .display()
            .visible(off)
            .color(Token::TextFillColorPrimary);
        let mut accent = Text::new(number.to_string())
            .display()
            .visible(on)
            .color(Token::TextOnAccentFillColorPrimary);
        if let Some(id) = &id {
            plain = plain.id(format!("{id}-texto"));
            accent = accent.id(format!("{id}-texto-realce"));
        }
        let mut common = Common::default();
        common.id = id;
        common.visible = Some(shown);
        Self {
            common,
            number,
            highlighted,
            children: vec![Box::new(plain), Box::new(accent)],
        }
    }
}

impl Widget for PaneBadge {
    fn kind(&self) -> &'static str {
        "pane_badge"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn text(&self) -> Option<String> {
        Some(format!("Painel {}", self.number))
    }

    fn look(&self) -> Look {
        let visual = if self.highlighted.get() {
            BorderVisual {
                background: Some(Brush::token(Token::AccentFillColorDefault)),
                border: None,
                corner_radius: BADGE_RADIUS,
            }
        } else {
            BorderVisual {
                background: Some(Brush::token(Token::SolidBackgroundFillColorBase)),
                border: Some((Brush::token(Token::SurfaceStrokeColorDefault), 1.0)),
                corner_radius: BADGE_RADIUS,
            }
        };
        Look::Border(visual)
    }

    fn flex(&self) -> Option<Flex> {
        Some(Flex {
            direction: Direction::Column,
            padding: Insets::all(1.0),
            align: Align::Center,
            justify: Justify::Center,
            ..Flex::default()
        })
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        std::mem::take(&mut self.children)
    }

    fn layer(&self) -> Option<Layer> {
        Some(Layer {
            modal: false,
            light_dismiss: false,
            input: false,
            pointer: false,
            placement: Placement::Over,
            gap: BADGE_MARGIN,
            min_size: Size::new(BADGE_SIZE, BADGE_SIZE),
            ..Layer::default()
        })
    }
}

impl fmt::Debug for PaneBadge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PaneBadge")
            .field("common", &self.common)
            .field("number", &self.number)
            .finish_non_exhaustive()
    }
}
