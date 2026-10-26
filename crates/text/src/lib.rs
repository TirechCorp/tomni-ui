//! Shaping e layout de texto com `parley`, usando a fonte Selawik (OFL) embutida no binário.
//!
//! Nenhuma fonte do sistema é carregada: o mesmo texto gera os mesmos glifos em todas as
//! plataformas. A Selawik não cobre chinês, japonês e coreano; com a feature `cjk`, a Noto Sans
//! JP (OFL, cerca de 4,5 MB) também é embutida e entra no lugar dela nesses caracteres. Com a
//! feature `mono`, a Cascadia Mono (OFL) também vem, para o texto em grade do terminal
//! ([`mono`]).

use std::borrow::Cow;
use std::ops::Range;
use std::sync::Arc;

use parley::fontique::{Blob, Collection, CollectionOptions, SourceCache};
use tomni_ui_render::kurbo::{Point, Rect};
use tomni_ui_render::{Canvas, Glyph, GlyphRun, Paint};
#[cfg(feature = "mono")]
pub mod mono;

use parley::{
    Alignment, AlignmentOptions, FontContext, FontFamily, FontFamilyName, FontStyle, FontWeight,
    Layout, LayoutContext, LineHeight, PositionedLayoutItem, StyleProperty,
};

/// Família de fonte usada por toda a UI.
pub const FAMILY: &str = "Selawik";

/// Família usada nos caracteres que a Selawik não tem, com a feature `cjk`.
#[cfg(feature = "cjk")]
pub const CJK_FAMILY: &str = "Noto Sans JP";

/// Noto Sans JP Regular, só com a feature `cjk`.
#[cfg(feature = "cjk")]
const NOTO_SANS_JP: &[u8] = include_bytes!("../fonts/NotoSansJP-Regular.otf");

/// Famílias em ordem de preferência: cada caractere usa a primeira que o tiver.
const FAMILIES: &[FontFamilyName<'static>] = &[
    FontFamilyName::named(FAMILY),
    #[cfg(feature = "cjk")]
    FontFamilyName::named(CJK_FAMILY),
];

/// Famílias do código inline (`RunStyle::mono`): a Cascadia Mono, que só existe com a feature
/// `mono`, e a Selawik, para quando ela não vem.
const CODE_FAMILIES: &[FontFamilyName<'static>] = &[
    FontFamilyName::named("Cascadia Mono"),
    FontFamilyName::named(FAMILY),
];

/// Pesos da Selawik embutidos: light, semilight, regular, semibold e bold.
const SELAWIK: [&[u8]; 5] = [
    include_bytes!("../fonts/selawkl.ttf"),
    include_bytes!("../fonts/selawksl.ttf"),
    include_bytes!("../fonts/selawk.ttf"),
    include_bytes!("../fonts/selawksb.ttf"),
    include_bytes!("../fonts/selawkb.ttf"),
];

/// Estilo de um bloco de texto.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    /// Tamanho da fonte em pixels.
    pub size: f32,
    /// Altura de linha em pixels.
    pub line_height: f32,
    /// Peso (400 = regular, 600 = semibold).
    pub weight: f32,
}

impl TextStyle {
    /// Rampa tipográfica Fluent "Body": 14/20, regular.
    pub const BODY: Self = Self {
        size: 14.0,
        line_height: 20.0,
        weight: 400.0,
    };

    /// Rampa tipográfica Fluent "Body Strong": 14/20, semibold.
    pub const BODY_STRONG: Self = Self {
        weight: 600.0,
        ..Self::BODY
    };

    /// Rampa tipográfica Fluent "Caption": 12/16, regular.
    pub const CAPTION: Self = Self {
        size: 12.0,
        line_height: 16.0,
        weight: 400.0,
    };

    /// Rampa tipográfica Fluent "Subtitle": 20/28, semibold.
    pub const SUBTITLE: Self = Self {
        size: 20.0,
        line_height: 28.0,
        weight: 600.0,
    };

    /// Rampa tipográfica Fluent "Title": 28/36, semibold.
    pub const TITLE: Self = Self {
        size: 28.0,
        line_height: 36.0,
        weight: 600.0,
    };

