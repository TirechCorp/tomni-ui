//! `TestApp`: o `Ui` da fachada sem janela, dirigido por id.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::rc::Rc;

use tomni_ui::dialogs::{FileDialogAnswer, FileDialogRequest, FileDialogService};
use tomni_ui::kurbo::{Point, Rect, Vec2};
use tomni_ui::lint::{Issue, Severity};
use tomni_ui::reactive::untrack;
use tomni_ui::render::Pixmap;
use tomni_ui::terminal::cell::Cell;
use tomni_ui::terminal::{PanesInfo, Session, Terminal, TerminalPanes};
use tomni_ui::theme::{FluentDrawer, Theme};
use tomni_ui::tray::{TrayBackend, TrayImage, TrayState};
use tomni_ui::webview::{Fake, WebView};
use tomni_ui::windows;
use tomni_ui::{
    Clipboard, EditVisual, GridRowInfo, Ime, Insets, Key, Look, Modifiers, Node, NodeId,
    PointerKind, PropertyInfo, Shortcut, SortDirection, TextEditorVisual, TextInput, Ui, View,
    WindowChrome, WindowGeometry, WindowHandle, WindowRequest,
};

/// Onde um arrasto do [`TestApp`] começa ou termina: o id de um widget (o centro dele) ou um
/// ponto da janela.
#[derive(Clone, Debug, PartialEq)]
pub enum Spot {
    /// O centro do widget com este id.
    Id(String),
    /// Um ponto da janela.
    Point(Point),
}

impl From<&str> for Spot {
    fn from(id: &str) -> Self {
        Self::Id(id.to_string())
    }
}

impl From<String> for Spot {
    fn from(id: String) -> Self {
        Self::Id(id)
    }
}

impl From<&String> for Spot {
    fn from(id: &String) -> Self {
        Self::Id(id.clone())
    }
}

impl From<Point> for Spot {
    fn from(point: Point) -> Self {
        Self::Point(point)
    }
}

/// Atalho de teclado para [`TestApp::shortcut`]: uma letra (Ctrl, ou Cmd no macOS, mais a
/// letra) ou um [`Shortcut`] com os modificadores que quiser.
pub trait Keys {
    /// A tecla e os modificadores.
    fn keys(self) -> (Key, Modifiers);
}

impl Keys for char {
    fn keys(self) -> (Key, Modifiers) {
        (Key::from(self), Modifiers::COMMAND)
    }
}

impl Keys for Shortcut {
    fn keys(self) -> (Key, Modifiers) {
        (self.key, self.modifiers)
    }
}

/// Uma linha do `TreeView`, como [`TestApp::tree_items`] a dá.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeRowInfo {
    /// Chave do item.
    pub key: String,
    /// Rótulo.
    pub label: String,
    /// Nível: 0 na raiz.
    pub depth: usize,
    /// Se a pasta está aberta.
    pub expanded: bool,
    /// Se é o item selecionado.
    pub selected: bool,
}

/// Uma coluna do `DataGrid`, como [`TestApp::grid`] a dá.
#[derive(Clone, Debug, PartialEq)]
pub struct GridColumnInfo {
    /// Chave da coluna.
    pub key: String,
    /// Título.
    pub header: String,
    /// Largura de hoje, em px.
    pub width: f64,
}

/// O `DataGrid`, como [`TestApp::grid`] o dá.
#[derive(Clone, Debug, PartialEq)]
pub struct DataGridInfo {
    /// As colunas, na ordem, com a largura de hoje.
    pub columns: Vec<GridColumnInfo>,
    /// Todas as linhas que passam do filtro, na ordem da tela (inclusive as que a janela de
    /// virtualização não montou): a chave, o texto de cada célula e se está selecionada.
    pub rows: Vec<GridRowInfo>,
    /// As chaves selecionadas que aparecem, na ordem das linhas.
    pub selected: Vec<String>,
    /// A chave da coluna ordenada e o sentido.
    pub sort: Option<(String, SortDirection)>,
    /// O texto do filtro.
    pub filter: String,
}

impl DataGridInfo {
    /// As chaves das linhas, na ordem da tela.
    pub fn keys(&self) -> Vec<&str> {
        self.rows.iter().map(|row| row.key.as_str()).collect()
    }
}

/// O seletor de arquivo do [`TestApp`]: guarda os pedidos e responde quando o teste manda.
#[derive(Default)]
struct FakeFileDialogs {
    /// Todos os pedidos feitos, na ordem.
    requests: RefCell<Vec<FileDialogRequest>>,
    /// Os pedidos que esperam resposta, do mais antigo ao mais novo.
    waiting: RefCell<VecDeque<FileDialogAnswer>>,
    /// Respostas dadas antes do pedido, para os próximos pedidos.
    early: RefCell<VecDeque<Vec<PathBuf>>>,
}

impl FileDialogService for FakeFileDialogs {
    fn show(&self, request: FileDialogRequest, answer: FileDialogAnswer) {
        self.requests.borrow_mut().push(request);
        let early = self.early.borrow_mut().pop_front();
        match early {
            Some(paths) => answer.send_many(paths),
            None => self.waiting.borrow_mut().push_back(answer),
        }
    }
}

/// A bandeja do [`TestApp`]: guarda o que o app mostrou, por bandeja.
#[derive(Default)]
struct FakeTray {
    shown: RefCell<std::collections::BTreeMap<u64, TrayState>>,
}

impl TrayBackend for FakeTray {
    fn show(&self, id: u64, _icon: &TrayImage, state: &TrayState, _activatable: bool) {
        self.shown.borrow_mut().insert(id, state.clone());
    }

    fn remove(&self, id: u64) {
        self.shown.borrow_mut().remove(&id);
    }
}

impl std::fmt::Debug for FakeTray {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeTray")
            .field("trays", &self.shown.borrow().len())
            .finish_non_exhaustive()
    }
}

/// Tamanho padrão da janela de teste.
const DEFAULT_SIZE: (u32, u32) = (400, 300);

/// App headless para testes: manda eventos como a plataforma mandaria e roda um frame depois
/// de cada um.
///
/// Os repaints da montagem (e dos builders [`TestApp::size`] e [`TestApp::theme`]) são
/// descartados: [`TestApp::repaints_since_last`] começa vazio. As ações (`click`, `type_into`,
/// `key`...) devolvem o próprio app e podem ser encadeadas.
///
/// ```
/// use tomni_ui::prelude::*;
/// use tomni_ui_test::TestApp;
///
/// let mut app = TestApp::new(|| {
///     let n = signal(0);
///     Form::new("Contador").child(
///         Column::new()
///             .child(Button::new(move || format!("Cliques: {}", n.get())).id("rotulo"))
///             .child(Button::new("+1").id("mais").on_click(move || n.update(|n| *n += 1))),
///     )
/// });
/// app.click("mais");
/// assert_eq!(app.label("rotulo"), "Cliques: 1");
/// assert_eq!(app.repaints_since_last(), ["rotulo", "mais"]);
/// ```
#[derive(Debug)]
pub struct TestApp {
    /// A tela da janela atual ([`TestApp::current_window`]); as das outras ficam em `others`.
    ui: Ui,
    /// Onde o ponteiro foi pressionado por [`TestApp::press`] e ainda não soltou.
    pressed: Option<Point>,
    /// O que a `TitleBar` pediu à janela, desde a última [`TestApp::window_requests`].
    requests: Vec<WindowRequest>,
    /// O seletor de arquivo falso, instalado em `dialogs` desta thread.
    file_dialogs: Rc<FakeFileDialogs>,
    /// A bandeja falsa, instalada em `tray` desta thread.
    tray: Rc<FakeTray>,
    /// A janela de `ui`.
    current: WindowHandle,
    /// As janelas que não são a atual (as abertas por `open_window`).
    others: Vec<Slot>,
    /// As janelas que o app pediu para chamar a atenção e que o teste ainda não leu.
    attentions: Vec<WindowHandle>,
    /// Tamanho e tema das janelas que a tela abre.
    default_size: (u32, u32),
    theme: Theme,
    /// Se o sistema informa onde as janelas estão (falso no Wayland).
    positions: bool,
    /// A janela principal fechou: o app acabou.
    exited: bool,
}

/// O que se guarda de uma janela que não é a atual.
#[derive(Debug)]
struct Slot {
    handle: WindowHandle,
    ui: Ui,
    pressed: Option<Point>,
    requests: Vec<WindowRequest>,
}

impl std::fmt::Debug for FakeFileDialogs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeFileDialogs")
            .field("requests", &self.requests.borrow().len())
            .finish_non_exhaustive()
    }
}

impl TestApp {
    /// Monta `view` numa janela de 400 × 300 com o tema claro.
    pub fn new<V: View>(view: impl FnOnce() -> V) -> Self {
        let (width, height) = DEFAULT_SIZE;
        let file_dialogs = Rc::new(FakeFileDialogs::default());
        tomni_ui::dialogs::set_service(file_dialogs.clone());
        windows::reset();
        tomni_ui::tray::reset();
        // A bandeja falsa já está lá quando a tela monta, como a do sistema depois de a janela
        // principal abrir.
        let tray = Rc::new(FakeTray::default());
        tomni_ui::tray::set_backend(tray.clone(), true);
        let mut ui = Ui::new(width, height, FluentDrawer::new(Theme::Light), view);
        ui.repaints_since_last();
        windows::set_geometry(
            WindowHandle::main(),
            WindowGeometry {
                position: Some((0, 0)),
                size: (width, height),
            },
        );
        let mut app = Self {
            ui,
            pressed: None,
            requests: Vec::new(),
            file_dialogs,
            tray,
            current: WindowHandle::main(),
            others: Vec::new(),
            attentions: Vec::new(),
            default_size: DEFAULT_SIZE,
            theme: Theme::Light,
            positions: true,
            exited: false,
        };
        // Janelas que a montagem da tela já pediu.
        app.open_pending();
        app
    }

    /// Muda o tamanho da janela. As que a tela abrir depois nascem com ele.
    pub fn size(mut self, width: u32, height: u32) -> Self {
        self.default_size = (width, height);
        self.resize(width, height);
        self.ui.repaints_since_last();
        self
    }

    /// O tema que está valendo agora: o do [`TestApp::theme`] ou o último de
    /// [`tomni_ui::set_theme`].
    pub fn current_theme(&self) -> Theme {
        self.theme
    }

    /// Troca o tema como `tomni_ui::set_theme`: `Some` fixa e `None` volta ao do sistema, que
    /// no teste é o claro.
    pub fn set_theme(&mut self, theme: Option<Theme>) {
        tomni_ui::set_theme(theme);
        self.step();
        self.ui.repaints_since_last();
    }

