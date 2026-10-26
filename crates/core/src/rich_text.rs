//! `RichText`: texto com trechos em negrito, itálico, riscado, código e links.

use std::fmt;
use std::ops::Range;

use kurbo::Point;

use crate::color::{ThemeColor, Token};
use crate::cx::EventCx;
use crate::event::{Cursor, Event, FocusSource, Key};
use crate::prop::{IntoProp, Prop};
use crate::text::TextRamp;
use crate::widget::{Common, Look, Widget, common_builders};

/// Um trecho de texto com o mesmo estilo.
///
/// ```
/// use tomni_ui_core::Span;
///
/// let trecho = Span::new("ver o manual").bold().link("https://exemplo.com/manual");
/// assert!(trecho.bold);
/// assert_eq!(trecho.link.as_deref(), Some("https://exemplo.com/manual"));
/// ```
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Span {
    /// O texto.
    pub text: String,
    /// Negrito.
    pub bold: bool,
    /// Itálico (inclinado por síntese: a Selawik não tem a face itálica).
    pub italic: bool,
    /// Riscado.
    pub strike: bool,
    /// Código: Cascadia Mono, com fundo discreto.
    pub code: bool,
    /// Destino do link; `Some` faz o trecho clicável, na cor accent e sublinhado ao hover.
    pub link: Option<String>,
}

impl Span {
    /// Trecho de texto comum.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Self::default()
        }
    }

    /// Em negrito.
    pub fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    /// Em itálico.
    pub fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    /// Riscado.
    pub fn strike(mut self) -> Self {
        self.strike = true;
        self
    }

    /// Como código.
    pub fn code(mut self) -> Self {
        self.code = true;
        self
    }

    /// Como link para `url`.
    pub fn link(mut self, url: impl Into<String>) -> Self {
        self.link = Some(url.into());
        self
    }
}

/// Um link do texto: o trecho (em bytes do texto inteiro) e o destino.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkRange {
    /// Faixa do link no texto concatenado dos trechos.
    pub range: Range<usize>,
    /// Destino.
    pub url: String,
}

/// Tudo o que o `Drawer` precisa saber do `RichText` para medi-lo e pintá-lo.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RichTextVisual {
    /// Os trechos, na ordem.
    pub spans: Vec<Span>,
    /// Tamanho e peso do texto comum.
    pub ramp: TextRamp,
    /// Se quebra linhas na largura disponível.
    pub wrap: bool,
    /// Máximo de linhas: a última termina em reticências quando o texto não cabe.
    pub max_lines: Option<u32>,
    /// Cor do texto comum. `None` é a herdada ou o `TextFillColorPrimary`.
    pub color: Option<ThemeColor>,
    /// O link sob o ponteiro (índice em [`RichTextVisual::links`]).
    pub hovered: Option<usize>,
    /// O link pressionado.
    pub pressed: Option<usize>,
    /// O link com o foco do teclado.
    pub focused: Option<usize>,
    /// Se o link com foco mostra o anel (foco vindo do teclado).
    pub focus_ring: bool,
    /// Prefixo dos ids dos links na árvore (`<prefixo>-link-<n>`).
    pub link_prefix: Option<String>,
    /// Número do primeiro link deste texto nos ids (um `MarkdownView` conta os links do
    /// documento inteiro).
    pub first_link: usize,
    /// Se o texto inteiro é em Cascadia Mono, sem o fundo do `Span::code` (um bloco de código).
    pub mono: bool,
}

impl RichTextVisual {
    /// O texto inteiro, sem os estilos.
    pub fn text(&self) -> String {
        self.spans.iter().map(|s| s.text.as_str()).collect()
    }

