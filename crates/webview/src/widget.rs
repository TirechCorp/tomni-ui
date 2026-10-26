//! O widget [`WebView`] e o visual que o `Drawer` pinta ([`WebViewVisual`]).

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use tomni_ui_core::kurbo::{Rect, Size};
use tomni_ui_core::{
    Align, Button, Column, Common, CustomLook, CustomVisual, Direction, Event, EventCx, Flex, Icon,
    Insets, IntoProp, Justify, Look, NativeFocus, NativePlace, ProgressRing, Prop, Symbol, Text,
    Widget,
};
use tomni_ui_reactive::{Get, GetUntracked, Set, Signal, effect, signal, untrack};

use crate::backend::{APP_ORIGIN, Asset, Backend, Capture, Notice, Notices, Policy, mime};
use crate::controller::{Command, LoadState, WebViewController};
use crate::fake::Fake;

/// Tamanho natural (e mínimo) do WebView: dê `.grow(1)` ou `.width()` e `.height()`.
pub const WEBVIEW_MIN_SIZE: Size = Size::new(200.0, 150.0);
/// Quanto o controle do sistema espera pela captura antes de sumir, quando algo o cobre.
pub const CAPTURE_WAIT: Duration = Duration::from_millis(200);

/// Resultado de uma carga, para o [`WebView::on_load`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loaded {
    /// A página.
    pub url: String,
    /// O erro, se não carregou.
    pub error: Option<String>,
}

type MessageHandler = Box<dyn FnMut(String)>;
type LoadHandler = Box<dyn FnMut(&Loaded)>;

/// Página da web dentro da tela, no controle do sistema: o WebView2 no Windows, o WKWebView no
/// macOS e o WebKitGTK no Linux (pelo `wry`). Como o `WebView2` do WinUI.
///
/// ```
/// # use tomni_ui_reactive::Scope;
/// use tomni_ui_webview::WebView;
///
/// # let scope = Scope::root();
/// # scope.run(|| {
/// // Uma página da internet: os links para outras origens abrem no navegador.
/// let ajuda = WebView::new("https://exemplo.com/ajuda").id("ajuda").grow(1);
///
/// // Uma página do próprio app, servida em `fluent://app/`, que conversa com ele.
/// let painel = WebView::html("<script>window.fluent.postMessage('oi')</script>")
///     .asset("estilo.css", "body { margin: 0 }")
///     .on_message(|texto| println!("a página disse {texto}"))
///     .id("painel")
///     .grow(1);
/// # });
/// # scope.clear();
/// ```
///
/// - **Conteúdo**: uma URL ([`WebView::new`]; fixa, closure ou signal, que navega quando muda)
///   ou páginas embutidas ([`WebView::html`] e [`WebView::asset`]), servidas na origem
///   `fluent://app/` (com `fetch`, módulos e o resto da web funcionando entre elas).
/// - **Navegação**: a página navega à vontade na própria origem, em `fluent://app/` e nas de
///   [`WebView::allow`]; um link para outra origem (`http`, `https`, `mailto`, `tel`) abre no
///   programa do sistema, e uma janela nova (`target="_blank"`, `window.open`) também.
/// - **Conversa**: a página manda texto com `window.fluent.postMessage(texto)`
///   ([`WebView::on_message`]) e recebe o que o app manda com
///   [`WebViewController::post_message`] num `MessageEvent` do `window.fluent`.
/// - **Estado e comandos**: [`WebView::controller`].
/// - **Carga**: um `ProgressRing` (`{id}-carregando`) na primeira; se falhar, um aviso com o
///   erro e o botão **Tentar de novo** (`{id}-erro` e `{id}-tentar`).
/// - **Tema**: a página recebe o tema do app no `prefers-color-scheme`.
/// - **Foco**: com Tab, o foco entra na página; o F6 (Shift+F6) o leva ao próximo (anterior)
///   widget, como no `Terminal`.
/// - **Sistemas**: no Windows 10, o runtime do WebView2 vem com o instalador do app (no 11, já
///   está lá); no Linux, a janela do app abre no X11 (no Wayland, pelo XWayland), porque o
///   WebKitGTK só se põe numa janela X11. No Android, ainda não há motor: o lugar da página
///   mostra o erro, como quando o motor não abre.
///
/// O controle do sistema é uma janela por cima da janela do app: nada que o app pinta fica
/// por cima dela. Quando um `ContentDialog`, um `Flyout` ou um menu se abre sobre a página, o
/// WebView tira uma captura dela e esconde o controle até fechar. Por isso ele não vai dentro
/// de um `ScrollViewer` (o lint avisa, `webview-in-scroll`): a página já rola sozinha.
///
/// Sem janela (nos testes, no `tomni-ui lint`, nas capturas da CLI), o motor é um falso
/// ([`Fake`]): as cargas terminam no frame seguinte e o lugar da página mostra um globo e o
/// endereço.
pub struct WebView {
    common: Common,
    flex: Flex,
    target: Signal<String>,
    requested: Option<String>,
    assets: HashMap<String, Asset>,
    allowed: Vec<String>,
    controller: WebViewController,
    policy: Rc<Policy>,
    notices: Notices,
    backend: Option<Box<dyn Backend>>,
    on_message: Option<MessageHandler>,
    on_load: Option<LoadHandler>,
    on_title: Option<MessageHandler>,
    place: NativePlace,
    dark: Option<bool>,
    shown: Option<Rect>,
    capture: Option<Capture>,
    waiting: Option<Duration>,
    focus_request: Cell<Option<NativeFocus>>,
    focused: bool,
}

