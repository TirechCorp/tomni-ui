//! Divisor do `Split`, no visual do `SizerBase` do Windows Community Toolkit.

use tomni_ui_core::{ButtonState, Direction, SPLIT_SIZER, SizerVisual};
use tomni_ui_render::kurbo::{Rect, RoundedRect, Size};
use tomni_ui_render::peniko::Color;
use tomni_ui_render::{Canvas, CanvasExt};

use crate::{FluentDrawer, GraphicInfo, Tokens, metrics};

/// Polegar: 4 × 24 px (`SizerBaseThumbWidth` e `Height`), em pé no divisor de um
/// `Split::row`.
const THUMB: Size = Size::new(4.0, 24.0);
/// Raio do polegar (`SizerBaseThumbRadius`).
const THUMB_RADIUS: f64 = 2.0;

/// Fundo do divisor: transparente, `ControlAltFillColorTertiary` no hover e `Quarternary`
/// arrastando (o `PointerOver` e o `Pressed` do estilo do `SizerBase`).
fn fill(t: &Tokens, visual: SizerVisual) -> Color {
    match visual.state {
        ButtonState::Hover => t.control_alt_fill_tertiary,
        ButtonState::Pressed => t.control_alt_fill_quarternary,
        ButtonState::Rest | ButtonState::Disabled => Color::TRANSPARENT,
    }
}

/// O polegar, centrado no divisor.
fn thumb(rect: Rect, visual: SizerVisual) -> Rect {
    let size = match visual.direction {
        Direction::Row => THUMB,
        Direction::Column => Size::new(THUMB.height, THUMB.width),
    };
    Rect::from_center_size(rect.center(), size)
}

pub(crate) fn measure(visual: SizerVisual) -> Size {
    match visual.direction {
        Direction::Row => Size::new(SPLIT_SIZER, 0.0),
        Direction::Column => Size::new(0.0, SPLIT_SIZER),
    }
}

pub(crate) fn draw(
    drawer: &FluentDrawer,
    canvas: &mut dyn Canvas,
    rect: Rect,
    visual: SizerVisual,
) {
    let t = &drawer.tokens;
    let fill = fill(t, visual);
    if fill != Color::TRANSPARENT {
        canvas.fill(&rect, fill);
    }
    canvas.fill(
        &RoundedRect::from_rect(thumb(rect, visual), THUMB_RADIUS),
        t.control_strong_fill_default,
    );
    if visual.focus_ring {
        crate::draw_focus_ring(
            canvas,
            rect.inset(-metrics::FOCUS_MARGIN),
            metrics::CONTROL_CORNER_RADIUS,
            t,
        );
    }
}

pub(crate) fn graphic(drawer: &FluentDrawer, rect: Rect, visual: SizerVisual) -> Vec<GraphicInfo> {
    let t = &drawer.tokens;
    let background = crate::over(fill(t, visual), drawer.backdrop());
    vec![GraphicInfo {
        name: "SizerThumb",
        color: crate::over(t.control_strong_fill_default, background),
        background,
        bounds: thumb(rect, visual),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polegar_fica_no_meio_e_deita_na_coluna() {
        let rect = Rect::new(100.0, 0.0, 108.0, 300.0);
        let row = SizerVisual {
            direction: Direction::Row,
            ..SizerVisual::default()
        };
        assert_eq!(thumb(rect, row), Rect::new(102.0, 138.0, 106.0, 162.0));
        let column = SizerVisual {
            direction: Direction::Column,
            ..SizerVisual::default()
        };
        let rect = Rect::new(0.0, 100.0, 300.0, 108.0);
        assert_eq!(thumb(rect, column), Rect::new(138.0, 102.0, 162.0, 106.0));
        assert_eq!(measure(column), Size::new(0.0, 8.0));
    }
}
