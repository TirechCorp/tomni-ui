//! `TextEditor`: o campo de várias linhas com rolagem, no visual do `TextBox` do WinUI.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use tomni_ui_core::{
    EditState, EditorPoints, EditorText, ScrollViewerVisual, StyleRange, TextEditorVisual,
};
use tomni_ui_render::kurbo::{Point, Rect, Shape, Size};
use tomni_ui_render::peniko::Color;
use tomni_ui_render::{Canvas, CanvasExt, PATH_TOLERANCE, Paint};
use tomni_ui_text::{Fonts, RunStyle, TextLayout, TextStyle};

use crate::edit::{draw_frame, fill, inner, multiline_area, placeholder_color};
use crate::{FluentDrawer, TextInfo, Tokens, metrics, scroll_viewer};

/// Quantos textos inteiros o cache guarda: o que aparece e o anterior bastam para digitar,
/// rolar e medir sem montar de novo a cada quadro.
const CACHE_ENTRIES: usize = 4;

/// Por quantas montagens um parágrafo que não aparece fica no cache: o bastante para dois
/// editores que se alternam não refazerem o shaping um do outro.
const PARAGRAPH_GENERATIONS: u64 = 16;

/// Sobra à direita de uma linha que termina em quebra de linha selecionada, como no WinUI.
const LINE_BREAK_TAIL: f64 = 4.0;

/// Fração do alfa do ícone de alerta que pinta o resultado atual de uma busca.
const CURRENT_MARK_ALPHA: u8 = 0x55;

/// O que muda o shaping de um parágrafo: o texto (sem o `\n`), as faixas cortadas nele (em
/// bytes do parágrafo, na ordem das faixas do texto), a largura da quebra e a fonte.
#[derive(Clone, PartialEq, Eq, Hash)]
struct ParagraphKey {
    text: Rc<str>,
    /// `(início, fim, negrito, itálico, riscado)`.
    ranges: Vec<(usize, usize, bool, bool, bool)>,
    wrap: Option<u32>,
    mono: bool,
}

/// Um parágrafo (o texto entre duas quebras de linha) com o layout e os pontos de cursor,
/// relativos ao começo dele.
struct Paragraph {
    /// `None` no parágrafo vazio, que não desenha nada.
    layout: Option<TextLayout>,
    /// `(byte, x, linha)`, do parágrafo.
    points: Vec<(usize, f64, usize)>,
    lines: usize,
    width: f64,
}

/// Um parágrafo no texto: onde ele começa e, para as cores, qual faixa do texto é cada faixa
/// do layout dele.
struct Placed {
    paragraph: Rc<Paragraph>,
    first_line: usize,
    /// A faixa `id` do layout do parágrafo é a `ranges[id - 1]` do texto.
    ranges: Vec<u32>,
}

/// O layout de um texto do editor, montado parágrafo a parágrafo: mudar um parágrafo refaz o
/// shaping só dele, e desenhar percorre só os que aparecem.
pub(crate) struct EditorLayout {
    paragraphs: Vec<Placed>,
}

impl EditorLayout {
    /// Desenha as linhas visuais `lines` com `origin` no canto do texto; `paint(id)` dá a cor
    /// da faixa `id` do texto (`0` é o texto fora delas), como no
    /// [`TextLayout::draw_styled_lines`].
    pub(crate) fn draw_styled_lines(
        &self,
        canvas: &mut dyn Canvas,
        origin: Point,
        lines: Range<usize>,
        mut paint: impl FnMut(u32) -> Paint,
    ) {
        let first = self
            .paragraphs
            .partition_point(|p| p.first_line + p.paragraph.lines <= lines.start);
        for placed in &self.paragraphs[first..] {
            if placed.first_line >= lines.end {
                break;
            }
            let Some(layout) = &placed.paragraph.layout else {
                continue;
            };
            let local = lines.start.saturating_sub(placed.first_line)
                ..(lines.end - placed.first_line).min(placed.paragraph.lines);
            let at = Point::new(
                origin.x,
                origin.y + placed.first_line as f64 * line_height(),
            );
            layout.draw_styled_lines(canvas, at, local, |id| {
                let global = id
                    .checked_sub(1)
                    .and_then(|i| placed.ranges.get(i as usize))
                    .map_or(0, |&g| g + 1);
                paint(global)
            });
        }
    }
}

