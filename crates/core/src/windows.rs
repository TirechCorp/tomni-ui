//! Várias janelas no mesmo processo: abrir, fechar, achar a ativa e perguntar ao app se pode
//! fechar.
//!
//! Todas as janelas rodam na mesma thread e no mesmo runtime reativo, então um signal criado
//! na janela principal e capturado pela closure de outra é o mesmo signal. A tela de cada
//! janela tem o seu `Scope`: quando a janela fecha, ele é descartado e os signals **criados
//! dentro dela** morrem. Estado que mais de uma janela usa nasce na janela principal (ou num
//! `Scope::detached()` do app) e entra nas outras por captura.
//!
//! A plataforma (e o `TestApp`) atendem os pedidos: [`take_open_requests`], [`take_close_requests`],
//! [`take_focus_requests`] e [`take_visibility_requests`]. Quem escreve o app usa só
//! [`open_window`], [`WindowHandle`],
//! [`active_window`] e [`on_close_requested`].

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::fmt;
use std::rc::Rc;

use tomni_ui_reactive::{Get, GetUntracked, Memo, Scope, Set, Signal, memo, signal};

use crate::wake::wake;
use crate::widget::{View, Widget};

/// Uma janela do processo. É barata de copiar e vale enquanto a janela existe: depois de fechada,
/// [`WindowHandle::is_open`] devolve `false` e os pedidos a ela não fazem nada.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowHandle(u64);

impl fmt::Debug for WindowHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Janela({})", self.0)
    }
}

/// Onde a janela está e o tamanho da área de conteúdo, em pixels lógicos.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowGeometry {
    /// A posição do canto de cima e à esquerda na tela. `None` onde o sistema não informa
    /// (o Wayland).
    pub position: Option<(i32, i32)>,
    /// A largura e a altura do conteúdo.
    pub size: (u32, u32),
}

/// O que se guarda de cada janela aberta.
struct Info {
    handle: WindowHandle,
    title: String,
    geometry: Option<WindowGeometry>,
}

struct State {
    next: u64,
    /// As janelas abertas (inclusive as que ainda esperam a plataforma criar), na ordem em que
    /// abriram.
    open: Vec<Info>,
    opens: VecDeque<OpenWindow>,
    closes: Vec<WindowHandle>,
    focuses: Vec<WindowHandle>,
    /// Os pedidos de atenção (`request_attention`) que a plataforma ainda não aplicou.
    attentions: Vec<WindowHandle>,
    /// Os pedidos de esconder (`false`) e mostrar (`true`) que a plataforma ainda não aplicou.
    visibility: Vec<(WindowHandle, bool)>,
    /// As janelas escondidas, como o app pediu (inclusive a principal de `start_hidden`).
    hidden: Vec<WindowHandle>,
    hooks: Vec<(WindowHandle, Rc<dyn Fn() -> bool>)>,
    /// Se a janela principal está perguntando se pode fechar (e levaria todas as janelas).
    closing_all: bool,
    /// Dono do signal da janela ativa: vive até o `reset`, não com a tela de uma janela.
    scope: Option<Scope>,
    active: Option<Signal<WindowHandle>>,
    /// O tema que o app fixou (`Some(true)` é o escuro) e que as janelas novas herdam.
    theme_override: Option<bool>,
    /// O pedido de troca de tema que a plataforma ainda não aplicou.
    theme_request: Option<Option<bool>>,
}

impl State {
    fn new() -> Self {
        Self {
            next: 1,
            open: vec![Info {
                handle: WindowHandle::MAIN,
                title: String::new(),
                geometry: None,
            }],
            opens: VecDeque::new(),
            closes: Vec::new(),
            focuses: Vec::new(),
            attentions: Vec::new(),
            visibility: Vec::new(),
            hidden: Vec::new(),
            hooks: Vec::new(),
            closing_all: false,
            scope: None,
            active: None,
            theme_override: None,
            theme_request: None,
        }
    }
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::new());
    static CURRENT: Cell<WindowHandle> = const { Cell::new(WindowHandle::MAIN) };
}

