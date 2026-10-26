//! `DockHost`: painéis encaixáveis nas bordas da janela, com alfinete.
//!
//! Cada painel ([`DockPane`]) fica num lado ([`DockSide`]) e tem um modo ([`DockMode`]):
//! **fixado** (ocupa espaço ao lado do conteúdo), **auto-ocultar** (um botão numa faixa da
//! borda; abre por cima do conteúdo), **flutuando** (numa janela própria) ou **fechado**. O
//! estado inteiro mora num `Signal<DockLayout>` do app, que o grava e devolve; o
//! [`DockController`] liga menus e atalhos.
//!
//! # Adaptações
//!
//! O WinUI 3 não tem alfinete, painel flutuante, painel embaixo nem vários painéis por lado.
//! O `DockHost` junta o `SplitView` (`Inline` para o fixado, `CompactOverlay` para o
//! auto-ocultar, com o mesmo light dismiss) com botões de cabeçalho no visual dos botões
//! "subtle" do Fluent:
//!
//! - a faixa mostra botões com ícone e dica (o título), sem texto girado; a faixa de baixo
//!   mostra o ícone e o título, um ao lado do outro, e tem 32 px de altura;
//! - sem animação de abrir e recolher, como o `NavigationView` da lib;
//! - o painel flutuante é uma janela comum ([`open_window`](crate::open_window)), sem "sempre
//!   por cima";
//! - a faixa não tem borda própria: ela se separa do conteúdo só pelo fundo;
//! - com a janela pequena, o conteúdo encolhe primeiro, até `min_content`; depois as áreas
//!   fixadas dividem o que sobra;
//! - `closable(false)` e `floatable(false)` só tiram o botão e o item de menu: o
//!   [`DockController`] continua fechando e flutuando o painel, para o app decidir;
//! - as janelas flutuantes pertencem ao `DockHost`: se o host sai da árvore (a tela some), elas
//!   não fecham sozinhas, e o app as fecha ao trocar de tela;
//! - abrir ao parar o ponteiro na faixa usa um atraso de 400 ms ([`DOCK_HOVER_DELAY`]), o mesmo
//!   do submenu, para não abrir ao passar de raspão;
//! - o Esc dentro do painel aberto por cima devolve o foco ao botão da faixa dele, e não a
//!   quem tinha o foco antes: o controlador não enxerga o foco de fora.
//!
//! # O conteúdo é montado de novo
//!
//! O conteúdo de um painel é um construtor (`Fn() -> View`), e não um widget pronto, porque um
//! widget não muda de pai na árvore: trocar o painel de lugar (fixado para auto-ocultar,
//! outro lado, maximizar, flutuar) o monta de novo. **Guarde o estado em signals de fora** (a
//! seleção da árvore, a nota aberta, a fila). Recolher e abrir um painel auto-ocultar não
//! monta de novo: o painel recolhido continua na árvore, só que invisível. Fechar desmonta.

mod build;
mod parts;
mod shared;

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::rc::Rc;
use std::time::Duration;

use tomni_ui_reactive::{Get, Set, Signal, signal, untrack};

use crate::kurbo::Size;
use crate::prop::{IntoProp, Prop};
use crate::symbol::Symbol;
use crate::widget::{Common, Direction, Flex, Look, View, Widget};
use crate::windows::WindowGeometry;

use shared::{PaneSpec, Shared};

/// Quanto o ponteiro precisa ficar parado num botão da faixa para o painel abrir por cima.
pub const DOCK_HOVER_DELAY: Duration = Duration::from_millis(400);
/// Quanto o ponteiro precisa ficar fora do painel aberto por cima e da faixa (e o foco também)
/// para o painel recolher.
pub const DOCK_LEAVE_DELAY: Duration = Duration::from_millis(400);
/// Largura da faixa lateral dos painéis auto-ocultar.
pub const DOCK_STRIP_WIDTH: f64 = 48.0;
/// Altura da faixa de baixo.
pub const DOCK_STRIP_HEIGHT: f64 = 32.0;
/// Tamanho padrão de um painel (a área fixada e o painel aberto por cima).
pub const DOCK_PANE_SIZE: f64 = 320.0;
/// Altura do cabeçalho de um painel.
pub const DOCK_HEADER_HEIGHT: f64 = 32.0;

