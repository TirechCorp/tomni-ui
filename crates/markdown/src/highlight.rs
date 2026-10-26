//! O realce do Markdown no `TextEditor`.

use std::ops::Range;

use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag};
use tomni_ui_core::{StyleRange, ThemeColor, Token};

use crate::parse::options;

/// A cor de uma faixa.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Tint {
    #[default]
    Normal,
    Accent,
    Secondary,
}

impl Tint {
    fn color(self) -> Option<ThemeColor> {
        match self {
            Self::Normal => None,
            Self::Accent => Some(Token::AccentTextFillColorPrimary.into()),
            Self::Secondary => Some(Token::TextFillColorSecondary.into()),
        }
    }
}

/// O estilo de um byte do texto.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Attr {
    bold: bool,
    italic: bool,
    strike: bool,
    tint: Tint,
    shaded: bool,
}

/// Uma pilha de camadas sobre o texto: cada chamada põe o estilo na faixa, e a cor e o fundo
/// da última a passar valem. No fim, as faixas saem sem sobreposição, que o editor espera.
struct Layers {
    attrs: Vec<Attr>,
}

impl Layers {
    fn new(len: usize) -> Self {
        Self {
            attrs: vec![Attr::default(); len],
        }
    }

    fn apply(&mut self, range: Range<usize>, f: impl Fn(&mut Attr)) {
        let end = range.end.min(self.attrs.len());
        let start = range.start.min(end);
        for attr in &mut self.attrs[start..end] {
            f(attr);
        }
    }

    fn tint(&mut self, range: Range<usize>, tint: Tint) {
        self.apply(range, |a| a.tint = tint);
    }

    fn into_ranges(self) -> Vec<StyleRange> {
        let mut out = Vec::new();
        let mut start = 0;
        while start < self.attrs.len() {
            let attr = self.attrs[start];
            let mut end = start + 1;
            while end < self.attrs.len() && self.attrs[end] == attr {
                end += 1;
            }
            if attr != Attr::default() {
                out.push(StyleRange {
                    range: start..end,
                    bold: attr.bold,
                    italic: attr.italic,
                    strike: attr.strike,
                    color: attr.tint.color(),
                    background: attr.shaded.then(|| Token::SubtleFillColorSecondary.into()),
                });
            }
            start = end;
        }
        out
    }
}

