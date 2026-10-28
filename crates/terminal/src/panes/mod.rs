//! Terminais em painéis (M23): o layout puro dos painéis e das abas e o widget
//! [`TerminalPanes`] que o mostra.

mod badge;
mod frame;
mod layout;

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt;
use std::rc::{Rc, Weak};
use std::time::Duration;

use tomni_ui_core::{
    Accelerator, Align, Button, Column, Common, Density, IntoProp, Justify, Key, Look, MenuFlyout,
    Modifiers, ProgressStatus, Prop, Shortcut, Slot, Split, Symbol, TabProgress, TabView, Text,
    ThemeColor, Token, Widget, WindowHandle,
};
use tomni_ui_reactive::prelude::*;

use crate::emulator::TermProgress;
use crate::scheme::Scheme;
use crate::session::Session;
use crate::widget::{READ_BUDGET, Terminal};
use badge::PaneBadge;
use frame::PaneFrame;
use layout::Shape;

pub use frame::{PANE_BORDER, PANE_DASH, PaneBorder, PaneVisual};
pub use layout::{Pane, PaneDirection, PaneLayout};

/// Diferença abaixo da qual duas frações são a mesma (corta o laço entre o signal do divisor e o
/// layout).
const RATIO_EPSILON: f64 = 1e-9;

/// Para quais sessões vai o que se digita num terminal do [`TerminalPanes`].
#[derive(Clone, PartialEq, Debug, Default)]
pub enum Broadcast {
    /// Cada terminal recebe só o que é dele.
    #[default]
    None,
    /// As sessões à mostra: a aba escolhida de cada painel (com zoom, só a do painel ampliado).
    Visible,
    /// Todas as sessões do layout.
    All,
    /// As sessões cuja chave ([`Session::key`]) está na lista.
    Group(Vec<String>),
}

impl Broadcast {
    /// O nome do envio nas ferramentas: `none`, `visible`, `all` ou `group:a,b`.
    pub fn name(&self) -> String {
        match self {
            Self::None => "none".to_owned(),
            Self::Visible => "visible".to_owned(),
            Self::All => "all".to_owned(),
            Self::Group(keys) => format!("group:{}", keys.join(",")),
        }
    }

    /// As sessões do `layout` que recebem o envio.
    pub(crate) fn targets(&self, layout: &PaneLayout) -> Vec<Session> {
        match self {
            Self::None => Vec::new(),
            Self::Visible => layout.visible(),
            Self::All => layout.sessions(),
            Self::Group(keys) => layout
                .sessions()
                .into_iter()
                .filter(|session| keys.contains(&session.key_str()))
                .collect(),
        }
    }
}

/// Um painel como as ferramentas (`inspect`, `TestApp::panes`) o descrevem.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct PaneInfo {
    /// A chave e o título de cada aba, da esquerda para a direita.
    pub tabs: Vec<(String, String)>,
    /// A chave da aba à mostra (`None` num painel sem abas).
    pub selected: Option<String>,
}

/// O estado de um [`TerminalPanes`] como as ferramentas o descrevem: o que o `inspect` mostra
/// e o `TestApp::panes` devolve.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct PanesInfo {
    /// A forma do layout, como o `Display` do [`PaneLayout`] a escreve (`col[row[0,1],2]`).
    pub layout: String,
    /// Os painéis, na ordem das folhas.
    pub panes: Vec<PaneInfo>,
    /// O índice do painel ativo.
    pub active: usize,
    /// O índice do painel ampliado, se há.
    pub zoomed: Option<usize>,
    /// O envio para vários, como [`Broadcast::name`].
    pub broadcast: String,
    /// Os crachás com o número dos painéis estão ligados ([`TerminalPanes::identify`]).
    pub identifying: bool,
    /// O índice do painel em destaque ([`TerminalPanes::highlight_pane`]), se há e se existe.
    pub highlighted: Option<usize>,
}

/// O que as sessões de um [`TerminalPanes`] estão fazendo, somado ([`TerminalPanes::activity`]):
/// o app usa para o progresso e o contador do dock ou da barra de tarefas.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PanesActivity {
    /// A média dos valores (de 0 a 1) das sessões que têm um, inclusive as pausadas e com erro.
    /// `None` se nenhuma tem valor.
    pub progress: Option<f64>,
    /// Se alguma sessão tem progresso, com ou sem valor.
    pub busy: bool,
    /// Quantas abas estão com o ponto de atenção aceso.
    pub attention: usize,
}

/// Os atalhos de teclado do [`TerminalPanes`], que valem com o foco num dos terminais dele.
/// Cada campo é uma lista: o mesmo comando pode ter várias teclas. Uma lista vazia desliga o
/// comando.
///
/// O zoom não tem atalho padrão: o app põe o
/// que quiser em `zoom`. **Adaptação:** dividir registra as duas teclas lógicas de cada
/// combinação (Alt+Shift+- e Alt+Shift+`_`; Alt+Shift++ e Alt+Shift+`=`), porque a plataforma
/// entrega a tecla que o teclado produz com o Shift.
///
/// ```
/// use tomni_ui_core::Shortcut;
/// use tomni_ui_terminal::PaneShortcuts;
///
/// let mut atalhos = PaneShortcuts::default();
/// atalhos.zoom = vec![Shortcut::ctrl_shift('z')];
/// let nenhum = PaneShortcuts::none();
/// assert!(nenhum.split_right.is_empty());
/// ```
#[derive(Clone, Debug)]
pub struct PaneShortcuts {
    /// Divide o painel ativo, com o painel novo embaixo. Padrão: Alt+Shift+- e Alt+Shift+`_`.
    pub split_down: Vec<Shortcut>,
    /// Divide o painel ativo, com o painel novo à direita. Padrão: Alt+Shift++ e Alt+Shift+`=`.
    pub split_right: Vec<Shortcut>,
    /// Duplica a sessão ativa num painel novo, pelo lado maior do painel. Padrão: Alt+Shift+D.
    pub duplicate: Vec<Shortcut>,
    /// Move o foco para o painel vizinho, na ordem de [`PaneDirection`] (esquerda, direita,
    /// cima, baixo). Padrão: Alt e as setas.
    pub focus: [Vec<Shortcut>; 4],
    /// Move em 5% a divisão mais próxima, na ordem de [`PaneDirection`]. Padrão: Alt+Shift e
    /// as setas.
    pub resize: [Vec<Shortcut>; 4],
    /// Fecha o painel ativo e as sessões dele. Padrão: Ctrl+Shift+W.
    pub close_pane: Vec<Shortcut>,
    /// Alterna o zoom do painel ativo. Sem padrão.
    pub zoom: Vec<Shortcut>,
}

impl PaneShortcuts {
    /// Sem nenhum atalho: o app liga os comandos que quiser, ou muda os painéis só pelo
    /// layout.
    pub fn none() -> Self {
        Self {
            split_down: Vec::new(),
            split_right: Vec::new(),
            duplicate: Vec::new(),
            focus: Default::default(),
            resize: Default::default(),
            close_pane: Vec::new(),
            zoom: Vec::new(),
        }
    }
}

impl Default for PaneShortcuts {
    fn default() -> Self {
        let alt_shift = Modifiers::ALT | Modifiers::SHIFT;
        let arrows = [Key::Left, Key::Right, Key::Up, Key::Down];
        Self {
            split_down: vec![Shortcut::new('-', alt_shift), Shortcut::new('_', alt_shift)],
            split_right: vec![Shortcut::new('+', alt_shift), Shortcut::new('=', alt_shift)],
            duplicate: vec![Shortcut::new('d', alt_shift)],
            focus: arrows.map(|key| vec![Shortcut::alt(key)]),
            resize: arrows.map(|key| vec![Shortcut::new(key, alt_shift)]),
            close_pane: vec![Shortcut::ctrl_shift('w')],
            zoom: Vec::new(),
        }
    }
}

