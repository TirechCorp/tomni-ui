//! `Expander` no visual do WinUI: o cabeçalho em cartão, a seta à direita e o painel do
//! conteúdo embaixo.

use tomni_ui_core::{ButtonState, ExpanderVisual};
use tomni_ui_render::kurbo::{Affine, BezPath, Point, Rect, RoundedRect, RoundedRectRadii, Shape};
use tomni_ui_render::peniko::Color;
use tomni_ui_render::{Canvas, CanvasExt, PATH_TOLERANCE, Paint};

use crate::{Tokens, icons, metrics};

/// Lado do botão da seta (`ExpanderChevronButtonSize`).
const CHEVRON_BUTTON: f64 = 32.0;
/// Espaço à direita do botão da seta (`ExpanderChevronMargin`).
const CHEVRON_MARGIN: f64 = 8.0;
/// Lado do ícone da seta (`ExpanderChevronGlyphSize`).
const CHEVRON_GLYPH: f64 = 12.0;

/// Fundo e cor da seta.
pub(crate) fn chevron_colors(t: &Tokens, state: ButtonState) -> (Color, Color) {
    match state {
        ButtonState::Rest => (Color::TRANSPARENT, t.text_fill_primary),
        ButtonState::Hover => (t.subtle_fill_secondary, t.text_fill_primary),
        ButtonState::Pressed => (t.subtle_fill_tertiary, t.text_fill_secondary),
        ButtonState::Disabled => (Color::TRANSPARENT, t.text_fill_disabled),
    }
}

/// Faixa entre dois retângulos com cantos diferentes, o de dentro furando o de fora.
fn ring(
    outer: Rect,
    outer_radii: RoundedRectRadii,
    inner: Rect,
    inner_radii: RoundedRectRadii,
) -> BezPath {
    let mut path = RoundedRect::from_rect(outer, outer_radii).to_path(PATH_TOLERANCE);
    path.extend(
        RoundedRect::from_rect(inner, inner_radii)
            .to_path(PATH_TOLERANCE)
            .reverse_subpaths(),
    );
    path
}

/// Pinta um cartão com fundo e borda que não se sobrepõem: as duas cores do WinUI são
/// translúcidas. `borders` são as espessuras à esquerda, em cima, à direita e embaixo.
pub(crate) fn card(
    canvas: &mut dyn Canvas,
    rect: Rect,
    radii: RoundedRectRadii,
    borders: (f64, f64, f64, f64),
    fill: Color,
    stroke: Color,
) {
    let (left, top, right, bottom) = borders;
    let inner = Rect::new(
        rect.x0 + left,
        rect.y0 + top,
        rect.x1 - right,
        rect.y1 - bottom,
    );
    let shrink = |r: f64| (r - metrics::BORDER).max(0.0);
    let inner_radii = RoundedRectRadii::new(
        shrink(radii.top_left),
        shrink(radii.top_right),
        shrink(radii.bottom_right),
        shrink(radii.bottom_left),
    );
    canvas.fill(&RoundedRect::from_rect(inner, inner_radii), fill);
    canvas.fill_path(
        &ring(rect, radii, inner, inner_radii),
        &Paint::Solid(stroke),
    );
}

pub(crate) fn draw(canvas: &mut dyn Canvas, rect: Rect, visual: ExpanderVisual, t: &Tokens) {
    let radius = metrics::CONTROL_CORNER_RADIUS;
    let border = metrics::BORDER;
    let height = visual.header_height();
    let header = Rect::new(rect.x0, rect.y0, rect.x1, rect.y1.min(rect.y0 + height));
    let open = visual.open && rect.height() > height;
    let header_radii = if open {
        RoundedRectRadii::new(radius, radius, 0.0, 0.0)
    } else {
        RoundedRectRadii::from_single_radius(radius)
    };
    card(
        canvas,
        header,
        header_radii,
        (border, border, border, border),
        t.card_background_default,
        t.card_stroke_default,
    );
    if open {
        // O painel encosta embaixo do cabeçalho, sem borda em cima: a de baixo do cabeçalho
        // faz a divisa.
        let content = Rect::new(rect.x0, header.y1, rect.x1, rect.y1);
        card(
            canvas,
            content,
            RoundedRectRadii::new(0.0, 0.0, radius, radius),
            (border, 0.0, border, border),
            t.card_background_secondary,
            t.card_stroke_default,
        );
    }

    let (background, glyph) = chevron_colors(t, visual.state);
    let x1 = header.x1 - CHEVRON_MARGIN;
    let y0 = header.y0 + ((header.height() - CHEVRON_BUTTON) / 2.0).round();
    let button = Rect::new(x1 - CHEVRON_BUTTON, y0, x1, y0 + CHEVRON_BUTTON);
    if background != Color::TRANSPARENT {
        canvas.fill(&RoundedRect::from_rect(button, radius), background);
    }
    let offset = (CHEVRON_BUTTON - CHEVRON_GLYPH) / 2.0;
    let mut path = icons::CHEVRON_DOWN_12.path(
        Point::new(button.x0 + offset, button.y0 + offset),
        CHEVRON_GLYPH,
    );
    if visual.open {
        // Para cima: a de baixo espelhada em torno do centro do botão.
        let cy = button.center().y;
        path.apply_affine(Affine::translate((0.0, 2.0 * cy)) * Affine::FLIP_Y);
    }
    canvas.fill_path(&path, &Paint::Solid(glyph));

    if visual.focus_ring {
        crate::draw_focus_ring(canvas, header, radius, t);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;

    #[test]
    fn seta_ganha_fundo_no_hover() {
        let t = Tokens::new(Theme::Light);
        assert_eq!(chevron_colors(&t, ButtonState::Rest).0, Color::TRANSPARENT);
        assert_eq!(
            chevron_colors(&t, ButtonState::Hover).0,
            t.subtle_fill_secondary
        );
    }
}
