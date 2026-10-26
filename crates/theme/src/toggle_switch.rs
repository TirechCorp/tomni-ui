//! Chave liga/desliga no visual do `ToggleSwitch` do WinUI.

use tomni_ui_core::{ButtonState, ToggleSwitchVisual};
use tomni_ui_render::kurbo::{Point, Rect, RoundedRect, Size};
use tomni_ui_render::peniko::Color;
use tomni_ui_render::{Canvas, CanvasExt, Paint};
use tomni_ui_text::TextStyle;

use crate::{FluentDrawer, TextInfo, Tokens, metrics, ring};

/// Trilho de 40 × 20 px.
const TRACK: Size = Size::new(40.0, 20.0);
/// Linha da chave: o trilho com 10 px em cima e embaixo (`ToggleSwitchPreContentMargin` e
/// `ToggleSwitchPostContentMargin`).
const ROW: f64 = 40.0;
/// Onde começa "Ligado"/"Desligado": o trilho e a coluna de 12 px.
const CONTENT_X: f64 = 52.0;
/// Espaço entre o rótulo de cima e a chave (`ToggleSwitchTopHeaderMargin`).
const HEADER_GAP: f64 = 4.0;
/// `ToggleSwitchThemeMinWidth`.
const MIN_WIDTH: f64 = 154.0;

struct Colors {
    fill: Color,
    stroke: Option<Color>,
    knob: Color,
    text: Color,
}

fn colors(t: &Tokens, visual: ToggleSwitchVisual) -> Colors {
    use ButtonState::*;
    let text = if visual.state == Disabled {
        t.text_fill_disabled
    } else {
        t.text_fill_primary
    };
    if visual.on {
        // Ligada, a chave não tem borda (`ToggleSwitchOnStrokeThickness` 0).
        let (fill, knob) = match visual.state {
            Rest => (t.accent_fill_default, t.text_on_accent_primary),
            Hover => (t.accent_fill_secondary, t.text_on_accent_primary),
            Pressed => (t.accent_fill_tertiary, t.text_on_accent_primary),
            Disabled => (t.accent_fill_disabled, t.text_on_accent_disabled),
        };
        Colors {
            fill,
            stroke: None,
            knob,
            text,
        }
    } else {
        let (fill, stroke, knob) = match visual.state {
            Rest => (
                t.control_alt_fill_secondary,
                t.control_strong_stroke_default,
                t.text_fill_secondary,
            ),
            Hover => (
                t.control_alt_fill_tertiary,
                t.control_strong_stroke_default,
                t.text_fill_secondary,
            ),
            Pressed => (
                t.control_alt_fill_quarternary,
                t.control_strong_stroke_default,
                t.text_fill_secondary,
            ),
            Disabled => (
                t.control_alt_fill_disabled,
                t.control_strong_stroke_disabled,
                t.text_fill_disabled,
            ),
        };
        Colors {
            fill,
            stroke: Some(stroke),
            knob,
            text,
        }
    }
}

/// Partes da chave num widget em `rect`.
struct Parts {
    /// Rótulo de cima, se houver.
    header: Option<Point>,
    /// Linha da chave, com 40 px de altura e a largura do widget.
    row: Rect,
}

fn parts(rect: Rect, header: &str) -> Parts {
    let header_height = if header.is_empty() {
        0.0
    } else {
        f64::from(TextStyle::BODY.line_height) + HEADER_GAP
    };
    // Num widget mais alto que o natural, a linha da chave desce para o centro do que sobra.
    let extra = ((rect.height() - header_height - ROW) / 2.0)
        .floor()
        .max(0.0);
    let top = rect.y0 + header_height + extra;
    Parts {
        header: (!header.is_empty()).then_some(rect.origin()),
        row: Rect::new(rect.x0, top, rect.x1, top + ROW),
    }
}

fn track(row: Rect) -> Rect {
    let y = row.y0 + (ROW - TRACK.height) / 2.0;
    Rect::new(row.x0, y, row.x0 + TRACK.width, y + TRACK.height)
}

/// Polegar: 12 px parado, 14 com o ponteiro em cima e 17 × 14 pressionado, esticando para o
/// centro do trilho.
fn knob(track: Rect, visual: ToggleSwitchVisual) -> Rect {
    let center_y = track.center().y;
    let (x0, x1) = match (visual.state, visual.on) {
        (ButtonState::Pressed, false) => (3.0, 20.0),
        (ButtonState::Pressed, true) => (20.0, 37.0),
        (state, on) => {
            let center = if on { 30.0 } else { 10.0 };
            let half = if state == ButtonState::Hover {
                7.0
            } else {
                6.0
            };
            (center - half, center + half)
        }
    };
    let half_height = if matches!(visual.state, ButtonState::Hover | ButtonState::Pressed) {
        7.0
    } else {
        6.0
    };
    Rect::new(
        track.x0 + x0,
        center_y - half_height,
        track.x0 + x1,
        center_y + half_height,
    )
}