type NewSession = Rc<dyn Fn() -> Session>;
type OnDuplicate = Rc<dyn Fn(Session, DuplicatePlace)>;
type OnClosed = Rc<dyn Fn(Session)>;
type Configure = Rc<dyn Fn(Terminal) -> Terminal>;
type TearOff = Rc<dyn Fn(Session, Option<(i32, i32)>)>;
type MenuExtra = Rc<dyn Fn(Session, MenuFlyout) -> MenuFlyout>;
type TabTooltip = Rc<dyn Fn(Session) -> Option<String>>;
type TabMark = Rc<dyn Fn(Session) -> Option<Symbol>>;
type TabMarkTint = Rc<dyn Fn(Session) -> Option<ThemeColor>>;

thread_local! {
    /// Os [`TerminalPanes`] vivos da thread (de todas as janelas), para uma aba trocar de
    /// layout sem o app ligar um ao outro.
    static LIVE: RefCell<Vec<Weak<Shared>>> = const { RefCell::new(Vec::new()) };
    /// Contador dos `token`s dos widgets.
    static NEXT_TOKEN: Cell<u64> = const { Cell::new(0) };
}

/// Os `TerminalPanes` vivos, sem os já descartados.
fn live() -> Vec<Rc<Shared>> {
    LIVE.with(|live| {
        let mut live = live.borrow_mut();
        live.retain(|shared| shared.strong_count() > 0);
        live.iter().filter_map(Weak::upgrade).collect()
    })
}

/// O que as partes montadas do [`TerminalPanes`] compartilham.
/// O lugar de uma duplicação pedida pelo atalho, entregue ao handler de
/// [`TerminalPanes::on_duplicate`]. Guarda o painel e o lado da divisão do momento do pedido;
/// pode ser usado depois, de volta na thread da interface.
pub struct DuplicatePlace {
    shared: Rc<Shared>,
    direction: PaneDirection,
}

impl DuplicatePlace {
    /// Divide o painel que era o ativo no pedido, pelo lado maior dele, e põe `session` no
    /// painel novo, que fica ativo e recebe o foco.
    pub fn place(self, session: Session) {
        self.shared.split_with(self.direction, Some(session));
    }
}

impl std::fmt::Debug for DuplicatePlace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DuplicatePlace")
            .field("direction", &self.direction)
            .finish_non_exhaustive()
    }
}

struct Shared {
    /// Distingue este widget dos outros vivos na thread.
    token: u64,
    id: RefCell<Option<String>>,
    /// O grupo de arrasto das abas; sem ele, `panes:{id}`.
    group: RefCell<Option<String>>,
    on_tear_off: RefCell<Option<TearOff>>,
    on_tab_click: RefCell<Option<OnClosed>>,
    menu_extra: RefCell<Option<MenuExtra>>,
    tab_tooltip: RefCell<Option<TabTooltip>>,
    tab_mark: RefCell<Option<TabMark>>,
    tab_mark_tint: RefCell<Option<TabMarkTint>>,
    layout: Signal<PaneLayout>,
    broadcast: Cell<Signal<Broadcast>>,
    /// Os crachás com o número dos painéis aparecem.
    identify: RefCell<Prop<bool>>,
    /// O painel em destaque.
    highlight: RefCell<Prop<Option<usize>>>,
    /// O esquema que pinta a aba escolhida de cada painel (`.tab_scheme`).
    tab_scheme: RefCell<Prop<Option<Scheme>>>,
    /// A barra de destaque na aba escolhida do painel ativo (`.active_tab_indicator`).
    active_indicator: RefCell<Prop<bool>>,
    new_session: RefCell<Option<NewSession>>,
    on_duplicate: RefCell<Option<OnDuplicate>>,
    on_closed: RefCell<Option<OnClosed>>,
    configure: RefCell<Option<Configure>>,
    /// O foco está num terminal do widget.
    inside: Cell<bool>,
    /// O id do terminal que deve receber o foco, pedido por um atalho ([`Widget::take_focus_request`]).
    request: RefCell<Option<String>>,
}

impl Drop for Shared {
    fn drop(&mut self) {
        // Pode rodar no fim da thread, quando o registro já não existe.
        let _ = LIVE.try_with(|live| {
            if let Ok(mut live) = live.try_borrow_mut() {
                live.retain(|shared| shared.strong_count() > 0);
            }
        });
    }
}

impl Shared {
    /// O grupo de arrasto das abas deste widget.
    fn group(&self) -> String {
        self.group
            .borrow()
            .clone()
            .unwrap_or_else(|| format!("panes:{}", self.id.borrow().as_deref().unwrap_or_default()))
    }

    /// Tira `session` do layout dos outros `TerminalPanes` vivos do mesmo grupo (outras
    /// janelas), sem descartá-la: a sessão segue viva, com o histórico, e entra aqui.
    fn take_from_others(&self, session: Session) {
        let group = self.group();
        for other in live() {
            if other.token != self.token && other.group() == group {
                leave(&other.layout, session);
            }
        }
    }

    /// Recebe `session` no painel `pane`, na posição `index`: se ela vinha do layout de outro
    /// widget (outra janela), sai de lá.
    fn receive(&self, session: Session, pane: usize, index: usize) {
        self.take_from_others(session);
        self.layout.update(|layout| {
            layout.move_tab_to(session, pane, index);
            layout.set_active(pane);
        });
    }

    /// O id de uma parte: `{id}-{suffix}`, ou nenhum se o widget não tem id.
    fn part(&self, suffix: impl fmt::Display) -> Option<String> {
        self.id.borrow().as_ref().map(|id| format!("{id}-{suffix}"))
    }

    fn mode(&self) -> Broadcast {
        self.broadcast.get().get()
    }

    /// Para quem vai o que se digita em `origin`: as sessões do envio ligado, sem as
    /// encerradas, se `origin` é uma delas; senão, só ela.
    fn recipients(&self, origin: Session) -> Vec<Session> {
        let mode = untrack(|| self.mode());
        let targets = self.layout.with_untracked(|layout| mode.targets(layout));
        if !targets.contains(&origin) {
            return vec![origin];
        }
        targets
            .into_iter()
            .filter(|session| session.core().is_some_and(|core| !core.borrow().closed))
            .collect()
    }

    /// Fecha a sessão `session` pela interface: tira do layout e avisa o app (ou descarta).
    fn close_session(&self, session: Session) {
        self.layout.update(|layout| {
            layout.remove(session);
        });
        self.release(session);
    }

    /// Entrega ao app uma sessão que já saiu do layout (ou a descarta, se ele não quis).
    fn release(&self, session: Session) {
        let handler = self.on_closed.borrow().clone();
        match handler {
            Some(handler) => handler(session),
            None => session.dispose(),
        }
    }

    /// Pede o foco para o terminal da aba escolhida do painel ativo.
    fn request_focus(&self) {
        *self.request.borrow_mut() = self.active_terminal();
    }

    /// O id do terminal da aba escolhida do painel ativo.
    fn active_terminal(&self) -> Option<String> {
        self.layout.with_untracked(|layout| {
            let session = layout.pane(layout.active()).selected?;
            self.part(format_args!("terminal-{}", slug(&session.key_str())))
        })
    }

    /// A sessão do painel novo de uma divisão: a do app (`new_session`) ou a cópia da ativa.
    fn fresh_session(&self) -> Option<Session> {
        let make = self.new_session.borrow().clone();
        match make {
            Some(make) => Some(make()),
            None => self
                .active_session()
                .and_then(|session| session.duplicate()),
        }
    }