impl WebView {
    /// WebView que carrega `url` (valor fixo, closure ou signal): carrega de novo quando ela
    /// muda. A origem dela fica permitida para os links da página.
    pub fn new<M>(url: impl IntoProp<String, M>) -> Self {
        let url = url.into_prop();
        let target = signal(untrack(|| url.get()));
        if let Prop::Dynamic(url) = url {
            effect(move || {
                let url = url();
                if target.get_untracked() != url {
                    target.set(url);
                }
            });
        }
        #[cfg(feature = "native")]
        crate::native::prepare();
        Self {
            common: Common::default(),
            flex: Flex {
                direction: Direction::Column,
                padding: Insets::all(24.0),
                align: Align::Center,
                justify: Justify::Center,
                ..Flex::default()
            },
            target,
            requested: None,
            assets: HashMap::new(),
            allowed: Vec::new(),
            controller: WebViewController::new(),
            policy: Rc::default(),
            notices: Rc::new(RefCell::new(VecDeque::new())),
            backend: None,
            on_message: None,
            on_load: None,
            on_title: None,
            place: NativePlace::HIDDEN,
            dark: None,
            shown: None,
            capture: None,
            waiting: None,
            focus_request: Cell::new(None),
            focused: false,
        }
    }

    /// WebView com a página `content`, servida em `fluent://app/index.html`. Os arquivos que
    /// ela usa entram com [`WebView::asset`].
    pub fn html(content: impl Into<String>) -> Self {
        Self::new(format!("{APP_ORIGIN}/index.html")).asset("index.html", content.into())
    }

    /// Serve `bytes` em `fluent://app/{path}` (`"css/estilo.css"`, `"logo.png"`), com o tipo
    /// pela extensão. Para embutir no executável: `.asset("logo.png", include_bytes!("logo.png"))`.
    pub fn asset(mut self, path: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        let path = path.into();
        let path = path.trim_start_matches('/').to_owned();
        let asset = Asset {
            mime: mime(&path),
            bytes: Arc::from(bytes.into()),
        };
        self.assets.insert(path, asset);
        self
    }

    /// Permite que a página navegue em `origin` (`"https://docs.exemplo.com"`) sem abrir no
    /// navegador do sistema.
    pub fn allow(mut self, origin: impl Into<String>) -> Self {
        self.allowed.push(origin.into());
        self
    }

    /// Chama `handler` com cada texto que a página manda (`window.fluent.postMessage(texto)`).
    pub fn on_message(mut self, handler: impl FnMut(String) + 'static) -> Self {
        self.on_message = Some(Box::new(handler));
        self
    }

    /// Chama `handler` quando uma página termina de carregar ou falha
    /// ([`Loaded::error`]).
    pub fn on_load(mut self, handler: impl FnMut(&Loaded) + 'static) -> Self {
        self.on_load = Some(Box::new(handler));
        self
    }

    /// Chama `handler` quando o `<title>` da página muda.
    pub fn on_title(mut self, handler: impl FnMut(String) + 'static) -> Self {
        self.on_title = Some(Box::new(handler));
        self
    }

    /// Liga o WebView a `controller`: o estado da página (URL, título, carga, histórico) e os
    /// comandos (navegar, voltar, recarregar, mandar mensagem).
    pub fn controller(mut self, controller: WebViewController) -> Self {
        self.controller = controller;
        self
    }

