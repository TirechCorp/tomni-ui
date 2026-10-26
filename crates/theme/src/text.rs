//! Texto solto (`Text`), na rampa tipográfica do Fluent.

use tomni_ui_core::{TextRamp, TextVisual, Token};
use tomni_ui_render::kurbo::{Point, Rect, Size};
use tomni_ui_render::peniko::Color;
use tomni_ui_render::{Canvas, Paint};
use tomni_ui_text::{TextLayout, TextStyle};

use crate::{FluentDrawer, TextInfo};

/// Fonte de cada degrau da rampa.
pub fn ramp_style(ramp: TextRamp) -> TextStyle {
    match ramp {
        TextRamp::Caption => TextStyle::CAPTION,
        TextRamp::Body => TextStyle::BODY,
        TextRamp::BodyStrong => TextStyle::BODY_STRONG,
        TextRamp::BodyLarge => TextStyle::BODY_LARGE,
        TextRamp::Subtitle => TextStyle::SUBTITLE,
        TextRamp::Title => TextStyle::TITLE,
        TextRamp::TitleLarge => TextStyle::TITLE_LARGE,
        TextRamp::Display => TextStyle::DISPLAY,
    }
}

/// Tamanho do bloco: largura arredondada para cima e pelo menos uma linha de altura, mesmo
/// sem texto.
pub(crate) fn block_size(layout: &TextLayout, style: TextStyle) -> Size {
    Size::new(
        f64::from(layout.width()).ceil(),
        f64::from(layout.height().max(style.line_height)).ceil(),
    )
}

/// Layout do texto em `max_width`, já cortado com reticências quando passa de `max_lines`.
/// O `bool` diz se cortou.
fn layout(
    drawer: &FluentDrawer,
    visual: &TextVisual,
    max_width: Option<f64>,
) -> (TextLayout, bool) {
    let style = ramp_style(visual.ramp);
    let width = max_width.map(|w| w.max(0.0) as f32);
    let mut fonts = drawer.fonts.borrow_mut();
    match visual.max_lines {
        Some(lines) => fonts.layout_lines(&visual.text, style, width, lines as usize),
        None => (fonts.layout(&visual.text, style, width), false),
    }
}

/// Largura em que o texto de um `Text` em `rect` quebra: a do retângulo com `wrap`, nenhuma
/// sem. Meio pixel a mais absorve o arredondamento da medida.
fn wrap_width(rect: Rect, wrap: bool) -> Option<f64> {
    wrap.then(|| rect.width() + 0.5)
}

/// Cor do texto: a própria, ou o `TextFillColorPrimary`. A herdada de um `Card` já chega no
/// `visual`, posta pelo `Ui`.
fn color(drawer: &FluentDrawer, visual: &TextVisual) -> Color {
    drawer
        .tokens
        .resolve(visual.color.unwrap_or(Token::TextFillColorPrimary.into()))
}

pub(crate) fn measure(drawer: &FluentDrawer, visual: &TextVisual, max_width: Option<f64>) -> Size {
    let (layout, _) = layout(drawer, visual, max_width);
    block_size(&layout, ramp_style(visual.ramp))
}

pub(crate) fn draw(
    drawer: &FluentDrawer,
    canvas: &mut dyn Canvas,
    rect: Rect,
    visual: &TextVisual,
) {
    let (layout, _) = layout(drawer, visual, wrap_width(rect, visual.wrap));
    layout.draw(canvas, rect.origin(), &Paint::Solid(color(drawer, visual)));
}

/// Se o texto em `rect` aparece cortado com reticências.
pub(crate) fn trimmed(drawer: &FluentDrawer, rect: Rect, visual: &TextVisual) -> bool {
    visual.max_lines.is_some() && layout(drawer, visual, wrap_width(rect, visual.wrap)).1
}

pub(crate) fn text_info(drawer: &FluentDrawer, rect: Rect, visual: &TextVisual) -> Vec<TextInfo> {
    if visual.text.is_empty() {
        return Vec::new();
    }
    let style = ramp_style(visual.ramp);
    let (layout, _) = layout(drawer, visual, wrap_width(rect, visual.wrap));
    // A largura sem arredondar: com `wrap`, uma linha de 308,3 px cabe nos 308,5 em que o
    // texto quebra, mas arredondada para 309 pareceria cortada.
    let size = Size::new(f64::from(layout.width()), block_size(&layout, style).height);
    vec![TextInfo {
        text: visual.text.clone(),
        color: crate::over(color(drawer, visual), drawer.backdrop()),
        background: drawer.backdrop(),
        style,
        bounds: Rect::from_origin_size(rect.origin(), size),
        clip: Some(rect),
    }]
}

/// Texto de uma linha com `origin` no canto superior esquerdo, visto pelo lint.
pub(crate) fn line_info(
    drawer: &FluentDrawer,
    text: &str,
    style: TextStyle,
    origin: Point,
    colors: (Color, Color),
    clip: Rect,
) -> TextInfo {
    let (color, background) = colors;
    let width = f64::from(drawer.fonts.borrow_mut().layout(text, style, None).width());
    TextInfo {
        text: text.to_owned(),
        color: crate::over(color, background),
        background,
        style,
        bounds: Rect::new(
            origin.x,
            origin.y,
            origin.x + width,
            origin.y + f64::from(style.line_height),
        ),
        clip: Some(clip),
    }
}
