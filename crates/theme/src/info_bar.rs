//! Barra de aviso no visual da `InfoBar` do WinUI.

use std::cell::RefCell;

use tomni_ui_core::{ButtonState, ButtonVisual, InfoBar, InfoBarSeverity, InfoBarVisual};
use tomni_ui_render::kurbo::{Circle, Point, Rect, RoundedRect, Size};
use tomni_ui_render::peniko::Color;
use tomni_ui_render::{Canvas, CanvasExt, Paint};
use tomni_ui_text::{Fonts, TextLayout, TextStyle};

use crate::{Drawer, FluentDrawer, TextInfo, Tokens, icons, metrics, ring};

/// `InfoBarMinHeight`.
const MIN_HEIGHT: f64 = 48.0;
/// Onde começa o ícone: a borda e o `InfoBarContentRootPadding` (16).
const ICON_X: f64 = 17.0;
/// Topo do ícone: a borda e a margem de cima do `InfoBarIconMargin` (16).
const ICON_Y: f64 = 17.0;
/// `InfoBarIconFontSize`.
const ICON: f64 = 16.0;
/// Onde começa o texto: o ícone e a margem direita dele (14).
const TEXT_X: f64 = ICON_X + ICON + 14.0;
/// Topo do título: a borda e a margem de cima (14).
const TEXT_Y: f64 = 15.0;
/// Entre o título e a mensagem na mesma linha.
const INLINE_GAP: f64 = 12.0;
/// Entre o título e a mensagem quando a mensagem desce.
const STACKED_GAP: f64 = 4.0;
/// Embaixo do texto quando ele quebra (`InfoBarPanelVerticalOrientationPadding`), mais a borda.
const STACKED_BOTTOM: f64 = 19.0;
/// Do texto até a borda direita, sem o botão de fechar: o `InfoBarPanelMargin` e a borda.
const RIGHT: f64 = 17.0;
/// Do texto até a borda direita, com o botão de fechar de 38 px.
const RIGHT_CLOSABLE: f64 = 48.0;
/// `InfoBarCloseButtonGlyphSize`.
const CLOSE_GLYPH: f64 = 16.0;
/// Entre o fim do texto e o botão de ação, na mesma linha.
const ACTION_GAP: f64 = 12.0;
/// Entre a mensagem e o botão de ação quando ele desce.
const ACTION_STACKED_GAP: f64 = 12.0;
/// Embaixo do botão de ação quando o conteúdo quebra.
const ACTION_BOTTOM: f64 = 16.0;

/// Título e mensagem já posicionados, relativos ao canto do widget.
struct Placed {
    title: Option<(TextLayout, Point)>,
    message: Option<(TextLayout, Point)>,
    action: Option<Rect>,
    size: Size,
}

/// O tamanho do botão de ação: o de um botão padrão com o rótulo `label`.
fn action_size(fonts: &RefCell<Fonts>, label: &str) -> Size {
    let (left, _, right, _) = metrics::BUTTON_PADDING;
    let text = fonts.borrow_mut().layout(label, TextStyle::BODY, None);
    Size::new(
        (f64::from(text.width()) + left + right + 2.0 * metrics::BORDER).ceil(),
        metrics::CONTROL_HEIGHT,
    )
}

/// Onde fica o botão de ação de uma barra com `width` de largura, relativo ao canto dela.
pub(crate) fn action_rect(
    fonts: &RefCell<Fonts>,
    visual: &InfoBarVisual,
    width: f64,
) -> Option<Rect> {
    place(fonts, visual, Some(width)).action
}

fn right(visual: &InfoBarVisual) -> f64 {
    if visual.closable {
        RIGHT_CLOSABLE
    } else {
        RIGHT
    }
}

