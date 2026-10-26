//! Opção de um `RadioButtons` no visual do `RadioButton` do WinUI.

use tomni_ui_core::{ButtonState, RadioButtonVisual};
use tomni_ui_render::kurbo::{Circle, Point, Rect, Shape, Size};
use tomni_ui_render::peniko::Color;
use tomni_ui_render::{Canvas, CanvasExt, PATH_TOLERANCE, Paint};
use tomni_ui_text::TextStyle;

use crate::{FluentDrawer, TextInfo, Tokens, metrics};

/// Diâmetro do círculo (`RadioButtonOuterEllipse`, 20 × 20).
const CIRCLE: f64 = 20.0;
/// Onde começa o rótulo: o círculo e o `RadioButtonPadding` da esquerda (8).
const LABEL_X: f64 = CIRCLE + 8.0;
/// `RadioButtonPadding` de cima.
const LABEL_Y: f64 = 5.0;
/// `RadioButtonMinWidth`.
const MIN_WIDTH: f64 = 120.0;

struct Colors {
    fill: Color,
    stroke: Color,
    /// Miolo: a cor e o diâmetro.
    glyph: Option<(Color, f64)>,
    label: Color,
}

fn colors(t: &Tokens, visual: RadioButtonVisual) -> Colors {
    use ButtonState::*;
    let label = if visual.state == Disabled {
        t.text_fill_disabled
    } else {
        t.text_fill_primary
    };
    if visual.checked {
        // O miolo cresce no hover e encolhe no press (`CheckGlyph` de 12, 14 e 10).
        let (fill, size) = match visual.state {
            Rest => (t.accent_fill_default, 12.0),
            Hover => (t.accent_fill_secondary, 14.0),
            Pressed => (t.accent_fill_tertiary, 10.0),
            Disabled => (t.accent_fill_disabled, 12.0),
        };
        Colors {
            fill,
            stroke: fill,
            glyph: Some((t.text_on_accent_primary, size)),
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
            // Desmarcada e pressionada, já mostra o miolo de 10 que vai ficar.
            glyph: (visual.state == Pressed).then_some((t.text_on_accent_primary, 10.0)),
            label,
        }
    }
}

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
    visual: RadioButtonVisual,
) {
    let colors = colors(&drawer.tokens, visual);
    let border = metrics::BORDER;
    let center = Point::new(
        rect.x0 + CIRCLE / 2.0,
        rect.y0 + ((rect.height() - CIRCLE) / 2.0).round() + CIRCLE / 2.0,
    );
    let radius = CIRCLE / 2.0;
    // Fundo e borda sem sobreposição: as duas cores podem ser translúcidas.
    canvas.fill(&Circle::new(center, radius - border), colors.fill);
    let mut stroke = Circle::new(center, radius).to_path(PATH_TOLERANCE);
    stroke.extend(
        Circle::new(center, radius - border)
            .to_path(PATH_TOLERANCE)
            .reverse_subpaths(),
    );
    canvas.fill_path(&stroke, &Paint::Solid(colors.stroke));
    if let Some((color, size)) = colors.glyph {
        canvas.fill(&Circle::new(center, size / 2.0), color);
    }

    drawer
        .fonts
        .borrow_mut()
        .layout(label, TextStyle::BODY, None)
        .draw(canvas, label_origin(rect), &Paint::Solid(colors.label));

    if visual.focus_ring {
        // `RadioButtonFocusVisualMargin`, como o da `CheckBox`.
        crate::draw_focus_ring(
            canvas,
            rect.inflate(4.0, 0.0),
            metrics::CONTROL_CORNER_RADIUS,
            &drawer.tokens,
        );
    }
}

pub(crate) fn text_info(
    drawer: &FluentDrawer,
    rect: Rect,
    label: &str,
    visual: RadioButtonVisual,
) -> Vec<TextInfo> {
    if label.is_empty() {
        return Vec::new();
    }
    let colors = colors(&drawer.tokens, visual);
    vec![crate::text::line_info(
        drawer,
        label,
        TextStyle::BODY,
        label_origin(rect),
        (colors.label, drawer.backdrop()),
        Rect::new(rect.x0 + LABEL_X, rect.y0, rect.x1, rect.y1),
    )]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;

    #[test]
    fn miolo_so_marcada_ou_pressionada() {
        let t = Tokens::new(Theme::Light);
        let visual = |checked, state| RadioButtonVisual {
            checked,
            state,
            focus_ring: false,
        };
        assert!(colors(&t, visual(false, ButtonState::Rest)).glyph.is_none());
        assert!(
            colors(&t, visual(false, ButtonState::Hover))
                .glyph
                .is_none()
        );
        assert_eq!(
            colors(&t, visual(false, ButtonState::Pressed))
                .glyph
                .map(|g| g.1),
            Some(10.0)
        );
        assert_eq!(
            colors(&t, visual(true, ButtonState::Hover))
                .glyph
                .map(|g| g.1),
            Some(14.0)
        );
    }
}