    /// Id estável, usado em testes, no MCP e no lint. O `ProgressRing` da carga fica com o id
    /// `{id}-carregando`, e o aviso de erro com `{id}-erro` (o botão, `{id}-tentar`).
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.common.id = Some(id.into());
        self
    }

    /// Ocupa o espaço do eixo principal do pai, como o `flex: n` do CSS.
    pub fn grow(mut self, grow: impl Into<f64>) -> Self {
        self.common.grow = grow.into();
        self
    }

    /// Largura fixa, em pixels.
    pub fn width(mut self, width: impl Into<f64>) -> Self {
        self.common.width = Some(width.into());
        self
    }

    /// Altura fixa, em pixels.
    pub fn height(mut self, height: impl Into<f64>) -> Self {
        self.common.height = Some(height.into());
        self
    }

    /// Mostra ou esconde o WebView (valor fixo, closure ou signal). Oculto, a página continua
    /// carregada.
    pub fn visible<M>(mut self, visible: impl IntoProp<bool, M>) -> Self {
        self.common.visible = Some(visible.into_prop());
        self
    }

    /// Dica (texto fixo, closure ou signal) com o ponteiro parado sobre o WebView, quando a
    /// página não aparece (carregando, com erro).
    pub fn tooltip<M>(mut self, text: impl IntoProp<String, M>) -> Self {
        self.common.tooltip = Some(text.into_prop());
        self
    }

    /// O controller (o dado com [`WebView::controller`] ou o próprio).
    pub fn current_controller(&self) -> WebViewController {
        self.controller
    }

    /// O motor falso, quando não há janela. O `TestApp` usa nos `webview_*`.
    pub fn fake(&self) -> Option<&Fake> {
        self.backend.as_ref()?.as_any().downcast_ref()
    }

    /// O motor falso, quando não há janela. O `TestApp` usa nos `webview_*`.
    pub fn fake_mut(&mut self) -> Option<&mut Fake> {
        self.backend.as_mut()?.as_any_mut().downcast_mut()
    }

    /// Se o controle do sistema aparece agora, por cima do que o app pinta.
    pub fn native_shown(&self) -> bool {
        self.shown.is_some() && self.backend.as_ref().is_some_and(|b| b.native())
    }

    /// Roda os comandos do app e trata o que o motor avisou. O loop de frame chama sozinho
    /// (pelo `tick`); o `TestApp` chama a cada passo.
    pub fn poll(&mut self) {
        let Some(backend) = &mut self.backend else {
            return;
        };
        // Primeiro o que o motor terminou desde o último turno; depois os pedidos novos, cujo
        // resultado chega no próximo: uma carga leva pelo menos um turno, como no motor real.
        backend.turn();
        let target = self.target.get_untracked();
        if self.requested.as_ref() != Some(&target) {
            self.requested = Some(target.clone());
            if !target.is_empty() {
                self.run(Command::Navigate(target));
            }
        }
        for command in self.controller.take_commands() {
            self.run(command);
        }
        loop {
            let notice = self.notices.borrow_mut().pop_front();
            let Some(notice) = notice else { break };
            self.notice(notice);
        }
        self.sync();
    }

    /// O visual, para o `Drawer`.
    pub fn visual(&self) -> WebViewVisual {
        let c = self.controller;
        WebViewVisual {
            url: c.url(),
            title: c.title(),
            state: c.state(),
            error: c.error(),
            can_go_back: c.can_go_back(),
            can_go_forward: c.can_go_forward(),
            page: c.page_shown(),
            native: self.native_shown(),
            covered: self.place.covered,
            capture: self.capture.clone(),
        }
    }

    fn start(&mut self) {
        // A janela vem aí (o `App` monta a tela antes de abrir): o motor do sistema espera
        // por ela, e o `Ui` coloca o widget de novo quando ela chegar.
        #[cfg(feature = "native")]
        if tomni_ui_core::native::host_pending() {
            return;
        }
        for origin in &self.allowed {
            self.policy.allow(origin);
        }
        let assets = Arc::new(std::mem::take(&mut self.assets));
        #[cfg(feature = "native")]
        if let Some(host) = tomni_ui_core::native::host() {
            let backend: Box<dyn Backend> = match crate::native::Native::new(
                &host,
                Rc::clone(&self.notices),
                Rc::clone(&self.policy),
                assets,
            ) {
                Ok(native) => Box::new(native),
                Err(error) => Box::new(crate::fake::Unavailable {
                    notices: Rc::clone(&self.notices),
                    error: format!("o WebView do sistema não abriu: {error}"),
                }),
            };
            self.backend = Some(backend);
            return;
        }
        self.backend = Some(Box::new(Fake::new(
            Rc::clone(&self.notices),
            Rc::clone(&self.policy),
            assets,
        )));
    }

    fn run(&mut self, command: Command) {
        let Some(backend) = &mut self.backend else {
            return;
        };
        match command {
            Command::Navigate(url) => {
                self.policy.allow(&url);
                backend.navigate(&url);
            }
            Command::Back => backend.back(),
            Command::Forward => backend.forward(),
            // Depois de uma falha, o motor pode não ter a página no histórico (o WebKitGTK
            // fica na anterior): carrega o endereço que falhou.
            Command::Reload if self.controller.state.get_untracked() == LoadState::Failed => {
                let url = self.controller.url.get_untracked();
                match url.is_empty() {
                    true => backend.reload(),
                    false => backend.navigate(&url),
                }
            }
            Command::Reload => backend.reload(),
            Command::Post(text) => backend.post(&text),
            Command::Eval(script) => backend.eval(&script),
        }
    }

    fn notice(&mut self, notice: Notice) {
        let c = self.controller;
        match notice {
            Notice::Started(url) => {
                WebViewController::set_if_changed(c.url, url);
                WebViewController::set_if_changed(c.state, LoadState::Loading);
                WebViewController::set_if_changed(c.error, None);
            }
            Notice::Finished(url) => {
                self.history();
                if c.state.get_untracked() == LoadState::Failed {
                    return;
                }
                WebViewController::set_if_changed(c.url, url.clone());
                WebViewController::set_if_changed(c.state, LoadState::Ready);
                WebViewController::set_if_changed(c.shown, true);
                if let Some(handler) = &mut self.on_load {
                    handler(&Loaded { url, error: None });
                }
            }
            Notice::Failed { url, error } => {
                WebViewController::set_if_changed(c.state, LoadState::Failed);
                WebViewController::set_if_changed(c.error, Some(error.clone()));
                WebViewController::set_if_changed(c.shown, false);
                if let Some(handler) = &mut self.on_load {
                    handler(&Loaded {
                        url,
                        error: Some(error),
                    });
                }
            }
            Notice::Title(title) => {
                if c.title.get_untracked() != title {
                    c.title.set(title.clone());
                    if let Some(handler) = &mut self.on_title {
                        handler(title);
                    }
                }
            }
            Notice::Message(text) => {
                if let Some(handler) = &mut self.on_message {
                    handler(text);
                }
            }
            Notice::History => self.history(),
            Notice::Focus => {
                if !self.focused {
                    self.focus_request.set(Some(NativeFocus::Took));
                }
            }
            Notice::Leave { back } => self.focus_request.set(Some(if back {
                NativeFocus::Prev
            } else {
                NativeFocus::Next
            })),
            Notice::Open(url) => {
                if let Some(backend) = &mut self.backend {
                    backend.navigate(&url);
                }
            }
            Notice::Capture(capture) => {
                if self.place.covered {
                    self.capture = capture;
                }
                self.waiting = None;
            }
        }
    }

    fn history(&mut self) {
        if let Some(backend) = &self.backend {
            let (back, forward) = backend.history();
            WebViewController::set_if_changed(self.controller.can_go_back, back);
            WebViewController::set_if_changed(self.controller.can_go_forward, forward);
        }
    }

    /// Mostra, move ou esconde o controle do sistema conforme o lugar e a carga.
    fn sync(&mut self) {
        let Some(backend) = &mut self.backend else {
            return;
        };
        let page = untrack(|| self.controller.page_shown());
        let rect = self.place.rect.filter(|_| page);
        // Coberto, o controle continua até a captura chegar (ou o tempo dela passar).
        let want = rect.filter(|_| !self.place.covered || self.waiting.is_some());
        if want != self.shown {
            self.shown = want;
            backend.place(want);
        }
    }
}