struct Entry {
    text: Rc<str>,
    ranges: Vec<StyleRange>,
    wrap: Option<u32>,
    mono: bool,
    layout: Rc<EditorLayout>,
    points: Rc<EditorPoints>,
}

/// Os layouts dos últimos textos do editor, compartilhados entre o `FluentDrawer` (que pinta)
/// e as [`FluentTextMetrics`](crate::FluentTextMetrics) (que medem para o widget): o texto é
/// medido uma vez por mudança, e só os parágrafos que mudaram passam pelo shaping de novo.
#[derive(Default)]
pub(crate) struct EditorCache {
    entries: Vec<Entry>,
    paragraphs: HashMap<ParagraphKey, (Rc<Paragraph>, u64)>,
    /// Quantas montagens já houve: marca quando cada parágrafo foi usado.
    generation: u64,
    /// Quantos parágrafos passaram pelo shaping, para os testes.
    shaped: usize,
}

impl std::fmt::Debug for EditorCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EditorCache")
            .field("entries", &self.entries.len())
            .field("paragraphs", &self.paragraphs.len())
            .finish()
    }
}

impl EditorCache {
    /// O layout e os pontos de cursor de `text` com `ranges`, quebrado em `wrap`.
    pub(crate) fn get(
        &mut self,
        fonts: &RefCell<Fonts>,
        text: &str,
        ranges: &[StyleRange],
        wrap: Option<f64>,
        mono: bool,
    ) -> (Rc<EditorLayout>, Rc<EditorPoints>) {
        let wrap = wrap.map(|w| (w.max(0.0) as f32).to_bits());
        if let Some(i) = self.entries.iter().position(|e| {
            e.wrap == wrap && e.mono == mono && *e.text == *text && e.ranges == ranges
        }) {
            // O mais recente fica no fim.
            let entry = self.entries.remove(i);
            let hit = (Rc::clone(&entry.layout), Rc::clone(&entry.points));
            self.entries.push(entry);
            return hit;
        }
        let (layout, points) = self.build(fonts, text, ranges, wrap, mono);
        let (layout, points) = (Rc::new(layout), Rc::new(points));
        if self.entries.len() >= CACHE_ENTRIES {
            self.entries.remove(0);
        }
        self.entries.push(Entry {
            text: Rc::from(text),
            ranges: ranges.to_vec(),
            wrap,
            mono,
            layout: Rc::clone(&layout),
            points: Rc::clone(&points),
        });
        (layout, points)
    }

    /// Quantos parágrafos passaram pelo shaping desde o começo.
    #[cfg(test)]
    fn shaped(&self) -> usize {
        self.shaped
    }