    /// Rampa tipográfica Fluent "Body Large": 18/24, regular.
    pub const BODY_LARGE: Self = Self {
        size: 18.0,
        line_height: 24.0,
        weight: 400.0,
    };

    /// Rampa tipográfica Fluent "Title Large": 40/52, semibold.
    pub const TITLE_LARGE: Self = Self {
        size: 40.0,
        line_height: 52.0,
        weight: 600.0,
    };

    /// Rampa tipográfica Fluent "Display": 68/92, semibold.
    pub const DISPLAY: Self = Self {
        size: 68.0,
        line_height: 92.0,
        weight: 600.0,
    };
}

impl Default for TextStyle {
    fn default() -> Self {
        Self::BODY
    }
}

/// Estilo de uma faixa do texto de [`Fonts::layout_ranged`], em bytes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunStyle {
    /// Faixa do texto, em bytes, nos limites de `char`.
    pub range: Range<usize>,
    /// Identificador da faixa, devolvido na hora de escolher a cor em
    /// [`TextLayout::draw_styled`]. O 0 é do texto fora de qualquer faixa: use de 1 em diante.
    pub id: u32,
    /// Negrito (peso 700).
    pub bold: bool,
    /// Itálico: a Selawik não tem a face itálica, e o itálico sai inclinado por síntese.
    pub italic: bool,
    /// Código: Cascadia Mono, um pouco menor (a Selawik quando a feature `mono` não está ligada).
    pub mono: bool,
    /// Sublinhado.
    pub underline: bool,
    /// Riscado.
    pub strike: bool,
}

/// Banco de fontes e área de trabalho do layout. Crie uma vez por app e reutilize.
pub struct Fonts {
    font_cx: FontContext,
    layout_cx: LayoutContext<u32>,
}

impl Fonts {
    /// Cria o banco só com as fontes embutidas: a Selawik e, com as features `cjk` e `mono`, a
    /// Noto Sans JP e a Cascadia Mono.
    pub fn new() -> Self {
        let mut collection = Collection::new(CollectionOptions {
            shared: false,
            system_fonts: false,
        });
        for data in SELAWIK {
            collection.register_fonts(Blob::new(Arc::new(data)), None);
        }
        #[cfg(feature = "cjk")]
        collection.register_fonts(Blob::new(Arc::new(NOTO_SANS_JP)), None);
        #[cfg(feature = "mono")]
        for data in mono::CASCADIA_MONO {
            collection.register_fonts(Blob::new(Arc::new(data)), None);
        }
        Self {
            font_cx: FontContext {
                collection,
                source_cache: SourceCache::default(),
            },
            layout_cx: LayoutContext::new(),
        }
    }

    /// Faz o shaping de `text` e quebra as linhas em `max_width`, se houver.
    pub fn layout(&mut self, text: &str, style: TextStyle, max_width: Option<f32>) -> TextLayout {
        self.layout_ranged(text, style, max_width, &[])
    }

