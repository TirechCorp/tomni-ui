//! Caixa de seleção no visual da `CheckBox` do WinUI.

use tomni_ui_core::{ButtonState, CheckBoxVisual};
use tomni_ui_render::kurbo::{Point, Rect, RoundedRect, Size};
use tomni_ui_render::peniko::Color;
use tomni_ui_render::{Canvas, CanvasExt, Paint};
use tomni_ui_text::TextStyle;

use crate::{FluentDrawer, TextInfo, Tokens, icons, metrics, ring};

/// `CheckBoxSize`: lado da caixa.
const BOX: f64 = 20.0;
/// Lado do ícone de marcado. O WinUI usa a Segoe Fluent Icons em 12 px (`CheckBoxGlyphSize`),
/// cujo traço ocupa uns 10 px; o `checkmark_12` tem folga em volta, e em 16 px dá o mesmo
/// traço.
const GLYPH: f64 = 16.0;
/// Onde começa o rótulo: a caixa e o `CheckBoxPadding` da esquerda (8).
const LABEL_X: f64 = BOX + 8.0;
/// `CheckBoxPadding` de cima.
const LABEL_Y: f64 = 5.0;
/// `CheckBoxMinWidth`.
const MIN_WIDTH: f64 = 120.0;

struct Colors {
    fill: Color,
    stroke: Color,
    glyph: Option<Color>,
    label: Color,
}

fn colors(t: &Tokens, visual: CheckBoxVisual) -> Colors {
    use ButtonState::*;
    let label = if visual.state == Disabled {
        t.text_fill_disabled
    } else {
        t.text_fill_primary
    };
    if visual.checked {
        let (fill, stroke, glyph) = match visual.state {
            Rest => (
                t.accent_fill_default,
                t.accent_fill_default,
                t.text_on_accent_primary,
            ),
            Hover => (
                t.accent_fill_secondary,
                t.accent_fill_secondary,
                t.text_on_accent_primary,
            ),
            Pressed => (
                t.accent_fill_tertiary,
                t.accent_fill_tertiary,
                t.text_on_accent_secondary,
            ),
            Disabled => (
                t.accent_fill_disabled,
                t.control_strong_stroke_disabled,
                t.text_on_accent_disabled,
            ),
        };
        Colors {
            fill,
            stroke,
            glyph: Some(glyph),
            label,
        }
    } else {
        let (fill, stroke) = match visual.state {
            Rest => (
                t.control_alt_fill_secondary,
                t.control_strong_stroke_default,
            ),
            Hover => (t.control_alt_fill_tertiary, t.control_strong_stroke_default),
            Pressed => (
                t.control_alt_fill_quarternary,
                t.control_strong_stroke_disabled,
            ),
            Disabled => (
                t.control_alt_fill_disabled,
                t.control_strong_stroke_disabled,
            ),
        };
        Colors {
            fill,
            stroke,
            glyph: None,
            label,
        }
    }
}

/// Canto superior esquerdo do rótulo; acompanha o centro quando a caixa é mais alta.
fn label_origin(rect: Rect) -> Point {
    Point::new(
        rect.x0 + LABEL_X,
        rect.y0 + LABEL_Y + crate::center_offset(rect),
    )
}

pub(crate) fn measure(drawer: &FluentDrawer, label: &str) -> Size {
    let width = f64::from(
        drawer
            .fonts
            .borrow_mut()
            .layout(label, TextStyle::BODY, None)
            .width(),
    );
    Size::new(
        (LABEL_X + width).ceil().max(MIN_WIDTH),
        metrics::CONTROL_HEIGHT,
    )
}

pub(crate) fn draw(
    drawer: &FluentDrawer,
    canvas: &mut dyn Canvas,
    rect: Rect,
    label: &str,
    visual: CheckBoxVisual,
) {
    let colors = colors(&drawer.tokens, visual);
    let radius = metrics::CONTROL_CORNER_RADIUS;
    let border = metrics::BORDER;
    let y = rect.y0 + ((rect.height() - BOX) / 2.0).round();
    let square = Rect::new(rect.x0, y, rect.x0 + BOX, y + BOX);
    // Fundo e borda sem sobreposição: as duas cores podem ser translúcidas.
    let inner = square.inset(-border);
    canvas.fill(&RoundedRect::from_rect(inner, radius - border), colors.fill);
    canvas.fill_path(
        &ring(square, radius, inner, radius - border),
        &Paint::Solid(colors.stroke),
    );
    if let Some(glyph) = colors.glyph {
        // Centralizado. O WinUI desce o glifo 1 px (`Margin="0,1,0,-1"`) para compensar a
        // Segoe; o ícone já vem centrado no quadro.
        let offset = (BOX - GLYPH) / 2.0;
        let origin = Point::new(square.x0 + offset, square.y0 + offset);
        canvas.fill_path(
            &icons::CHECKMARK_12.path(origin, GLYPH),
            &Paint::Solid(glyph),
        );
    }

    let text = drawer
        .fonts
        .borrow_mut()
        .layout(label, TextStyle::BODY, None);
    text.draw(canvas, label_origin(rect), &Paint::Solid(colors.label));

    if visual.focus_ring {
        // `CheckBoxFocusVisualMargin`: 7 px dos lados e 3 em cima e embaixo.
        crate::draw_focus_ring(canvas, rect.inflate(4.0, 0.0), radius, &drawer.tokens);
    }
}

pub(crate) fn text_info(
    drawer: &FluentDrawer,
    rect: Rect,
    label: &str,
    visual: CheckBoxVisual,
) -> Vec<TextInfo> {
    if label.is_empty() {
        return Vec::new();
    }
    let t = &drawer.tokens;
    let colors = colors(t, visual);
    vec![crate::text::line_info(
        drawer,
        label,
        TextStyle::BODY,
        label_origin(rect),
        (colors.label, drawer.backdrop()),
        Rect::new(rect.x0 + LABEL_X, rect.y0, rect.x1, rect.y1),
    )]
}
