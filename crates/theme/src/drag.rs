//! Etiqueta do arrasto: o texto do item com o fundo, a borda e a sombra do `Flyout`.

use tomni_ui_core::PopupKind;
use tomni_ui_render::kurbo::{Point, Rect, Size};
use tomni_ui_render::{Canvas, Paint};
use tomni_ui_text::TextStyle;

use crate::{FluentDrawer, TextInfo, popup};

/// Padding da etiqueta: o do `ToolTip` (8,5,8,7) com 12 dos lados, para o texto não encostar
/// no raio de 8 do `Flyout`.
const PADDING: (f64, f64, f64, f64) = (12.0, 5.0, 12.0, 7.0);
/// Fonte do texto (`ControlContentThemeFontSize`).
const STYLE: TextStyle = TextStyle::BODY;

fn origin(rect: Rect) -> Point {
    Point::new(rect.x0 + PADDING.0, rect.y0 + PADDING.1)
}

pub(crate) fn measure(drawer: &FluentDrawer, label: &str) -> Size {
    let text = f64::from(drawer.fonts.borrow_mut().layout(label, STYLE, None).width());
    Size::new(
        (PADDING.0 + text + PADDING.2).ceil(),
        PADDING.1 + f64::from(STYLE.line_height) + PADDING.3,
    )
}

pub(crate) fn draw(drawer: &FluentDrawer, canvas: &mut dyn Canvas, rect: Rect, label: &str) {
    let t = &drawer.tokens;
    popup::draw_popup(canvas, rect, PopupKind::Flyout, t);
    drawer.fonts.borrow_mut().layout(label, STYLE, None).draw(
        canvas,
        origin(rect),
        &Paint::Solid(t.text_fill_primary),
    );
}

pub(crate) fn info(drawer: &FluentDrawer, rect: Rect, label: &str) -> Vec<TextInfo> {
    if label.is_empty() {
        return Vec::new();
    }
    let t = &drawer.tokens;
    vec![crate::text::line_info(
        drawer,
        label,
        STYLE,
        origin(rect),
        (t.text_fill_primary, popup::popup_surface(t)),
        rect,
    )]
}