/// Onde o painel encaixa.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DockSide {
    /// À esquerda do conteúdo.
    Left,
    /// À direita do conteúdo.
    Right,
    /// Embaixo do conteúdo.
    Bottom,
}

impl DockSide {
    /// Os três lados, na ordem do layout.
    pub const ALL: [Self; 3] = [Self::Left, Self::Right, Self::Bottom];

    /// O nome do lado nos ids gerados.
    pub(crate) fn id_name(self) -> &'static str {
        match self {
            Self::Left => "esquerda",
            Self::Right => "direita",
            Self::Bottom => "embaixo",
        }
    }
}

/// Como o painel aparece.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockMode {
    /// Ocupa espaço ao lado do conteúdo.
    Pinned,
    /// Um botão na faixa da borda; abre por cima do conteúdo.
    AutoHide,
    /// Numa janela própria.
    Floating,
    /// Escondido; volta pelo [`DockController::show`].
    Closed,
}

/// O estado de um painel.
#[derive(Clone, Debug, PartialEq)]
pub struct DockPaneState {
    /// Como o painel aparece agora.
    pub mode: DockMode,
    /// Em que lado ele está.
    pub side: DockSide,
    /// A parte da área fixada que o painel ocupa entre os fixados do mesmo lado (pesos).
    pub weight: f64,
    /// A largura (ou a altura, embaixo) do painel aberto por cima.
    pub overlay_size: f64,
    /// A geometria da janela flutuante, quando já flutuou alguma vez.
    pub floating: Option<WindowGeometry>,
    /// Para onde o painel volta ao reabrir ou ao sair da janela flutuante
    /// ([`DockMode::Pinned`] ou [`DockMode::AutoHide`]).
    pub docked: DockMode,
}

/// Uma área fixada: o tamanho e a ordem dos painéis do lado.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DockArea {
    /// A largura (ou a altura, embaixo) da área fixada, em pixels lógicos. Zero deixa o
    /// `DockHost` usar o tamanho do primeiro painel do lado.
    pub size: f64,
    /// As chaves dos painéis deste lado, na ordem (em qualquer modo).
    pub order: Vec<String>,
}

/// O estado inteiro do [`DockHost`]. O app guarda e devolve.
///
/// Um layout vazio ([`DockLayout::default`]) vale: o `DockHost` o preenche, na montagem, com o
/// padrão dos painéis. Uma chave que o layout tem e os painéis não são ignorada (e some do
/// signal); um painel que o layout não conhece entra com o padrão do construtor.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DockLayout {
    /// A área fixada à esquerda.
    pub left: DockArea,
    /// A área fixada à direita.
    pub right: DockArea,
    /// A área fixada embaixo.
    pub bottom: DockArea,
    /// O estado de cada painel, pela chave.
    pub panes: BTreeMap<String, DockPaneState>,
    /// O painel maximizado.
    pub maximized: Option<String>,
}

impl DockLayout {
    /// O estado do painel `key`.
    pub fn pane(&self, key: &str) -> Option<&DockPaneState> {
        self.panes.get(key)
    }

    /// A área do lado `side`.
    pub fn area(&self, side: DockSide) -> &DockArea {
        match side {
            DockSide::Left => &self.left,
            DockSide::Right => &self.right,
            DockSide::Bottom => &self.bottom,
        }
    }

    /// A área do lado `side`, para mudar.
    pub fn area_mut(&mut self, side: DockSide) -> &mut DockArea {
        match side {
            DockSide::Left => &mut self.left,
            DockSide::Right => &mut self.right,
            DockSide::Bottom => &mut self.bottom,
        }
    }
}

