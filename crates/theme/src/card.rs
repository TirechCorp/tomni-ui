//! `Border` e `Card`: fundo, borda e, no card clicável, os estados e o anel de foco.

use tomni_ui_core::{
    BorderVisual, Brush, ButtonState, CARD_BORDER, CARD_CORNER_RADIUS, CardVisual,
    SETTINGS_CARD_BORDER, SETTINGS_CARD_CORNER_RADIUS, SettingsCardPart, SettingsCardVisual,
};
use tomni_ui_render::kurbo::{Point, Rect, RoundedRect, RoundedRectRadii};
use tomni_ui_render::peniko::{Color, Gradient};
use tomni_ui_render::{Canvas, CanvasExt, Paint};

use crate::{Border, Theme, Tokens, draw_focus_ring, metrics, ring};

/// O que pinta `brush` em `rect`: a cor ou o gradiente, com as pontas (relativas ao
/// retângulo) levadas para a janela. `None` num gradiente sem cores.
fn paint(brush: &Brush, rect: Rect, t: &Tokens) -> Option<Paint> {
    match brush {
        Brush::Solid(color) => Some(Paint::Solid(t.resolve(*color))),
        Brush::Linear(gradient) => {
            let at =
                |p: Point| Point::new(rect.x0 + p.x * rect.width(), rect.y0 + p.y * rect.height());
            let stops: Vec<(f32, Color)> = gradient
                .stops
                .iter()
                .map(|s| (s.offset as f32, t.resolve(s.color)))
                .collect();
            match stops.as_slice() {
                [] => None,
                [(_, color)] => Some(Paint::Solid(*color)),
                stops => Some(Paint::Gradient(
                    Gradient::new_linear(at(gradient.start), at(gradient.end)).with_stops(stops),
                )),
            }
        }
    }
}

/// Pinta um `Border`: o fundo dentro da borda (o `BackgroundSizing` padrão do WinUI,
/// `InnerBorderEdge`) e a borda por cima.
pub(crate) fn draw_border(canvas: &mut dyn Canvas, rect: Rect, visual: &BorderVisual, t: &Tokens) {
    let radius = visual.corner_radius.max(0.0);
    let thickness = visual
        .border
        .as_ref()
        .map_or(0.0, |(_, thickness)| thickness.max(0.0))
        .min(rect.width() / 2.0)
        .min(rect.height() / 2.0);
    let inner = rect.inset(-thickness);
    let inner_radius = (radius - thickness).max(0.0);
    if let Some(paint) = visual.background.as_ref().and_then(|b| paint(b, inner, t)) {
        canvas.fill(&RoundedRect::from_rect(inner, inner_radius), paint);
    }
    if let Some((brush, _)) = &visual.border
        && thickness > 0.0
        && let Some(paint) = paint(brush, rect, t)
    {
        canvas.fill_path(&ring(rect, radius, inner, inner_radius), &paint);
    }
}

/// Fundo e borda do card em cada estado. As cores são as do `SettingsCard` do Windows
/// Community Toolkit; o hover usa a borda de elevação do botão (`ControlElevationBorderBrush`).
fn card_colors(visual: CardVisual, t: &Tokens) -> (Color, Border) {
    match visual.state {
        ButtonState::Rest => (
            t.card_background_default,
            Border::Solid(t.card_stroke_default),
        ),
        ButtonState::Hover => (
            t.control_fill_secondary,
            Border::Elevation {
                secondary: t.control_stroke_secondary,
                default: t.control_stroke_default,
                inverted: t.theme == Theme::Light,
            },
        ),
        ButtonState::Pressed => (
            t.control_fill_tertiary,
            Border::Solid(t.control_stroke_default),
        ),
        ButtonState::Disabled => (
            t.control_fill_disabled,
            Border::Solid(t.control_stroke_default),
        ),
    }
}

/// Cor de fundo do card, para o lint compor as cores dos textos de dentro.
pub(crate) fn card_background(visual: CardVisual, t: &Tokens) -> Color {
    card_colors(visual, t).0
}

