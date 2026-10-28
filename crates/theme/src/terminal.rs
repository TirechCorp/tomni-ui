//! O visual do `Terminal`: a grade com a Cascadia Mono, o cursor, a
//! seleção e a barra de rolagem do WinUI.
//!
//! Cada linha da grade é gravada uma vez e reproduzida enquanto não muda: quando chega saída
//! nova, só as linhas alteradas são desenhadas de novo (a "repintura por linha" do M17).

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::time::Instant;

use tomni_ui_core::Color as Rgba;
use tomni_ui_render::kurbo::{Rect, RoundedRect, Shape, Stroke, Vec2};
use tomni_ui_render::peniko::Color;
use tomni_ui_render::{Canvas, CanvasExt, PATH_TOLERANCE, Paint, Recording};
use tomni_ui_terminal::cell::{Cell, Flags};
use tomni_ui_terminal::grid::Row;
use tomni_ui_terminal::{
    CursorShape, PANE_BORDER, PANE_DASH, Palette, PaneBorder, PaneVisual, TerminalVisual,
    char_width, grid_origin,
};
use tomni_ui_text::mono::{CellMetrics, CellStyle, CellText};
use tomni_ui_text::{Fonts, TextStyle};

use crate::{TextInfo, Theme, Tokens, draw_focus_ring, metrics, over, scroll_viewer};

/// Linhas gravadas guardadas antes de começar uma geração nova.
const CACHE_LINES: usize = 2048;
/// Opacidade do lampejo da campainha, por cima de tudo, na cor do texto.
const FLASH_ALPHA: u8 = 0x40;

/// Linhas já gravadas, pela aparência (o conteúdo, as cores e o tamanho da fonte). Duas
/// gerações: o que não aparece há uma geração inteira sai.
#[derive(Debug, Default)]
pub(crate) struct Cache {
    current: HashMap<u64, Rc<Recording>>,
    previous: HashMap<u64, Rc<Recording>>,
}

impl Cache {
    fn get_or_record(&mut self, key: u64, record: impl FnOnce() -> Recording) -> Rc<Recording> {
        if let Some(found) = self.current.get(&key) {
            return Rc::clone(found);
        }
        let found = self
            .previous
            .remove(&key)
            .unwrap_or_else(|| Rc::new(record()));
        if self.current.len() >= CACHE_LINES {
            self.previous = std::mem::take(&mut self.current);
        }
        self.current.insert(key, Rc::clone(&found));
        found
    }
}

fn color(c: Rgba) -> Color {
    Color::from_rgba8(c.r, c.g, c.b, c.a)
}

fn rgba(c: Color) -> Rgba {
    let c = c.to_rgba8();
    Rgba::rgba(c.r, c.g, c.b, c.a)
}

/// As cores do terminal no tema de `tokens`.
pub(crate) fn palette(visual: &TerminalVisual, tokens: &Tokens) -> Palette {
    visual.scheme().palette(
        tokens.theme == Theme::Dark,
        rgba(tokens.solid_background_base),
        rgba(tokens.text_fill_primary),
    )
}

