//! `TitleBar`: a barra de título desenhada pelo app, com os botões da janela, e o que ela
//! pede à plataforma (arrastar, maximizar, fechar).

use std::fmt;
use std::rc::Rc;

use kurbo::Point;
use tomni_ui_reactive::{Get, GetUntracked, Set, Signal, signal};

use crate::border::Border;
use crate::button::ButtonState;
use crate::color::{Color, ThemeColor, Token};
use crate::containers::Column;
use crate::cx::EventCx;
use crate::event::Event;
use crate::icon::Icon;
use crate::image::Image;
use crate::press::Pressable;
use crate::prop::{IntoProp, Prop};
use crate::rich_text::{RichText, Span};
use crate::symbol::Symbol;
use crate::text::Text;
use crate::widget::{
    Align, Common, Direction, Flex, Insets, Justify, Look, View, Widget, common_builders,
};

/// Altura da barra com conteúdo no meio ou à direita (a `TitleBar` do WinUI com `Content`).
pub const TITLE_BAR_HEIGHT: f64 = 48.0;
/// Altura da barra só com o ícone, o título e os botões.
pub const TITLE_BAR_COMPACT_HEIGHT: f64 = 32.0;
/// Largura de cada botão da janela (minimizar, maximizar e fechar), como no Windows 11.
pub const CAPTION_BUTTON_WIDTH: f64 = 46.0;
/// Largura do voltar e do ☰ da barra: 40 px mais a margem de 2 dos dois lados.
pub const TITLE_BAR_BUTTON_WIDTH: f64 = 44.0;
/// Faixa da borda da janela sem moldura que redimensiona.
pub const RESIZE_BORDER: f64 = 8.0;
/// Quanto o ponteiro anda com o botão pressionado na barra antes de a janela começar a ser
/// arrastada: um clique parado não arrasta, e o segundo clique do duplo clique chega.
pub const DRAG_SLOP: f64 = 4.0;

/// Espaço à esquerda do conteúdo da barra no macOS, onde ficam os três botões do sistema.
const MAC_TRAFFIC_LIGHTS: f64 = 78.0;
/// Área mínima de arrasto à direita do conteúdo (a coluna `DragRegion` do WinUI).
const MIN_DRAG_WIDTH: f64 = 48.0;

/// O que um widget pede à janela ([`EventCx::request_window`]). A plataforma atende depois
/// que o evento termina; no `TestApp`, um runner falso registra e simula.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WindowRequest {
    /// Minimiza.
    Minimize,
    /// Maximiza, ou restaura se já está maximizada.
    ToggleMaximize,
    /// Fecha.
    Close,
    /// Começa a arrastar a janela com o botão que está pressionado.
    Drag,
    /// Começa a redimensionar a janela pela borda ou pelo canto.
    Resize(ResizeEdge),
}

/// Borda ou canto da janela que redimensiona.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResizeEdge {
    /// Em cima.
    North,
    /// Embaixo.
    South,
    /// À direita.
    East,
    /// À esquerda.
    West,
    /// Canto de cima, à direita.
    NorthEast,
    /// Canto de cima, à esquerda.
    NorthWest,
    /// Canto de baixo, à direita.
    SouthEast,
    /// Canto de baixo, à esquerda.
    SouthWest,
}

/// Quem desenha a moldura da janela com uma [`TitleBar`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum WindowChrome {
    /// A janela não tem moldura: a barra desenha os botões da janela, arrasta e redimensiona
    /// (Windows e Linux, no X11 e no Wayland).
    #[default]
    Custom,
    /// macOS: o sistema desenha os três botões à esquerda, sobre a barra; ela deixa o lugar
    /// livre e não desenha os dela.
    MacOs,
    /// A plataforma não deixa tirar a moldura (Wayland sem `drag_window`, Android): a barra do
    /// sistema fica, e a do app aparece embaixo dela, sem os botões da janela e sem arrastar.
    System,
}