fn content_origin(row: Rect) -> Point {
    Point::new(
        row.x0 + CONTENT_X,
        row.y0 + (ROW - f64::from(TextStyle::BODY.line_height)) / 2.0,
    )
}

pub(crate) fn measure(drawer: &FluentDrawer, header: &str) -> Size {
    let mut fonts = drawer.fonts.borrow_mut();
    let mut width = |text: &str| f64::from(fonts.layout(text, TextStyle::BODY, None).width());
    let strings = tomni_ui_core::strings();
    let content = CONTENT_X + width(&strings.toggle_on).max(width(&strings.toggle_off));
    let (header_width, header_height) = if header.is_empty() {
        (0.0, 0.0)
    } else {
        (
            width(header),
            f64::from(TextStyle::BODY.line_height) + HEADER_GAP,
        )
    };
    Size::new(
        content.max(header_width).max(MIN_WIDTH).ceil(),
        header_height + ROW,
    )
}

pub(crate) fn draw(
    drawer: &FluentDrawer,
    canvas: &mut dyn Canvas,
    rect: Rect,
    header: &str,
    visual: ToggleSwitchVisual,
) {
    let colors = colors(&drawer.tokens, visual);
    let parts = parts(rect, header);
    let mut fonts = drawer.fonts.borrow_mut();
    let paint = Paint::Solid(colors.text);
    if let Some(origin) = parts.header {
        fonts
            .layout(header, TextStyle::BODY, None)
            .draw(canvas, origin, &paint);
    }

    let track = track(parts.row);
    let radius = TRACK.height / 2.0;
    match colors.stroke {
        Some(stroke) => {
            let border = metrics::BORDER;
            let inner = track.inset(-border);
            canvas.fill(&RoundedRect::from_rect(inner, radius - border), colors.fill);
            canvas.fill_path(
                &ring(track, radius, inner, radius - border),
                &Paint::Solid(stroke),
            );
        }
        None => canvas.fill(&RoundedRect::from_rect(track, radius), colors.fill),
    }
    let knob = knob(track, visual);
    canvas.fill(
        &RoundedRect::from_rect(knob, knob.height() / 2.0),
        colors.knob,
    );

    let strings = tomni_ui_core::strings();
    let content = if visual.on {
        &*strings.toggle_on
    } else {
        &*strings.toggle_off
    };
    fonts
        .layout(content, TextStyle::BODY, None)
        .draw(canvas, content_origin(parts.row), &paint);

    if visual.focus_ring {
        // O foco cerca a linha da chave sem os 5 px de cima e de baixo, com a margem de 7 px
        // dos lados e 3 em cima e embaixo (`FocusVisualMargin`).
        let target = parts.row.inset((0.0, -5.0));
        drop(fonts);
        crate::draw_focus_ring(
            canvas,
            target.inflate(4.0, 0.0),
            metrics::CONTROL_CORNER_RADIUS,
            &drawer.tokens,
        );
    }
}

pub(crate) fn text_info(
    drawer: &FluentDrawer,
    rect: Rect,
    header: &str,
    visual: ToggleSwitchVisual,
) -> Vec<TextInfo> {
    let t = &drawer.tokens;
    let colors = (colors(t, visual).text, drawer.backdrop());
    let parts = parts(rect, header);
    let mut infos = Vec::new();
    if let Some(origin) = parts.header {
        infos.push(crate::text::line_info(
            drawer,
            header,
            TextStyle::BODY,
            origin,
            colors,
            Rect::new(rect.x0, rect.y0, rect.x1, parts.row.y0),
        ));
    }
    let strings = tomni_ui_core::strings();
    let content = if visual.on {
        &*strings.toggle_on
    } else {
        &*strings.toggle_off
    };
    infos.push(crate::text::line_info(
        drawer,
        content,
        TextStyle::BODY,
        content_origin(parts.row),
        colors,
        Rect::new(
            parts.row.x0 + CONTENT_X,
            parts.row.y0,
            rect.x1,
            parts.row.y1,
        ),
    ));
    infos
}
