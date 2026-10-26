//! Texto com estilos por trecho e links (`RichText`).

use std::cell::RefCell;

use tomni_ui_core::{RichTextVisual, Token};
use tomni_ui_render::kurbo::{Rect, RoundedRect, Size};
use tomni_ui_render::peniko::Color;
use tomni_ui_render::{Canvas, CanvasExt, Paint};
use tomni_ui_text::{Fonts, RunStyle, TextLayout};

use crate::text::{block_size, ramp_style};
use crate::{FluentDrawer, TextInfo, Tokens, draw_focus_ring};

/// Folga do fundo do código além do texto, dos lados.
const CODE_PADDING: f64 = 2.0;
/// Raio do fundo do código.
const CODE_RADIUS: f64 = 3.0;

/// Papel de uma faixa na hora de escolher a cor.
#[derive(Clone, Copy, Debug)]
enum Role {
    Text,
    Link(usize),
}

/// As faixas de estilo de `visual`: uma por trecho com estilo ou link, com o id `índice + 1`.
/// Devolve também o papel de cada uma, na mesma ordem dos ids.
fn runs(visual: &RichTextVisual) -> (Vec<RunStyle>, Vec<Role>) {
    let links = visual.links();
    let mut runs = Vec::new();
    let mut roles = Vec::new();
    let mut start = 0;
    for span in &visual.spans {
        let range = start..start + span.text.len();
        start = range.end;
        let styled = visual.mono
            || span.bold
            || span.italic
            || span.strike
            || span.code
            || span.link.is_some();
        if range.is_empty() || !styled {
            continue;
        }
        let link = span
            .link
            .as_ref()
            .and_then(|_| links.iter().position(|l| l.range.contains(&range.start)));
        runs.push(RunStyle {
            range,
            id: runs.len() as u32 + 1,
            bold: span.bold,
            italic: span.italic,
            mono: visual.mono || span.code,
            underline: link.is_some() && link == visual.hovered.or(visual.pressed),
            strike: span.strike,
        });
        roles.push(link.map_or(Role::Text, Role::Link));
    }
    (runs, roles)
}

/// Layout de `visual` em `max_width`, cortado em `max_lines`. O `bool` diz se cortou.
fn layout(
    fonts: &RefCell<Fonts>,
    visual: &RichTextVisual,
    max_width: Option<f64>,
) -> (TextLayout, bool, Vec<Role>) {
    let style = ramp_style(visual.ramp);
    let width = max_width.map(|w| w.max(0.0) as f32);
    let (runs, roles) = runs(visual);
    let text = visual.text();
    let mut fonts = fonts.borrow_mut();
    let (layout, trimmed) = match visual.max_lines {
        Some(lines) => fonts.layout_ranged_lines(&text, style, width, lines as usize, &runs),
        None => (fonts.layout_ranged(&text, style, width, &runs), false),
    };
    (layout, trimmed, roles)
}

fn wrap_width(rect: Rect, wrap: bool) -> Option<f64> {
    wrap.then(|| rect.width() + 0.5)
}

fn color(tokens: &Tokens, visual: &RichTextVisual) -> Color {
    tokens.resolve(visual.color.unwrap_or(Token::TextFillColorPrimary.into()))
}

/// Cor de um link no estado em que está.
fn link_color(tokens: &Tokens, visual: &RichTextVisual, link: usize) -> Color {
    if visual.pressed == Some(link) {
        tokens.accent_text_tertiary
    } else if visual.hovered == Some(link) {
        tokens.accent_text_secondary
    } else {
        tokens.accent_text_primary
    }
}

pub(crate) fn measure(
    drawer: &FluentDrawer,
    visual: &RichTextVisual,
    max_width: Option<f64>,
) -> Size {
    let (layout, ..) = layout(&drawer.fonts, visual, max_width);
    block_size(&layout, ramp_style(visual.ramp))
}