/// Estado da janela que a [`TitleBar`] mostra: se está ativa e maximizada e quem desenha a
/// moldura. A plataforma atualiza pelo `Ui`; os widgets da barra leem nos efeitos.
#[derive(Clone, Copy, Debug)]
pub struct WindowState {
    active: Signal<bool>,
    maximized: Signal<bool>,
    chrome: Signal<WindowChrome>,
    pane: Signal<Option<PaneLink>>,
}

impl WindowState {
    fn new() -> Self {
        Self {
            active: signal(true),
            maximized: signal(false),
            chrome: signal(WindowChrome::Custom),
            pane: signal(None),
        }
    }

    /// Se a janela tem o foco do sistema.
    pub fn active(&self) -> bool {
        self.active.get()
    }

    /// Se a janela está maximizada.
    pub fn maximized(&self) -> bool {
        self.maximized.get()
    }

    /// Quem desenha a moldura.
    pub fn chrome(&self) -> WindowChrome {
        self.chrome.get()
    }

    /// Muda [`WindowState::active`] (fora de um efeito).
    pub fn set_active(&self, active: bool) {
        if self.active.get_untracked() != active {
            self.active.set(active);
        }
    }

    /// Muda [`WindowState::maximized`] (fora de um efeito).
    pub fn set_maximized(&self, maximized: bool) {
        if self.maximized.get_untracked() != maximized {
            self.maximized.set(maximized);
        }
    }

    /// Muda [`WindowState::chrome`] (fora de um efeito).
    pub fn set_chrome(&self, chrome: WindowChrome) {
        if self.chrome.get_untracked() != chrome {
            self.chrome.set(chrome);
        }
    }

    /// Liga o painel de um `NavigationView` ao ☰ da barra: o ☰ do painel some, e o da barra
    /// abre e fecha o painel. A árvore liga quando monta os dois.
    pub fn link_pane(&self, link: PaneLink) {
        link.external.set(true);
        self.pane.set(Some(link));
    }

    /// Se a moldura é do app, sem a do sistema.
    fn custom(&self) -> bool {
        self.chrome.get_untracked() != WindowChrome::System
    }
}

/// Ligação entre o painel de um `NavigationView` e a [`TitleBar`] ([`WindowState::link_pane`]).
#[derive(Clone, Copy, Debug)]
pub struct PaneLink {
    open: Signal<bool>,
    external: Signal<bool>,
}

impl PaneLink {
    /// Ligação ao painel aberto em `open`; `external` passa a `true` quando o ☰ vai para a
    /// barra.
    pub(crate) fn new(open: Signal<bool>, external: Signal<bool>) -> Self {
        Self { open, external }
    }
}

/// Altura do selo de marca da barra ([`TitleBar::title_badge`]).
const BADGE_HEIGHT: f64 = 20.0;
/// Espaço entre a borda do selo e o texto, dos dois lados.
const BADGE_PADDING: f64 = 8.0;
/// Raio dos cantos do selo: o `ControlCornerRadius` do WinUI.
const BADGE_CORNER_RADIUS: f64 = 4.0;
/// Lado do ícone da barra, por símbolo ou por imagem.
const ICON_SIDE: f64 = 16.0;

/// Um selo de marca no lugar do texto do título: partes de texto com peso e cor próprios, sobre
/// um fundo de cor fixa. As cores não seguem o tema nem a janela ativa ou inativa: o selo é a
/// marca e fica igual no claro e no escuro.
///
/// ```
/// use tomni_ui_core::{Color, TitleBadge, TitleBar};
///
/// let selo = TitleBadge::new(Color::hex(0x063A66))
///     .part("Meu", Color::WHITE)
///     .bold_part(" App", Color::hex(0x4CC2FF));
/// let barra = TitleBar::new("Meu App").title_badge(selo);
/// ```
///
/// O texto das partes é o do `caption` (12/16), centralizado na vertical num retângulo de
/// 20 px de altura. As partes ficam lado a lado sem espaço entre elas: o espaço, se quiser,
/// vai no texto. O lint confere o contraste de cada parte com o fundo do selo.
#[derive(Clone, Debug, PartialEq)]
pub struct TitleBadge {
    parts: Vec<(String, bool, Color)>,
    background: Color,
}

