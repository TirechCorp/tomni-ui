//! O `MarkdownView`: o texto em Markdown desenhado com widgets da lib.

use std::cell::Cell;
use std::fmt;
use std::path::PathBuf;
use std::rc::Rc;

use pulldown_cmark::Alignment;
use tomni_ui_core::{
    Align, Border, Button, CheckBox, Common, Density, Direction, Event, EventCx, Flex, Insets,
    IntoProp, Look, Prop, RichText, RichTextVisual, ScrollViewer, Slot, Span, Symbol, Token,
    Widget, clipboard,
};
use tomni_ui_reactive::prelude::*;

use crate::parse::{Block, Inlines, Item, Part, parse};

/// Entre dois blocos do documento.
const BLOCK_GAP: f64 = 12.0;
/// Entre dois itens de uma lista.
const ITEM_GAP: f64 = 4.0;
/// O recuo de cada nível de lista, que é a largura do marcador.
const LIST_INDENT: f64 = 20.0;
/// A largura da barra da citação.
const QUOTE_BAR: f64 = 3.0;
/// O marcador dos itens, em todos os níveis: a Selawik só tem o `•` (não tem `◦` nem `▪`), e o
/// recuo já mostra o nível.
const BULLET: &str = "•";

type OnLink = Rc<dyn Fn(String)>;
type OnTask = Rc<dyn Fn(usize, bool)>;
type OnCopy = Rc<dyn Fn(String)>;
type FenceFn = Rc<dyn Fn(FenceCx) -> Box<dyn Widget>>;
type ActionFn = Rc<dyn Fn(FenceCx)>;
type ImageFn = Rc<dyn Fn(ImageCx) -> Option<Box<dyn Widget>>>;

/// O que a função de [`MarkdownView::fence`] recebe: o bloco de código que ela desenha.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FenceCx {
    /// A linguagem do bloco (a primeira palavra depois da cerca), como foi escrita.
    pub lang: String,
    /// O código, sem a quebra de linha do fim.
    pub text: String,
    /// O número do bloco de código no documento, desde 0.
    pub index: usize,
    /// O id que o bloco teria: `<id>-bloco-<n>`.
    pub id: String,
}

/// O que a função de [`MarkdownView::image`] recebe: a imagem que ela desenha.
#[derive(Clone)]
pub struct ImageCx {
    /// O endereço da imagem, como está escrito.
    pub url: String,
    /// O texto alternativo.
    pub alt: String,
    /// O título, se houver.
    pub title: Option<String>,
    /// O número da imagem no documento, desde 0.
    pub index: usize,
    /// O id da imagem: `<id>-imagem-<n>`.
    pub id: String,
    fallback: Rc<Fallback>,
}

/// O que o padrão de uma imagem precisa.
struct Fallback {
    id: String,
    prefix: String,
    link_index: usize,
    label: String,
    dest: String,
    on_link: Option<OnLink>,
}

impl ImageCx {
    /// O desenho padrão da imagem: o texto alternativo como link para o endereço (pelo
    /// `on_link` do `MarkdownView`); sem texto alternativo, o próprio endereço. Serve a quem
    /// usa o [`MarkdownView::image`] e não conseguiu carregar a imagem.
    pub fn fallback(&self) -> Box<dyn Widget> {
        Box::new(self.fallback_text())
    }

    /// O mesmo desenho de [`ImageCx::fallback`], como o `RichText` que ele é: serve para pôr o
    /// padrão dentro de um `Show` (que pede uma view, não um `Box<dyn Widget>`), por exemplo
    /// enquanto a imagem carrega e depois falha.
    pub fn fallback_text(&self) -> RichText {
        let f = &self.fallback;
        let mut text = RichText::new(vec![Span::new(f.label.clone()).link(f.dest.clone())])
            .wrap()
            .link_ids(f.prefix.clone(), f.link_index)
            .id(f.id.clone());
        if let Some(on_link) = &f.on_link {
            let on_link = Rc::clone(on_link);
            text = text.on_link(move |url| on_link(url));
        }
        text
    }
}

impl fmt::Debug for ImageCx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImageCx")
            .field("url", &self.url)
            .field("alt", &self.alt)
            .field("title", &self.title)
            .field("index", &self.index)
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// As funções do app, que o desenho chama.
#[derive(Default)]
struct Config {
    on_link: Option<OnLink>,
    on_task: Option<OnTask>,
    on_copy: Option<OnCopy>,
    fences: Vec<(String, FenceFn)>,
    /// As ações extras dos blocos de código: o rótulo, o ícone e a função.
    actions: Vec<(String, Symbol, ActionFn)>,
    image: Option<ImageFn>,
}