    /// Monta o texto com os parágrafos do cache, fazendo o shaping só dos que faltam.
    fn build(
        &mut self,
        fonts: &RefCell<Fonts>,
        text: &str,
        ranges: &[StyleRange],
        wrap: Option<u32>,
        mono: bool,
    ) -> (EditorLayout, EditorPoints) {
        self.generation += 1;
        let generation = self.generation;
        // Os parágrafos: `(início, fim)` em bytes do texto, sem o `\n`.
        let mut bounds = Vec::new();
        let mut start = 0;
        for (i, _) in text.match_indices('\n') {
            bounds.push((start, i));
            start = i + 1;
        }
        bounds.push((start, text.len()));
        // As faixas de cada parágrafo, na ordem das faixas.
        let mut local: Vec<Vec<(usize, usize, usize)>> = vec![Vec::new(); bounds.len()];
        for (index, range) in ranges.iter().enumerate() {
            let (from, to) = (
                range.range.start.min(text.len()),
                range.range.end.min(text.len()),
            );
            if from >= to {
                continue;
            }
            let first = bounds.partition_point(|&(_, end)| end < from);
            for (p, &(s, e)) in bounds.iter().enumerate().skip(first) {
                if s >= to {
                    break;
                }
                let (a, b) = (from.max(s), to.min(e));
                if a < b {
                    local[p].push((index, a - s, b - s));
                }
            }
        }
        let mut paragraphs = Vec::with_capacity(bounds.len());
        let mut points = Vec::with_capacity(text.len() + bounds.len());
        let mut line = 0;
        let mut width: f64 = 0.0;
        for (&(s, e), local) in bounds.iter().zip(local) {
            let key = ParagraphKey {
                text: Rc::from(&text[s..e]),
                ranges: local
                    .iter()
                    .map(|&(index, a, b)| {
                        let r = &ranges[index];
                        (a, b, r.bold, r.italic, r.strike)
                    })
                    .collect(),
                wrap,
                mono,
            };
            let paragraph = match self.paragraphs.get_mut(&key) {
                Some((paragraph, used)) => {
                    *used = generation;
                    Rc::clone(paragraph)
                }
                None => {
                    self.shaped += 1;
                    let paragraph = Rc::new(shape(fonts, &key));
                    self.paragraphs
                        .insert(key, (Rc::clone(&paragraph), generation));
                    paragraph
                }
            };
            points.extend(
                paragraph
                    .points
                    .iter()
                    .map(|&(byte, x, l)| (s + byte, x, line + l)),
            );
            width = width.max(paragraph.width);
            paragraphs.push(Placed {
                first_line: line,
                ranges: local.iter().map(|&(index, _, _)| index as u32).collect(),
                paragraph: Rc::clone(&paragraph),
            });
            line += paragraph.lines;
        }
        // Os parágrafos que não aparecem há algumas montagens saem, quando o cache cresce.
        if self.paragraphs.len() > 2 * bounds.len() + 1024 {
            self.paragraphs
                .retain(|_, (_, used)| *used + PARAGRAPH_GENERATIONS > generation);
        }
        (
            EditorLayout { paragraphs },
            EditorPoints {
                points,
                width,
                lines: line.max(1),
            },
        )
    }
}

/// O shaping de um parágrafo.
fn shape(fonts: &RefCell<Fonts>, key: &ParagraphKey) -> Paragraph {
    if key.text.is_empty() {
        return Paragraph {
            layout: None,
            points: vec![(0, 0.0, 0)],
            lines: 1,
            width: 0.0,
        };
    }
    let mut runs = Vec::with_capacity(key.ranges.len() + 1);
    if key.mono {
        runs.push(RunStyle {
            range: 0..key.text.len(),
            id: 0,
            mono: true,
            ..RunStyle::default()
        });
    }
    for (i, &(a, b, bold, italic, strike)) in key.ranges.iter().enumerate() {
        runs.push(RunStyle {
            range: a..b,
            id: i as u32 + 1,
            bold,
            italic,
            strike,
            ..RunStyle::default()
        });
    }
    let layout = fonts.borrow_mut().layout_ranged(
        &key.text,
        TextStyle::BODY,
        key.wrap.map(f32::from_bits),
        &runs,
    );
    let points: Vec<(usize, f64, usize)> = layout
        .caret_points(&key.text)
        .into_iter()
        .map(|(byte, x, line)| (byte, f64::from(x), line))
        .collect();
    let lines = points.last().map_or(1, |p| p.2 + 1);
    Paragraph {
        width: f64::from(layout.width()),
        layout: Some(layout),
        points,
        lines,
    }
}

/// Os pontos de `text`, pelo cache.
pub(crate) fn editor_points(
    fonts: &RefCell<Fonts>,
    cache: &RefCell<EditorCache>,
    text: &EditorText,
) -> Rc<EditorPoints> {
    cache
        .borrow_mut()
        .get(fonts, text.text, text.ranges, text.wrap, text.mono)
        .1
}