impl TitleBadge {
    /// Selo sem partes, com o fundo `background`.
    pub fn new(background: Color) -> Self {
        Self {
            parts: Vec::new(),
            background,
        }
    }

    /// Acrescenta um trecho de texto comum na cor `color`.
    pub fn part(mut self, text: impl Into<String>, color: Color) -> Self {
        self.parts.push((text.into(), false, color));
        self
    }

    /// Acrescenta um trecho de texto em negrito na cor `color`.
    pub fn bold_part(mut self, text: impl Into<String>, color: Color) -> Self {
        self.parts.push((text.into(), true, color));
        self
    }

    /// O texto do selo inteiro, as partes emendadas.
    pub fn text(&self) -> String {
        self.parts
            .iter()
            .map(|(text, _, _)| text.as_str())
            .collect()
    }

    /// O selo como widget: o retângulo com as partes dentro.
    fn into_widget(self, state: WindowState) -> Box<dyn Widget> {
        let mut row = Part::new(state);
        row.common.height = Some(BADGE_HEIGHT);
        for (text, bold, color) in self.parts {
            if bold {
                row.children.push(Box::new(
                    RichText::new(vec![Span::new(text).bold()])
                        .caption()
                        .color(color),
                ));
            } else {
                row.children
                    .push(Box::new(Text::new(text).caption().color(color)));
            }
        }
        let mut badge = Border::new()
            .background(self.background)
            .corner_radius(BADGE_CORNER_RADIUS)
            .padding(Insets::symmetric(0.0, BADGE_PADDING))
            .child(row);
        badge = badge.height(BADGE_HEIGHT);
        Box::new(badge)
    }
}

/// O ícone da barra: um símbolo da fonte de ícones ou uma imagem (PNG ou SVG).
#[derive(Clone, Copy, Debug)]
enum TitleIcon {
    Symbol(Symbol),
    Image(&'static [u8]),
}

/// Barra de título desenhada pelo app, como a `TitleBar` do WinUI 3: ícone, título,
/// subtítulo, o voltar, o ☰ do `NavigationView`, conteúdo no meio (uma busca) e à direita, e
/// os botões da janela. Vai no [`Form::title_bar`](crate::Form::title_bar); a janela perde a
/// moldura do sistema.
///
/// ```
/// use tomni_ui_core::{Form, Symbol, TitleBar};
///
/// let tela = Form::new("Biblioteca").title_bar(
///     TitleBar::new("Biblioteca").subtitle("Prévia").icon(Symbol::Library),
/// );
/// ```
///
/// A altura é de 48 px com [`TitleBar::content`] ou [`TitleBar::right_header`] e de 32 sem
/// eles. Com a janela inativa, o título fica com a cor terciária e o ícone e o conteúdo, meio
/// apagados. Arrastar a barra move a janela, e o duplo clique maximiza e restaura.
///
/// As partes ganham ids derivados do da barra (`barra-de-titulo` se não tiver um):
/// `-voltar`, `-alternar` (o ☰), `-minimizar`, `-maximizar` e `-fechar`.
pub struct TitleBar {
    common: Common,
    title: Prop<String>,
    subtitle: Option<Prop<String>>,
    icon: Option<TitleIcon>,
    badge: Option<TitleBadge>,
    back: Option<Prop<bool>>,
    back_enabled: Option<Prop<bool>>,
    on_back: Option<Box<dyn FnMut()>>,
    content: Option<Box<dyn Widget>>,
    right: Option<Box<dyn Widget>>,
    state: WindowState,
    /// Onde o botão foi pressionado na barra, até o arrasto começar.
    drag: Option<Point>,
}

impl TitleBar {
    /// Barra com o título `title` (texto fixo, closure ou signal). Não muda o título da janela,
    /// que é o do `Form`.
    pub fn new<M>(title: impl IntoProp<String, M>) -> Self {
        Self {
            common: Common {
                id: Some("barra-de-titulo".to_owned()),
                ..Common::default()
            },
            title: title.into_prop(),
            subtitle: None,
            icon: None,
            badge: None,
            back: None,
            back_enabled: None,
            on_back: None,
            content: None,
            right: None,
            state: WindowState::new(),
            drag: None,
        }
    }

