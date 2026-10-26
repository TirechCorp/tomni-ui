//! `DataGrid` no visual do WinUI: o cabeçalho com o glifo de ordenação e os divisores, as
//! linhas com a pílula de seleção e as células de texto e de andamento.

use std::f64::consts::PI;

use tomni_ui_core::{
    ButtonState, CellValue, DATA_GRID_CELL_PADDING, DataGridHeaderVisual, DataGridRowVisual,
    DataGridVisual, ProgressVisual, RowTone, SortDirection,
};
use tomni_ui_render::kurbo::{Affine, Point, Rect, RoundedRect, Shape, Size};
use tomni_ui_render::peniko::Color;
use tomni_ui_render::{Canvas, CanvasExt, PATH_TOLERANCE, Paint};
use tomni_ui_text::TextStyle;

use crate::{FluentDrawer, GraphicInfo, TextInfo, Tokens, icon, icons, metrics};

/// Fonte das células.
const STYLE: TextStyle = TextStyle::BODY;
/// Fonte do cabeçalho.
const HEADER_STYLE: TextStyle = TextStyle::BODY_STRONG;
/// Lado do glifo de ordenação.
const SORT_GLYPH: f64 = 12.0;
/// Espaço entre o glifo de ordenação e o texto.
const SORT_GAP: f64 = 6.0;
/// Indicador de selecionada, como o do `TreeViewItem`.
const INDICATOR: Size = Size::new(3.0, 16.0);
/// Raio do indicador.
const INDICATOR_RADIUS: f64 = 2.0;
/// Altura do traço que separa as colunas no cabeçalho.
const SEPARATOR_HEIGHT: f64 = 16.0;
/// Distância do texto de uma lista vazia ao topo da área das linhas.
const EMPTY_TOP: f64 = 16.0;

/// Fundo e cor do texto de uma linha, como no WinUI (`ListViewItem`).
fn row_colors(t: &Tokens, selected: bool, state: ButtonState) -> (Color, Color) {
    match state {
        ButtonState::Pressed if selected => (t.subtle_fill_secondary, t.text_fill_secondary),
        ButtonState::Hover if selected => (t.subtle_fill_tertiary, t.text_fill_primary),
        ButtonState::Pressed => (t.subtle_fill_tertiary, t.text_fill_secondary),
        ButtonState::Hover => (t.subtle_fill_secondary, t.text_fill_primary),
        ButtonState::Disabled => (Color::TRANSPARENT, t.text_fill_disabled),
        ButtonState::Rest if selected => (t.subtle_fill_secondary, t.text_fill_primary),
        ButtonState::Rest => (Color::TRANSPARENT, t.text_fill_primary),
    }
}

/// O fundo de um tom: os tokens `SystemFillColor…Background` do WinUI.
fn tone_fill(t: &Tokens, tone: RowTone) -> Color {
    match tone {
        RowTone::Success => t.info_bar_success_background,
        RowTone::Caution => t.info_bar_warning_background,
        RowTone::Critical => t.info_bar_error_background,
    }
}

/// Fundo a pintar: o realce de soltar dentro da pasta vale mais que o estado, e o estado
/// (seleção, hover) vai por cima do tom, para a seleção continuar visível.
fn row_fill(t: &Tokens, visual: &DataGridRowVisual) -> Color {
    if visual.drop {
        return t.subtle_fill_secondary;
    }
    let state = row_colors(t, visual.selected, visual.state).0;
    match visual.row.tone {
        Some(tone) => crate::over(state, tone_fill(t, tone)),
        None => state,
    }
}

/// Onde o texto da célula `index` da linha cabe.
fn row_text_area(rect: Rect, visual: &DataGridRowVisual, index: usize) -> Option<Rect> {
    let cells = visual.cells(rect);
    let cell = cells.get(index)?;
    let pill = visual.pill(rect);
    let x0 = if index == 0 {
        visual.label_x(rect)
    } else {
        cell.x0 + DATA_GRID_CELL_PADDING
    };
    let x1 = cell.x1 - DATA_GRID_CELL_PADDING;
    (x1 > x0).then(|| Rect::new(x0, pill.y0, x1, pill.y1))
}

fn text_y(area: Rect, style: TextStyle) -> f64 {
    area.y0 + ((area.height() - f64::from(style.line_height)) / 2.0).floor()
}