    /// A aba escolhida do painel ativo.
    fn active_session(&self) -> Option<Session> {
        self.layout
            .with_untracked(|layout| layout.pane(layout.active()).selected)
    }

    /// Divide o painel ativo para `direction` e põe `session` (se houver) no painel novo, que
    /// fica ativo.
    fn split_with(&self, direction: PaneDirection, session: Option<Session>) {
        self.layout.update(|layout| {
            let pane = layout.active();
            let created = layout.split(pane, direction);
            if let Some(session) = session {
                layout.add_tab(session, created);
            }
        });
        self.request_focus();
    }

    /// O lado em que o painel ativo se divide ao duplicar: à direita se ele é mais largo que
    /// alto, senão embaixo. A medida vem do tamanho do terminal (colunas e linhas, com a célula
    /// duas vezes mais alta que larga); sem ele, das frações do layout.
    fn longer_side(&self) -> PaneDirection {
        let cells = self.active_session().and_then(|session| {
            session.emulator(|emulator| (emulator.cols() as f64, emulator.rows() as f64 * 2.0))
        });
        let (width, height) = cells.unwrap_or_else(|| {
            self.layout.with_untracked(|layout| {
                let [_, _, w, h] = layout.rects()[layout.active()];
                (w, h)
            })
        });
        if width > height {
            PaneDirection::Right
        } else {
            PaneDirection::Down
        }
    }

    fn split(&self, direction: PaneDirection) {
        let session = self.fresh_session();
        self.split_with(direction, session);
    }

    fn duplicate(self: &Rc<Self>) {
        let direction = self.longer_side();
        let handler = self.on_duplicate.borrow().clone();
        if let Some(handler) = handler {
            if let Some(origin) = self.active_session() {
                let place = DuplicatePlace {
                    shared: Rc::clone(self),
                    direction,
                };
                handler(origin, place);
            }
            return;
        }
        let session = self.active_session().and_then(|s| s.duplicate());
        self.split_with(direction, session);
    }

    fn focus(&self, direction: PaneDirection) {
        self.layout.update(|layout| {
            if let Some(pane) = layout.neighbor(layout.active(), direction) {
                layout.set_active(pane);
            }
        });
        self.request_focus();
    }

    fn resize(&self, direction: PaneDirection) {
        self.layout.update(|layout| {
            layout.resize(layout.active(), direction);
        });
    }

    fn close_pane(&self) {
        self.close_pane_at(self.layout.with_untracked(PaneLayout::active));
    }

    /// Fecha o painel `pane`. Um painel vazio não tem abas para soltar.
    fn close_pane_at(&self, pane: usize) {
        let mut closed = Vec::new();
        self.layout.update(|layout| {
            closed = layout.close_pane(pane);
        });
        for session in closed {
            self.release(session);
        }
        self.request_focus();
    }

    fn zoom(&self) {
        self.layout.update(|layout| layout.zoom(layout.active()));
        self.request_focus();
    }

    /// Faz ativo o painel de `session`, se ainda não é.
    fn activate(&self, session: Session) {
        let pane = self
            .layout
            .with_untracked(|layout| layout.pane_of(session).filter(|&p| p != layout.active()));
        if let Some(pane) = pane {
            self.layout.update(|layout| layout.set_active(pane));
        }
    }

    /// A borda do painel `pane`.
    fn border(&self, pane: usize) -> PaneBorder {
        let mode = self.mode();
        self.layout.with(|layout| {
            if pane >= layout.pane_count() || layout.pane_count() < 2 || layout.zoomed().is_some() {
                return PaneBorder::None;
            }
            if layout.active() == pane || self.highlight.borrow().get() == Some(pane) {
                return PaneBorder::Solid;
            }
            match layout.pane(pane).selected {
                Some(session) if mode.targets(layout).contains(&session) => PaneBorder::Dashed,
                _ => PaneBorder::None,
            }
        })
    }

    /// As sessões do layout que nenhum terminal mostra agora.
    fn hidden(&self) -> Vec<Session> {
        self.layout.with_untracked(|layout| {
            let visible = layout.visible();
            layout
                .sessions()
                .into_iter()
                .filter(|session| !visible.contains(session))
                .collect()
        })
    }
}

/// Como o `TerminalPanes` se comporta no layout de fora: vale para o widget raiz que ele monta.
struct Root {
    grow: f64,
    width: Option<f64>,
    height: Option<f64>,
    visible: Option<Prop<bool>>,
}

impl Root {
    fn common(&self, id: Option<String>) -> Common {
        let mut common = Common::generated(id);
        common.grow = self.grow;
        common.width = self.width;
        common.height = self.height;
        common.visible = self.visible.clone();
        common
    }
}

/// O nome de uma chave de sessão num id: letras e números em minúsculas, o resto vira `-`.
fn slug(key: &str) -> String {
    key.chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .flat_map(char::to_lowercase)
        .collect()
}

/// A chave da aba escolhida do painel `pane`, ou vazia.
fn selected_key(layout: &PaneLayout, pane: usize) -> String {
    if pane >= layout.pane_count() {
        return String::new();
    }
    layout
        .pane(pane)
        .selected
        .map(|session| session.key_str())
        .unwrap_or_default()
}

/// Terminais em painéis: as sessões de um [`PaneLayout`] como abas, em painéis divididos por
/// divisores que se arrastam.
///
/// ```
/// use tomni_ui_core::{Column, View};
/// use tomni_ui_reactive::prelude::*;
/// use tomni_ui_terminal::{Loopback, PaneLayout, Session, TerminalPanes};
///
/// # let scope = Scope::root();
/// # scope.run(|| {
/// let sessoes = ["web-01", "web-02"]
///     .map(|nome| Session::new(Loopback::new().echo()).key(nome).name(nome));
/// let layout = signal(PaneLayout::with(sessoes));
/// layout.update(|l| l.grid(2, 1));
/// let paineis = TerminalPanes::bind(layout)
///     .id("paineis")
///     .on_session_closed(|sessao| sessao.dispose())
///     .grow(1);
/// # });
/// # scope.clear();
/// ```
///
/// O widget não guarda as sessões: elas vivem no [`PaneLayout`] (e no registro de
/// [`Session`]), e o app muda o layout pelo signal. Trocar o layout (`grid`, `single`, mover
/// uma aba) refaz só os painéis; as sessões, o histórico e a conexão continuam.
///
/// Cada painel é um `TabView` compacto com as sessões dele como abas, e só a aba escolhida
/// aparece. Uma sessão fica em **um** painel por vez. Sessões do layout que não estão à mostra
/// continuam sendo lidas pelo widget, para a conexão não encher nem ficar sem notícia.
///
/// Os ids, com `.id("paineis")`: `paineis-divisao-N` (as divisões, em preordem),
/// `paineis-painel-N-quadro` (o quadro do painel N), `paineis-painel-N` (o `TabView` dele, com
/// as abas `paineis-painel-N-{chave}`), `paineis-painel-N-vazio` (o texto do painel sem abas),
/// `paineis-painel-N-fechar` (o botão "Fechar o painel" dele, que só aparece havendo mais de um
/// painel; o menu de contexto do painel vazio tem o mesmo item) e `paineis-terminal-{chave}` (o
/// terminal da sessão). A dica de cada aba é `paineis-painel-N-{chave}-tooltip`.
///
/// A aba de uma sessão encerrada ([`Session::closed`]) troca o ícone por um aviso, e o terminal
/// dela fica com o contorno de aviso. [`TerminalPanes::on_tab_tooltip`] escolhe a dica da aba (sem
/// ela, a dica é o título), e [`PaneLayout::close_emptied`] faz o painel que perde a última aba
/// sumir sozinho.
///
/// A borda de 1 px na cor de destaque marca o painel ativo (o do terminal com foco). Com um
/// painel só, ou com zoom, não há borda.
pub struct TerminalPanes {
    common: Common,
    shared: Rc<Shared>,
    /// A forma e o zoom da última árvore montada.
    built: Option<(Shape, Option<usize>)>,
}