/// Pinta um `Card`: fundo dentro da borda (as duas cores são translúcidas), a borda e o anel
/// de foco.
pub(crate) fn draw_card(canvas: &mut dyn Canvas, rect: Rect, visual: CardVisual, t: &Tokens) {
    let (fill, border) = card_colors(visual, t);
    let radius = CARD_CORNER_RADIUS;
    let inner = rect.inset(-CARD_BORDER);
    canvas.fill(&RoundedRect::from_rect(inner, radius - CARD_BORDER), fill);
    let ring = ring(rect, radius, inner, radius - CARD_BORDER);
    match border {
        Border::None => {}
        Border::Solid(color) => canvas.fill_path(&ring, &Paint::Solid(color)),
        Border::Elevation {
            secondary,
            default,
            inverted,
        } => {
            let h = metrics::ELEVATION_BORDER_HEIGHT;
            let (start, end) = if inverted {
                (Point::new(0.0, rect.y1), Point::new(0.0, rect.y1 - h))
            } else {
                (Point::new(0.0, rect.y0), Point::new(0.0, rect.y0 + h))
            };
            let gradient = Gradient::new_linear(start, end)
                .with_stops([(metrics::ELEVATION_BORDER_STOP, secondary), (1.0, default)]);
            canvas.fill_path(&ring, &Paint::Gradient(gradient));
        }
    }
    if visual.focus_ring {
        draw_focus_ring(canvas, rect, radius, t);
    }
}

/// Cantos e espessuras das bordas (esquerda, em cima, direita, embaixo) do `SettingsCard`
/// no lugar dele.
fn settings_card_shape(part: SettingsCardPart) -> (RoundedRectRadii, (f64, f64, f64, f64)) {
    let r = SETTINGS_CARD_CORNER_RADIUS;
    let b = SETTINGS_CARD_BORDER;
    match part {
        SettingsCardPart::Alone => (RoundedRectRadii::from_single_radius(r), (b, b, b, b)),
        SettingsCardPart::ExpanderHeader { open: false } => {
            (RoundedRectRadii::from_single_radius(r), (b, b, b, b))
        }
        // Aberto, o separador do primeiro filho faz a divisa: sem borda embaixo.
        SettingsCardPart::ExpanderHeader { open: true } => {
            (RoundedRectRadii::new(r, r, 0.0, 0.0), (b, b, b, 0.0))
        }
        SettingsCardPart::ExpanderItem { last } => (
            RoundedRectRadii::new(
                0.0,
                0.0,
                if last { r } else { 0.0 },
                if last { r } else { 0.0 },
            ),
            (b, b, b, if last { b } else { 0.0 }),
        ),
    }
}

/// Fundo do `SettingsCard`: o dos filhos de um expander é o do corpo dele, que não muda com o
/// ponteiro.
fn settings_card_colors(visual: SettingsCardVisual, t: &Tokens) -> (Color, Border) {
    let (fill, border) = card_colors(
        CardVisual {
            clickable: visual.clickable,
            state: visual.state,
            focus_ring: visual.focus_ring,
        },
        t,
    );
    match visual.part {
        SettingsCardPart::ExpanderItem { .. } if visual.state == ButtonState::Rest => (
            t.card_background_secondary,
            Border::Solid(t.card_stroke_default),
        ),
        SettingsCardPart::ExpanderItem { .. } => (fill, Border::Solid(t.card_stroke_default)),
        _ => (fill, border),
    }
}

/// Cor de fundo do `SettingsCard`, para o lint compor as cores dos textos de dentro.
pub(crate) fn settings_card_background(visual: SettingsCardVisual, t: &Tokens) -> Color {
    settings_card_colors(visual, t).0
}

/// Pinta um `SettingsCard`: fundo, borda (no filho de expander, só os lados, o separador de
/// cima e, no último, a de baixo) e o anel de foco.
pub(crate) fn draw_settings_card(
    canvas: &mut dyn Canvas,
    rect: Rect,
    visual: SettingsCardVisual,
    t: &Tokens,
) {
    let (fill, border) = settings_card_colors(visual, t);
    let (radii, thickness) = settings_card_shape(visual.part);
    let stroke = match border {
        Border::None => None,
        Border::Solid(color) => Some(color),
        // O degradê de elevação só vale num card inteiro; nos de dentro de um expander a
        // borda é lisa.
        Border::Elevation { default, .. } => Some(default),
    };
    match stroke {
        Some(stroke) => crate::expander::card(canvas, rect, radii, thickness, fill, stroke),
        None => canvas.fill(&RoundedRect::from_rect(rect, radii), fill),
    }
    if visual.focus_ring {
        draw_focus_ring(canvas, rect, SETTINGS_CARD_CORNER_RADIUS, t);
    }
}

/// Cor de `brush` no ponto `at` da janela, dentro de `rect`, para o lint.
pub(crate) fn brush_color(brush: &Brush, rect: Rect, at: Point, t: &Tokens) -> Option<Color> {
    let relative = Point::new(
        if rect.width() > 0.0 {
            (at.x - rect.x0) / rect.width()
        } else {
            0.0
        },
        if rect.height() > 0.0 {
            (at.y - rect.y0) / rect.height()
        } else {
            0.0
        },
    );
    brush.nearest_color(relative).map(|c| t.resolve(c))
}
