//! Slider no visual do `Slider` do WinUI.

use tomni_ui_core::{ButtonState, SLIDER_THUMB_SIZE, SliderVisual};
use tomni_ui_render::kurbo::{Circle, Point, Rect, RoundedRect, Shape, Size};
use tomni_ui_render::peniko::{Color, Gradient};
use tomni_ui_render::{Canvas, CanvasExt, PATH_TOLERANCE, Paint};
use tomni_ui_text::TextStyle;

use crate::{FluentDrawer, TextInfo, Theme, Tokens, metrics};

/// `SliderHorizontalHeight`: a linha do trilho e do polegar.
const ROW: f64 = 32.0;
/// `SliderTrackThemeHeight`.
const TRACK: f64 = 4.0;
/// Espaço entre o rótulo de cima e o trilho (`SliderTopHeaderMargin`).
const HEADER_GAP: f64 = 4.0;
/// Largura natural: o WinUI não tem mínimo, mas um slider precisa de espaço para andar.
const NATURAL_WIDTH: f64 = 200.0;

struct Colors {
    track: Color,
    value: Color,
    /// Miolo do polegar: a cor e o diâmetro (`SliderInnerThumb`, 12, 14 no hover e 10
    /// pressionado).
    inner: (Color, f64),
    header: Color,
}

fn colors(t: &Tokens, state: ButtonState) -> Colors {
    use ButtonState::*;
    let (track, header) = if state == Disabled {
        (t.control_strong_fill_disabled, t.text_fill_disabled)
    } else {
        (t.control_strong_fill_default, t.text_fill_primary)
    };
    let (value, size) = match state {
        Rest => (t.accent_fill_default, 12.0),
        Hover => (t.accent_fill_secondary, 14.0),
        Pressed => (t.accent_fill_tertiary, 10.0),
        Disabled => (t.accent_fill_disabled, 12.0),
    };
    Colors {
        track,
        value,
        inner: (value, size),
        header,
    }
}

fn header_height(header: &str) -> f64 {
    if header.is_empty() {
        0.0
    } else {
        f64::from(TextStyle::BODY.line_height) + HEADER_GAP
    }
}

/// Linha do trilho; num widget mais alto que o natural, desce para o centro do que sobra.
fn row(rect: Rect, header: &str) -> Rect {
    let header_height = header_height(header);
    let extra = ((rect.height() - header_height - ROW) / 2.0)
        .floor()
        .max(0.0);
    let top = rect.y0 + header_height + extra;
    Rect::new(rect.x0, top, rect.x1, top + ROW)
}

pub(crate) fn measure(drawer: &FluentDrawer, header: &str) -> Size {
    let header_width = if header.is_empty() {
        0.0
    } else {
        f64::from(
            drawer
                .fonts
                .borrow_mut()
                .layout(header, TextStyle::BODY, None)
                .width(),
        )
    };
    Size::new(
        header_width.max(NATURAL_WIDTH).ceil(),
        header_height(header) + ROW,
    )
}

pub(crate) fn draw(
    drawer: &FluentDrawer,
    canvas: &mut dyn Canvas,
    rect: Rect,
    header: &str,
    visual: SliderVisual,
) {
    let t = &drawer.tokens;
    let colors = colors(t, visual.state);
    if !header.is_empty() {
        drawer
            .fonts
            .borrow_mut()
            .layout(header, TextStyle::BODY, None)
            .draw(canvas, rect.origin(), &Paint::Solid(colors.header));
    }

    let row = row(rect, header);
    let cy = row.center().y;
    let track = Rect::new(row.x0, cy - TRACK / 2.0, row.x1, cy + TRACK / 2.0);
    canvas.fill(&RoundedRect::from_rect(track, TRACK / 2.0), colors.track);

    // O centro do polegar anda de meia largura dele até a outra ponta, e o valor pinta o
    // trilho até ali.
    let half = SLIDER_THUMB_SIZE / 2.0;
    let travel = (row.width() - SLIDER_THUMB_SIZE).max(0.0);
    let center = Point::new(row.x0 + half + visual.fraction.clamp(0.0, 1.0) * travel, cy);
    let filled = Rect::new(track.x0, track.y0, center.x, track.y1);
    canvas.fill(&RoundedRect::from_rect(filled, TRACK / 2.0), colors.value);

    // Polegar: o círculo de fora com a borda de elevação e o miolo accent.
    let border = metrics::BORDER;
    let outer = Circle::new(center, half);
    let mut ring = outer.to_path(PATH_TOLERANCE);
    ring.extend(
        Circle::new(center, half - border)
            .to_path(PATH_TOLERANCE)
            .reverse_subpaths(),
    );
    canvas.fill(
        &Circle::new(center, half - border),
        t.control_solid_fill_default,
    );
    // `ControlElevationBorderBrush`: no claro a borda mais marcada fica embaixo; no escuro,
    // em cima.
    let h = metrics::ELEVATION_BORDER_HEIGHT;
    let (start, end) = if t.theme == Theme::Light {
        let bottom = center.y + half;
        (Point::new(0.0, bottom), Point::new(0.0, bottom - h))
    } else {
        let top = center.y - half;
        (Point::new(0.0, top), Point::new(0.0, top + h))
    };
    let gradient = Gradient::new_linear(start, end).with_stops([
        (metrics::ELEVATION_BORDER_STOP, t.control_stroke_secondary),
        (1.0, t.control_stroke_default),
    ]);
    canvas.fill_path(&ring, &Paint::Gradient(gradient));
    let (inner, size) = colors.inner;
    canvas.fill(&Circle::new(center, size / 2.0), inner);

    if visual.focus_ring {
        // Em volta da linha do trilho, como o `FocusVisualMargin` do WinUI.
        crate::draw_focus_ring(canvas, row, metrics::CONTROL_CORNER_RADIUS, t);
    }
}

pub(crate) fn text_info(
    drawer: &FluentDrawer,
    rect: Rect,
    header: &str,
    visual: SliderVisual,
) -> Vec<TextInfo> {
    if header.is_empty() {
        return Vec::new();
    }
    let colors = colors(&drawer.tokens, visual.state);
    vec![crate::text::line_info(
        drawer,
        header,
        TextStyle::BODY,
        rect.origin(),
        (colors.header, drawer.backdrop()),
        rect,
    )]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linha_desce_com_o_cabecalho() {
        let rect = Rect::new(0.0, 0.0, 200.0, 56.0);
        assert_eq!(row(rect, "").y0, 12.0);
        assert_eq!(row(rect, "Volume").y0, 24.0);
    }

    #[test]
    fn miolo_muda_com_o_estado() {
        let t = Tokens::new(Theme::Dark);
        assert_eq!(colors(&t, ButtonState::Rest).inner.1, 12.0);
        assert_eq!(colors(&t, ButtonState::Hover).inner.1, 14.0);
        assert_eq!(colors(&t, ButtonState::Pressed).inner.1, 10.0);
        assert_eq!(
            colors(&t, ButtonState::Disabled).track,
            t.control_strong_fill_disabled
        );
    }
}