/// Um construtor de conteúdo: roda de novo a cada troca de lugar do painel.
pub(crate) type Content = Rc<dyn Fn() -> Box<dyn Widget>>;

/// Um painel do [`DockHost`].
///
/// ```
/// use tomni_ui_core::{DockMode, DockPane, DockSide, Symbol, Text};
///
/// let painel = DockPane::new("conexoes", Symbol::Server, "Conexões", || Text::new("lista"))
///     .side(DockSide::Left)
///     .mode(DockMode::Pinned)
///     .size(340.0)
///     .min_size(160.0);
/// # let _ = painel;
/// ```
///
/// A `key` é a chave no [`DockLayout`] e a base dos ids dos widgets gerados. O ícone vale para
/// o botão da faixa. O conteúdo é um construtor que pode rodar de novo a cada troca de lugar do
/// painel: guarde o estado em signals de fora.
pub struct DockPane {
    key: String,
    icon: Symbol,
    title: Prop<String>,
    content: Content,
    side: DockSide,
    mode: DockMode,
    size: f64,
    min_size: f64,
    weight: f64,
    closable: bool,
    floatable: bool,
    actions: Option<Content>,
}

impl DockPane {
    /// Um painel `key`, com o ícone `icon` na faixa, o título `title` (fixo, closure ou
    /// signal) e o conteúdo montado por `content`.
    pub fn new<M, V: View>(
        key: impl Into<String>,
        icon: Symbol,
        title: impl IntoProp<String, M>,
        content: impl Fn() -> V + 'static,
    ) -> Self {
        Self {
            key: key.into(),
            icon,
            title: title.into_prop(),
            content: Rc::new(move || Box::new(content())),
            side: DockSide::Left,
            mode: DockMode::Pinned,
            size: DOCK_PANE_SIZE,
            min_size: 120.0,
            weight: 1.0,
            closable: true,
            floatable: true,
            actions: None,
        }
    }

    /// O lado em que o painel começa (padrão: [`DockSide::Left`]).
    pub fn side(mut self, side: DockSide) -> Self {
        self.side = side;
        self
    }

    /// O modo em que o painel começa (padrão: [`DockMode::Pinned`]).
    pub fn mode(mut self, mode: DockMode) -> Self {
        self.mode = mode;
        self
    }

    /// O tamanho padrão da área fixada e do painel aberto por cima, em px (padrão: 320).
    pub fn size(mut self, size: f64) -> Self {
        self.size = size.max(0.0);
        self
    }

    /// O menor tamanho que o divisor aceita, em px (padrão: 120).
    pub fn min_size(mut self, min_size: f64) -> Self {
        self.min_size = min_size.max(0.0);
        self
    }

    /// O peso do painel entre os fixados do mesmo lado (padrão: 1).
    pub fn weight(mut self, weight: f64) -> Self {
        self.weight = weight.max(0.0);
        self
    }

    /// Se o painel tem o botão e o item de menu "Fechar" (padrão: sim).
    pub fn closable(mut self, closable: bool) -> Self {
        self.closable = closable;
        self
    }

    /// Se o painel tem o botão e o item de menu "Flutuar" (padrão: sim).
    pub fn floatable(mut self, floatable: bool) -> Self {
        self.floatable = floatable;
        self
    }

    /// Botões do app no cabeçalho, antes dos da lib. Monte botões de 32×32.
    pub fn header_actions<V: View>(mut self, actions: impl Fn() -> V + 'static) -> Self {
        self.actions = Some(Rc::new(move || Box::new(actions())));
        self
    }

    fn into_spec(self) -> PaneSpec {
        PaneSpec {
            key: self.key,
            icon: self.icon,
            title: self.title,
            content: self.content,
            side: self.side,
            mode: self.mode,
            size: if self.size > 0.0 {
                self.size
            } else {
                DOCK_PANE_SIZE
            },
            min_size: self.min_size,
            weight: if self.weight > 0.0 { self.weight } else { 1.0 },
            closable: self.closable,
            floatable: self.floatable,
            actions: self.actions,
        }
    }
}