/// Um contêiner de widgets em caixa, na coluna ou na linha: o `Column` e o `Row` do core só
/// aceitam o tipo concreto.
struct Group {
    common: Common,
    flex: Flex,
    children: Vec<Box<dyn Widget>>,
    /// Um bloco de cima do documento ([`Builder::unit`]), com a árvore de layout dele.
    boundary: bool,
}

impl Group {
    fn new(direction: Direction) -> Self {
        Self {
            common: Common::default(),
            flex: Flex {
                direction,
                ..Flex::default()
            },
            children: Vec::new(),
            boundary: false,
        }
    }

    fn column() -> Self {
        Self::new(Direction::Column)
    }

    fn row() -> Self {
        Self::new(Direction::Row)
    }

    fn gap(mut self, gap: f64) -> Self {
        self.flex.gap = gap;
        self
    }

    fn align(mut self, align: Align) -> Self {
        self.flex.align = align;
        self
    }

    fn grow(mut self, grow: f64) -> Self {
        self.common.grow = grow;
        self
    }

    fn with(mut self, child: Box<dyn Widget>) -> Self {
        self.children.push(child);
        self
    }

    fn with_all(mut self, children: Vec<Box<dyn Widget>>) -> Self {
        self.children.extend(children);
        self
    }
}

impl Widget for Group {
    fn kind(&self) -> &'static str {
        match self.flex.direction {
            Direction::Column => "column",
            Direction::Row => "row",
        }
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        match self.flex.direction {
            Direction::Column => Look::Column,
            Direction::Row => Look::Row,
        }
    }

    fn flex(&self) -> Option<Flex> {
        Some(self.flex)
    }

    fn layout_boundary(&self) -> bool {
        self.boundary
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        std::mem::take(&mut self.children)
    }
}

/// O Markdown como widgets: títulos, parágrafos, listas, tarefas, citações, réguas, tabelas,
/// blocos de código, links e imagens.
///
/// ```
/// use tomni_ui_markdown::MarkdownView;
///
/// let nota = MarkdownView::new("# Título\n\nUm parágrafo com **negrito** e um [link](https://x.y).")
///     .id("nota")
///     .on_link(|url| println!("abrir {url}"));
/// ```
///
/// Não rola sozinho: ponha num `ScrollViewer`. Monta os widgets de novo a cada mudança do texto
/// (um signal ou uma closure acompanham a mudança).
///
/// O desenho segue a escala tipográfica do Fluent: título 1 em `title`, título 2 em
/// `subtitle`, título 3 em `body_large` em negrito e os outros em `body_strong`; parágrafos no
/// `body`, com 12 px entre os blocos. Listas recuam 20 px por nível, com os marcadores `•` ou o
/// número; uma tarefa (`- [ ] x`) é uma `CheckBox` que chama
/// [`on_task`](MarkdownView::on_task) (sem ele, fica desabilitada). Citação tem uma barra de 3
/// px à esquerda (o WinUI não tem `ControlStrongStrokeColorDefault` no tema da lib: a barra usa
/// `TextFillColorTertiary`) e texto secundário. Tabela tem borda de 1 px, cabeçalho com fundo e
/// rola na horizontal se for larga. Bloco de código usa a Cascadia Mono sobre
/// `SubtleFillColorSecondary`, sem quebra de linha (rola na horizontal), com um botão "Copiar"
/// no canto. HTML aparece como texto. Imagens em títulos e em células de tabela viram links.
///
/// Os ids, para os testes e as ferramentas, contam desde 0 no documento todo, e o `<id>` é o
/// do `MarkdownView` (ou `markdown`): `<id>-bloco-<n>` em cada bloco de código,
/// `<id>-copiar-<n>` no botão dele, `<id>-acao-<k>-<n>` nos de [`MarkdownView::code_action`], `<id>-tarefa-<n>` em cada tarefa, `<id>-link-<n>` em cada
/// link e `<id>-imagem-<n>` em cada imagem. Toda imagem reserva também um número de link, para
/// o padrão ([`ImageCx::fallback`]) ter o id dele; se o app desenha a imagem por conta própria,
/// o número fica sem uso.
///
/// O `MarkdownView` **nunca** lê arquivo nem rede: quem desenha as imagens é o app
/// ([`MarkdownView::image`]).
pub struct MarkdownView {
    common: Common,
    source: Prop<String>,
    config: Config,
    files: Option<Box<dyn Fn(Vec<PathBuf>)>>,
    density: Rc<Cell<Density>>,
}

