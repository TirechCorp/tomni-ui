//! `App`: abre um `Ui` numa janela da plataforma.

use tomni_ui_core::kurbo::{Point, Vec2};
use tomni_ui_core::windows::OpenWindow;
use tomni_ui_core::{
    Clipboard, Cursor, DragData, Ime, Insets, Key, Modifiers, PointerKind, ResizeEdge, TextInput,
    View, WindowChrome, WindowRequest,
};
use tomni_ui_platform::{Backdrop, Handler, WindowIcon, WindowOptions};
use tomni_ui_render::Canvas;
use tomni_ui_render::peniko::Color;
use tomni_ui_theme::{Drawer, FluentDrawer, Theme};

use crate::Ui;

/// App numa janela: o `Ui` com o tema Fluent, renderizado na GPU (ou na CPU, quando o
/// adaptador é um driver em software, como o WARP de uma VM Windows sem driver de vídeo).
///
/// ```no_run
/// use tomni_ui::prelude::*;
///
/// fn contador() -> impl View {
///     let n = signal(0);
///     Form::new("Contador").child(
///         Column::new()
///             .padding(16)
///             .child(Button::new(move || format!("Cliques: {}", n.get())))
///             .child(Button::new("+1").on_click(move || n.update(|n| *n += 1))),
///     )
/// }
///
/// fn main() -> Result<(), tomni_ui::platform::Error> {
///     tomni_ui::App::new(contador).size(320, 200).run()
/// }
/// ```
///
/// Com `TOMNI_UI_TOOLS=1` (feature `tools`, ligada por padrão), [`App::run`] não abre janela:
/// atende a CLI `tomni-ui` e o servidor MCP pela entrada e saída padrão, com as telas
/// registradas em [`App::new`] e [`App::view`].
pub struct App {
    pub(crate) views: Vec<NamedView>,
    pub(crate) options: WindowOptions,
    pub(crate) theme: Option<Theme>,
    pub(crate) start_hidden: bool,
}

/// Uma tela registrada, que as ferramentas montam quantas vezes precisarem.
pub(crate) struct NamedView {
    pub(crate) name: String,
    pub(crate) mount: Box<dyn Fn(u32, u32, Theme) -> Ui>,
}

impl NamedView {
    fn new<V: View, F: Fn() -> V + 'static>(view: F) -> Self {
        Self {
            name: view_name::<F>(),
            mount: Box::new(move |width, height, theme| {
                Ui::new(width, height, FluentDrawer::new(theme), &view)
            }),
        }
    }
}

/// Nome de uma tela: o da função (`login` para `fn login()`). Uma closure leva o nome da
/// função em que foi escrita.
fn view_name<F>() -> String {
    let path = std::any::type_name::<F>();
    let path = path.split('<').next().unwrap_or(path);
    path.rsplit("::")
        .find(|segment| !segment.starts_with('{'))
        .unwrap_or("view")
        .to_owned()
}