fn layout_of(
    drawer: &FluentDrawer,
    visual: &TextEditorVisual,
    area: Rect,
) -> (Rc<EditorLayout>, Rc<EditorPoints>) {
    let wrap = visual.wrap.then(|| area.width());
    drawer.editors.borrow_mut().get(
        &drawer.fonts,
        &visual.text,
        &visual.ranges,
        wrap,
        visual.mono,
    )
}

/// Altura de uma linha.
fn line_height() -> f64 {
    f64::from(TextStyle::BODY.line_height)
}

/// Tamanho natural: a largura mínima de um campo; a altura é a mínima, ou, com `max_height`,
/// a do texto quebrado na largura oferecida, entre a mínima e a máxima.
pub(crate) fn measure(
    drawer: &FluentDrawer,
    visual: &TextEditorVisual,
    max_width: Option<f64>,
) -> Size {
    let width = metrics::TEXT_CONTROL_MIN_WIDTH;
    let Some(max_height) = visual.max_height else {
        return Size::new(width, visual.min_height);
    };
    let probe = multiline_area(Rect::new(0.0, 0.0, 1000.0, 1000.0), visual.density);
    let (pad_x, pad_y) = (1000.0 - probe.width(), 1000.0 - probe.height());
    let wrap = max_width
        .filter(|_| visual.wrap)
        .map(|w| (w - pad_x).max(1.0));
    let (_, points) = drawer.editors.borrow_mut().get(
        &drawer.fonts,
        &visual.text,
        &visual.ranges,
        wrap,
        visual.mono,
    );
    let height = points.lines as f64 * line_height() + pad_y;
    Size::new(
        width,
        height.clamp(visual.min_height, max_height.max(visual.min_height)),
    )
}

/// Onde o cursor aparece, na janela. `None` sem cursor.
pub(crate) fn caret_rect(
    drawer: &FluentDrawer,
    rect: Rect,
    visual: &TextEditorVisual,
) -> Option<Rect> {
    let caret = visual.caret?;
    let area = multiline_area(rect, visual.density);
    let (_, points) = layout_of(drawer, visual, area);
    let offset = visual.scroller.offset();
    let origin = Point::new(area.x0 - offset.x, area.y0 - offset.y);
    Some(caret_at(&points, origin, caret, visual.caret_up))
}

fn caret_at(points: &EditorPoints, origin: Point, caret: usize, up: bool) -> Rect {
    let (x, line) = points.locate(caret, up);
    let x = (origin.x + x).round();
    let y = origin.y + line as f64 * line_height();
    Rect::new(x, y, x + metrics::CARET_WIDTH, y + line_height())
}

/// As faixas do trecho `range`, uma por linha visual, já na janela. Só as linhas `visible`.
fn spans(
    points: &EditorPoints,
    origin: Point,
    range: &Range<usize>,
    visible: &Range<usize>,
) -> Vec<Rect> {
    let line = line_height();
    let top = |l: usize| origin.y + l as f64 * line;
    let (x0, l0) = points.locate(range.start, false);
    let (x1, l1) = points.locate(range.end, true);
    let first = l0.max(visible.start);
    let last = l1.min(visible.end.saturating_sub(1));
    let mut rects = Vec::new();
    for l in first..=last.max(first) {
        if l > last || l < l0 {
            continue;
        }
        let on_line = points.line_points(l);
        let (start, end) = if l0 == l1 {
            (x0, x1)
        } else {
            let line_end = on_line.last().map_or(0.0, |p| p.1);
            // Uma quebra suave termina a linha no mesmo byte em que a seguinte começa; só a
            // quebra de verdade ganha a sobra.
            let hard = on_line.last().is_some_and(|last| {
                points
                    .line_points(l + 1)
                    .first()
                    .is_none_or(|next| next.0 != last.0)
            });
            let tail = if hard { LINE_BREAK_TAIL } else { 0.0 };
            (
                if l == l0 { x0 } else { 0.0 },
                if l == l1 { x1 } else { line_end + tail },
            )
        };
        rects.push(Rect::new(
            (origin.x + start).round(),
            top(l),
            (origin.x + end).round(),
            top(l) + line,
        ));
    }
    rects
}