impl Widget for WebView {
    fn kind(&self) -> &'static str {
        "webview"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::Custom(CustomLook::new(self.visual()))
    }

    fn flex(&self) -> Option<Flex> {
        Some(self.flex)
    }

    fn measured(&self) -> bool {
        true
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        let id = self.common.id.as_deref().unwrap_or("webview");
        let c = self.controller;
        let ring = ProgressRing::new()
            .id(format!("{id}-carregando"))
            .visible(move || c.state() == LoadState::Loading && !c.shown.get());
        let error = Column::new()
            .id(format!("{id}-erro"))
            .gap(8)
            .align(Align::Center)
            .visible(move || c.state() == LoadState::Failed)
            .child(Icon::new(Symbol::Globe).size(32).decorative())
            .child(Text::new(tomni_ui_core::strings().page_failed.to_string()).body_strong())
            .child(
                Text::new(move || c.error().unwrap_or_default())
                    .secondary()
                    .wrap()
                    .max_lines(3),
            )
            .child(
                Button::new(tomni_ui_core::strings().try_again.to_string())
                    .id(format!("{id}-tentar"))
                    .height(44)
                    .on_click(move || c.reload()),
            );
        vec![Box::new(ring), Box::new(error)]
    }

    fn handle(&mut self, event: &Event, _cx: &mut EventCx) -> bool {
        match event {
            Event::FocusIn(_) => {
                self.focused = true;
                if let Some(backend) = &mut self.backend {
                    backend.focus();
                }
                true
            }
            Event::FocusOut => {
                self.focused = false;
                if let Some(backend) = &mut self.backend {
                    backend.unfocus();
                }
                true
            }
            Event::PointerDown { .. } => true,
            _ => false,
        }
    }

    fn interactive(&self) -> bool {
        true
    }

    fn focusable(&self) -> bool {
        true
    }

    fn text(&self) -> Option<String> {
        Some(self.controller.url.get_untracked())
    }

    fn animating(&self) -> bool {
        true
    }

    fn ticking(&self) -> bool {
        let Some(backend) = &self.backend else {
            return false;
        };
        self.waiting.is_some()
            || backend.busy()
            || self.controller.has_commands()
            || !self.notices.borrow().is_empty()
            || self.requested.as_ref() != Some(&self.target.get_untracked())
    }

    fn tick(&mut self, dt: Duration) {
        if let Some(waited) = self.waiting {
            let waited = waited + dt;
            self.waiting = (waited < CAPTURE_WAIT).then_some(waited);
        }
        self.poll();
    }

    fn native(&self) -> bool {
        true
    }

    fn place_native(&mut self, place: NativePlace) {
        if self.backend.is_none() {
            self.start();
        }
        let was_covered = self.place.covered;
        self.place = place;
        if let Some(backend) = &mut self.backend {
            if self.dark != Some(place.dark) {
                self.dark = Some(place.dark);
                backend.set_dark(place.dark);
            }
            if place.covered && !was_covered && self.shown.is_some() {
                self.waiting = Some(Duration::ZERO);
                backend.capture();
            }
        }
        if !place.covered {
            self.waiting = None;
            self.capture = None;
        }
        self.sync();
    }

    fn take_native_focus(&self) -> Option<NativeFocus> {
        self.focus_request.take()
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}