/// O texto de uma célula, com reticências se não cabe: a largura dele e o x onde começa.
fn place(
    drawer: &FluentDrawer,
    text: &str,
    style: TextStyle,
    area: Rect,
    align_end: bool,
) -> (tomni_ui_text::TextLayout, f64) {
    let (layout, _) =
        drawer
            .fonts
            .borrow_mut()
            .layout_lines(text, style, Some(area.width() as f32), 1);
    let width = f64::from(layout.width());
    let x = if align_end {
        (area.x1 - width).max(area.x0)
    } else {
        area.x0
    };
    (layout, x)
}

pub(crate) fn draw_row(
    drawer: &FluentDrawer,
    canvas: &mut dyn Canvas,
    rect: Rect,
    visual: &DataGridRowVisual,
) {
    let t = &drawer.tokens;
    let pill = visual.pill(rect);
    let radius = metrics::CONTROL_CORNER_RADIUS;
    let (_, foreground) = row_colors(t, visual.selected, visual.state);
    let background = row_fill(t, visual);
    if background != Color::TRANSPARENT {
        canvas.fill(&RoundedRect::from_rect(pill, radius), background);
    }
    if visual.drop {
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
    if let Some(symbol) = visual.row.icon {
        icon::draw_symbol(canvas, symbol, visual.icon_square(rect), foreground);
    }
    for (index, column) in visual.columns.iter().enumerate() {
        let Some((text, value)) = visual.row.cells.get(index) else {
            continue;
        };
        if index == 0 && visual.renaming {
            continue;
        }
        let Some(area) = row_text_area(rect, visual, index) else {
            continue;
        };
        if let CellValue::Progress(fraction) = value {
            crate::progress::draw_bar(
                canvas,
                area,
                ProgressVisual {
                    value: Some(*fraction),
                    phase: 0.0,
                    ..Default::default()
                },
                t,
            );
            continue;
        }
        if text.is_empty() {
            continue;
        }
        let (layout, x) = place(drawer, text, STYLE, area, column.align_end);
        canvas.push_clip(&area.to_path(PATH_TOLERANCE));
        layout.draw(
            canvas,
            Point::new(x, text_y(area, STYLE)),
            &Paint::Solid(foreground),
        );
        canvas.pop_clip();
    }
    if visual.focus_ring {
        crate::draw_focus_ring(canvas, pill, radius, t);
    }
}

pub(crate) fn row_info(
    drawer: &FluentDrawer,
    rect: Rect,
    visual: &DataGridRowVisual,
) -> Vec<TextInfo> {
    let (_, foreground) = row_colors(&drawer.tokens, visual.selected, visual.state);
    let background = crate::over(row_fill(&drawer.tokens, visual), drawer.backdrop());
    let mut infos = Vec::new();
    for (index, column) in visual.columns.iter().enumerate() {
        let Some((text, value)) = visual.row.cells.get(index) else {
            continue;
        };
        if (index == 0 && visual.renaming)
            || text.is_empty()
            || matches!(value, CellValue::Progress(_))
        {
            continue;
        }
        let Some(area) = row_text_area(rect, visual, index) else {
            continue;
        };
        let (layout, x) = place(drawer, text, STYLE, area, column.align_end);
        let width = f64::from(layout.width());
        let y = text_y(area, STYLE);
        infos.push(TextInfo {
            text: text.clone(),
            color: crate::over(foreground, background),
            background,
            style: STYLE,
            bounds: Rect::new(x, y, x + width, y + f64::from(STYLE.line_height)),
            clip: Some(area),
        });
    }
    infos
}

pub(crate) fn row_graphic(
    drawer: &FluentDrawer,
    rect: Rect,
    visual: &DataGridRowVisual,
) -> Vec<GraphicInfo> {
    let Some(symbol) = visual.row.icon else {
        return Vec::new();
    };
    let (_, foreground) = row_colors(&drawer.tokens, visual.selected, visual.state);
    let background = crate::over(row_fill(&drawer.tokens, visual), drawer.backdrop());
    vec![GraphicInfo {
        name: symbol.name(),
        color: crate::over(foreground, background),
        background,
        bounds: visual.icon_square(rect),
    }]
}

/// Onde o texto do cabeçalho da coluna `index` cabe: a célula menos o espaço, de um lado, do
/// glifo de ordenação (à direita; nas colunas à direita, à esquerda).
fn header_text_area(
    rect: Rect,
    visual: &DataGridHeaderVisual,
    index: usize,
) -> Option<(Rect, Option<Rect>)> {
    let cells = visual.cells(rect);
    let cell = cells.get(index)?;
    let column = visual.columns.get(index)?;
    let (mut x0, mut x1) = (
        cell.x0 + DATA_GRID_CELL_PADDING,
        cell.x1 - DATA_GRID_CELL_PADDING,
    );
    let sorted = matches!(visual.sort, Some((i, _)) if i == index);
    let mut glyph = None;
    if sorted {
        let y = cell.y0 + ((cell.height() - SORT_GLYPH) / 2.0).round();
        if column.align_end {
            glyph = Some(Rect::new(x0, y, x0 + SORT_GLYPH, y + SORT_GLYPH));
            x0 += SORT_GLYPH + SORT_GAP;
        } else {
            glyph = Some(Rect::new(x1 - SORT_GLYPH, y, x1, y + SORT_GLYPH));
            x1 -= SORT_GLYPH + SORT_GAP;
        }
    }
    (x1 > x0).then(|| (Rect::new(x0, cell.y0, x1, cell.y1), glyph))
}

fn header_fill(t: &Tokens, visual: &DataGridHeaderVisual, index: usize) -> Color {
    if visual.pressed == Some(index) {
        t.subtle_fill_tertiary
    } else if visual.hovered == Some(index) && visual.resizing.is_none() {
        t.subtle_fill_secondary
    } else {
        Color::TRANSPARENT
    }
}

pub(crate) fn draw_header(
    drawer: &FluentDrawer,
    canvas: &mut dyn Canvas,
    rect: Rect,
    visual: &DataGridHeaderVisual,
) {
    let t = &drawer.tokens;
    let radius = metrics::CONTROL_CORNER_RADIUS;
    let cells = visual.cells(rect);
    for (index, cell) in cells.iter().enumerate() {
        let fill = header_fill(t, visual, index);
        if fill != Color::TRANSPARENT {
            canvas.fill(&RoundedRect::from_rect(*cell, radius), fill);
        }
    }
    // Os traços entre as colunas; o divisor sob o ponteiro ou em arrasto ganha destaque.
    for (index, cell) in cells.iter().enumerate().take(cells.len().saturating_sub(1)) {
        let active = visual.divider_hover == Some(index) || visual.resizing == Some(index);
        let color = if active {
            t.control_strong_stroke_default
        } else {
            t.divider_stroke_default
        };
        let y = cell.y0 + ((cell.height() - SEPARATOR_HEIGHT) / 2.0).round();
        let thickness = if active { 2.0 } else { metrics::BORDER };
        canvas.fill(
            &Rect::new(
                cell.x1 - thickness / 2.0,
                y,
                cell.x1 + thickness / 2.0,
                y + SEPARATOR_HEIGHT,
            ),
            color,
        );
    }
    // A linha embaixo do cabeçalho.
    canvas.fill(
        &Rect::new(rect.x0, rect.y1 - metrics::BORDER, rect.x1, rect.y1),
        t.divider_stroke_default,
    );
    for (index, column) in visual.columns.iter().enumerate() {
        let Some((area, glyph)) = header_text_area(rect, visual, index) else {
            continue;
        };
        if !column.header.is_empty() {
            let (layout, x) = place(drawer, &column.header, HEADER_STYLE, area, column.align_end);
            canvas.push_clip(&area.to_path(PATH_TOLERANCE));
            layout.draw(
                canvas,
                Point::new(x, text_y(area, HEADER_STYLE)),
                &Paint::Solid(t.text_fill_primary),
            );
            canvas.pop_clip();
        }
        if let (Some(glyph), Some((_, direction))) = (glyph, visual.sort) {
            let mut path = icons::CHEVRON_DOWN_12.path(glyph.origin(), SORT_GLYPH);
            if direction == SortDirection::Ascending {
                // Para cima: a de baixo girada meia volta, no centro do glifo.
                path.apply_affine(Affine::rotate_about(PI, glyph.center()));
            }
            canvas.fill_path(&path, &Paint::Solid(t.text_fill_primary));
        }
    }
    if let Some(index) = visual.focus_ring
        && let Some(cell) = cells.get(index)
    {
        crate::draw_focus_ring(canvas, *cell, radius, t);
    }
}

pub(crate) fn header_info(
    drawer: &FluentDrawer,
    rect: Rect,
    visual: &DataGridHeaderVisual,
) -> Vec<TextInfo> {
    let t = &drawer.tokens;
    let mut infos = Vec::new();
    for (index, column) in visual.columns.iter().enumerate() {
        if column.header.is_empty() {
            continue;
        }
        let Some((area, _)) = header_text_area(rect, visual, index) else {
            continue;
        };
        let background = crate::over(header_fill(t, visual, index), drawer.backdrop());
        let (layout, x) = place(drawer, &column.header, HEADER_STYLE, area, column.align_end);
        let width = f64::from(layout.width());
        let y = text_y(area, HEADER_STYLE);
        infos.push(TextInfo {
            text: column.header.clone(),
            color: crate::over(t.text_fill_primary, background),
            background,
            style: HEADER_STYLE,
            bounds: Rect::new(x, y, x + width, y + f64::from(HEADER_STYLE.line_height)),
            clip: Some(area),
        });
    }
    infos
}

/// Onde o texto da lista vazia cabe.
fn empty_area(rect: Rect, visual: &DataGridVisual) -> Rect {
    let body = visual.body(rect);
    let top = body.y0 + EMPTY_TOP;
    Rect::new(
        body.x0 + DATA_GRID_CELL_PADDING,
        top,
        (body.x1 - DATA_GRID_CELL_PADDING).max(body.x0 + DATA_GRID_CELL_PADDING),
        top + f64::from(STYLE.line_height),
    )
}

/// O texto da lista vazia, centrado, se há.
fn empty_text(visual: &DataGridVisual) -> Option<&str> {
    (visual.rows.rows.is_empty() && !visual.empty_text.is_empty())
        .then_some(visual.empty_text.as_str())
}

pub(crate) fn draw(
    drawer: &FluentDrawer,
    canvas: &mut dyn Canvas,
    rect: Rect,
    visual: &DataGridVisual,
) {
    let t = &drawer.tokens;
    if let Some(text) = empty_text(visual) {
        let area = empty_area(rect, visual);
        if area.width() > 0.0 {
            let (layout, _) = place(drawer, text, STYLE, area, false);
            let x = area.x0 + ((area.width() - f64::from(layout.width())) / 2.0).floor();
            canvas.push_clip(&area.to_path(PATH_TOLERANCE));
            layout.draw(
                canvas,
                Point::new(x.max(area.x0), area.y0),
                &Paint::Solid(t.text_fill_secondary),
            );
            canvas.pop_clip();
        }
    }
    if visual.drop_background {
        // O contorno do arrasto que cai no fundo, por cima de tudo que o grid pintou antes
        // (as linhas são filhos e pintam depois, mas o contorno fica nas bordas).
        let border = 2.0;
        canvas.fill_path(
            &crate::ring(
                rect,
                metrics::CONTROL_CORNER_RADIUS,
                rect.inflate(-border, -border),
                (metrics::CONTROL_CORNER_RADIUS - border).max(0.0),
            ),
            &Paint::Solid(t.accent_fill_default),
        );
    }
}

pub(crate) fn info(drawer: &FluentDrawer, rect: Rect, visual: &DataGridVisual) -> Vec<TextInfo> {
    let Some(text) = empty_text(visual) else {
        return Vec::new();
    };
    let area = empty_area(rect, visual);
    if area.width() <= 0.0 {
        return Vec::new();
    }
    let (layout, _) = place(drawer, text, STYLE, area, false);
    let width = f64::from(layout.width());
    let x = area.x0 + ((area.width() - width) / 2.0).floor();
    let background = drawer.backdrop();
    vec![TextInfo {
        text: text.to_owned(),
        color: crate::over(drawer.tokens.text_fill_secondary, background),
        background,
        style: STYLE,
        bounds: Rect::new(
            x.max(area.x0),
            area.y0,
            x.max(area.x0) + width,
            area.y0 + f64::from(STYLE.line_height),
        ),
        clip: Some(area),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;

    #[test]
    fn cores_da_linha() {
        let t = Tokens::new(Theme::Light);
        assert_eq!(
            row_colors(&t, false, ButtonState::Rest).0,
            Color::TRANSPARENT
        );
        assert_eq!(
            row_colors(&t, true, ButtonState::Rest).0,
            t.subtle_fill_secondary
        );
        assert_eq!(
            row_colors(&t, true, ButtonState::Hover).0,
            t.subtle_fill_tertiary
        );
    }
}