    /// Subtítulo, depois do título, na cor secundária (uma versão, "Prévia").
    pub fn subtitle<M>(mut self, subtitle: impl IntoProp<String, M>) -> Self {
        self.subtitle = Some(subtitle.into_prop());
        self
    }

    /// Ícone do app, à esquerda do título, com 16 px.
    pub fn icon(mut self, icon: Symbol) -> Self {
        self.icon = Some(TitleIcon::Symbol(icon));
        self
    }

    /// Ícone do app por imagem (um PNG ou SVG embutido, como `include_bytes!("icone.svg")`), à
    /// esquerda do título, com 16 × 16 px, no mesmo lugar do [`icon`](Self::icon). Se os dois
    /// forem dados, vale o último chamado.
    pub fn icon_image(mut self, bytes: &'static [u8]) -> Self {
        self.icon = Some(TitleIcon::Image(bytes));
        self
    }

    /// Troca o texto do título por um selo de marca ([`TitleBadge`]). O `title` da barra
    /// continua sendo o nome dela na árvore e para o leitor de tela; o selo é só o desenho, e o
    /// subtítulo continua ao lado dele. Com a janela inativa o selo não se apaga, ao contrário do
    /// texto do título: é a marca, e as cores dela não mudam. (Adaptação: o WinUI 3 não tem
    /// selo na `TitleBar`.)
    pub fn title_badge(mut self, badge: TitleBadge) -> Self {
        self.badge = Some(badge);
        self
    }

    /// Mostra o botão voltar (valor fixo, closure ou signal), à esquerda de tudo.
    pub fn back<M>(mut self, visible: impl IntoProp<bool, M>) -> Self {
        self.back = Some(visible.into_prop());
        self
    }

    /// Habilita o botão voltar (valor fixo, closure ou signal); o padrão é habilitado.
    pub fn back_enabled<M>(mut self, enabled: impl IntoProp<bool, M>) -> Self {
        self.back_enabled = Some(enabled.into_prop());
        self
    }

    /// O que o botão voltar faz.
    pub fn on_back(mut self, f: impl FnMut() + 'static) -> Self {
        self.on_back = Some(Box::new(f));
        self
    }

    /// Conteúdo no meio da barra, centralizado no espaço livre (a caixa de busca). A barra
    /// passa a ter 48 px.
    pub fn content(mut self, content: impl View) -> Self {
        self.content = Some(Box::new(content));
        self
    }

    /// Conteúdo à direita, antes dos botões da janela (o avatar da conta). A barra passa a ter
    /// 48 px.
    pub fn right_header(mut self, content: impl View) -> Self {
        self.right = Some(Box::new(content));
        self
    }

    fn sub(&self, suffix: &str) -> Option<String> {
        self.common.id.as_ref().map(|id| format!("{id}-{suffix}"))
    }
}

common_builders!(TitleBar);

impl Widget for TitleBar {
    fn kind(&self) -> &'static str {
        "title_bar"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::TitleBar {
            active: self.state.active(),
        }
    }

