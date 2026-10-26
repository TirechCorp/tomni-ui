//! [`WebViewController`]: o estado da página em signals e os comandos do app.

use tomni_ui_reactive::{Get, Set, Signal, Update, WithUntracked, signal};

/// Em que pé está a carga da página.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LoadState {
    /// Carregando.
    #[default]
    Loading,
    /// Carregada.
    Ready,
    /// Não carregou: o erro está em [`WebViewController::error`].
    Failed,
}

impl LoadState {
    /// Nome no `tomni-ui tree` e no MCP: `loading`, `ready` ou `failed`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Loading => "loading",
            Self::Ready => "ready",
            Self::Failed => "failed",
        }
    }
}

/// Pedido do app ao WebView, na fila até o próximo frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Command {
    Navigate(String),
    Back,
    Forward,
    Reload,
    Post(String),
    Eval(String),
}

/// O estado da página e os comandos de um [`WebView`](crate::WebView), como o `CoreWebView2`
/// do WinUI. É `Copy`: crie com [`WebViewController::new`] na tela, passe com
/// [`WebView::controller`](crate::WebView::controller) e use nos botões.
///
/// ```
/// # use tomni_ui_reactive::Scope;
/// use tomni_ui_webview::{WebView, WebViewController};
///
/// # let scope = Scope::root();
/// # scope.run(|| {
/// let web = WebViewController::new();
/// let pagina = WebView::new("https://exemplo.com").id("pagina").controller(web);
/// // Num botão: `.on_click(move || web.go_back())`, `.enabled(move || web.can_go_back())`.
/// # });
/// # scope.clear();
/// ```
///
/// Os getters leem signals: numa closure de propriedade (`Text::new(move || web.title())`), a
/// tela acompanha.
#[derive(Clone, Copy, Debug)]
pub struct WebViewController {
    pub(crate) url: Signal<String>,
    pub(crate) title: Signal<String>,
    pub(crate) state: Signal<LoadState>,
    pub(crate) error: Signal<Option<String>>,
    pub(crate) can_go_back: Signal<bool>,
    pub(crate) can_go_forward: Signal<bool>,
    /// Se a página atual já apareceu: a primeira carga (e a que segue um erro) mostra o
    /// `ProgressRing`; as outras deixam a página antiga até a nova chegar, como num navegador.
    pub(crate) shown: Signal<bool>,
    pub(crate) commands: Signal<Vec<Command>>,
}

impl WebViewController {
    /// Controller de um WebView que ainda não carregou nada.
    pub fn new() -> Self {
        Self {
            url: signal(String::new()),
            title: signal(String::new()),
            state: signal(LoadState::Loading),
            error: signal(None),
            can_go_back: signal(false),
            can_go_forward: signal(false),
            shown: signal(false),
            commands: signal(Vec::new()),
        }
    }

    /// A página atual (a última que começou a carregar). As embutidas são `fluent://app/...`
    /// em todos os sistemas.
    pub fn url(&self) -> String {
        self.url.get()
    }

    /// O `<title>` da página.
    pub fn title(&self) -> String {
        self.title.get()
    }

    /// Em que pé está a carga.
    pub fn state(&self) -> LoadState {
        self.state.get()
    }

    /// Se está carregando.
    pub fn loading(&self) -> bool {
        self.state.get() == LoadState::Loading
    }

    /// O erro da última carga, se ela falhou.
    pub fn error(&self) -> Option<String> {
        self.error.get()
    }

    /// Se há página para trás no histórico.
    pub fn can_go_back(&self) -> bool {
        self.can_go_back.get()
    }

    /// Se há página para a frente no histórico.
    pub fn can_go_forward(&self) -> bool {
        self.can_go_forward.get()
    }

    /// Carrega `url`. A origem dela passa a ser permitida para os links da página.
    pub fn navigate(&self, url: impl Into<String>) {
        self.push(Command::Navigate(url.into()));
    }

    /// Volta uma página.
    pub fn go_back(&self) {
        self.push(Command::Back);
    }

    /// Avança uma página.
    pub fn go_forward(&self) {
        self.push(Command::Forward);
    }

    /// Carrega a página de novo.
    pub fn reload(&self) {
        self.push(Command::Reload);
    }

    /// Manda `text` à página: chega como um `MessageEvent` no `window.fluent`
    /// (`window.fluent.addEventListener('message', e => e.data)`).
    pub fn post_message(&self, text: impl Into<String>) {
        self.push(Command::Post(text.into()));
    }

    /// Roda `script` na página, sem esperar resultado.
    pub fn eval(&self, script: impl Into<String>) {
        self.push(Command::Eval(script.into()));
    }

    fn push(&self, command: Command) {
        self.commands.update(|commands| commands.push(command));
        tomni_ui_core::wake();
    }

    pub(crate) fn has_commands(&self) -> bool {
        self.commands
            .with_untracked(|commands| !commands.is_empty())
    }

    pub(crate) fn take_commands(&self) -> Vec<Command> {
        if !self.has_commands() {
            return Vec::new();
        }
        let mut taken = Vec::new();
        self.commands
            .update(|commands| taken = std::mem::take(commands));
        taken
    }

    /// Se a página está à mostra (carregada, ou carregando outra por cima de uma carregada).
    pub(crate) fn page_shown(&self) -> bool {
        self.shown.get() && self.state.get() != LoadState::Failed
    }

    pub(crate) fn set_if_changed<T: PartialEq + Send + Sync + 'static>(
        signal: Signal<T>,
        value: T,
    ) {
        if signal.with_untracked(|current| *current != value) {
            signal.set(value);
        }
    }
}

impl Default for WebViewController {
    fn default() -> Self {
        Self::new()
    }
}