impl WindowHandle {
    const MAIN: Self = Self(0);

    /// A janela principal, a do `App::new`.
    pub fn main() -> Self {
        Self::MAIN
    }

    /// A janela cuja tela está sendo montada ou tratando o evento agora.
    ///
    /// Dentro de um efeito o valor é o da janela cujo frame rodou o efeito, o que não é
    /// confiável: guarde o handle ao montar a tela (`let eu = WindowHandle::current();`).
    pub fn current() -> Self {
        CURRENT.get()
    }

    /// As janelas abertas, na ordem em que abriram.
    pub fn all() -> Vec<Self> {
        STATE.with_borrow(|s| s.open.iter().map(|i| i.handle).collect())
    }

    /// Se a janela existe (ou foi pedida e a plataforma ainda não a criou).
    pub fn is_open(self) -> bool {
        STATE.with_borrow(|s| s.open.iter().any(|i| i.handle == self))
    }

    /// Pede para fechar a janela. Passa pelo [`on_close_requested`] dela; fechar a principal
    /// pergunta a todas as janelas e encerra o app.
    pub fn close(self) {
        if self.is_open() {
            STATE.with_borrow_mut(|s| s.closes.push(self));
            wake();
        }
    }

    /// Pede ao sistema para trazer a janela à frente e dar o foco a ela.
    pub fn focus(self) {
        if self.is_open() {
            STATE.with_borrow_mut(|s| s.focuses.push(self));
            wake();
        }
    }

    /// Pede ao sistema para chamar a atenção de quem usa para a janela, sem tirar o foco de quem
    /// está em outra: o botão dela na barra de tarefas pisca ou o dock avisa. Numa janela que já
    /// tem o foco não faz nada, e o sistema apaga o aviso quando a janela ganha o foco.
    pub fn request_attention(self) {
        if self.is_open() {
            STATE.with_borrow_mut(|s| {
                if !s.attentions.contains(&self) {
                    s.attentions.push(self);
                }
            });
            wake();
        }
    }

    /// Esconde a janela: ela some da tela e da barra de tarefas, mas continua existindo, com a
    /// árvore, os signals, os efeitos e os alarmes rodando. Esconder a janela principal **não**
    /// encerra o app. Uma janela escondida não recebe eventos de entrada; [`show`](Self::show)
    /// a traz de volta como estava.
    ///
    /// Onde o sistema não tem como esconder uma janela (o Wayland), a plataforma a minimiza.
    pub fn hide(self) {
        if self.is_open() {
            STATE.with_borrow_mut(|s| {
                if !s.hidden.contains(&self) {
                    s.hidden.push(self);
                }
                s.visibility.push((self, false));
            });
            wake();
        }
    }

    /// Mostra a janela escondida por [`hide`](Self::hide) (ou por `App::start_hidden`), traz
    /// à frente e dá o foco a ela. Numa janela que já está à mostra, só a foca.
    pub fn show(self) {
        if self.is_open() {
            STATE.with_borrow_mut(|s| {
                s.hidden.retain(|h| *h != self);
                s.visibility.push((self, true));
            });
            wake();
        }
    }

    /// Se a janela está escondida: [`hide`](Self::hide) foi chamado e [`show`](Self::show) não.
    /// Não é reativo. `false` para uma janela que não existe.
    pub fn is_hidden(self) -> bool {
        STATE.with_borrow(|s| s.hidden.contains(&self))
    }

    /// O título da janela (o do `Form` da tela dela ou o de [`NewWindow::title`]).
    pub fn title(self) -> String {
        STATE.with_borrow(|s| {
            s.open
                .iter()
                .find(|i| i.handle == self)
                .map(|i| i.title.clone())
                .unwrap_or_default()
        })
    }
}

