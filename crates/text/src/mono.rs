//! Texto em grade, para o terminal: a Cascadia Mono (OFL) embutida, com cada caractere preso
//! à sua célula. Só com a feature `mono`.
//!
//! O shaping é o mesmo do resto da UI (`parley`), mas a posição de cada glifo não vem do
//! avanço da fonte: vem da coluna da célula. Assim um caractere que a Cascadia Mono não tem
//! (um ideograma da Noto Sans JP, com a feature `cjk`) não empurra os seguintes, e um
//! caractere largo ocupa duas células, centralizado nelas.

use std::borrow::Cow;

use parley::{FontFamily, FontFamilyName, FontStyle, FontWeight, LineHeight, StyleProperty};
use tomni_ui_render::kurbo::Point;
use tomni_ui_render::peniko::FontData;
use tomni_ui_render::{Glyph, GlyphRun};

use crate::{FAMILY, Fonts};

/// Família monoespaçada do terminal.
pub const MONO_FAMILY: &str = "Cascadia Mono";

/// Cascadia Mono regular, negrito, itálico e negrito itálico.
pub(crate) const CASCADIA_MONO: [&[u8]; 4] = [
    include_bytes!("../fonts/CascadiaMono-Regular.ttf"),
    include_bytes!("../fonts/CascadiaMono-Bold.ttf"),
    include_bytes!("../fonts/CascadiaMono-Italic.ttf"),
    include_bytes!("../fonts/CascadiaMono-BoldItalic.ttf"),
];

/// Famílias da grade em ordem de preferência: a Cascadia Mono, a Noto Sans JP (com `cjk`) e a
/// Selawik, para o pouco que as outras não tenham.
const MONO_FAMILIES: &[FontFamilyName<'static>] = &[
    FontFamilyName::named(MONO_FAMILY),
    #[cfg(feature = "cjk")]
    FontFamilyName::named(crate::CJK_FAMILY),
    FontFamilyName::named(FAMILY),
];

/// Medidas da célula num tamanho de fonte, em pixels inteiros.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellMetrics {
    /// Tamanho da fonte.
    pub font_size: f32,
    /// Largura da célula: o avanço da Cascadia Mono, arredondado.
    pub width: f32,
    /// Altura da célula: a subida mais a descida da fonte, arredondadas para cima.
    pub height: f32,
    /// Distância do topo da célula à linha de base.
    pub baseline: f32,
    /// Distância do topo da célula ao sublinhado.
    pub underline: f32,
    /// Distância do topo da célula ao tachado.
    pub strikeout: f32,
    /// Espessura do sublinhado e do tachado, em pixels inteiros.
    pub thickness: f32,
}

/// Estilo de um trecho da grade.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CellStyle {
    /// Negrito.
    pub bold: bool,
    /// Itálico.
    pub italic: bool,
}

/// Um caractere da grade: a coluna onde começa, quantas células ocupa (1 ou 2) e o texto (o
/// caractere com os acentos combinados que vierem depois dele).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellText<'a> {
    /// Coluna da primeira célula.
    pub col: u16,
    /// Células ocupadas: 2 para os caracteres largos.
    pub width: u8,
    /// O caractere, com os acentos combinados.
    pub text: &'a str,
}

/// Glifos de uma fonte já posicionados, com a fonte guardada (para gravar e desenhar depois).
#[derive(Clone, Debug)]
pub struct OwnedGlyphRun {
    /// Fonte dos glifos.
    pub font: FontData,
    /// Tamanho em pixels.
    pub font_size: f32,
    /// Coordenadas de fontes variáveis (vazio nas estáticas).
    pub normalized_coords: Vec<i16>,
    /// Glifos já posicionados.
    pub glyphs: Vec<Glyph>,
}

impl OwnedGlyphRun {
    /// O trecho no formato do `Canvas`.
    pub fn as_run(&self) -> GlyphRun<'_> {
        GlyphRun {
            font: &self.font,
            font_size: self.font_size,
            normalized_coords: &self.normalized_coords,
            glyphs: &self.glyphs,
            skew: 0.0,
        }
    }
}