fn mark_color(t: &Tokens, current: bool) -> Color {
    if current {
        let icon = t.info_bar_warning_icon.to_rgba8();
        crate::over(
            Color::from_rgba8(icon.r, icon.g, icon.b, CURRENT_MARK_ALPHA),
            t.info_bar_warning_background,
        )
    } else {
        t.info_bar_warning_background
    }
}

pub(crate) fn draw(
    drawer: &FluentDrawer,
    canvas: &mut dyn Canvas,
    rect: Rect,
    visual: &TextEditorVisual,
) {
    let t = &drawer.tokens;
    let state = visual.state;
    let inner = draw_frame(t, canvas, rect, state);
    let area = multiline_area(rect, visual.density);
    let clip = Rect::new(area.x0, inner.y0, area.x1, inner.y1).to_path(PATH_TOLERANCE);
    canvas.push_clip(&clip);

    let disabled = state == EditState::Disabled;
    let base = if disabled {
        t.text_fill_disabled
    } else {
        t.text_fill_primary
    };

    if visual.text.is_empty() {
        let placeholder =
            drawer
                .fonts
                .borrow_mut()
                .layout(&visual.placeholder, TextStyle::BODY, None);
        placeholder.draw(
            canvas,
            area.origin(),
            &Paint::Solid(placeholder_color(t, state)),
        );
    } else {
        let (layout, points) = layout_of(drawer, visual, area);
        let offset = visual.scroller.offset();
        let origin = Point::new(area.x0 - offset.x, area.y0 - offset.y);
        let line = line_height();
        let first = ((offset.y / line).floor().max(0.0) as usize).saturating_sub(1);
        let last = (((offset.y + area.height()) / line).ceil() as usize + 1).min(points.lines);
        let visible = first..last;

        for style in visual.ranges.iter() {
            if let Some(background) = style.background {
                for span in spans(&points, origin, &style.range, &visible) {
                    canvas.fill(&span, t.resolve(background));
                }
            }
        }
        for mark in visual.marks.iter() {
            for span in spans(&points, origin, &mark.range, &visible) {
                canvas.fill(&span, mark_color(t, mark.current));
            }
        }
        if let Some(selection) = &visual.selection {
            for span in spans(&points, origin, selection, &visible) {
                canvas.fill(&span, t.accent_selected_text_background);
            }
        }

        let ranges = &visual.ranges;
        layout.draw_styled_lines(canvas, origin, visible.clone(), |id| {
            let color = match id.checked_sub(1).and_then(|i| ranges.get(i as usize)) {
                Some(StyleRange {
                    color: Some(color), ..
                }) if !disabled => t.resolve(*color),
                _ => base,
            };
            Paint::Solid(color)
        });

        if let Some(selection) = &visual.selection {
            for span in spans(&points, origin, selection, &visible) {
                canvas.push_clip(&span.to_path(PATH_TOLERANCE));
                layout.draw_styled_lines(canvas, origin, visible.clone(), |_| {
                    Paint::Solid(t.text_on_accent_selected_text)
                });
                canvas.pop_clip();
            }
        }

        if let Some(composing) = &visual.composing {
            for span in spans(&points, origin, composing, &visible) {
                let y = span.y0 + metrics::COMPOSITION_UNDERLINE;
                canvas.fill(&Rect::new(span.x0, y, span.x1, y + 1.0), base);
            }
        }

        if visual.selection.is_none()
            && let Some(caret) = visual.caret
        {
            canvas.fill(
                &caret_at(&points, origin, caret, visual.caret_up),
                t.text_fill_primary,
            );
        }
    }
    canvas.pop_clip();

    scroll_viewer::draw(
        canvas,
        TextEditorVisual::bars_rect(rect),
        &ScrollViewerVisual {
            scroller: visual.scroller,
            state: visual.bar,
            bars_hidden: false,
        },
        t,
    );
}

