//! `TreeView` no visual do WinUI: a linha com o chevron, o ícone e o rótulo, o realce da
//! pasta que vai receber o arrasto e a linha de inserção.

use std::f64::consts::FRAC_PI_2;

use tomni_ui_core::{
    ButtonState, DropMark, ItemStatus, Symbol, TREE_VIEW_INDENT, TreeViewItemVisual,
};
use tomni_ui_render::kurbo::{Affine, Circle, Point, Rect, RoundedRect, Shape, Size};
use tomni_ui_render::peniko::Color;
use tomni_ui_render::{Canvas, CanvasExt, PATH_TOLERANCE, Paint};
use tomni_ui_text::TextStyle;

use crate::{FluentDrawer, GraphicInfo, TextInfo, Tokens, icon, icons, metrics};

/// Lado do chevron.
const CHEVRON: f64 = 12.0;
/// Espaço à direita do rótulo.
const LABEL_END: f64 = 8.0;
/// Indicador de selecionada (`TreeViewItemSelectionIndicator`).
const INDICATOR: Size = Size::new(3.0, 16.0);
/// Raio do indicador.
const INDICATOR_RADIUS: f64 = 2.0;
/// Espessura da linha de inserção.
const INSERT_THICKNESS: f64 = 2.0;
/// Diâmetro do círculo na ponta da linha de inserção.
const INSERT_DOT: f64 = 6.0;
/// Lado do selo que vai depois do rótulo.
const BADGE: f64 = 12.0;
/// Espaço entre o rótulo e o selo.
const BADGE_GAP: f64 = 6.0;
/// Largura do contorno do selo de estado, na cor do fundo da linha.
const STATUS_OUTLINE: f64 = 1.0;
/// Espessura do anel do selo de estado.
const STATUS_RING: f64 = 2.0;
/// Fonte do rótulo.
const STYLE: TextStyle = TextStyle::BODY;

/// Fundo e cor do texto e do ícone, como no WinUI (`TreeViewItem`).
pub(crate) fn colors(t: &Tokens, visual: TreeViewItemVisual) -> (Color, Color) {
    match visual.state {
        ButtonState::Pressed if visual.selected => (t.subtle_fill_secondary, t.text_fill_secondary),
        ButtonState::Hover if visual.selected => (t.subtle_fill_tertiary, t.text_fill_primary),
        ButtonState::Pressed => (t.subtle_fill_tertiary, t.text_fill_secondary),
        ButtonState::Hover => (t.subtle_fill_secondary, t.text_fill_primary),
        ButtonState::Disabled => (Color::TRANSPARENT, t.text_fill_disabled),
        ButtonState::Rest if visual.selected => (t.subtle_fill_secondary, t.text_fill_primary),
        ButtonState::Rest => (Color::TRANSPARENT, t.text_fill_primary),
    }
}

/// A cor do selo: a secundária do texto, que some junto com o resto quando a linha está
/// desabilitada.
fn secondary(t: &Tokens, visual: TreeViewItemVisual) -> Color {
    if visual.state == ButtonState::Disabled {
        t.text_fill_disabled
    } else {
        t.text_fill_secondary
    }
}

/// Fundo a pintar: o realce de soltar dentro da pasta vale mais que o estado.
fn fill(t: &Tokens, visual: TreeViewItemVisual) -> Color {
    if visual.drop == Some(DropMark::Into) {
        t.subtle_fill_secondary
    } else {
        colors(t, visual).0
    }
}

/// Onde o rótulo precisa caber.
fn label_clip(rect: Rect, visual: TreeViewItemVisual) -> Rect {
    let pill = visual.pill(rect);
    let x0 = visual.label_x(rect);
    Rect::new(x0, pill.y0, (pill.x1 - LABEL_END).max(x0), pill.y1)
}

fn label_origin(rect: Rect, visual: TreeViewItemVisual) -> Point {
    let pill = visual.pill(rect);
    Point::new(
        visual.label_x(rect),
        pill.y0 + ((pill.height() - f64::from(STYLE.line_height)) / 2.0).floor(),
    )
}

/// O quadrado do selo: logo depois do rótulo, centrado na altura da linha. `None` se o rótulo
/// já ocupa a linha e o selo não cabe.
fn badge_square(
    drawer: &FluentDrawer,
    rect: Rect,
    label: &str,
    visual: TreeViewItemVisual,
) -> Option<Rect> {
    let clip = label_clip(rect, visual);
    let width = if label.is_empty() || visual.renaming {
        0.0
    } else {
        f64::from(drawer.fonts.borrow_mut().layout(label, STYLE, None).width())
    };
    let x = clip.x0 + width.ceil() + BADGE_GAP;
    let pill = visual.pill(rect);
    let y = pill.y0 + ((pill.height() - BADGE) / 2.0).floor();
    (x + BADGE <= clip.x1).then(|| Rect::new(x, y, x + BADGE, y + BADGE))
}