    /// Como [`Fonts::layout`], com um estilo por faixa do texto (negrito, itálico, código,
    /// sublinhado e riscado). O tamanho da fonte e a altura da linha são os de `style`, em todas
    /// as faixas (o código usa um tamanho um pouco menor, na mesma altura de linha).
    pub fn layout_ranged(
        &mut self,
        text: &str,
        style: TextStyle,
        max_width: Option<f32>,
        runs: &[RunStyle],
    ) -> TextLayout {
        let mut builder = self
            .layout_cx
            .ranged_builder(&mut self.font_cx, text, 1.0, true);
        // Uma faixa de código sobre o texto todo vira o estilo-base: com a faixa por cima de um
        // estilo-base de outra fonte, cada linha ficava 1 px mais alta que `line_height` (as
        // duas caixas de linha, com linhas de base diferentes, se somam), e o editor
        // monoespaçado, que conta linhas por `line_height`, pintava em branco no fim de um
        // texto longo.
        let all_mono = !text.is_empty()
            && runs
                .iter()
                .any(|r| r.mono && r.range.start == 0 && r.range.end >= text.len());
        let (families, size) = if all_mono {
            (CODE_FAMILIES, (style.size * 13.0 / 14.0).round())
        } else {
            (FAMILIES, style.size)
        };
        builder.push_default(StyleProperty::FontFamily(FontFamily::List(Cow::Borrowed(
            families,
        ))));
        builder.push_default(StyleProperty::FontSize(size));
        builder.push_default(StyleProperty::LineHeight(LineHeight::Absolute(
            style.line_height,
        )));
        builder.push_default(StyleProperty::FontWeight(FontWeight::new(style.weight)));
        for run in runs {
            let range = run.range.start.min(text.len())..run.range.end.min(text.len());
            if range.is_empty() {
                continue;
            }
            builder.push(StyleProperty::Brush(run.id), range.clone());
            if run.bold {
                builder.push(StyleProperty::FontWeight(FontWeight::BOLD), range.clone());
            }
            if run.italic {
                builder.push(StyleProperty::FontStyle(FontStyle::Italic), range.clone());
            }
            if run.mono && !(all_mono && range.start == 0 && range.end >= text.len()) {
                builder.push(
                    StyleProperty::FontFamily(FontFamily::List(Cow::Borrowed(CODE_FAMILIES))),
                    range.clone(),
                );
                builder.push(
                    StyleProperty::FontSize((style.size * 13.0 / 14.0).round()),
                    range,
                );
            }
        }
        let mut layout = builder.build(text);
        layout.break_all_lines(max_width);
        layout.align(Alignment::Start, AlignmentOptions::default());
        TextLayout {
            layout,
            runs: runs.to_vec(),
        }
    }

    /// Como [`Fonts::layout`], mas com no máximo `max_lines` linhas: se o texto passar disso,
    /// a última linha mostrada termina em reticências ("…"), cortada entre caracteres, como o
    /// `TextTrimming="CharacterEllipsis"` do WinUI. Devolve também se cortou.
    pub fn layout_lines(
        &mut self,
        text: &str,
        style: TextStyle,
        max_width: Option<f32>,
        max_lines: usize,
    ) -> (TextLayout, bool) {
        self.layout_ranged_lines(text, style, max_width, max_lines, &[])
    }

    /// Como [`Fonts::layout_lines`], com as faixas de [`Fonts::layout_ranged`]. As faixas que
    /// passam do corte são cortadas junto, e as reticências ficam sem faixa.
    pub fn layout_ranged_lines(
        &mut self,
        text: &str,
        style: TextStyle,
        max_width: Option<f32>,
        max_lines: usize,
        runs: &[RunStyle],
    ) -> (TextLayout, bool) {
        let max_lines = max_lines.max(1);
        let full = self.layout_ranged(text, style, max_width, runs);
        let Some(last) = full.layout.get(max_lines - 1) else {
            return (full, false);
        };
        if full.layout.len() <= max_lines {
            return (full, false);
        }
        // O texto até a última linha mostrada inteira continua quebrando igual; na última,
        // procura o corte mais longo em que o texto e as reticências ainda cabem nela.
        let range = last.text_range();
        let cuts: Vec<usize> = text[range.clone()]
            .char_indices()
            .map(|(i, _)| range.start + i)
            .chain([range.end])
            .collect();
        let trimmed = |fonts: &mut Self, cut: usize| {
            let base = text[..cut].trim_end();
            let shown = format!("{base}…");
            let cut_runs: Vec<RunStyle> = runs
                .iter()
                .filter(|r| r.range.start < base.len())
                .map(|r| RunStyle {
                    range: r.range.start..r.range.end.min(base.len()),
                    ..r.clone()
                })
                .collect();
            fonts.layout_ranged(&shown, style, max_width, &cut_runs)
        };
        let (mut lo, mut hi) = (0, cuts.len() - 1);
        let mut best = trimmed(self, cuts[0]);
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            let layout = trimmed(self, cuts[mid]);
            if layout.layout.len() <= max_lines {
                lo = mid;
                best = layout;
            } else {
                hi = mid - 1;
            }
        }
        if lo > 0 && best.layout.len() > max_lines {
            best = trimmed(self, cuts[lo]);
        }
        (best, true)
    }

    /// Caracteres de `text` que nenhuma fonte embutida tem: na tela, viram um retângulo
    /// (o glifo `.notdef`). Sem repetição, na ordem em que aparecem; espaços e caracteres de
    /// controle ficam de fora.
    pub fn missing_glyphs(&mut self, text: &str, style: TextStyle) -> Vec<char> {
        self.layout(text, style, None).missing_chars(text)
    }
}