impl fmt::Debug for DockPane {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DockPane")
            .field("key", &self.key)
            .field("side", &self.side)
            .field("mode", &self.mode)
            .finish_non_exhaustive()
    }
}

/// Controla o [`DockHost`] por fora: menus, atalhos e botões do app. Clonável; as cópias falam
/// com o mesmo `DockHost`. Antes da montagem, ou com uma chave desconhecida, os métodos não
/// fazem nada e devolvem `false`.
///
/// | Método | Faz |
/// |---|---|
/// | [`show`](Self::show) | Fechado volta ao modo de antes; auto-ocultar abre por cima; flutuando dá foco à janela; o foco vai para o primeiro item focável do painel |
/// | [`hide`](Self::hide) | recolhe o painel aberto por cima; nos outros modos, nada |
/// | [`toggle`](Self::toggle) | fechado ou recolhido: `show`; aberto: `close` |
/// | [`pin`](Self::pin) / [`unpin`](Self::unpin) | fixado ↔ auto-ocultar |
/// | [`float`](Self::float) / [`dock`](Self::dock) | para a janela própria e de volta |
/// | [`close`](Self::close) | fechado (a janela flutuante fecha junto) |
/// | [`move_to`](Self::move_to) | troca o lado, no fim da ordem do lado novo |
/// | [`maximize`](Self::maximize) / [`restore`](Self::restore) | maximiza o painel / desfaz |
/// | [`mode`](Self::mode), [`is_open`](Self::is_open) | reativos |
/// | [`shown`](Self::shown) | um `Signal<bool>` que acompanha "não está fechado", e que abre e fecha quando se muda |
#[derive(Clone, Default)]
pub struct DockController {
    inner: Rc<RefCell<ControllerState>>,
}

#[derive(Default)]
struct ControllerState {
    shared: Option<Rc<Shared>>,
    shown: HashMap<String, Signal<bool>>,
}

impl DockController {
    /// Um controlador solto; o [`DockHost::controller`] o liga.
    pub fn new() -> Self {
        Self::default()
    }

    fn bind(&self, shared: &Rc<Shared>) {
        self.inner.borrow_mut().shared = Some(Rc::clone(shared));
    }

    /// Liga ao layout os signals de [`DockController::shown`] já criados (na montagem).
    pub(crate) fn wire(&self, shared: &Rc<Shared>) {
        let shown: Vec<(String, Signal<bool>)> = self
            .inner
            .borrow()
            .shown
            .iter()
            .map(|(key, signal)| (key.clone(), *signal))
            .collect();
        for (key, signal) in shown {
            shared.wire_shown(&key, signal);
        }
    }

    fn shared(&self) -> Option<Rc<Shared>> {
        self.inner.borrow().shared.clone()
    }

    fn act(&self, key: &str, f: impl FnOnce(&Rc<Shared>, &str) -> bool) -> bool {
        match self.shared() {
            Some(shared) if shared.spec(key).is_some() => untrack(|| f(&shared, key)),
            _ => false,
        }
    }

    /// Mostra o painel `key` e põe o foco nele.
    pub fn show(&self, key: &str) -> bool {
        self.act(key, |s, k| s.show(k, true))
    }

    /// Recolhe o painel `key`, se estiver aberto por cima.
    pub fn hide(&self, key: &str) -> bool {
        self.act(key, |s, k| s.hide(k))
    }

    /// Fechado ou recolhido: mostra; aberto (fixado, por cima ou flutuando): fecha.
    pub fn toggle(&self, key: &str) -> bool {
        self.act(key, |s, k| s.toggle(k))
    }

    /// Auto-ocultar vira fixado.
    pub fn pin(&self, key: &str) -> bool {
        self.act(key, |s, k| s.pin(k))
    }