impl TerminalPanes {
    /// Mostra o layout que está em `layout`.
    pub fn bind(layout: Signal<PaneLayout>) -> Self {
        let mut panes = Self {
            common: Common::default(),
            shared: Rc::new(Shared {
                token: NEXT_TOKEN.with(|next| {
                    next.set(next.get() + 1);
                    next.get()
                }),
                id: RefCell::new(None),
                group: RefCell::new(None),
                on_tear_off: RefCell::new(None),
                on_tab_click: RefCell::new(None),
                menu_extra: RefCell::new(None),
                tab_tooltip: RefCell::new(None),
                tab_mark: RefCell::new(None),
                tab_mark_tint: RefCell::new(None),
                layout,
                broadcast: Cell::new(signal(Broadcast::None)),
                identify: RefCell::new(Prop::Value(false)),
                highlight: RefCell::new(Prop::Value(None)),
                tab_scheme: RefCell::new(Prop::Value(None)),
                active_indicator: RefCell::new(Prop::Value(false)),
                new_session: RefCell::new(None),
                on_duplicate: RefCell::new(None),
                on_closed: RefCell::new(None),
                configure: RefCell::new(None),
                inside: Cell::new(false),
                request: RefCell::new(None),
            }),
            built: None,
        };
        panes.register(Prop::Value(PaneShortcuts::default()));
        LIVE.with(|live| live.borrow_mut().push(Rc::downgrade(&panes.shared)));
        panes.watch_alerts(layout);
        panes
    }

    /// Pede atenção para a janela quando uma sessão do layout dá um aviso novo. O sistema só
    /// atende se a janela estiver sem o foco. Uma sessão que acabou de chegar ao layout não
    /// conta: o que ela já tinha avisado não é novidade.
    fn watch_alerts(&self, layout: Signal<PaneLayout>) {
        let window = WindowHandle::current();
        let known: RefCell<HashMap<String, u64>> = RefCell::new(HashMap::new());
        effect(move || {
            let mut now = HashMap::new();
            let mut raised = false;
            let mut known = known.borrow_mut();
            for session in layout.with(PaneLayout::sessions) {
                let alerts = session.alerts();
                let key = session.key_str();
                raised |= known.get(&key).is_some_and(|&before| alerts > before);
                now.insert(key, alerts);
            }
            *known = now;
            if raised {
                window.request_attention();
            }
        });
    }