/// Título e mensagem numa linha, se couberem em `width`; senão um abaixo do outro, quebrando
/// as linhas. Sem `width`, sempre numa linha.
fn place(fonts: &RefCell<Fonts>, visual: &InfoBarVisual, width: Option<f64>) -> Placed {
    let action_size = visual
        .action
        .as_deref()
        .map(|label| action_size(fonts, label));
    let mut fonts = fonts.borrow_mut();
    let right = right(visual);
    let has_title = !visual.title.is_empty();
    let has_message = !visual.message.is_empty();
    let title = has_title.then(|| fonts.layout(&visual.title, TextStyle::BODY_STRONG, None));
    let message = has_message.then(|| fonts.layout(&visual.message, TextStyle::BODY, None));
    let title_width = title.as_ref().map_or(0.0, |t| f64::from(t.width()));
    let message_width = message.as_ref().map_or(0.0, |m| f64::from(m.width()));
    let gap = if has_title && has_message {
        INLINE_GAP
    } else {
        0.0
    };
    let text_end = TEXT_X + title_width + gap + message_width;
    let action_x = if has_title || has_message {
        text_end + ACTION_GAP
    } else {
        TEXT_X
    };
    let inline_width = (action_size.map_or(text_end, |a| action_x + a.width) + right).ceil();
    if width.is_none_or(|w| inline_width <= w + 0.5) {
        let message_x = TEXT_X + title_width + gap;
        return Placed {
            title: title.map(|t| (t, Point::new(TEXT_X, TEXT_Y))),
            message: message.map(|m| (m, Point::new(message_x, TEXT_Y))),
            action: action_size
                .map(|a| Rect::from_origin_size((action_x, (MIN_HEIGHT - a.height) / 2.0), a)),
            size: Size::new(inline_width, MIN_HEIGHT),
        };
    }

    let available = (width.unwrap_or(0.0) - TEXT_X - right).max(0.0) as f32 + 0.5;
    let title =
        has_title.then(|| fonts.layout(&visual.title, TextStyle::BODY_STRONG, Some(available)));
    let message =
        has_message.then(|| fonts.layout(&visual.message, TextStyle::BODY, Some(available)));
    let mut y = TEXT_Y;
    let mut text_width: f64 = 0.0;
    let title = title.map(|t| {
        let origin = Point::new(TEXT_X, y);
        y += f64::from(t.height()) + STACKED_GAP;
        text_width = text_width.max(f64::from(t.width()));
        (t, origin)
    });
    let message = message.map(|m| {
        let origin = Point::new(TEXT_X, y);
        y += f64::from(m.height());
        text_width = text_width.max(f64::from(m.width()));
        (m, origin)
    });
    if message.is_none() {
        y -= STACKED_GAP;
    }
    let mut bottom = STACKED_BOTTOM;
    let action = action_size.map(|a| {
        if title.is_some() || message.is_some() {
            y += ACTION_STACKED_GAP;
        }
        let rect = Rect::from_origin_size((TEXT_X, y), a);
        y += a.height;
        bottom = ACTION_BOTTOM;
        text_width = text_width.max(a.width);
        rect
    });
    Placed {
        title,
        message,
        action,
        size: Size::new(
            (TEXT_X + text_width + right).ceil(),
            (y + bottom).ceil().max(MIN_HEIGHT),
        ),
    }
}

fn background(t: &Tokens, severity: InfoBarSeverity) -> Color {
    match severity {
        InfoBarSeverity::Informational => t.info_bar_informational_background,
        InfoBarSeverity::Success => t.info_bar_success_background,
        InfoBarSeverity::Warning => t.info_bar_warning_background,
        InfoBarSeverity::Error => t.info_bar_error_background,
    }
}

fn icon(t: &Tokens, severity: InfoBarSeverity) -> (icons::Icon, Color) {
    match severity {
        InfoBarSeverity::Informational => (icons::INFO_16, t.accent_fill_default),
        InfoBarSeverity::Success => (icons::CHECKMARK_CIRCLE_16, t.info_bar_success_icon),
        InfoBarSeverity::Warning => (icons::ERROR_CIRCLE_16, t.info_bar_warning_icon),
        InfoBarSeverity::Error => (icons::DISMISS_CIRCLE_16, t.info_bar_error_icon),
    }
}

pub(crate) fn measure(drawer: &FluentDrawer, visual: &InfoBarVisual, width: Option<f64>) -> Size {
    place(&drawer.fonts, visual, width).size
}

/// O botão padrão que a barra desenha para o botão de ação.
fn action_button(visual: &InfoBarVisual) -> ButtonVisual {
    ButtonVisual {
        state: visual.action_state,
        focus_ring: visual.action_focus_ring,
        ..ButtonVisual::default()
    }
}