    /// Os links, em ordem: cada sequência de trechos seguidos com o mesmo destino é um só.
    pub fn links(&self) -> Vec<LinkRange> {
        let mut links: Vec<LinkRange> = Vec::new();
        let mut start = 0;
        let mut previous_linked = false;
        for span in &self.spans {
            let end = start + span.text.len();
            match &span.link {
                Some(url) => {
                    match links.last_mut() {
                        Some(last) if previous_linked && last.url == *url => last.range.end = end,
                        _ => links.push(LinkRange {
                            range: start..end,
                            url: url.clone(),
                        }),
                    }
                    previous_linked = true;
                }
                None => previous_linked = false,
            }
            start = end;
        }
        links
    }

    /// Id do link `n` na árvore, se há prefixo.
    pub fn link_id(&self, n: usize) -> Option<String> {
        self.link_prefix
            .as_ref()
            .map(|prefix| format!("{prefix}-link-{}", self.first_link + n))
    }
}

/// Texto com estilos por trecho e links, na rampa tipográfica do Fluent.
///
/// ```
/// use tomni_ui_core::{RichText, Span};
///
/// let texto = RichText::new(vec![
///     Span::new("Leia o "),
///     Span::new("manual").bold().link("https://exemplo.com"),
///     Span::new(" e rode "),
///     Span::new("ssh -V").code(),
///     Span::new("."),
/// ])
/// .id("aviso")
/// .wrap();
/// assert_eq!(texto.visual().text(), "Leia o manual e rode ssh -V.");
/// assert_eq!(texto.visual().links().len(), 1);
/// ```
///
/// Cada link é uma parada de Tab (Tab e Shift+Tab percorrem os links do texto antes de seguir)
/// e abre com Enter ou clique, por [`RichText::on_link`]. Na árvore das ferramentas, o texto
/// aparece com o conteúdo inteiro e cada link como filho de id `<id>-link-<n>` (n desde 0),
/// clicável pelo `TestApp::click`.
pub struct RichText {
    common: Common,
    spans: Prop<Vec<Span>>,
    ramp: TextRamp,
    wrap: bool,
    max_lines: Option<u32>,
    color: Option<ThemeColor>,
    on_link: Option<Box<dyn FnMut(String)>>,
    link_prefix: Option<String>,
    first_link: usize,
    mono: bool,
    hovered: Option<usize>,
    pressed: Option<usize>,
    focused: Option<usize>,
    focus: Option<FocusSource>,
}

impl RichText {
    /// Texto com os trechos `spans` (fixos, closure ou signal) no degrau [`TextRamp::Body`].
    pub fn new<M>(spans: impl IntoProp<Vec<Span>, M>) -> Self {
        Self {
            common: Common::default(),
            spans: spans.into_prop(),
            ramp: TextRamp::Body,
            wrap: false,
            max_lines: None,
            color: None,
            on_link: None,
            link_prefix: None,
            first_link: 0,
            mono: false,
            hovered: None,
            pressed: None,
            focused: None,
            focus: None,
        }
    }

    /// Degrau da rampa.
    pub fn ramp(mut self, ramp: TextRamp) -> Self {
        self.ramp = ramp;
        self
    }

    /// [`TextRamp::Caption`]: 12/16.
    pub fn caption(self) -> Self {
        self.ramp(TextRamp::Caption)
    }

    /// [`TextRamp::BodyStrong`]: 14/20 semibold.
    pub fn body_strong(self) -> Self {
        self.ramp(TextRamp::BodyStrong)
    }

    /// [`TextRamp::BodyLarge`]: 18/24.
    pub fn body_large(self) -> Self {
        self.ramp(TextRamp::BodyLarge)
    }

    /// [`TextRamp::Subtitle`]: 20/28 semibold.
    pub fn subtitle(self) -> Self {
        self.ramp(TextRamp::Subtitle)
    }

    /// [`TextRamp::Title`]: 28/36 semibold.
    pub fn title(self) -> Self {
        self.ramp(TextRamp::Title)
    }

    /// Quebra as linhas na largura que o pai der.
    pub fn wrap(mut self) -> Self {
        self.wrap = true;
        self
    }