impl Default for Fonts {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Fonts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fonts").finish_non_exhaustive()
    }
}

/// Texto já posicionado, pronto para medir e desenhar.
pub struct TextLayout {
    layout: Layout<u32>,
    runs: Vec<RunStyle>,
}

impl TextLayout {
    /// Largura ocupada, sem o espaço em branco do fim das linhas.
    pub fn width(&self) -> f32 {
        self.layout.width()
    }

    /// Altura total das linhas.
    pub fn height(&self) -> f32 {
        self.layout.height()
    }

    /// Número de linhas.
    pub fn lines(&self) -> usize {
        self.layout.len()
    }

    /// Posições onde o cursor pode parar: `(byte, x)` em ordem crescente de byte, de `(0, 0.0)`
    /// até `(text.len(), largura)`. Um cluster (letra com acento combinado, ligadura) não se
    /// divide. Pensado para uma linha só, da esquerda para a direita.
    pub fn carets(&self) -> Vec<(usize, f32)> {
        let mut carets = Vec::new();
        let mut end = (0, 0.0);
        for line in self.layout.lines() {
            let mut x = line.metrics().offset;
            for run in line.runs() {
                for cluster in run.visual_clusters() {
                    let range = cluster.text_range();
                    carets.push((range.start, x));
                    x += cluster.advance();
                    if range.end >= end.0 {
                        end = (range.end, x);
                    }
                }
            }
        }
        carets.push(end);
        carets.sort_by_key(|&(byte, _)| byte);
        carets.dedup_by_key(|&mut (byte, _)| byte);
        carets
    }

    /// Como [`TextLayout::carets`], para texto de várias linhas (quebras `\n`, sem quebra
    /// automática): `(byte, x, linha)` em ordem crescente de byte. O fim de cada linha vem antes
    /// da quebra, e o começo da seguinte é o byte logo depois dela, com `x` zero. `text` é o
    /// texto deste layout.
    pub fn caret_points(&self, text: &str) -> Vec<(usize, f32, usize)> {
        let mut points: Vec<(usize, f32, usize)> = Vec::new();
        for (index, line) in self.layout.lines().enumerate() {
            let mut range = line.text_range();
            range.end = range.end.min(text.len());
            range.start = range.start.min(range.end);
            let end = range.end - usize::from(text[range.clone()].ends_with('\n'));
            let mut x = line.metrics().offset;
            points.push((range.start, x, index));
            for run in line.runs() {
                for cluster in run.visual_clusters() {
                    let cluster_range = cluster.text_range();
                    x += cluster.advance();
                    if cluster_range.end <= end && cluster_range.end > range.start {
                        points.push((cluster_range.end, x, index));
                    }
                }
            }
            if points.last().is_none_or(|last| last.0 != end) {
                points.push((end, x, index));
            }
        }
        // O texto que termina em quebra tem uma última linha vazia, que o parley pode não criar.
        if points.last().is_none_or(|last| last.0 != text.len()) {
            points.push((text.len(), 0.0, self.layout.len()));
        }
        points.sort_by(|a, b| a.0.cmp(&b.0).then(a.2.cmp(&b.2)));
        points.dedup_by(|b, a| a.0 == b.0 && a.2 == b.2);
        points
    }

    /// Caracteres de `text` (o texto deste layout) que saíram com o glifo `.notdef`, sem
    /// repetição e sem espaços nem caracteres de controle.
    pub fn missing_chars(&self, text: &str) -> Vec<char> {
        let mut missing = Vec::new();
        for line in self.layout.lines() {
            for run in line.runs() {
                for cluster in run.visual_clusters() {
                    if !cluster.glyphs().any(|g| g.id == 0) {
                        continue;
                    }
                    let Some(chars) = text.get(cluster.text_range()) else {
                        continue;
                    };
                    for c in chars.chars() {
                        if !c.is_whitespace() && !c.is_control() && !missing.contains(&c) {
                            missing.push(c);
                        }
                    }
                }
            }
        }
        missing
    }