/// O texto visível, para o lint: o placeholder (cortado se não couber) ou o texto, com um item
/// a mais por faixa de cor própria.
pub(crate) fn text_info(
    drawer: &FluentDrawer,
    rect: Rect,
    visual: &TextEditorVisual,
) -> Vec<TextInfo> {
    let t = &drawer.tokens;
    let background = crate::over(fill(t, visual.state), drawer.backdrop());
    let area = multiline_area(rect, visual.density);
    let inner = inner(rect, visual.state);
    let disabled = visual.state == EditState::Disabled;
    if visual.text.is_empty() {
        if visual.placeholder.is_empty() {
            return Vec::new();
        }
        let layout = drawer
            .fonts
            .borrow_mut()
            .layout(&visual.placeholder, TextStyle::BODY, None);
        return vec![TextInfo {
            text: visual.placeholder.clone(),
            color: crate::over(placeholder_color(t, visual.state), background),
            background,
            style: TextStyle::BODY,
            bounds: Rect::new(
                area.x0,
                area.y0,
                area.x0 + f64::from(layout.width()),
                area.y0 + line_height(),
            ),
            clip: Some(Rect::new(area.x0, inner.y0, area.x1, inner.y1)),
        }];
    }
    let (_, points) = layout_of(drawer, visual, area);
    let base = if disabled {
        t.text_fill_disabled
    } else {
        t.text_fill_primary
    };
    let offset = visual.scroller.offset();
    let origin = Point::new(area.x0 - offset.x, area.y0 - offset.y);
    let mut infos = vec![TextInfo {
        text: visual.text.to_string(),
        color: crate::over(base, background),
        background,
        style: TextStyle::BODY,
        bounds: Rect::new(
            origin.x,
            origin.y,
            origin.x + points.width,
            origin.y + points.lines as f64 * line_height(),
        ),
        clip: None,
    }];
    if !disabled {
        for style in visual.ranges.iter() {
            let (Some(color), Some(text)) = (style.color, visual.text.get(style.range.clone()))
            else {
                continue;
            };
            let behind = style
                .background
                .map_or(background, |b| crate::over(t.resolve(b), background));
            let (x, line) = points.locate(style.range.start, false);
            let (x1, _) = points.locate(style.range.end, true);
            let top = origin.y + line as f64 * line_height();
            infos.push(TextInfo {
                text: text.to_owned(),
                color: crate::over(t.resolve(color), behind),
                background: behind,
                style: TextStyle::BODY,
                bounds: Rect::new(origin.x + x, top, origin.x + x1.max(x), top + line_height()),
                clip: None,
            });
        }
    }
    infos
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Drawer, Theme};
    use tomni_ui_core::{Density, Mark, Scroller};

    fn visual(text: &str) -> TextEditorVisual {
        TextEditorVisual {
            text: Rc::from(text),
            placeholder: String::new(),
            caret: None,
            caret_up: false,
            selection: None,
            composing: None,
            ranges: Rc::default(),
            marks: Rc::new(Vec::<Mark>::new()),
            wrap: true,
            mono: false,
            state: EditState::Rest,
            density: Density::Standard,
            scroller: Scroller::default(),
            bar: tomni_ui_core::ScrollBarState::Rest,
            min_height: 96.0,
            max_height: None,
            read_only: false,
        }
    }

    #[test]
    fn altura_natural_segue_o_texto_ate_o_maximo() {
        let mut drawer = FluentDrawer::new(Theme::Light);
        let mut v = visual("a\nb\nc\nd\ne\nf\ng\nh\ni\nj");
        v.min_height = 40.0;
        v.max_height = Some(120.0);
        let size = drawer.measure_text_editor(&v, Some(300.0));
        assert_eq!(size.height, 120.0);
        v.text = Rc::from("só uma");
        let size = drawer.measure_text_editor(&v, Some(300.0));
        assert_eq!(size.height, 40.0);
    }

    #[test]
    fn quebra_de_linha_aumenta_a_altura() {
        let mut drawer = FluentDrawer::new(Theme::Light);
        let mut v = visual(&"palavra ".repeat(40));
        v.min_height = 20.0;
        v.max_height = Some(1000.0);
        let estreito = drawer.measure_text_editor(&v, Some(120.0)).height;
        let largo = drawer.measure_text_editor(&v, Some(2000.0)).height;
        assert!(estreito > largo * 2.0, "{estreito} contra {largo}");
    }

    #[test]
    fn digitar_refaz_o_shaping_so_do_paragrafo_que_mudou() {
        let fonts = RefCell::new(Fonts::new());
        let mut cache = EditorCache::default();
        let linhas: Vec<String> = (0..500).map(|i| format!("linha {i} **negrito**")).collect();
        let texto = linhas.join("\n");
        let faixas: Vec<StyleRange> = texto
            .match_indices("**negrito**")
            .map(|(i, m)| StyleRange {
                bold: true,
                ..StyleRange::new(i..i + m.len())
            })
            .collect();
        let (_, antes) = cache.get(&fonts, &texto, &faixas, Some(400.0), true);
        assert_eq!(cache.shaped(), 500);
        assert_eq!(antes.lines, 500);

        // Uma letra a mais no começo: as faixas andam, mas cada parágrafo, por dentro, é igual.
        let novo = format!("x{texto}");
        let andadas: Vec<StyleRange> = faixas
            .iter()
            .map(|f| StyleRange::new(f.range.start + 1..f.range.end + 1))
            .map(|f| StyleRange { bold: true, ..f })
            .collect();
        let (_, depois) = cache.get(&fonts, &novo, &andadas, Some(400.0), true);
        assert_eq!(cache.shaped(), 501, "só a primeira linha muda");
        assert_eq!(depois.points.len(), antes.points.len() + 1);
        assert_eq!(depois.locate(novo.len(), false).1, 499);

        // O mesmo texto de novo não refaz nada; outra largura refaz tudo.
        cache.get(&fonts, &novo, &andadas, Some(400.0), true);
        assert_eq!(cache.shaped(), 501);
        cache.get(&fonts, &novo, &andadas, Some(300.0), true);
        assert_eq!(cache.shaped(), 1001);
    }

    #[test]
    fn pontos_por_paragrafo_sao_os_do_texto_inteiro() {
        let fonts = RefCell::new(Fonts::new());
        let mut cache = EditorCache::default();
        let texto = "primeira linha com palavras para quebrar\n\nterceira\n";
        for wrap in [None, Some(90.0)] {
            let (_, pontos) = cache.get(&fonts, texto, &[], wrap, false);
            let inteiro = fonts.borrow_mut().layout_ranged(
                texto,
                TextStyle::BODY,
                wrap.map(|w| w as f32),
                &[],
            );
            let esperado: Vec<(usize, f64, usize)> = inteiro
                .caret_points(texto)
                .into_iter()
                .map(|(b, x, l)| (b, f64::from(x), l))
                .collect();
            assert_eq!(pontos.points, esperado, "wrap {wrap:?}");
            assert_eq!(pontos.lines, esperado.last().unwrap().2 + 1);
        }
    }

    #[test]
    fn cursor_acompanha_a_linha_e_a_rolagem() {
        let mut drawer = FluentDrawer::new(Theme::Light);
        let rect = Rect::new(0.0, 0.0, 300.0, 120.0);
        let mut v = visual("ab\ncd");
        v.caret = Some(3);
        let c = drawer.text_editor_caret_rect(rect, &v).unwrap();
        let area = multiline_area(rect, Density::Standard);
        assert_eq!((c.x0, c.y0), (area.x0, area.y0 + 20.0));
        v.caret = None;
        assert_eq!(drawer.text_editor_caret_rect(rect, &v), None);
    }
}