    /// Quebra as linhas e mostra no máximo `lines` delas, com reticências na última.
    pub fn max_lines(mut self, lines: u32) -> Self {
        self.wrap = true;
        self.max_lines = Some(lines.max(1));
        self
    }

    /// Cor do texto comum: um [`Token`] do tema ou uma cor fixa.
    pub fn color(mut self, color: impl Into<ThemeColor>) -> Self {
        self.color = Some(color.into());
        self
    }

    /// Texto secundário (`TextFillColorSecondary`).
    pub fn secondary(self) -> Self {
        self.color(Token::TextFillColorSecondary)
    }

    /// Roda `f` com o destino do link clicado, ou aberto com Enter.
    pub fn on_link(mut self, f: impl FnMut(String) + 'static) -> Self {
        self.on_link = Some(Box::new(f));
        self
    }

    /// Ids dos links na árvore: `<prefix>-link-<first>`, `<prefix>-link-<first + 1>`… Sem isto,
    /// o prefixo é o `.id()` do texto e a conta começa em 0. O `MarkdownView` usa para numerar
    /// os links do documento inteiro.
    pub fn link_ids(mut self, prefix: impl Into<String>, first: usize) -> Self {
        self.link_prefix = Some(prefix.into());
        self.first_link = first;
        self
    }

    /// O texto todo em Cascadia Mono (a do terminal, com a feature `mono`; sem ela, a do texto
    /// comum), sem o fundo do [`Span::code`]: para um bloco de código que já tem fundo.
    pub fn monospace(mut self) -> Self {
        self.mono = true;
        self
    }

    /// Estado para o `Drawer`.
    pub fn visual(&self) -> RichTextVisual {
        RichTextVisual {
            spans: self.spans.get(),
            ramp: self.ramp,
            wrap: self.wrap,
            max_lines: self.max_lines,
            color: self.color,
            hovered: self.hovered,
            pressed: self.pressed,
            focused: self.focused,
            focus_ring: self.focus == Some(FocusSource::Keyboard),
            link_prefix: self.link_prefix.clone().or_else(|| self.common.id.clone()),
            first_link: self.first_link,
            mono: self.mono,
        }
    }

    fn links(&self) -> Vec<LinkRange> {
        self.visual().links()
    }

    /// O link sob o ponteiro: pede ao `Drawer` os retângulos dos links.
    fn link_at(&self, cx: &mut EventCx) -> Option<usize> {
        let rect = cx.rect();
        let point = cx.pointer();
        let local = Point::new(point.x - rect.x0, point.y - rect.y0);
        let width = self.wrap.then(|| rect.width() + 0.5);
        let visual = self.visual();
        cx.metrics()
            .rich_links(&visual, width)
            .iter()
            .position(|rects| rects.iter().any(|r| r.contains(local)))
    }

    fn open(&mut self, n: usize) {
        let url = self.links().get(n).map(|l| l.url.clone());
        if let (Some(url), Some(f)) = (url, &mut self.on_link) {
            f(url);
        }
    }
}

common_builders!(RichText);