impl MarkdownView {
    /// Mostra o Markdown de `source` (texto fixo, closure ou signal).
    pub fn new<M>(source: impl IntoProp<String, M>) -> Self {
        Self {
            common: Common::default(),
            source: source.into_prop(),
            config: Config::default(),
            files: None,
            density: Rc::new(Cell::new(Density::default())),
        }
    }

    /// Id estável, usado em testes, no MCP e no lint, e prefixo dos ids de dentro.
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.common.id = Some(id.into());
        self
    }

    /// Parte do eixo principal do pai que o widget ocupa.
    pub fn grow(mut self, grow: impl Into<f64>) -> Self {
        self.common.grow = grow.into();
        self
    }

    /// Largura fixa, em pixels.
    pub fn width(mut self, width: impl Into<f64>) -> Self {
        self.common.width = Some(width.into());
        self
    }

    /// Largura máxima, em pixels.
    pub fn max_width(mut self, width: impl Into<f64>) -> Self {
        self.common.max_width = Some(width.into());
        self
    }

    /// Se aparece (valor fixo, closure ou signal).
    pub fn visible<M>(mut self, visible: impl IntoProp<bool, M>) -> Self {
        self.common.visible = Some(visible.into_prop());
        self
    }

    /// Roda `f` com o destino do link clicado ou aberto com Enter. Sem ele, o clique não faz
    /// nada.
    pub fn on_link(mut self, f: impl Fn(String) + 'static) -> Self {
        self.config.on_link = Some(Rc::new(f));
        self
    }

    /// Roda `f(offset, marcada)` quando o usuário alterna uma tarefa: `offset` é o byte do `[`
    /// da tarefa no texto (o `markdown::edit::toggle_task` troca o marcador a partir dele) e
    /// `marcada` é o novo estado. Sem ele, as tarefas ficam desabilitadas.
    pub fn on_task(mut self, f: impl Fn(usize, bool) + 'static) -> Self {
        self.config.on_task = Some(Rc::new(f));
        self
    }

    /// Roda `f` com o código do bloco quando o usuário aperta "Copiar". O padrão é gravar no
    /// clipboard.
    pub fn on_copy(mut self, f: impl Fn(String) + 'static) -> Self {
        self.config.on_copy = Some(Rc::new(f));
        self
    }

    /// Desenha com `f` os blocos de código cercados cuja linguagem é `lang`, no lugar do bloco
    /// de código.
    pub fn fence(mut self, lang: &str, f: impl Fn(FenceCx) -> Box<dyn Widget> + 'static) -> Self {
        self.config.fences.push((lang.to_owned(), Rc::new(f)));
        self
    }

    /// Põe em cada bloco de código comum um botão de ícone `icon` (com `label` como nome e dica),
    /// ao lado do "Copiar". Apertar chama `f` com o bloco: a linguagem, o código e o número. Pode
    /// ser chamado mais de uma vez; os botões vêm na ordem das chamadas, antes do "Copiar", com os
    /// ids `<id>-acao-<k>-<n>` (`k` é a ordem da ação e `n` o número do bloco). Blocos que o
    /// [`MarkdownView::fence`] desenha ficam de fora: quem os desenha põe os próprios botões.
    pub fn code_action(
        mut self,
        label: impl Into<String>,
        icon: Symbol,
        f: impl Fn(FenceCx) + 'static,
    ) -> Self {
        self.config.actions.push((label.into(), icon, Rc::new(f)));
        self
    }

    /// Chama `f` para cada imagem. `Some(widget)` desenha o widget; `None` desenha o padrão,
    /// que é o texto alternativo como link para o endereço (e o que acontece sem esta função).
    /// Uma imagem que divide o parágrafo com texto sai do fluxo e vem como bloco logo depois
    /// dele; se `f` devolve `None`, ela fica no texto, como link.
    pub fn image(mut self, f: impl Fn(ImageCx) -> Option<Box<dyn Widget>> + 'static) -> Self {
        self.config.image = Some(Rc::new(f));
        self
    }

    /// Com isto, o `MarkdownView` consome os arquivos soltos sobre ele e chama `f` com os
    /// caminhos. Sem, o evento sobe para o widget de cima.
    pub fn on_files_dropped(mut self, f: impl Fn(Vec<PathBuf>) + 'static) -> Self {
        self.files = Some(Box::new(f));
        self
    }
}

