//! `PropertyGrid`: a linha de propriedade (nome, linha de divisão e valor), o cabeçalho de
//! categoria e o divisor da descrição. O fundo e o indicador da linha escolhida seguem o
//! `TreeViewItem` do WinUI; o chevron do cabeçalho, o do `Expander`.

use tomni_ui_core::{
    ButtonState, Density, PropertyGridCategoryVisual, PropertyGridRowVisual,
    PropertyGridSizerVisual, TreeViewItemVisual,
};
use tomni_ui_render::kurbo::{Affine, Point, Rect, RoundedRect, Shape, Size};
use tomni_ui_render::peniko::Color;
use tomni_ui_render::{Canvas, CanvasExt, PATH_TOLERANCE, Paint};
use tomni_ui_text::TextStyle;

use crate::{FluentDrawer, TextInfo, Tokens, expander, icon, icons, metrics, tree_view};

/// Espessura do divisor da descrição: a área de pegar.
const SIZER_HEIGHT: f64 = 8.0;
/// Espaço entre o texto e a borda direita da coluna.
const TEXT_END: f64 = 4.0;
/// Lado do glifo do chevron do cabeçalho.
const CHEVRON_GLYPH: f64 = 12.0;
/// Indicador de escolhida.
const INDICATOR: Size = Size::new(3.0, 16.0);
const INDICATOR_RADIUS: f64 = 2.0;

fn row_style(visual: PropertyGridRowVisual) -> TextStyle {
    if visual.changed {
        TextStyle::BODY_STRONG
    } else {
        TextStyle::BODY
    }
}

/// Fundo e cor do nome, como no `TreeViewItem`.
fn colors(t: &Tokens, selected: bool, state: ButtonState) -> (Color, Color) {
    tree_view::colors(
        t,
        TreeViewItemVisual {
            selected,
            state,
            ..TreeViewItemVisual::default()
        },
    )
}

/// Cor do valor: cinza se só leitura, apagada se desabilitada.
fn value_color(t: &Tokens, visual: PropertyGridRowVisual, name: Color) -> Color {
    if visual.disabled {
        t.text_fill_disabled
    } else if visual.read_only {
        t.text_fill_secondary
    } else {
        name
    }
}

/// Onde o valor começa: no mesmo x do texto do `Edit` da linha escolhida.
fn value_x(rect: Rect, visual: PropertyGridRowVisual) -> f64 {
    let padding = match visual.density {
        Density::Standard => metrics::TEXT_CONTROL_PADDING,
        Density::Compact => metrics::TEXT_CONTROL_PADDING_COMPACT,
    };
    visual.divider_x(rect) + 1.0 + metrics::BORDER + padding.0
}

fn text_y(rect: Rect, style: TextStyle) -> f64 {
    rect.y0 + ((rect.height() - f64::from(style.line_height)) / 2.0).floor()
}

fn name_clip(rect: Rect, visual: PropertyGridRowVisual) -> Rect {
    let x0 = visual.name_x(rect);
    Rect::new(
        x0,
        rect.y0,
        (visual.divider_x(rect) - TEXT_END).max(x0),
        rect.y1,
    )
}

fn value_clip(rect: Rect, visual: PropertyGridRowVisual) -> Rect {
    let x0 = value_x(rect, visual);
    Rect::new(x0, rect.y0, (rect.x1 - TEXT_END).max(x0), rect.y1)
}

fn indicator(canvas: &mut dyn Canvas, rect: Rect, t: &Tokens) {
    let y = rect.y0 + ((rect.height() - INDICATOR.height) / 2.0).round();
    canvas.fill(
        &RoundedRect::from_rect(
            Rect::from_origin_size((rect.x0, y), INDICATOR),
            INDICATOR_RADIUS,
        ),
        t.accent_fill_default,
    );
}