    /// Glifos de todas as linhas, com `origin` no canto superior esquerdo do bloco.
    pub fn glyphs(&self, origin: Point) -> Vec<Glyph> {
        let mut glyphs = Vec::new();
        self.for_each_run(origin, 0..usize::MAX, |run, _| {
            glyphs.extend_from_slice(run.glyphs);
        });
        glyphs
    }

    /// Desenha o texto com `origin` no canto superior esquerdo do bloco.
    pub fn draw(&self, canvas: &mut (impl Canvas + ?Sized), origin: Point, paint: &Paint) {
        self.for_each_run(origin, 0..usize::MAX, |run, _| {
            canvas.draw_glyphs(run, paint)
        });
    }

    /// Desenha o texto de um layout com faixas ([`Fonts::layout_ranged`]): `paint(id)` dá a cor
    /// de cada faixa (o 0 é o texto fora delas), e o sublinhado e o riscado das faixas que os
    /// pedem saem como linhas finas na mesma cor.
    pub fn draw_styled(
        &self,
        canvas: &mut (impl Canvas + ?Sized),
        origin: Point,
        paint: impl FnMut(u32) -> Paint,
    ) {
        self.draw_styled_lines(canvas, origin, 0..usize::MAX, paint);
    }

    /// Como [`TextLayout::draw_styled`], mas só das linhas visuais `lines` (a contar do zero):
    /// o editor de uma nota longa desenha só as que aparecem na tela.
    pub fn draw_styled_lines(
        &self,
        canvas: &mut (impl Canvas + ?Sized),
        origin: Point,
        lines: Range<usize>,
        mut paint: impl FnMut(u32) -> Paint,
    ) {
        self.for_each_run(origin, lines, |run, geometry| {
            let paint = paint(geometry.id);
            canvas.draw_glyphs(run, &paint);
            let Some(style) = self.runs.iter().find(|r| r.id == geometry.id) else {
                return;
            };
            let thickness = f64::from((geometry.font_size / 14.0).round().max(1.0));
            let width = f64::from(geometry.advance);
            let x = f64::from(geometry.x);
            let baseline = f64::from(geometry.baseline);
            if style.underline {
                let y = (baseline + f64::from(geometry.font_size) * 0.1)
                    .round()
                    .max(baseline + 1.0);
                canvas.fill_rect(Rect::new(x, y, x + width, y + thickness), &paint);
            }
            if style.strike {
                let y = (baseline - f64::from(geometry.font_size) * 0.3).round();
                canvas.fill_rect(Rect::new(x, y, x + width, y + thickness), &paint);
            }
        });
    }

    /// Os retângulos que a faixa `range` (em bytes de `text`) ocupa, um por linha visual,
    /// relativos ao canto superior esquerdo do bloco. Serve para o fundo do código e para a
    /// área clicável de um link.
    pub fn range_rects(&self, range: Range<usize>) -> Vec<Rect> {
        self.range_rects_in(range, 0..usize::MAX)
    }

    /// Como [`TextLayout::range_rects`], só nas linhas visuais `lines`: o realce de uma busca
    /// numa nota longa mede só as linhas que aparecem.
    pub fn range_rects_in(&self, range: Range<usize>, lines: Range<usize>) -> Vec<Rect> {
        let mut rects: Vec<Rect> = Vec::new();
        for index in lines.start..lines.end.min(self.layout.len()) {
            let Some(line) = self.layout.get(index) else {
                break;
            };
            let metrics = line.metrics();
            let mut span: Option<(f32, f32)> = None;
            let mut x = metrics.offset;
            for run in line.runs() {
                for cluster in run.visual_clusters() {
                    let cluster_range = cluster.text_range();
                    let start = x;
                    x += cluster.advance();
                    if cluster_range.start < range.end && cluster_range.end > range.start {
                        span = Some(match span {
                            Some((a, b)) => (a.min(start), b.max(x)),
                            None => (start, x),
                        });
                    }
                }
            }
            if let Some((a, b)) = span {
                rects.push(Rect::new(
                    f64::from(a),
                    f64::from(metrics.block_min_coord),
                    f64::from(b),
                    f64::from(metrics.block_max_coord),
                ));
            }
        }
        rects
    }