impl App {
    /// App com a tela `view`, numa janela de 800 × 600 pixels lógicos.
    ///
    /// `view` é a função que monta a tela. Ela pode ser chamada mais de uma vez: as
    /// ferramentas para agentes montam a tela de novo a cada sessão.
    pub fn new<V: View, F: Fn() -> V + 'static>(view: F) -> Self {
        Self {
            views: vec![NamedView::new(view)],
            options: WindowOptions::default(),
            theme: None,
            start_hidden: false,
        }
    }

    /// Registra outra tela, só para as ferramentas (`tomni-ui screenshot --view cadastro`). A
    /// janela abre sempre a de [`App::new`]. O nome é o da função.
    pub fn view<V: View, F: Fn() -> V + 'static>(mut self, view: F) -> Self {
        self.views.push(NamedView::new(view));
        self
    }

    /// Tamanho inicial da janela, em pixels lógicos. No Android vale a tela.
    pub fn size(mut self, width: u32, height: u32) -> Self {
        self.options.width = width;
        self.options.height = height;
        self
    }

    /// A janela principal nasce escondida, como se o app tivesse chamado
    /// [`WindowHandle::hide`](crate::WindowHandle::hide) antes de abrir: é como um app que mora
    /// na bandeja começa. [`WindowHandle::show`](crate::WindowHandle::show) a mostra. No Wayland,
    /// que não tem como esconder, ela abre minimizada.
    pub fn start_hidden(mut self, hidden: bool) -> Self {
        self.start_hidden = hidden;
        self
    }

    /// Fixa o tema. Sem isso, segue o claro ou escuro do sistema.
    pub fn theme(mut self, theme: Theme) -> Self {
        self.theme = Some(theme);
        self
    }

    /// Fundo da janela. Com [`Backdrop::Mica`], no Windows 11, o Mica do sistema aparece atrás
    /// do `Form`; nos outros sistemas, e no lint e nos snapshots, fica a cor sólida.
    pub fn backdrop(mut self, backdrop: Backdrop) -> Self {
        self.options.backdrop = backdrop;
        self
    }

    /// O ícone das janelas do app: o da moldura, da barra de tarefas e do alternador de janelas.
    /// Cada desenho é um PNG ou um SVG embutido (`include_bytes!`); o SVG é rasterizado no
    /// tamanho de cada uso. As janelas de [`open_window`](crate::open_window) herdam o ícone. No
    /// Wayland o ícone vem do atalho `.desktop` que tem o mesmo [`app_id`](Self::app_id), não da
    /// janela. Sem isto vale o ícone do sistema.
    pub fn icon(mut self, icon: WindowIcon) -> Self {
        self.options.icon = Some(icon);
        self
    }

    /// O nome com que o ambiente agrupa as janelas do app e as casa com o atalho `.desktop`: o
    /// `app_id` do Wayland e a classe (`WM_CLASS`) do X11. As janelas de
    /// [`open_window`](crate::open_window) herdam. Sem isto vale o nome do executável. Não tem
    /// efeito no Windows nem no macOS.
    pub fn app_id(mut self, app_id: impl Into<String>) -> Self {
        self.options.app_id = Some(app_id.into());
        self
    }

    /// Liga a guarda da abertura, gravada em `path`: se a abertura anterior caiu antes do
    /// primeiro quadro (um driver de vídeo que derruba o processo), esta desenha de um jeito
    /// mais simples (o quadro na CPU, outro backend), um degrau por vez. Ver
    /// [`WindowOptions::render_guard`](tomni_ui_platform::WindowOptions::render_guard).
    pub fn render_guard(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.options.render_guard = Some(path.into());
        self
    }

    /// Liga o registro de travamentos, gravado em `path`: uma thread vigia escreve quando a
    /// interface fica parada mais de 2 s numa etapa (um evento, o quadro, o layout, os
    /// terminais, a textura da janela) e, a cada minuto em que ela pesou, o tempo por etapa.
    /// Ver [`WindowOptions::stall_log`](tomni_ui_platform::WindowOptions::stall_log).
    pub fn stall_log(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.options.stall_log = Some(path.into());
        self
    }

    /// `AndroidApp` recebido em `android_main`. Obrigatório no Android.
    #[cfg(target_os = "android")]
    pub fn android(mut self, app: tomni_ui_platform::AndroidApp) -> Self {
        self.options = self.options.android_app(app);
        self
    }

    /// Abre a janela e roda até ela ser fechada. Bloqueia a thread principal.
    ///
    /// Com `TOMNI_UI_TOOLS=1`, atende as ferramentas até a entrada padrão fechar.
    pub fn run(self) -> Result<(), tomni_ui_platform::Error> {
        #[cfg(feature = "tools")]
        if crate::tools::requested() {
            crate::tools::serve(self);
            return Ok(());
        }
        let (width, height) = (self.options.width, self.options.height);
        let theme = self.theme.unwrap_or_default();
        let first = self
            .views
            .into_iter()
            .next()
            .expect("App::new registra uma tela");
        tomni_ui_core::native::expect_host();
        if self.start_hidden {
            tomni_ui_core::windows::WindowHandle::main().hide();
        }
        let ui = (first.mount)(width, height, theme);
        let mut options = self.options;
        options.title_bar = ui.has_title_bar();
        let fixed = self.theme.is_some();
        let window = Window {
            ui,
            theme,
            fixed,
            transparent: false,
        };
        // As janelas pedidas por `open_window` levam o tema e o tamanho do app.
        let factory = move |request: OpenWindow| {
            let (width, height) = request.size.unwrap_or((width, height));
            let (theme, fixed) = match tomni_ui_core::windows::theme_override() {
                Some(dark) => (if dark { Theme::Dark } else { Theme::Light }, true),
                None => (theme, fixed),
            };
            let ui = Ui::new_boxed(width, height, FluentDrawer::new(theme), request.view);
            Some(Window {
                ui,
                theme,
                fixed,
                transparent: false,
            })
        };
        tomni_ui_platform::run_windows(options, window, factory)
    }
}

impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let views: Vec<&str> = self.views.iter().map(|v| v.name.as_str()).collect();
        f.debug_struct("App")
            .field("views", &views)
            .field("size", &(self.options.width, self.options.height))
            .field("theme", &self.theme)
            .field("backdrop", &self.options.backdrop)
            .finish_non_exhaustive()
    }
}

/// O `Ui` visto pela plataforma.
struct Window {
    ui: Ui,
    theme: Theme,
    /// Tema escolhido pelo app, que não segue o do sistema.
    fixed: bool,
    /// Janela transparente, com o Mica atrás.
    transparent: bool,
}

impl Window {
    fn apply_theme(&mut self, theme: Theme) {
        if theme != self.theme {
            self.theme = theme;
            let mut drawer = FluentDrawer::new(theme);
            drawer.set_window_transparent(self.transparent);
            self.ui.set_drawer(drawer);
        }
    }
}

impl Handler for Window {
    fn resize(&mut self, width: u32, height: u32) {
        self.ui.resize(width, height);
    }

    fn set_safe_area(&mut self, insets: Insets) {
        self.ui.set_safe_area(insets);
    }

    fn set_dark(&mut self, dark: bool) {
        if !self.fixed {
            self.apply_theme(if dark { Theme::Dark } else { Theme::Light });
        }
    }

    fn set_theme_override(&mut self, dark: Option<bool>) {
        self.fixed = dark.is_some();
        if let Some(dark) = dark {
            self.apply_theme(if dark { Theme::Dark } else { Theme::Light });
        }
    }

    fn set_clipboard(&mut self, clipboard: Box<dyn Clipboard>) {
        self.ui.set_clipboard(clipboard);
    }

    fn pointer_move(&mut self, point: Point, kind: PointerKind) {
        self.ui.pointer_move(point, kind);
    }

    fn pointer_down(&mut self, point: Point, kind: PointerKind, clicks: u8) {
        self.ui.pointer_down(point, kind, clicks);
    }

    fn pointer_up(&mut self, point: Point, kind: PointerKind) {
        self.ui.pointer_up(point, kind);
    }

    fn secondary_down(&mut self, point: Point) {
        self.ui.secondary_down(point);
    }

    fn secondary_up(&mut self, point: Point) {
        self.ui.secondary_up(point);
    }

    fn middle_down(&mut self, point: Point) {
        self.ui.middle_down(point);
    }

    fn middle_up(&mut self, point: Point) {
        self.ui.middle_up(point);
    }

    fn files_dropped(&mut self, paths: Vec<std::path::PathBuf>, point: Point) -> bool {
        self.ui.files_dropped(paths, point)
    }

    fn pointer_cancel(&mut self) {
        self.ui.pointer_cancel();
    }

    fn wheel(&mut self, point: Point, delta: Vec2) {
        self.ui.wheel(point, delta);
    }

    fn set_modifiers(&mut self, modifiers: Modifiers) {
        self.ui.set_modifiers(modifiers);
    }

    fn key_down(&mut self, key: Key) {
        self.ui.key_down(key);
    }

    fn key_up(&mut self, key: Key) {
        self.ui.key_up(key);
    }

    fn type_text(&mut self, text: &str) {
        self.ui.type_text(text);
    }

    fn ime(&mut self, ime: Ime) {
        self.ui.ime(ime);
    }

    fn set_caret_visible(&mut self, visible: bool) {
        self.ui.set_caret_visible(visible);
    }

    fn frame(&mut self) -> bool {
        self.ui.frame()
    }

    fn render(&self, canvas: &mut dyn Canvas) {
        self.ui.render(canvas);
    }