impl Widget for MarkdownView {
    fn kind(&self) -> &'static str {
        "markdown"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::Column
    }

    fn flex(&self) -> Option<Flex> {
        Some(Flex {
            direction: Direction::Column,
            gap: BLOCK_GAP,
            ..Flex::default()
        })
    }

    fn set_density(&mut self, density: Density) {
        self.density.set(density);
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        let body = Body {
            common: Common::default(),
            source: self.source.clone(),
            config: Rc::new(std::mem::take(&mut self.config)),
            id: self
                .common
                .id
                .clone()
                .unwrap_or_else(|| "markdown".to_owned()),
            density: Rc::clone(&self.density),
            shown: None,
            units: Vec::new(),
            recount: signal(0),
        };
        vec![Box::new(body)]
    }

    fn handle(&mut self, event: &Event, _cx: &mut EventCx) -> bool {
        match (event, &self.files) {
            (Event::FilesDropped { paths }, Some(f)) => {
                f(paths.clone());
                true
            }
            _ => false,
        }
    }
}

impl fmt::Debug for MarkdownView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MarkdownView")
            .field("common", &self.common)
            .finish_non_exhaustive()
    }
}

/// O fragmento com os blocos do documento, um filho por bloco. Quando o texto muda, só os
/// blocos que mudaram são montados de novo: os iguais, no começo e no fim, ficam com os
/// widgets (e o layout medido) que tinham. Digitar num documento grande refaz um bloco, não o
/// documento.
struct Body {
    common: Common,
    source: Prop<String>,
    config: Rc<Config>,
    id: String,
    density: Rc<Cell<Density>>,
    /// O texto e o alvo mínimo (da densidade) dos blocos de agora.
    shown: Option<(String, u64)>,
    /// Os blocos de agora, na ordem dos filhos.
    units: Vec<Unit>,
    /// Muda quando a conta dos contadores de um bloco novo errou: os blocos depois dele são
    /// montados de novo com os números certos.
    recount: Signal<u64>,
}

/// Um bloco montado: o bloco, os contadores com que foi montado e os do fim dele.
struct Unit {
    block: Block,
    start: Counters,
    /// Os contadores do fim, que a montagem grava.
    end: Rc<Cell<Counters>>,
}

impl Widget for Body {
    fn kind(&self) -> &'static str {
        "markdown-body"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::Fragment
    }

    fn fragment(&self) -> bool {
        true
    }

    fn track_children(&self) {
        let _ = self.source.get();
        let _ = self.recount.get();
    }

    fn reconcile(&mut self) -> Option<Vec<Slot>> {
        let source = self.source.get();
        let min_target = self.density.get().min_target();
        let key = (source, min_target.to_bits());
        // Uma conta que errou (veja o `recount`) deixa um bloco com números que não continuam
        // os do anterior.
        let stale = self
            .units
            .iter()
            .zip(self.units.iter().skip(1))
            .any(|(a, b)| a.end.get() != b.start);
        if self.shown.as_ref() == Some(&key) && !stale {
            return None;
        }
        let blocks = parse(&key.0);
        let same_target = self.shown.as_ref().is_some_and(|(_, t)| *t == key.1);
        self.shown = Some(key);
        let old = std::mem::take(&mut self.units);
        let images_drawn = self.config.image.is_some();
        // Os iguais do começo (com os mesmos números) e os do fim (os números são conferidos
        // abaixo): o meio é montado de novo.
        let mut prefix = 0;
        let mut suffix = 0;
        if same_target {
            let mut n = Counters::default();
            while prefix < blocks.len().min(old.len())
                && old[prefix].block == blocks[prefix]
                && old[prefix].start == n
            {
                n = old[prefix].end.get();
                prefix += 1;
            }
            while suffix < (blocks.len() - prefix).min(old.len() - prefix)
                && old[old.len() - 1 - suffix].block == blocks[blocks.len() - 1 - suffix]
            {
                suffix += 1;
            }
        }
        let mut slots = Vec::with_capacity(blocks.len());
        let mut n = Counters::default();
        let new_len = blocks.len();
        let tail = new_len - suffix;
        let old_len = old.len();
        let mut old = old.into_iter().map(Some).collect::<Vec<_>>();
        for (i, block) in blocks.into_iter().enumerate() {
            let previous = if i < prefix {
                Some(i)
            } else if i >= tail {
                Some(old_len - (new_len - i))
            } else {
                None
            };
            if let Some(j) = previous
                && let Some(unit) = old[j].take_if(|u| u.start == n)
            {
                n = unit.end.get();
                self.units.push(unit);
                slots.push(Slot::Keep(j));
                continue;
            }
            let start = n;
            let predicted = start.after(&block, images_drawn);
            let end = Rc::new(Cell::new(predicted));
            let unit_end = Rc::clone(&end);
            let config = Rc::clone(&self.config);
            let id = self.id.clone();
            let built = block.clone();
            let recount = self.recount;
            slots.push(Slot::New(Box::new(move || {
                let mut builder = Builder::new(&config, id, min_target, start);
                let widget = builder.unit(&built);
                if builder.n != predicted {
                    // A função de imagem deixou uma imagem no texto: os números dos blocos
                    // depois deste mudam.
                    unit_end.set(builder.n);
                    recount.set(recount.get_untracked().wrapping_add(1));
                }
                widget
            })));
            self.units.push(Unit { block, start, end });
            n = predicted;
        }
        Some(slots)
    }
}