    fn for_each_run(
        &self,
        origin: Point,
        lines: Range<usize>,
        mut f: impl FnMut(&GlyphRun<'_>, RunGeometry),
    ) {
        let (ox, oy) = (origin.x as f32, origin.y as f32);
        let mut glyphs = Vec::new();
        let mut coords = Vec::new();
        for index in lines.start..lines.end.min(self.layout.len()) {
            let Some(line) = self.layout.get(index) else {
                break;
            };
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                    continue;
                };
                glyphs.clear();
                glyphs.extend(glyph_run.positioned_glyphs().map(|g| Glyph {
                    id: g.id,
                    x: ox + g.x,
                    y: oy + g.y,
                }));
                let run = glyph_run.run();
                coords.clear();
                coords.extend(run.normalized_coords().iter().map(|c| c.to_bits()));
                let skew = run
                    .synthesis()
                    .skew()
                    .map_or(0.0, |angle| angle.to_radians().tan());
                f(
                    &GlyphRun {
                        font: run.font(),
                        font_size: run.font_size(),
                        normalized_coords: &coords,
                        glyphs: &glyphs,
                        skew,
                    },
                    RunGeometry {
                        id: glyph_run.style().brush,
                        x: ox + glyph_run.offset(),
                        baseline: oy + glyph_run.baseline(),
                        advance: glyph_run.advance(),
                        font_size: run.font_size(),
                    },
                );
            }
        }
    }
}

/// Onde um trecho de glifos cai, para as linhas do sublinhado e do riscado.
struct RunGeometry {
    id: u32,
    x: f32,
    baseline: f32,
    advance: f32,
    font_size: f32,
}