    fn flex(&self) -> Option<Flex> {
        Some(Flex {
            direction: Direction::Row,
            align: Align::Center,
            ..Flex::default()
        })
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        let tall = self.content.is_some() || self.right.is_some();
        let height = *self.common.height.get_or_insert(if tall {
            TITLE_BAR_HEIGHT
        } else {
            TITLE_BAR_COMPACT_HEIGHT
        });
        let state = self.state;
        let mut children: Vec<Box<dyn Widget>> = Vec::new();

        children.push(Box::new(Column::new().width(2)));
        children.push(Box::new(
            Column::new()
                .width(MAC_TRAFFIC_LIGHTS)
                .visible(move || state.chrome() == WindowChrome::MacOs),
        ));
        let visible = self.back.take().unwrap_or(Prop::Value(false));
        let enabled = self.back_enabled.take().unwrap_or(Prop::Value(true));
        children.push(Box::new(BarButton {
            common: Common {
                width: Some(TITLE_BAR_BUTTON_WIDTH),
                height: Some(height),
                visible: Some(visible),
                tooltip: Some(Prop::Value(crate::strings().back.to_string())),
                ..Common::generated(self.sub("voltar"))
            },
            role: BarRole::Back {
                enabled,
                on_back: self.on_back.take(),
            },
            state,
            press: Pressable::default(),
        }));
        children.push(Box::new(BarButton {
            common: Common {
                width: Some(TITLE_BAR_BUTTON_WIDTH),
                height: Some(height),
                visible: Some(Prop::Dynamic(Rc::new(move || state.pane.get().is_some()))),
                tooltip: Some(Prop::Dynamic(Rc::new(move || toggle_label(state)))),
                ..Common::generated(self.sub("alternar"))
            },
            role: BarRole::Toggle,
            state,
            press: Pressable::default(),
        }));
        children.push(Box::new(Column::new().width(14)));

        if let Some(icon) = self.icon {
            let mut part = Part::new(state);
            part.fades = true;
            part.flex.padding = Insets::new(0.0, 16.0, 0.0, 0.0);
            match icon {
                TitleIcon::Symbol(symbol) => {
                    part.children.push(Box::new(Icon::new(symbol).size(16)));
                }
                TitleIcon::Image(bytes) => part.children.push(Box::new(
                    Image::bytes(bytes)
                        .width(ICON_SIDE)
                        .height(ICON_SIDE)
                        .decorative(),
                )),
            }
            children.push(Box::new(part));
        }
        let mut title = Part::new(state);
        title.foreground = Some(Token::TextFillColorPrimary);
        title.flex.padding = Insets::new(0.0, 8.0, 0.0, 0.0);
        if let Some(badge) = self.badge.take() {
            title.children.push(badge.into_widget(state));
        } else {
            let text = self.title.clone();
            title.children.push(Box::new(
                Text::new(move || text.get()).caption().max_lines(1),
            ));
        }
        children.push(Box::new(title));
        if let Some(subtitle) = self.subtitle.take() {
            let mut part = Part::new(state);
            part.foreground = Some(Token::TextFillColorSecondary);
            part.flex.padding = Insets::new(0.0, 16.0, 0.0, 0.0);
            part.children.push(Box::new(
                Text::new(move || subtitle.get()).caption().max_lines(1),
            ));
            children.push(Box::new(part));
        }

        let mut content = Part::new(state);
        content.fades = true;
        content.common.grow = 1.0;
        content.flex.justify = Justify::Center;
        content.children.extend(self.content.take());
        children.push(Box::new(content));
        if let Some(right) = self.right.take() {
            let mut part = Part::new(state);
            part.fades = true;
            part.children.push(right);
            children.push(Box::new(part));
        }
        children.push(Box::new(Column::new().width(MIN_DRAG_WIDTH)));

        for (role, suffix) in [
            (CaptionRole::Minimize, "minimizar"),
            (CaptionRole::Maximize, "maximizar"),
            (CaptionRole::Close, "fechar"),
        ] {
            children.push(Box::new(CaptionButton {
                common: Common {
                    width: Some(CAPTION_BUTTON_WIDTH),
                    height: Some(height),
                    visible: Some(Prop::Dynamic(Rc::new(move || {
                        state.chrome() == WindowChrome::Custom
                    }))),
                    tooltip: Some(Prop::Dynamic(Rc::new(move || caption_label(role, state)))),
                    ..Common::generated(self.sub(suffix))
                },
                role,
                state,
                press: Pressable::default(),
            }));
        }
        children
    }

    fn handle(&mut self, event: &Event, cx: &mut EventCx) -> bool {
        if !self.state.custom() {
            return false;
        }
        match *event {
            Event::PointerDown { clicks: 2, .. } => {
                self.drag = None;
                cx.request_window(WindowRequest::ToggleMaximize);
                true
            }
            Event::PointerDown { .. } => {
                self.drag = Some(cx.pointer());
                true
            }
            Event::PointerMove { .. } => {
                if let Some(origin) = self.drag
                    && (cx.pointer() - origin).hypot() >= DRAG_SLOP
                {
                    self.drag = None;
                    cx.request_window(WindowRequest::Drag);
                }
                true
            }
            Event::PointerUp { .. } | Event::PointerCancel => {
                self.drag = None;
                true
            }
            _ => false,
        }
    }