pub(crate) fn draw_row(
    drawer: &FluentDrawer,
    canvas: &mut dyn Canvas,
    rect: Rect,
    name: &str,
    value: &str,
    visual: PropertyGridRowVisual,
) {
    let t = &drawer.tokens;
    let (background, foreground) = colors(t, visual.selected, visual.state);
    if background != Color::TRANSPARENT {
        canvas.fill(&rect, background);
    }
    if visual.selected {
        indicator(canvas, rect, t);
    }
    let foreground = if visual.disabled {
        t.text_fill_disabled
    } else {
        foreground
    };
    // As linhas: embaixo e entre o nome e o valor.
    let line = t.divider_stroke_default;
    canvas.fill(
        &Rect::new(rect.x0, rect.y1 - metrics::BORDER, rect.x1, rect.y1),
        line,
    );
    let x = visual.divider_x(rect);
    canvas.fill(&Rect::new(x, rect.y0, x + 1.0, rect.y1), line);
    if !name.is_empty() {
        let style = TextStyle::BODY;
        canvas.push_clip(&name_clip(rect, visual).to_path(PATH_TOLERANCE));
        drawer.fonts.borrow_mut().layout(name, style, None).draw(
            canvas,
            Point::new(visual.name_x(rect), text_y(rect, style)),
            &Paint::Solid(foreground),
        );
        canvas.pop_clip();
    }
    if !visual.editing && !value.is_empty() {
        let style = row_style(visual);
        canvas.push_clip(&value_clip(rect, visual).to_path(PATH_TOLERANCE));
        drawer.fonts.borrow_mut().layout(value, style, None).draw(
            canvas,
            Point::new(value_x(rect, visual), text_y(rect, style)),
            &Paint::Solid(value_color(t, visual, foreground)),
        );
        canvas.pop_clip();
    }
    if visual.focus_ring {
        crate::draw_focus_ring(canvas, rect, 0.0, t);
    }
}

pub(crate) fn row_info(
    drawer: &FluentDrawer,
    rect: Rect,
    name: &str,
    value: &str,
    visual: PropertyGridRowVisual,
) -> Vec<TextInfo> {
    let t = &drawer.tokens;
    let (fill, foreground) = colors(t, visual.selected, visual.state);
    let foreground = if visual.disabled {
        t.text_fill_disabled
    } else {
        foreground
    };
    let background = crate::over(fill, drawer.backdrop());
    let mut infos = Vec::new();
    if !name.is_empty() {
        infos.push(crate::text::line_info(
            drawer,
            name,
            TextStyle::BODY,
            Point::new(visual.name_x(rect), text_y(rect, TextStyle::BODY)),
            (foreground, background),
            name_clip(rect, visual),
        ));
    }
    if !visual.editing && !value.is_empty() {
        let style = row_style(visual);
        infos.push(crate::text::line_info(
            drawer,
            value,
            style,
            Point::new(value_x(rect, visual), text_y(rect, style)),
            (value_color(t, visual, foreground), background),
            value_clip(rect, visual),
        ));
    }
    infos
}

const CATEGORY_STYLE: TextStyle = TextStyle::BODY_STRONG;

fn category_clip(rect: Rect, visual: PropertyGridCategoryVisual) -> Rect {
    let x0 = rect.x0 + tomni_ui_core::PROPERTY_GRID_NAME_X;
    Rect::new(
        x0,
        rect.y0,
        (visual.chevron_square(rect).x0 - TEXT_END).max(x0),
        rect.y1,
    )
}

pub(crate) fn draw_category(
    drawer: &FluentDrawer,
    canvas: &mut dyn Canvas,
    rect: Rect,
    label: &str,
    visual: PropertyGridCategoryVisual,
) {
    let t = &drawer.tokens;
    let (background, foreground) = colors(t, visual.selected, visual.state);
    if background != Color::TRANSPARENT {
        canvas.fill(&rect, background);
    }
    if visual.selected {
        indicator(canvas, rect, t);
    }
    canvas.fill(
        &Rect::new(rect.x0, rect.y1 - metrics::BORDER, rect.x1, rect.y1),
        t.divider_stroke_default,
    );
    if !label.is_empty() {
        canvas.push_clip(&category_clip(rect, visual).to_path(PATH_TOLERANCE));
        drawer
            .fonts
            .borrow_mut()
            .layout(label, CATEGORY_STYLE, None)
            .draw(
                canvas,
                Point::new(
                    rect.x0 + tomni_ui_core::PROPERTY_GRID_NAME_X,
                    text_y(rect, CATEGORY_STYLE),
                ),
                &Paint::Solid(foreground),
            );
        canvas.pop_clip();
    }
    // O chevron do `Expander`: para cima com a categoria aberta.
    let square = visual.chevron_square(rect);
    let (hover, glyph) = expander::chevron_colors(t, visual.chevron_state);
    if hover != Color::TRANSPARENT {
        let side = square.height().min(square.width());
        let button = Rect::from_center_size(square.center(), (side - 4.0, side - 4.0));
        canvas.fill(
            &RoundedRect::from_rect(button, metrics::CONTROL_CORNER_RADIUS),
            hover,
        );
    }
    let glyph_square = icon::centered(square, CHEVRON_GLYPH);
    let mut path = icons::CHEVRON_DOWN_12.path(glyph_square.origin(), CHEVRON_GLYPH);
    if visual.expanded {
        let cy = glyph_square.center().y;
        path.apply_affine(Affine::translate((0.0, 2.0 * cy)) * Affine::FLIP_Y);
    }
    canvas.fill_path(&path, &Paint::Solid(glyph));
    if visual.focus_ring {
        crate::draw_focus_ring(canvas, rect, 0.0, t);
    }
}

