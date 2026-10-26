//! `NavigationView` no visual do WinUI: os itens do painel, o painel aberto por cima do
//! conteúdo e o fundo do conteúdo.

use tomni_ui_core::{ButtonState, DockSide, NAV_INDENT, NavigationItemVisual, Symbol};
use tomni_ui_render::kurbo::{Affine, Point, Rect, RoundedRect, RoundedRectRadii, Shape, Size};
use tomni_ui_render::peniko::Color;
use tomni_ui_render::{Canvas, CanvasExt, PATH_TOLERANCE, Paint};
use tomni_ui_text::TextStyle;

use crate::{FluentDrawer, GraphicInfo, TextInfo, Tokens, icon, icons, metrics};

/// Distância do fundo do item às bordas dele, dos lados: com os 2 px da lista, os 4 px do
/// `NavigationViewItemButtonMargin`.
const INSET: f64 = 2.0;
/// Caixa do ícone (`NavigationViewItemOnLeftIconBoxHeight` numa coluna de 40).
const ICON_BOX: f64 = 40.0;
/// Lado do ícone.
const ICON: f64 = 16.0;
/// Espaço entre a caixa do ícone e o rótulo.
const LABEL_GAP: f64 = 4.0;
/// Espaço à direita do rótulo, sem a seta.
const LABEL_END: f64 = 16.0;
/// Coluna da seta dos grupos, à direita (`NavigationViewItemExpandChevronSize` com a margem).
const CHEVRON_COLUMN: f64 = 40.0;
/// Lado da seta.
const CHEVRON: f64 = 12.0;
/// Indicador de escolhido (`NavigationViewSelectionIndicatorWidth` e `Height`).
const INDICATOR: Size = Size::new(3.0, 16.0);
/// `NavigationViewSelectionIndicatorRadius`.
const INDICATOR_RADIUS: f64 = 2.0;
/// `OverlayCornerRadius`: o painel por cima do conteúdo, à direita, e o canto do conteúdo.
const OVERLAY_RADIUS: f64 = 8.0;
/// Fonte do rótulo.
const STYLE: TextStyle = TextStyle::BODY;

/// Fundo do item: os 44 px de cima (32 no compacto), sem as margens dos lados.
fn pill(rect: Rect, visual: NavigationItemVisual) -> Rect {
    Rect::new(
        rect.x0 + INSET,
        rect.y0,
        rect.x1 - INSET,
        rect.y1.min(rect.y0 + visual.height()),
    )
}