    /// Id estável, usado em testes, no MCP e no lint. Os ids das partes saem dele.
    pub fn id(mut self, id: impl Into<String>) -> Self {
        let id = id.into();
        *self.shared.id.borrow_mut() = Some(id.clone());
        self.common.id = Some(id);
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

    /// Mostra ou esconde os painéis (valor fixo, closure ou signal). Escondidos, as sessões
    /// continuam sendo lidas.
    pub fn visible<M>(mut self, visible: impl IntoProp<bool, M>) -> Self {
        self.common.visible = Some(visible.into_prop());
        self
    }

    /// Para onde vai o que se digita: o signal com o [`Broadcast`]. Os painéis que recebem
    /// ganham a borda tracejada, e as abas, o ícone de transmissão.
    pub fn broadcast(self, mode: Signal<Broadcast>) -> Self {
        self.shared.broadcast.set(mode);
        self
    }

    /// Mostra no centro de cada painel um crachá com o número dele, a ordem das folhas do
    /// layout (`Painel 1` é o `pane(0)`; é a mesma do menu "Mover para o painel" da aba).
    /// Aceita um valor fixo, uma closure ou um signal; ligar e desligar não monta os painéis
    /// de novo. O crachá é uma sobreposição ([`tomni_ui_core::Placement::Over`]): fica acima
    /// do terminal e abaixo dos menus e diálogos, e o ponteiro e o teclado passam por ele. Num
    /// painel pequeno demais para o crachá (120 px mais a folga), ele não aparece; com zoom,
    /// só o painel ampliado tem crachá.
    ///
    /// **Adaptação:** o "Identificar" do Windows põe o número no canto de cada monitor; aqui
    /// ele fica no centro do painel, porque o canto tem as abas e a barra de rolagem.
    pub fn identify<M>(self, identify: impl IntoProp<bool, M>) -> Self {
        *self.shared.identify.borrow_mut() = identify.into_prop();
        self
    }

    /// O painel em destaque, pelo índice a partir de 0 (valor fixo, closure ou signal): o
    /// crachá dele fica na cor de destaque e o quadro ganha a borda sólida do painel ativo.
    /// Um índice que não existe é ignorado. Serve para mostrar para onde uma ação vai levar.
    pub fn highlight_pane<M>(self, pane: impl IntoProp<Option<usize>, M>) -> Self {
        *self.shared.highlight.borrow_mut() = pane.into_prop();
        self
    }

    /// Pinta a aba escolhida de cada painel com o fundo e o texto do esquema dos terminais
    /// (valor fixo, closure ou signal), como o Windows Terminal: a aba parece sair do terminal
    /// embaixo dela. `None` deixa as cores do `TabView`. Passe o mesmo esquema do `.scheme` dos
    /// terminais.
    pub fn tab_scheme<M>(self, scheme: impl IntoProp<Option<Scheme>, M>) -> Self {
        *self.shared.tab_scheme.borrow_mut() = scheme.into_prop();
        self
    }

    /// Uma barra de destaque embaixo da aba escolhida do painel ativo (o
    /// `.selected_indicator` do `TabView`): entre vários painéis, mostra onde vai o que se
    /// digita.
    pub fn active_tab_indicator<M>(self, on: impl IntoProp<bool, M>) -> Self {
        *self.shared.active_indicator.borrow_mut() = on.into_prop();
        self
    }

    /// Cria a sessão de um painel novo (ao dividir). Sem ela, o painel novo fica vazio.
    pub fn new_session(self, f: impl Fn() -> Session + 'static) -> Self {
        *self.shared.new_session.borrow_mut() = Some(Rc::new(f));
        self
    }

    /// Quem duplica a sessão no atalho de duplicar ([`PaneShortcuts::duplicate`], Alt+Shift+D).
    /// Sem ele, o painel faz `origem.duplicate()` sozinho e divide pelo lado maior. Com ele, o
    /// painel não duplica nada: chama `f` com a sessão ativa e um [`DuplicatePlace`], e o app
    /// cria a cópia como quiser (a pasta atual, a rede, os comandos ao conectar), inclusive
    /// depois de uma consulta demorada numa thread, e então entrega a cópia com
    /// [`DuplicatePlace::place`], que divide o painel que era o ativo. Se o app não entregar,
    /// nada muda na tela.
    pub fn on_duplicate(self, f: impl Fn(Session, DuplicatePlace) + 'static) -> Self {
        *self.shared.on_duplicate.borrow_mut() = Some(Rc::new(f));
        self
    }

    /// Uma sessão foi fechada pela interface (o ✕ da aba, o menu da aba, o atalho): ela já saiu
    /// do layout e `f` decide o que fazer com ela, em geral [`Session::dispose`]. Sem este
    /// handler, o widget descarta a sessão.
    pub fn on_session_closed(self, f: impl Fn(Session) + 'static) -> Self {
        *self.shared.on_closed.borrow_mut() = Some(Rc::new(f));
        self
    }

    /// Põe as abas deste widget num grupo de arrasto: elas passam a ir para qualquer
    /// `TerminalPanes` do mesmo `group` na thread, inclusive de outra janela. A sessão sai do
    /// layout de origem (sem ser descartada, com o histórico) e entra no de destino. Sem
    /// grupo, as abas só andam entre os painéis do próprio widget.
    pub fn drag_group(self, group: impl Into<String>) -> Self {
        *self.shared.group.borrow_mut() = Some(group.into());
        self
    }

    /// Uma aba foi solta fora de todas as janelas: `f(sessão, ponto)` roda com a sessão e o
    /// ponto na tela onde caiu (`None` onde o sistema não informa, como no Wayland). É o
    /// gancho de "destacar a aba numa janela nova"; em geral `f` abre a janela e chama
    /// [`move_session`]. Sem `f`, soltar fora não faz nada.
    pub fn on_tear_off(self, f: impl Fn(Session, Option<(i32, i32)>) + 'static) -> Self {
        *self.shared.on_tear_off.borrow_mut() = Some(Rc::new(f));
        self
    }

    /// A pessoa clicou numa aba (ou deu Enter ou Espaço nela): `f(sessão)` roda depois de a aba
    /// ser escolhida, inclusive quando ela já era a ativa, que não muda o layout (o
    /// [`TabView::on_tab_click`]). É o que o app usa para mostrar na árvore a conexão da aba
    /// clicada. Escolher pelo teclado ou pelo código não chama `f`.
    pub fn on_tab_click(self, f: impl Fn(Session) + 'static) -> Self {
        *self.shared.on_tab_click.borrow_mut() = Some(Rc::new(f));
        self
    }

    /// Dá às abas uma dica com o ponteiro parado em cima: `f(sessão)` devolve o texto, ou `None`
    /// para ficar o padrão, o título da aba. A closure é reativa (lê signals, como o
    /// [`Session::closed`]), e a dica muda com eles. É o que o app usa para mostrar o destino
    /// e o estado da conexão de cada aba. A dica tem o id `{id da aba}-tooltip`.
    pub fn on_tab_tooltip(self, f: impl Fn(Session) -> Option<String> + 'static) -> Self {
        *self.shared.tab_tooltip.borrow_mut() = Some(Rc::new(f));
        self
    }

    /// Dá às abas uma marca do app no lugar do ícone: `f(sessão)` devolve o símbolo, ou `None`
    /// para a aba ficar como estava. A closure é reativa (lê signals) e a marca muda com eles. A
    /// ordem dos ícones: a sessão encerrada (aviso) vale mais que a marca do app, e a marca vale
    /// mais que o do envio para várias abas. O anel de progresso e o ponto de atenção são
    /// outros indicadores e continuam aparecendo. É o que o app usa para mostrar que um
    /// assistente está usando a aba.
    pub fn on_tab_mark(self, f: impl Fn(Session) -> Option<Symbol> + 'static) -> Self {
        *self.shared.tab_mark.borrow_mut() = Some(Rc::new(f));
        self
    }

    /// Dá cor à marca de [`on_tab_mark`](Self::on_tab_mark): `f(sessão)` devolve a cor, ou
    /// `None` para a marca ficar na cor do título. Só vale enquanto a marca é a que aparece (a
    /// sessão encerrada e o aviso dela não têm cor). A closure é reativa.
    pub fn on_tab_mark_tint(self, f: impl Fn(Session) -> Option<ThemeColor> + 'static) -> Self {
        *self.shared.tab_mark_tint.borrow_mut() = Some(Rc::new(f));
        self
    }

    /// Acrescenta itens ao menu de contexto das abas: `f(sessão, menu)` recebe o menu pronto
    /// (antes do separador e dos itens de reiniciar e fechar) e devolve o menu com os itens
    /// a mais, como "Mover para uma janela nova".
    pub fn tab_menu_extra(self, f: impl Fn(Session, MenuFlyout) -> MenuFlyout + 'static) -> Self {
        *self.shared.menu_extra.borrow_mut() = Some(Rc::new(f));
        self
    }

    /// Troca os atalhos de teclado (os padrões estão em [`PaneShortcuts::default`]).
    /// [`PaneShortcuts::none`] desliga todos. Aceita um valor fixo, uma closure ou um signal:
    /// os atalhos são lidos na hora da tecla, então trocar o signal troca os atalhos sem
    /// montar a tela de novo.
    pub fn shortcuts<M>(mut self, shortcuts: impl IntoProp<PaneShortcuts, M>) -> Self {
        self.common.accelerators.clear();
        self.register(shortcuts.into_prop());
        self
    }

    /// O estado atual (layout, abas, painel ativo, zoom e envio), sem registrar dependência
    /// em signal: serve às ferramentas e aos testes.
    pub fn info(&self) -> PanesInfo {
        let broadcast = untrack(|| self.shared.mode()).name();
        let identifying = untrack(|| self.shared.identify.borrow().get());
        let highlighted = untrack(|| self.shared.highlight.borrow().get());
        self.shared.layout.with_untracked(|layout| PanesInfo {
            layout: layout.to_string(),
            panes: (0..layout.pane_count())
                .map(|index| {
                    let pane = layout.pane(index);
                    PaneInfo {
                        tabs: pane
                            .tabs
                            .iter()
                            .map(|session| (session.key_str(), untrack(|| session.title())))
                            .collect(),
                        selected: pane.selected.map(|session| session.key_str()),
                    }
                })
                .collect(),
            active: layout.active(),
            zoomed: layout.zoomed(),
            broadcast,
            identifying,
            highlighted: highlighted.filter(|&pane| pane < layout.pane_count()),
        })
    }

    /// O id do terminal da aba escolhida do painel ativo (`{id}-terminal-{chave}`), o que
    /// recebe o que se digita. `None` sem id no widget ou com o painel ativo vazio.
    pub fn active_terminal(&self) -> Option<String> {
        self.shared.active_terminal()
    }

    /// O que as sessões deste widget estão fazendo, somado: a média do progresso, se alguma
    /// anda e quantas abas pedem atenção. **Reativo**: lê o layout e o progresso e os avisos
    /// de cada sessão, então serve dentro de um efeito (o app leva isso ao dock).
    pub fn activity(&self) -> PanesActivity {
        activity_of(&self.shared.layout)
    }

    /// Registra um acelerador por comando, com as teclas lidas de `shortcuts` na hora da tecla.
    fn register(&mut self, shortcuts: Prop<PaneShortcuts>) {
        let shortcuts = Rc::new(shortcuts);
        let mut add = |keys: fn(&PaneShortcuts) -> &[Shortcut], action: Rc<dyn Fn()>| {
            let shortcuts = Rc::clone(&shortcuts);
            let keys = Prop::Dynamic(Rc::new(move || keys(&shortcuts.get()).to_vec()));
            self.common
                .accelerators
                .push(Accelerator::for_keys(keys, move || action()));
        };
        let shared = &self.shared;
        let command = |run: fn(&Shared)| -> Rc<dyn Fn()> {
            let shared = Rc::clone(shared);
            Rc::new(move || run(&shared))
        };
        add(|s| &s.split_down, command(|s| s.split(PaneDirection::Down)));
        add(
            |s| &s.split_right,
            command(|s| s.split(PaneDirection::Right)),
        );
        add(|s| &s.duplicate, {
            let shared = Rc::clone(shared);
            Rc::new(move || shared.duplicate())
        });
        let sides = [
            PaneDirection::Left,
            PaneDirection::Right,
            PaneDirection::Up,
            PaneDirection::Down,
        ];
        let focus: [fn(&PaneShortcuts) -> &[Shortcut]; 4] = [
            |s| &s.focus[0],
            |s| &s.focus[1],
            |s| &s.focus[2],
            |s| &s.focus[3],
        ];
        let resize: [fn(&PaneShortcuts) -> &[Shortcut]; 4] = [
            |s| &s.resize[0],
            |s| &s.resize[1],
            |s| &s.resize[2],
            |s| &s.resize[3],
        ];
        for (keys, side) in focus.into_iter().zip(sides) {
            let shared = Rc::clone(shared);
            add(keys, Rc::new(move || shared.focus(side)));
        }
        for (keys, side) in resize.into_iter().zip(sides) {
            let shared = Rc::clone(shared);
            add(keys, Rc::new(move || shared.resize(side)));
        }
        add(|s| &s.close_pane, command(Shared::close_pane));
        add(|s| &s.zoom, command(Shared::zoom));
    }

    /// Ajusta cada [`Terminal`] que o widget monta (o esquema de cores, a fonte). O id, a
    /// sessão e a ligação com os painéis já vêm prontos.
    pub fn terminal(self, f: impl Fn(Terminal) -> Terminal + 'static) -> Self {
        *self.shared.configure.borrow_mut() = Some(Rc::new(f));
        self
    }
}

/// Tira `session` de `layout`, se ela está lá, sem descartá-la.
fn leave(layout: &Signal<PaneLayout>, session: Session) {
    if layout.with_untracked(|l| l.pane_of(session).is_some()) {
        layout.update(|l| {
            l.remove(session);
        });
    }
}

/// Move `session` para o painel `pane` do layout `to`, tirando-a do layout de qualquer
/// `TerminalPanes` vivo (de outra janela, por exemplo). A sessão não é descartada: segue viva,
/// com o histórico e a conexão. É o que `on_tear_off` usa para levar a aba a uma janela nova.
///
/// Só os layouts mostrados por um `TerminalPanes` vivo são esvaziados dela; o app cuida dos
/// que guarda por conta própria.
pub fn move_session(session: Session, to: Signal<PaneLayout>, pane: usize) {
    for other in live() {
        leave(&other.layout, session);
    }
    to.update(|layout| layout.add_tab(session, pane));
}

/// O terminal da sessão `session`, ligado ao painel dele.
fn terminal(shared: &Rc<Shared>, session: Session) -> Terminal {
    let focus = Rc::clone(shared);
    let mount = Rc::clone(shared);
    let route = Rc::clone(shared);
    let mut terminal = Terminal::session(session)
        .grow(1)
        .router(Rc::new(move |origin| route.recipients(origin)))
        .on_focus(Rc::new(move |session, focused| {
            focus.inside.set(focused);
            if focused {
                focus.activate(session);
            }
        }))
        .focus_mount(Rc::new(move |session| {
            mount.inside.get()
                && mount
                    .layout
                    .with_untracked(|layout| layout.pane(layout.active()).selected == Some(session))
        }));
    if let Some(id) = shared.part(format_args!("terminal-{}", slug(&session.key_str()))) {
        terminal = terminal.id(id);
    }
    let configure = shared.configure.borrow().clone();
    match configure {
        Some(configure) => configure(terminal),
        None => terminal,
    }
}

/// O signal da fração da divisão `index`, ligado nos dois sentidos ao layout. Compara antes de
/// gravar, senão os dois efeitos se alimentariam.
fn ratio_signal(shared: &Shared, index: usize) -> Signal<f64> {
    let layout = shared.layout;
    let initial = layout.with_untracked(|l| l.ratios().get(index).copied().unwrap_or(0.5));
    let ratio = signal(initial);
    effect(move || {
        let wanted = ratio.get();
        let differs = layout.with_untracked(|l| {
            l.ratios()
                .get(index)
                .is_some_and(|current| (current - wanted).abs() > RATIO_EPSILON)
        });
        if differs {
            layout.update(|l| {
                l.set_ratio(index, wanted);
            });
        }
    });
    effect(move || {
        let current = layout.with(|l| l.ratios().get(index).copied());
        if let Some(current) = current
            && (ratio.get_untracked() - current).abs() > RATIO_EPSILON
        {
            ratio.set(current);
        }
    });
    ratio
}

/// O signal com a chave da aba escolhida do painel `pane`, ligado nos dois sentidos ao layout.
fn selected_signal(shared: &Shared, pane: usize) -> Signal<String> {
    let layout = shared.layout;
    let selected = signal(layout.with_untracked(|l| selected_key(l, pane)));
    effect(move || {
        let key = selected.get();
        let chosen = layout.with_untracked(|l| {
            if pane >= l.pane_count() {
                return None;
            }
            let pane = l.pane(pane);
            pane.tabs
                .iter()
                .copied()
                .find(|session| session.key_str() == key)
                .filter(|&session| pane.selected != Some(session))
        });
        if let Some(session) = chosen {
            layout.update(|l| l.select(session));
        }
    });
    effect(move || {
        let key = layout.with(|l| selected_key(l, pane));
        if selected.get_untracked() != key {
            selected.set(key);
        }
    });
    selected
}

/// O quadro do painel `pane`: o `TabView` das sessões dele e o aviso de painel vazio.
fn pane_frame(shared: &Rc<Shared>, pane: usize, common: Common) -> Box<dyn Widget> {
    let layout = shared.layout;
    let selected = selected_signal(shared, pane);
    let group = shared.group();

    let tabs = move || layout.with(|l| tabs_of(l, pane));
    let has_tabs = move || layout.with(|l| !tabs_of(l, pane).is_empty());

    // A aba escolhida está à vista: o que ela avisar já está visto, e o ponto não acende nela.
    // Um efeito por painel, que acompanha a escolha e os avisos da aba escolhida.
    effect(move || {
        if let Some(session) = Session::find(&selected.get()) {
            let _ = session.alerts();
            untrack(|| session.mark_seen());
        }
    });
    let for_content = Rc::clone(shared);
    let for_close = Rc::clone(shared);
    let for_menu = Rc::clone(shared);
    let mut view = TabView::bind(selected)
        .density(Density::Compact)
        .content_padding(0.0)
        .grow(1)
        .visible(has_tabs)
        .items(
            tabs,
            |session: &Session| session.key_str(),
            |session: &Session| {
                let session = *session;
                move || session.title()
            },
            move |session: Session| terminal(&for_content, session),
        )
        .on_close(move |key| {
            if let Some(session) = Session::find(&key) {
                for_close.close_session(session);
            }
        })
        .on_reorder(move |from, to| {
            layout.update(|l| {
                if let Some(&session) = tabs_of(l, pane).get(from) {
                    l.move_tab_to(session, pane, to);
                }
            });
        })
        .on_tab_click({
            let shared = Rc::clone(shared);
            move |key| {
                let on_click = shared.on_tab_click.borrow().clone();
                if let (Some(f), Some(session)) = (on_click, Session::find(&key)) {
                    f(session);
                }
            }
        })
        .tab_context_flyout(move |key| tab_menu(&for_menu, pane, key))
        .drag_group(group.clone())
        .on_tab_drop({
            let shared = Rc::clone(shared);
            move |key, index| {
                if let Some(session) = Session::find(&key) {
                    shared.receive(session, pane, index);
                }
            }
        })
        .tab_icon({
            let shared = Rc::clone(shared);
            move |key| {
                let session = Session::find(&key)?;
                // A sessão encerrada vale mais que o envio: ela nem recebe o que se digita.
                if session.closed() {
                    return Some(Symbol::Warning);
                }
                let mark = shared.tab_mark.borrow().clone();
                if let Some(symbol) = mark.and_then(|mark| mark(session)) {
                    return Some(symbol);
                }
                let mode = shared.mode();
                let receives = layout.with(|l| mode.targets(l).contains(&session));
                receives.then_some(Symbol::Broadcast)
            }
        })
        .tab_icon_tint({
            let shared = Rc::clone(shared);
            move |key| {
                let session = Session::find(&key)?;
                // O aviso da sessão que caiu com motivo é vermelho; encerrada sem motivo fica como
                // está. A marca do app só vale com a sessão aberta.
                if session.closed() {
                    return session
                        .close_reason()
                        .map(|_| ThemeColor::Token(Token::SystemFillColorCritical));
                }
                let mark = shared.tab_mark.borrow().clone()?;
                mark(session)?;
                let tint = shared.tab_mark_tint.borrow().clone()?;
                tint(session)
            }
        })
        .tab_progress(move |key| {
            let session = Session::find(&key)?;
            // O programa fala depois de a conexão abrir: até lá, o anel indeterminado.
            para_tab(session.progress()).or_else(|| session.connecting().then(connecting_ring))
        })
        .tab_connecting(move |key| Session::find(&key).is_some_and(|s| s.connecting()))
        .tab_attention(move |key| Session::find(&key).is_some_and(|s| s.attention()))
        .selected_colors({
            let shared = Rc::clone(shared);
            move || {
                let scheme = shared.tab_scheme.borrow().clone();
                scheme.get().map(Scheme::tab_colors)
            }
        })
        .selected_indicator({
            let shared = Rc::clone(shared);
            move || {
                let on = shared.active_indicator.borrow().clone();
                on.get() && layout.with(|l| l.active() == pane)
            }
        });
    let tooltip = shared.tab_tooltip.borrow().clone();
    if let Some(tooltip) = tooltip {
        view = view.tab_tooltip(move |key| Session::find(&key).and_then(|s| tooltip(s)));
    }
    let tear_off = shared.on_tear_off.borrow().clone();
    if let Some(tear_off) = tear_off {
        view = view.on_tab_drop_outside(move |key, at| {
            if let Some(session) = Session::find(&key) {
                tear_off(session, at);
            }
        });
    }
    let strings = tomni_ui_core::strings();
    let mut text = Text::new(strings.pane_empty.to_string()).secondary();
    let mut close = Button::new(strings.pane_close.to_string())
        .icon(Symbol::Dismiss)
        .height(44)
        .on_click({
            let shared = Rc::clone(shared);
            move || shared.close_pane_at(pane)
        });
    let can_close = {
        let layout = shared.layout;
        move || layout.with(|l| l.pane_count() > 1)
    };
    let mut menu = MenuFlyout::new();
    if let Some(id) = shared.part(format_args!("painel-{pane}-vazio-menu")) {
        menu = menu.id(id);
    }
    let item = shared
        .part(format_args!("painel-{pane}-fechar-menu"))
        .unwrap_or_else(|| format!("painel-{pane}-fechar-menu"));
    let menu = menu
        .item(item, strings.pane_close.to_string(), {
            let shared = Rc::clone(shared);
            move || shared.close_pane_at(pane)
        })
        .item_visible(can_close);
    if let Some(id) = shared.part(format_args!("painel-{pane}")) {
        view = view.id(id);
    }
    if let Some(id) = shared.part(format_args!("painel-{pane}-vazio")) {
        text = text.id(id);
    }
    if let Some(id) = shared.part(format_args!("painel-{pane}-fechar")) {
        close = close.id(id);
    }
    let empty = Column::new()
        .gap(12)
        .align(Align::Center)
        .justify(Justify::Center)
        .grow(1)
        .visible(move || !has_tabs())
        .context_flyout(menu)
        .child(text)
        .child(close.visible(can_close));

    let border_shared = Rc::clone(shared);
    let drop_target = Rc::clone(shared);
    Box::new(PaneFrame::new(
        common,
        Prop::Dynamic(Rc::new(move || border_shared.border(pane))),
        format!("tab:{group}"),
        Rc::new(move |key| {
            if let Some(session) = Session::find(&key) {
                drop_target.receive(session, pane, usize::MAX);
            }
        }),
        vec![Box::new(view), Box::new(empty), badge(shared, pane)],
    ))
}

/// O crachá com o número do painel `pane`: `{id}-painel-{n}-numero`.
fn badge(shared: &Rc<Shared>, pane: usize) -> Box<dyn Widget> {
    let (on, mark) = (Rc::clone(shared), Rc::clone(shared));
    Box::new(PaneBadge::new(
        shared.part(format_args!("painel-{pane}-numero")),
        pane,
        Prop::Dynamic(Rc::new(move || on.identify.borrow().get())),
        Prop::Dynamic(Rc::new(move || mark.highlight.borrow().get() == Some(pane))),
    ))
}

/// O menu de contexto da aba `key` do painel `pane`: mover para outro painel, abrir um painel
/// novo para ela, reiniciar a sessão e fechar a aba. Sem `.shortcut()` nos itens, que
/// repetiriam o atalho em cada aba (o lint acusaria `duplicate-accelerator`).
///
/// Os ids têm a chave da aba, porque cada aba tem o seu menu:
/// `{id}-painel-{n}-{chave}-menu` e, dentro, `-mover` (com a lista `-mover-lista` e os itens
/// `-mover-{m}`), `-direita`, `-abaixo`, `-reiniciar` e `-fechar`.
fn tab_menu(shared: &Rc<Shared>, pane: usize, key: String) -> MenuFlyout {
    let name = format!("painel-{pane}-{}-menu", slug(&key));
    let base = shared.part(&name).unwrap_or(name);
    let count = shared.layout.with_untracked(PaneLayout::pane_count);
    // Cada ação acha a sessão pela chave na hora de rodar: a aba pode ter mudado de painel.
    let act = |run: fn(&Shared, Session)| {
        let (shared, key) = (Rc::clone(shared), key.clone());
        move || {
            if let Some(session) = Session::find(&key) {
                run(&shared, session);
            }
        }
    };
    let strings = tomni_ui_core::strings();
    let mut menu = MenuFlyout::new().id(base.clone());
    if count > 1 {
        let mut targets = MenuFlyout::new().id(format!("{base}-mover-lista"));
        for other in (0..count).filter(|&other| other != pane) {
            let (shared, key) = (Rc::clone(shared), key.clone());
            targets = targets.item(
                format!("{base}-mover-{other}"),
                strings.pane_name.replace("{n}", &(other + 1).to_string()),
                move || {
                    if let Some(session) = Session::find(&key) {
                        shared.layout.update(|l| l.move_tab(session, other));
                        shared.request_focus();
                    }
                },
            );
        }
        menu = menu.submenu(
            format!("{base}-mover"),
            strings.pane_move_to.to_string(),
            targets,
        );
    }
    for (suffix, label, direction) in [
        (
            "direita",
            strings.pane_new_right.to_string(),
            PaneDirection::Right,
        ),
        (
            "abaixo",
            strings.pane_new_below.to_string(),
            PaneDirection::Down,
        ),
    ] {
        let (shared, key) = (Rc::clone(shared), key.clone());
        menu = menu.item(format!("{base}-{suffix}"), label, move || {
            if let Some(session) = Session::find(&key) {
                shared.layout.update(|l| {
                    let created = l.split(pane, direction);
                    l.move_tab(session, created);
                });
                shared.request_focus();
            }
        });
    }
    let extra = shared.menu_extra.borrow().clone();
    if let Some(extra) = extra
        && let Some(session) = Session::find(&key)
    {
        menu = extra(session, menu);
    }
    menu.separator()
        .item(
            format!("{base}-reiniciar"),
            strings.pane_restart.to_string(),
            act(|_, session| {
                session.restart();
            }),
        )
        .item(
            format!("{base}-fechar"),
            strings.pane_close_tab.to_string(),
            act(|shared, session| shared.close_session(session)),
        )
}

/// As abas do painel `pane`.
/// A soma da atividade das sessões de `layout` (rastreada).
fn activity_of(layout: &Signal<PaneLayout>) -> PanesActivity {
    let sessions = layout.with(PaneLayout::sessions);
    let (mut sum, mut with_value) = (0.0, 0_u32);
    let mut activity = PanesActivity::default();
    for session in sessions {
        let progress = session.progress();
        if progress != TermProgress::None {
            activity.busy = true;
        }
        if let Some(percent) = progress.percent() {
            sum += f64::from(percent) / 100.0;
            with_value += 1;
        }
        if session.attention() {
            activity.attention += 1;
        }
    }
    if with_value > 0 {
        activity.progress = Some(sum / f64::from(with_value));
    }
    activity
}

/// O progresso do terminal no anel da aba: o valor vira fração; sem valor, é indeterminado.
fn para_tab(progress: TermProgress) -> Option<TabProgress> {
    let value = progress.percent().map(|percent| f64::from(percent) / 100.0);
    let status = match progress {
        TermProgress::None => return None,
        TermProgress::Normal(_) | TermProgress::Indeterminate => ProgressStatus::Normal,
        TermProgress::Paused(_) => ProgressStatus::Paused,
        TermProgress::Error(_) => ProgressStatus::Error,
    };
    Some(TabProgress { value, status })
}

/// O anel de uma aba cuja conexão ainda está abrindo.
fn connecting_ring() -> TabProgress {
    TabProgress {
        value: None,
        status: ProgressStatus::Normal,
    }
}

fn tabs_of(layout: &PaneLayout, pane: usize) -> Vec<Session> {
    if pane >= layout.pane_count() {
        return Vec::new();
    }
    layout.pane(pane).tabs.clone()
}

/// Monta a árvore de `shape`. `root` é o estilo do widget de cima, e `None` nos de dentro.
fn build(shared: &Rc<Shared>, shape: &Shape, root: Option<&Root>) -> Box<dyn Widget> {
    match shape {
        Shape::Pane(pane) => {
            let id = shared.part(format_args!("painel-{pane}-quadro"));
            let common = match root {
                Some(root) => root.common(id),
                None => {
                    let mut common = Common::generated(id);
                    common.grow = 1.0;
                    common
                }
            };
            pane_frame(shared, *pane, common)
        }
        Shape::Split {
            row,
            index,
            first,
            second,
        } => {
            let first = build(shared, first, None);
            let second = build(shared, second, None);
            let mut split = Split::boxed(*row, first, second).ratio(ratio_signal(shared, *index));
            if let Some(id) = shared.part(format_args!("divisao-{index}")) {
                split = split.id(id);
            }
            split = match root {
                Some(root) => {
                    let mut split = split.grow(root.grow);
                    if let Some(width) = root.width {
                        split = split.width(width);
                    }
                    if let Some(height) = root.height {
                        split = split.height(height);
                    }
                    if let Some(visible) = root.visible.clone() {
                        split = split.visible(move || visible.get());
                    }
                    split
                }
                None => split.grow(1),
            };
            Box::new(split)
        }
    }
}

impl Widget for TerminalPanes {
    fn kind(&self) -> &'static str {
        "terminal_panes"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::Fragment
    }