/// Quanto cada tipo de texto escolhe da escala tipográfica.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Style {
    Body,
    Title,
    Subtitle,
    BodyLarge,
    BodyStrong,
}

/// Os contadores do documento, que numeram os ids de dentro: blocos de código, tarefas,
/// links, imagens, textos com link e tabelas.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Counters {
    blocks: usize,
    tasks: usize,
    links: usize,
    images: usize,
    texts: usize,
    tables: usize,
}

impl Counters {
    /// Os contadores depois de `block`, sem montar os widgets. Só os links de um parágrafo
    /// que mistura texto e imagem dependem do que a função de imagem do app devolve (a imagem
    /// que ela não desenha fica no texto, como link): aqui, com `images_drawn`, a conta supõe
    /// que ela desenha todas. O [`Body`] confere a conta com a montagem de verdade.
    fn after(mut self, block: &Block, images_drawn: bool) -> Self {
        match block {
            Block::Heading(level, inlines) => {
                self.text(&spans_of(inlines, *level == 3));
            }
            Block::Paragraph(inlines) => {
                let images = inlines
                    .parts
                    .iter()
                    .filter(|p| matches!(p, Part::Image(_)))
                    .count();
                self.images += images;
                self.links += images;
                if !inlines.only_images() {
                    let spans: Vec<Span> = inlines
                        .parts
                        .iter()
                        .filter(|p| !images_drawn || matches!(p, Part::Text { .. }))
                        .map(|p| span_of(p, false))
                        .collect();
                    if spans.iter().any(|s| !s.text.trim().is_empty()) {
                        self.text(&spans);
                    }
                }
            }
            Block::Code { .. } => self.blocks += 1,
            Block::Quote(blocks) => {
                for block in blocks {
                    self = self.after(block, images_drawn);
                }
            }
            Block::List { items, .. } => {
                for item in items {
                    if item.task.is_some() {
                        self.tasks += 1;
                    }
                    for block in &item.blocks {
                        self = self.after(block, images_drawn);
                    }
                }
            }
            Block::Rule | Block::Html(_) => {}
            Block::Table { head, rows, .. } => {
                let empty = Inlines::default();
                for (column, title) in head.iter().enumerate() {
                    self.text(&spans_of(title, false));
                    for row in rows {
                        self.text(&spans_of(row.get(column).unwrap_or(&empty), false));
                    }
                }
                self.tables += 1;
            }
        }
        self
    }

    /// Um texto corrido com `spans`: os links dele e, se houver algum, o id do texto.
    fn text(&mut self, spans: &[Span]) {
        let links = link_count(spans);
        self.links += links;
        if links > 0 {
            self.texts += 1;
        }
    }
}

/// Quantos links um texto corrido com `spans` tem, na numeração do `RichText`.
fn link_count(spans: &[Span]) -> usize {
    RichTextVisual {
        spans: spans.to_vec(),
        ..RichTextVisual::default()
    }
    .links()
    .len()
}

/// Monta os widgets de um documento, contando os blocos, as tarefas, os links e as imagens.
struct Builder<'a> {
    config: &'a Config,
    id: String,
    min_target: f64,
    /// Os contadores, que continuam os dos blocos de antes.
    n: Counters,
    /// Dentro de uma citação, o texto é secundário.
    secondary: bool,
}