/// Luminância relativa (WCAG) de uma cor opaca.
fn luminance(c: Rgba) -> f64 {
    let channel = |v: u8| {
        let v = f64::from(v) / 255.0;
        if v <= 0.040_45 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(c.r) + 0.7152 * channel(c.g) + 0.0722 * channel(c.b)
}

/// Quantas células a célula ocupa: 2 no começo de um largo.
fn span(cell: &Cell) -> usize {
    if cell.flags.contains(Flags::WIDE) {
        2
    } else {
        1
    }
}

/// Cor e estilo de um grupo de caracteres.
type Ink = (Rgba, CellStyle);
/// Um caractere: a coluna, as células que ocupa e o texto.
type Glyph = (u16, u8, String);

/// Grava a linha com o canto superior esquerdo na origem.
fn record_row(
    fonts: &mut Fonts,
    row: &Row,
    palette: &Palette,
    reverse: bool,
    m: &CellMetrics,
) -> Recording {
    let (cw, ch) = (f64::from(m.width), f64::from(m.height));
    let mut rec = Recording::new(
        (row.cells.len() as f64 * cw).ceil() as u32,
        ch.ceil() as u32,
    );
    let resolved: Vec<_> = row
        .cells
        .iter()
        .map(|cell| palette.resolve(cell, reverse))
        .collect();

    // Fundos, juntando as células vizinhas da mesma cor.
    let mut col = 0;
    while col < resolved.len() {
        let Some(bg) = resolved[col].bg else {
            col += 1;
            continue;
        };
        let start = col;
        while col < resolved.len() && resolved[col].bg == Some(bg) {
            col += 1;
        }
        rec.fill_rect(
            Rect::new(start as f64 * cw, 0.0, col as f64 * cw, ch),
            &Paint::Solid(color(bg)),
        );
    }

    // Texto, em grupos da mesma cor e estilo.
    let mut groups: Vec<(Ink, Vec<Glyph>)> = Vec::new();
    for (col, (cell, colors)) in row.cells.iter().zip(&resolved).enumerate() {
        let flags = cell.flags;
        if flags.intersects(Flags::WIDE_SPACER | Flags::WIDE_PAD | Flags::HIDDEN)
            || (cell.c == ' ' && cell.combining.is_none())
        {
            continue;
        }
        let style = CellStyle {
            bold: flags.contains(Flags::BOLD),
            italic: flags.contains(Flags::ITALIC),
        };
        let key = (colors.fg, style);
        let item = (col as u16, span(cell) as u8, cell.text());
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, items)) => items.push(item),
            None => groups.push((key, vec![item])),
        }
    }
    for ((fg, style), items) in &groups {
        let cells: Vec<CellText<'_>> = items
            .iter()
            .map(|(col, width, text)| CellText {
                col: *col,
                width: *width,
                text,
            })
            .collect();
        let paint = Paint::Solid(color(*fg));
        for run in fonts.shape_cells(&cells, *style, m, Default::default()) {
            rec.draw_glyphs(&run.as_run(), &paint);
        }
    }

    // Sublinhado, sublinhado duplo e tachado, na cor do texto.
    let thickness = f64::from(m.thickness);
    for (col, (cell, colors)) in row.cells.iter().zip(&resolved).enumerate() {
        let flags = cell.flags;
        if flags.contains(Flags::HIDDEN) || flags.contains(Flags::WIDE_SPACER) {
            continue;
        }
        let x0 = col as f64 * cw;
        let x1 = x0 + span(cell) as f64 * cw;
        let paint = Paint::Solid(color(colors.fg));
        let mut line = |y: f64| rec.fill_rect(Rect::new(x0, y, x1, y + thickness), &paint);
        let underline = f64::from(m.underline);
        if flags.contains(Flags::DOUBLE_UNDERLINE) {
            line(underline);
            line((underline - 2.0 * thickness).max(0.0));
        } else if flags.contains(Flags::UNDERLINE) {
            line(underline);
        }
        if flags.contains(Flags::STRIKE) {
            line(f64::from(m.strikeout));
        }
    }
    rec
}

/// A chave da linha no [`Cache`].
fn row_key(row: &Row, palette: &Palette, reverse: bool, m: &CellMetrics) -> u64 {
    let mut hasher = DefaultHasher::new();
    row.hash(&mut hasher);
    palette.hash(&mut hasher);
    reverse.hash(&mut hasher);
    m.font_size.to_bits().hash(&mut hasher);
    hasher.finish()
}

/// Tamanho natural.
pub(crate) fn measure(visual: &TerminalVisual) -> tomni_ui_render::kurbo::Size {
    visual.natural_size()
}

/// Pinta o terminal em `rect` e soma o tempo ao da sessão ([`TerminalVisual::add_busy`]).
pub(crate) fn draw(
    canvas: &mut dyn Canvas,
    rect: Rect,
    visual: &TerminalVisual,
    tokens: &Tokens,
    fonts: &mut Fonts,
    cache: &mut Cache,
) {
    let start = Instant::now();
    paint(canvas, rect, visual, tokens, fonts, cache);
    visual.add_busy(start.elapsed());
}