    fn names_children(&self) -> bool {
        true
    }

    fn fragment(&self) -> bool {
        true
    }

    fn track_children(&self) {
        let _ = self.shared.layout.get();
    }

    fn reconcile(&mut self) -> Option<Vec<Slot>> {
        let state = self
            .shared
            .layout
            .with(|layout| (layout.shape(), layout.zoomed()));
        if self.built.as_ref() == Some(&state) {
            return None;
        }
        self.built = Some(state.clone());
        let shared = Rc::clone(&self.shared);
        let root = Root {
            grow: self.common.grow,
            width: self.common.width,
            height: self.common.height,
            visible: self.common.visible.clone(),
        };
        let (shape, zoomed) = state;
        Some(vec![Slot::New(Box::new(move || match zoomed {
            Some(pane) => build(&shared, &Shape::Pane(pane), Some(&root)),
            None => build(&shared, &shape, Some(&root)),
        }))])
    }

    fn take_focus_request(&mut self) -> Option<String> {
        self.shared.request.borrow_mut().take()
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn animating(&self) -> bool {
        true
    }

    fn ticking(&self) -> bool {
        self.shared.hidden().into_iter().any(Session::pending)
    }

    fn tick(&mut self, _dt: Duration) {
        let hidden = self.shared.hidden();
        let Some(share) = u32::try_from(hidden.len()).ok().filter(|&n| n > 0) else {
            return;
        };
        let _ocultos = tomni_ui_core::stages::stage("terminais ocultos");
        for session in hidden {
            session.pump(Some(READ_BUDGET / share));
        }
    }
}

impl fmt::Debug for TerminalPanes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TerminalPanes")
            .field("common", &self.common)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use tomni_ui_reactive::Scope;