impl<'a> Builder<'a> {
    fn new(config: &'a Config, id: String, min_target: f64, n: Counters) -> Self {
        Self {
            config,
            id,
            min_target,
            n,
            secondary: false,
        }
    }

    /// O widget de um bloco do documento. Um parágrafo com imagens fora do texto vira uma
    /// coluna com o texto e as imagens; um que não desenha nada, uma coluna oculta, que não
    /// ocupa espaço nem soma o espaço entre os blocos.
    ///
    /// Um bloco que é um contêiner (lista, citação, código, tabela) fica numa coluna com a
    /// árvore de layout dela ([`Widget::layout_boundary`]): digitar num bloco refaz o layout
    /// só dele, mesmo quando quem está em volta mede a pré-visualização em várias larguras.
    fn unit(&mut self, block: &Block) -> Box<dyn Widget> {
        let mut out = Vec::new();
        self.block(block, &mut out);
        if out.len() == 1 && out[0].flex().is_none() {
            return out.pop().expect("tem um");
        }
        let mut group = Group::column().gap(BLOCK_GAP).with_all(out);
        group.boundary = true;
        if group.children.is_empty() {
            group.common.visible = Some(Prop::Value(false));
        }
        Box::new(group)
    }

    fn blocks_of(&mut self, blocks: &[Block]) -> Vec<Box<dyn Widget>> {
        let mut out = Vec::new();
        for block in blocks {
            self.block(block, &mut out);
        }
        out
    }

    fn block(&mut self, block: &Block, out: &mut Vec<Box<dyn Widget>>) {
        match block {
            Block::Heading(level, inlines) => {
                let (style, bold) = match level {
                    1 => (Style::Title, false),
                    2 => (Style::Subtitle, false),
                    3 => (Style::BodyLarge, true),
                    _ => (Style::BodyStrong, false),
                };
                let spans = spans_of(inlines, bold);
                out.push(Box::new(self.rich(spans, style, true)));
            }
            Block::Paragraph(inlines) => self.paragraph(inlines, out),
            Block::Code { lang, text } => out.push(self.code(lang.as_deref(), text)),
            Block::Quote(blocks) => {
                let was = std::mem::replace(&mut self.secondary, true);
                let content = self.blocks_of(blocks);
                self.secondary = was;
                let bar = Border::new()
                    .background(Token::TextFillColorTertiary)
                    .width(QUOTE_BAR);
                out.push(Box::new(Group::row().gap(12.0).with(Box::new(bar)).with(
                    Box::new(Group::column().gap(BLOCK_GAP).grow(1.0).with_all(content)),
                )));
            }
            Block::List { start, items } => out.push(Box::new(self.list(*start, items))),
            Block::Rule => out.push(Box::new(
                Border::new()
                    .background(Token::DividerStrokeColorDefault)
                    .height(1),
            )),
            Block::Table { aligns, head, rows } => out.push(self.table(aligns, head, rows)),
            Block::Html(html) => {
                let spans = vec![Span::new(html.trim_end().to_owned())];
                out.push(Box::new(self.rich(spans, Style::Body, true)));
            }
        }
    }

    /// Um texto corrido, com a numeração dos links do documento.
    fn rich(&mut self, spans: Vec<Span>, style: Style, wrap: bool) -> RichText {
        let first = self.n.links;
        self.n.links += link_count(&spans);
        let has_links = self.n.links > first;
        let mut text = RichText::new(spans).link_ids(self.id.clone(), first);
        if has_links {
            text = text.id(format!("{}-texto-{}", self.id, self.n.texts));
            self.n.texts += 1;
        }
        if wrap {
            text = text.wrap();
        }
        text = match style {
            Style::Body => text,
            Style::Title => text.title(),
            Style::Subtitle => text.subtitle(),
            Style::BodyLarge => text.body_large(),
            Style::BodyStrong => text.body_strong(),
        };
        if self.secondary {
            text = text.secondary();
        }
        if let Some(on_link) = &self.config.on_link {
            let on_link = Rc::clone(on_link);
            text = text.on_link(move |url| on_link(url));
        }
        text
    }