impl fmt::Debug for WebView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebView")
            .field("common", &self.common)
            .field("controller", &self.controller)
            .field("place", &self.place)
            .finish_non_exhaustive()
    }
}

/// O que o `Drawer` precisa para pintar o [`WebView`] e o inspetor para descrevê-lo.
#[derive(Clone, Debug, PartialEq)]
pub struct WebViewVisual {
    /// A página atual.
    pub url: String,
    /// O `<title>` dela.
    pub title: String,
    /// Em que pé está a carga.
    pub state: LoadState,
    /// O erro, se a carga falhou.
    pub error: Option<String>,
    /// Se há página para trás.
    pub can_go_back: bool,
    /// Se há página para a frente.
    pub can_go_forward: bool,
    /// Se a página está à mostra (e não o `ProgressRing` da primeira carga ou o aviso de
    /// erro).
    pub page: bool,
    /// Se o controle do sistema aparece por cima: o `Drawer` só pinta o fundo.
    pub native: bool,
    /// Se algo pintado pelo app cobre o WebView.
    pub covered: bool,
    /// A captura da página, pintada no lugar dela enquanto está coberta.
    pub capture: Option<Capture>,
}

impl CustomVisual for WebViewVisual {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn eq_visual(&self, other: &dyn CustomVisual) -> bool {
        other
            .as_any()
            .downcast_ref::<Self>()
            .is_some_and(|other| self == other)
    }
}
