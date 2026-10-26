//! `WebView`: a área da página. A página de verdade é uma janela do sistema por cima; aqui
//! fica o fundo dela, a captura que a substitui sob um popup e, no motor falso (testes e
//! ferramentas), um quadro com o globo e o endereço.

use std::sync::Arc;

use tomni_ui_core::Symbol;
use tomni_ui_render::kurbo::{Point, Rect, Size};
use tomni_ui_render::peniko::{Blob, ImageAlphaType, ImageFormat};
use tomni_ui_render::{Canvas, CanvasExt, ImageData, Paint};
use tomni_ui_text::TextStyle;
use tomni_ui_webview::{WEBVIEW_MIN_SIZE, WebViewVisual};

use crate::{FluentDrawer, TextInfo, icon, over};

/// Lado do globo do quadro.
const GLOBE: f64 = 32.0;
/// Espaço entre o globo e o endereço.
const GAP: f64 = 8.0;
/// Margem do endereço nas laterais.
const MARGIN: f64 = 12.0;

pub(crate) fn measure() -> Size {
    WEBVIEW_MIN_SIZE
}

/// Se o quadro do motor falso aparece: a página está à mostra, mas não há janela do sistema
/// nem captura.
fn placeholder(visual: &WebViewVisual) -> bool {
    visual.page && !visual.native && visual.capture.is_none()
}

/// O fundo da área, opaco: o `LayerFillColorDefault` sobre o fundo da janela.
fn background(drawer: &FluentDrawer) -> tomni_ui_render::peniko::Color {
    over(drawer.tokens.layer_fill_default, drawer.backdrop())
}

/// Onde ficam o globo e o endereço do quadro.
fn layout(rect: Rect) -> (Rect, Point, f64) {
    let style = TextStyle::CAPTION;
    let height = GLOBE + GAP + f64::from(style.line_height);
    let top = rect.y0 + ((rect.height() - height) / 2.0).round();
    let x = rect.x0 + ((rect.width() - GLOBE) / 2.0).round();
    let globe = Rect::new(x, top, x + GLOBE, top + GLOBE);
    let width = (rect.width() - 2.0 * MARGIN).max(0.0);
    (
        globe,
        Point::new(rect.x0 + MARGIN, top + GLOBE + GAP),
        width,
    )
}

pub(crate) fn draw(
    drawer: &FluentDrawer,
    canvas: &mut dyn Canvas,
    rect: Rect,
    visual: &WebViewVisual,
) {
    if rect.is_zero_area() {
        return;
    }
    canvas.fill(&rect, Paint::Solid(background(drawer)));
    if let Some(capture) = &visual.capture {
        let image = ImageData {
            data: Blob::new(Arc::clone(&capture.pixels) as Arc<dyn AsRef<[u8]> + Send + Sync>),
            format: ImageFormat::Rgba8,
            alpha_type: ImageAlphaType::AlphaPremultiplied,
            width: capture.width,
            height: capture.height,
        };
        canvas.draw_image(&image, rect);
        return;
    }
    if !placeholder(visual) {
        return;
    }
    let (globe, origin, width) = layout(rect);
    let secondary = drawer.tokens.text_fill_secondary;
    icon::draw_symbol(canvas, Symbol::Globe, globe, secondary);
    let (text, _) = drawer.fonts.borrow_mut().layout_lines(
        &visual.url,
        TextStyle::CAPTION,
        Some(width as f32),
        1,
    );
    let x = origin.x + ((width - f64::from(text.width())) / 2.0).round().max(0.0);
    text.draw(canvas, Point::new(x, origin.y), &Paint::Solid(secondary));
}

pub(crate) fn text_info(
    drawer: &FluentDrawer,
    rect: Rect,
    visual: &WebViewVisual,
) -> Vec<TextInfo> {
    if !placeholder(visual) || visual.url.is_empty() {
        return Vec::new();
    }
    let (_, origin, width) = layout(rect);
    let style = TextStyle::CAPTION;
    let (text, _) =
        drawer
            .fonts
            .borrow_mut()
            .layout_lines(&visual.url, style, Some(width as f32), 1);
    let text_width = f64::from(text.width());
    let x = origin.x + ((width - text_width) / 2.0).round().max(0.0);
    let background = background(drawer);
    vec![TextInfo {
        text: visual.url.clone(),
        color: over(drawer.tokens.text_fill_secondary, background),
        background,
        style,
        bounds: Rect::new(
            x,
            origin.y,
            x + text_width,
            origin.y + f64::from(style.line_height),
        ),
        clip: Some(rect),
    }]
}
