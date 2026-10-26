//! Botões dentro de um campo, como os do `TextBox`, do `PasswordBox` e do `NumberBox` do
//! WinUI: fundo só com o ponteiro, dentro da margem do botão, e o ícone na cor secundária do
//! texto.

use tomni_ui_core::{ButtonState, FieldButtonVisual, FieldGlyph};
use tomni_ui_render::kurbo::{Point, Rect, RoundedRect, Size};
use tomni_ui_render::peniko::Color;
use tomni_ui_render::{Canvas, CanvasExt, Paint};

use crate::{Tokens, icons};

/// Altura natural do botão normal; no campo ele estica até a altura do campo.
const HEIGHT: f64 = 32.0;
/// Lado do botão grande (o do popup do `NumberBox`).
const LARGE: f64 = 36.0;
/// Ícone do botão normal (`TextBoxIconFontSize` do WinUI).
const GLYPH: f64 = 12.0;
/// Ícone do botão grande.
const GLYPH_LARGE: f64 = 16.0;
/// Raio do fundo (`ControlCornerRadius`).
const RADIUS: f64 = 4.0;

pub(crate) fn measure(visual: FieldButtonVisual) -> Size {
    // A largura vem do widget (`width`); esta é só a natural.
    if visual.large {
        Size::new(LARGE, LARGE)
    } else {
        Size::new(HEIGHT, HEIGHT)
    }
}

pub(crate) fn draw(canvas: &mut dyn Canvas, rect: Rect, visual: FieldButtonVisual, t: &Tokens) {
    let (fill, glyph) = match visual.state {
        ButtonState::Rest => (Color::TRANSPARENT, t.text_fill_secondary),
        ButtonState::Hover => (t.subtle_fill_secondary, t.text_fill_secondary),
        ButtonState::Pressed => (t.subtle_fill_tertiary, t.text_fill_tertiary),
        ButtonState::Disabled => (Color::TRANSPARENT, t.text_fill_disabled),
    };
    let margin = visual.margin;
    let area = Rect::new(
        rect.x0 + margin.left,
        rect.y0 + margin.top,
        (rect.x1 - margin.right).max(rect.x0 + margin.left),
        (rect.y1 - margin.bottom).max(rect.y0 + margin.top),
    );
    canvas.fill(&RoundedRect::from_rect(area, RADIUS), fill);
    let size = if visual.large { GLYPH_LARGE } else { GLYPH };
    let icon = match (visual.glyph, visual.large) {
        (FieldGlyph::Reveal, _) => icons::EYE_12,
        (FieldGlyph::Clear, _) => icons::DISMISS_12,
        (FieldGlyph::Up, false) => icons::CHEVRON_UP_12,
        (FieldGlyph::Up, true) => icons::CHEVRON_UP_16,
        (FieldGlyph::Down, false) => icons::CHEVRON_DOWN_12,
        (FieldGlyph::Down, true) => icons::CHEVRON_DOWN_16,
        (FieldGlyph::UpDown, _) => icons::CHEVRON_UP_DOWN_16,
        (FieldGlyph::Symbol(symbol), _) => {
            let square = crate::icon::centered(area, size);
            crate::icon::draw_symbol(canvas, symbol, square, glyph);
            return;
        }
    };
    let center = area.center();
    let origin = Point::new(
        (center.x - size / 2.0).round(),
        (center.y - size / 2.0).round(),
    );
    canvas.fill_path(&icon.path(origin, size), &Paint::Solid(glyph));
}