    /// Fixado vira auto-ocultar.
    pub fn unpin(&self, key: &str) -> bool {
        self.act(key, |s, k| s.unpin(k))
    }

    /// Passa o painel para uma janela própria.
    pub fn float(&self, key: &str) -> bool {
        self.act(key, |s, k| s.float(k))
    }

    /// Traz o painel flutuante de volta ao lugar.
    pub fn dock(&self, key: &str) -> bool {
        self.act(key, |s, k| s.dock(k))
    }

    /// Fecha o painel.
    pub fn close(&self, key: &str) -> bool {
        self.act(key, |s, k| s.close(k))
    }

    /// Troca o lado do painel, no fim da ordem do lado novo.
    pub fn move_to(&self, key: &str, side: DockSide) -> bool {
        self.act(key, |s, k| s.move_to(k, side))
    }

    /// Maximiza o painel: ele ocupa o `DockHost` inteiro.
    pub fn maximize(&self, key: &str) -> bool {
        self.act(key, |s, k| s.maximize(k))
    }

    /// Desfaz o maximizar.
    pub fn restore(&self) -> bool {
        match self.shared() {
            Some(shared) => untrack(|| shared.restore()),
            None => false,
        }
    }

    /// O modo do painel (reativo).
    pub fn mode(&self, key: &str) -> Option<DockMode> {
        self.shared()?.mode_of(key)
    }

    /// Se o painel está visível agora: fixado, aberto por cima ou flutuando (reativo).
    pub fn is_open(&self, key: &str) -> bool {
        let Some(shared) = self.shared() else {
            return false;
        };
        match shared.mode_of(key) {
            Some(DockMode::Pinned | DockMode::Floating) => true,
            Some(DockMode::AutoHide) => shared.opened.get().as_deref() == Some(key),
            _ => false,
        }
    }

    /// `true` quando o painel não está fechado. O `DockHost` mantém o signal em dia, e um `set`
    /// nele chama [`show`](Self::show) ou [`close`](Self::close): serve ao
    /// [`MenuFlyout::toggle`](crate::MenuFlyout::toggle). Vale também antes da montagem (o
    /// menu costuma ser montado antes do `DockHost`): o signal começa `false` e passa a
    /// acompanhar o layout quando o `DockHost` monta. Uma chave que o `DockHost` não conhece
    /// devolve um signal solto.
    pub fn shown(&self, key: &str) -> Signal<bool> {
        let mut state = self.inner.borrow_mut();
        if let Some(signal) = state.shown.get(key) {
            return *signal;
        }
        let shared = state.shared.clone();
        if shared.as_ref().is_some_and(|s| s.spec(key).is_none()) {
            return signal(false);
        }
        let open = shared
            .as_ref()
            .and_then(|s| s.state(key))
            .is_some_and(|pane| pane.mode != DockMode::Closed);
        let created = signal(open);
        state.shown.insert(key.to_string(), created);
        drop(state);
        if let Some(shared) = shared {
            shared.wire_shown(key, created);
        }
        created
    }
}

impl fmt::Debug for DockController {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DockController")
            .field("bound", &self.inner.borrow().shared.is_some())
            .finish()
    }
}