impl std::fmt::Debug for TextLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextLayout")
            .field("width", &self.width())
            .field("height", &self.height())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn o_texto_todo_em_codigo_tem_a_altura_de_linha_pedida() {
        let mut fonts = Fonts::new();
        let texto = "uma linha\n".repeat(200);
        let runs = [RunStyle {
            range: 0..texto.len(),
            mono: true,
            ..RunStyle::default()
        }];
        let layout = fonts.layout_ranged(&texto, TextStyle::BODY, None, &runs);
        // 200 linhas e a vazia depois da última quebra.
        assert_eq!(layout.height(), 201.0 * TextStyle::BODY.line_height);
    }

    #[test]
    fn selawik_cobre_acentos_do_portugues() {
        let mut fonts = Fonts::new();
        let layout = fonts.layout("Ação, coração, órgão, você!", TextStyle::BODY, None);
        let glyphs = layout.glyphs(Point::ZERO);
        assert!(!glyphs.is_empty());
        // O glifo 0 é o .notdef: aparece quando a fonte não tem o caractere.
        assert!(glyphs.iter().all(|g| g.id != 0), "caractere sem glifo");
    }

    fn faixa(range: Range<usize>, bold: bool, italic: bool) -> RunStyle {
        RunStyle {
            range,
            id: 1,
            bold,
            italic,
            ..RunStyle::default()
        }
    }

    #[test]
    fn negrito_e_mais_largo() {
        let mut fonts = Fonts::new();
        let texto = "Texto de teste";
        let normal = fonts.layout(texto, TextStyle::BODY, None).width();
        let negrito = fonts
            .layout_ranged(
                texto,
                TextStyle::BODY,
                None,
                &[faixa(0..texto.len(), true, false)],
            )
            .width();
        assert!(negrito > normal, "{negrito} <= {normal}");
    }

    #[test]
    fn italico_inclina_so_a_faixa() {
        let mut fonts = Fonts::new();
        let texto = "reto itálico";
        let layout = fonts.layout_ranged(
            texto,
            TextStyle::BODY,
            None,
            &[faixa(5..texto.len(), false, true)],
        );
        let mut inclinacoes = Vec::new();
        layout.for_each_run(Point::ZERO, 0..usize::MAX, |run, geometry| {
            inclinacoes.push((geometry.id, run.skew));
        });
        assert!(inclinacoes.iter().any(|&(id, skew)| id == 0 && skew == 0.0));
        assert!(inclinacoes.iter().any(|&(id, skew)| id == 1 && skew > 0.0));
    }

    #[test]
    fn retangulos_da_faixa_um_por_linha() {
        let mut fonts = Fonts::new();
        let texto = "um dois três quatro cinco seis sete oito nove dez";
        let layout = fonts.layout(texto, TextStyle::BODY, Some(120.0));
        assert!(layout.lines() > 2);
        let inicio = texto.find("quatro").unwrap();
        let faixa = layout.range_rects(inicio..inicio + "quatro".len());
        assert_eq!(faixa.len(), 1);
        assert!(faixa[0].width() > 20.0);
        let tudo = layout.range_rects(0..texto.len());
        assert_eq!(tudo.len(), layout.lines());
        assert!(tudo.windows(2).all(|w| w[0].y1 <= w[1].y0 + 0.5));
    }

    #[test]
    fn linhas_limitadas_cortam_as_faixas() {
        let mut fonts = Fonts::new();
        let texto = "um dois três quatro cinco seis sete oito nove dez onze doze";
        let faixas = [faixa(0..texto.len(), true, false)];
        let (layout, cortou) =
            fonts.layout_ranged_lines(texto, TextStyle::BODY, Some(120.0), 2, &faixas);
        assert!(cortou);
        assert_eq!(layout.lines(), 2);
        let (inteiro, cortou) =
            fonts.layout_ranged_lines("curto", TextStyle::BODY, Some(120.0), 2, &[]);
        assert!(!cortou);
        assert_eq!(inteiro.lines(), 1);
    }

    #[test]
    fn pontos_do_cursor_em_varias_linhas() {
        let mut fonts = Fonts::new();
        let texto = "ab\n\ncde\n";
        let layout = fonts.layout(texto, TextStyle::BODY, None);
        let pontos = layout.caret_points(texto);
        let por_linha = |linha: usize| -> Vec<usize> {
            pontos
                .iter()
                .filter(|p| p.2 == linha)
                .map(|p| p.0)
                .collect()
        };
        assert_eq!(por_linha(0), [0, 1, 2]);
        assert_eq!(por_linha(1), [3]);
        assert_eq!(por_linha(2), [4, 5, 6, 7]);
        assert_eq!(por_linha(3), [8]);
        // Cada linha começa em x zero e o x só cresce dentro dela.
        for linha in 0..4 {
            let xs: Vec<f32> = pontos
                .iter()
                .filter(|p| p.2 == linha)
                .map(|p| p.1)
                .collect();
            assert_eq!(xs[0], 0.0);
            assert!(xs.windows(2).all(|w| w[0] <= w[1]));
        }
        assert!(pontos[1].1 > 0.0);
        // Sem texto, há o ponto único.
        let vazio = fonts.layout("", TextStyle::BODY, None);
        assert_eq!(vazio.caret_points(""), [(0, 0.0, 0)]);
    }

    #[test]
    fn quebra_linhas_na_largura_maxima() {
        let mut fonts = Fonts::new();
        let uma = fonts.layout("Entrar no sistema agora", TextStyle::BODY, None);
        let varias = fonts.layout("Entrar no sistema agora", TextStyle::BODY, Some(60.0));
        assert_eq!(uma.height(), 20.0);
        assert!(varias.height() > uma.height());
        assert!(varias.width() <= 60.0);
    }

    #[test]
    fn corta_com_reticencias_na_ultima_linha() {
        let mut fonts = Fonts::new();
        let texto = "Uma descrição longa que não cabe em duas linhas de cento e vinte pixels.";
        let (layout, cortou) = fonts.layout_lines(texto, TextStyle::BODY, Some(120.0), 2);
        assert!(cortou);
        assert_eq!(layout.lines(), 2);
        assert!(layout.width() <= 120.0);
        assert!(
            fonts.missing_glyphs("…", TextStyle::BODY).is_empty(),
            "a Selawik tem as reticências"
        );
        let (curto, cortou) = fonts.layout_lines("Curto.", TextStyle::BODY, Some(120.0), 2);
        assert!(!cortou);
        assert_eq!(curto.lines(), 1);
        // Mais estreito que a palavra: continua em duas linhas, só com o que couber.
        let (estreito, _) = fonts.layout_lines(texto, TextStyle::BODY, Some(30.0), 2);
        assert!(estreito.lines() <= 2);
    }

    #[test]
    fn cursor_para_entre_clusters() {
        let mut fonts = Fonts::new();
        let layout = fonts.layout("ae\u{301}b", TextStyle::BODY, None);
        let carets = layout.carets();
        let bytes: Vec<usize> = carets.iter().map(|c| c.0).collect();
        assert_eq!(bytes, [0, 1, 4, 5], "o acento combinado fica junto do e");
        assert_eq!(carets[0].1, 0.0);
        assert!(carets.windows(2).all(|w| w[0].1 < w[1].1));
        assert_eq!(carets[3].1, layout.width());
        assert_eq!(fonts.layout("", TextStyle::BODY, None).carets(), [(0, 0.0)]);
    }

    #[test]
    fn selawik_tem_o_caractere_da_senha() {
        // O PasswordBox do WinUI usa o ● (U+25CF), que a Selawik não tem; o Edit usa o •.
        let mut fonts = Fonts::new();
        let glyphs = fonts
            .layout("\u{2022}", TextStyle::BODY, None)
            .glyphs(Point::ZERO);
        assert!(glyphs.iter().all(|g| g.id != 0));
    }

    #[test]
    fn acentos_do_portugues_nao_faltam() {
        let mut fonts = Fonts::new();
        let texto = "Ação, coração, órgão, você! Àquela pé-de-moleque à ÇÃÕÉÊÍÓÔÚÜ «» € – —";
        for style in [TextStyle::BODY, TextStyle::TITLE] {
            assert_eq!(fonts.missing_glyphs(texto, style), Vec::<char>::new());
        }
    }

    #[cfg(not(feature = "cjk"))]
    #[test]
    fn selawik_nao_tem_os_ordinais() {
        // "1º" e "2ª" saem com retângulo: escreva por extenso ("primeiro", "segunda via").
        let mut fonts = Fonts::new();
        assert_eq!(
            fonts.missing_glyphs("1º lugar, 2ª via", TextStyle::BODY),
            ['º', 'ª']
        );
    }

    #[test]
    fn emoji_falta_mesmo_com_cjk() {
        let mut fonts = Fonts::new();
        assert_eq!(
            fonts.missing_glyphs("Pronto 🎉 🎉 ok", TextStyle::BODY),
            ['🎉']
        );
    }

    #[cfg(not(feature = "cjk"))]
    #[test]
    fn sinal_de_menos_falta_sem_cjk() {
        let mut fonts = Fonts::new();
        assert_eq!(
            fonts.missing_glyphs("Saldo: \u{2212}10 e -5", TextStyle::BODY),
            ['\u{2212}']
        );
    }

    #[cfg(feature = "cjk")]
    #[test]
    fn cjk_usa_a_noto_sans_jp() {
        let mut fonts = Fonts::new();
        let layout = fonts.layout("日本語とカナ", TextStyle::BODY, None);
        let glyphs = layout.glyphs(Point::ZERO);
        assert_eq!(glyphs.len(), 6);
        assert!(glyphs.iter().all(|g| g.id != 0), "caractere sem glifo");
        assert_eq!(layout.carets().len(), 7);
    }

    #[test]
    fn semibold_e_mais_largo_que_regular() {
        let mut fonts = Fonts::new();
        let regular = fonts.layout("Entrar", TextStyle::BODY, None).width();
        let semibold = fonts.layout("Entrar", TextStyle::BODY_STRONG, None).width();
        assert!(
            semibold > regular,
            "{semibold} <= {regular}: peso não aplicado"
        );
    }
}