    fn background(&self) -> Color {
        if self.transparent {
            Color::TRANSPARENT
        } else {
            self.ui.drawer().tokens().solid_background_base
        }
    }

    fn title(&self) -> Option<String> {
        self.ui.title()
    }

    fn text_input(&mut self) -> Option<TextInput> {
        self.ui.text_input()
    }

    fn advance(&mut self, dt: std::time::Duration) {
        self.ui.advance(dt);
    }

    fn deadline(&self) -> Option<std::time::Duration> {
        self.ui.deadline()
    }

    fn deadline_reason(&self) -> Option<String> {
        self.ui.deadline_reason()
    }

    fn window_requests(&mut self) -> Vec<WindowRequest> {
        self.ui.take_window_requests()
    }

    fn resize_edge(&self, point: Point) -> Option<ResizeEdge> {
        self.ui.resize_edge(point)
    }

    fn cursor(&self, point: Point) -> Cursor {
        self.ui.cursor(point)
    }

    fn set_window_active(&mut self, active: bool) {
        self.ui.set_window_active(active);
    }

    fn has_title_bar(&self) -> bool {
        self.ui.has_title_bar()
    }

    fn drag_data(&self) -> Option<DragData> {
        self.ui.drag_data()
    }

    fn foreign_drag_over(&mut self, data: &DragData, point: Point) -> bool {
        self.ui.foreign_drag_over(data, point)
    }

    fn foreign_drag_leave(&mut self) {
        self.ui.foreign_drag_leave();
    }

    fn foreign_drop(&mut self, data: DragData, point: Point) -> bool {
        self.ui.foreign_drop(data, point)
    }

    fn cancel_drag(&mut self) {
        self.ui.cancel_drag();
    }

    fn drop_outside(&mut self, at: Option<(i32, i32)>) -> bool {
        self.ui.drop_outside(at)
    }

    fn set_maximized(&mut self, maximized: bool) {
        self.ui.set_maximized(maximized);
    }

    fn set_chrome(&mut self, chrome: WindowChrome) {
        self.ui.set_chrome(chrome);
    }

    fn set_transparent(&mut self, transparent: bool) {
        self.transparent = transparent;
        self.ui.drawer_mut().set_window_transparent(transparent);
        self.ui.invalidate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::*;

    fn login() -> tomni_ui_core::Form {
        tomni_ui_core::Form::new("Login")
    }

    #[test]
    fn nome_da_tela_e_o_da_funcao() {
        assert_eq!(NamedView::new(login).name, "login");
        let closure = || login();
        assert_eq!(NamedView::new(closure).name, "nome_da_tela_e_o_da_funcao");
    }

    fn tela() -> impl View {
        Form::new("Teste").child(Text::new("oi"))
    }

    #[test]
    fn o_app_guarda_o_icone_e_o_app_id_para_as_janelas() {
        let icone = WindowIcon::new(b"grande").with_small(b"pequeno");
        let app = App::new(tela).icon(icone).app_id("meu-app");
        assert_eq!(app.options.icon, Some(icone));
        assert_eq!(app.options.app_id.as_deref(), Some("meu-app"));
        // O runner clona estas opções para cada janela de `open_window`.
        let herdada = app.options.clone();
        assert_eq!(herdada.icon, Some(icone));
        assert_eq!(herdada.app_id.as_deref(), Some("meu-app"));
    }

    #[test]
    fn o_app_passa_o_arquivo_da_guarda_ao_runner() {
        let app = App::new(tela).render_guard("/tmp/guarda.txt");
        assert_eq!(
            app.options.render_guard.as_deref(),
            Some(std::path::Path::new("/tmp/guarda.txt"))
        );
    }

    #[test]
    fn o_app_passa_o_registro_de_travamentos_ao_runner() {
        assert!(App::new(tela).options.stall_log.is_none());
        let app = App::new(tela).stall_log("/tmp/travamentos.log");
        assert_eq!(
            app.options.stall_log.as_deref(),
            Some(std::path::Path::new("/tmp/travamentos.log"))
        );
    }

    #[test]
    fn sem_icone_nem_app_id_fica_o_do_sistema() {
        let app = App::new(tela);
        assert_eq!(app.options.icon, None);
        assert_eq!(app.options.app_id, None);
    }
}