pub(crate) fn draw(
    drawer: &FluentDrawer,
    canvas: &mut dyn Canvas,
    rect: Rect,
    visual: &RichTextVisual,
) {
    let (layout, _, roles) = layout(&drawer.fonts, visual, wrap_width(rect, visual.wrap));
    let tokens = &drawer.tokens;
    let origin = rect.origin();
    let shift = origin.to_vec2();

    // Fundo do código, atrás do texto.
    let mut start = 0;
    for span in &visual.spans {
        let range = start..start + span.text.len();
        start = range.end;
        if !span.code || range.is_empty() {
            continue;
        }
        for line in layout.range_rects(range) {
            let fill = Rect::new(
                line.x0 - CODE_PADDING,
                line.y0 + 1.0,
                line.x1 + CODE_PADDING,
                line.y1 - 1.0,
            ) + shift;
            canvas.fill(
                &RoundedRect::from_rect(fill, CODE_RADIUS),
                Paint::Solid(tokens.subtle_fill_secondary),
            );
        }
    }

    let normal = color(tokens, visual);
    layout.draw_styled(canvas, origin, |id| {
        let role = (id as usize)
            .checked_sub(1)
            .and_then(|i| roles.get(i))
            .copied()
            .unwrap_or(Role::Text);
        Paint::Solid(match role {
            Role::Text => normal,
            Role::Link(n) => link_color(tokens, visual, n),
        })
    });

    // Anel de foco do link, só com o foco vindo do teclado.
    if visual.focus_ring
        && let Some(n) = visual.focused
        && let Some(link) = visual.links().get(n)
    {
        for line in layout.range_rects(link.range.clone()) {
            draw_focus_ring(canvas, (line + shift).inflate(1.0, 0.0), 2.0, tokens);
        }
    }
}

/// Retângulos dos links de `visual`, relativos ao canto do widget: um `Vec` por link, com um
/// retângulo por linha visual.
pub(crate) fn links(
    fonts: &RefCell<Fonts>,
    visual: &RichTextVisual,
    max_width: Option<f64>,
) -> Vec<Vec<Rect>> {
    let links = visual.links();
    if links.is_empty() {
        return Vec::new();
    }
    let (layout, ..) = layout(fonts, visual, max_width);
    links
        .into_iter()
        .map(|link| layout.range_rects(link.range))
        .collect()
}

pub(crate) fn trimmed(drawer: &FluentDrawer, rect: Rect, visual: &RichTextVisual) -> bool {
    visual.max_lines.is_some() && layout(&drawer.fonts, visual, wrap_width(rect, visual.wrap)).1
}

pub(crate) fn text_info(
    drawer: &FluentDrawer,
    rect: Rect,
    visual: &RichTextVisual,
) -> Vec<TextInfo> {
    let text = visual.text();
    if text.is_empty() {
        return Vec::new();
    }
    let style = ramp_style(visual.ramp);
    let (layout, ..) = layout(&drawer.fonts, visual, wrap_width(rect, visual.wrap));
    let size = Size::new(f64::from(layout.width()), block_size(&layout, style).height);
    let backdrop = drawer.backdrop();
    let mut infos = vec![TextInfo {
        text: text.clone(),
        color: crate::over(color(&drawer.tokens, visual), backdrop),
        background: backdrop,
        style,
        bounds: Rect::from_origin_size(rect.origin(), size),
        clip: Some(rect),
    }];
    for (n, link) in visual.links().into_iter().enumerate() {
        let Some(bounds) = layout
            .range_rects(link.range.clone())
            .into_iter()
            .reduce(|a, b| a.union(b))
        else {
            continue;
        };
        infos.push(TextInfo {
            text: text[link.range].to_owned(),
            color: crate::over(link_color(&drawer.tokens, visual, n), backdrop),
            background: backdrop,
            style,
            bounds: bounds + rect.origin().to_vec2(),
            clip: Some(rect),
        });
    }
    infos
}