/// Medidas da célula da Cascadia Mono em `font_size` pixels, lidas das tabelas da fonte (sem
/// montar o [`Fonts`]: o widget usa para saber quantas colunas e linhas cabem).
pub fn cell_metrics(font_size: f32) -> CellMetrics {
    use skrifa::MetadataProvider;
    use skrifa::instance::{LocationRef, Size};

    let font = skrifa::FontRef::new(CASCADIA_MONO[0]).expect("a Cascadia Mono embutida é válida");
    let size = Size::new(font_size);
    let metrics = font.metrics(size, LocationRef::default());
    let advance = font
        .charmap()
        .map('0')
        .and_then(|gid| {
            font.glyph_metrics(size, LocationRef::default())
                .advance_width(gid)
        })
        .unwrap_or(font_size * 0.6);
    let ascent = metrics.ascent;
    let descent = -metrics.descent;
    let width = advance.round().max(1.0);
    let height = (ascent + descent).ceil().max(1.0);
    // A sobra de altura fica metade em cima e metade embaixo.
    let baseline = ((height - ascent - descent) / 2.0 + ascent).round();
    let thickness = metrics
        .underline
        .map_or(1.0, |u| u.thickness)
        .round()
        .max(1.0);
    let underline_offset = metrics.underline.map_or(descent / 2.0, |u| -u.offset);
    let underline = (baseline + underline_offset.max(1.0))
        .round()
        .min(height - thickness);
    let strikeout = metrics
        .strikeout
        .map_or(ascent * 0.3, |s| s.offset)
        .max(1.0);
    let strikeout = (baseline - strikeout).round();
    CellMetrics {
        font_size,
        width,
        height,
        baseline,
        underline,
        strikeout,
        thickness,
    }
}

impl Fonts {
    /// Medidas da célula da Cascadia Mono em `font_size` pixels ([`cell_metrics`]).
    pub fn cell_metrics(&mut self, font_size: f32) -> CellMetrics {
        cell_metrics(font_size)
    }