    fn interactive(&self) -> bool {
        self.state.custom()
    }

    fn names_children(&self) -> bool {
        true
    }

    fn text(&self) -> Option<String> {
        Some(self.title.get())
    }

    fn window_state(&self) -> Option<WindowState> {
        Some(self.state)
    }
}

impl fmt::Debug for TitleBar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TitleBar")
            .field("common", &self.common)
            .field("title", &self.title)
            .field("subtitle", &self.subtitle)
            .field("icon", &self.icon)
            .field("badge", &self.badge)
            .finish_non_exhaustive()
    }
}

/// Contêiner de uma parte da barra: apaga com a janela inativa (o ícone e o conteúdo) ou
/// passa a cor do texto (o título e o subtítulo, que viram terciários).
struct Part {
    common: Common,
    flex: Flex,
    state: WindowState,
    children: Vec<Box<dyn Widget>>,
    fades: bool,
    foreground: Option<Token>,
}

impl Part {
    fn new(state: WindowState) -> Self {
        Self {
            common: Common::default(),
            flex: Flex {
                direction: Direction::Row,
                align: Align::Center,
                ..Flex::default()
            },
            state,
            children: Vec::new(),
            fades: false,
            foreground: None,
        }
    }
}

impl Widget for Part {
    fn kind(&self) -> &'static str {
        "row"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::TitleBar {
            active: self.state.active(),
        }
    }

    fn flex(&self) -> Option<Flex> {
        Some(self.flex)
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        std::mem::take(&mut self.children)
    }

    fn foreground(&self) -> Option<ThemeColor> {
        let token = self.foreground?;
        Some(if self.state.active() {
            token.into()
        } else {
            Token::TextFillColorTertiary.into()
        })
    }

    fn passes_foreground(&self) -> bool {
        self.foreground.is_some()
    }

    fn opacity(&self) -> f64 {
        if self.fades && !self.state.active() {
            0.5
        } else {
            1.0
        }
    }
}

/// Rótulo (e dica) do ☰ da barra.
fn toggle_label(state: WindowState) -> String {
    let strings = crate::strings();
    match state.pane.get() {
        Some(link) if link.open.get() => strings.navigation_close.to_string(),
        _ => strings.navigation_open.to_string(),
    }
}

/// Estado do voltar e do ☰ da barra, para o `Drawer`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TitleBarButtonVisual {
    /// Hover, pressionado ou desabilitado.
    pub state: ButtonState,
    /// Anel de foco (foco vindo do teclado).
    pub focus_ring: bool,
    /// Se a janela está ativa: inativa, o glifo fica terciário.
    pub active: bool,
}

/// O que um botão da barra faz.
enum BarRole {
    /// Voltar.
    Back {
        enabled: Prop<bool>,
        on_back: Option<Box<dyn FnMut()>>,
    },
    /// ☰ do `NavigationView` ligado.
    Toggle,
}

/// O voltar e o ☰ da barra.
struct BarButton {
    common: Common,
    role: BarRole,
    state: WindowState,
    press: Pressable,
}

impl BarButton {
    fn is_enabled(&self) -> bool {
        match &self.role {
            BarRole::Back { enabled, .. } => enabled.get(),
            BarRole::Toggle => true,
        }
    }

    fn label(&self) -> String {
        match self.role {
            BarRole::Back { .. } => crate::strings().back.to_string(),
            BarRole::Toggle => toggle_label(self.state),
        }
    }
}