pub(crate) fn draw(
    drawer: &mut FluentDrawer,
    canvas: &mut dyn Canvas,
    rect: Rect,
    visual: &InfoBarVisual,
) {
    let t = &drawer.tokens;
    let radius = metrics::CONTROL_CORNER_RADIUS;
    let border = metrics::BORDER;
    let inner = rect.inset(-border);
    canvas.fill(
        &RoundedRect::from_rect(inner, radius - border),
        background(t, visual.severity),
    );
    canvas.fill_path(
        &ring(rect, radius, inner, radius - border),
        &Paint::Solid(t.card_stroke_default),
    );

    // O ícone é um círculo cheio com o símbolo vazado; o miolo vazado mostra a cor inversa.
    let (icon, color) = icon(t, visual.severity);
    let origin = Point::new(rect.x0 + ICON_X, rect.y0 + ICON_Y);
    let center = origin + (ICON / 2.0, ICON / 2.0);
    canvas.fill(&Circle::new(center, 6.5), t.text_fill_inverse);
    canvas.fill_path(&icon.path(origin, ICON), &Paint::Solid(color));

    let placed = place(&drawer.fonts, visual, Some(rect.width()));
    let text = Paint::Solid(t.text_fill_primary);
    for (layout, offset) in [placed.title, placed.message].into_iter().flatten() {
        layout.draw(canvas, rect.origin() + offset.to_vec2(), &text);
    }
    if let (Some(action), Some(label)) = (placed.action, visual.action.as_deref()) {
        let button = action + rect.origin().to_vec2();
        drawer.draw_button(canvas, button, label, action_button(visual));
    }
    let t = &drawer.tokens;

    if visual.closable {
        let button = InfoBar::close_area(rect).inset(-3.0);
        let (fill, glyph) = match visual.close_state {
            ButtonState::Rest => (Color::TRANSPARENT, t.text_fill_primary),
            ButtonState::Hover => (t.subtle_fill_secondary, t.text_fill_primary),
            ButtonState::Pressed => (t.subtle_fill_tertiary, t.text_fill_secondary),
            ButtonState::Disabled => (Color::TRANSPARENT, t.text_fill_disabled),
        };
        canvas.fill(&RoundedRect::from_rect(button, radius), fill);
        let offset = (button.width() - CLOSE_GLYPH) / 2.0;
        let origin = Point::new(button.x0 + offset, button.y0 + offset);
        canvas.fill_path(
            &icons::DISMISS_16.path(origin, CLOSE_GLYPH),
            &Paint::Solid(glyph),
        );
        if visual.focus_ring {
            crate::draw_focus_ring(canvas, button, radius, t);
        }
    }
}

pub(crate) fn text_info(
    drawer: &mut FluentDrawer,
    rect: Rect,
    visual: &InfoBarVisual,
) -> Vec<TextInfo> {
    let t = &drawer.tokens;
    let background = crate::over(background(t, visual.severity), drawer.backdrop());
    let color = crate::over(t.text_fill_primary, background);
    let clip = Rect::new(
        rect.x0 + TEXT_X,
        rect.y0 + metrics::BORDER,
        rect.x1 - right(visual),
        rect.y1 - metrics::BORDER,
    );
    let placed = place(&drawer.fonts, visual, Some(rect.width()));
    let info = |text: &str, style: TextStyle, (layout, offset): (TextLayout, Point)| {
        let origin = rect.origin() + offset.to_vec2();
        TextInfo {
            text: text.to_owned(),
            color,
            background,
            style,
            bounds: Rect::from_origin_size(origin, crate::text::block_size(&layout, style)),
            clip: Some(clip),
        }
    };
    let mut infos = Vec::new();
    if let Some(title) = placed.title {
        infos.push(info(&visual.title, TextStyle::BODY_STRONG, title));
    }
    if let Some(message) = placed.message {
        infos.push(info(&visual.message, TextStyle::BODY, message));
    }
    if let (Some(action), Some(label)) = (placed.action, visual.action.as_deref()) {
        let button = action + rect.origin().to_vec2();
        infos.extend(drawer.button_info(button, label, action_button(visual)));
    }
    infos
}