/// As faixas de estilo de `source` para o [`TextEditor::highlight`]: títulos em negrito na cor
/// de destaque, negrito, itálico, riscado, código com fundo, marcadores de lista e de tarefa e
/// links na cor de destaque, e as cercas, os `#`, `>` e `|` na cor secundária (a terciária não passa
/// do contraste AA sobre o fundo do editor). As faixas não se
/// sobrepõem.
///
/// ```
/// use tomni_ui_markdown::highlight;
///
/// let faixas = highlight("# Título\n\nUm **forte**.");
/// assert!(faixas[0].bold);
/// assert_eq!(faixas[0].range.start, 0);
/// ```
///
/// [`TextEditor::highlight`]: tomni_ui_core::TextEditor::highlight
pub fn highlight(source: &str) -> Vec<StyleRange> {
    let mut layers = Layers::new(source.len());
    // Primeiro a estrutura (cores de fundo do bloco), depois o texto corrido por cima.
    let events: Vec<(Event<'_>, Range<usize>)> = Parser::new_ext(source, options())
        .into_offset_iter()
        .collect();
    for (event, range) in &events {
        match event {
            Event::Start(Tag::Heading { .. }) => {
                layers.apply(range.clone(), |a| {
                    a.bold = true;
                    a.tint = Tint::Accent;
                });
                let marks = source[range.clone()]
                    .bytes()
                    .take_while(|&b| b == b'#')
                    .count();
                layers.tint(range.start..range.start + marks, Tint::Secondary);
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                layers.apply(range.clone(), |a| {
                    a.tint = Tint::Secondary;
                    a.shaded = true;
                });
                if matches!(kind, CodeBlockKind::Fenced(_)) {
                    fence_lines(source, range, &mut layers);
                }
            }
            Event::Start(Tag::BlockQuote(_)) => quote_marks(source, range, &mut layers),
            Event::Start(Tag::Item) => {
                let marker = source[range.clone()]
                    .find(char::is_whitespace)
                    .unwrap_or(range.len());
                layers.tint(range.start..range.start + marker, Tint::Accent);
            }
            Event::TaskListMarker(_) => layers.tint(range.clone(), Tint::Accent),
            Event::Rule => layers.tint(range.clone(), Tint::Secondary),
            Event::Start(Tag::Table(_)) => table_marks(source, range, &mut layers),
            _ => {}
        }
    }
    for (event, range) in &events {
        match event {
            Event::Start(Tag::Strong) => layers.apply(range.clone(), |a| a.bold = true),
            Event::Start(Tag::Emphasis) => layers.apply(range.clone(), |a| a.italic = true),
            Event::Start(Tag::Strikethrough) => layers.apply(range.clone(), |a| a.strike = true),
            Event::Start(Tag::Link { .. } | Tag::Image { .. }) => {
                layers.tint(range.clone(), Tint::Accent);
            }
            Event::Code(_) => layers.apply(range.clone(), |a| {
                a.tint = Tint::Secondary;
                a.shaded = true;
            }),
            _ => {}
        }
    }
    layers.into_ranges()
}

/// As linhas de abrir e de fechar a cerca de um bloco de código.
fn fence_lines(source: &str, range: &Range<usize>, layers: &mut Layers) {
    let text = &source[range.clone()];
    let is_fence = |line: &str| {
        let line = line.trim_start();
        line.starts_with("```") || line.starts_with("~~~")
    };
    let first_end = text.find('\n').unwrap_or(text.len());
    if is_fence(&text[..first_end]) {
        layers.tint(range.start..range.start + first_end, Tint::Secondary);
    }
    let trimmed = text.trim_end_matches(['\n', '\r']);
    let last_start = trimmed.rfind('\n').map_or(0, |i| i + 1);
    if last_start > first_end && is_fence(&trimmed[last_start..]) {
        layers.tint(
            range.start + last_start..range.start + trimmed.len(),
            Tint::Secondary,
        );
    }
}

/// Os `>` do começo de cada linha da citação.
fn quote_marks(source: &str, range: &Range<usize>, layers: &mut Layers) {
    let mut offset = range.start;
    for line in source[range.clone()].split_inclusive('\n') {
        for (i, b) in line.bytes().enumerate() {
            match b {
                b'>' => layers.tint(offset + i..offset + i + 1, Tint::Secondary),
                b' ' | b'\t' => {}
                _ => break,
            }
        }
        offset += line.len();
    }
}

/// Os `|` da tabela e a linha de divisão do cabeçalho.
fn table_marks(source: &str, range: &Range<usize>, layers: &mut Layers) {
    let mut offset = range.start;
    for (n, line) in source[range.clone()].split_inclusive('\n').enumerate() {
        if n == 1 {
            let body = line.trim_end_matches(['\n', '\r']);
            layers.tint(offset..offset + body.len(), Tint::Secondary);
        } else {
            for (i, b) in line.bytes().enumerate() {
                if b == b'|' {
                    layers.tint(offset + i..offset + i + 1, Tint::Secondary);
                }
            }
        }
        offset += line.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cor(range: &StyleRange) -> Option<Token> {
        let color = range.color?;
        Token::ALL
            .into_iter()
            .find(|t| ThemeColor::from(*t) == color)
    }

    fn em(faixas: &[StyleRange], texto: &str, trecho: &str) -> Option<StyleRange> {
        let start = texto.find(trecho)?;
        faixas
            .iter()
            .find(|f| f.range.start <= start && f.range.end >= start + trecho.len())
            .cloned()
    }

    #[test]
    fn titulo_negrito_e_cercas() {
        let texto = "# Título\n\n**forte** e *itálico* e ~~riscado~~\n\n```rust\nlet x = 1;\n```\n";
        let faixas = highlight(texto);
        let marca = faixas.iter().find(|f| f.range == (0..1)).unwrap();
        assert_eq!(cor(marca), Some(Token::TextFillColorSecondary));
        let titulo = em(&faixas, texto, "Título").unwrap();
        assert!(titulo.bold);
        assert_eq!(cor(&titulo), Some(Token::AccentTextFillColorPrimary));
        assert!(em(&faixas, texto, "forte").unwrap().bold);
        assert!(em(&faixas, texto, "itálico").unwrap().italic);
        assert!(em(&faixas, texto, "riscado").unwrap().strike);
        let codigo = em(&faixas, texto, "let x").unwrap();
        assert_eq!(cor(&codigo), Some(Token::TextFillColorSecondary));
        assert!(codigo.background.is_some());
        let cerca = em(&faixas, texto, "```rust").unwrap();
        assert_eq!(cor(&cerca), Some(Token::TextFillColorSecondary));
    }

    #[test]
    fn marcadores_links_citacao_e_tabela() {
        let texto = "- [ ] fazer\n- item\n\n> citado\n\n[ver](http://x)\n\n| a | b |\n|---|---|\n| 1 | 2 |\n";
        let faixas = highlight(texto);
        for trecho in ["-", "[ ]", "ver", ">"] {
            let faixa = em(&faixas, texto, trecho).unwrap_or_else(|| panic!("{trecho}"));
            assert!(faixa.color.is_some(), "{trecho}");
        }
        let segundo = texto.find("- item").unwrap();
        let marcador = faixas
            .iter()
            .find(|f| f.range.contains(&segundo))
            .expect("marcador do segundo item");
        assert_eq!(cor(marcador), Some(Token::AccentTextFillColorPrimary));
        let barra = em(&faixas, texto, "|").unwrap();
        assert_eq!(cor(&barra), Some(Token::TextFillColorSecondary));
        let divisao = em(&faixas, texto, "---").unwrap();
        assert_eq!(cor(&divisao), Some(Token::TextFillColorSecondary));
    }

    #[test]
    fn faixas_sem_sobreposicao_e_dentro_do_texto() {
        let texto = "# **negrito** `código` [link](u) áé\n\n> - **a**\n";
        let faixas = highlight(texto);
        let mut fim = 0;
        for faixa in &faixas {
            assert!(faixa.range.start >= fim, "{faixas:?}");
            assert!(faixa.range.end <= texto.len());
            assert!(texto.is_char_boundary(faixa.range.start));
            assert!(texto.is_char_boundary(faixa.range.end));
            fim = faixa.range.end;
        }
        assert!(highlight("").is_empty());
        assert!(highlight("só texto").is_empty());
    }
}