fn paint(
    canvas: &mut dyn Canvas,
    rect: Rect,
    visual: &TerminalVisual,
    tokens: &Tokens,
    fonts: &mut Fonts,
    cache: &mut Cache,
) {
    let palette = palette(visual, tokens);
    let emu = visual.emulator();
    let modes = emu.modes();
    let reverse = modes.reverse_video;
    let m = *visual.metrics();
    let (cw, ch) = (f64::from(m.width), f64::from(m.height));
    let screen = palette.screen_background(reverse);
    let radius = metrics::CONTROL_CORNER_RADIUS;
    canvas.fill(&RoundedRect::from_rect(rect, radius), color(screen));

    // A grade fica dentro da borda, sem invadir a barra de rolagem.
    let origin = grid_origin(rect);
    let grid = Rect::new(
        origin.x,
        origin.y,
        (origin.x + emu.cols() as f64 * cw).min(rect.x1 - tomni_ui_core::SCROLL_BAR_SIZE),
        (origin.y + emu.rows() as f64 * ch).min(rect.y1),
    );
    canvas.push_clip(&grid.to_path(PATH_TOLERANCE));
    for r in 0..emu.rows() {
        let row = emu.view_row(r);
        let key = row_key(row, &palette, reverse, &m);
        let rec = cache.get_or_record(key, || record_row(fonts, row, &palette, reverse, &m));
        rec.replay_at(canvas, Vec2::new(origin.x, origin.y + r as f64 * ch));
    }

    // Seleção, translúcida por cima do texto.
    if let Some(selection) = visual.selection() {
        let paint = Paint::Solid(color(palette.selection));
        for r in 0..emu.rows() {
            if let Some((from, to)) = selection.columns(&emu, emu.view_line(r)) {
                canvas.fill_rect(visual.cell_rect(rect, r, from, to - from), &paint);
            }
        }
    }

    // O link sob o ponteiro: sublinhado contínuo com o Ctrl, pontilhado de 1 px sem ele, na
    // cor do texto de cada célula.
    if let Some((cells, solid)) = visual.link() {
        let underline = f64::from(m.underline);
        let thickness = f64::from(m.thickness);
        for &(r, col) in cells {
            let Some(cell) = emu.view_row(r).cells.get(col) else {
                continue;
            };
            if cell.flags.contains(Flags::WIDE_SPACER) {
                continue;
            }
            let area = visual.cell_rect(rect, r, col, span(cell));
            let y = area.y0 + underline;
            let paint = Paint::Solid(color(palette.resolve(cell, reverse).fg));
            if solid {
                canvas.fill_rect(Rect::new(area.x0, y, area.x1, y + thickness), &paint);
            } else {
                let mut x = area.x0;
                while x < area.x1 {
                    canvas.fill_rect(Rect::new(x, y, (x + 1.0).min(area.x1), y + 1.0), &paint);
                    x += 2.0;
                }
            }
        }
    }

    // O texto do IME, sublinhado no lugar do cursor, e o cursor no fim dele.
    let mut cursor = visual.cursor();
    if let (Some(preedit), Some((row, col, shape))) = (visual.preedit(), emu_cursor(visual)) {
        let mut cells = Vec::new();
        let mut at = col;
        let texts: Vec<(usize, usize, String)> = preedit
            .chars()
            .filter_map(|c| {
                let w = char_width(c);
                (w > 0).then(|| {
                    let item = (at, w, c.to_string());
                    at += w;
                    item
                })
            })
            .collect();
        for (col, w, text) in &texts {
            cells.push(CellText {
                col: *col as u16,
                width: *w as u8,
                text,
            });
        }
        let area = visual.cell_rect(rect, row, col, at - col);
        canvas.fill_rect(area, &Paint::Solid(color(screen)));
        let fg = Paint::Solid(color(palette.foreground));
        let row_origin = visual.cell_rect(rect, row, 0, 1).origin();
        for run in fonts.shape_cells(&cells, CellStyle::default(), &m, row_origin) {
            canvas.draw_glyphs(&run.as_run(), &fg);
        }
        let y = area.y0 + f64::from(m.underline);
        canvas.fill_rect(
            Rect::new(area.x0, y, area.x1, y + f64::from(m.thickness)),
            &fg,
        );
        cursor = Some((row, at.min(emu.cols().saturating_sub(1)), shape));
    }
    if let Some((row, col, shape)) = cursor {
        let cell = emu.view_row(row).cells.get(col);
        let cells = cell.map_or(1, span);
        let cell_rect = visual.cell_rect(rect, row, col, cells);
        let paint = Paint::Solid(color(palette.cursor));
        let thin = (cw / 8.0).round().max(1.0);
        if !visual.focused() || visual.closed() {
            // Sem foco, ou com a sessão encerrada, só o contorno da célula.
            canvas.stroke(&cell_rect.inset(-0.5), &Stroke::new(1.0), paint);
        } else {
            match shape {
                CursorShape::Bar => canvas.fill_rect(
                    Rect::new(
                        cell_rect.x0,
                        cell_rect.y0,
                        cell_rect.x0 + thin,
                        cell_rect.y1,
                    ),
                    &paint,
                ),
                CursorShape::Underline => canvas.fill_rect(
                    Rect::new(
                        cell_rect.x0,
                        cell_rect.y1 - 2.0 * thin,
                        cell_rect.x1,
                        cell_rect.y1,
                    ),
                    &paint,
                ),
                CursorShape::Block => {
                    canvas.fill_rect(cell_rect, &paint);
                    // O caractere embaixo aparece na cor do fundo.
                    if let Some(cell) = cell.filter(|c| c.c != ' ' || c.combining.is_some()) {
                        let text = cell.text();
                        let cells = [CellText {
                            col: col as u16,
                            width: span(cell) as u8,
                            text: &text,
                        }];
                        let style = CellStyle {
                            bold: cell.flags.contains(Flags::BOLD),
                            italic: cell.flags.contains(Flags::ITALIC),
                        };
                        let row_origin = visual.cell_rect(rect, row, 0, 1).origin();
                        let ink = Paint::Solid(color(screen));
                        for run in fonts.shape_cells(&cells, style, &m, row_origin) {
                            canvas.draw_glyphs(&run.as_run(), &ink);
                        }
                    }
                }
            }
        }
    }
    canvas.pop_clip();

    if visual.flash() {
        let f = palette.foreground;
        canvas.fill(
            &RoundedRect::from_rect(rect, radius),
            color(Rgba::rgba(f.r, f.g, f.b, FLASH_ALPHA)),
        );
    }

    // A barra do WinUI, nas cores do tema que combina com o fundo do terminal: o Campbell é
    // escuro também no tema claro.
    if let Some(parts) = visual
        .scroller(rect)
        .bar(rect, tomni_ui_core::ScrollAxis::Y)
    {
        let theme = if luminance(screen) < 0.5 {
            Theme::Dark
        } else {
            Theme::Light
        };
        let bar_tokens;
        let t = if theme == tokens.theme {
            tokens
        } else {
            bar_tokens = Tokens::new(theme);
            &bar_tokens
        };
        scroll_viewer::draw_bar(canvas, &parts, visual.bar_state().expanded(), t);
    }

    if visual.focus_ring() {
        draw_focus_ring(canvas, rect, radius, tokens);
    }
}