impl WindowHandle {
    /// Onde a janela está e o tamanho dela agora, em pixels lógicos. `None` enquanto o sistema
    /// não criou a janela. Não é reativo: serve para guardar a disposição das janelas (o app
    /// lê na hora de gravar), não para desenhar.
    pub fn geometry(self) -> Option<WindowGeometry> {
        STATE.with_borrow(|s| {
            s.open
                .iter()
                .find(|i| i.handle == self)
                .and_then(|i| i.geometry)
        })
    }
}

/// Roda `f` com `handle` como a janela [`WindowHandle::current`] e restaura a anterior depois.
/// A plataforma faz isto em volta de cada chamada ao `Ui` de uma janela.
pub fn enter<R>(handle: WindowHandle, f: impl FnOnce() -> R) -> R {
    struct Restore(WindowHandle);
    impl Drop for Restore {
        fn drop(&mut self) {
            CURRENT.set(self.0);
        }
    }
    let _restore = Restore(CURRENT.replace(handle));
    f()
}

/// Fixa a janela [`WindowHandle::current`] até a próxima chamada. O `TestApp` usa quando muda de
/// janela.
pub fn set_current(handle: WindowHandle) {
    CURRENT.set(handle);
}

/// Pedido de uma janela nova, que a plataforma atende: monta a tela com [`OpenWindow::view`].
pub struct OpenWindow {
    /// O handle, que já vale desde o pedido.
    pub handle: WindowHandle,
    /// Título pedido; vazio usa o do `Form` da tela.
    pub title: String,
    /// Tamanho pedido, em pixels lógicos; `None` usa o do app.
    pub size: Option<(u32, u32)>,
    /// Posição pedida na tela, em pixels lógicos. O Wayland não deixa o app escolher.
    pub position: Option<(i32, i32)>,
    /// Monta a tela. Roda uma vez, com a janela como [`WindowHandle::current`].
    pub view: Box<dyn FnOnce() -> Box<dyn Widget>>,
}

impl fmt::Debug for OpenWindow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenWindow")
            .field("handle", &self.handle)
            .field("title", &self.title)
            .field("size", &self.size)
            .field("position", &self.position)
            .finish_non_exhaustive()
    }
}

/// Uma janela a abrir, montada por [`open_window`].
#[must_use = "a janela só abre com `.open()`"]
pub struct NewWindow {
    title: String,
    size: Option<(u32, u32)>,
    position: Option<(i32, i32)>,
    view: Box<dyn FnOnce() -> Box<dyn Widget>>,
}

/// Abre outra janela com a tela de `view`. Chame [`NewWindow::open`] para valer.
///
/// A janela abre no próximo ciclo do loop de eventos, não dentro da chamada; o handle já vale
/// na hora. A closure roda com a nova janela como [`WindowHandle::current`].
///
/// ```
/// use tomni_ui_core::{Button, Column, Text, WindowHandle, open_window};
///
/// let botao = Button::new("Nova janela").on_click(|| {
///     open_window(|| Column::new().child(Text::new("Outra janela")))
///         .title("Outra")
///         .size(320, 200)
///         .open();
/// });
/// # let _ = botao;
/// # let _ = WindowHandle::main();
/// ```
pub fn open_window<V: View>(view: impl FnOnce() -> V + 'static) -> NewWindow {
    NewWindow {
        title: String::new(),
        size: None,
        position: None,
        view: Box::new(move || Box::new(view())),
    }
}

impl NewWindow {
    /// Título inicial, até a tela dar o dela (o `Form`).
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// Tamanho da janela, em pixels lógicos. Sem isso vale o de `App::size`.
    pub fn size(mut self, width: u32, height: u32) -> Self {
        self.size = Some((width, height));
        self
    }

    /// Posição da janela na tela, em pixels lógicos. O Wayland não deixa o app escolher e
    /// ignora.
    pub fn position(mut self, x: i32, y: i32) -> Self {
        self.position = Some((x, y));
        self
    }

