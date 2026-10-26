//! Itens da `MenuBar` no visual do WinUI (`MenuBar_themeresources.xaml`).

use tomni_ui_core::{ButtonState, MenuBarItemVisual};
use tomni_ui_render::kurbo::{Point, Rect, RoundedRect, Size};
use tomni_ui_render::peniko::Color;
use tomni_ui_render::{Canvas, CanvasExt, Paint};
use tomni_ui_text::TextStyle;

use crate::{FluentDrawer, TextInfo, Tokens, metrics};

/// `MenuBarItemButtonPadding`: 10 dos lados.
const PADDING: f64 = 10.0;
/// Altura do item: os 40 px da barra menos a margem de 4 em cima e embaixo.
const HEIGHT: f64 = 32.0;
/// Fonte do rótulo (`ControlContentThemeFontSize`).
const STYLE: TextStyle = TextStyle::BODY;

/// Fundo e cor do rótulo: `SubtleFillColorSecondary` no hover, `Tertiary` pressionado e com
/// o menu aberto. A borda (`ControlStrokeColorDefault` no hover, pressionado e aberto) tem
/// espessura 0 fora do alto contraste (`MenuBarItemBorderThickness`), então não aparece.
fn colors(t: &Tokens, visual: MenuBarItemVisual) -> (Color, Color) {
    match visual.state {
        ButtonState::Disabled => (Color::TRANSPARENT, t.text_fill_disabled),
        _ if visual.open => (t.subtle_fill_tertiary, t.text_fill_primary),
        ButtonState::Pressed => (t.subtle_fill_tertiary, t.text_fill_primary),
        ButtonState::Hover => (t.subtle_fill_secondary, t.text_fill_primary),
        ButtonState::Rest => (Color::TRANSPARENT, t.text_fill_primary),
    }
}

fn origin(rect: Rect) -> Point {
    Point::new(
        rect.x0 + PADDING,
        rect.y0 + ((rect.height() - f64::from(STYLE.line_height)) / 2.0).floor(),
    )
}

pub(crate) fn measure_item(drawer: &FluentDrawer, label: &str) -> Size {
    let text = f64::from(drawer.fonts.borrow_mut().layout(label, STYLE, None).width());
    Size::new((PADDING + text + PADDING).ceil(), HEIGHT)
}

pub(crate) fn draw_item(
    drawer: &FluentDrawer,
    canvas: &mut dyn Canvas,
    rect: Rect,
    label: &str,
    visual: MenuBarItemVisual,
) {
    let t = &drawer.tokens;
    let (fill, text) = colors(t, visual);
    let radius = metrics::CONTROL_CORNER_RADIUS;
    if fill != Color::TRANSPARENT {
        canvas.fill(&RoundedRect::from_rect(rect, radius), fill);
    }
    drawer.fonts.borrow_mut().layout(label, STYLE, None).draw(
        canvas,
        origin(rect),
        &Paint::Solid(text),
    );
    if visual.focus_ring {
        crate::draw_focus_ring(canvas, rect.inset(-metrics::FOCUS_MARGIN), radius, t);
    }
}

pub(crate) fn item_info(
    drawer: &FluentDrawer,
    rect: Rect,
    label: &str,
    visual: MenuBarItemVisual,
) -> Vec<TextInfo> {
    if label.is_empty() {
        return Vec::new();
    }
    let (fill, text) = colors(&drawer.tokens, visual);
    let background = crate::over(fill, drawer.backdrop());
    vec![crate::text::line_info(
        drawer,
        label,
        STYLE,
        origin(rect),
        (text, background),
        rect,
    )]
}