    use super::*;
    use crate::transport::Loopback;

    fn sessao() -> Session {
        Session::new(Loopback::new())
    }

    #[test]
    fn o_progresso_do_terminal_vira_o_do_anel() {
        assert_eq!(para_tab(TermProgress::None), None);
        let normal = |value, status| Some(TabProgress { value, status });
        assert_eq!(
            para_tab(TermProgress::Normal(45)),
            normal(Some(0.45), ProgressStatus::Normal)
        );
        assert_eq!(
            para_tab(TermProgress::Indeterminate),
            normal(None, ProgressStatus::Normal)
        );
        assert_eq!(
            para_tab(TermProgress::Paused(Some(60))),
            normal(Some(0.6), ProgressStatus::Paused)
        );
        assert_eq!(
            para_tab(TermProgress::Paused(None)),
            normal(None, ProgressStatus::Paused)
        );
        assert_eq!(
            para_tab(TermProgress::Error(Some(100))),
            normal(Some(1.0), ProgressStatus::Error)
        );
    }

    #[test]
    fn a_atividade_soma_a_media_o_ocupado_e_a_atencao() {
        let scope = Scope::root();
        scope.run(|| {
            let (a, b, c) = (sessao(), sessao(), sessao());
            let layout = signal(PaneLayout::with(vec![a, b, c]));
            assert_eq!(activity_of(&layout), PanesActivity::default());

            // Indeterminada ocupa, mas não entra na média.
            a.feed(b"\x1b]9;4;3\x07");
            assert_eq!(
                activity_of(&layout),
                PanesActivity {
                    progress: None,
                    busy: true,
                    attention: 0
                }
            );

            // A média conta as com valor, inclusive a pausada e a com erro.
            b.feed(b"\x1b]9;4;1;40\x07");
            c.feed(b"\x1b]9;4;2;80\x07");
            let activity = activity_of(&layout);
            assert!(activity.busy);
            assert!((activity.progress.unwrap() - 0.6).abs() < 1e-9);

            // Os avisos não vistos contam uma vez por aba.
            a.feed(b"\x07\x07");
            b.feed(b"\x07");
            assert_eq!(activity_of(&layout).attention, 2);
            a.mark_seen();
            assert_eq!(activity_of(&layout).attention, 1);

            a.feed(b"\x1b]9;4;0\x07");
            b.feed(b"\x1b]9;4;0\x07");
            c.feed(b"\x1b]9;4;0\x07");
            let activity = activity_of(&layout);
            assert!(!activity.busy && activity.progress.is_none());

            // Uma sessão que sai do layout sai da soma.
            layout.update(|l| {
                l.remove(b);
            });
            assert_eq!(activity_of(&layout).attention, 0);
            for session in [a, b, c] {
                session.dispose();
            }
        });
        scope.clear();
    }
}