pub(crate) fn draw_item(
    drawer: &FluentDrawer,
    canvas: &mut dyn Canvas,
    rect: Rect,
    label: &str,
    symbol: Symbol,
    badge: Option<Symbol>,
    visual: TreeViewItemVisual,
) {
    let t = &drawer.tokens;
    let pill = visual.pill(rect);
    let radius = metrics::CONTROL_CORNER_RADIUS;
    let (_, foreground) = colors(t, visual);
    let background = fill(t, visual);
    if background != Color::TRANSPARENT {
        canvas.fill(&RoundedRect::from_rect(pill, radius), background);
    }
    if visual.drop == Some(DropMark::Into) {
        let border = metrics::BORDER;
        canvas.fill_path(
            &crate::ring(
                pill,
                radius,
                pill.inflate(-border, -border),
                radius - border,
            ),
            &Paint::Solid(t.accent_fill_default),
        );
    }
    if visual.selected {
        let y = pill.y0 + ((pill.height() - INDICATOR.height) / 2.0).round();
        canvas.fill(
            &RoundedRect::from_rect(
                Rect::from_origin_size((pill.x0, y), INDICATOR),
                INDICATOR_RADIUS,
            ),
            t.accent_fill_default,
        );
    }
    if visual.has_children {
        let square = icon::centered(visual.chevron_column(rect), CHEVRON);
        let mut path = icons::CHEVRON_DOWN_12.path(square.origin(), CHEVRON);
        if !visual.expanded {
            // Para a direita: a de baixo girada um quarto de volta, no centro do glifo.
            path.apply_affine(Affine::rotate_about(-FRAC_PI_2, square.center()));
        }
        canvas.fill_path(&path, &Paint::Solid(foreground));
    }
    icon::draw_symbol(canvas, symbol, visual.icon_square(rect), foreground);
    if !visual.renaming && !label.is_empty() {
        canvas.push_clip(&label_clip(rect, visual).to_path(PATH_TOLERANCE));
        drawer.fonts.borrow_mut().layout(label, STYLE, None).draw(
            canvas,
            label_origin(rect, visual),
            &Paint::Solid(foreground),
        );
        canvas.pop_clip();
    }
    if let Some(badge) = badge
        && let Some(square) = badge_square(drawer, rect, label, visual)
    {
        icon::draw_symbol(canvas, badge, square, secondary(t, visual));
    }
    if let Some(status) = visual.status {
        draw_status(drawer, canvas, rect, status, visual);
    }
    if let Some(DropMark::Top(level) | DropMark::Bottom(level)) = visual.drop {
        let y = if matches!(visual.drop, Some(DropMark::Top(_))) {
            rect.y0
        } else {
            rect.y1
        };
        let x0 = visual.insert_x(rect, level);
        let x1 = (rect.x1 - TREE_VIEW_INDENT / 4.0).max(x0);
        canvas.push_clip(&rect.inflate(0.0, INSERT_DOT / 2.0).to_path(PATH_TOLERANCE));
        canvas.fill(
            &Rect::new(
                x0,
                y - INSERT_THICKNESS / 2.0,
                x1,
                y + INSERT_THICKNESS / 2.0,
            ),
            t.accent_fill_default,
        );
        canvas.fill(
            &Circle::new((x0, y), INSERT_DOT / 2.0),
            t.accent_fill_default,
        );
        canvas.pop_clip();
    }
    if visual.focus_ring {
        crate::draw_focus_ring(canvas, pill, radius, t);
    }
}

/// A cor do selo de estado.
fn status_color(t: &Tokens, status: ItemStatus) -> Color {
    match status {
        ItemStatus::Success => t.info_bar_success_icon,
        ItemStatus::Caution => t.info_bar_warning_icon,
        ItemStatus::Critical => t.info_bar_error_icon,
        ItemStatus::Neutral => t.text_fill_tertiary,
    }
}

/// O fundo da linha, opaco: o contorno do selo tem a cor que a linha tem naquele estado.
fn row_background(drawer: &FluentDrawer, visual: TreeViewItemVisual) -> Color {
    crate::over(fill(&drawer.tokens, visual), drawer.backdrop())
}