/// Fundo e cor do texto e do ícone, como no WinUI: o escolhido clareia no hover
/// (`SubtleFillColorTertiary`) e volta ao `SubtleFillColorSecondary` pressionado.
fn colors(t: &Tokens, visual: NavigationItemVisual) -> (Color, Color) {
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

/// Começo da caixa do ícone: recuada pelo nível, fora do painel compacto.
fn content_x(pill: Rect, visual: NavigationItemVisual) -> f64 {
    if visual.compact {
        pill.x0
    } else {
        pill.x0 + NAV_INDENT * f64::from(visual.depth)
    }
}

/// Onde o rótulo precisa caber.
fn label_clip(pill: Rect, visual: NavigationItemVisual) -> Rect {
    let end = if visual.expanded.is_some() {
        CHEVRON_COLUMN
    } else {
        LABEL_END
    };
    let x0 = content_x(pill, visual) + ICON_BOX + LABEL_GAP;
    Rect::new(x0, pill.y0, (pill.x1 - end).max(x0), pill.y1)
}

fn label_origin(pill: Rect, visual: NavigationItemVisual) -> Point {
    Point::new(
        label_clip(pill, visual).x0,
        pill.y0 + ((pill.height() - f64::from(STYLE.line_height)) / 2.0).floor(),
    )
}

fn icon_square(pill: Rect, visual: NavigationItemVisual) -> Rect {
    let x = content_x(pill, visual);
    icon::centered(Rect::new(x, pill.y0, x + ICON_BOX, pill.y1), ICON)
}

pub(crate) fn measure_item(
    drawer: &FluentDrawer,
    label: &str,
    visual: NavigationItemVisual,
) -> Size {
    if visual.compact {
        return Size::new(ICON_BOX + 2.0 * INSET, visual.height());
    }
    let width = f64::from(drawer.fonts.borrow_mut().layout(label, STYLE, None).width());
    let end = if visual.expanded.is_some() {
        CHEVRON_COLUMN
    } else {
        LABEL_END
    };
    Size::new(
        (2.0 * INSET + NAV_INDENT * f64::from(visual.depth) + ICON_BOX + LABEL_GAP + width + end)
            .ceil(),
        visual.height(),
    )
}

pub(crate) fn draw_item(
    drawer: &FluentDrawer,
    canvas: &mut dyn Canvas,
    rect: Rect,
    label: &str,
    symbol: Symbol,
    visual: NavigationItemVisual,
) {
    let t = &drawer.tokens;
    let pill = pill(rect, visual);
    let radius = metrics::CONTROL_CORNER_RADIUS;
    let (fill, foreground) = colors(t, visual);
    if fill != Color::TRANSPARENT {
        canvas.fill(&RoundedRect::from_rect(pill, radius), fill);
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
    icon::draw_symbol(canvas, symbol, icon_square(pill, visual), foreground);
    if !visual.compact && !label.is_empty() {
        canvas.push_clip(&label_clip(pill, visual).to_path(PATH_TOLERANCE));
        drawer.fonts.borrow_mut().layout(label, STYLE, None).draw(
            canvas,
            label_origin(pill, visual),
            &Paint::Solid(foreground),
        );
        canvas.pop_clip();
    }
    if let Some(open) = visual.expanded.filter(|_| !visual.compact) {
        let column = Rect::new(pill.x1 - CHEVRON_COLUMN, pill.y0, pill.x1, pill.y1);
        let square = icon::centered(column, CHEVRON);
        let mut path = icons::CHEVRON_DOWN_12.path(square.origin(), CHEVRON);
        if open {
            // Para cima: a de baixo espelhada em torno do centro da coluna.
            let cy = square.center().y;
            path.apply_affine(Affine::translate((0.0, 2.0 * cy)) * Affine::FLIP_Y);
        }
        canvas.fill_path(&path, &Paint::Solid(foreground));
    }
    if visual.focus_ring {
        crate::draw_focus_ring(canvas, pill, radius, t);
    }
}

pub(crate) fn item_info(
    drawer: &FluentDrawer,
    rect: Rect,
    label: &str,
    visual: NavigationItemVisual,
) -> Vec<TextInfo> {
    if visual.compact || label.is_empty() {
        return Vec::new();
    }
    let pill = pill(rect, visual);
    let (fill, foreground) = colors(&drawer.tokens, visual);
    let background = crate::over(fill, drawer.backdrop());
    vec![crate::text::line_info(
        drawer,
        label,
        STYLE,
        label_origin(pill, visual),
        (foreground, background),
        label_clip(pill, visual),
    )]
}

pub(crate) fn item_graphic(
    drawer: &FluentDrawer,
    rect: Rect,
    symbol: Symbol,
    visual: NavigationItemVisual,
) -> GraphicInfo {
    let pill = pill(rect, visual);
    let (fill, foreground) = colors(&drawer.tokens, visual);
    let background = crate::over(fill, drawer.backdrop());
    GraphicInfo {
        name: symbol.name(),
        color: crate::over(foreground, background),
        background,
        bounds: icon_square(pill, visual),
    }
}

/// Painel aberto por cima do conteúdo: acrílico (a cor de reserva, sem o desfoque), borda,
/// cantos de 8 à direita e sombra. Ao lado do conteúdo, o painel não pinta nada.
pub(crate) fn draw_pane(canvas: &mut dyn Canvas, rect: Rect, overlay: bool, t: &Tokens) {
    if !overlay {
        return;
    }
    crate::popup::draw_shadow(canvas, rect, OVERLAY_RADIUS, t);
    let border = metrics::BORDER;
    crate::expander::card(
        canvas,
        rect,
        RoundedRectRadii::new(0.0, OVERLAY_RADIUS, OVERLAY_RADIUS, 0.0),
        (0.0, border, border, border),
        t.acrylic_in_app_fill_default,
        t.divider_stroke_default,
    );
}

/// Painel do `DockHost` aberto por cima do conteúdo: como o do `NavigationView` (acrílico, borda
/// e sombra), com os cantos arredondados do lado de dentro e sem borda no lado da janela em que
/// encosta.
pub(crate) fn draw_dock_overlay(canvas: &mut dyn Canvas, rect: Rect, side: DockSide, t: &Tokens) {
    crate::popup::draw_shadow(canvas, rect, OVERLAY_RADIUS, t);
    let border = metrics::BORDER;
    let r = OVERLAY_RADIUS;
    // Raios na ordem canto de cima à esquerda, de cima à direita, de baixo à direita, de baixo
    // à esquerda; bordas na ordem esquerda, em cima, direita, embaixo.
    let (radii, borders) = match side {
        DockSide::Left => (
            RoundedRectRadii::new(0.0, r, r, 0.0),
            (0.0, border, border, border),
        ),
        DockSide::Right => (
            RoundedRectRadii::new(r, 0.0, 0.0, r),
            (border, border, 0.0, border),
        ),
        DockSide::Bottom => (
            RoundedRectRadii::new(r, r, 0.0, 0.0),
            (border, border, border, 0.0),
        ),
    };
    crate::expander::card(
        canvas,
        rect,
        radii,
        borders,
        t.acrylic_in_app_fill_default,
        t.divider_stroke_default,
    );
}

/// Fundo do conteúdo: `LayerFillColorDefault` com a borda em cima e à esquerda e o canto de
/// cima arredondado; no modo mínimo, só a borda de cima, sem canto.
pub(crate) fn draw_content(canvas: &mut dyn Canvas, rect: Rect, minimal: bool, t: &Tokens) {
    let border = metrics::BORDER;
    let (radius, left) = if minimal {
        (0.0, 0.0)
    } else {
        (OVERLAY_RADIUS, border)
    };
    crate::expander::card(
        canvas,
        rect,
        RoundedRectRadii::new(radius, 0.0, 0.0, 0.0),
        (left, border, 0.0, 0.0),
        t.layer_fill_default,
        t.card_stroke_default,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;
    use tomni_ui_core::Density;

    fn visual(selected: bool, state: ButtonState) -> NavigationItemVisual {
        NavigationItemVisual {
            selected,
            state,
            ..NavigationItemVisual::default()
        }
    }

    #[test]
    fn cores_do_item() {
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
            colors(&t, visual(false, ButtonState::Hover)).0,
            t.subtle_fill_secondary
        );
        assert_eq!(
            colors(&t, visual(false, ButtonState::Pressed)),
            (t.subtle_fill_tertiary, t.text_fill_secondary)
        );
        assert_eq!(
            colors(&t, visual(true, ButtonState::Hover)).0,
            t.subtle_fill_tertiary
        );
        assert_eq!(
            colors(&t, visual(true, ButtonState::Pressed)),
            (t.subtle_fill_secondary, t.text_fill_secondary)
        );
    }

    #[test]
    fn recuo_e_rotulo() {
        let rect = Rect::new(0.0, 0.0, 316.0, 132.0);
        let mut v = visual(false, ButtonState::Rest);
        let pill = pill(rect, v);
        assert_eq!(pill, Rect::new(2.0, 0.0, 314.0, 44.0));
        assert_eq!(label_clip(pill, v).x0, 46.0);
        v.depth = 1;
        v.expanded = Some(false);
        assert_eq!(label_clip(pill, v), Rect::new(77.0, 0.0, 274.0, 44.0));
        v.compact = true;
        assert_eq!(icon_square(pill, v), Rect::new(14.0, 14.0, 30.0, 30.0));
    }

    #[test]
    fn item_compacto_tem_32_px() {
        let mut v = visual(false, ButtonState::Rest);
        v.density = Density::Compact;
        let pill = pill(Rect::new(0.0, 0.0, 316.0, 132.0), v);
        assert_eq!(pill, Rect::new(2.0, 0.0, 314.0, 32.0));
        v.compact = true;
        assert_eq!(icon_square(pill, v), Rect::new(14.0, 8.0, 30.0, 24.0));
    }
}