impl Widget for RichText {
    fn kind(&self) -> &'static str {
        "rich_text"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::RichText(self.visual())
    }

    fn text(&self) -> Option<String> {
        Some(self.visual().text())
    }

    fn part_ids(&self) -> Vec<String> {
        let visual = self.visual();
        (0..visual.links().len())
            .filter_map(|n| visual.link_id(n))
            .collect()
    }

    fn interactive(&self) -> bool {
        !self.links().is_empty()
    }

    fn focusable(&self) -> bool {
        self.interactive()
    }

    fn captures_tab(&self) -> bool {
        self.interactive()
    }

    fn cursor(&self) -> Option<Cursor> {
        self.hovered.map(|_| Cursor::Hand)
    }

    fn handle(&mut self, event: &Event, cx: &mut EventCx) -> bool {
        let count = self.links().len();
        match *event {
            Event::PointerMove { kind, inside } => {
                self.hovered = if inside && kind.hovers() {
                    self.link_at(cx)
                } else {
                    None
                };
                true
            }
            Event::PointerDown { kind, .. } => {
                self.hovered = if kind.hovers() {
                    self.link_at(cx)
                } else {
                    None
                };
                self.pressed = self.link_at(cx);
                if self.pressed.is_some() {
                    self.focused = self.pressed;
                }
                true
            }
            Event::PointerUp { kind, inside } => {
                let over = if inside { self.link_at(cx) } else { None };
                self.hovered = over.filter(|_| kind.hovers());
                let Some(pressed) = self.pressed.take() else {
                    return false;
                };
                if over == Some(pressed) {
                    self.open(pressed);
                }
                true
            }
            Event::PointerCancel => {
                self.hovered = None;
                self.pressed = None;
                true
            }
            Event::KeyDown(Key::Tab) => {
                let Some(current) = self.focused else {
                    return false;
                };
                let next = if cx.modifiers().shift {
                    current.checked_sub(1)
                } else {
                    Some(current + 1).filter(|&n| n < count)
                };
                match next {
                    Some(n) => {
                        self.focused = Some(n);
                        true
                    }
                    None => false,
                }
            }
            Event::KeyDown(Key::Enter) if self.focus.is_some() => {
                if let Some(n) = self.focused {
                    self.open(n);
                }
                true
            }
            Event::FocusIn(source) => {
                self.focus = Some(source);
                let pointed = (source == FocusSource::Pointer)
                    .then(|| self.link_at(cx))
                    .flatten();
                self.focused =
                    pointed.or(if cx.modifiers().shift && source == FocusSource::Keyboard {
                        count.checked_sub(1)
                    } else {
                        (count > 0).then_some(0)
                    });
                true
            }
            Event::FocusOut => {
                self.focus = None;
                self.focused = None;
                true
            }
            _ => false,
        }
    }
}

impl fmt::Debug for RichText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RichText")
            .field("common", &self.common)
            .field("spans", &self.spans)
            .field("ramp", &self.ramp)
            .field("wrap", &self.wrap)
            .field("max_lines", &self.max_lines)
            .field("color", &self.color)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exemplo() -> RichText {
        RichText::new(vec![
            Span::new("a "),
            Span::new("b").bold().link("u1"),
            Span::new("c").link("u1"),
            Span::new(" d "),
            Span::new("e").link("u2"),
        ])
        .id("t")
    }

    #[test]
    fn links_agrupam_trechos_seguidos() {
        let visual = exemplo().visual();
        assert_eq!(visual.text(), "a bc d e");
        let links = visual.links();
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].range, 2..4);
        assert_eq!(links[0].url, "u1");
        assert_eq!(links[1].range, 7..8);
    }

    #[test]
    fn ids_dos_links() {
        let t = exemplo();
        assert_eq!(t.part_ids(), vec!["t-link-0", "t-link-1"]);
        let t = exemplo().link_ids("doc", 5);
        assert_eq!(t.part_ids(), vec!["doc-link-5", "doc-link-6"]);
        let sem_id = RichText::new(vec![Span::new("x").link("u")]);
        assert!(sem_id.part_ids().is_empty());
    }

    #[test]
    fn so_com_links_recebe_foco() {
        assert!(exemplo().interactive());
        assert!(exemplo().focusable());
        assert!(exemplo().captures_tab());
        let simples = RichText::new(vec![Span::new("x").bold()]);
        assert!(!simples.interactive());
        assert!(!simples.focusable());
        assert_eq!(simples.kind(), "rich_text");
        assert_eq!(simples.text().as_deref(), Some("x"));
    }

    #[test]
    fn look_e_quebra() {
        let t = exemplo().wrap().max_lines(2).caption();
        let Look::RichText(visual) = t.look() else {
            panic!("não é RichText");
        };
        assert_eq!(visual.max_lines, Some(2));
        assert_eq!(visual.ramp, TextRamp::Caption);
        assert!(t.look().reflows());
        assert!(!exemplo().look().reflows());
    }
}