pub(crate) fn category_info(
    drawer: &FluentDrawer,
    rect: Rect,
    label: &str,
    visual: PropertyGridCategoryVisual,
) -> Vec<TextInfo> {
    if label.is_empty() {
        return Vec::new();
    }
    let t = &drawer.tokens;
    let (fill, foreground) = colors(t, visual.selected, visual.state);
    let background = crate::over(fill, drawer.backdrop());
    vec![crate::text::line_info(
        drawer,
        label,
        CATEGORY_STYLE,
        Point::new(
            rect.x0 + tomni_ui_core::PROPERTY_GRID_NAME_X,
            text_y(rect, CATEGORY_STYLE),
        ),
        (foreground, background),
        category_clip(rect, visual),
    )]
}

pub(crate) fn measure_sizer() -> Size {
    Size::new(0.0, SIZER_HEIGHT)
}

fn sizer_fill(t: &Tokens, visual: PropertyGridSizerVisual) -> Color {
    match visual.state {
        ButtonState::Hover => t.control_alt_fill_tertiary,
        ButtonState::Pressed => t.control_alt_fill_quarternary,
        ButtonState::Rest | ButtonState::Disabled => Color::TRANSPARENT,
    }
}

/// A linha do meio do divisor.
fn sizer_line(rect: Rect) -> Rect {
    let y = rect.center().y.floor();
    Rect::new(rect.x0, y, rect.x1, y + metrics::BORDER)
}

pub(crate) fn draw_sizer(
    drawer: &FluentDrawer,
    canvas: &mut dyn Canvas,
    rect: Rect,
    visual: PropertyGridSizerVisual,
) {
    let t = &drawer.tokens;
    let fill = sizer_fill(t, visual);
    if fill != Color::TRANSPARENT {
        canvas.fill(&rect, fill);
    }
    canvas.fill(&sizer_line(rect), t.divider_stroke_default);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;

    #[test]
    fn valor_alterado_usa_peso_600() {
        let visual = PropertyGridRowVisual {
            changed: true,
            ..PropertyGridRowVisual::default()
        };
        assert_eq!(row_style(visual).weight, 600.0);
        assert_eq!(row_style(PropertyGridRowVisual::default()).weight, 400.0);
    }

    #[test]
    fn valor_so_leitura_e_desabilitado_mudam_de_cor() {
        let t = Tokens::new(Theme::Light);
        let base = t.text_fill_primary;
        let mut v = PropertyGridRowVisual::default();
        assert_eq!(value_color(&t, v, base), base);
        v.read_only = true;
        assert_eq!(value_color(&t, v, base), t.text_fill_secondary);
        v.disabled = true;
        assert_eq!(value_color(&t, v, base), t.text_fill_disabled);
    }

    #[test]
    fn valor_comeca_onde_o_texto_do_edit_comeca() {
        let rect = Rect::new(0.0, 0.0, 300.0, 24.0);
        let v = PropertyGridRowVisual {
            name_width: 150.0,
            density: Density::Compact,
            ..PropertyGridRowVisual::default()
        };
        assert_eq!(value_x(rect, v), 150.0 + 1.0 + 1.0 + 2.0);
    }
}