    /// Pede a janela. `None` onde só cabe uma (o Android).
    pub fn open(self) -> Option<WindowHandle> {
        if cfg!(target_os = "android") {
            return None;
        }
        let handle = STATE.with_borrow_mut(|s| {
            let handle = WindowHandle(s.next);
            s.next += 1;
            s.open.push(Info {
                handle,
                title: self.title.clone(),
                geometry: None,
            });
            s.opens.push_back(OpenWindow {
                handle,
                title: self.title,
                size: self.size,
                position: self.position,
                view: self.view,
            });
            handle
        });
        wake();
        Some(handle)
    }
}

fn active_signal() -> Signal<WindowHandle> {
    STATE.with_borrow_mut(|s| match s.active {
        Some(active) => active,
        None => {
            let scope = Scope::detached();
            let active = scope.run(|| signal(WindowHandle::MAIN));
            s.scope = Some(scope);
            s.active = Some(active);
            active
        }
    })
}

/// A janela com o foco do sistema, reativa: muda quando a pessoa clica em outra. Começa na
/// principal.
pub fn active_window() -> Memo<WindowHandle> {
    let active = active_signal();
    memo(move || active.get())
}

/// Registra o que a janela [`WindowHandle::current`] responde ao pedido de fechar (o botão da
/// `TitleBar`, Alt+F4, o sistema, [`WindowHandle::close`]): `false` cancela. Vários podem ser
/// registrados; todos precisam dizer `true`. Fechar a janela principal pergunta às de todas as
/// janelas. Serve também de gancho de saída do app.
pub fn on_close_requested(f: impl Fn() -> bool + 'static) {
    let handle = WindowHandle::current();
    STATE.with_borrow_mut(|s| s.hooks.push((handle, Rc::new(f))));
}

// O que a plataforma e o `TestApp` usam.

/// As janelas que o app pediu desde a última chamada, na ordem.
pub fn take_open_requests() -> Vec<OpenWindow> {
    STATE.with_borrow_mut(|s| s.opens.drain(..).collect())
}

/// Pede a troca do tema de todas as janelas: `Some(true)` fixa o escuro, `Some(false)` o claro
/// e `None` volta a seguir o sistema. A plataforma (e o `TestApp`) aplica; quem escreve o app
/// usa `tomni_ui::set_theme`.
pub fn request_theme_override(dark: Option<bool>) {
    STATE.with_borrow_mut(|s| {
        s.theme_override = dark;
        s.theme_request = Some(dark);
    });
    wake();
}

/// O tema que o app fixou, `None` enquanto segue o sistema. As janelas abertas depois nascem
/// com ele.
pub fn theme_override() -> Option<bool> {
    STATE.with_borrow(|s| s.theme_override)
}

/// O pedido de [`request_theme_override`] que ainda não foi aplicado, uma única vez.
pub fn take_theme_request() -> Option<Option<bool>> {
    STATE.with_borrow_mut(|s| s.theme_request.take())
}

/// As janelas que o app pediu para fechar ([`WindowHandle::close`]) desde a última chamada.
pub fn take_close_requests() -> Vec<WindowHandle> {
    STATE.with_borrow_mut(|s| std::mem::take(&mut s.closes))
}

/// As janelas que o app pediu para focar ([`WindowHandle::focus`]) desde a última chamada.
pub fn take_focus_requests() -> Vec<WindowHandle> {
    STATE.with_borrow_mut(|s| std::mem::take(&mut s.focuses))
}

/// As janelas que o app pediu para chamar a atenção ([`WindowHandle::request_attention`]) desde a
/// última chamada.
pub fn take_attention_requests() -> Vec<WindowHandle> {
    STATE.with_borrow_mut(|s| std::mem::take(&mut s.attentions))
}

/// Os pedidos de [`WindowHandle::hide`] (`false`) e [`WindowHandle::show`] (`true`) desde a última
/// chamada, na ordem.
pub fn take_visibility_requests() -> Vec<(WindowHandle, bool)> {
    STATE.with_borrow_mut(|s| std::mem::take(&mut s.visibility))
}

