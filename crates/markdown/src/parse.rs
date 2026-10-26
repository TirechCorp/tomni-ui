//! O texto em Markdown vira uma árvore de blocos, que o `MarkdownView` desenha.

use std::iter::Peekable;
use std::ops::Range;

use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

/// As opções do `pulldown-cmark` que o crate usa: tabelas, tarefas e riscado.
pub(crate) fn options() -> Options {
    Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS | Options::ENABLE_STRIKETHROUGH
}

/// Uma imagem no meio do texto.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ImageRef {
    pub url: String,
    pub alt: String,
    pub title: Option<String>,
    /// O destino do link que envolve a imagem, se houver.
    pub link: Option<String>,
}

/// Um pedaço de um texto corrido: um trecho com estilo ou uma imagem.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Part {
    Text {
        text: String,
        bold: bool,
        italic: bool,
        strike: bool,
        code: bool,
        link: Option<String>,
    },
    Image(ImageRef),
}

/// Um texto corrido.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Inlines {
    pub parts: Vec<Part>,
}

impl Inlines {
    /// Se só há imagens (e espaços entre elas).
    pub(crate) fn only_images(&self) -> bool {
        let mut any = false;
        for part in &self.parts {
            match part {
                Part::Image(_) => any = true,
                Part::Text { text, .. } if text.trim().is_empty() => {}
                Part::Text { .. } => return false,
            }
        }
        any
    }

    /// O texto sem estilo (as imagens entram pelo texto alternativo).
    #[cfg(test)]
    pub(crate) fn plain(&self) -> String {
        self.parts
            .iter()
            .map(|part| match part {
                Part::Text { text, .. } => text.as_str(),
                Part::Image(image) => image.alt.as_str(),
            })
            .collect()
    }
}

/// Um item de lista.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Item {
    /// `Some((marcada, byte do '[' no texto))` numa tarefa.
    pub task: Option<(bool, usize)>,
    pub blocks: Vec<Block>,
}

/// Um bloco do documento.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Block {
    Heading(u8, Inlines),
    Paragraph(Inlines),
    Code {
        lang: Option<String>,
        text: String,
    },
    Quote(Vec<Block>),
    List {
        start: Option<u64>,
        items: Vec<Item>,
    },
    Rule,
    Table {
        aligns: Vec<Alignment>,
        head: Vec<Inlines>,
        rows: Vec<Vec<Inlines>>,
    },
    Html(String),
}

/// Os blocos de `source`.
pub(crate) fn parse(source: &str) -> Vec<Block> {
    let events: Vec<_> = Parser::new_ext(source, options())
        .into_offset_iter()
        .collect();
    let mut parser = Walker {
        events: events.into_iter().peekable(),
    };
    parser.blocks()
}

/// Os eventos já lidos (e clonáveis, para olhar dois à frente).
struct Walker<'a> {
    events: Peekable<std::vec::IntoIter<(Event<'a>, Range<usize>)>>,
}

/// O estilo em vigor dentro de um texto corrido.
#[derive(Clone, Default)]
struct Style {
    bold: bool,
    italic: bool,
    strike: bool,
    link: Option<String>,
}

impl<'a> Walker<'a> {
    /// Os blocos até o fim do contêiner (o `End` é consumido) ou do texto.
    fn blocks(&mut self) -> Vec<Block> {
        let mut blocks = Vec::new();
        while let Some((event, _)) = self.events.peek() {
            match event {
                Event::End(_) => {
                    self.events.next();
                    break;
                }
                Event::Start(tag) if is_block(tag) => {
                    let Some((Event::Start(tag), _)) = self.events.next() else {
                        break;
                    };
                    if let Some(block) = self.block(tag) {
                        blocks.push(block);
                    }
                }
                Event::Rule => {
                    self.events.next();
                    blocks.push(Block::Rule);
                }
                // Texto direto no item de uma lista compacta, sem parágrafo.
                _ => {
                    let inlines = self.inlines(false);
                    if inlines.parts.is_empty() {
                        // Evento que não vira nada: segue em frente sem travar.
                        self.events.next();
                    } else {
                        blocks.push(Block::Paragraph(inlines));
                    }
                }
            }
        }
        blocks
    }