    /// Glifos de `cells`, todos de uma linha da grade e com o mesmo estilo, com `origin` no
    /// canto superior esquerdo da coluna 0. Cada caractere fica centralizado nas suas células,
    /// qualquer que seja a fonte que o desenhe; os espaços não geram glifos.
    pub fn shape_cells(
        &mut self,
        cells: &[CellText<'_>],
        style: CellStyle,
        metrics: &CellMetrics,
        origin: Point,
    ) -> Vec<OwnedGlyphRun> {
        let mut text = String::new();
        let mut starts = Vec::with_capacity(cells.len());
        for cell in cells {
            starts.push(text.len());
            text.push_str(cell.text);
        }
        if text.is_empty() {
            return Vec::new();
        }
        let layout = self.mono_layout(&text, metrics.font_size, style);
        let (ox, oy) = (origin.x as f32, origin.y as f32);
        let baseline_y = oy + metrics.baseline;
        let mut out: Vec<OwnedGlyphRun> = Vec::new();
        for line in layout.lines() {
            for run in line.runs() {
                let mut glyphs = Vec::new();
                for cluster in run.visual_clusters() {
                    if cluster.is_space_or_nbsp() {
                        continue;
                    }
                    let start = cluster.text_range().start;
                    let index = starts.partition_point(|&s| s <= start).saturating_sub(1);
                    let cell = cells[index];
                    let span = f32::from(cell.width.max(1)) * metrics.width;
                    let mut x =
                        ox + f32::from(cell.col) * metrics.width + (span - cluster.advance()) / 2.0;
                    for glyph in cluster.glyphs() {
                        glyphs.push(Glyph {
                            id: glyph.id,
                            x: x + glyph.x,
                            y: baseline_y + glyph.y,
                        });
                        x += glyph.advance;
                    }
                }
                if glyphs.is_empty() {
                    continue;
                }
                out.push(OwnedGlyphRun {
                    font: run.font().clone(),
                    font_size: run.font_size(),
                    normalized_coords: run
                        .normalized_coords()
                        .iter()
                        .map(|c| c.to_bits())
                        .collect(),
                    glyphs,
                });
            }
        }
        out
    }

    /// Caracteres de `text` que nenhuma fonte da grade tem.
    pub fn missing_cell_glyphs(&mut self, text: &str, font_size: f32) -> Vec<char> {
        let layout = crate::TextLayout {
            layout: self.mono_layout(text, font_size, CellStyle::default()),
            runs: Vec::new(),
        };
        layout.missing_chars(text)
    }

    fn mono_layout(&mut self, text: &str, font_size: f32, style: CellStyle) -> parley::Layout<u32> {
        let mut builder = self
            .layout_cx
            .ranged_builder(&mut self.font_cx, text, 1.0, true);
        builder.push_default(StyleProperty::FontFamily(FontFamily::List(Cow::Borrowed(
            MONO_FAMILIES,
        ))));
        builder.push_default(StyleProperty::FontSize(font_size));
        builder.push_default(StyleProperty::LineHeight(LineHeight::MetricsRelative(1.0)));
        builder.push_default(StyleProperty::FontWeight(if style.bold {
            FontWeight::BOLD
        } else {
            FontWeight::NORMAL
        }));
        if style.italic {
            builder.push_default(StyleProperty::FontStyle(FontStyle::Italic));
        }
        let mut layout = builder.build(text);
        layout.break_all_lines(None);
        layout
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn celula_tem_pixels_inteiros() {
        let mut fonts = Fonts::new();
        let m = fonts.cell_metrics(16.0);
        assert_eq!(m.width, m.width.round());
        assert_eq!(m.height, m.height.round());
        assert!((8.0..=11.0).contains(&m.width), "{m:?}");
        assert!((17.0..=22.0).contains(&m.height), "{m:?}");
        assert!(m.baseline < m.height && m.underline < m.height);
        assert!(m.strikeout < m.baseline);
    }

    #[test]
    fn cada_caractere_na_sua_coluna() {
        let mut fonts = Fonts::new();
        let m = fonts.cell_metrics(16.0);
        let cells: Vec<CellText> = ["l", "M", "i", "W"]
            .iter()
            .enumerate()
            .map(|(col, text)| CellText {
                col: col as u16 * 2,
                width: 1,
                text,
            })
            .collect();
        let runs = fonts.shape_cells(&cells, CellStyle::default(), &m, Point::ZERO);
        let glyphs: Vec<&Glyph> = runs.iter().flat_map(|r| &r.glyphs).collect();
        assert_eq!(glyphs.len(), 4);
        assert!(glyphs.iter().all(|g| g.id != 0));
        for (i, g) in glyphs.iter().enumerate() {
            let celula = i as f32 * 2.0 * m.width;
            assert!((g.x - celula).abs() < 1.0, "glifo {i} em {}", g.x);
            assert_eq!(g.y, m.baseline);
        }
    }

    #[test]
    fn negrito_e_italico_trocam_a_fonte() {
        let mut fonts = Fonts::new();
        let m = fonts.cell_metrics(16.0);
        let cell = [CellText {
            col: 0,
            width: 1,
            text: "a",
        }];
        let fontes: Vec<u64> = [(false, false), (true, false), (false, true), (true, true)]
            .into_iter()
            .map(|(bold, italic)| {
                let runs = fonts.shape_cells(&cell, CellStyle { bold, italic }, &m, Point::ZERO);
                runs[0].font.data.id()
            })
            .collect();
        for i in 0..fontes.len() {
            for j in i + 1..fontes.len() {
                assert_ne!(fontes[i], fontes[j], "estilos {i} e {j} na mesma fonte");
            }
        }
    }

    #[test]
    fn espacos_nao_geram_glifos() {
        let mut fonts = Fonts::new();
        let m = fonts.cell_metrics(16.0);
        let cells = [
            CellText {
                col: 0,
                width: 1,
                text: " ",
            },
            CellText {
                col: 1,
                width: 1,
                text: "x",
            },
        ];
        let runs = fonts.shape_cells(&cells, CellStyle::default(), &m, Point::new(10.0, 5.0));
        let glyphs: Vec<&Glyph> = runs.iter().flat_map(|r| &r.glyphs).collect();
        assert_eq!(glyphs.len(), 1);
        assert!((glyphs[0].x - 10.0 - m.width).abs() < 1.0);
        assert_eq!(glyphs[0].y, 5.0 + m.baseline);
    }

    #[test]
    fn cascadia_tem_os_desenhos_de_caixa() {
        let mut fonts = Fonts::new();
        let texto = "─│┌┐└┘├┤┬┴┼═║╔╗╚╝█▄▀░▒▓ Ação ç ~ ^ ` ← → ↑ ↓ ✓";
        assert_eq!(fonts.missing_cell_glyphs(texto, 16.0), Vec::<char>::new());
    }

    #[cfg(feature = "cjk")]
    #[test]
    fn largo_fica_centralizado_em_duas_celulas() {
        let mut fonts = Fonts::new();
        let m = fonts.cell_metrics(16.0);
        let cells = [
            CellText {
                col: 0,
                width: 2,
                text: "日",
            },
            CellText {
                col: 2,
                width: 1,
                text: "a",
            },
        ];
        let runs = fonts.shape_cells(&cells, CellStyle::default(), &m, Point::ZERO);
        assert_eq!(runs.len(), 2, "a Noto e a Cascadia");
        let largo = runs[0].glyphs[0];
        assert!(largo.id != 0);
        // A Noto Sans JP tem avanço de 1 em: sobra o mesmo dos dois lados.
        assert!((largo.x - (2.0 * m.width - 16.0) / 2.0).abs() < 0.01);
        assert!((runs[1].glyphs[0].x - 2.0 * m.width).abs() < 1.0);
    }
}