/// Pergunta ao app se `handle` pode fechar: roda os [`on_close_requested`] dela ou, sendo a
/// principal, os de todas as janelas. `false` se algum cancelou. Não fecha nada.
pub fn confirm_close(handle: WindowHandle) -> bool {
    let hooks: Vec<(WindowHandle, Rc<dyn Fn() -> bool>)> = STATE.with_borrow(|s| {
        s.hooks
            .iter()
            .filter(|(h, _)| handle == WindowHandle::MAIN || *h == handle)
            .cloned()
            .collect()
    });
    // Todos são consultados, mesmo depois de um `false`: cada um pode abrir o seu aviso.
    let all = handle == WindowHandle::MAIN;
    STATE.with_borrow_mut(|s| s.closing_all = all);
    let ok = hooks
        .into_iter()
        .fold(true, |ok, (h, hook)| enter(h, || hook()) && ok);
    STATE.with_borrow_mut(|s| s.closing_all = false);
    ok
}

/// Se o app está perguntando se pode fechar **todas** as janelas (o pedido de fechar a principal).
/// Só vale dentro de um [`on_close_requested`]: a janela que fecha sozinha e a que fecha junto com
/// as outras se distinguem por aqui (o `DockHost` guarda o painel flutuante como flutuante só no
/// segundo caso). Se outro gancho cancelar, nada fecha.
pub fn closing_all_windows() -> bool {
    STATE.with_borrow(|s| s.closing_all)
}

/// Registra que `handle` fechou e devolve as janelas que fecharam com ela: só ela, ou todas, se
/// for a principal. O resto vazio quer dizer que o app acabou.
pub fn finish_close(handle: WindowHandle) -> Vec<WindowHandle> {
    let (closed, reactivate) = STATE.with_borrow_mut(|s| {
        let closed: Vec<WindowHandle> = if handle == WindowHandle::MAIN {
            s.open.iter().map(|i| i.handle).collect()
        } else {
            s.open
                .iter()
                .map(|i| i.handle)
                .filter(|h| *h == handle)
                .collect()
        };
        s.open.retain(|i| !closed.contains(&i.handle));
        s.hooks.retain(|(h, _)| !closed.contains(h));
        s.closes.retain(|h| !closed.contains(h));
        s.focuses.retain(|h| !closed.contains(h));
        s.visibility.retain(|(h, _)| !closed.contains(h));
        s.hidden.retain(|h| !closed.contains(h));
        s.opens.retain(|o| !closed.contains(&o.handle));
        (closed, s.open.first().map(|i| i.handle))
    });
    if let (Some(active), Some(next)) = (STATE.with_borrow(|s| s.active), reactivate)
        && closed.contains(&active.get_untracked())
    {
        active.set(next);
    }
    closed
}

/// Guarda o título que a janela mostra agora, para [`WindowHandle::title`].
pub fn set_title(handle: WindowHandle, title: &str) {
    STATE.with_borrow_mut(|s| {
        if let Some(info) = s.open.iter_mut().find(|i| i.handle == handle)
            && info.title != title
        {
            info.title = title.to_owned();
        }
    });
}

/// Guarda onde a janela `handle` está e o tamanho dela, para [`WindowHandle::geometry`]. A
/// plataforma chama ao criar, mover e redimensionar a janela.
pub fn set_geometry(handle: WindowHandle, geometry: WindowGeometry) {
    STATE.with_borrow_mut(|s| {
        if let Some(info) = s.open.iter_mut().find(|i| i.handle == handle) {
            info.geometry = Some(geometry);
        }
    });
}

/// A janela `handle` ganhou o foco do sistema: muda o [`active_window`].
pub fn set_active(handle: WindowHandle) {
    if handle.is_open() {
        let active = active_signal();
        if active.get_untracked() != handle {
            active.set(handle);
        }
    }
}

/// Volta ao começo: só a janela principal, sem pedidos nem ganchos. A plataforma chama ao
/// terminar e o `TestApp`, ao nascer.
pub fn reset() {
    let scope = STATE.with_borrow_mut(|s| {
        let scope = s.scope.take();
        *s = State::new();
        scope
    });
    if let Some(scope) = scope {
        scope.clear();
    }
    CURRENT.set(WindowHandle::MAIN);
}