    fn block(&mut self, tag: Tag<'a>) -> Option<Block> {
        match tag {
            Tag::Paragraph => Some(Block::Paragraph(self.inlines(true))),
            Tag::Heading { level, .. } => {
                Some(Block::Heading(level_number(level), self.inlines(true)))
            }
            Tag::BlockQuote(_) => Some(Block::Quote(self.blocks())),
            Tag::CodeBlock(kind) => {
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => info
                        .split_whitespace()
                        .next()
                        .map(str::to_owned)
                        .filter(|lang| !lang.is_empty()),
                    CodeBlockKind::Indented => None,
                };
                let mut text = String::new();
                for (event, _) in self.events.by_ref() {
                    match event {
                        Event::Text(t) | Event::Code(t) => text.push_str(&t),
                        Event::End(TagEnd::CodeBlock) => break,
                        _ => {}
                    }
                }
                if text.ends_with('\n') {
                    text.pop();
                }
                Some(Block::Code { lang, text })
            }
            Tag::HtmlBlock => {
                let mut text = String::new();
                for (event, _) in self.events.by_ref() {
                    match event {
                        Event::Html(t) | Event::Text(t) => text.push_str(&t),
                        Event::End(TagEnd::HtmlBlock) => break,
                        _ => {}
                    }
                }
                Some(Block::Html(text.trim_end().to_owned()))
            }
            Tag::List(start) => {
                let mut items = Vec::new();
                while let Some((event, _)) = self.events.peek() {
                    match event {
                        Event::Start(Tag::Item) => {
                            self.events.next();
                            items.push(self.item());
                        }
                        Event::End(_) => {
                            self.events.next();
                            break;
                        }
                        _ => {
                            self.events.next();
                        }
                    }
                }
                Some(Block::List { start, items })
            }
            Tag::Table(aligns) => Some(self.table(aligns)),
            _ => {
                // Um bloco que o crate não desenha (notas de rodapé, metadados): lê até o
                // fim para o resto do texto seguir certo.
                self.skip();
                None
            }
        }
    }

    /// Consome até o `End` que fecha o `Start` já lido.
    fn skip(&mut self) {
        let mut depth = 1;
        for (event, _) in self.events.by_ref() {
            match event {
                Event::Start(_) => depth += 1,
                Event::End(_) => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }
    }

    fn item(&mut self) -> Item {
        let mut first = None;
        let task = match self.events.peek() {
            Some((Event::TaskListMarker(checked), range)) => {
                let task = Some((*checked, range.start));
                self.events.next();
                task
            }
            // Numa lista com linha em branco entre os itens, o marcador vem dentro do parágrafo.
            Some((Event::Start(Tag::Paragraph), _)) => {
                let mut ahead = self.events.clone();
                ahead.next();
                match ahead.next() {
                    Some((Event::TaskListMarker(checked), range)) => {
                        self.events.next();
                        self.events.next();
                        first = Some(Block::Paragraph(self.inlines(true)));
                        Some((checked, range.start))
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        let mut blocks = self.blocks();
        if let Some(first) = first {
            blocks.insert(0, first);
        }
        Item { task, blocks }
    }

    fn table(&mut self, aligns: Vec<Alignment>) -> Block {
        let mut head = Vec::new();
        let mut rows = Vec::new();
        while let Some((event, _)) = self.events.next() {
            match event {
                Event::Start(Tag::TableHead) => head = self.cells(TagEnd::TableHead),
                Event::Start(Tag::TableRow) => rows.push(self.cells(TagEnd::TableRow)),
                Event::End(TagEnd::Table) => break,
                _ => {}
            }
        }
        Block::Table { aligns, head, rows }
    }

    /// As células até o `end` (a linha ou o cabeçalho).
    fn cells(&mut self, end: TagEnd) -> Vec<Inlines> {
        let mut cells = Vec::new();
        while let Some((event, _)) = self.events.next() {
            match event {
                Event::Start(Tag::TableCell) => cells.push(self.inlines(true)),
                // O cabeçalho do `pulldown-cmark` também pode trazer uma linha.
                Event::Start(Tag::TableRow) => cells.extend(self.cells(TagEnd::TableRow)),
                Event::End(e) if e == end => break,
                _ => {}
            }
        }
        cells
    }

    /// O texto corrido. Com `explicit`, até o `End` do bloco que o contém (consumido); sem
    /// ele, até o próximo bloco ou o fim do contêiner (que fica para quem chamou).
    fn inlines(&mut self, explicit: bool) -> Inlines {
        let mut out = Inlines::default();
        let mut stack: Vec<Style> = vec![Style::default()];
        while let Some((event, _)) = self.events.peek() {
            match event {
                Event::Start(Tag::Image { .. }) => {
                    let Some((
                        Event::Start(Tag::Image {
                            dest_url, title, ..
                        }),
                        _,
                    )) = self.events.next()
                    else {
                        break;
                    };
                    let alt = self.alt();
                    let link = stack.last().and_then(|s| s.link.clone());
                    out.parts.push(Part::Image(ImageRef {
                        url: dest_url.to_string(),
                        alt,
                        title: Some(title.to_string()).filter(|t| !t.is_empty()),
                        link,
                    }));
                }
                Event::Start(tag) if !is_inline(tag) => break,
                Event::End(end) if !is_inline_end(end) => {
                    if explicit {
                        self.events.next();
                    }
                    break;
                }
                Event::Rule | Event::TaskListMarker(_) if !explicit => break,
                _ => {
                    let Some((event, _)) = self.events.next() else {
                        break;
                    };
                    self.inline(event, &mut stack, &mut out);
                }
            }
        }
        out
    }

    /// O texto alternativo de uma imagem, até o `End` dela.
    fn alt(&mut self) -> String {
        let mut alt = String::new();
        let mut depth = 1;
        for (event, _) in self.events.by_ref() {
            match event {
                Event::Start(_) => depth += 1,
                Event::End(_) => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                Event::Text(t) | Event::Code(t) => alt.push_str(&t),
                Event::SoftBreak | Event::HardBreak => alt.push(' '),
                _ => {}
            }
        }
        alt
    }

    fn inline(&mut self, event: Event<'a>, stack: &mut Vec<Style>, out: &mut Inlines) {
        let current = stack.last().cloned().unwrap_or_default();
        let mut text = |text: String, code: bool| {
            if text.is_empty() {
                return;
            }
            out.parts.push(Part::Text {
                text,
                bold: current.bold,
                italic: current.italic,
                strike: current.strike,
                code,
                link: current.link.clone(),
            });
        };
        match event {
            Event::Text(t) => text(t.to_string(), false),
            Event::Code(t) => text(t.to_string(), true),
            // Quebra simples vira espaço, como no CommonMark; a dupla (dois espaços ou `\`)
            // vira quebra de linha.
            Event::SoftBreak => text(" ".to_owned(), false),
            Event::HardBreak => text("\n".to_owned(), false),
            // HTML fica como texto.
            Event::Html(t) | Event::InlineHtml(t) => text(t.to_string(), false),
            Event::FootnoteReference(t) => text(format!("[^{t}]"), false),
            Event::Start(Tag::Emphasis) => stack.push(Style {
                italic: true,
                ..current
            }),
            Event::Start(Tag::Strong) => stack.push(Style {
                bold: true,
                ..current
            }),
            Event::Start(Tag::Strikethrough) => stack.push(Style {
                strike: true,
                ..current
            }),
            Event::Start(Tag::Link { dest_url, .. }) => stack.push(Style {
                link: Some(dest_url.to_string()),
                ..current
            }),
            Event::End(_) if stack.len() > 1 => {
                stack.pop();
            }
            _ => {}
        }
    }
}

fn level_number(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// Se a tag abre um bloco (e não um trecho do texto corrido).
fn is_block(tag: &Tag<'_>) -> bool {
    !is_inline(tag)
}

fn is_inline(tag: &Tag<'_>) -> bool {
    matches!(
        tag,
        Tag::Emphasis
            | Tag::Strong
            | Tag::Strikethrough
            | Tag::Superscript
            | Tag::Subscript
            | Tag::Link { .. }
            | Tag::Image { .. }
    )
}

fn is_inline_end(end: &TagEnd) -> bool {
    matches!(
        end,
        TagEnd::Emphasis
            | TagEnd::Strong
            | TagEnd::Strikethrough
            | TagEnd::Superscript
            | TagEnd::Subscript
            | TagEnd::Link
            | TagEnd::Image
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(inlines: &Inlines) -> String {
        inlines.plain()
    }

    #[test]
    fn titulos_paragrafos_e_estilos() {
        let blocks = parse("# Título\n\nUm **forte**, *itálico*, ~~riscado~~ e `código`.\n");
        assert_eq!(blocks.len(), 2);
        let Block::Heading(1, titulo) = &blocks[0] else {
            panic!("{:?}", blocks[0]);
        };
        assert_eq!(text(titulo), "Título");
        let Block::Paragraph(p) = &blocks[1] else {
            panic!("{:?}", blocks[1]);
        };
        assert_eq!(text(p), "Um forte, itálico, riscado e código.");
        let estilos: Vec<(bool, bool, bool, bool)> = p
            .parts
            .iter()
            .filter_map(|part| match part {
                Part::Text {
                    bold,
                    italic,
                    strike,
                    code,
                    ..
                } => Some((*bold, *italic, *strike, *code)),
                Part::Image(_) => None,
            })
            .collect();
        assert!(estilos.contains(&(true, false, false, false)));
        assert!(estilos.contains(&(false, true, false, false)));
        assert!(estilos.contains(&(false, false, true, false)));
        assert!(estilos.contains(&(false, false, false, true)));
    }

    #[test]
    fn listas_com_tarefas_e_aninhamento() {
        let fonte = "- [ ] a\n- [x] b\n  - filho\n\n1. um\n2. dois\n";
        let blocks = parse(fonte);
        let Block::List { start: None, items } = &blocks[0] else {
            panic!("{:?}", blocks[0]);
        };
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].task, Some((false, 2)));
        assert_eq!(items[1].task, Some((true, 10)));
        assert_eq!(&fonte[2..5], "[ ]");
        assert_eq!(&fonte[10..13], "[x]");
        assert!(
            items[1]
                .blocks
                .iter()
                .any(|b| matches!(b, Block::List { .. }))
        );
        let Block::List {
            start: Some(1),
            items,
        } = &blocks[1]
        else {
            panic!("{:?}", blocks[1]);
        };
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn tarefas_em_lista_com_linha_em_branco_entre_os_itens() {
        let fonte = "- [x] um\n\n- [ ] dois\n\n- sem tarefa\n";
        let blocks = parse(fonte);
        let Block::List { items, .. } = &blocks[0] else {
            panic!("{:?}", blocks[0]);
        };
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].task, Some((true, 2)));
        assert_eq!(items[1].task, Some((false, 12)));
        assert_eq!(items[2].task, None);
        let Block::Paragraph(p) = &items[0].blocks[0] else {
            panic!("{:?}", items[0].blocks);
        };
        assert_eq!(text(p), "um");
    }

    #[test]
    fn codigo_citacao_regua_e_tabela() {
        let blocks = parse(
            "```rust x\nfn a() {}\n```\n\n> citado\n\n---\n\n| a | b |\n|:--|--:|\n| 1 | 2 |\n",
        );
        assert_eq!(
            blocks[0],
            Block::Code {
                lang: Some("rust".into()),
                text: "fn a() {}".into()
            }
        );
        assert!(matches!(&blocks[1], Block::Quote(b) if b.len() == 1));
        assert_eq!(blocks[2], Block::Rule);
        let Block::Table { aligns, head, rows } = &blocks[3] else {
            panic!("{:?}", blocks[3]);
        };
        assert_eq!(aligns, &[Alignment::Left, Alignment::Right]);
        assert_eq!(head.len(), 2);
        assert_eq!(rows.len(), 1);
        assert_eq!(text(&rows[0][1]), "2");
    }

    #[test]
    fn imagens_links_e_html() {
        let blocks = parse("![foto](a.png \"título\")\n\nVeja [aqui](http://x) <b>y</b>\n");
        let Block::Paragraph(p) = &blocks[0] else {
            panic!("{:?}", blocks[0]);
        };
        assert!(p.only_images());
        let Part::Image(image) = &p.parts[0] else {
            panic!()
        };
        assert_eq!(image.url, "a.png");
        assert_eq!(image.alt, "foto");
        assert_eq!(image.title.as_deref(), Some("título"));
        let Block::Paragraph(p) = &blocks[1] else {
            panic!("{:?}", blocks[1]);
        };
        assert!(!p.only_images());
        assert!(p.parts.iter().any(|part| matches!(
            part,
            Part::Text { link: Some(l), .. } if l == "http://x"
        )));
        assert!(text(p).contains("<b>"));
    }
}