    /// Um parágrafo: o texto e, fora dele, as imagens que o `image` desenha.
    fn paragraph(&mut self, inlines: &Inlines, out: &mut Vec<Box<dyn Widget>>) {
        let mut images: Vec<(ImageCx, Option<Box<dyn Widget>>)> = Vec::new();
        for part in &inlines.parts {
            if let Part::Image(image) = part {
                let index = self.n.images;
                self.n.images += 1;
                let link_index = self.n.links;
                self.n.links += 1;
                let dest = image.link.clone().unwrap_or_else(|| image.url.clone());
                let label = if image.alt.is_empty() {
                    image.url.clone()
                } else {
                    image.alt.clone()
                };
                let cx = ImageCx {
                    url: image.url.clone(),
                    alt: image.alt.clone(),
                    title: image.title.clone(),
                    index,
                    id: format!("{}-imagem-{index}", self.id),
                    fallback: Rc::new(Fallback {
                        id: format!("{}-imagem-{index}", self.id),
                        prefix: self.id.clone(),
                        link_index,
                        label,
                        dest,
                        on_link: self.config.on_link.clone(),
                    }),
                };
                let drawn = self.config.image.as_ref().and_then(|f| f(cx.clone()));
                images.push((cx, drawn));
            }
        }
        if inlines.only_images() {
            for (cx, drawn) in images {
                out.push(drawn.unwrap_or_else(|| cx.fallback()));
            }
            return;
        }
        let mut spans = Vec::new();
        let mut apart = Vec::new();
        let mut images = images.into_iter();
        for part in &inlines.parts {
            match part {
                Part::Text { .. } => spans.push(span_of(part, false)),
                Part::Image(_) => {
                    let Some((cx, drawn)) = images.next() else {
                        continue;
                    };
                    match drawn {
                        Some(widget) => apart.push(widget),
                        None => spans.push(
                            Span::new(cx.fallback.label.clone()).link(cx.fallback.dest.clone()),
                        ),
                    }
                }
            }
        }
        if spans.iter().any(|s| !s.text.trim().is_empty()) {
            out.push(Box::new(self.rich(spans, Style::Body, true)));
        }
        out.extend(apart);
    }

    fn code(&mut self, lang: Option<&str>, text: &str) -> Box<dyn Widget> {
        let index = self.n.blocks;
        self.n.blocks += 1;
        let text = text.trim_end_matches(['\n', '\r']).to_owned();
        let id = format!("{}-bloco-{index}", self.id);
        let word = lang.and_then(|l| l.split_whitespace().next());
        if let Some(word) = word
            && let Some((_, fence)) = self.config.fences.iter().find(|(name, _)| name == word)
        {
            return fence(FenceCx {
                lang: word.to_owned(),
                text,
                index,
                id,
            });
        }
        let on_copy = self.config.on_copy.clone();
        let copied = text.clone();
        let copy = Button::icon_only(Symbol::Copy)
            .label("Copiar")
            .id(format!("{}-copiar-{index}", self.id))
            .on_click(move || match &on_copy {
                Some(f) => f(copied.clone()),
                None => clipboard::write(copied.clone()),
            });
        let actions: Vec<Box<dyn Widget>> = self
            .config
            .actions
            .iter()
            .enumerate()
            .map(|(k, (label, icon, f))| {
                let f = Rc::clone(f);
                let cx = FenceCx {
                    lang: word.unwrap_or_default().to_owned(),
                    text: text.clone(),
                    index,
                    id: id.clone(),
                };
                Box::new(
                    Button::icon_only(*icon)
                        .label(label.clone())
                        .id(format!("{}-acao-{k}-{index}", self.id))
                        .on_click(move || f(cx.clone())),
                ) as Box<dyn Widget>
            })
            .collect();
        let code = RichText::new(vec![Span::new(text)]).monospace().id(id);
        let scroll = ScrollViewer::horizontal()
            .id(format!("{}-rolagem-{index}", self.id))
            .grow(1)
            .child(code);
        Box::new(
            Border::new()
                .background(Token::SubtleFillColorSecondary)
                .corner_radius(4)
                .padding(Insets::symmetric(4.0, 8.0))
                .child(
                    Group::row()
                        .align(Align::Start)
                        .gap(if actions.is_empty() { 0.0 } else { 2.0 })
                        .with(Box::new(scroll))
                        .with_all(actions)
                        .with(Box::new(copy)),
                ),
        )
    }