/// O selo de estado no canto do ícone, como o `InfoBadge` do WinUI: cheio (sucesso), com "!"
/// (erro) ou em anel (cautela e neutro), com um contorno de 1 px na cor do fundo da linha.
fn draw_status(
    drawer: &FluentDrawer,
    canvas: &mut dyn Canvas,
    rect: Rect,
    status: ItemStatus,
    visual: TreeViewItemVisual,
) {
    let t = &drawer.tokens;
    let square = visual.status_square(rect);
    let center = square.center();
    let radius = square.width() / 2.0;
    let color = status_color(t, status);
    let background = row_background(drawer, visual);
    canvas.fill(&Circle::new(center, radius + STATUS_OUTLINE), background);
    canvas.fill(&Circle::new(center, radius), color);
    match status {
        ItemStatus::Success => {}
        ItemStatus::Caution | ItemStatus::Neutral => {
            canvas.fill(&Circle::new(center, radius - STATUS_RING), background);
        }
        ItemStatus::Critical => {
            // O "!" são dois retângulos: o glifo da fonte não fica legível em 10 px.
            let ink = t.text_on_accent_primary;
            let (x, y) = (center.x - 0.75, center.y);
            canvas.fill(&Rect::new(x, y - 3.0, x + 1.5, y + 0.5), ink);
            canvas.fill(&Rect::new(x, y + 1.5, x + 1.5, y + 3.0), ink);
        }
    }
}

pub(crate) fn item_info(
    drawer: &FluentDrawer,
    rect: Rect,
    label: &str,
    visual: TreeViewItemVisual,
) -> Vec<TextInfo> {
    if visual.renaming || label.is_empty() {
        return Vec::new();
    }
    let (_, foreground) = colors(&drawer.tokens, visual);
    let background = crate::over(fill(&drawer.tokens, visual), drawer.backdrop());
    vec![crate::text::line_info(
        drawer,
        label,
        STYLE,
        label_origin(rect, visual),
        (foreground, background),
        label_clip(rect, visual),
    )]
}

pub(crate) fn item_graphic(
    drawer: &FluentDrawer,
    rect: Rect,
    label: &str,
    symbol: Symbol,
    badge: Option<Symbol>,
    visual: TreeViewItemVisual,
) -> Vec<GraphicInfo> {
    let (_, foreground) = colors(&drawer.tokens, visual);
    let background = crate::over(fill(&drawer.tokens, visual), drawer.backdrop());
    let mut graphics = vec![GraphicInfo {
        name: symbol.name(),
        color: crate::over(foreground, background),
        background,
        bounds: visual.icon_square(rect),
    }];
    if let Some(badge) = badge
        && let Some(square) = badge_square(drawer, rect, label, visual)
    {
        graphics.push(GraphicInfo {
            name: badge.name(),
            color: crate::over(secondary(&drawer.tokens, visual), background),
            background,
            bounds: square,
        });
    }
    if let Some(status) = visual.status {
        graphics.push(GraphicInfo {
            name: status.name(),
            color: status_color(&drawer.tokens, status),
            background: row_background(drawer, visual),
            bounds: visual.status_square(rect),
        });
    }
    graphics
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;

    fn visual(selected: bool, state: ButtonState) -> TreeViewItemVisual {
        TreeViewItemVisual {
            selected,
            state,
            ..TreeViewItemVisual::default()
        }
    }

    #[test]
    fn cores_da_linha() {
        let t = Tokens::new(Theme::Light);
        assert_eq!(
            colors(&t, visual(false, ButtonState::Rest)).0,
            Color::TRANSPARENT
        );
        assert_eq!(
            colors(&t, visual(true, ButtonState::Rest)).0,
            t.subtle_fill_secondary
        );
        assert_eq!(
            colors(&t, visual(true, ButtonState::Hover)).0,
            t.subtle_fill_tertiary
        );
        assert_eq!(
            colors(&t, visual(true, ButtonState::Pressed)),
            (t.subtle_fill_secondary, t.text_fill_secondary)
        );
        assert_eq!(
            colors(&t, visual(false, ButtonState::Disabled)),
            (Color::TRANSPARENT, t.text_fill_disabled)
        );
    }

    #[test]
    fn realce_da_pasta_vale_mais_que_o_estado() {
        let t = Tokens::new(Theme::Light);
        let mut v = visual(false, ButtonState::Rest);
        v.drop = Some(DropMark::Into);
        assert_eq!(fill(&t, v), t.subtle_fill_secondary);
    }

    #[test]
    fn rotulo_cabe_depois_do_icone() {
        let rect = Rect::new(0.0, 0.0, 300.0, 44.0);
        let v = visual(false, ButtonState::Rest);
        assert_eq!(label_clip(rect, v), Rect::new(68.0, 2.0, 288.0, 42.0));
    }
}