/// O cursor do emulador na vista, mesmo na fase apagada do pisca (para o texto do IME).
fn emu_cursor(visual: &TerminalVisual) -> Option<(usize, usize, CursorShape)> {
    let emu = visual.emulator();
    let (row, col) = emu.cursor();
    let row = row + emu.display_offset();
    (row < emu.rows()).then_some((row, col, emu.cursor_shape()))
}

/// O texto para o lint: a cor padrão do esquema sobre o fundo padrão (o resto das cores é do
/// programa que roda no terminal).
pub(crate) fn text_info(
    rect: Rect,
    visual: &TerminalVisual,
    tokens: &Tokens,
    backdrop: Color,
) -> TextInfo {
    let palette = palette(visual, tokens);
    let background = over(color(palette.background), backdrop);
    let m = visual.metrics();
    TextInfo {
        text: format!("texto padrão do esquema {}", visual.scheme().name()),
        color: over(color(palette.foreground), background),
        background,
        style: TextStyle {
            size: m.font_size,
            line_height: m.height,
            weight: 400.0,
        },
        bounds: visual.cell_rect(rect, 0, 0, visual.emulator().cols()),
        clip: None,
    }
}

/// A célula do cursor, para a janela de candidatos do IME.
pub(crate) fn caret_rect(rect: Rect, visual: &TerminalVisual) -> Option<Rect> {
    let (row, col, _) = emu_cursor(visual)?;
    Some(visual.cell_rect(rect, row, col, 1))
}

/// O quadro de um painel do `TerminalPanes`: a borda de 1 px na cor de destaque, por dentro do
/// retângulo. A tracejada é desenhada com retângulos (traço e vão de `PANE_DASH`), para sair
/// igual nos dois backends.
pub(crate) fn draw_pane(canvas: &mut dyn Canvas, rect: Rect, visual: &PaneVisual, tokens: &Tokens) {
    let paint = tokens.accent_fill_default;
    let t = PANE_BORDER;
    match visual.border {
        PaneBorder::None => {}
        PaneBorder::Solid => {
            canvas.fill(&Rect::new(rect.x0, rect.y0, rect.x1, rect.y0 + t), paint);
            canvas.fill(&Rect::new(rect.x0, rect.y1 - t, rect.x1, rect.y1), paint);
            canvas.fill(
                &Rect::new(rect.x0, rect.y0 + t, rect.x0 + t, rect.y1 - t),
                paint,
            );
            canvas.fill(
                &Rect::new(rect.x1 - t, rect.y0 + t, rect.x1, rect.y1 - t),
                paint,
            );
        }
        PaneBorder::Dashed => {
            let step = PANE_DASH * 2.0;
            let mut x = rect.x0;
            while x < rect.x1 {
                let end = (x + PANE_DASH).min(rect.x1);
                canvas.fill(&Rect::new(x, rect.y0, end, rect.y0 + t), paint);
                canvas.fill(&Rect::new(x, rect.y1 - t, end, rect.y1), paint);
                x += step;
            }
            let mut y = rect.y0 + t;
            while y < rect.y1 - t {
                let end = (y + PANE_DASH).min(rect.y1 - t);
                canvas.fill(&Rect::new(rect.x0, y, rect.x0 + t, end), paint);
                canvas.fill(&Rect::new(rect.x1 - t, y, rect.x1, end), paint);
                y += step;
            }
        }
    }
}