    fn list(&mut self, start: Option<u64>, items: &[Item]) -> Group {
        let digits = start.map_or(1.0, |first| {
            (first + items.len() as u64).to_string().len() as f64
        });
        let mut list = Group::column().gap(ITEM_GAP);
        for (n, item) in items.iter().enumerate() {
            let task = item
                .task
                .map(|(checked, offset)| self.task(checked, offset));
            let marker: Box<dyn Widget> = match (&task, start) {
                (Some(_), _) => Box::new(Group::column()),
                (None, Some(first)) => Box::new(
                    RichText::new(vec![Span::new(format!("{}.", first + n as u64))])
                        .width(LIST_INDENT + 8.0 * (digits - 1.0).max(0.0)),
                ),
                (None, None) => Box::new(RichText::new(vec![Span::new(BULLET)]).width(LIST_INDENT)),
            };
            let content = Group::column()
                .gap(ITEM_GAP)
                .grow(1.0)
                .with_all(self.blocks_of(&item.blocks));
            let row = match task {
                Some(check) => Group::row()
                    .gap(4.0)
                    .align(Align::Center)
                    .with(check)
                    .with(Box::new(content)),
                None => Group::row()
                    .align(Align::Start)
                    .with(marker)
                    .with(Box::new(content)),
            };
            list = list.with(Box::new(row));
        }
        list
    }

    /// A caixa de uma tarefa: alternar chama o `on_task` com o byte do `[` e o novo estado.
    fn task(&mut self, checked: bool, offset: usize) -> Box<dyn Widget> {
        let index = self.n.tasks;
        self.n.tasks += 1;
        let state = signal(checked);
        let on_task = self.config.on_task.clone();
        if let Some(on_task) = on_task.clone() {
            let mut last = checked;
            effect(move || {
                let now = state.get();
                if now != last {
                    last = now;
                    untrack(|| on_task(offset, now));
                }
            });
        }
        Box::new(
            CheckBox::bind(state, String::new())
                .id(format!("{}-tarefa-{index}", self.id))
                .width(self.min_target)
                .height(self.min_target)
                .disabled(on_task.is_none()),
        )
    }

    fn table(
        &mut self,
        aligns: &[Alignment],
        head: &[Inlines],
        rows: &[Vec<Inlines>],
    ) -> Box<dyn Widget> {
        let divider = || Border::new().background(Token::DividerStrokeColorDefault);
        let mut table = Group::row().align(Align::Start);
        for (column, title) in head.iter().enumerate() {
            let align = match aligns.get(column) {
                Some(Alignment::Right) => Align::End,
                Some(Alignment::Center) => Align::Center,
                _ => Align::Start,
            };
            let mut cells = Group::column();
            let cell = |builder: &mut Self, inlines: &Inlines, style: Style| {
                let text = builder.rich(spans_of(inlines, false), style, false);
                Box::new(
                    Border::new()
                        .padding(Insets::symmetric(6.0, 12.0))
                        .child(Group::column().align(align).with(Box::new(text))),
                ) as Box<dyn Widget>
            };
            let header = cell(self, title, Style::BodyStrong);
            cells = cells.with(Box::new(
                Border::new()
                    .background(Token::SubtleFillColorSecondary)
                    .child(Group::column().with(header)),
            ));
            for row in rows {
                cells = cells.with(Box::new(divider().height(1)));
                let empty = Inlines::default();
                let inlines = row.get(column).unwrap_or(&empty);
                cells = cells.with(cell(self, inlines, Style::Body));
            }
            if column > 0 {
                table = table.with(Box::new(divider().width(1)));
            }
            table = table.with(Box::new(cells));
        }
        let index = self.n.tables;
        self.n.tables += 1;
        Box::new(
            ScrollViewer::horizontal()
                .id(format!("{}-tabela-{index}", self.id))
                .child(
                    Border::new()
                        .border(Token::DividerStrokeColorDefault, 1)
                        .corner_radius(4)
                        .child(table),
                ),
        )
    }
}

/// Os trechos de um texto corrido; uma imagem entra como o texto alternativo, em link.
fn spans_of(inlines: &Inlines, bold: bool) -> Vec<Span> {
    inlines
        .parts
        .iter()
        .map(|part| span_of(part, bold))
        .collect()
}

fn span_of(part: &Part, all_bold: bool) -> Span {
    match part {
        Part::Text {
            text,
            bold,
            italic,
            strike,
            code,
            link,
        } => Span {
            text: text.clone(),
            bold: *bold || all_bold,
            italic: *italic,
            strike: *strike,
            code: *code,
            link: link.clone(),
        },
        Part::Image(image) => Span {
            text: if image.alt.is_empty() {
                image.url.clone()
            } else {
                image.alt.clone()
            },
            bold: all_bold,
            link: Some(image.link.clone().unwrap_or_else(|| image.url.clone())),
            ..Span::default()
        },
    }
}
