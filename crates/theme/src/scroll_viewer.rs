//! Barras de rolagem no visual da `ScrollBar` do WinUI.

use tomni_ui_core::{ScrollAxis, ScrollBarParts, ScrollViewerVisual};
use tomni_ui_render::kurbo::{BezPath, Point, Rect, RoundedRect};
use tomni_ui_render::{Canvas, CanvasExt};

use crate::Tokens;

/// Onde o polegar fino começa, medido da borda de dentro da faixa de 12 px.
const THIN_START: f64 = 7.0;
/// Onde o polegar expandido começa (`ScrollBarThumbStrokeThickness`: 6 px, até 9).
const WIDE_START: f64 = 3.0;
/// Onde o polegar termina, nos dois casos.
const THUMB_END: f64 = 9.0;
/// Folga do polegar nas pontas, para não encostar nas setas.
const THUMB_INSET: f64 = 3.0;
/// `ScrollBarCornerRadius`.
const THUMB_RADIUS: f64 = 3.0;
/// Raio do trilho, que aparece com a barra expandida.
const TRACK_RADIUS: f64 = 6.0;
/// Largura e altura do triângulo das setas (o `ChevronUpSmall` de 8 px da Segoe).
const ARROW: (f64, f64) = (6.0, 3.5);

/// Pinta as barras do `ScrollViewer` que ocupa `rect`, por cima do conteúdo.
pub(crate) fn draw(canvas: &mut dyn Canvas, rect: Rect, visual: &ScrollViewerVisual, t: &Tokens) {
    for axis in [ScrollAxis::Y, ScrollAxis::X] {
        if let Some(parts) = visual
            .scroller
            .bar(rect, axis)
            .filter(|_| !visual.bars_hidden)
        {
            draw_bar(canvas, &parts, visual.state.expanded(), t);
        }
    }
}

pub(crate) fn draw_bar(
    canvas: &mut dyn Canvas,
    parts: &ScrollBarParts,
    expanded: bool,
    t: &Tokens,
) {
    if expanded {
        canvas.fill(
            &RoundedRect::from_rect(parts.bar, TRACK_RADIUS),
            t.acrylic_in_app_fill_default,
        );
        // Na caixa de 12 × 12 da seta de cima, o triângulo fica centrado em (6, 8); na de
        // baixo, em (6, 4): os dois puxados para o meio da barra.
        canvas.fill(
            &arrow(parts.decrease, parts.axis, true),
            t.control_strong_fill_default,
        );
        canvas.fill(
            &arrow(parts.increase, parts.axis, false),
            t.control_strong_fill_default,
        );
    }
    let start = if expanded { WIDE_START } else { THIN_START };
    let thumb = parts.thumb;
    let thumb = match parts.axis {
        ScrollAxis::Y => Rect::new(
            thumb.x0 + start,
            thumb.y0 + THUMB_INSET,
            thumb.x0 + THUMB_END,
            thumb.y1 - THUMB_INSET,
        ),
        ScrollAxis::X => Rect::new(
            thumb.x0 + THUMB_INSET,
            thumb.y0 + start,
            thumb.x1 - THUMB_INSET,
            thumb.y0 + THUMB_END,
        ),
    };
    canvas.fill(
        &RoundedRect::from_rect(thumb, THUMB_RADIUS),
        t.control_strong_fill_default,
    );
}

/// Triângulo da seta na caixa `cell`, apontando para o começo (`decrease`) ou o fim.
fn arrow(cell: Rect, axis: ScrollAxis, decrease: bool) -> BezPath {
    let (w, h) = ARROW;
    let offset = if decrease { 2.0 } else { -2.0 };
    let center = cell.center();
    // Ponta e base no eixo da barra, com `along` crescendo para o fim.
    let (tip, base) = if decrease {
        (-h / 2.0, h / 2.0)
    } else {
        (h / 2.0, -h / 2.0)
    };
    let point = |along: f64, across: f64| match axis {
        ScrollAxis::Y => Point::new(center.x + across, center.y + offset + along),
        ScrollAxis::X => Point::new(center.x + offset + along, center.y + across),
    };
    let mut path = BezPath::new();
    path.move_to(point(tip, 0.0));
    path.line_to(point(base, w / 2.0));
    path.line_to(point(base, -w / 2.0));
    path.close_path();
    path
}