impl Widget for BarButton {
    fn kind(&self) -> &'static str {
        match self.role {
            BarRole::Back { .. } => "title_bar_back",
            BarRole::Toggle => "navigation_toggle",
        }
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        let enabled = self.is_enabled();
        Look::TitleBarButton {
            label: self.label(),
            icon: match self.role {
                BarRole::Back { .. } => Symbol::ArrowLeft,
                BarRole::Toggle => Symbol::Navigation,
            },
            visual: TitleBarButtonVisual {
                state: if enabled {
                    self.press.state()
                } else {
                    ButtonState::Disabled
                },
                focus_ring: enabled && self.press.focus_ring(),
                active: self.state.active(),
            },
        }
    }

    fn handle(&mut self, event: &Event, _: &mut EventCx) -> bool {
        if !self.is_enabled() {
            self.press.reset();
            return false;
        }
        let (consumed, clicked) = self.press.handle(event, true);
        if clicked {
            match &mut self.role {
                BarRole::Back { on_back, .. } => {
                    if let Some(f) = on_back {
                        f();
                    }
                }
                BarRole::Toggle => {
                    if let Some(link) = self.state.pane.get_untracked() {
                        link.open.set(!link.open.get_untracked());
                    }
                }
            }
        }
        consumed
    }

    fn interactive(&self) -> bool {
        true
    }

    fn focusable(&self) -> bool {
        self.is_enabled()
    }

    fn enabled(&self) -> bool {
        self.is_enabled()
    }

    fn text(&self) -> Option<String> {
        Some(self.label())
    }
}

impl fmt::Debug for BarButton {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BarButton")
            .field("common", &self.common)
            .finish_non_exhaustive()
    }
}

/// Glifo de um botão da janela.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CaptionGlyph {
    /// Traço.
    #[default]
    Minimize,
    /// Quadrado.
    Maximize,
    /// Dois quadrados (a janela está maximizada).
    Restore,
    /// X, com o fundo vermelho no hover.
    Close,
}

/// Estado de um botão da janela, para o `Drawer`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CaptionButtonVisual {
    /// Que botão é.
    pub glyph: CaptionGlyph,
    /// Hover ou pressionado.
    pub state: ButtonState,
    /// Se a janela está ativa: inativa, o glifo em repouso fica apagado.
    pub active: bool,
}

/// Botão da janela.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CaptionRole {
    Minimize,
    Maximize,
    Close,
}

/// Rótulo (e dica) de um botão da janela.
fn caption_label(role: CaptionRole, state: WindowState) -> String {
    let strings = crate::strings();
    match role {
        CaptionRole::Minimize => strings.minimize.to_string(),
        CaptionRole::Maximize if state.maximized() => strings.restore.to_string(),
        CaptionRole::Maximize => strings.maximize.to_string(),
        CaptionRole::Close => strings.close.to_string(),
    }
}

/// Minimizar, maximizar (ou restaurar) e fechar. Não recebem foco, como os da moldura do
/// Windows; o teclado tem os atalhos do sistema.
struct CaptionButton {
    common: Common,
    role: CaptionRole,
    state: WindowState,
    press: Pressable,
}

impl Widget for CaptionButton {
    fn kind(&self) -> &'static str {
        match self.role {
            CaptionRole::Minimize => "window_minimize",
            CaptionRole::Maximize => "window_maximize",
            CaptionRole::Close => "window_close",
        }
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::CaptionButton(CaptionButtonVisual {
            glyph: match self.role {
                CaptionRole::Minimize => CaptionGlyph::Minimize,
                CaptionRole::Maximize if self.state.maximized() => CaptionGlyph::Restore,
                CaptionRole::Maximize => CaptionGlyph::Maximize,
                CaptionRole::Close => CaptionGlyph::Close,
            },
            state: self.press.state(),
            active: self.state.active(),
        })
    }

    fn handle(&mut self, event: &Event, cx: &mut EventCx) -> bool {
        let (consumed, clicked) = self.press.handle(event, false);
        if clicked {
            cx.request_window(match self.role {
                CaptionRole::Minimize => WindowRequest::Minimize,
                CaptionRole::Maximize => WindowRequest::ToggleMaximize,
                CaptionRole::Close => WindowRequest::Close,
            });
        }
        consumed
    }

    fn interactive(&self) -> bool {
        true
    }

    fn text(&self) -> Option<String> {
        Some(caption_label(self.role, self.state))
    }
}

impl fmt::Debug for CaptionButton {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CaptionButton")
            .field("common", &self.common)
            .field("role", &self.role)
            .finish_non_exhaustive()
    }
}