    /// Troca o tema. As janelas que a tela abrir depois nascem com ele.
    pub fn theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self.ui.set_drawer(FluentDrawer::new(theme));
        self.step();
        self.ui.repaints_since_last();
        self
    }

    /// `Ui` por baixo, para o que não tem atalho aqui.
    pub fn ui(&mut self) -> &mut Ui {
        &mut self.ui
    }

    /// Muda o tamanho da janela e roda um frame.
    pub fn resize(&mut self, width: u32, height: u32) -> &mut Self {
        self.ui.resize(width, height);
        self.step();
        self
    }

    /// Muda a safe area e roda um frame.
    pub fn safe_area(&mut self, insets: Insets) {
        self.ui.set_safe_area(insets);
        self.step();
    }

    /// Responde o pedido de seletor de arquivo mais antigo que espera resposta: `Some(caminho)`
    /// para a pessoa escolher, `None` para cancelar (`None::<&str>`; veja também
    /// [`TestApp::file_dialog_cancel`]). Sem pedido esperando, a resposta fica guardada para o
    /// próximo. O callback do app roda no frame que isto dispara, como na plataforma.
    ///
    /// ```
    /// use tomni_ui::prelude::*;
    /// use tomni_ui_test::TestApp;
    ///
    /// let mut app = TestApp::new(|| {
    ///     let caminho = signal(String::new());
    ///     Form::new("Arquivo").child(
    ///         Column::new()
    ///             .child(Button::new("Procurar").id("procurar").on_click(move || {
    ///                 tomni_ui::dialogs::open_file().show(move |p| {
    ///                     if let Some(p) = p {
    ///                         caminho.set(p.display().to_string());
    ///                     }
    ///                 })
    ///             }))
    ///             .child(Text::new(move || caminho.get()).id("caminho")),
    ///     )
    /// });
    /// app.click("procurar");
    /// app.file_dialog_answer(Some("/tmp/a.pem"));
    /// assert_eq!(app.text("caminho"), "/tmp/a.pem");
    /// ```
    pub fn file_dialog_answer(&mut self, path: Option<impl Into<PathBuf>>) -> &mut Self {
        self.file_dialog_answer_many(path)
    }

    /// Responde o pedido mais antigo com vários arquivos, como a pessoa que marca mais de um no
    /// `dialogs::open_files()`. Vazio é cancelar.
    pub fn file_dialog_answer_many(
        &mut self,
        paths: impl IntoIterator<Item = impl Into<PathBuf>>,
    ) -> &mut Self {
        let paths: Vec<PathBuf> = paths.into_iter().map(Into::into).collect();
        let waiting = self.file_dialogs.waiting.borrow_mut().pop_front();
        match waiting {
            Some(answer) => answer.send_many(paths),
            None => self.file_dialogs.early.borrow_mut().push_back(paths),
        }
        self.step();
        self
    }

    /// Cancela o pedido de seletor de arquivo mais antigo: o mesmo que
    /// `file_dialog_answer(None::<&str>)`.
    pub fn file_dialog_cancel(&mut self) -> &mut Self {
        self.file_dialog_answer(None::<&str>)
    }

    /// Todos os pedidos de seletor de arquivo que o app fez, na ordem (título, filtros, pasta).
    pub fn file_dialogs(&self) -> Vec<FileDialogRequest> {
        self.file_dialogs.requests.borrow().clone()
    }

    /// A bandeja que o app mostra agora (a primeira, se mostrar mais de uma): a dica e os itens
    /// do menu, como o sistema os veria. `None` se o app não criou nenhuma ou se o sistema não
    /// tem bandeja ([`TestApp::set_tray_available`]).
    ///
    /// ```
    /// use tomni_ui::prelude::*;
    /// use tomni_ui::tray::{Tray, TrayImage};
    /// use tomni_ui_test::TestApp;
    ///
    /// let mut app = TestApp::new(|| {
    ///     let n = signal(0);
    ///     Tray::new(TrayImage::from_png(&[]))
    ///         .tooltip(move || format!("Cliques: {}", n.get()))
    ///         .item("somar", || "Somar".to_string(), move || n.update(|n| *n += 1))
    ///         .show()
    ///         .keep();
    ///     Form::new("Contador")
    /// });
    /// app.frame();
    /// app.tray_click("somar");
    /// assert_eq!(app.tray().unwrap().tooltip, "Cliques: 1");
    /// ```
    pub fn tray(&self) -> Option<TrayState> {
        if !tomni_ui::reactive::untrack(tomni_ui::tray::available) {
            return None;
        }
        self.tray.shown.borrow().values().next().cloned()
    }

    /// Diz se o sistema tem bandeja (começa que tem). Sem ela, [`TestApp::tray`] devolve
    /// `None` e `tomni_ui::tray::available()` fica `false`, o caso do GNOME sem a extensão.
    pub fn set_tray_available(&mut self, available: bool) -> &mut Self {
        tomni_ui::tray::set_available(available);
        self.step();
        self
    }

    /// Clica no item `id` do menu da bandeja. Item desabilitado não faz nada, como no sistema.
    ///
    /// # Panics
    ///
    /// Se o app não mostra bandeja agora, ou ela não tem um item com esse `id`.
    pub fn tray_click(&mut self, id: &str) -> &mut Self {
        let (tray, state) = self.shown_tray();
        assert!(
            state.items.iter().any(|item| item.id == id),
            "a bandeja não tem o item {id:?}; tem {:?}",
            state
                .items
                .iter()
                .map(|i| i.id.as_str())
                .collect::<Vec<_>>()
        );
        tomni_ui::tray::sender().item(tray, id);
        self.step();
        self
    }

    /// Clica no ícone da bandeja (o clique esquerdo no Windows e no Linux).
    ///
    /// # Panics
    ///
    /// Se o app não mostra bandeja agora.
    pub fn tray_activate(&mut self) -> &mut Self {
        let (tray, _) = self.shown_tray();
        tomni_ui::tray::sender().activate(tray);
        self.step();
        self
    }

    fn shown_tray(&self) -> (u64, TrayState) {
        assert!(
            tomni_ui::reactive::untrack(tomni_ui::tray::available),
            "o sistema não tem bandeja (set_tray_available(false))"
        );
        let shown = self.tray.shown.borrow();
        let (id, state) = shown
            .iter()
            .next()
            .expect("o app não mostra nenhuma bandeja");
        (*id, state.clone())
    }

    /// Roda um frame sem evento nenhum.
    pub fn frame(&mut self) {
        self.step();
    }

    /// O runner falso: atende os pedidos da `TitleBar` à janela, como a plataforma faria
    /// depois do evento, e roda um frame. Maximizar troca o estado; minimizar, fechar, arrastar
    /// e redimensionar só ficam registrados.
    fn step(&mut self) {
        windows::set_current(self.current);
        self.apply_theme_request();
        let mut close = false;
        for request in self.ui.take_window_requests() {
            if request == WindowRequest::ToggleMaximize {
                let maximized = self.ui.is_maximized();
                self.ui.set_maximized(!maximized);
            }
            close |= request == WindowRequest::Close;
            self.requests.push(request);
        }
        pump_terminals(&mut self.ui);
        pump_webviews(&mut self.ui);
        self.ui.frame();
        // As outras janelas também rodam o frame, cada uma como a janela atual.
        for slot in &mut self.others {
            windows::set_current(slot.handle);
            for request in slot.ui.take_window_requests() {
                if request == WindowRequest::ToggleMaximize {
                    let maximized = slot.ui.is_maximized();
                    slot.ui.set_maximized(!maximized);
                }
                slot.requests.push(request);
            }
            pump_terminals(&mut slot.ui);
            pump_webviews(&mut slot.ui);
            slot.ui.frame();
        }
        windows::set_current(self.current);
        if close {
            self.close_window(self.current);
        }
        self.open_pending();
    }

    /// Aplica o `tomni_ui::set_theme` que o app pediu, a todas as janelas, como o runner faz.
    fn apply_theme_request(&mut self) {
        let Some(dark) = windows::take_theme_request() else {
            return;
        };
        self.theme = if dark == Some(true) {
            Theme::Dark
        } else {
            Theme::Light
        };
        self.ui.set_drawer(FluentDrawer::new(self.theme));
        for slot in &mut self.others {
            slot.ui.set_drawer(FluentDrawer::new(self.theme));
        }
    }

    /// Atende o que o app pediu às janelas ([`open_window`](tomni_ui::open_window),
    /// [`WindowHandle::close`], [`WindowHandle::focus`]), como o runner faria.
    fn open_pending(&mut self) {
        loop {
            let opens = windows::take_open_requests();
            let closes = windows::take_close_requests();
            let visibility = windows::take_visibility_requests();
            let focuses = windows::take_focus_requests();
            let attentions = windows::take_attention_requests();
            let nothing_else = opens.is_empty()
                && closes.is_empty()
                && visibility.is_empty()
                && focuses.is_empty();
            if nothing_else && attentions.is_empty() {
                break;
            }
            self.attentions.extend(attentions);
            for request in opens {
                let handle = request.handle;
                let (width, height) = request.size.unwrap_or(self.default_size);
                windows::set_current(handle);
                let mut ui =
                    Ui::new_boxed(width, height, FluentDrawer::new(self.theme), request.view);
                ui.repaints_since_last();
                if let Some(title) = ui.title() {
                    windows::set_title(handle, &title);
                }
                windows::set_geometry(
                    handle,
                    WindowGeometry {
                        position: if self.positions {
                            request.position
                        } else {
                            None
                        },
                        size: (width, height),
                    },
                );
                self.others.push(Slot {
                    handle,
                    ui,
                    pressed: None,
                    requests: Vec::new(),
                });
            }
            windows::set_current(self.current);
            for handle in closes {
                self.close_window(handle);
            }
            // Esconder e mostrar já valem em `is_hidden`; mostrar também traz a janela à frente.
            for (handle, visible) in visibility {
                if visible && handle.is_open() {
                    self.set_active_window(handle);
                }
            }
            for handle in focuses {
                self.set_active_window(handle);
            }
        }
        windows::set_current(self.current);
    }

    /// As janelas abertas, na ordem em que abriram (a primeira é a principal). Inclui as que o
    /// app abriu com [`open_window`](tomni_ui::open_window).
    pub fn windows(&self) -> Vec<WindowHandle> {
        WindowHandle::all()
    }

    /// Se a janela está escondida ([`WindowHandle::hide`], `App::start_hidden`). O `TestApp` não
    /// tem tela de verdade: esconder só muda isto, e [`WindowHandle::show`] também torna a janela
    /// a ativa.
    pub fn window_hidden(&self, handle: WindowHandle) -> bool {
        handle.is_hidden()
    }

    /// A janela em que os métodos agem agora. Começa na principal.
    pub fn current_window(&self) -> WindowHandle {
        self.current
    }

    /// Passa a agir sobre a janela `handle`: clicar, digitar, `text`, `rect`, `snapshot`,
    /// `lint` e o resto valem para ela daí em diante.
    ///
    /// # Panics
    ///
    /// Se a janela não existe.
    #[track_caller]
    pub fn switch_to(&mut self, handle: WindowHandle) -> &mut Self {
        if handle == self.current {
            return self;
        }
        let slot = self
            .others
            .iter_mut()
            .find(|slot| slot.handle == handle)
            .unwrap_or_else(|| panic!("a janela {handle:?} não existe"));
        std::mem::swap(&mut self.ui, &mut slot.ui);
        std::mem::swap(&mut self.pressed, &mut slot.pressed);
        std::mem::swap(&mut self.requests, &mut slot.requests);
        std::mem::swap(&mut self.current, &mut slot.handle);
        windows::set_current(self.current);
        self
    }

    /// Pede para fechar a janela `handle`, como o botão de fechar faria: passa pelos
    /// [`on_close_requested`](tomni_ui::on_close_requested) dela (os de todas as janelas, sendo
    /// a principal). Devolve `false` se algum recusou; a janela fica. Fechar a principal fecha
    /// todas e encerra o app ([`TestApp::exited`]).
    pub fn close_window(&mut self, handle: WindowHandle) -> bool {
        if !handle.is_open() {
            return true;
        }
        if !windows::confirm_close(handle) {
            self.step_after_close();
            return false;
        }
        let closed = windows::finish_close(handle);
        if closed.contains(&WindowHandle::main()) {
            self.exited = true;
        }
        if closed.contains(&self.current) && !self.exited {
            let next = self
                .others
                .iter()
                .map(|slot| slot.handle)
                .find(|h| !closed.contains(h))
                .expect("a principal continua aberta");
            self.switch_to(next);
        }
        self.others.retain(|slot| !closed.contains(&slot.handle));
        windows::set_current(self.current);
        self.step_after_close();
        true
    }

    /// Roda um frame depois de um pedido de fechar, para o que os ganchos mudaram aparecer.
    fn step_after_close(&mut self) {
        if !self.exited {
            self.step();
        }
    }

    /// Se o app terminou: a janela principal foi fechada.
    pub fn exited(&self) -> bool {
        self.exited
    }

    /// Simula o foco do sistema indo para a janela `handle`: ela vira a
    /// [`active_window`](tomni_ui::active_window), as outras ficam inativas.
    pub fn set_active_window(&mut self, handle: WindowHandle) -> &mut Self {
        if !handle.is_open() {
            return self;
        }
        windows::set_active(handle);
        self.ui.set_window_active(self.current == handle);
        for slot in &mut self.others {
            slot.ui.set_window_active(slot.handle == handle);
        }
        self.step();
        self
    }

    /// Simula o Wayland: o sistema não informa onde as janelas estão, então uma aba arrastada
    /// para fora da janela de origem nunca chega a outra janela, e o soltar é sempre um
    /// tear-off, sem o ponto na tela.
    pub fn without_window_positions(mut self) -> Self {
        self.positions = false;
        self
    }

    /// Arrasta a aba `tab` da janela atual até `target` (um id ou um ponto) na janela `window`,
    /// e solta ali: a janela de destino recebe o `on_tab_drop`. Sem posições das janelas
    /// ([`TestApp::without_window_positions`]) a aba cai fora de todas, como no Wayland. Termina
    /// na janela de origem, ou na de destino se a origem fechou por ficar sem abas.
    ///
    /// # Panics
    ///
    /// Se a janela de destino não existe ou se o id não é de um widget visível.
    #[track_caller]
    pub fn drag_tab_to_window(
        &mut self,
        tab: &str,
        window: WindowHandle,
        target: impl Into<Spot>,
    ) -> &mut Self {
        assert!(window.is_open(), "a janela {window:?} não existe");
        assert_ne!(window, self.current, "o destino é a janela de origem");
        let source = self.current;
        let target = target.into();
        let from = self.target(tab);
        let away = self.outside_point();
        self.ui.pointer_move(from, PointerKind::Mouse);
        self.ui.pointer_down(from, PointerKind::Mouse, 1);
        self.step();
        self.ui.pointer_move(away, PointerKind::Mouse);
        self.step();
        let data = self.ui.drag_data().unwrap_or_else(|| {
            panic!(
                "{tab:?} não começou um arrasto (a aba precisa de `.drag_group` ou `.on_reorder`)"
            )
        });
        if self.positions {
            self.switch_to(window);
            let point = match target {
                Spot::Point(point) => point,
                Spot::Id(id) => self.target(&id),
            };
            self.ui.foreign_drag_over(&data, point);
            self.step();
            self.ui.foreign_drop(data, point);
            self.step();
            // A janela de origem pode ter fechado ao ficar sem abas.
            if !source.is_open() {
                return self;
            }
            self.switch_to(source);
            self.ui.cancel_drag();
        } else {
            self.ui.drop_outside(None);
        }
        self.ui.pointer_up(away, PointerKind::Mouse);
        self.step();
        self
    }

    /// Arrasta a aba `tab` da janela atual e solta fora de todas as janelas (o tear-off): a
    /// aba recebe o `on_tab_drop_outside`, com um ponto da tela, ou `None` sem posições das
    /// janelas ([`TestApp::without_window_positions`]).
    #[track_caller]
    pub fn drag_tab_outside(&mut self, tab: &str) -> &mut Self {
        let from = self.target(tab);
        let away = self.outside_point();
        self.ui.pointer_move(from, PointerKind::Mouse);
        self.ui.pointer_down(from, PointerKind::Mouse, 1);
        self.step();
        self.ui.pointer_move(away, PointerKind::Mouse);
        self.step();
        assert!(
            self.ui.drag_data().is_some(),
            "{tab:?} não começou um arrasto (a aba precisa de `.drag_group` ou `.on_reorder`)"
        );
        let at = self.positions.then_some((10_000, 200));
        self.ui.drop_outside(at);
        self.ui.pointer_up(away, PointerKind::Mouse);
        self.step();
        self
    }

    /// Um ponto fora da janela atual, para onde a aba é levada.
    fn outside_point(&self) -> Point {
        let (width, height) = self.ui.size();
        Point::new(f64::from(width) + 40.0, f64::from(height) / 2.0)
    }

    /// O motor falso do `WebView` `id`.
    fn webview<R>(&self, id: &str, f: impl FnOnce(&Fake) -> R) -> R {
        let tree = self.ui.tree();
        let widget = tree.widget(self.node(id));
        let fake = widget
            .as_any()
            .and_then(|any| any.downcast_ref::<WebView>())
            .unwrap_or_else(|| panic!("{id:?} não é um WebView"))
            .fake()
            .unwrap_or_else(|| panic!("o WebView {id:?} ainda não tem motor"));
        untrack(|| f(fake))
    }

    /// Mexe no motor falso do `WebView` `id` e roda um passo.
    fn webview_mut<R>(&mut self, id: &str, f: impl FnOnce(&mut Fake) -> R) -> R {
        let node = self.node(id);
        let result = self.ui.with_widget_mut(node, |widget| {
            widget
                .as_any_mut()
                .and_then(|any| any.downcast_mut::<WebView>())
                .and_then(WebView::fake_mut)
                .map(|fake| untrack(|| f(fake)))
        });
        let result = result.unwrap_or_else(|| panic!("{id:?} não é um WebView com motor"));
        self.step();
        result
    }

    /// O `Terminal` `id`.
    fn terminal<R>(&self, id: &str, f: impl FnOnce(&Terminal) -> R) -> R {
        let tree = self.ui.tree();
        let widget = tree.widget(self.node(id));
        let terminal = widget
            .as_any()
            .and_then(|any| any.downcast_ref::<Terminal>())
            .unwrap_or_else(|| panic!("{id:?} não é um Terminal"));
        untrack(|| f(terminal))
    }

    /// Se o widget com foco é um `Terminal`.
    fn terminal_focused(&self) -> bool {
        let tree = self.ui.tree();
        tree.focused()
            .is_some_and(|id| tree.widget(id).kind() == "terminal")
    }

    fn editor_focused(&self) -> bool {
        let tree = self.ui.tree();
        tree.focused()
            .is_some_and(|id| tree.widget(id).kind() == "text_editor")
    }

    /// O que a `TitleBar` pediu à janela desde a última chamada, na ordem: o duplo clique na
    /// barra pede [`WindowRequest::ToggleMaximize`], arrastar pede [`WindowRequest::Drag`].
    ///
    /// ```
    /// use tomni_ui::prelude::*;
    /// use tomni_ui::WindowRequest;
    /// use tomni_ui_test::TestApp;
    ///
    /// let mut app = TestApp::new(|| {
    ///     Form::new("Notas")
    ///         .title_bar(TitleBar::new("Notas"))
    ///         .child(Text::new("Nenhuma nota"))
    /// });
    /// app.click("barra-de-titulo-fechar");
    /// assert_eq!(app.window_requests(), [WindowRequest::Close]);
    /// app.double_click("barra-de-titulo");
    /// assert!(app.is_maximized());
    /// ```
    pub fn window_requests(&mut self) -> Vec<WindowRequest> {
        std::mem::take(&mut self.requests)
    }

    /// As janelas para as quais o app chamou [`WindowHandle::request_attention`] desde a última
    /// chamada, na ordem. Como no runner, só conta janela que existe.
    pub fn attention_requests(&mut self) -> Vec<WindowHandle> {
        std::mem::take(&mut self.attentions)
    }

    /// Se a janela está maximizada (pelo [`WindowRequest::ToggleMaximize`] da `TitleBar` ou
    /// por [`TestApp::set_maximized`]).
    pub fn is_maximized(&self) -> bool {
        self.ui.is_maximized()
    }

    /// Maximiza ou restaura a janela, como o sistema faria, e roda um frame.
    pub fn set_maximized(&mut self, maximized: bool) -> &mut Self {
        self.ui.set_maximized(maximized);
        self.step();
        self
    }

    /// Ativa ou desativa a janela (o foco em outro app) e roda um frame: inativa, a
    /// `TitleBar` fica apagada.
    pub fn set_window_active(&mut self, active: bool) -> &mut Self {
        self.ui.set_window_active(active);
        self.step();
        self
    }

    /// Quem desenha a moldura: [`WindowChrome::System`] simula a plataforma que não deixa
    /// tirar a barra do sistema (a `TitleBar` fica sem os botões da janela e não arrasta).
    pub fn chrome(mut self, chrome: WindowChrome) -> Self {
        self.ui.set_chrome(chrome);
        self.step();
        self.ui.repaints_since_last();
        self.requests.clear();
        self
    }

    fn node(&self, id: &str) -> NodeId {
        self.ui
            .find(id)
            .unwrap_or_else(|| panic!("nenhum widget com id {id:?}"))
    }

    /// Retângulo do widget `id`.
    ///
    /// # Panics
    ///
    /// Se não houver widget com esse id (vale para todos os métodos que recebem `id`).
    pub fn rect(&self, id: &str) -> Rect {
        self.ui.rect(self.node(id))
    }

    /// Visual atual do widget `id`, como é pintado (um `Text` com a cor herdada do card).
    pub fn look(&self, id: &str) -> Look {
        self.ui.look(self.node(id))
    }

    fn kind(&self, id: &str) -> &'static str {
        self.ui.tree().widget(self.node(id)).kind()
    }

    /// Como o campo de texto `id` está sendo exibido: texto (mascarado no modo senha),
    /// cursor, seleção, composição do IME e rolagem. O cursor vem do estado do campo, aceso ou
    /// apagado no pisca.
    pub fn edit(&self, id: &str) -> EditVisual {
        match self.ui.tree().look(self.node(id)) {
            Look::Edit(visual) => visual,
            look => panic!("{id:?} não é um campo de texto: {look:?}"),
        }
    }

    /// Como o `TextEditor` `id` está sendo exibido: texto, cursor, seleção, marcas, rolagem e
    /// estado. O cursor vem do estado do editor, aceso ou apagado no pisca.
    pub fn text_editor(&self, id: &str) -> TextEditorVisual {
        match self.ui.tree().look(self.node(id)) {
            Look::TextEditor(visual) => visual,
            look => panic!("{id:?} não é um TextEditor: {look:?}"),
        }
    }

    /// Rótulo do botão, do `ToggleButton`, da caixa de seleção, da opção do `RadioButtons`, do item de menu, do
    /// item do `ComboBox` ou das sugestões do `AutoSuggestBox`, o título da aba `id` ou o rótulo
    /// do item do `NavigationView`; o cabeçalho do `ToggleSwitch`, do `Slider`, do `Expander` ou
    /// do `RadioButtons`.
    pub fn label(&self, id: &str) -> String {
        match self.look(id) {
            Look::Button { label, .. }
            | Look::ToggleButton { label, .. }
            | Look::CheckBox { label, .. }
            | Look::RadioButton { label, .. }
            | Look::MenuItem { label, .. }
            | Look::MenuBarItem { label, .. }
            | Look::ComboBoxItem { label, .. }
            | Look::TabHeader { label, .. }
            | Look::NavigationItem { label, .. } => label,
            Look::ToggleSwitch { header, .. } | Look::Slider { header, .. } => header,
            Look::Expander(_) => self.text(id),
            look => self
                .tree()
                .find(id)
                .and_then(|node| node.label.clone())
                .unwrap_or_else(|| panic!("{id:?} não tem rótulo: {look:?}")),
        }
    }

    /// Se a `CheckBox` `id` está marcada, o `ToggleButton` ou o `ToggleSwitch` `id` está ligado,
    /// a opção `id` do `RadioButtons` está marcada, o item de marcar (`.toggle`) ou de rádio
    /// (`.radio`) `id` do `MenuFlyout` está marcado ou a aba ou o item do `NavigationView` `id`
    /// é o escolhido (num grupo fechado, ou no painel compacto, o grupo do escolhido).
    pub fn checked(&self, id: &str) -> bool {
        match self.look(id) {
            Look::CheckBox { visual, .. } => visual.checked,
            Look::ToggleButton { visual, .. } => visual.checked,
            Look::ToggleSwitch { visual, .. } => visual.on,
            Look::RadioButton { visual, .. } => visual.checked,
            Look::MenuItem { visual, .. } => match visual.kind {
                tomni_ui::MenuItemKind::Toggle { checked }
                | tomni_ui::MenuItemKind::Radio { checked } => checked,
                kind => panic!("{id:?} é um item de menu sem marca ({kind:?})"),
            },
            Look::TabHeader { visual, .. } => visual.selected,
            Look::NavigationItem { visual, .. } => visual.selected,
            look => panic!(
                "{id:?} não é uma CheckBox, um ToggleButton, um ToggleSwitch, uma opção de RadioButtons, um item de menu de marcar, uma aba nem um item do NavigationView: {look:?}"
            ),
        }
    }

    /// Valor do `Slider` ou do `NumberBox` `id`, o andamento da `ProgressBar`/`ProgressRing`
    /// `id`, de 0 a 1, ou o tamanho do primeiro painel do `Split` `id` (ou do divisor dele), em
    /// pixels.
    ///
    /// # Panics
    ///
    /// Se `id` não for um desses, se a barra ou o anel estiver no modo indeterminado (sem
    /// `.value`) ou se o `NumberBox` estiver vazio ou sem um número: aí o [`Node`] de
    /// [`tree`](TestApp::tree) tem `value: None`, e [`text`](TestApp::text) dá o texto.
    pub fn value(&self, id: &str) -> f64 {
        if matches!(self.kind(id), "split" | "split_sizer") {
            return self
                .text(id)
                .parse()
                .expect("o Split dá o tamanho como número");
        }
        if self.kind(id) == "number_box" {
            let text = self.text(id);
            return tomni_ui::parse_number(&text)
                .unwrap_or_else(|| panic!("{id:?} não tem um número: {text:?}"));
        }
        match self.look(id) {
            Look::Slider { .. } => self
                .text(id)
                .parse()
                .expect("o Slider dá o valor como número"),
            Look::ProgressBar(visual) | Look::ProgressRing(visual) => visual
                .value
                .unwrap_or_else(|| panic!("{id:?} está no modo indeterminado")),
            look => panic!(
                "{id:?} não é um Slider, um NumberBox, uma ProgressBar, um ProgressRing nem um Split: {look:?}"
            ),
        }
    }

    /// Leva o `Slider` ou o `NumberBox` `id` a `value` como uma pessoa faria. No `Slider`,
    /// clica na trilha no ponto do valor, e ele arredonda para o passo dele (`.step`) e para
    /// dentro da faixa. No `NumberBox`, clica no campo, seleciona tudo, digita o número (com
    /// vírgula) e confirma com Enter, e ele arredonda para as casas decimais (`.precision`) e
    /// para dentro de `.min` e `.max`.
    ///
    /// ```
    /// use tomni_ui::prelude::*;
    /// use tomni_ui_test::TestApp;
    ///
    /// let mut app = TestApp::new(|| {
    ///     let volume = signal(40.0);
    ///     Form::new("Som").child(Slider::bind(volume, 0..=100).step(5.0).id("volume"))
    /// });
    /// app.set_value("volume", 72.0);
    /// assert_eq!(app.value("volume"), 70.0);
    /// ```
    ///
    /// # Panics
    ///
    /// Se `id` não for um `Slider` nem um `NumberBox`, ou estiver desabilitado.
    pub fn set_value(&mut self, id: &str, value: f64) -> &mut Self {
        if self.kind(id) == "number_box" {
            assert!(self.enabled(id), "{id:?} está desabilitado");
            let text = value.to_string().replace('.', ",");
            self.click(id)
                .shortcut('a')
                .type_text(&text)
                .key(Key::Enter);
            return self;
        }
        let node = self.node(id);
        assert!(
            untrack(|| self.ui.tree().widget(node).range()).is_some(),
            "{id:?} não é um Slider"
        );
        assert!(self.enabled(id), "{id:?} está desabilitado");
        self.target(id);
        let point = self.ui.slider_point(node, value).expect("é um Slider");
        self.click_at(point, 1);
        self
    }

    /// Se o widget `id` aparece: `false` com `.visible(false)` nele ou num contêiner em
    /// volta. Oculto, ele não ocupa espaço, não recebe foco nem ponteiro e o lint o ignora.
    pub fn visible(&self, id: &str) -> bool {
        self.ui.shown(self.node(id))
    }

    /// Se alguma parte do widget `id` aparece na área visível dos `ScrollViewer` em volta:
    /// `false` quando está rolado para fora. As ações por id (`click`, `hover`...) rolam até
    /// ele antes, como o Tab faria.
    pub fn in_view(&self, id: &str) -> bool {
        let rect = self.ui.tree().visible_rect(self.node(id));
        rect.width() > 0.0 && rect.height() > 0.0
    }

    /// Rolagem atual do `ScrollViewer` `id`, em pixels (`x` para a direita, `y` para baixo).
    pub fn scroll_offset(&self, id: &str) -> Vec2 {
        self.scroller(id).offset()
    }

    /// Tamanho do conteúdo do `ScrollViewer` `id`: rola enquanto for maior que o
    /// [`rect`](TestApp::rect) dele.
    pub fn scroll_content(&self, id: &str) -> tomni_ui::kurbo::Size {
        self.scroller(id).content()
    }

    fn scroller(&self, id: &str) -> tomni_ui::Scroller {
        self.ui
            .tree()
            .scroller(self.node(id))
            .unwrap_or_else(|| panic!("{id:?} não é um ScrollViewer"))
    }

    /// Centro da parte de `id` que recebe o ponteiro (na `InfoBar`, o botão de fechar), depois
    /// de rolar os `ScrollViewer` em volta até ele aparecer.
    fn target(&mut self, id: &str) -> Point {
        if self.ui.find(id).is_none()
            && let Some((owner, _)) = self.ui.find_link(id)
        {
            // Um link de `RichText`: clica no lugar do trecho, não no centro do texto.
            self.ui.bring_into_view(owner);
            self.step();
            return self.ui.find_link(id).map_or_else(
                || self.ui.hit_rect(owner).center(),
                |(_, rect)| rect.center(),
            );
        }
        let node = self.node(id);
        assert!(
            self.ui.shown(node),
            "{id:?} está oculto (`.visible(false)` nele ou num contêiner em volta) e não recebe o ponteiro"
        );
        self.ui.bring_into_view(node);
        self.step();
        self.ui.hit_rect(node).center()
    }

    /// Texto do campo `id` (o valor de verdade, mesmo no modo senha), o conteúdo do `Text` ou o
    /// valor escolhido no `ComboBox` (vazio se nenhum).
    pub fn text(&self, id: &str) -> String {
        let tree = self.ui.tree();
        untrack(|| tree.widget(self.node(id)).text())
            .unwrap_or_else(|| panic!("{id:?} não tem texto"))
    }

    /// Se o widget `id` está habilitado.
    pub fn enabled(&self, id: &str) -> bool {
        let tree = self.ui.tree();
        untrack(|| tree.widget(self.node(id)).enabled())
    }

    /// Id do widget com foco.
    pub fn focused(&self) -> Option<String> {
        let tree = self.ui.tree();
        tree.focused().map(|id| tree.name(id))
    }

    /// Se o popup `id` está aberto: um `ContentDialog`, `Flyout` ou `MenuFlyout` pelo id dele,
    /// a lista de um `ComboBox` ou das sugestões de um `AutoSuggestBox` pelo id da caixa e a
    /// dica de um widget por `{id}-tooltip`. Num `Expander`, se ele está aberto. Num
    /// `NumberBox`, se os botões de incremento estão à vista sobre ele (no modo compacto, com
    /// o foco; nos outros modos, sempre `false`).
    ///
    /// ```
    /// use tomni_ui::prelude::*;
    /// use tomni_ui_test::TestApp;
    ///
    /// let mut app = TestApp::new(|| {
    ///     let uf = signal(String::new());
    ///     Form::new("Endereço").child(Column::new().child(ComboBox::bind(uf, ["MG", "SP"]).id("uf")))
    /// });
    /// app.click("uf");
    /// assert!(app.is_open("uf"));
    /// app.click("uf-sp");
    /// assert!(!app.is_open("uf"));
    /// assert_eq!(app.text("uf"), "SP");
    /// ```
    ///
    /// # Panics
    ///
    /// Se `id` não for um popup, um `ComboBox`, um `AutoSuggestBox`, um `NumberBox` nem um
    /// `Expander`.
    pub fn is_open(&self, id: &str) -> bool {
        let node = self.node(id);
        let tree = self.ui.tree();
        let popup = if tree.layer(node).is_some() {
            node
        } else if let Look::Expander(visual) = tree.look(node) {
            return visual.open;
        } else if tree.widget(node).kind() == "number_box" {
            // Só o modo compacto tem o popup dos botões de incremento.
            let Some(spin) = tree
                .children(node)
                .iter()
                .copied()
                .find(|&child| tree.layer(child).is_some())
            else {
                return false;
            };
            spin
        } else if matches!(tree.widget(node).kind(), "combo_box" | "auto_suggest_box") {
            tree.children(node)
                .iter()
                .copied()
                .find(|&child| tree.layer(child).is_some())
                .expect("a caixa tem a lista")
        } else {
            panic!("{id:?} não é um popup, um ComboBox, um AutoSuggestBox nem um Expander")
        };
        self.ui.shown(popup)
    }

    /// Escolhe `option` como uma pessoa faria: no `ComboBox` `id`, clica nele e no item; no
    /// `RadioButtons` `id`, clica na opção; no `TabView` `id`, clica na aba com esse título;
    /// no `NavigationView` `id`, clica no item com esse rótulo, abrindo antes o grupo em volta
    /// dele (no painel compacto, o popup) ou, no modo mínimo, o painel.
    ///
    /// ```
    /// use tomni_ui::prelude::*;
    /// use tomni_ui_test::TestApp;
    ///
    /// let mut app = TestApp::new(|| {
    ///     let tema = signal("Claro".to_string());
    ///     Form::new("Aparência").child(RadioButtons::bind(tema, ["Claro", "Escuro"]).id("tema"))
    /// });
    /// app.select("tema", "Escuro");
    /// assert_eq!(app.text("tema"), "Escuro");
    /// assert!(app.checked("tema-escuro"));
    /// ```
    ///
    /// # Panics
    ///
    /// Se `id` não for um desses ou não tiver a opção.
    pub fn select(&mut self, id: &str, option: &str) -> &mut Self {
        let node = self.node(id);
        let kind = self.ui.tree().widget(node).kind();
        assert!(
            matches!(
                kind,
                "combo_box" | "radio_buttons" | "tab_view" | "navigation_view"
            ),
            "{id:?} não é um ComboBox, um RadioButtons, um TabView nem um NavigationView"
        );
        if kind == "navigation_view" {
            // Abre o painel e os grupos em volta do item, um clique por vez.
            for _ in 0..5 {
                let (step, last) = self
                    .ui
                    .navigation_step(node, option)
                    .unwrap_or_else(|labels| {
                        panic!("{id:?} não tem o item {option:?}; itens: {labels:?}")
                    });
                self.ui.bring_into_view(step);
                self.step();
                let center = self.ui.hit_rect(step).center();
                self.click_at(center, 1);
                if last {
                    return self;
                }
            }
            panic!("o item {option:?} de {id:?} não apareceu");
        }
        let item = self.ui.part(node, option).unwrap_or_else(|labels| {
            panic!("{id:?} não tem a opção {option:?}; opções: {labels:?}")
        });
        if kind == "combo_box" {
            self.click(id);
            assert!(self.ui.shown(item), "a lista de {id:?} não abriu");
        }
        self.ui.bring_into_view(item);
        self.step();
        let center = self.ui.hit_rect(item).center();
        self.click_at(center, 1);
        self
    }

    /// Passa `duration` de tempo e roda um frame: a dica de um widget abre com o mouse parado
    /// sobre ele por [`TOOLTIP_DELAY`](tomni_ui::TOOLTIP_DELAY).
    ///
    /// ```
    /// use tomni_ui::prelude::*;
    /// use tomni_ui::TOOLTIP_DELAY;
    /// use tomni_ui_test::TestApp;
    ///
    /// let mut app = TestApp::new(|| {
    ///     Form::new("Editor").child(Button::new("Salvar").id("salvar").tooltip("Salva o rascunho"))
    /// });
    /// app.hover("salvar");
    /// assert!(!app.is_open("salvar-tooltip"));
    /// app.wait(TOOLTIP_DELAY);
    /// assert!(app.is_open("salvar-tooltip"));
    /// ```
    pub fn wait(&mut self, duration: std::time::Duration) -> &mut Self {
        self.ui.advance(duration);
        self.step();
        self
    }

    /// Passa o mouse sobre o centro do widget `id`.
    ///
    /// # Panics
    ///
    /// Se o widget estiver oculto (vale também para `click` e `double_click`).
    pub fn hover(&mut self, id: &str) -> &mut Self {
        let center = self.target(id);
        self.ui.pointer_move(center, PointerKind::Mouse);
        self.step();
        self
    }

    /// Clica com o mouse no centro do widget `id`: move, pressiona e solta, com um frame
    /// depois de cada passo. Na `InfoBar`, clica no botão de fechar.
    pub fn click(&mut self, id: &str) -> &mut Self {
        let center = self.target(id);
        self.click_at(center, 1);
        self
    }

    /// Clica no centro do widget `id` com os modificadores `mods` pressionados
    /// (`Modifiers::COMMAND` soma a uma seleção, `Modifiers::SHIFT` marca a faixa).
    pub fn click_with(&mut self, id: &str, mods: Modifiers) -> &mut Self {
        let center = self.target(id);
        self.ui.set_modifiers(mods);
        self.click_at(center, 1);
        self.ui.set_modifiers(Modifiers::NONE);
        self
    }

    /// Pressiona o mouse no centro do widget `id` e não solta, até [`TestApp::release`]: o botão
    /// de revelar a senha mostra enquanto está pressionado, e os botões de incremento do
    /// `NumberBox` repetem a cada [`SPIN_REPEAT`](tomni_ui::SPIN_REPEAT) (passe o tempo com
    /// [`TestApp::wait`]).
    ///
    /// ```
    /// use tomni_ui::prelude::*;
    /// use tomni_ui::SPIN_REPEAT;
    /// use tomni_ui_test::TestApp;
    ///
    /// let mut app = TestApp::new(|| {
    ///     let qtd = signal(1.0);
    ///     Form::new("Pedido").child(NumberBox::bind(qtd).id("qtd").spin_buttons(SpinButtons::Inline))
    /// });
    /// app.press("qtd-aumentar");
    /// assert_eq!(app.value("qtd"), 2.0);
    /// app.wait(SPIN_REPEAT * 3).release();
    /// assert_eq!(app.value("qtd"), 5.0);
    /// ```
    ///
    /// # Panics
    ///
    /// Se já houver um botão pressionado.
    pub fn press(&mut self, id: &str) -> &mut Self {
        assert!(
            self.pressed.is_none(),
            "solte o anterior com `release` antes"
        );
        let center = self.target(id);
        self.ui.pointer_move(center, PointerKind::Mouse);
        self.step();
        self.ui.pointer_down(center, PointerKind::Mouse, 1);
        self.step();
        self.pressed = Some(center);
        self
    }

    /// Solta o mouse pressionado por [`TestApp::press`], no mesmo ponto.
    ///
    /// # Panics
    ///
    /// Se nada estiver pressionado.
    pub fn release(&mut self) -> &mut Self {
        let point = self
            .pressed
            .take()
            .expect("nada pressionado: use `press` antes");
        self.ui.pointer_up(point, PointerKind::Mouse);
        self.step();
        self
    }

    /// Duplo clique no centro do widget `id`: num campo de texto, seleciona a palavra.
    pub fn double_click(&mut self, id: &str) -> &mut Self {
        let center = self.target(id);
        self.click_at(center, 1);
        self.click_at(center, 2);
        self
    }

    /// Clica com o mouse em `point`, na janela. `clicks` é 1 no clique simples, 2 no
    /// segundo clique de um duplo e 3 no terceiro de um triplo.
    pub fn click_at(&mut self, point: Point, clicks: u8) -> &mut Self {
        self.ui.pointer_move(point, PointerKind::Mouse);
        self.step();
        self.ui.pointer_down(point, PointerKind::Mouse, clicks);
        self.step();
        self.ui.pointer_up(point, PointerKind::Mouse);
        self.step();
        self
    }

    /// Clica com o botão direito no centro do widget `id` (pressiona e solta): abre o
    /// `ContextMenu` e, num `Terminal`, cola (ou, quando o programa pediu o mouse, manda o
    /// clique a ele).
    pub fn right_click(&mut self, id: &str) -> &mut Self {
        let center = self.target(id);
        self.ui.pointer_move(center, PointerKind::Mouse);
        self.step();
        self.ui.secondary_down(center);
        self.step();
        self.ui.secondary_up(center);
        self.step();
        self
    }

    /// [`TestApp::right_click`] com os modificadores `mods` pressionados: num `Terminal` com
    /// `RightClick::CopyPaste`, `Modifiers::CTRL` abre o menu em vez de copiar ou colar.
    pub fn right_click_with(&mut self, id: &str, mods: Modifiers) -> &mut Self {
        self.ui.set_modifiers(mods);
        self.right_click(id);
        self.ui.set_modifiers(Modifiers::NONE);
        self
    }

    /// Abre o menu de contexto do widget `id` (`.context_flyout`, ou o `tab_context_flyout` numa
    /// aba) com o botão direito no centro dele. O menu fica aberto: clique num item pelo id.
    ///
    /// ```
    /// use tomni_ui::prelude::*;
    /// use tomni_ui_test::TestApp;
    ///
    /// let mut app = TestApp::new(|| {
    ///     let copiado = signal(false);
    ///     Form::new("Lista").child(
    ///         Text::new("linha 1").id("linha").context_flyout(
    ///             MenuFlyout::new().id("menu").item("copiar", "Copiar", move || copiado.set(true)),
    ///         ),
    ///     )
    /// });
    /// app.context_menu("linha");
    /// assert!(app.is_open("menu"));
    /// app.click("copiar");
    /// assert!(!app.is_open("menu"));
    /// ```
    pub fn context_menu(&mut self, id: &str) -> &mut Self {
        self.right_click(id)
    }

    /// Clica com o botão do meio no centro do widget `id`: numa aba do `TabView` que fecha,
    /// pede para fechar.
    pub fn middle_click(&mut self, id: &str) -> &mut Self {
        let center = self.target(id);
        self.ui.pointer_move(center, PointerKind::Mouse);
        self.step();
        self.ui.middle_down(center);
        self.step();
        self.ui.middle_up(center);
        self.step();
        self
    }

    /// Fecha a aba de chave `key` do `TabView` `id` como uma pessoa faria: passa o mouse
    /// sobre a aba (o botão de fechar aparece) e clica no X. Quem fecha é o `on_close`.
    ///
    /// ```
    /// use tomni_ui::prelude::*;
    /// use tomni_ui_test::TestApp;
    ///
    /// let mut app = TestApp::new(|| {
    ///     let notas = signal(vec!["Compras".to_string(), "Ideias".to_string()]);
    ///     let aberta = signal("Compras".to_string());
    ///     Form::new("Notas").child(
    ///         TabView::bind(aberta)
    ///             .id("notas")
    ///             .items(notas, |n| n.clone(), |n| n.clone(), |n| Text::new(n))
    ///             .on_close(move |chave| notas.update(|v| v.retain(|n| *n != chave))),
    ///     )
    /// });
    /// app.close_tab("notas", "Compras");
    /// assert_eq!(app.tab_titles("notas"), ["Ideias"]);
    /// ```
    ///
    /// # Panics
    ///
    /// Se `id` não for um `TabView` com `.on_close` ou não tiver a aba.
    pub fn close_tab(&mut self, id: &str, key: &str) -> &mut Self {
        let Look::TabView(visual) = self.look(id) else {
            panic!("{id:?} não é um TabView");
        };
        assert!(
            visual.closable,
            "as abas de {id:?} não fecham: falta o `.on_close`"
        );
        let keys: Vec<String> = visual.tabs.into_iter().map(|(k, _)| k).collect();
        assert!(
            keys.iter().any(|k| k == key),
            "{id:?} não tem a aba {key:?}; abas: {keys:?}"
        );
        let header = format!("{id}-{}", slug(key));
        self.hover(&header);
        self.click(&format!("{header}-fechar"))
    }

    /// As linhas visíveis do `TreeView` `id`, em ordem: chave, rótulo, nível, se a pasta está
    /// aberta e se é a selecionada. Lê o estado da árvore, não os widgets, então inclui as
    /// linhas que a virtualização ainda não montou.
    ///
    /// ```
    /// use tomni_ui::prelude::*;
    /// use tomni_ui_test::TestApp;
    ///
    /// let mut app = TestApp::new(|| {
    ///     let itens = signal(vec![
    ///         TreeItem::folder("a", "Pasta A").child(TreeItem::leaf("b", "Item B")),
    ///     ]);
    ///     Form::new("Árvore").child(TreeView::bind(itens, signal(None)).id("arvore").grow(1))
    /// });
    /// assert_eq!(app.tree_items("arvore").len(), 1);
    /// app.expand("arvore", "a");
    /// let linhas = app.tree_items("arvore");
    /// assert_eq!(linhas[1].key, "b");
    /// assert_eq!(linhas[1].depth, 1);
    /// ```
    ///
    /// # Panics
    ///
    /// Se `id` não for um `TreeView`.
    pub fn tree_items(&self, id: &str) -> Vec<TreeRowInfo> {
        match self.look(id) {
            Look::TreeView(visual) => visual
                .rows
                .rows
                .iter()
                .map(|row| TreeRowInfo {
                    key: row.key.clone(),
                    label: row.label.clone(),
                    depth: row.depth,
                    expanded: row.expanded,
                    selected: visual.selected.as_deref() == Some(row.key.as_str()),
                })
                .collect(),
            look => panic!("{id:?} não é um TreeView: {look:?}"),
        }
    }

    /// O `DataGrid` `id`: colunas (com a largura de hoje), todas as linhas na ordem da tela,
    /// a seleção, a ordenação e o filtro.
    ///
    /// # Panics
    ///
    /// Se `id` não for um `DataGrid`.
    pub fn grid(&self, id: &str) -> DataGridInfo {
        let Look::DataGrid(visual) = self.look(id) else {
            panic!("{id:?} não é um DataGrid");
        };
        let widths = match self.ui.find(&format!("{id}-cabecalho")) {
            Some(header) => match self.ui.tree().look(header) {
                Look::DataGridHeader(h) => h
                    .cells(self.ui.rect(header))
                    .iter()
                    .map(Rect::width)
                    .collect(),
                _ => Vec::new(),
            },
            None => Vec::new(),
        };
        DataGridInfo {
            columns: visual
                .columns
                .iter()
                .enumerate()
                .map(|(i, column)| GridColumnInfo {
                    key: column.key.clone(),
                    header: column.header.clone(),
                    width: widths.get(i).copied().unwrap_or_default(),
                })
                .collect(),
            rows: visual
                .rows
                .rows
                .iter()
                .map(|row| GridRowInfo {
                    key: row.key.clone(),
                    cells: row.cells.iter().map(|(text, _)| text.clone()).collect(),
                    selected: visual.selected.contains(&row.key),
                    tone: row.tone.map(|tone| tone.name().to_owned()),
                })
                .collect(),
            selected: visual.selected.clone(),
            sort: visual.sort.clone(),
            filter: visual.filter.clone(),
        }
    }

    /// Troca a seleção do `DataGrid` `id` pelas linhas `keys` (as que existem), sem clicar: a
    /// linha pode nem estar montada. O foco não muda: o Delete só apaga a seleção inteira se a
    /// linha com foco está nela (clique numa das linhas antes).
    ///
    /// # Panics
    ///
    /// Se `id` não for um `DataGrid`.
    pub fn grid_select(&mut self, id: &str, keys: &[&str]) -> &mut Self {
        let keys: Vec<String> = keys.iter().map(|key| (*key).to_owned()).collect();
        let node = self.node(id);
        let found = self.ui.with_widget_mut(node, |widget| {
            widget
                .as_any_mut()
                .and_then(|any| any.downcast_mut::<tomni_ui::DataGrid>())
                .map(|grid| grid.select_keys(&keys))
                .is_some()
        });
        assert!(found, "{id:?} não é um DataGrid");
        self.step();
        self
    }

    /// O ponto do divisor à direita da coluna `column` (0 é a primeira) do `DataGrid` `id`,
    /// no meio do cabeçalho: arraste dali para redimensionar.
    ///
    /// # Panics
    ///
    /// Se `id` não for um `DataGrid` ou não houver essa coluna.
    pub fn grid_divider(&self, id: &str, column: usize) -> Point {
        let header = self.node(&format!("{id}-cabecalho"));
        let Look::DataGridHeader(visual) = self.ui.tree().look(header) else {
            panic!("{id:?} não é um DataGrid");
        };
        let rect = self.ui.rect(header);
        let cell = visual.cells(rect)[column];
        Point::new(cell.x1, rect.center().y)
    }

    /// Seleciona `keys` no `DataGrid` `id` e as arrasta, pela primeira, até a linha
    /// `target_row` do grid `target_id` (uma pasta, em geral) ou, com `None`, até o fundo dele.
    /// O fundo é um ponto vazio abaixo das linhas: o grid de destino precisa ter espaço sobrando.
    ///
    /// # Panics
    ///
    /// Se `keys` estiver vazio ou algum id não existir.
    pub fn drag_rows(
        &mut self,
        id: &str,
        keys: &[&str],
        target_id: &str,
        target_row: Option<&str>,
    ) -> &mut Self {
        assert!(
            !keys.is_empty(),
            "`drag_rows` precisa de ao menos uma chave"
        );
        self.grid_select(id, keys);
        let to = match target_row {
            Some(row) => Spot::Id(format!("{target_id}-{row}")),
            None => {
                let rect = self.rect(target_id);
                Spot::Point(Point::new(rect.center().x, rect.y1 - 4.0))
            }
        };
        self.drag(Spot::Id(format!("{id}-{}", keys[0])), to)
    }

    /// Solta arquivos do sistema (`paths`) sobre o widget `id` (o centro dele): um `DataGrid`,
    /// ou uma linha dele (`grade-pasta`).
    pub fn drop_files(&mut self, id: &str, paths: &[PathBuf]) -> &mut Self {
        let point = self.target(id);
        self.ui.pointer_move(point, PointerKind::Mouse);
        self.step();
        self.ui.files_dropped(paths.to_vec(), point);
        self.step();
        self
    }

    /// As propriedades da `PropertyGrid` `id`, na ordem da declaração: categoria, nome, valor
    /// (a senha mascarada, a `.check` como `true` ou `false`), se difere do `.default`, se é a
    /// escolhida, se é só de leitura, se está habilitada e se está escondida (`hidden`, por
    /// `.hidden`). Lê o estado da grade, não as linhas montadas, então inclui as das
    /// categorias fechadas e as escondidas.
    ///
    /// ```
    /// use tomni_ui::prelude::*;
    /// use tomni_ui_test::TestApp;
    ///
    /// let app = TestApp::new(|| {
    ///     let porta = signal(2222.0);
    ///     Form::new("Conexão").child(
    ///         PropertyGrid::new()
    ///             .id("props")
    ///             .category("Conexão")
    ///             .number("Porta", porta, 1.0..=65535.0)
    ///             .default(22)
    ///             .grow(1),
    ///     )
    /// });
    /// let porta = &app.properties("props")[0];
    /// assert_eq!(porta.name, "Porta");
    /// assert_eq!(porta.value, "2222");
    /// assert!(porta.changed);
    /// ```
    ///
    /// # Panics
    ///
    /// Se `id` não for uma `PropertyGrid`.
    pub fn properties(&self, id: &str) -> Vec<PropertyInfo> {
        match self.look(id) {
            Look::PropertyGrid(visual) => visual.properties,
            look => panic!("{id:?} não é uma PropertyGrid: {look:?}"),
        }
    }

    /// Abre a pasta `key` do `TreeView` `id`, sem clicar (a linha pode nem estar montada).
    ///
    /// # Panics
    ///
    /// Se `id` não for um `TreeView`.
    pub fn expand(&mut self, id: &str, key: &str) -> &mut Self {
        self.set_open(id, key, true)
    }

    /// Fecha a pasta `key` do `TreeView` `id`.
    ///
    /// # Panics
    ///
    /// Se `id` não for um `TreeView`.
    pub fn collapse(&mut self, id: &str, key: &str) -> &mut Self {
        self.set_open(id, key, false)
    }

    fn set_open(&mut self, id: &str, key: &str, open: bool) -> &mut Self {
        let node = self.node(id);
        let found = self.ui.with_widget_mut(node, |widget| {
            widget
                .as_any_mut()
                .and_then(|any| any.downcast_mut::<tomni_ui::TreeView>())
                .map(|tree| tree.set_open(key, open))
                .is_some()
        });
        assert!(found, "{id:?} não é um TreeView");
        self.step();
        self
    }

    /// Títulos das abas do `TabView` `id`, na ordem.
    pub fn tab_titles(&self, id: &str) -> Vec<String> {
        match self.look(id) {
            Look::TabView(visual) => visual.tabs.into_iter().map(|(_, t)| t).collect(),
            look => panic!("{id:?} não é um TabView: {look:?}"),
        }
    }

    fn spot(&mut self, spot: Spot) -> Point {
        match spot {
            Spot::Id(id) => self.target(&id),
            Spot::Point(point) => point,
        }
    }

    /// Arrasta o mouse pressionado de `from` até `to`: cada um é o id de um widget (o centro
    /// dele) ou um ponto da janela. Serve para o divisor do `Split`, o `Slider`, a seleção de
    /// texto e o arrasto de itens, como as abas de um `TabView` com `.on_reorder`; para soltar
    /// numa parte do widget (a metade da direita de uma aba), use [`TestApp::drag_to`].
    ///
    /// ```
    /// use tomni_ui::prelude::*;
    /// use tomni_ui_test::TestApp;
    ///
    /// let mut app = TestApp::new(|| {
    ///     let logs = signal(vec!["app".to_string(), "erro".to_string(), "acesso".to_string()]);
    ///     let aberto = signal("app".to_string());
    ///     Form::new("Logs").child(
    ///         TabView::bind(aberto)
    ///             .id("logs")
    ///             .items(logs, |l| l.clone(), |l| l.clone(), |l| Text::new(l))
    ///             .on_reorder(move |de, para| {
    ///                 logs.update(|v| {
    ///                     let l = v.remove(de);
    ///                     v.insert(para, l);
    ///                 })
    ///             }),
    ///     )
    /// })
    /// .size(600, 300);
    /// // Soltar sobre a metade da esquerda de uma aba põe a arrastada antes dela.
    /// app.drag("logs-acesso", "logs-app");
    /// assert_eq!(app.tab_titles("logs"), ["acesso", "app", "erro"]);
    /// ```
    pub fn drag(&mut self, from: impl Into<Spot>, to: impl Into<Spot>) -> &mut Self {
        let from = self.spot(from.into());
        let to = self.spot(to.into());
        self.ui.pointer_move(from, PointerKind::Mouse);
        self.ui.pointer_down(from, PointerKind::Mouse, 1);
        self.step();
        self.ui.pointer_move(to, PointerKind::Mouse);
        self.step();
        self.ui.pointer_up(to, PointerKind::Mouse);
        self.step();
        self
    }

    /// Arrasta o widget `id` (do centro dele) até `point`, na janela.
    pub fn drag_to(&mut self, id: &str, point: Point) -> &mut Self {
        self.drag(id, point)
    }

    /// Move o mouse até `point`, na janela. Com o botão pressionado por [`TestApp::press`],
    /// arrasta (e o [`TestApp::release`] solta ali): dá para parar no meio de um arrasto e
    /// conferir a etiqueta ([`TestApp::drag_label`]) ou cancelar com o Esc.
    pub fn move_to(&mut self, point: Point) -> &mut Self {
        self.ui.pointer_move(point, PointerKind::Mouse);
        self.step();
        if let Some(pressed) = &mut self.pressed {
            *pressed = point;
        }
        self
    }

    /// Texto da etiqueta que segue o ponteiro durante um arrasto de item (o título da aba,
    /// por exemplo), ou `None` se nada está sendo arrastado.
    pub fn drag_label(&self) -> Option<String> {
        self.ui.tree().drag_label().map(|(label, _)| label)
    }

    /// Rola o `ScrollViewer` `id` `dy` pixels para baixo (negativo sobe), limitado ao que dá
    /// para rolar, e roda um frame. É o que a roda do mouse faz sobre ele.
    ///
    /// ```
    /// use tomni_ui::prelude::*;
    /// use tomni_ui_test::TestApp;
    ///
    /// let mut app = TestApp::new(|| {
    ///     Form::new("Lista").child(
    ///         ScrollViewer::new().id("lista").child(Column::new().child(For::each(
    ///             (1..=50).collect::<Vec<u32>>(),
    ///             |&i| i,
    ///             |i| Text::new(format!("Item {i}")).id(format!("item-{i}")),
    ///         ))),
    ///     )
    /// });
    /// assert!(!app.in_view("item-50"));
    /// app.scroll("lista", 10_000.0);
    /// assert!(app.in_view("item-50"));
    /// assert_eq!(app.repaints_since_last(), ["lista"]); // os itens só se moveram
    /// ```
    ///
    /// # Panics
    ///
    /// Se `id` não for um `ScrollViewer` (vale também para `scroll_x`, `scroll_offset` e
    /// `scroll_content`).
    pub fn scroll(&mut self, id: &str, dy: f64) -> &mut Self {
        self.scroll_by(id, Vec2::new(0.0, dy))
    }

    /// Rola o `ScrollViewer` `id` `dx` pixels para a direita (negativo volta).
    pub fn scroll_x(&mut self, id: &str, dx: f64) -> &mut Self {
        self.scroll_by(id, Vec2::new(dx, 0.0))
    }

    fn scroll_by(&mut self, id: &str, delta: Vec2) -> &mut Self {
        self.scroller(id);
        let node = self.node(id);
        self.ui.scroll_by(node, delta);
        self.step();
        self
    }

    /// Roda do mouse em `point`, na janela: rola o `ScrollViewer` mais interno sob o ponteiro
    /// que ainda anda na direção de `delta` (em pixels; `y` positivo desce). Sobre um
    /// `NumberBox` com o foco, a roda muda o valor em `small_change` (para cima aumenta) e
    /// não rola a página.
    pub fn wheel(&mut self, point: Point, delta: Vec2) -> &mut Self {
        self.ui.pointer_move(point, PointerKind::Mouse);
        self.ui.wheel(point, delta);
        self.step();
        self
    }

    /// Pressiona e solta `key`.
    pub fn key(&mut self, key: Key) -> &mut Self {
        self.key_with(key, Modifiers::NONE);
        self
    }

    /// Pressiona e solta `key` com os modificadores `mods` (`Modifiers::SHIFT`, por exemplo).
    pub fn key_with(&mut self, key: Key, mods: Modifiers) -> &mut Self {
        self.ui.set_modifiers(mods);
        self.ui.key_down(key);
        self.step();
        self.ui.key_up(key);
        self.ui.set_modifiers(Modifiers::NONE);
        self.step();
        self
    }

    /// Pressiona um atalho: `shortcut('c')` é Ctrl+C (Cmd+C no macOS), e
    /// `shortcut(Shortcut::ctrl_shift('N'))` é o Ctrl+Shift+N. Roda o `.accelerator` de quem
    /// tem o atalho, ou o do item de menu com `.shortcut`.
    pub fn shortcut(&mut self, keys: impl Keys) -> &mut Self {
        let (key, mods) = keys.keys();
        self.key_with(key, mods);
        self
    }

    /// Clica no campo `id`, vai ao fim do texto e digita `text`, um caractere por frame.
    ///
    /// ```
    /// use tomni_ui::prelude::*;
    /// use tomni_ui_test::TestApp;
    ///
    /// let mut app = TestApp::new(|| {
    ///     let nome = signal(String::new());
    ///     Form::new("Cadastro").child(Edit::bind(nome).id("nome"))
    /// });
    /// app.type_into("nome", "Ana").type_into("nome", " Souza");
    /// assert_eq!(app.text("nome"), "Ana Souza");
    /// ```
    ///
    /// Num `Terminal`, só clica e digita (o End iria ao programa), e `\n` é o Enter. Num
    /// `TextEditor`, vai ao fim do texto todo (Ctrl+End) e `\n` é o Enter.
    pub fn type_into(&mut self, id: &str, text: &str) -> &mut Self {
        self.click(id);
        if self.editor_focused() {
            self.key_with(Key::End, Modifiers::COMMAND);
        } else if !self.terminal_focused() {
            self.key(Key::End);
        }
        self.type_text(text);
        self
    }

    /// Digita `text` no widget com foco, um caractere por frame. Num `Terminal`, `\n` é o
    /// Enter e `\t` é o Tab; num `TextEditor`, `\n` é o Enter.
    pub fn type_text(&mut self, text: &str) -> &mut Self {
        let terminal = self.terminal_focused();
        let editor = self.editor_focused();
        let mut buf = [0; 4];
        for c in text.chars() {
            match c {
                '\n' if terminal || editor => {
                    self.key(Key::Enter);
                }
                '\t' if terminal => {
                    self.key(Key::Tab);
                }
                _ => {
                    self.ui.type_text(c.encode_utf8(&mut buf));
                    self.step();
                }
            }
        }
        self
    }

    /// Bytes que chegam ao `Terminal` `id` como se viessem do programa, interpretados na hora;
    /// as respostas do terminal (posição do cursor, tipo de terminal) vão pela conexão.
    ///
    /// ```
    /// use tomni_ui::prelude::*;
    /// use tomni_ui_test::TestApp;
    ///
    /// let mut app = TestApp::new(|| {
    ///     Form::new("Console").child(Terminal::new(Loopback::new()).id("console").grow(1))
    /// });
    /// app.feed("console", "\x1b[1;32mpronto\x1b[0m\r\n$ ");
    /// assert_eq!(app.terminal_text("console"), "pronto\n$");
    /// let p = app.terminal_cell("console", 0, 0);
    /// assert_eq!(p.c, 'p');
    /// assert!(p.flags.contains(tomni_ui::terminal::cell::Flags::BOLD));
    /// ```
    pub fn feed(&mut self, id: &str, bytes: impl AsRef<[u8]>) -> &mut Self {
        let node = self.node(id);
        let found = self.ui.with_widget_mut(node, |widget| {
            widget
                .as_any_mut()
                .and_then(|any| any.downcast_mut::<Terminal>())
                .map(|terminal| terminal.feed(bytes.as_ref()))
                .is_some()
        });
        assert!(found, "{id:?} não é um Terminal");
        self.step();
        self
    }

    /// O estado do `TerminalPanes` `id`: o layout, as abas de cada painel, o painel ativo, o
    /// zoom e o envio para vários.
    ///
    /// # Panics
    ///
    /// Se `id` não for um `TerminalPanes`.
    pub fn panes(&self, id: &str) -> PanesInfo {
        let tree = self.ui.tree();
        let widget = tree.widget(self.node(id));
        widget
            .as_any()
            .and_then(|any| any.downcast_ref::<TerminalPanes>())
            .unwrap_or_else(|| panic!("{id:?} não é um TerminalPanes"))
            .info()
    }

    /// Texto da tela da sessão `key` (sem o histórico), esteja ela à mostra ou não: o mesmo
    /// formato de [`TestApp::terminal_text`].
    ///
    /// # Panics
    ///
    /// Se não houver sessão com essa chave.
    pub fn session_text(&self, key: &str) -> String {
        Self::session(key).text()
    }

    /// Texto do histórico e da tela da sessão `key`: o mesmo formato de
    /// [`TestApp::terminal_history`].
    ///
    /// # Panics
    ///
    /// Se não houver sessão com essa chave.
    pub fn session_history(&self, key: &str) -> String {
        Self::session(key).history()
    }

    fn session(key: &str) -> Session {
        Session::find(key).unwrap_or_else(|| panic!("nenhuma sessão com a chave {key:?}"))
    }

    /// Texto da tela do `Terminal` `id` (sem o histórico): uma linha por linha, sem os espaços
    /// do fim e sem as linhas vazias do fim.
    pub fn terminal_text(&self, id: &str) -> String {
        self.terminal(id, |t| t.emulator().screen_text())
    }

    /// Texto do histórico e da tela do `Terminal` `id`, em linhas lógicas (as quebras
    /// automáticas juntadas).
    pub fn terminal_history(&self, id: &str) -> String {
        self.terminal(id, |t| t.emulator().all_text())
    }

    /// A célula na coluna `col` e na linha `row` da tela do `Terminal` `id`, contadas de 0:
    /// o caractere, as cores e os atributos.
    ///
    /// # Panics
    ///
    /// Se a posição estiver fora da tela.
    pub fn terminal_cell(&self, id: &str, col: usize, row: usize) -> Cell {
        self.terminal(id, |t| {
            t.emulator()
                .cell(row, col)
                .cloned()
                .unwrap_or_else(|| panic!("({col}, {row}) fora da tela de {id:?}"))
        })
    }

    /// A página em que o motor do `WebView` `id` está (o estado da carga fica na árvore:
    /// `state`, `url`, `title`, `error`). Nos testes, o motor é o falso: carrega as páginas
    /// embutidas e devolve o `<title>` delas, e qualquer outra como uma página vazia com o
    /// nome do site no título.
    ///
    /// ```
    /// use tomni_ui::prelude::*;
    /// use tomni_ui_test::TestApp;
    ///
    /// let mut app = TestApp::new(|| {
    ///     Form::new("Ajuda").child(
    ///         WebView::html("<title>Ajuda</title><a href='a.html'>A</a>")
    ///             .asset("a.html", "<title>Página A</title>")
    ///             .id("pagina")
    ///             .grow(1),
    ///     )
    /// });
    /// app.frame(); // começa a carregar
    /// app.frame(); // carregou
    /// assert_eq!(app.tree().find("pagina").unwrap().title.as_deref(), Some("Ajuda"));
    /// assert!(app.webview_follow("pagina", "fluent://app/a.html"));
    /// app.frame();
    /// assert_eq!(app.webview_url("pagina"), "fluent://app/a.html");
    /// assert_eq!(app.tree().find("pagina").unwrap().title.as_deref(), Some("Página A"));
    /// ```
    pub fn webview_url(&self, id: &str) -> String {
        self.webview(id, |fake| fake.url().unwrap_or_default().to_owned())
    }

    /// Um link clicado na página do `WebView` `id`: carrega se a origem é permitida (as
    /// páginas embutidas, a inicial, as de `.allow`), abre no navegador se é `http`, `https`,
    /// `mailto` ou `tel` (veja [`TestApp::webview_opened`]), ou é ignorado. Devolve se carregou;
    /// a página fica pronta no passo seguinte.
    pub fn webview_follow(&mut self, id: &str, url: &str) -> bool {
        self.webview_mut(id, |fake| fake.follow(url))
    }

    /// A página do `WebView` `id` manda `text` ao app (`window.fluent.postMessage(text)`):
    /// chega no `on_message`.
    pub fn webview_message(&mut self, id: &str, text: &str) -> &mut Self {
        self.webview_mut(id, |fake| fake.page_message(text));
        self
    }

    /// A carga em andamento (ou a última) do `WebView` `id` falha com `error`: no passo, a
    /// árvore fica com `state: failed` e o `error`, e o widget mostra o aviso com o
    /// "Tentar de novo" (`<id>-tentar`).
    pub fn webview_fail(&mut self, id: &str, error: &str) -> &mut Self {
        self.webview_mut(id, |fake| fake.fail(error));
        self
    }

    /// A página do `WebView` `id` ganha o foco (um clique nela): o widget passa a ser o
    /// focado.
    pub fn webview_page_focus(&mut self, id: &str) -> &mut Self {
        self.webview_mut(id, Fake::page_focus);
        self
    }

    /// Segura as respostas da página do `WebView` `id`, como numa rede lenta: as cargas ficam
    /// em andamento (`loading`) até chamar de novo com `false`.
    pub fn webview_hold(&mut self, id: &str, hold: bool) -> &mut Self {
        self.webview_mut(id, |fake| fake.hold(hold));
        self
    }

    /// F6 dentro da página do `WebView` `id` (Shift+F6 com `back`): o foco volta ao app, no
    /// próximo widget (ou no anterior).
    pub fn webview_f6(&mut self, id: &str, back: bool) -> &mut Self {
        self.webview_mut(id, |fake| fake.page_f6(back));
        self
    }

    /// As mensagens que o app mandou à página do `WebView` `id`
    /// (`WebViewController::post_message`), na ordem.
    pub fn webview_posted(&self, id: &str) -> Vec<String> {
        self.webview(id, |fake| fake.posted().to_vec())
    }

    /// Os scripts que o app rodou na página do `WebView` `id` (`WebViewController::eval`).
    pub fn webview_scripts(&self, id: &str) -> Vec<String> {
        self.webview(id, |fake| fake.scripts().to_vec())
    }

    /// As URLs que o app mandou o `WebView` `id` carregar, na ordem (a inicial primeiro).
    pub fn webview_navigations(&self, id: &str) -> Vec<String> {
        self.webview(id, |fake| fake.navigations().to_vec())
    }

    /// Os endereços que a página do `WebView` `id` abriu no navegador do sistema.
    pub fn webview_opened(&self, id: &str) -> Vec<String> {
        self.webview(id, |fake| fake.opened().to_vec())
    }

    /// Onde a página do `WebView` `id` (a janela do sistema) estaria, em pixels lógicos, ou
    /// `None` se estaria escondida: carregando a primeira vez, com erro, ou coberta por um
    /// popup (aí aparece a captura no lugar dela).
    pub fn webview_shown(&self, id: &str) -> Option<Rect> {
        self.webview(id, Fake::rect)
    }

    /// Se a página do `WebView` `id` está no tema escuro.
    pub fn webview_dark(&self, id: &str) -> bool {
        self.webview(id, Fake::dark)
    }

    /// Se a página do `WebView` `id` tem o foco do teclado do sistema (o Tab chegou nela ou
    /// houve um clique dentro da página).
    pub fn webview_focused(&self, id: &str) -> bool {
        self.webview(id, Fake::focused)
    }

    /// Composição do IME em andamento: `text` aparece sublinhado no cursor.
    pub fn ime_preedit(&mut self, text: &str) -> &mut Self {
        self.ui.ime(Ime::Preedit {
            text: text.to_owned(),
            cursor: None,
        });
        self.step();
        self
    }

    /// O IME confirma `text`, que entra no lugar da composição.
    pub fn ime_commit(&mut self, text: &str) -> &mut Self {
        self.ui.ime(Ime::Commit(text.to_owned()));
        self.step();
        self
    }

    /// Área de transferência (em memória, só deste app).
    pub fn clipboard(&mut self) -> &mut dyn Clipboard {
        self.ui.clipboard()
    }

    /// Entrada de texto que a plataforma ligaria agora (IME, teclado virtual).
    pub fn text_input(&mut self) -> Option<TextInput> {
        self.ui.text_input()
    }

    /// Tab.
    pub fn tab(&mut self) -> &mut Self {
        self.key(Key::Tab);
        self
    }

    /// Shift+Tab.
    pub fn shift_tab(&mut self) -> &mut Self {
        self.ui.focus_prev();
        self.step();
        self
    }

    /// Widgets repintados desde a última chamada (ou desde a criação), na ordem da árvore.
    pub fn repaints_since_last(&mut self) -> Vec<String> {
        self.ui.repaints_since_last()
    }

    /// Árvore de widgets agora, com id, tipo, retângulo e estado (o `tomni-ui tree`).
    pub fn tree(&self) -> Node {
        self.ui.inspect()
    }

    /// Problemas de acessibilidade e layout da tela agora (o `tomni-ui lint`).
    pub fn lint(&mut self) -> Vec<Issue> {
        self.ui.lint()
    }

    /// Falha se o lint achar erros. Avisos (como [`Rule::MissingId`](tomni_ui::lint::Rule))
    /// não contam.
    ///
    /// ```
    /// use tomni_ui::prelude::*;
    /// use tomni_ui_test::TestApp;
    ///
    /// let mut app = TestApp::new(|| {
    ///     Form::new("Olá").child(Column::new().padding(16).child(Button::new("OK").id("ok").height(44)))
    /// });
    /// app.assert_lint();
    /// ```
    ///
    /// # Panics
    ///
    /// Com a lista de erros, se houver algum.
    #[track_caller]
    pub fn assert_lint(&mut self) {
        let errors: Vec<String> = self
            .lint()
            .iter()
            .filter(|issue| issue.severity == Severity::Error)
            .map(ToString::to_string)
            .collect();
        assert!(
            errors.is_empty(),
            "a tela não passa no lint:\n  {}",
            errors.join("\n  ")
        );
    }

    /// Compara a tela com `snapshots/<name>.png`, na pasta do crate que está sendo testado.
    /// Para aprovar, rode com `TOMNI_UI_UPDATE_SNAPSHOTS=1`.
    ///
    /// # Panics
    ///
    /// Se a imagem diferir da referência, se não houver referência ou fora do `cargo test`
    /// (sem `CARGO_MANIFEST_DIR`) sem [`DIR_ENV`](crate::snapshot::DIR_ENV).
    #[track_caller]
    pub fn assert_snapshot(&self, name: &str) {
        let dir = std::env::var_os("CARGO_MANIFEST_DIR")
            .map(|dir| std::path::Path::new(&dir).join("snapshots"))
            .or_else(|| std::env::var_os(crate::snapshot::DIR_ENV).map(Into::into))
            .expect("CARGO_MANIFEST_DIR não definido: rode com `cargo test`");
        crate::snapshot::assert_snapshot(dir, name, &self.snapshot());
    }

    /// Pixels da tela no backend CPU determinístico.
    pub fn snapshot(&self) -> Pixmap {
        let (width, height) = self.ui.size();
        let background = self.ui.drawer().tokens().solid_background_base;
        crate::render(
            u16::try_from(width).expect("largura cabe em u16"),
            u16::try_from(height).expect("altura cabe em u16"),
            background,
            |canvas| self.ui.render(canvas),
        )
    }
}