/// Painéis encaixáveis em volta de um conteúdo.
///
/// ```
/// use tomni_ui_core::{DockHost, DockLayout, DockPane, Symbol, Text};
/// use tomni_ui_reactive::prelude::*;
///
/// # let scope = Scope::root();
/// # scope.run(|| {
/// let layout = signal(DockLayout::default());
/// let tela = DockHost::new(Text::new("Conteúdo"))
///     .id("paineis")
///     .layout(layout)
///     .pane(DockPane::new("arvore", Symbol::Server, "Árvore", || Text::new("itens")));
/// # });
/// # scope.clear();
/// ```
///
/// O `DockHost` cresce (`grow` 1). Ids gerados, com `H` o id do host e `k` a chave do painel:
/// `H-conteudo`, `H-k` (a casca do painel, onde estiver), `H-k-cabecalho`, `H-k-titulo`,
/// `H-k-flutuar`, `H-k-alfinete`, `H-k-fechar`, `H-k-faixa` (o botão na faixa, no
/// auto-ocultar), `H-esquerda-divisor`, `H-direita-divisor` e `H-embaixo-divisor` (a área
/// fixada e o centro), `H-k-divisor` (entre `k` e o próximo da mesma área), `H-k-borda` (o
/// tamanho do painel aberto por cima) e `H-fora` (a área ao lado do painel aberto por cima, que
/// recolhe o painel com um clique).
///
/// Os divisores das áreas respeitam o `min_size` dos painéis e o [`DockHost::min_content`] do
/// centro. Com a janela pequena demais para tudo, o centro encolhe até o mínimo e, depois dele,
/// as áreas encolhem por igual: nenhum painel some sozinho.
pub struct DockHost {
    common: Common,
    layout: Signal<DockLayout>,
    min_content: f64,
    panes: Vec<Rc<PaneSpec>>,
    controller: Option<DockController>,
    content: Option<Box<dyn Widget>>,
}

impl DockHost {
    /// O `conteudo` é o que fica no centro, entre os painéis.
    pub fn new(content: impl View) -> Self {
        Self {
            common: Common {
                id: Some(shared::next_id()),
                grow: 1.0,
                ..Common::default()
            },
            layout: signal(DockLayout::default()),
            min_content: 200.0,
            panes: Vec::new(),
            controller: None,
            content: Some(Box::new(content)),
        }
    }

    /// Id estável, usado em testes, no MCP e no lint, e base dos ids dos painéis.
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.common.id = Some(id.into());
        self
    }

    /// O signal com o estado dos painéis. O `DockHost` preenche o que falta na montagem e o
    /// atualiza a cada mudança.
    pub fn layout(mut self, layout: Signal<DockLayout>) -> Self {
        self.layout = layout;
        self
    }

    /// Liga o controlador `controller` a este `DockHost`.
    pub fn controller(mut self, controller: &DockController) -> Self {
        self.controller = Some(controller.clone());
        self
    }

    /// O menor tamanho do conteúdo, em px (padrão: 200).
    pub fn min_content(mut self, px: f64) -> Self {
        self.min_content = px.max(0.0);
        self
    }

    /// Acrescenta um painel.
    pub fn pane(mut self, pane: DockPane) -> Self {
        let spec = pane.into_spec();
        debug_assert!(
            self.panes.iter().all(|p| p.key != spec.key),
            "o `DockHost` já tem um painel com a chave `{}`: as chaves precisam ser únicas",
            spec.key
        );
        self.panes.push(Rc::new(spec));
        self
    }
}

impl Widget for DockHost {
    fn kind(&self) -> &'static str {
        "dock_host"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::Row
    }

    fn flex(&self) -> Option<Flex> {
        Some(Flex {
            direction: Direction::Row,
            ..Flex::default()
        })
    }

    fn names_children(&self) -> bool {
        true
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        let Some(content) = self.content.take() else {
            return Vec::new();
        };
        let id = self.common.id.clone().unwrap_or_default();
        let shared = Rc::new(Shared::new(
            id,
            self.layout,
            self.min_content,
            std::mem::take(&mut self.panes),
            signal(None),
            signal(Size::ZERO),
        ));
        // Antes de qualquer efeito: o layout recebe o padrão dos painéis.
        let mut layout = untrack(|| self.layout.get());
        shared::normalize(&mut layout, &shared.panes);
        if untrack(|| self.layout.get()) != layout {
            self.layout.set(layout);
        }
        if let Some(controller) = &self.controller {
            controller.bind(&shared);
        }
        build::root(&shared, self.controller.clone(), content)
    }
}

impl fmt::Debug for DockHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DockHost")
            .field("common", &self.common)
            .field("panes", &self.panes.len())
            .finish_non_exhaustive()
    }
}