/// Lê o que chegou pela conexão de cada `Terminal`, como a plataforma faria ao ser acordada
/// pela thread de leitura: o eco do `Loopback` aparece no mesmo passo da tecla.
fn pump_terminals(ui: &mut Ui) {
    let terminals: Vec<NodeId> = {
        let tree = ui.tree();
        tree.ids()
            .filter(|&id| tree.widget(id).kind() == "terminal")
            .collect()
    };
    for id in terminals {
        ui.with_widget_mut(id, |widget| {
            if let Some(terminal) = widget
                .as_any_mut()
                .and_then(|any| any.downcast_mut::<Terminal>())
            {
                terminal.poll();
            }
        });
    }
    // As sessões fora da tela (outra aba, outro painel, sem widget) também leem.
    Session::poll_all();
}

/// Roda cada `WebView`: os comandos do app e o que o motor falso terminou no passo anterior.
/// Uma carga leva um passo: no seguinte, a página está pronta.
fn pump_webviews(ui: &mut Ui) {
    let webviews: Vec<NodeId> = {
        let tree = ui.tree();
        tree.ids()
            .filter(|&id| tree.widget(id).kind() == "webview")
            .collect()
    };
    for id in webviews {
        ui.with_widget_mut(id, |widget| {
            if let Some(webview) = widget
                .as_any_mut()
                .and_then(|any| any.downcast_mut::<WebView>())
            {
                webview.poll();
            }
        });
    }
}

/// Parte do id de uma aba: a chave em minúsculas, com o que não é letra ou número trocado por
/// `-` (a mesma regra do `TabView`).
fn slug(key: &str) -> String {
    key.chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .flat_map(char::to_lowercase)
        .collect()
}
