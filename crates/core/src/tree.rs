//! Árvore montada: dono dos widgets, hit test, captura do ponteiro, foco, rolagem e a fila de
//! widgets que podem ter mudado de visual.

use std::cell::{Ref, RefCell};
use std::fmt;
use std::rc::{Rc, Weak};
use std::time::Duration;

use kurbo::{Point, Rect, Size, Vec2};
use slotmap::{SecondaryMap, SlotMap};
use tomni_ui_reactive::{Scope, effect, untrack};

use crate::cx::{Clipboard, EventCx, FixedMetrics, FocusMove, MemoryClipboard, TextMetrics};
use crate::density::Density;
use crate::drag::{DRAG_THRESHOLD, DragData};
use crate::event::{Cursor, Event, FocusSource, Ime, Key, Modifiers, PointerKind};
use crate::scroll_viewer::{SCROLL_LINE, ScrollMode, Scroller};
use crate::shortcut::Shortcut;
use crate::title_bar::{PaneLink, WindowRequest, WindowState};
use crate::tooltip::ToolTip;
use crate::widget::{Layer, Look, Placement, Slot, Widget, Within};

slotmap::new_key_type! {
    /// Widget montado. O id não muda enquanto o widget estiver na árvore, mesmo que outros
    /// entrem ou saiam; a posição na ordem da árvore é [`Tree::position`].
    pub struct NodeId;
}

/// Folga em volta do widget que recebe o foco pelo teclado, ao rolar até ele: o anel de foco
/// fica 3 px para fora.
const FOCUS_MARGIN: f64 = 4.0;

/// Quantas vezes o loop de frame procura o widget de um [`EventCx::focus_later`] antes de
/// desistir.
const FOCUS_LATER_TRIES: u8 = 8;
/// Quanto o dedo anda antes de o toque virar rolagem.
const PAN_SLOP: f64 = 8.0;
/// Tempo com o ponteiro parado (ou o foco) sobre um widget até a dica aparecer
/// (`ToolTipService.InitialShowDelay`).
pub const TOOLTIP_DELAY: Duration = Duration::from_millis(500);
/// Tempo com o dedo parado até o toque longo abrir o menu de contexto (o `Holding` do WinUI).
pub const HOLD_DELAY: Duration = Duration::from_millis(500);
/// Intervalo entre os frames de uma animação (os indeterminados): uns 60 por segundo.
pub const ANIMATION_FRAME: Duration = Duration::from_millis(16);

type Shared = Rc<RefCell<Box<dyn Widget>>>;

struct Node {
    widget: Shared,
    /// Dono do efeito do widget e dos escopos dos filhos: descartado quando o widget sai da
    /// árvore.
    scope: Scope,
    parent: Option<NodeId>,
    children: Vec<NodeId>,
    /// Retângulo dado pelo layout, sem descontar a rolagem.
    rect: Rect,
    /// Se é uma camada, e como ela se comporta.
    layer: Option<Layer>,
    /// A dica do widget (`.tooltip()`), filha dele.
    tooltip: Option<NodeId>,
    /// O menu de contexto do widget (`.context_flyout()`), filho dele.
    context: Option<NodeId>,
    /// Ponto da janela onde a camada abriu (o menu de contexto aberto pelo botão direito),
    /// no lugar do retângulo da origem.
    anchor: Option<Rect>,
    /// Se anima sozinho ([`Widget::animating`], lido na montagem).
    animates: bool,
    /// Se quer saber quando o ponteiro e o foco entram e saem ([`Widget::watches_within`]).
    watches: bool,
    /// O último estado avisado ao widget: se o ponteiro e se o foco estão dentro dele.
    within: (bool, bool),
    /// Densidade efetiva: a do widget ([`Widget::density`]) ou a herdada do pai.
    density: Density,
}

/// Camada aberta que recebe entrada.
#[derive(Clone, Copy, Debug)]
struct Open {
    id: NodeId,
    /// Quem tinha o foco quando ela abriu.
    restore: Option<NodeId>,
}

/// Como a dica foi chamada.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TipVia {
    /// Ponteiro parado sobre o dono.
    Hover,
    /// Foco vindo do teclado.
    Focus,
}

/// Dica em contagem ou aberta.
#[derive(Clone, Copy, Debug)]
struct Tip {
    /// Widget dono da dica.
    owner: NodeId,
    via: TipVia,
    /// Tempo contado até abrir; `None` depois de um clique ou tecla, até o ponteiro sair.
    elapsed: Option<Duration>,
    open: bool,
}

/// Dedo parado sobre um widget, contando até o toque longo.
#[derive(Clone, Copy, Debug)]
struct Hold {
    origin: Point,
    target: NodeId,
    elapsed: Duration,
}

/// Arrasto de um item ([`Widget::drag_data`]), do botão pressionado até ser solto.
#[derive(Clone, Debug)]
struct Drag {
    source: NodeId,
    data: DragData,
    origin: Point,
    /// Pode começar ao passar do limiar: com o mouse, logo; no toque, depois do toque longo.
    armed: bool,
    /// Passou do limiar: a etiqueta segue o ponteiro e os alvos recebem o item.
    active: bool,
    /// Alvo sob o ponteiro, que aceitou o item.
    target: Option<NodeId>,
}

/// Toque que pode virar rolagem.
#[derive(Clone, Copy, Debug)]
struct Pan {
    origin: Point,
    last: Point,
    active: bool,
}

/// O que mudou na estrutura da árvore num [`Tree::reconcile`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    /// Widgets criados, com as subárvores inteiras; o pai vem antes dos filhos.
    pub added: Vec<NodeId>,
    /// Widgets descartados, com as subárvores inteiras.
    pub removed: Vec<NodeId>,
    /// Fragmentos (`Show`, `For`) cujos filhos mudaram.
    pub parents: Vec<NodeId>,
}

impl Changes {
    /// Se nada mudou.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.parents.is_empty()
    }
}

/// Widgets montados a partir da view.
///
/// Cada widget ganha um escopo reativo, filho do escopo do pai, e um efeito que lê o
/// [`Look`]: quando uma propriedade reativa usada nele muda, o widget entra na fila de
/// [`Tree::take_dirty`]. Os eventos também põem na fila o widget que os recebeu. Nos
/// fragmentos (`Show`, `For`), o efeito também lê os dados dos filhos, e o
/// [`Tree::reconcile`] cria e descarta só os widgets que entraram e saíram.
///
/// A árvore guarda o que os widgets usam ao tratar eventos: as medidas de texto (o `Ui`
/// instala as do `Drawer`; o padrão é [`FixedMetrics`]), o clipboard ([`MemoryClipboard`] até
/// a plataforma ligar o do sistema), a última posição do ponteiro e os modificadores.
pub struct Tree {
    nodes: SlotMap<NodeId, Node>,
    root: NodeId,
    /// Ordem da árvore (pai antes dos filhos), que também é a de pintura e de Tab.
    order: Vec<NodeId>,
    position: SecondaryMap<NodeId, usize>,
    scope: Scope,
    dirty: Rc<RefCell<Vec<NodeId>>>,
    /// Fragmentos cujos dados mudaram.
    structure: Rc<RefCell<Vec<NodeId>>>,
    focus: Option<NodeId>,
    hover: Option<NodeId>,
    capture: Option<NodeId>,
    /// Onde o botão do meio desceu.
    middle: Option<NodeId>,
    /// Quem usou o último [`Event::SecondaryDown`]: recebe o [`Event::SecondaryUp`].
    secondary: Option<NodeId>,
    pan: Option<Pan>,
    /// Camadas abertas que recebem entrada, de baixo para cima.
    layers: Vec<Open>,
    /// Camadas abertas que só recebem o ponteiro ([`Layer::pointer`]), na ordem da árvore.
    passive: Vec<NodeId>,
    /// Sobreposições abertas ([`Placement::Over`] sem entrada): pintadas por cima do conteúdo
    /// e fora do ponteiro, do foco e do teclado, na ordem da árvore.
    overlays: Vec<NodeId>,
    /// Widgets que animam, na ordem da árvore.
    animators: Vec<NodeId>,
    /// Widgets que querem saber onde estão o ponteiro e o foco ([`Widget::watches_within`]).
    watchers: Vec<NodeId>,
    /// `ScrollViewer`s comandados por fora ([`Widget::scroll_link`]).
    linked: Vec<NodeId>,
    /// Widgets que medem a si mesmos depois do layout ([`Widget::places`]).
    placers: Vec<NodeId>,
    /// Widgets com dica, na ordem da árvore.
    tip_owners: Vec<NodeId>,
    /// Widgets com atalhos (`.accelerator()`), na ordem da árvore.
    accelerator_owners: Vec<NodeId>,
    tip: Option<Tip>,
    /// De onde veio a última entrada: o foco que uma camada põe ao abrir ou fechar segue ela.
    last_source: FocusSource,
    /// Widget focado por uma camada que abriu, para rolar até ele depois do layout.
    pending_reveal: Option<NodeId>,
    /// Id do widget que um evento pediu para focar ([`EventCx::focus_later`]) e quantas
    /// tentativas faltam.
    pending_focus: Option<(String, u8)>,
    /// Como `pending_focus`, mas para a primeira parada de Tab dentro do widget
    /// ([`crate::request_focus_inside`]).
    pending_focus_inside: Option<(String, u8)>,
    /// Id do widget que o app pediu para mostrar sem focar ([`crate::start_bring_into_view`]).
    pending_bring: Option<(String, u8)>,
    pointer: Point,
    pointer_kind: Option<PointerKind>,
    modifiers: Modifiers,
    metrics: Box<dyn TextMetrics>,
    clipboard: Box<dyn Clipboard>,
    /// Estado da janela da `TitleBar`, se a view tem uma.
    window: Option<WindowState>,
    /// Painel do primeiro `NavigationView`, para o ☰ da `TitleBar`.
    pane: Option<PaneLink>,
    /// Pedidos à janela feitos nos eventos, na ordem.
    window_requests: Vec<WindowRequest>,
    /// Se o Alt foi pressionado sem outra tecla depois: solto assim, põe o foco na barra de
    /// menus.
    alt_alone: bool,
    /// Quem tinha o foco antes de o F10 ou o Alt porem o foco na barra de menus.
    menu_restore: Option<NodeId>,
    /// Um atalho usou a última tecla: o texto que vem junto dela não é digitado.
    swallow_text: bool,
    /// Toque parado, contando até o toque longo.
    hold: Option<Hold>,
    /// A camada está fechando pelo Esc.
    escaping: bool,
    /// Arrasto em andamento, ou o botão pressionado sobre um widget que arrasta.
    drag: Option<Drag>,
    /// Alvo sob o ponteiro de um arrasto que veio de outra janela ([`Tree::foreign_drag_over`]).
    foreign_target: Option<NodeId>,
}

impl Tree {
    /// Monta a árvore de `root`. Os escopos dos widgets são filhos de `scope`.
    ///
    /// Os efeitos rodam no próximo `run_effects`, e todos os widgets começam na fila.
    pub fn mount(root: Box<dyn Widget>, scope: &Scope) -> Self {
        let mut tree = Self {
            nodes: SlotMap::with_key(),
            root: NodeId::default(),
            order: Vec::new(),
            position: SecondaryMap::new(),
            scope: scope.child(),
            dirty: Rc::default(),
            structure: Rc::default(),
            focus: None,
            hover: None,
            capture: None,
            middle: None,
            secondary: None,
            pan: None,
            layers: Vec::new(),
            passive: Vec::new(),
            overlays: Vec::new(),
            animators: Vec::new(),
            watchers: Vec::new(),
            linked: Vec::new(),
            placers: Vec::new(),
            tip_owners: Vec::new(),
            accelerator_owners: Vec::new(),
            tip: None,
            last_source: FocusSource::Keyboard,
            pending_reveal: None,
            pending_focus: None,
            pending_focus_inside: None,
            pending_bring: None,
            pointer: Point::ZERO,
            pointer_kind: None,
            modifiers: Modifiers::NONE,
            metrics: Box::new(FixedMetrics::default()),
            clipboard: Box::new(MemoryClipboard::default()),
            window: None,
            pane: None,
            window_requests: Vec::new(),
            alt_alone: false,
            menu_restore: None,
            swallow_text: false,
            hold: None,
            escaping: false,
            drag: None,
            foreign_target: None,
        };
        let scope = tree.scope.child();
        tree.root = tree.add(root, None, scope, &mut Vec::new());
        tree.reorder();
        tree
    }

    /// Cria o nó de `widget` no escopo `scope` (só dele) e, recursivamente, os filhos. Não
    /// liga o nó ao pai: quem chama põe o id na lista de filhos.
    fn add(
        &mut self,
        mut widget: Box<dyn Widget>,
        parent: Option<NodeId>,
        scope: Scope,
        added: &mut Vec<NodeId>,
    ) -> NodeId {
        let inherited = parent.map_or(Density::Standard, |p| self.nodes[p].density);
        let density = widget.density().unwrap_or(inherited);
        widget.set_density(density);
        let children = widget.take_children();
        let fragment = widget.fragment();
        let layer = widget.layer();
        let animates = untrack(|| widget.animating());
        let watches = widget.watches_within();
        let tip = (!fragment)
            .then(|| widget.common().tooltip.clone())
            .flatten()
            .map(|text| (text, widget.common().id.clone()));
        let context = (!fragment)
            .then(|| {
                widget
                    .common()
                    .context_flyout
                    .as_ref()
                    .and_then(|c| c.take())
            })
            .flatten();
        if let Some(state) = widget.window_state() {
            self.window = Some(state);
            if let Some(link) = self.pane {
                state.link_pane(link);
            }
        }
        if let Some(link) = widget.pane_link()
            && self.pane.is_none()
        {
            self.pane = Some(link);
            if let Some(state) = self.window {
                state.link_pane(link);
            }
        }
        let widget: Shared = Rc::new(RefCell::new(widget));
        let id = self.nodes.insert(Node {
            widget: Rc::clone(&widget),
            scope: scope.clone(),
            parent,
            children: Vec::new(),
            rect: Rect::ZERO,
            layer,
            tooltip: None,
            context: None,
            anchor: None,
            animates,
            watches,
            within: (false, false),
            density,
        });
        added.push(id);

        let weak: Weak<RefCell<Box<dyn Widget>>> = Rc::downgrade(&widget);
        let dirty = Rc::clone(&self.dirty);
        let structure = fragment.then(|| Rc::clone(&self.structure));
        scope.run(|| {
            effect(move || {
                if let Some(widget) = weak.upgrade() {
                    // Só para registrar as dependências: o `Look`, a visibilidade e os filhos
                    // são lidos de novo no frame.
                    let widget = widget.borrow();
                    let _ = widget.look();
                    let _ = widget.visible();
                    let _ = widget.dynamic_width();
                    let _ = widget.dynamic_basis();
                    let _ = widget.dynamic_grow();
                    let _ = widget.opacity();
                    if let Some(structure) = &structure {
                        widget.track_children();
                        structure.borrow_mut().push(id);
                    }
                    dirty.borrow_mut().push(id);
                }
            });
        });
        for child in children {
            let child = self.add(child, Some(id), scope.child(), added);
            self.nodes[id].children.push(child);
        }
        if let Some((text, owner)) = tip {
            let child_scope = scope.child();
            let tooltip = child_scope.run(|| ToolTip::new(text, owner.as_deref()));
            let child = self.add(Box::new(tooltip), Some(id), child_scope, added);
            self.nodes[id].children.push(child);
            self.nodes[id].tooltip = Some(child);
        }
        if let Some(menu) = context {
            let child = self.add(menu, Some(id), scope.child(), added);
            self.nodes[id].children.push(child);
            self.nodes[id].context = Some(child);
        }
        if fragment {
            let slots = untrack(|| widget.borrow_mut().reconcile()).unwrap_or_default();
            self.apply(id, slots, added, &mut Vec::new());
        }
        id
    }

    /// Troca os filhos do fragmento `fragment` pela lista `slots`.
    fn apply(
        &mut self,
        fragment: NodeId,
        slots: Vec<Slot>,
        added: &mut Vec<NodeId>,
        removed: &mut Vec<NodeId>,
    ) {
        let old = std::mem::take(&mut self.nodes[fragment].children);
        let mut kept = vec![false; old.len()];
        let scope = self.nodes[fragment].scope.clone();
        let mut children = Vec::with_capacity(slots.len());
        for slot in slots {
            match slot {
                Slot::Keep(i) if i < old.len() && !kept[i] => {
                    kept[i] = true;
                    children.push(old[i]);
                }
                Slot::Keep(i) => debug_assert!(false, "Slot::Keep({i}) inválido"),
                Slot::New(make) => {
                    // Signals e efeitos criados pela view ficam no escopo do item.
                    let item = scope.child();
                    let widget = item.run(|| untrack(make));
                    children.push(self.add(widget, Some(fragment), item, added));
                }
            }
        }
        for (child, kept) in old.into_iter().zip(kept) {
            if !kept {
                self.remove(child, removed);
            }
        }
        self.nodes[fragment].children = children;
    }

    /// Descarta `id` e a subárvore: widgets, efeitos e o que foi criado nos escopos deles. Quem
    /// sai não recebe evento (nem `FocusOut`).
    fn remove(&mut self, id: NodeId, removed: &mut Vec<NodeId>) {
        let Some(node) = self.nodes.remove(id) else {
            return;
        };
        removed.push(id);
        for child in node.children {
            self.remove(child, removed);
        }
        node.scope.clear();
        if self.hold.is_some_and(|hold| hold.target == id) {
            self.hold = None;
        }
        if let Some(drag) = &mut self.drag {
            if drag.target == Some(id) {
                drag.target = None;
            }
            // Um arrasto em andamento segue sem a origem (a linha de um `TreeView` pode sair da
            // tela ou mudar de nível no meio do arrasto); antes do limiar, acaba.
            if drag.source == id && !drag.active {
                self.end_drag();
            }
        }
        for slot in [
            &mut self.focus,
            &mut self.hover,
            &mut self.capture,
            &mut self.menu_restore,
        ] {
            if *slot == Some(id) {
                *slot = None;
            }
        }
    }

    /// Refaz a ordem da árvore.
    fn reorder(&mut self) {
        self.order.clear();
        self.position.clear();
        self.tip_owners.clear();
        self.accelerator_owners.clear();
        self.animators.clear();
        self.watchers.clear();
        self.linked.clear();
        self.placers.clear();
        let mut stack = vec![self.root];
        while let Some(id) = stack.pop() {
            self.position.insert(id, self.order.len());
            self.order.push(id);
            if self.nodes[id].tooltip.is_some() {
                self.tip_owners.push(id);
            }
            if !self.nodes[id]
                .widget
                .borrow()
                .common()
                .accelerators
                .is_empty()
            {
                self.accelerator_owners.push(id);
            }
            if self.nodes[id].animates {
                self.animators.push(id);
            }
            if self.nodes[id].watches {
                self.watchers.push(id);
            }
            if self.nodes[id].widget.borrow().scroll_link().is_some() {
                self.linked.push(id);
            }
            if self.nodes[id].widget.borrow().places() {
                self.placers.push(id);
            }
            stack.extend(self.nodes[id].children.iter().rev());
        }
    }

    /// Atualiza os filhos dos fragmentos (`Show`, `For`) cujos dados mudaram. O loop de frame
    /// chama depois de rodar os efeitos.
    ///
    /// Itens novos ganham widgets novos, que entram na fila no próximo `run_effects`; os que
    /// saíram são descartados com os efeitos; os que ficaram mantêm o widget e o estado.
    pub fn reconcile(&mut self) -> Changes {
        let mut changes = Changes::default();
        let mut pending = std::mem::take(&mut *self.structure.borrow_mut());
        pending.retain(|&id| self.position.contains_key(id));
        if pending.is_empty() {
            return changes;
        }
        pending.sort_unstable_by_key(|&id| self.position[id]);
        pending.dedup();
        for fragment in pending {
            // Um fragmento de dentro de outro pode ter saído junto com o de fora.
            let Some(node) = self.nodes.get(fragment) else {
                continue;
            };
            let widget = Rc::clone(&node.widget);
            let slots = untrack(|| widget.borrow_mut().reconcile());
            if let Some(slots) = slots {
                self.apply(fragment, slots, &mut changes.added, &mut changes.removed);
                changes.parents.push(fragment);
            }
            self.grant_focus_request(fragment);
        }
        self.reorder();
        for &id in &changes.added {
            let widget = self.widget(id);
            if untrack(|| widget.focus_on_mount())
                && let Some(name) = widget.common().id.clone()
            {
                drop(widget);
                self.pending_focus = Some((name, FOCUS_LATER_TRIES));
            }
        }
        changes
    }

    /// Raiz.
    pub fn root(&self) -> NodeId {
        self.root
    }

    /// Número de widgets.
    pub fn len(&self) -> usize {
        self.order.len()
    }

    /// Se a árvore está vazia (nunca está: sempre há a raiz).
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// Todos os widgets, na ordem da árvore: pai antes dos filhos, irmãos da esquerda para a
    /// direita. É a ordem de pintura e de Tab.
    pub fn ids(&self) -> impl DoubleEndedIterator<Item = NodeId> + ExactSizeIterator + '_ {
        self.order.iter().copied()
    }

    /// Se `id` ainda está na árvore.
    pub fn contains(&self, id: NodeId) -> bool {
        self.position.contains_key(id)
    }

    /// Posição de `id` na ordem da árvore ([`Tree::ids`]).
    pub fn position(&self, id: NodeId) -> usize {
        self.position[id]
    }

    /// Filhos de `id`, em ordem.
    pub fn children(&self, id: NodeId) -> &[NodeId] {
        &self.nodes[id].children
    }

    /// Pai de `id`.
    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.nodes[id].parent
    }

    /// O widget em `id`.
    ///
    /// # Panics
    ///
    /// Se o widget estiver tratando um evento (chamada de dentro de um `on_click`, por exemplo).
    pub fn widget(&self, id: NodeId) -> Ref<'_, dyn Widget> {
        Ref::map(self.nodes[id].widget.borrow(), |w| &**w)
    }

    /// Muda o widget em `id` de fora de um evento (o `feed` do `TestApp`, que põe bytes na
    /// grade do `Terminal`), sem registrar dependências, e põe o widget na fila de repintura.
    ///
    /// # Panics
    ///
    /// Se o widget estiver tratando um evento.
    pub fn with_widget_mut<R>(&mut self, id: NodeId, f: impl FnOnce(&mut dyn Widget) -> R) -> R {
        let widget = Rc::clone(&self.nodes[id].widget);
        let result = untrack(|| f(&mut **widget.borrow_mut()));
        self.dirty.borrow_mut().push(id);
        result
    }

    /// Visual atual de `id`, lido sem registrar dependências.
    pub fn look(&self, id: NodeId) -> Look {
        untrack(|| self.widget(id).look())
    }

    /// Id do widget ou, sem id, o tipo (`"column"`).
    pub fn name(&self, id: NodeId) -> String {
        let widget = self.widget(id);
        match &widget.common().id {
            Some(name) => name.clone(),
            None => widget.kind().to_owned(),
        }
    }

    /// Procura o widget com o id `name`.
    pub fn find(&self, name: &str) -> Option<NodeId> {
        self.ids()
            .find(|&id| self.widget(id).common().id.as_deref() == Some(name))
    }

    /// Retângulo de `id` na janela: o do layout menos a rolagem dos `ScrollViewer` em volta.
    /// Pode ficar fora da área visível deles ([`Tree::clip`]).
    pub fn rect(&self, id: NodeId) -> Rect {
        self.nodes[id].rect - self.scrolled(id)
    }

    /// Retângulo de `id` dado pelo layout, sem descontar a rolagem: onde o widget estaria com
    /// todos os `ScrollViewer` no começo.
    pub fn layout_rect(&self, id: NodeId) -> Rect {
        self.nodes[id].rect
    }

    /// Soma da rolagem dos `ScrollViewer` em volta de `id`.
    fn scrolled(&self, id: NodeId) -> Vec2 {
        let mut offset = Vec2::ZERO;
        let mut node = self.up(id);
        while let Some(parent) = node {
            if let Some(scroller) = self.scroller(parent) {
                offset += scroller.offset();
            }
            node = self.up(parent);
        }
        offset
    }

    /// Pai de `id`, a não ser que `id` seja uma camada: ela tem posição própria, e a rolagem e
    /// o clip de fora não valem dentro dela.
    /// Se `id` é `ancestor` ou está dentro dele (as camadas contam como filhas da origem).
    fn is_within(&self, id: NodeId, ancestor: NodeId) -> bool {
        std::iter::successors(Some(id), |&id| self.parent(id)).any(|id| id == ancestor)
    }

    fn up(&self, id: NodeId) -> Option<NodeId> {
        if self.nodes[id].layer.is_some() {
            None
        } else {
            self.parent(id)
        }
    }

    /// Área da janela onde `id` pode aparecer: a interseção das áreas visíveis dos
    /// `ScrollViewer` em volta dele (a janela inteira, fora de um `ScrollViewer`).
    pub fn clip(&self, id: NodeId) -> Rect {
        let mut clip = Rect::new(
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
            f64::INFINITY,
            f64::INFINITY,
        );
        let mut node = self.up(id);
        while let Some(parent) = node {
            if self.scroller(parent).is_some() {
                clip = clip.intersect(self.rect(parent));
            }
            node = self.up(parent);
        }
        clip
    }

    /// Parte de `id` que aparece: o retângulo dele cortado pelos `ScrollViewer` em volta.
    /// Vazio se estiver todo fora.
    pub fn visible_rect(&self, id: NodeId) -> Rect {
        self.rect(id).intersect(self.clip(id))
    }

    /// Densidade efetiva de `id`: a do contêiner mais próximo com `.density()`, ou a padrão.
    pub fn density(&self, id: NodeId) -> Density {
        self.nodes[id].density
    }

    /// Parte do retângulo de `id` que recebe o ponteiro (o botão de fechar de uma `InfoBar`).
    pub fn hit_rect(&self, id: NodeId) -> Rect {
        self.widget(id).hit_rect(self.rect(id))
    }

    /// Se o ponteiro em `point` está sobre `id` e numa parte visível dele.
    fn inside(&self, id: NodeId, point: Point) -> bool {
        self.widget(id).hits(self.rect(id), point) && self.clip(id).contains(point)
    }

    /// Se `id` aparece: ele e todos os ancestrais estão visíveis (`.visible()`). Lido sem
    /// registrar dependências. Um widget rolado para fora da área visível continua aparecendo.
    pub fn shown(&self, id: NodeId) -> bool {
        let mut node = Some(id);
        while let Some(id) = node {
            if !self.visible(id) {
                return false;
            }
            node = self.parent(id);
        }
        true
    }

    /// Se `id` está visível pelo `.visible()` dele, sem olhar os ancestrais. Lido sem
    /// registrar dependências.
    pub fn visible(&self, id: NodeId) -> bool {
        untrack(|| self.widget(id).visible())
    }

    /// Se `id` aparece e pode receber o foco agora: com uma camada aberta, só os widgets de
    /// dentro dela.
    fn can_focus(&self, id: NodeId) -> bool {
        self.widget(id).focusable() && self.shown(id) && self.in_scope(id)
    }

    /// Se o Tab para em `id` ([`Widget::tab_stop`]). Lido sem registrar dependências.
    fn tab_stop(&self, id: NodeId) -> bool {
        untrack(|| self.widget(id).tab_stop())
    }

    /// Se `id` está na camada de cima (ou, sem camada aberta, fora de qualquer camada): só
    /// ela recebe o teclado e o ponteiro.
    pub fn in_scope(&self, id: NodeId) -> bool {
        self.input_layer_of(id) == self.layers.last().map(|open| open.id)
    }

    /// Camada que recebe entrada mais próxima de `id`, contando o próprio `id`.
    fn input_layer_of(&self, id: NodeId) -> Option<NodeId> {
        let mut node = Some(id);
        while let Some(id) = node {
            if self.nodes[id].layer.is_some_and(|layer| layer.input) {
                return Some(id);
            }
            node = self.parent(id);
        }
        None
    }

    /// Ponto da janela onde a camada `id` abriu (o menu de contexto aberto pelo botão
    /// direito). `None` é junto da origem.
    pub fn anchor(&self, id: NodeId) -> Option<Point> {
        self.nodes[id].anchor.map(|rect| rect.origin())
    }

    /// O retângulo de origem de uma camada que não fica junto do widget: o ponto do clique
    /// direito (sem tamanho) ou o que [`Widget::tip_anchor`] pediu.
    pub fn anchor_rect(&self, id: NodeId) -> Option<Rect> {
        self.nodes[id].anchor
    }

    /// Menu de contexto de `id` (`.context_flyout()`).
    pub fn context_flyout(&self, id: NodeId) -> Option<NodeId> {
        self.nodes[id].context
    }

    /// Comportamento de `id`, se for uma camada ([`Widget::layer`]).
    pub fn layer(&self, id: NodeId) -> Option<Layer> {
        self.nodes[id].layer
    }

    /// Camada mais próxima de `id`, contando o próprio `id`: o popup em que ele está.
    pub fn layer_of(&self, id: NodeId) -> Option<NodeId> {
        let mut node = Some(id);
        while let Some(id) = node {
            if self.nodes[id].layer.is_some() {
                return Some(id);
            }
            node = self.parent(id);
        }
        None
    }

    /// Camadas abertas, na ordem de pintura: as sobreposições ([`Placement::Over`]) primeiro,
    /// por cima do conteúdo e abaixo de tudo o mais; depois as que recebem entrada, de baixo
    /// para cima, cada uma seguida das que só recebem o ponteiro de dentro dela (a lista de um
    /// `AutoSuggestBox`), e a dica por último.
    pub fn open_layers(&self) -> Vec<NodeId> {
        let passive_in = |owner: Option<NodeId>| {
            self.passive
                .iter()
                .copied()
                .filter(move |&p| self.parent(p).and_then(|o| self.input_layer_of(o)) == owner)
        };
        let mut layers: Vec<NodeId> = self.overlays.clone();
        layers.extend(passive_in(None));
        for open in &self.layers {
            layers.push(open.id);
            layers.extend(passive_in(Some(open.id)));
        }
        if let Some(tip) = self.tip.filter(|tip| tip.open)
            && let Some(tooltip) = self.nodes.get(tip.owner).and_then(|n| n.tooltip)
        {
            layers.push(tooltip);
        }
        layers
    }

    /// Fecha a camada `id` (Esc, clique fora).
    pub fn close_layer(&mut self, id: NodeId) {
        let widget = Rc::clone(&self.nodes[id].widget);
        untrack(|| widget.borrow_mut().set_open(false));
        self.dirty.borrow_mut().push(id);
        self.sync_layers();
    }

    /// Muda o retângulo de layout de `id`. Devolve `true` se mudou. Se o tamanho mudou, o
    /// widget fica sabendo ([`Widget::resized`]).
    pub fn set_rect(&mut self, id: NodeId, rect: Rect) -> bool {
        let old = std::mem::replace(&mut self.nodes[id].rect, rect);
        if old.size() != rect.size() {
            let widget = Rc::clone(&self.nodes[id].widget);
            untrack(|| widget.borrow_mut().resized(rect.size()));
        }
        old != rect
    }

    /// Widgets que podem ter mudado de visual desde a última chamada, sem repetição e na ordem
    /// da árvore.
    pub fn take_dirty(&mut self) -> Vec<NodeId> {
        let mut dirty = std::mem::take(&mut *self.dirty.borrow_mut());
        dirty.retain(|&id| self.position.contains_key(id));
        dirty.sort_unstable_by_key(|&id| self.position[id]);
        dirty.dedup();
        dirty
    }

    /// Widget com o foco.
    pub fn focused(&self) -> Option<NodeId> {
        self.focus
    }

    /// Forma do ponteiro do mouse em `point`: a do widget que segura o ponteiro (um divisor
    /// sendo arrastado) ou, sem ele, a do que está embaixo.
    pub fn cursor(&self, point: Point) -> Cursor {
        self.capture
            .or_else(|| self.hit(point))
            .and_then(|id| untrack(|| self.widget(id).cursor()))
            .unwrap_or_default()
    }

    /// Widget interativo mais profundo sob `point`, mesmo desabilitado: um botão desabilitado
    /// não repassa o clique para quem está atrás dele. Widgets ocultos e a parte do conteúdo de
    /// um `ScrollViewer` fora da área visível dele não contam; a barra de rolagem fica por cima
    /// do conteúdo. Com uma camada aberta, só conta o que está dentro dela.
    pub fn hit(&self, point: Point) -> Option<NodeId> {
        match self.passive_at(point) {
            Some(passive) => self.hit_in(passive, point),
            None => self.hit_in(self.input_root_at(point), point),
        }
    }

    /// Onde começa a busca por ponteiro em `point`: a camada de cima ou, fora dela, o que
    /// está embaixo enquanto as camadas deixam o ponteiro passar ([`Layer::pass_through`]):
    /// o menu de onde saiu o submenu e a barra de menus do dono. Sem camada, a raiz.
    fn input_root_at(&self, point: Point) -> NodeId {
        for i in (0..self.layers.len()).rev() {
            let layer = self.layers[i].id;
            let passes = self.nodes[layer].layer.is_some_and(|l| l.pass_through);
            if !passes || self.rect(layer).contains(point) {
                return layer;
            }
            let below = i.checked_sub(1).map(|b| self.layers[b].id);
            if let Some(bar) = self.menu_bar_of(layer)
                && self.input_layer_of(bar) == below
                && self.rect(bar).contains(point)
                && self.clip(bar).contains(point)
            {
                return bar;
            }
            match below {
                Some(below) if self.is_within(layer, below) => {}
                _ => return layer,
            }
        }
        self.root
    }

    /// Barra de menus em volta de `id` ([`Widget::menu_bar`]), se houver.
    fn menu_bar_of(&self, id: NodeId) -> Option<NodeId> {
        std::iter::successors(self.parent(id), |&p| self.parent(p))
            .find(|&p| untrack(|| self.widget(p).menu_bar()))
    }

    /// Camada que só recebe o ponteiro sob `point`, das que estão em cima.
    fn passive_at(&self, point: Point) -> Option<NodeId> {
        self.passive
            .iter()
            .rev()
            .copied()
            .find(|&id| self.contains(id) && self.in_scope(id) && self.rect(id).contains(point))
    }

    fn hit_in(&self, id: NodeId, point: Point) -> Option<NodeId> {
        if !self.visible(id) {
            return None;
        }
        if self.nodes[id].layer.is_some() && !self.rect(id).contains(point) {
            return None;
        }
        let rect = self.rect(id);
        let (hits, scrolls) = {
            let widget = self.widget(id);
            (
                widget.interactive() && widget.hits(rect, point),
                widget.scroller().is_some(),
            )
        };
        if scrolls {
            if !rect.contains(point) {
                return None;
            }
            if hits {
                return Some(id);
            }
        }
        if let Some(hit) = self
            .stacked(id)
            .into_iter()
            .rev()
            .find_map(|child| self.hit_in(child, point))
        {
            return Some(hit);
        }
        (hits && !scrolls).then_some(id)
    }

    /// Filhos de `id` que não são camadas, na ordem de pintura: os que ficam por cima dos
    /// irmãos ([`Widget::raised`]) no fim.
    pub fn stacked(&self, id: NodeId) -> Vec<NodeId> {
        let (mut flat, raised): (Vec<NodeId>, Vec<NodeId>) = self
            .children(id)
            .iter()
            .copied()
            .filter(|&child| self.nodes[child].layer.is_none())
            .partition(|&child| !self.widget(child).raised());
        flat.extend(raised);
        flat
    }

    /// `ScrollViewer` visíveis sob `point`, do mais externo para o mais interno.
    pub fn scrollers_at(&self, point: Point) -> Vec<NodeId> {
        let mut found = Vec::new();
        let start = self
            .passive_at(point)
            .unwrap_or_else(|| self.layers.last().map_or(self.root, |open| open.id));
        if self.visible(start) {
            if self.scroller(start).is_some() && self.rect(start).contains(point) {
                found.push(start);
            }
            self.scrollers_in(start, point, &mut found);
        }
        found
    }

    fn scrollers_in(&self, id: NodeId, point: Point, found: &mut Vec<NodeId>) -> bool {
        for child in self.stacked(id).into_iter().rev() {
            if !self.visible(child) {
                continue;
            }
            if self.scroller(child).is_some() {
                if self.rect(child).contains(point) {
                    found.push(child);
                    self.scrollers_in(child, point, found);
                    return true;
                }
            } else if self.scrollers_in(child, point, found) {
                return true;
            }
        }
        false
    }

    /// Rolagem de `id`, se for um `ScrollViewer`.
    pub fn scroller(&self, id: NodeId) -> Option<Scroller> {
        self.widget(id).scroller().copied()
    }

    /// Muda a rolagem de `id` com `f` e põe o widget na fila se mudou.
    fn update_scroller(&mut self, id: NodeId, f: impl FnOnce(&mut Scroller) -> bool) -> bool {
        let widget = Rc::clone(&self.nodes[id].widget);
        let mut widget = widget.borrow_mut();
        let changed = widget.scroller_mut().is_some_and(f);
        if changed {
            self.dirty.borrow_mut().push(id);
            if let (Some(link), Some(scroller)) = (widget.scroll_link(), widget.scroller()) {
                untrack(|| link.publish(scroller));
            }
        }
        changed
    }

    /// Atende os pedidos de rolagem dos [`ScrollLink`](crate::ScrollLink)s.
    fn serve_scroll_links(&mut self) {
        for id in self.linked.clone() {
            let Some(node) = self.nodes.get(id) else {
                continue;
            };
            let (dx, dy) = node
                .widget
                .borrow()
                .scroll_link()
                .map_or((0.0, 0.0), |link| link.take_request());
            if dx != 0.0 || dy != 0.0 {
                self.scroll_by(id, Vec2::new(dx, dy));
            }
        }
    }

    /// Tamanhos do conteúdo e da área visível de um `ScrollViewer`, medidos pelo layout.
    /// Devolve `true` se mudaram (o deslocamento pode ter diminuído).
    pub fn set_scroll_extent(&mut self, id: NodeId, content: Size, viewport: Size) -> bool {
        self.update_scroller(id, |s| s.set_extent(content, viewport))
    }

    /// Rola o `ScrollViewer` `id` em `delta` pixels. Devolve `true` se mudou.
    pub fn scroll_by(&mut self, id: NodeId, delta: Vec2) -> bool {
        self.update_scroller(id, |s| s.scroll_by(delta))
    }

    /// Rola o `ScrollViewer` `id` até `offset`. Devolve `true` se mudou.
    pub fn scroll_to(&mut self, id: NodeId, offset: Vec2) -> bool {
        self.update_scroller(id, |s| s.scroll_to(offset))
    }

    /// Rola o `ScrollViewer` mais interno sob `point` que ainda anda na direção de `delta`.
    fn scroll_at(&mut self, point: Point, delta: Vec2) -> bool {
        for id in self.scrollers_at(point).into_iter().rev() {
            let horizontal = self
                .scroller(id)
                .is_some_and(|s| s.mode() == ScrollMode::Horizontal);
            // A roda comum rola na horizontal um `ScrollViewer` que só rola assim.
            let delta = if horizontal && delta.x == 0.0 {
                Vec2::new(delta.y, 0.0)
            } else {
                delta
            };
            if self.scroll_by(id, delta) {
                return true;
            }
        }
        false
    }

    /// Roda do mouse ou rolagem do touchpad em `point`, em pixels: `delta` positivo mostra o
    /// que está mais abaixo (ou à direita). Com Shift, a roda vertical rola na horizontal.
    ///
    /// Antes, os widgets sob o ponteiro com [`Widget::wants_scroll`] (o `Terminal`) recebem
    /// [`Event::Scroll`]. Depois, rola o `ScrollViewer` mais interno sob o ponteiro que ainda
    /// anda nessa direção; se ele já está no fim, rola o de fora. Devolve `true` se algo
    /// rolou.
    pub fn wheel(&mut self, point: Point, delta: Vec2) -> bool {
        self.pointer = point;
        self.cancel_tip();
        let scroll = Event::Scroll {
            x: delta.x.round() as i32,
            y: delta.y.round() as i32,
        };
        let mut node = self.hit(point);
        while let Some(id) = node {
            if self.widget(id).wants_scroll() && self.send(id, &scroll) {
                return true;
            }
            node = self.parent(id);
        }
        if let Some(focused) = self.focus
            && delta.y != 0.0
            && !self.modifiers.shift
            && self.shown(focused)
            && self
                .hit(point)
                .is_some_and(|hit| self.is_within(hit, focused))
            && self.send(focused, &Event::Wheel { up: delta.y < 0.0 })
        {
            return true;
        }
        let delta = if self.modifiers.shift && delta.x == 0.0 {
            Vec2::new(delta.y, 0.0)
        } else {
            delta
        };
        let moved = self.scroll_at(point, delta);
        if moved {
            // O conteúdo andou embaixo do ponteiro parado.
            self.pointer_move(point, PointerKind::Mouse);
        }
        moved
    }

    /// Rola os `ScrollViewer` em volta de `id` até ele aparecer inteiro (com uma folga para o
    /// anel de foco), do mais interno para fora. O foco pelo teclado chama sozinho.
    pub fn bring_into_view(&mut self, id: NodeId) {
        let mut node = self.up(id);
        while let Some(sv) = node {
            node = self.up(sv);
            if self.scroller(sv).is_none() {
                continue;
            }
            let target = self.rect(id).inflate(FOCUS_MARGIN, FOCUS_MARGIN);
            let view = self.rect(sv);
            let delta = Vec2::new(
                reveal(target.x0, target.x1, view.x0, view.x1),
                reveal(target.y0, target.y1, view.y0, view.y1),
            );
            self.scroll_by(sv, delta);
        }
    }

    /// Rola com o teclado a partir de `from` (o widget com foco), subindo pelos `ScrollViewer`
    /// até um rolar.
    fn scroll_key(&mut self, key: Key) {
        let from = self
            .focus
            .or_else(|| self.scrollers_at(self.pointer).last().copied())
            .or_else(|| {
                self.ids()
                    .find(|&id| self.scroller(id).is_some() && self.shown(id) && self.in_scope(id))
            });
        let mut node = from;
        while let Some(id) = node {
            node = self.up(id);
            let Some(s) = self.scroller(id) else {
                continue;
            };
            let (x, y) = (s.mode().x(), s.mode().y());
            let along = |v: f64| {
                if y {
                    Vec2::new(0.0, v)
                } else {
                    Vec2::new(v, 0.0)
                }
            };
            let page = if y {
                s.viewport().height
            } else {
                s.viewport().width
            };
            let offset = s.offset();
            let moved = match key {
                Key::Up if y => self.scroll_by(id, Vec2::new(0.0, -SCROLL_LINE)),
                Key::Down if y => self.scroll_by(id, Vec2::new(0.0, SCROLL_LINE)),
                Key::Left if x => self.scroll_by(id, Vec2::new(-SCROLL_LINE, 0.0)),
                Key::Right if x => self.scroll_by(id, Vec2::new(SCROLL_LINE, 0.0)),
                Key::PageUp => self.scroll_by(id, along(-page)),
                Key::PageDown => self.scroll_by(id, along(page)),
                Key::Home if y => self.scroll_to(id, Vec2::new(offset.x, 0.0)),
                Key::Home => self.scroll_to(id, Vec2::new(0.0, offset.y)),
                Key::End if y => self.scroll_to(id, Vec2::new(offset.x, f64::INFINITY)),
                Key::End => self.scroll_to(id, Vec2::new(f64::INFINITY, offset.y)),
                Key::Up | Key::Down | Key::Left | Key::Right => false,
                _ => return,
            };
            if moved {
                if let Some(kind) = self.pointer_kind
                    && kind.hovers()
                {
                    self.pointer_move(self.pointer, kind);
                }
                return;
            }
        }
    }

    /// Troca as medidas de texto usadas pelos widgets.
    pub fn set_text_metrics(&mut self, metrics: Box<dyn TextMetrics>) {
        self.metrics = metrics;
    }

    /// Troca o clipboard.
    pub fn set_clipboard(&mut self, clipboard: Box<dyn Clipboard>) {
        self.clipboard = clipboard;
    }

    /// Clipboard atual.
    pub fn clipboard_mut(&mut self) -> &mut dyn Clipboard {
        &mut *self.clipboard
    }

    /// Modificadores pressionados agora. A plataforma avisa quando mudam.
    pub fn set_modifiers(&mut self, modifiers: Modifiers) {
        self.modifiers = modifiers;
    }

    /// Modificadores pressionados.
    pub fn modifiers(&self) -> Modifiers {
        self.modifiers
    }

    /// Entrega `event` a `id` e põe o widget na fila.
    fn send(&mut self, id: NodeId, event: &Event) -> bool {
        let rect = self.rect(id);
        let widget = Rc::clone(&self.nodes[id].widget);
        let mut cx = EventCx::new(
            rect,
            self.pointer,
            self.modifiers,
            &mut *self.metrics,
            &mut *self.clipboard,
        );
        let consumed = widget.borrow_mut().handle(event, &mut cx);
        let request = cx.focus_request();
        if let Some(id) = cx.take_focus_later() {
            self.pending_focus = Some((id, FOCUS_LATER_TRIES));
        }
        self.window_requests.extend(cx.window_request());
        self.dirty.borrow_mut().push(id);
        self.serve_scroll_links();
        if let Some(to) = request {
            self.focus_group(id, to);
        }
        consumed
    }

    /// Move o foco a pedido de `id` ([`EventCx::move_focus`]).
    fn focus_group(&mut self, id: NodeId, to: FocusMove) {
        if to == FocusMove::Parent {
            let parent = std::iter::successors(self.parent(id), |&p| self.parent(p))
                .take_while(|&p| self.nodes[p].layer.is_none())
                .find(|&p| self.can_focus(p));
            if let Some(parent) = parent {
                self.set_focus(Some(parent), FocusSource::Keyboard);
            }
            return;
        }
        if to == FocusMove::Forward {
            let next = self.order[self.position[id] + 1..]
                .iter()
                .copied()
                .find(|&n| self.can_focus(n) && self.tab_stop(n));
            if let Some(next) = next {
                self.set_focus(Some(next), FocusSource::Keyboard);
            }
            return;
        }
        if to == FocusMove::Backward {
            let prev = self.order[..self.position[id]]
                .iter()
                .rev()
                .copied()
                .find(|&n| self.can_focus(n) && self.tab_stop(n) && !self.is_inside(id, n));
            if let Some(prev) = prev {
                self.set_focus(Some(prev), FocusSource::Keyboard);
            }
            return;
        }
        let group = match to {
            FocusMove::Inside => id,
            _ => {
                let parent = self.parent(id);
                let scope = std::iter::successors(parent, |&p| self.parent(p))
                    .take_while(|&p| self.nodes[p].layer.is_none())
                    .find(|&p| self.widget(p).focus_scope());
                match scope.or(parent) {
                    Some(group) => group,
                    None => return,
                }
            }
        };
        let start = self.position[group] + 1;
        let members: Vec<NodeId> = self.order[start..]
            .iter()
            .copied()
            .take_while(|&n| self.is_inside(n, group))
            .filter(|&n| n == id || self.can_focus(n))
            .collect();
        let at = members.iter().position(|&n| n == id);
        let target = match (to, at) {
            (FocusMove::Inside, _) => members
                .iter()
                .copied()
                .find(|&n| n != id && self.tab_stop(n)),
            (FocusMove::First, _) => members.first().copied(),
            (FocusMove::Last, _) => members.last().copied(),
            (FocusMove::Prev, Some(i)) => i.checked_sub(1).map(|i| members[i]),
            (FocusMove::Next, Some(i)) => members.get(i + 1).copied(),
            (_, None) | (FocusMove::Parent | FocusMove::Forward | FocusMove::Backward, _) => None,
        };
        if let Some(target) = target.filter(|&t| self.can_focus(t)) {
            self.set_focus(Some(target), FocusSource::Keyboard);
        }
    }

    /// Estado da janela da `TitleBar`, se a view tem uma.
    pub fn window_state(&self) -> Option<WindowState> {
        self.window
    }

    /// Pedidos à janela ([`EventCx::request_window`]) desde a última chamada, na ordem.
    pub fn take_window_requests(&mut self) -> Vec<WindowRequest> {
        std::mem::take(&mut self.window_requests)
    }

    /// Ponteiro em `point`. Com o ponteiro capturado, só o widget que capturou recebe.
    ///
    /// Um dedo que anda mais de 8 px sobre um `ScrollViewer` passa a rolar o conteúdo: o
    /// widget que tinha capturado o toque recebe [`Event::PointerCancel`] e não clica.
    pub fn pointer_move(&mut self, point: Point, kind: PointerKind) {
        self.pointer = point;
        self.pointer_kind = Some(kind);
        if self
            .hold
            .is_some_and(|hold| (point - hold.origin).hypot() >= PAN_SLOP)
        {
            self.hold = None;
            // O dedo andou antes do toque longo: é rolagem ou toque, não arrasto.
            if self.drag.as_ref().is_some_and(|drag| !drag.armed) {
                self.drag = None;
            }
        }
        if self.drag_move(point) {
            return;
        }
        if kind == PointerKind::Touch
            && let Some(mut pan) = self.pan
        {
            if !pan.active && (point - pan.origin).hypot() >= PAN_SLOP {
                pan.active = true;
                let captured = self.capture.take();
                let hovered = self.hover.take().filter(|&id| Some(id) != captured);
                for id in [captured, hovered].into_iter().flatten() {
                    self.send(id, &Event::PointerCancel);
                }
            }
            if pan.active {
                self.scroll_at(pan.origin, pan.last - point);
                pan.last = point;
                self.pan = Some(pan);
                return;
            }
        }
        if let Some(captured) = self.capture {
            let inside = self.inside(captured, point);
            self.send(captured, &Event::PointerMove { kind, inside });
            self.hover = inside.then_some(captured);
            self.sync_within();
            return;
        }
        let target = self.hit(point);
        if target != self.hover
            && let Some(old) = self.hover
        {
            self.send(
                old,
                &Event::PointerMove {
                    kind,
                    inside: false,
                },
            );
        }
        self.hover = target;
        if let Some(target) = target {
            self.send(target, &Event::PointerMove { kind, inside: true });
        }
        self.sync_within();
        let owner = kind.hovers().then(|| self.tip_owner_at(point)).flatten();
        self.hover_tip(owner);
        if let Some(tip) = self.tip {
            self.anchor_tip(tip.owner);
        }
    }

    /// Botão primário ou dedo pressionado em `point`. O widget atingido recebe o foco (se
    /// aceitar) e captura o ponteiro até o [`Tree::pointer_up`]. `clicks` é 2 no duplo clique e
    /// 3 no triplo; quem conta é a plataforma.
    ///
    /// Clicar fora de um widget que aceita foco não tira o foco de quem o tem.
    ///
    /// Com uma camada aberta, um clique fora dela a fecha, se ela fecha assim
    /// ([`Layer::light_dismiss`]), e não chega a ninguém.
    pub fn pointer_down(&mut self, point: Point, kind: PointerKind, clicks: u8) {
        self.pan = None;
        self.hold = None;
        self.end_drag();
        self.alt_alone = false;
        self.last_source = FocusSource::Pointer;
        self.pointer_move(point, kind);
        self.cancel_tip();
        if let Some(&Open { id: top, .. }) = self.layers.last()
            && !self.rect(top).contains(point)
            && self.passive_at(point).is_none()
        {
            let root = self.input_root_at(point);
            if root == top || self.hit(point).is_none() {
                self.light_dismiss(top);
                self.pointer_move(point, kind);
                return;
            }
            // O ponteiro passou ao menu de baixo (o de onde saiu o submenu) ou à barra de
            // menus: fecham as camadas acima dele, e o clique chega lá.
            if let Some(i) = self.layers.iter().position(|open| open.id == root) {
                self.close_above(i);
            }
        }
        let target = self.hit(point);
        let on_bar = target.is_some_and(|id| self.scroller(id).is_some());
        if kind == PointerKind::Touch
            && !on_bar
            && self
                .scrollers_at(point)
                .iter()
                .any(|&id| self.scroller(id).is_some_and(|s| s.can_scroll()))
        {
            self.pan = Some(Pan {
                origin: point,
                last: point,
                active: false,
            });
        }
        let Some(target) = target else {
            return;
        };
        if kind == PointerKind::Touch && self.context_owner(Some(target), Some(point)).is_some() {
            self.hold = Some(Hold {
                origin: point,
                target,
                elapsed: Duration::ZERO,
            });
        }
        if self.can_focus(target) {
            self.set_focus(Some(target), FocusSource::Pointer);
        }
        if self.send(target, &Event::PointerDown { kind, clicks }) {
            self.capture = Some(target);
            let data = self.nodes.get(target).and_then(|node| {
                let widget = Rc::clone(&node.widget);
                untrack(|| widget.borrow().drag_data())
            });
            if let Some(data) = data {
                let touch = kind == PointerKind::Touch;
                if touch && self.hold.is_none() {
                    self.hold = Some(Hold {
                        origin: point,
                        target,
                        elapsed: Duration::ZERO,
                    });
                }
                self.drag = Some(Drag {
                    source: target,
                    data,
                    origin: point,
                    armed: !touch,
                    active: false,
                    target: None,
                });
            }
        }
    }

    /// Ponteiro em `point` com o botão pressionado sobre um widget que arrasta: começa o
    /// arrasto depois do limiar e, com ele começado, leva o item aos alvos. Devolve `true`
    /// se o arrasto ficou com o movimento.
    fn drag_move(&mut self, point: Point) -> bool {
        let Some(drag) = &mut self.drag else {
            return false;
        };
        if !drag.active {
            if !drag.armed || (point - drag.origin).hypot() < DRAG_THRESHOLD {
                return false;
            }
            drag.active = true;
            self.hold = None;
            self.pan = None;
            let captured = self.capture.take();
            let hovered = self.hover.take().filter(|&id| Some(id) != captured);
            for id in [captured, hovered].into_iter().flatten() {
                self.send(id, &Event::PointerCancel);
            }
            self.cancel_tip();
        }
        let Some(data) = self.drag.as_ref().map(|drag| drag.data.clone()) else {
            return false;
        };
        let target = self.drag_target_at(point, data);
        let old = self
            .drag
            .as_mut()
            .and_then(|drag| std::mem::replace(&mut drag.target, target));
        if let Some(old) = old
            && Some(old) != target
            && self.contains(old)
        {
            self.send(old, &Event::DragLeave);
        }
        true
    }

    /// O alvo que aceita `data` em `point`: o widget sob o ponteiro ou o ancestral mais
    /// próximo que responde ao [`Event::DragOver`].
    fn drag_target_at(&mut self, point: Point, data: DragData) -> Option<NodeId> {
        let event = Event::DragOver(data);
        let mut node = self.hit(point).or_else(|| self.deepest_at(point));
        while let Some(id) = node {
            if self.send(id, &event) {
                return Some(id);
            }
            if self.nodes.get(id).is_none_or(|n| n.layer.is_some()) {
                break;
            }
            node = self.parent(id);
        }
        None
    }

    /// O item que está sendo arrastado a partir de um widget desta árvore, depois de passar do
    /// limiar. A plataforma o leva às outras janelas.
    pub fn drag_data(&self) -> Option<DragData> {
        self.drag
            .as_ref()
            .filter(|drag| drag.active)
            .map(|drag| drag.data.clone())
    }

    /// Um item arrastado a partir de outra janela passou por `point` desta: os alvos recebem o
    /// [`Event::DragOver`], como se o arrasto fosse daqui. Devolve se algum aceitou.
    pub fn foreign_drag_over(&mut self, data: &DragData, point: Point) -> bool {
        self.pointer = point;
        let target = self.drag_target_at(point, data.clone());
        let old = std::mem::replace(&mut self.foreign_target, target);
        if let Some(old) = old
            && Some(old) != target
            && self.contains(old)
        {
            self.send(old, &Event::DragLeave);
        }
        target.is_some()
    }

    /// O item arrastado a partir de outra janela saiu desta: o alvo apaga a marca.
    pub fn foreign_drag_leave(&mut self) {
        if let Some(target) = self.foreign_target.take()
            && self.contains(target)
        {
            self.send(target, &Event::DragLeave);
        }
    }

    /// Um item arrastado a partir de outra janela foi solto em `point` desta: o alvo recebe o
    /// [`Event::Drop`]. Devolve se algum aceitou.
    pub fn foreign_drop(&mut self, data: DragData, point: Point) -> bool {
        self.foreign_drag_over(&data, point);
        match self.foreign_target.take().filter(|&t| self.contains(t)) {
            Some(target) => {
                self.send(target, &Event::Drop(data));
                true
            }
            None => false,
        }
    }

    /// Cancela o arrasto que começou aqui, sem soltar: o alvo apaga a marca. A plataforma chama
    /// quando o item foi para outra janela.
    pub fn cancel_drag(&mut self) {
        self.end_drag();
    }

    /// Arquivos do sistema foram soltos em `point` (ou, sem ponto, onde o ponteiro estava): o
    /// [`Event::FilesDropped`] vai ao widget sob o ponteiro e, se ele não usar, aos
    /// ancestrais. Devolve se algum usou.
    pub fn files_dropped(&mut self, paths: Vec<std::path::PathBuf>, point: Option<Point>) -> bool {
        if let Some(point) = point {
            self.pointer = point;
        }
        let point = self.pointer;
        let node = self.hit(point).or_else(|| self.deepest_at(point));
        self.bubble_from(node, &Event::FilesDropped { paths })
    }

    /// O arrasto que começou aqui foi solto fora de todas as janelas: o widget de onde o item
    /// saiu recebe o [`Event::DropOutside`], com o ponto na tela. Devolve se havia arrasto.
    pub fn drop_outside(&mut self, at: Option<(i32, i32)>) -> bool {
        let Some(drag) = self.drag.take().filter(|drag| drag.active) else {
            return false;
        };
        self.capture = None;
        if let Some(target) = drag.target.filter(|&t| self.contains(t)) {
            self.send(target, &Event::DragLeave);
        }
        if self.contains(drag.source) {
            self.send(
                drag.source,
                &Event::DropOutside {
                    data: drag.data,
                    at,
                },
            );
        }
        true
    }

    /// Termina o arrasto sem soltar: o alvo apaga a marca.
    fn end_drag(&mut self) {
        if let Some(Drag {
            target: Some(target),
            ..
        }) = self.drag.take()
            && self.contains(target)
        {
            self.send(target, &Event::DragLeave);
        }
    }

    /// Texto e posição da etiqueta do arrasto em andamento, se houver: a `Ui` a pinta no
    /// ponteiro, por cima de tudo.
    pub fn drag_label(&self) -> Option<(String, Point)> {
        self.drag
            .as_ref()
            .filter(|drag| drag.active)
            .map(|drag| (drag.data.label.clone(), self.pointer))
    }

    /// Widget mais profundo sob `point`, interativo ou não: de onde o [`Event::DragOver`] e o
    /// botão direito sobem quando não há um widget interativo embaixo.
    fn deepest_at(&self, point: Point) -> Option<NodeId> {
        let mut id = self.input_root_at(point);
        if !self.visible(id) || !self.rect(id).contains(point) {
            return None;
        }
        loop {
            let child = self.stacked(id).into_iter().rev().find(|&child| {
                self.visible(child)
                    && self.rect(child).contains(point)
                    && self.clip(child).contains(point)
            });
            match child {
                Some(child) => id = child,
                None => return Some(id),
            }
        }
    }

    /// Fecha a camada `top` se ela fecha com o clique fora ([`Layer::light_dismiss`]).
    /// Um submenu leva junto os menus de onde saiu ([`Layer::pass_through`]).
    fn light_dismiss(&mut self, top: NodeId) {
        let mut target = top;
        if let Some(mut i) = self.layers.iter().position(|open| open.id == top) {
            while i > 0
                && self.nodes[self.layers[i].id]
                    .layer
                    .is_some_and(|layer| layer.pass_through)
                && self.is_within(self.layers[i].id, self.layers[i - 1].id)
            {
                i -= 1;
            }
            target = self.layers[i].id;
        }
        if self.nodes[target]
            .layer
            .is_some_and(|layer| layer.light_dismiss)
        {
            self.close_layer(target);
        }
    }

    /// Fecha as camadas abertas acima da posição `i` da pilha, de cima para baixo.
    fn close_above(&mut self, i: usize) {
        while self.layers.len() > i + 1 {
            let len = self.layers.len();
            let top = self.layers[len - 1].id;
            self.close_layer(top);
            if self.layers.len() >= len {
                // A camada não fechou (o app a mantém visível): não insiste.
                break;
            }
        }
    }

    /// Dono do menu de contexto mais próximo a partir de `start`, subindo pelos ancestrais até
    /// a camada em que ele está.
    fn context_owner(&self, start: Option<NodeId>, at: Option<Point>) -> Option<NodeId> {
        let mut node = start;
        while let Some(id) = node {
            if self.nodes[id].context.is_some()
                && self.shown(id)
                && untrack(|| self.widget(id).context_available(self.rect(id), at))
            {
                return Some(id);
            }
            if self.nodes[id].layer.is_some() {
                return None;
            }
            node = self.parent(id);
        }
        None
    }

    /// Abre o menu de contexto de `owner`: com o canto no ponto `at` ou, sem ponto, junto do
    /// widget, como um menu de botão.
    pub fn open_context(&mut self, owner: NodeId, at: Option<Point>) {
        let Some(menu) = self.nodes[owner].context else {
            return;
        };
        self.cancel_tip();
        let rect = self.rect(owner);
        let owner_widget = Rc::clone(&self.nodes[owner].widget);
        untrack(|| owner_widget.borrow_mut().context_opening_at(rect, at));
        if self.send(owner, &Event::ContextOpening) && self.can_focus(owner) {
            self.set_focus(Some(owner), self.last_source);
        }
        self.nodes[menu].anchor = at.map(|at| Rect::from_origin_size(at, Size::ZERO));
        let widget = Rc::clone(&self.nodes[menu].widget);
        untrack(|| widget.borrow_mut().set_open(true));
        self.dirty.borrow_mut().push(menu);
        self.sync_layers();
    }

    /// Botão secundário (o direito) pressionado em `point`: [`Event::SecondaryDown`] vai ao
    /// widget sob o ponteiro e, se ele não usar, aos ancestrais. Não muda o foco. Com uma
    /// camada aberta, só os widgets dela recebem.
    ///
    /// O primeiro widget com `.context_flyout()` no caminho abre o menu de contexto com o canto
    /// no ponteiro, no lugar de receber o evento (o `Terminal` só cola sem menu de contexto).
    /// Um menu aberto fecha antes, como no clique fora.
    pub fn secondary_down(&mut self, point: Point) {
        self.pointer = point;
        self.secondary = None;
        self.alt_alone = false;
        // O menu de contexto aberto pelo mouse foca o primeiro item sem o anel de foco.
        self.last_source = FocusSource::Pointer;
        self.cancel_tip();
        if let Some(&Open { id: top, .. }) = self.layers.last()
            && !self.rect(top).contains(point)
            && self.passive_at(point).is_none()
            && self.input_root_at(point) == top
        {
            self.light_dismiss(top);
        }
        // Um `Text` com menu de contexto não é interativo: sem um widget interativo embaixo,
        // vale o mais profundo.
        let mut node = self.hit(point).or_else(|| self.deepest_at(point));
        while let Some(id) = node {
            if self.nodes[id].context.is_some()
                && self
                    .widget(id)
                    .context_on_secondary_at(self.rect(id), point, self.modifiers)
            {
                self.open_context(id, Some(point));
                return;
            }
            if self.send(id, &Event::SecondaryDown) {
                self.secondary = Some(id);
                return;
            }
            if self.nodes.get(id).is_none_or(|n| n.layer.is_some()) {
                return;
            }
            node = self.parent(id);
        }
    }

    /// Botão secundário (o direito) solto em `point`: [`Event::SecondaryUp`] vai ao widget que
    /// usou o [`Event::SecondaryDown`], se algum usou.
    pub fn secondary_up(&mut self, point: Point) {
        self.pointer = point;
        if let Some(target) = self.secondary.take()
            && self.contains(target)
        {
            self.send(target, &Event::SecondaryUp);
        }
    }

    /// Botão do meio pressionado em `point`: [`Event::MiddleDown`] vai ao widget sob o
    /// ponteiro, que fica guardado para o [`Tree::middle_up`].
    pub fn middle_down(&mut self, point: Point) {
        self.pointer = point;
        self.cancel_tip();
        self.middle = self.hit(point);
        if let Some(target) = self.middle {
            self.send(target, &Event::MiddleDown);
        }
    }

    /// Botão do meio solto em `point`: [`Event::MiddleUp`] vai ao widget em que o botão desceu
    /// e, se ele não usar e o ponteiro ainda estiver sobre ele, [`Event::MiddleClick`] vai a
    /// ele e, se ele não usar, aos ancestrais.
    pub fn middle_up(&mut self, point: Point) {
        self.pointer = point;
        let Some(target) = self.middle.take() else {
            return;
        };
        if !self.contains(target) || self.send(target, &Event::MiddleUp) {
            return;
        }
        if self.hit(point) == Some(target) {
            self.bubble_from(Some(target), &Event::MiddleClick);
        }
    }

    /// Botão primário ou dedo solto em `point`.
    pub fn pointer_up(&mut self, point: Point, kind: PointerKind) {
        self.hold = None;
        if let Some(drag) = self.drag.take() {
            if drag.active {
                self.pan = None;
                self.capture = None;
                self.pointer = point;
                if let Some(target) = drag.target.filter(|&t| self.contains(t)) {
                    self.send(target, &Event::Drop(drag.data));
                }
                self.pointer_move(point, kind);
                return;
            }
            // O toque longo armou o arrasto e o dedo saiu sem andar: é o menu de contexto.
            if kind == PointerKind::Touch
                && drag.armed
                && self.contains(drag.source)
                && let Some(owner) = self.context_owner(Some(drag.source), Some(drag.origin))
            {
                self.pan = None;
                self.pointer = point;
                self.open_context(owner, Some(drag.origin));
                return;
            }
        }
        if let Some(pan) = self.pan.take()
            && pan.active
        {
            // O toque virou rolagem: ninguém recebe o fim dele.
            self.pointer = point;
            return;
        }
        if let Some(captured) = self.capture.take() {
            let inside = self.inside(captured, point);
            self.send(captured, &Event::PointerUp { kind, inside });
        }
        self.pointer_move(point, kind);
    }

    /// O ponteiro saiu da janela ou o gesto foi cancelado.
    pub fn pointer_cancel(&mut self) {
        self.pan = None;
        self.hold = None;
        self.end_drag();
        let captured = self.capture.take();
        let hovered = self.hover.take().filter(|&id| Some(id) != captured);
        for id in [captured, hovered].into_iter().flatten() {
            self.send(id, &Event::PointerCancel);
        }
        self.sync_within();
    }

    /// Tecla pressionada. Tab move o foco (Shift+Tab, para trás) pelas paradas de Tab
    /// ([`Widget::tab_stop`]), menos no widget com [`Widget::captures_tab`], que recebe o Tab
    /// e o Shift+Tab; o resto vai para o widget com foco. Ctrl+Tab vai antes para o
    /// widget com foco e os ancestrais (o `TabView` troca de aba) e, se ninguém usar, move o
    /// foco como o Tab.
    ///
    /// Com uma camada aberta, a tecla que o widget não usa vai para ela: Esc a fecha, e ↑, ↓,
    /// Home e End movem o foco entre os widgets dela nas que têm [`Layer::arrows`]. Setas,
    /// PageUp/PageDown e Home/End que ninguém usou rolam o `ScrollViewer` em volta do widget
    /// (sem foco, o que está sob o ponteiro ou o primeiro da tela).
    ///
    /// A tecla que o widget com foco não usa vai aos atalhos (`.accelerator()`); no widget com
    /// [`Widget::accelerators_first`] (o `Terminal`), os atalhos vêm antes. O Shift+F10 e a
    /// tecla Menu abrem o menu de contexto do widget com foco (ou do ancestral mais próximo
    /// que tem um); o F10 e o Alt pressionado e solto sozinho põem o foco na barra de menus, e
    /// o Esc, o F10 e o Alt devolvem o foco a quem tinha antes.
    pub fn key_down(&mut self, key: Key) {
        self.last_source = FocusSource::Keyboard;
        self.cancel_tip();
        self.swallow_text = false;
        let others = Modifiers {
            alt: false,
            ..self.modifiers
        };
        self.alt_alone = key == Key::Alt && others == Modifiers::NONE;
        if key == Key::Alt {
            return;
        }
        if key == Key::Escape && self.drag.as_ref().is_some_and(|drag| drag.active) {
            self.end_drag();
            return;
        }
        if ((key == Key::ContextMenu && self.modifiers == Modifiers::NONE)
            || (key == Key::F(10) && self.modifiers == Modifiers::SHIFT))
            && let Some(owner) = self.context_owner(self.focus, None)
        {
            self.open_context(owner, None);
            return;
        }
        if key == Key::Tab {
            if self.modifiers.ctrl && self.bubble(&Event::KeyDown(key)) {
                return;
            }
            if !self.modifiers.ctrl
                && let Some(id) = self.focus
                && self.widget(id).captures_tab()
                && self.send(id, &Event::KeyDown(key))
            {
                return;
            }
            if self.modifiers.shift {
                self.focus_prev();
            } else {
                self.focus_next();
            }
            return;
        }
        if self
            .focus
            .is_some_and(|id| untrack(|| self.widget(id).accelerators_first()))
            && self.run_accelerator(key)
        {
            return;
        }
        // O Ctrl+F4 sobe até um `TabView` em volta, que fecha a aba escolhida.
        let consumed = if key == Key::F(4) && self.modifiers.ctrl {
            self.bubble(&Event::KeyDown(key))
        } else {
            match self.focus {
                Some(id) => self.send(id, &Event::KeyDown(key)),
                None => false,
            }
        };
        if consumed {
            return;
        }
        if key == Key::F(10) && self.modifiers == Modifiers::NONE && self.toggle_menu_bar() {
            return;
        }
        let plain = !self.modifiers.command() && !self.modifiers.alt;
        if let Some(&Open { id: top, .. }) = self.layers.last() {
            let arrows = self.nodes[top].layer.is_some_and(|layer| layer.arrows);
            if plain && arrows && matches!(key, Key::Up | Key::Down | Key::Home | Key::End) {
                self.focus_arrow(top, key);
                return;
            }
            if key == Key::Escape {
                self.escaping = true;
                self.close_layer(top);
                self.escaping = false;
                return;
            }
        }
        if self.run_accelerator(key) {
            return;
        }
        if let Some(&Open { id: top, .. }) = self.layers.last() {
            if self.focus != Some(top) && self.send(top, &Event::KeyDown(key)) {
                // A letra que a camada usou (a tecla de acesso de um item de menu) não é
                // digitada em quem recebe o foco quando ela fecha.
                if plain && matches!(key, Key::Char(_)) {
                    self.swallow_text = true;
                }
                return;
            }
        } else if key == Key::Escape && self.leave_menu_bar() {
            return;
        } else if key == Key::Escape
            && let Some(focus) = self.focus
            && self.bubble_from(self.parent(focus), &Event::KeyDown(key))
        {
            // Sem camada aberta, o Esc que o widget com foco não usou sobe aos ancestrais (o
            // `NavigationView` fecha o painel que abriu por cima do conteúdo).
            return;
        }
        if plain {
            self.scroll_key(key);
        }
    }

    /// Se o atalho de `id` vale agora: o widget aparece e está habilitado (um `MenuFlyout`
    /// fechado no caminho não conta, [`Widget::accelerators_while_closed`]) e, com um
    /// `ContentDialog` aberto, está dentro dele.
    pub fn accelerator_active(&self, id: NodeId) -> bool {
        if !untrack(|| self.widget(id).enabled()) {
            return false;
        }
        let mut node = Some(id);
        while let Some(n) = node {
            let closed_menu = self.nodes[n].layer.is_some()
                && untrack(|| self.widget(n).accelerators_while_closed());
            if !closed_menu && !self.visible(n) {
                return false;
            }
            node = self.parent(n);
        }
        let modal = self
            .layers
            .iter()
            .rev()
            .find(|open| self.nodes[open.id].layer.is_some_and(|layer| layer.modal));
        modal.is_none_or(|modal| self.is_within(id, modal.id))
    }

    /// Roda o primeiro atalho, na ordem da árvore, com `key` e os modificadores de agora que
    /// vale agora. Devolve `true` se rodou.
    fn run_accelerator(&mut self, key: Key) -> bool {
        let shortcut = Shortcut::new(key, self.modifiers);
        let found = self.accelerator_owners.iter().copied().find_map(|id| {
            let accelerator = self
                .widget(id)
                .common()
                .accelerators
                .iter()
                .find(|a| a.matches(shortcut))
                .cloned()?;
            self.accelerator_active(id).then_some((id, accelerator))
        });
        let Some((id, accelerator)) = found else {
            return false;
        };
        untrack(|| accelerator.run());
        self.dirty.borrow_mut().push(id);
        self.swallow_text = true;
        self.grant_focus_request(id);
        true
    }

    /// Leva o pedido de foco de `id` ([`Widget::take_focus_request`]) para o foco pendente.
    fn grant_focus_request(&mut self, id: NodeId) {
        let Some(node) = self.nodes.get(id) else {
            return;
        };
        let widget = Rc::clone(&node.widget);
        let request = untrack(|| widget.borrow_mut().take_focus_request());
        if let Some(name) = request {
            self.pending_focus = Some((name, FOCUS_LATER_TRIES));
        }
    }

    /// Barras de menus que aparecem, na ordem da árvore.
    fn menu_bars(&self) -> Vec<NodeId> {
        self.order
            .iter()
            .copied()
            .filter(|&id| untrack(|| self.widget(id).menu_bar()) && self.shown(id))
            .collect()
    }

    /// F10 ou Alt solto sozinho: põe o foco no primeiro item da barra de menus ou, com o foco
    /// já nela (ou num menu aberto dela), fecha os menus e devolve o foco a quem tinha antes.
    /// Devolve `true` se havia barra.
    fn toggle_menu_bar(&mut self) -> bool {
        let bars = self.menu_bars();
        if let Some(&Open { id: top, .. }) = self.layers.last()
            && let Some(&bar) = bars.iter().find(|&&bar| self.is_within(top, bar))
        {
            if let Some(first) = self
                .layers
                .iter()
                .position(|open| self.is_within(open.id, bar))
            {
                self.close_above(first);
                if let Some(&Open { id: menu, .. }) = self.layers.get(first) {
                    self.close_layer(menu);
                }
            }
            self.leave_menu_bar();
            return true;
        }
        if self.leave_menu_bar() {
            return true;
        }
        let Some(bar) = bars.into_iter().find(|&bar| self.in_scope(bar)) else {
            return false;
        };
        let first = self.order[self.position[bar] + 1..]
            .iter()
            .copied()
            .take_while(|&n| self.is_inside(n, bar))
            .find(|&n| self.can_focus(n) && self.tab_stop(n));
        let Some(first) = first else {
            return false;
        };
        self.set_focus(Some(first), FocusSource::Keyboard);
        true
    }

    /// Com o foco na barra de menus, devolve o foco a quem tinha antes de ele chegar nela.
    /// Devolve `true` se o foco estava na barra.
    fn leave_menu_bar(&mut self) -> bool {
        let Some(focus) = self.focus else {
            return false;
        };
        if self.menu_bar_of(focus).is_none() || self.layer_of(focus).is_some() {
            return false;
        }
        let target = self
            .menu_restore
            .take()
            .filter(|&r| self.contains(r) && self.can_focus(r));
        if target.is_some() {
            self.set_focus(target, FocusSource::Keyboard);
        }
        true
    }

    /// Entrega `event` ao widget com foco e, se ele não usar, aos ancestrais, até a camada em
    /// que ele está. Devolve `true` se alguém usou.
    fn bubble(&mut self, event: &Event) -> bool {
        self.bubble_from(self.focus, event)
    }

    /// Entrega `event` a `start` e, se ele não usar, aos ancestrais, até a camada em que ele
    /// está. Devolve `true` se alguém usou.
    fn bubble_from(&mut self, start: Option<NodeId>, event: &Event) -> bool {
        let mut node = start;
        while let Some(id) = node {
            if self.send(id, event) {
                return true;
            }
            if self.nodes.get(id).is_none_or(|n| n.layer.is_some()) {
                return false;
            }
            node = self.parent(id);
        }
        false
    }

    /// ↑, ↓, Home e End numa camada com [`Layer::arrows`]: foco no widget anterior, no
    /// próximo, no primeiro ou no último dela, sem dar a volta.
    fn focus_arrow(&mut self, layer: NodeId, key: Key) {
        let start = self.position[layer];
        let order: Vec<NodeId> = self.order[start..]
            .iter()
            .copied()
            .take_while(|&id| id == layer || self.is_inside(id, layer))
            .filter(|&id| self.can_focus(id))
            .collect();
        let (Some(&first), Some(&last)) = (order.first(), order.last()) else {
            return;
        };
        let current = self
            .focus
            .and_then(|f| order.iter().position(|&id| id == f));
        let next = match (key, current) {
            (Key::Home, _) => first,
            (Key::End, _) | (Key::Up, None) => last,
            (Key::Up, Some(i)) => order[i.saturating_sub(1)],
            (_, None) => first,
            (_, Some(i)) => order[(i + 1).min(order.len() - 1)],
        };
        self.set_focus(Some(next), FocusSource::Keyboard);
    }

    /// Se `id` está dentro de `ancestor` (sem contar o próprio).
    fn is_inside(&self, id: NodeId, ancestor: NodeId) -> bool {
        let mut node = self.parent(id);
        while let Some(parent) = node {
            if parent == ancestor {
                return true;
            }
            node = self.parent(parent);
        }
        false
    }

    /// Tecla solta, entregue ao widget com foco. O Alt solto sem outra tecla depois dele põe o
    /// foco na barra de menus (ou devolve, se ele já estava lá).
    pub fn key_up(&mut self, key: Key) {
        if key == Key::Alt {
            if std::mem::take(&mut self.alt_alone) {
                self.last_source = FocusSource::Keyboard;
                self.toggle_menu_bar();
            }
            return;
        }
        if let Some(id) = self.focus {
            self.send(id, &Event::KeyUp(key));
        }
    }

    /// Texto digitado, já com o layout do teclado e as teclas mortas aplicados, entregue ao
    /// widget com foco.
    ///
    /// O texto da tecla que rodou um atalho (o `a` do Alt+A) não é digitado.
    pub fn text(&mut self, text: &str) {
        if std::mem::take(&mut self.swallow_text) {
            return;
        }
        if let Some(id) = self.focus {
            self.send(id, &Event::Text(text.to_owned()));
        }
    }

    /// Composição do IME, entregue ao widget com foco.
    pub fn ime(&mut self, ime: Ime) {
        if let Some(id) = self.focus {
            self.send(id, &Event::Ime(ime));
        }
    }

    /// Tab: foco no próximo widget que aceita foco, voltando ao primeiro depois do último.
    pub fn focus_next(&mut self) {
        self.move_focus(false);
    }

    /// Shift+Tab: foco no widget anterior.
    pub fn focus_prev(&mut self) {
        self.move_focus(true);
    }

    fn move_focus(&mut self, backward: bool) {
        let order: Vec<NodeId> = self
            .ids()
            .filter(|&id| Some(id) == self.focus || (self.can_focus(id) && self.tab_stop(id)))
            .collect();
        let next = match self
            .focus
            .and_then(|f| order.iter().position(|&id| id == f))
        {
            Some(i) if backward => order[(i + order.len() - 1) % order.len()],
            Some(i) => order[(i + 1) % order.len()],
            None if backward => match order.last() {
                Some(&id) => id,
                None => return,
            },
            None => match order.first() {
                Some(&id) => id,
                None => return,
            },
        };
        if self.can_focus(next) {
            self.set_focus(Some(next), FocusSource::Keyboard);
        }
    }

    /// Põe o foco em `id` (ou tira, com `None`). Pelo teclado, rola até o widget aparecer e
    /// mostra a dica dele depois do atraso.
    pub fn set_focus(&mut self, id: Option<NodeId>, source: FocusSource) {
        self.focus_to(id, source, source == FocusSource::Keyboard);
    }

    fn focus_to(&mut self, id: Option<NodeId>, source: FocusSource, reveal: bool) {
        if id == self.focus {
            return;
        }
        // O foco que entra numa barra de menus (pelo F10, pelo Alt ou por um clique) volta,
        // com o Esc, para onde estava.
        if let Some(new) = id
            && self.menu_bar_of(new).is_some()
            && self.focus.is_none_or(|old| self.menu_bar_of(old).is_none())
        {
            self.menu_restore = self.focus;
        }
        if let Some(old) = self.focus.take() {
            self.send(old, &Event::FocusOut);
            if self
                .tip
                .is_some_and(|tip| tip.via == TipVia::Focus && tip.owner == old)
            {
                self.close_tip();
            }
        }
        if let Some(new) = id {
            self.focus = Some(new);
            self.send(new, &Event::FocusIn(source));
            if reveal {
                self.bring_into_view(new);
            }
            if source == FocusSource::Keyboard && self.nodes[new].tooltip.is_some() {
                self.close_tip();
                self.tip = Some(Tip {
                    owner: new,
                    via: TipVia::Focus,
                    elapsed: Some(Duration::ZERO),
                    open: false,
                });
            }
        }
        self.sync_within();
    }

    /// Avisa os widgets que [`Widget::watches_within`] de que o ponteiro ou o foco entrou ou saiu
    /// deles. Só chama quando o estado muda.
    fn sync_within(&mut self) {
        for id in self.watchers.clone() {
            if !self.contains(id) {
                continue;
            }
            let pointer = self.hover.is_some_and(|hover| self.is_within(hover, id));
            let focus = self.focus.is_some_and(|focus| self.is_within(focus, id));
            let (was_pointer, was_focus) = self.nodes[id].within;
            self.nodes[id].within = (pointer, focus);
            for change in [
                (pointer != was_pointer).then_some(Within::Pointer(pointer)),
                (focus != was_focus).then_some(Within::Focus(focus)),
            ]
            .into_iter()
            .flatten()
            {
                let widget = Rc::clone(&self.nodes[id].widget);
                untrack(|| widget.borrow_mut().within(change));
                self.dirty.borrow_mut().push(id);
            }
        }
    }

    /// Acompanha as camadas que abriram e fecharam desde a última chamada.
    ///
    /// Uma camada que abriu entra na pilha e recebe o foco (no widget com
    /// [`Widget::autofocus`] ou no primeiro que aceita foco); uma que fechou sai, e o foco volta
    /// para quem o tinha quando ela abriu.
    fn sync_layers(&mut self) {
        self.overlays = self
            .order
            .iter()
            .copied()
            .filter(|&id| {
                self.nodes[id].layer.is_some_and(|layer| {
                    !layer.input && !layer.pointer && layer.placement == Placement::Over
                }) && self.shown(id)
            })
            .collect();
        self.passive = self
            .order
            .iter()
            .copied()
            .filter(|&id| {
                self.nodes[id]
                    .layer
                    .is_some_and(|layer| !layer.input && layer.pointer)
                    && self.shown(id)
            })
            .collect();
        let opened: Vec<NodeId> = self
            .order
            .iter()
            .copied()
            .filter(|&id| {
                self.nodes[id].layer.is_some_and(|layer| layer.input)
                    && !self.layers.iter().any(|open| open.id == id)
                    && self.shown(id)
            })
            .collect();
        if let Some(i) = self
            .layers
            .iter()
            .position(|open| !self.contains(open.id) || !self.shown(open.id))
        {
            let mut restore = self.layers[i].restore;
            // As camadas de dentro da que fechou (os submenus de um menu) fecham junto, para
            // não voltarem abertas com ela.
            let base = self.layers[i].id;
            let nested: Vec<NodeId> = self.layers[i + 1..]
                .iter()
                .map(|open| open.id)
                .filter(|&id| self.contains(id) && self.contains(base) && self.is_within(id, base))
                .collect();
            for id in nested {
                let widget = Rc::clone(&self.nodes[id].widget);
                untrack(|| widget.borrow_mut().set_open(false));
                self.dirty.borrow_mut().push(id);
            }
            let gone: Vec<NodeId> = self.layers[i..].iter().map(|open| open.id).collect();
            self.layers.truncate(i);
            for id in gone.into_iter().filter(|&id| self.contains(id)) {
                let widget = Rc::clone(&self.nodes[id].widget);
                untrack(|| widget.borrow_mut().layer_closed());
            }
            // Um menu da barra que fechou sem o Esc (um item escolhido, um clique fora) devolve
            // o foco a quem tinha antes da barra, e não ao item dela; trocar de menu pelas
            // setas ou pelo hover não conta.
            if !std::mem::take(&mut self.escaping)
                && let Some(r) = restore
                && self.contains(r)
                && self.layer_of(r).is_none()
                && let Some(bar) = self.menu_bar_of(r)
                && !opened.iter().any(|&o| self.is_within(o, bar))
                && let Some(before) = self.menu_restore.take()
            {
                restore = Some(before);
            }
            if self
                .focus
                .is_none_or(|f| !self.contains(f) || !self.can_focus(f))
            {
                let target = restore.filter(|&r| self.contains(r) && self.can_focus(r));
                self.focus_to(target, self.last_source, false);
                self.pending_reveal = target;
            }
        }
        for id in opened {
            self.close_tip();
            // O menu de um item da barra devolve o foco ao item, mesmo depois de trocar de
            // menu pelo ponteiro ou pelas setas.
            let bar_item = self.parent(id).filter(|&owner| {
                self.menu_bar_of(owner)
                    .is_some_and(|bar| self.layer_of(owner) == self.layer_of(bar))
                    && self.can_focus(owner)
            });
            self.layers.push(Open {
                id,
                restore: bar_item.or(self.focus),
            });
            let start = self.position[id];
            let inside: Vec<NodeId> = self.order[start..]
                .iter()
                .copied()
                .take_while(|&n| n == id || self.is_inside(n, id))
                .filter(|&n| self.can_focus(n) && self.tab_stop(n))
                .collect();
            let target = inside
                .iter()
                .copied()
                .find(|&n| untrack(|| self.widget(n).autofocus()))
                .or_else(|| inside.first().copied());
            if target.is_some() || self.focus.is_some_and(|f| !self.can_focus(f)) {
                self.focus_to(target, self.last_source, false);
            }
            self.pending_reveal = target;
        }
    }

    /// Chama [`Widget::placed`] nos widgets que medem a si mesmos, com o retângulo que o layout
    /// deu a cada um. O loop de frame chama depois do layout, junto de
    /// [`Tree::reveal_pending`]; quem mudou entra na fila de repintura.
    pub fn place_widgets(&mut self) {
        for index in 0..self.placers.len() {
            let id = self.placers[index];
            let Some(node) = self.nodes.get(id) else {
                continue;
            };
            let (rect, widget) = (node.rect, Rc::clone(&node.widget));
            let mut cx = EventCx::new(
                rect,
                self.pointer,
                self.modifiers,
                &mut *self.metrics,
                &mut *self.clipboard,
            );
            let changed = untrack(|| widget.borrow_mut().placed(rect, &mut cx));
            if changed {
                self.dirty.borrow_mut().push(id);
            }
        }
    }

    /// Rola até o widget que uma camada focou ao abrir ou fechar. O loop de frame chama depois
    /// do layout, quando a camada já tem posição.
    pub fn reveal_pending(&mut self) {
        if let Some(name) = crate::text_editor::take_focus_request() {
            self.pending_focus = Some((name, FOCUS_LATER_TRIES));
        }
        if let Some((name, tries)) = self.pending_focus.take() {
            match self.find(&name).filter(|&id| self.can_focus(id)) {
                Some(id) => {
                    self.set_focus(Some(id), FocusSource::Keyboard);
                    self.bring_into_view(id);
                }
                None if tries > 1 => self.pending_focus = Some((name, tries - 1)),
                None => {}
            }
        }
        if let Some(name) = crate::text_editor::take_focus_inside_request() {
            self.pending_focus_inside = Some((name, FOCUS_LATER_TRIES));
        }
        if let Some((name, tries)) = self.pending_focus_inside.take() {
            let first = self.find(&name).and_then(|root| {
                self.order[self.position[root] + 1..]
                    .iter()
                    .copied()
                    .take_while(|&n| self.is_inside(n, root))
                    .find(|&n| self.shown(n) && self.can_focus(n) && self.tab_stop(n))
            });
            match first {
                Some(id) => {
                    self.set_focus(Some(id), FocusSource::Keyboard);
                    self.bring_into_view(id);
                }
                None if tries > 1 => self.pending_focus_inside = Some((name, tries - 1)),
                None => {}
            }
        }
        if let Some(id) = self.pending_reveal.take()
            && self.contains(id)
        {
            self.bring_into_view(id);
        }
        if let Some(name) = crate::text_editor::take_reveal_request() {
            self.pending_bring = Some((name, FOCUS_LATER_TRIES));
        }
        if let Some((name, tries)) = self.pending_bring.take() {
            match self.find(&name).filter(|&id| self.shown(id)) {
                Some(id) => self.bring_into_view(id),
                None if tries > 1 => self.pending_bring = Some((name, tries - 1)),
                None => {}
            }
        }
    }

    /// Dono da dica sob `point`: o widget mais de cima, com `.tooltip()`, cuja parte visível
    /// contém o ponteiro.
    fn tip_owner_at(&self, point: Point) -> Option<NodeId> {
        if self.capture.is_some() {
            return None;
        }
        let hit = self.hit(point);
        self.tip_owners.iter().rev().copied().find(|&id| {
            self.visible_rect(id).contains(point)
                && self.shown(id)
                && self.in_scope(id)
                && hit.is_none_or(|hit| self.is_within(hit, id) || self.is_within(id, hit))
        })
    }

    /// O ponteiro está sobre `owner` (ou sobre nenhum dono de dica).
    fn hover_tip(&mut self, owner: Option<NodeId>) {
        match (self.tip, owner) {
            (Some(tip), Some(owner)) if tip.owner == owner => {}
            (Some(tip), None) if tip.via == TipVia::Focus => {}
            (_, owner) => {
                self.close_tip();
                self.tip = owner.map(|owner| Tip {
                    owner,
                    via: TipVia::Hover,
                    elapsed: Some(Duration::ZERO),
                    open: false,
                });
            }
        }
    }

    /// Fecha a dica e para a contagem.
    fn close_tip(&mut self) {
        if let Some(tip) = self.tip.take() {
            self.set_tip_open(tip.owner, false);
        }
    }

    /// Fecha a dica depois de um clique, uma tecla ou a roda: ela só volta quando o ponteiro
    /// sair do dono e entrar de novo.
    fn cancel_tip(&mut self) {
        if let Some(tip) = self.tip {
            self.close_tip();
            if tip.via == TipVia::Hover {
                self.tip = Some(Tip {
                    elapsed: None,
                    ..tip
                });
            }
        }
    }

    /// Põe a dica de `owner` no ponto que o widget pede ([`Widget::tip_anchor`]).
    fn anchor_tip(&mut self, owner: NodeId) {
        let Some(tooltip) = self.nodes.get(owner).and_then(|node| node.tooltip) else {
            return;
        };
        let rect = self.rect(owner);
        let anchor = untrack(|| self.nodes[owner].widget.borrow().tip_anchor(rect));
        if self.nodes[tooltip].anchor != anchor {
            self.nodes[tooltip].anchor = anchor;
            self.dirty.borrow_mut().push(tooltip);
        }
    }

    fn set_tip_open(&mut self, owner: NodeId, open: bool) {
        if open {
            self.anchor_tip(owner);
        }
        if let Some(tooltip) = self.nodes.get(owner).and_then(|node| node.tooltip) {
            let widget = Rc::clone(&self.nodes[tooltip].widget);
            untrack(|| widget.borrow_mut().set_open(open));
            self.dirty.borrow_mut().push(tooltip);
        }
    }

    /// Passa o tempo das animações dos widgets que aparecem ([`Widget::tick`]) e da dica: ela
    /// abre [`TOOLTIP_DELAY`] depois de o ponteiro parar sobre o dono (ou de o foco chegar nele
    /// pelo teclado). O loop de frame chama enquanto há [`Tree::deadline`].
    ///
    /// O toque parado por [`HOLD_DELAY`] sobre um widget com menu de contexto abre o menu, e
    /// o widget que tinha o toque recebe [`Event::PointerCancel`].
    pub fn advance(&mut self, dt: Duration) {
        if let Some(mut hold) = self.hold.take() {
            hold.elapsed += dt;
            if hold.elapsed < HOLD_DELAY {
                self.hold = Some(hold);
            } else if let Some(drag) = self
                .drag
                .as_mut()
                .filter(|drag| drag.source == hold.target && !drag.armed)
            {
                // Num item que arrasta, o toque longo arma o arrasto; o menu de contexto abre se
                // o dedo sair sem andar.
                drag.armed = true;
                self.pan = None;
                if let Some(captured) = self.capture.take() {
                    self.send(captured, &Event::PointerCancel);
                }
            } else if self.contains(hold.target)
                && let Some(owner) = self.context_owner(Some(hold.target), Some(hold.origin))
            {
                self.pan = None;
                if let Some(captured) = self.capture.take() {
                    self.send(captured, &Event::PointerCancel);
                }
                self.open_context(owner, Some(hold.origin));
            }
        }
        for id in self.animators.clone() {
            // O `ticking` vem antes do `shown`: ele é barato, e o `shown` sobe os ancestrais
            // rodando o `.visible()` de cada um, para cada animador parado da árvore.
            if self.contains(id) && self.ticking(id) && self.shown(id) {
                let widget = Rc::clone(&self.nodes[id].widget);
                untrack(|| widget.borrow_mut().tick(dt));
                self.dirty.borrow_mut().push(id);
            }
        }
        self.serve_scroll_links();
        let Some(mut tip) = self.tip else {
            return;
        };
        let Some(elapsed) = tip.elapsed.filter(|_| !tip.open) else {
            return;
        };
        tip.elapsed = Some(elapsed + dt);
        if elapsed + dt >= TOOLTIP_DELAY {
            tip.open = true;
            self.set_tip_open(tip.owner, true);
        }
        self.tip = Some(tip);
    }

    /// Quanto falta para algo mudar sozinho: [`ANIMATION_FRAME`] enquanto um widget que anima
    /// aparece, ou o tempo até a dica em contagem abrir. O loop de frame acorda a tempo.
    pub fn deadline(&self) -> Option<Duration> {
        let animation = self
            .animators
            .iter()
            .any(|&id| self.shown(id) && self.ticking(id))
            .then_some(ANIMATION_FRAME);
        let tip = self
            .tip
            .filter(|tip| !tip.open)
            .and_then(|tip| Some(TOOLTIP_DELAY.saturating_sub(tip.elapsed?)));
        let hold = self
            .hold
            .map(|hold| HOLD_DELAY.saturating_sub(hold.elapsed));
        animation.into_iter().chain(tip).chain(hold).min()
    }

    /// O que o [`Tree::deadline`] espera, em texto: os widgets que animam (tipo e id), a dica e
    /// o toque longo. Para o rastro dos quadros da plataforma (`TOMNI_UI_TRACE_FRAMES`).
    #[doc(hidden)]
    pub fn deadline_reason(&self) -> Option<String> {
        let mut parts: Vec<String> = self
            .animators
            .iter()
            .filter(|&&id| self.shown(id) && self.ticking(id))
            .map(|&id| {
                let kind = untrack(|| self.nodes[id].widget.borrow().kind());
                format!("{kind} {id:?}")
            })
            .collect();
        if self
            .tip
            .is_some_and(|tip| !tip.open && tip.elapsed.is_some())
        {
            parts.push("dica".into());
        }
        if self.hold.is_some() {
            parts.push("toque longo".into());
        }
        (!parts.is_empty()).then(|| parts.join(", "))
    }

    /// Se o tempo passa agora no widget animado `id` ([`Widget::ticking`]).
    fn ticking(&self, id: NodeId) -> bool {
        untrack(|| self.nodes[id].widget.borrow().ticking())
    }

    /// A dica aberta, se houver.
    pub fn open_tooltip(&self) -> Option<NodeId> {
        let tip = self.tip.filter(|tip| tip.open)?;
        self.nodes.get(tip.owner)?.tooltip
    }

    /// Tira o foco de um widget que deixou de aceitar foco (que ficou desabilitado ou oculto,
    /// por exemplo) e cancela o ponteiro sobre um widget que sumiu. Também acompanha as
    /// camadas que abriram e fecharam e a dica de um widget que sumiu. O loop de frame chama
    /// depois de rodar os efeitos.
    pub fn sync_focus(&mut self) {
        self.sync_layers();
        if let Some(tip) = self.tip
            && (!self.contains(tip.owner) || !self.shown(tip.owner) || !self.in_scope(tip.owner))
        {
            self.close_tip();
        }
        if let Some(id) = self.focus
            && !self.can_focus(id)
        {
            self.set_focus(None, FocusSource::Keyboard);
        }
        if let Some(id) = self.capture
            && !self.shown(id)
        {
            self.capture = None;
            self.send(id, &Event::PointerCancel);
        }
        if let Some(id) = self.hover
            && !self.shown(id)
        {
            self.hover = None;
            self.send(id, &Event::PointerCancel);
        }
        self.sync_within();
    }
}

/// Quanto rolar para o trecho `start..end` caber em `view_start..view_end`; se não couber,
/// mostra o começo.
fn reveal(start: f64, end: f64, view_start: f64, view_end: f64) -> f64 {
    if start < view_start {
        start - view_start
    } else if end > view_end {
        (end - view_end).min(start - view_start)
    } else {
        0.0
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        self.scope.clear();
    }
}

impl fmt::Debug for Tree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut list = f.debug_list();
        for id in self.ids() {
            list.entry(&(self.position(id), self.name(id), self.rect(id)));
        }
        list.finish()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use tomni_ui_reactive::prelude::*;

    use super::*;
    use crate::{
        AutoSuggestBox, Button, ButtonState, Column, Edit, For, ProgressBar, RadioButtons, Row,
        ScrollViewer, Show, TabView, Text,
    };

    const MOUSE: PointerKind = PointerKind::Mouse;

    /// O widget na posição `n` da ordem da árvore.
    fn nth(tree: &Tree, n: usize) -> NodeId {
        tree.ids().nth(n).unwrap()
    }

    /// Monta a view num escopo novo, roda o teste e descarta tudo.
    fn with_tree(view: impl FnOnce() -> Box<dyn Widget>, test: impl FnOnce(&mut Tree)) {
        let scope = Scope::root();
        scope.run(|| {
            let mut tree = Tree::mount(view(), &scope);
            run_effects();
            test(&mut tree);
        });
        scope.clear();
    }

    /// Dois botões lado a lado (0..100 e 100..200) dentro de uma coluna.
    fn place(tree: &mut Tree) {
        tree.set_rect(nth(tree, 0), Rect::new(0.0, 0.0, 200.0, 32.0));
        tree.set_rect(nth(tree, 1), Rect::new(0.0, 0.0, 200.0, 32.0));
        tree.set_rect(nth(tree, 2), Rect::new(0.0, 0.0, 100.0, 32.0));
        tree.set_rect(nth(tree, 3), Rect::new(100.0, 0.0, 200.0, 32.0));
    }

    fn state(tree: &Tree, name: &str) -> ButtonState {
        match tree.look(tree.find(name).unwrap()) {
            Look::Button { visual, .. } => visual.state,
            look => panic!("{look:?}"),
        }
    }

    #[test]
    fn monta_em_ordem_de_arvore() {
        with_tree(
            || {
                Box::new(
                    Column::new()
                        .child(Row::new().child(Button::new("A").id("a")))
                        .child(Button::new("B").id("b")),
                )
            },
            |tree| {
                let names: Vec<_> = tree.ids().map(|id| tree.name(id)).collect();
                assert_eq!(names, ["column", "row", "a", "b"]);
                assert_eq!(tree.children(tree.root()), [nth(tree, 1), nth(tree, 3)]);
                assert_eq!(tree.parent(nth(tree, 2)), Some(nth(tree, 1)));
                assert_eq!(tree.take_dirty().len(), 4, "todos começam na fila");
                assert!(tree.take_dirty().is_empty());
            },
        );
    }

    #[test]
    fn efeito_poe_na_fila_so_quem_depende() {
        with_tree(
            || {
                let n = signal(0);
                Box::new(
                    Row::new()
                        .child(Button::new(move || format!("{}", n.get())).id("n"))
                        .child(
                            Button::new("+1")
                                .id("mais")
                                .on_click(move || n.update(|n| *n += 1)),
                        ),
                )
            },
            |tree| {
                tree.set_rect(nth(tree, 2), Rect::new(100.0, 0.0, 200.0, 32.0));
                tree.take_dirty();
                tree.pointer_down(Point::new(150.0, 10.0), MOUSE, 1);
                tree.pointer_up(Point::new(150.0, 10.0), MOUSE);
                assert_eq!(tree.take_dirty(), [nth(tree, 2)], "antes dos efeitos");
                run_effects();
                assert_eq!(tree.take_dirty(), [nth(tree, 1)]);
                assert_eq!(
                    tree.look(nth(tree, 1)),
                    Look::Button {
                        label: "1".into(),
                        visual: Default::default(),
                    }
                );
            },
        );
    }

    #[test]
    fn hover_captura_e_foco_pelo_ponteiro() {
        let clicks = Rc::new(Cell::new(0));
        let counter = Rc::clone(&clicks);
        with_tree(
            move || {
                Box::new(
                    Column::new().child(
                        Row::new()
                            .child(
                                Button::new("A")
                                    .id("a")
                                    .on_click(move || counter.set(counter.get() + 1)),
                            )
                            .child(Button::new("B").id("b")),
                    ),
                )
            },
            |tree| {
                place(tree);
                tree.pointer_move(Point::new(50.0, 10.0), MOUSE);
                assert_eq!(state(tree, "a"), ButtonState::Hover);
                tree.pointer_move(Point::new(150.0, 10.0), MOUSE);
                assert_eq!(state(tree, "a"), ButtonState::Rest);
                assert_eq!(state(tree, "b"), ButtonState::Hover);

                // Pressiona em A, arrasta para B e solta: B não recebe nada e A não clica.
                tree.pointer_down(Point::new(50.0, 10.0), MOUSE, 1);
                assert_eq!(tree.focused(), tree.find("a"));
                tree.pointer_move(Point::new(150.0, 10.0), MOUSE);
                assert_eq!(state(tree, "a"), ButtonState::Rest);
                assert_eq!(
                    state(tree, "b"),
                    ButtonState::Rest,
                    "B recebeu hover na captura"
                );
                tree.pointer_up(Point::new(150.0, 10.0), MOUSE);
                assert_eq!(clicks.get(), 0);
                assert_eq!(state(tree, "b"), ButtonState::Hover);

                tree.pointer_down(Point::new(50.0, 10.0), MOUSE, 1);
                tree.pointer_up(Point::new(50.0, 10.0), MOUSE);
                assert_eq!(clicks.get(), 1);

                tree.pointer_cancel();
                assert_eq!(state(tree, "a"), ButtonState::Rest);
            },
        );
    }

    #[test]
    fn tab_pula_desabilitados_e_da_a_volta() {
        with_tree(
            || {
                Box::new(
                    Column::new()
                        .child(Button::new("A").id("a"))
                        .child(Button::new("B").id("b").disabled(true))
                        .child(Button::new("C").id("c")),
                )
            },
            |tree| {
                let a = tree.find("a");
                let c = tree.find("c");
                tree.key_down(Key::Tab);
                assert_eq!(tree.focused(), a);
                tree.key_down(Key::Tab);
                assert_eq!(tree.focused(), c);
                tree.focus_next();
                assert_eq!(tree.focused(), a);
                tree.focus_prev();
                assert_eq!(tree.focused(), c);
                tree.set_modifiers(Modifiers::SHIFT);
                tree.key_down(Key::Tab);
                assert_eq!(tree.focused(), a, "Shift+Tab volta");
                tree.key_down(Key::Tab);
                tree.set_modifiers(Modifiers::NONE);
                assert_eq!(tree.focused(), c);
                let Look::Button { visual, .. } = tree.look(c.unwrap()) else {
                    unreachable!()
                };
                assert!(visual.focus_ring);
            },
        );
    }

    #[test]
    fn perde_o_foco_ao_ficar_desabilitado() {
        let scope = Scope::root();
        scope.run(|| {
            let off = signal(false);
            let mut tree = Tree::mount(
                Box::new(
                    Column::new()
                        .child(Button::new("A").id("a").disabled(off))
                        .child(Button::new("B").id("b")),
                ),
                &scope,
            );
            run_effects();
            tree.focus_next();
            assert_eq!(tree.focused(), tree.find("a"));

            off.set(true);
            run_effects();
            tree.sync_focus();
            assert_eq!(tree.focused(), None);
            tree.focus_prev();
            assert_eq!(
                tree.focused(),
                tree.find("b"),
                "Shift+Tab pulou o desabilitado"
            );
        });
        scope.clear();
    }

    #[test]
    fn oculto_sai_do_tab_e_do_hit_test() {
        let scope = Scope::root();
        scope.run(|| {
            let mostra = signal(true);
            let mut tree = Tree::mount(
                Box::new(
                    Column::new()
                        .child(Row::new().visible(mostra).child(Button::new("A").id("a")))
                        .child(Button::new("B").id("b")),
                ),
                &scope,
            );
            run_effects();
            tree.set_rect(nth(&tree, 2), Rect::new(0.0, 0.0, 100.0, 32.0));
            tree.set_rect(nth(&tree, 3), Rect::new(0.0, 40.0, 100.0, 72.0));
            let a = tree.find("a").unwrap();
            assert!(tree.shown(a));
            tree.focus_next();
            assert_eq!(tree.focused(), Some(a));
            tree.pointer_move(Point::new(50.0, 10.0), MOUSE);
            tree.take_dirty();

            mostra.set(false);
            run_effects();
            assert_eq!(tree.take_dirty(), [nth(&tree, 1)], "só a linha muda");
            assert!(!tree.shown(a), "o pai oculto esconde o filho");
            tree.sync_focus();
            assert_eq!(tree.focused(), None);
            assert_eq!(
                state(&tree, "a"),
                ButtonState::Rest,
                "o hover foi cancelado"
            );
            assert_eq!(tree.hit(Point::new(50.0, 10.0)), None);
            tree.focus_next();
            assert_eq!(tree.focused(), tree.find("b"));
            tree.focus_next();
            assert_eq!(tree.focused(), tree.find("b"), "o Tab não volta ao oculto");
        });
        scope.clear();
    }

    fn names(tree: &Tree) -> Vec<String> {
        tree.ids().map(|id| tree.name(id)).collect()
    }

    #[test]
    fn for_cria_e_descarta_so_os_itens_que_mudam() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let scope = Scope::root();
        let descartados = Arc::new(AtomicUsize::new(0));
        let contador = Arc::clone(&descartados);
        scope.run(|| {
            let itens = signal(vec![1, 2, 3]);
            let mut tree = Tree::mount(
                Box::new(Column::new().child(For::each(
                    itens,
                    |n| *n,
                    move |n| {
                        let contador = Arc::clone(&contador);
                        on_cleanup(move || {
                            contador.fetch_add(1, Ordering::SeqCst);
                        });
                        Edit::bind(signal(String::new())).id(format!("e{n}"))
                    },
                ))),
                &scope,
            );
            run_effects();
            assert!(tree.reconcile().is_empty(), "a montagem já criou os itens");
            assert_eq!(names(&tree), ["column", "for", "e1", "e2", "e3"]);
            let e2 = tree.find("e2").unwrap();
            tree.set_focus(Some(e2), FocusSource::Keyboard);
            tree.text("oi");
            run_effects();
            tree.take_dirty();

            itens.set(vec![3, 2, 4]);
            run_effects();
            let changes = tree.reconcile();
            assert_eq!((changes.added.len(), changes.removed.len()), (1, 1));
            assert_eq!(changes.parents, [nth(&tree, 1)]);
            assert_eq!(names(&tree), ["column", "for", "e3", "e2", "e4"]);
            assert_eq!(tree.find("e2"), Some(e2), "o id não muda");
            assert_eq!(tree.focused(), Some(e2), "o foco fica");
            assert_eq!(
                tree.widget(e2).text().as_deref(),
                Some("oi"),
                "o texto fica"
            );
            assert_eq!(descartados.load(Ordering::SeqCst), 1, "só o e1 saiu");

            run_effects();
            let e4 = tree.find("e4").unwrap();
            let dirty = tree.take_dirty();
            assert!(dirty.contains(&e4));
            assert!(!dirty.contains(&e2), "quem ficou não é repintado");

            itens.set(Vec::new());
            run_effects();
            assert_eq!(tree.reconcile().removed.len(), 3);
            assert_eq!(tree.focused(), None, "o focado saiu");
            assert_eq!(names(&tree), ["column", "for"]);
        });
        scope.clear();
        assert_eq!(descartados.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn show_troca_a_view() {
        let scope = Scope::root();
        scope.run(|| {
            let logado = signal(false);
            let mut tree = Tree::mount(
                Box::new(
                    Column::new().child(
                        Show::when(logado, || Button::new("Sair").id("sair"))
                            .otherwise(|| Text::new("Entre").id("aviso")),
                    ),
                ),
                &scope,
            );
            run_effects();
            assert_eq!(names(&tree), ["column", "show", "aviso"]);
            logado.set(true);
            run_effects();
            let changes = tree.reconcile();
            assert_eq!((changes.added.len(), changes.removed.len()), (1, 1));
            assert_eq!(names(&tree), ["column", "show", "sair"]);
            run_effects();
            assert!(tree.reconcile().is_empty());
        });
        scope.clear();
    }

    /// Coluna com um `ScrollViewer` de 200×100 e dez botões de 40 px dentro dele.
    fn rolavel(cliques: Rc<Cell<u32>>) -> impl FnOnce() -> Box<dyn Widget> {
        move || {
            let mut lista = Column::new();
            for i in 0..10 {
                let cliques = Rc::clone(&cliques);
                lista = lista.child(
                    Button::new(format!("{i}"))
                        .id(format!("b{i}"))
                        .on_click(move || cliques.set(cliques.get() + 1)),
                );
            }
            Box::new(Column::new().child(ScrollViewer::new().id("sv").child(lista)))
        }
    }

    fn place_rolavel(tree: &mut Tree) -> NodeId {
        let sv = tree.find("sv").unwrap();
        tree.set_rect(tree.root(), Rect::new(0.0, 0.0, 200.0, 300.0));
        tree.set_rect(sv, Rect::new(0.0, 0.0, 200.0, 100.0));
        tree.set_rect(nth(tree, 2), Rect::new(0.0, 0.0, 200.0, 400.0));
        for i in 0..10 {
            let b = tree.find(&format!("b{i}")).unwrap();
            let y = f64::from(i) * 40.0;
            tree.set_rect(b, Rect::new(0.0, y, 200.0, y + 40.0));
        }
        tree.set_scroll_extent(sv, Size::new(200.0, 400.0), Size::new(200.0, 100.0));
        sv
    }

    fn offset(tree: &Tree, sv: NodeId) -> f64 {
        tree.scroller(sv).unwrap().offset().y
    }

    #[test]
    fn rolagem_move_o_hit_test_e_corta() {
        with_tree(rolavel(Rc::default()), |tree| {
            let sv = place_rolavel(tree);
            let at = Point::new(50.0, 50.0);
            assert_eq!(tree.hit(at), tree.find("b1"));
            assert_eq!(
                tree.hit(Point::new(194.0, 50.0)),
                Some(sv),
                "a barra fica por cima"
            );
            assert_eq!(
                tree.hit(Point::new(50.0, 150.0)),
                None,
                "fora da área visível"
            );

            assert!(tree.wheel(at, Vec2::new(0.0, 100.0)));
            assert_eq!(offset(tree, sv), 100.0);
            let b3 = tree.find("b3").unwrap();
            assert_eq!(tree.hit(at), Some(b3));
            assert_eq!(tree.rect(b3), Rect::new(0.0, 20.0, 200.0, 60.0));
            assert_eq!(tree.layout_rect(b3), Rect::new(0.0, 120.0, 200.0, 160.0));
            assert_eq!(
                state(tree, "b3"),
                ButtonState::Hover,
                "o hover segue o conteúdo"
            );
            let b0 = tree.find("b0").unwrap();
            assert!(tree.visible_rect(b0).is_zero_area(), "rolou para fora");

            assert!(tree.wheel(at, Vec2::new(0.0, 1000.0)));
            assert_eq!(offset(tree, sv), 300.0);
            assert!(!tree.wheel(at, Vec2::new(0.0, 10.0)), "já está no fim");
            assert!(
                !tree.wheel(Point::new(50.0, 200.0), Vec2::new(0.0, -10.0)),
                "fora do sv"
            );
        });
    }

    #[test]
    fn foco_pelo_teclado_rola_ate_o_widget() {
        with_tree(rolavel(Rc::default()), |tree| {
            let sv = place_rolavel(tree);
            tree.set_focus(tree.find("b9"), FocusSource::Keyboard);
            assert_eq!(offset(tree, sv), 300.0);
            tree.set_focus(tree.find("b8"), FocusSource::Keyboard);
            assert_eq!(offset(tree, sv), 300.0, "o b8 já aparece");
            tree.focus_prev();
            tree.focus_prev();
            assert_eq!(tree.focused(), tree.find("b6"));
            assert_eq!(
                offset(tree, sv),
                236.0,
                "mostra o b6 com a folga do anel de foco"
            );
            tree.set_focus(tree.find("b0"), FocusSource::Pointer);
            assert_eq!(offset(tree, sv), 236.0, "o clique não rola");
        });
    }

    #[test]
    fn teclas_de_rolagem() {
        with_tree(rolavel(Rc::default()), |tree| {
            let sv = place_rolavel(tree);
            tree.pointer_move(Point::new(50.0, 50.0), MOUSE);
            tree.key_down(Key::PageDown);
            assert_eq!(offset(tree, sv), 100.0);
            tree.key_down(Key::End);
            assert_eq!(offset(tree, sv), 300.0);
            tree.key_down(Key::Home);
            assert_eq!(offset(tree, sv), 0.0);
            tree.key_down(Key::Down);
            assert_eq!(offset(tree, sv), 16.0);

            // O botão com foco não usa as setas: elas sobem para o `ScrollViewer`.
            tree.set_focus(tree.find("b1"), FocusSource::Pointer);
            tree.key_down(Key::Down);
            assert_eq!(offset(tree, sv), 32.0);
            tree.key_down(Key::Up);
            tree.key_down(Key::Up);
            tree.key_down(Key::Up);
            assert_eq!(offset(tree, sv), 0.0);
        });
    }

    #[test]
    fn arrastar_o_dedo_rola_sem_clicar() {
        let cliques = Rc::new(Cell::new(0));
        with_tree(rolavel(Rc::clone(&cliques)), |tree| {
            let sv = place_rolavel(tree);
            let touch = PointerKind::Touch;
            tree.pointer_down(Point::new(50.0, 50.0), touch, 1);
            assert_eq!(state(tree, "b1"), ButtonState::Pressed);
            tree.pointer_move(Point::new(50.0, 45.0), touch);
            assert_eq!(offset(tree, sv), 0.0, "ainda na folga");
            tree.pointer_move(Point::new(50.0, 20.0), touch);
            assert_eq!(offset(tree, sv), 30.0);
            assert_eq!(
                state(tree, "b1"),
                ButtonState::Rest,
                "o toque foi cancelado"
            );
            tree.pointer_up(Point::new(50.0, 20.0), touch);
            assert_eq!(cliques.get(), 0);

            tree.pointer_down(Point::new(50.0, 50.0), touch, 1);
            tree.pointer_up(Point::new(50.0, 52.0), touch);
            assert_eq!(cliques.get(), 1, "um toque parado clica");
        });
    }

    /// Põe `rect` no widget com o id `name`.
    fn put(tree: &mut Tree, name: &str, rect: Rect) {
        let id = tree.find(name).unwrap();
        tree.set_rect(id, rect);
    }

    /// Roda os efeitos e acompanha as camadas, como o loop de frame.
    fn frame(tree: &mut Tree) {
        run_effects();
        tree.sync_focus();
    }

    fn focused(tree: &Tree) -> Option<String> {
        tree.focused().map(|id| tree.name(id))
    }

    fn click(tree: &mut Tree, point: Point) {
        tree.pointer_down(point, MOUSE, 1);
        tree.pointer_up(point, MOUSE);
        frame(tree);
    }

    /// Botão "mais" com um menu de dois itens e um separador, e um botão "outro".
    fn com_menu(escolhas: Rc<RefCell<Vec<&'static str>>>) -> impl FnOnce() -> Box<dyn Widget> {
        move || {
            let (a, b) = (Rc::clone(&escolhas), escolhas);
            Box::new(
                Column::new()
                    .child(
                        Button::new("Mais").id("mais").flyout(
                            crate::MenuFlyout::new()
                                .id("menu")
                                .item("sair", "Sair", move || a.borrow_mut().push("sair"))
                                .separator()
                                .item("excluir", "Excluir", move || {
                                    b.borrow_mut().push("excluir");
                                }),
                        ),
                    )
                    .child(Button::new("Outro").id("outro")),
            )
        }
    }

    fn place_menu(tree: &mut Tree) {
        put(tree, "mais", Rect::new(0.0, 0.0, 100.0, 44.0));
        put(tree, "outro", Rect::new(0.0, 100.0, 100.0, 144.0));
        put(tree, "menu", Rect::new(0.0, 48.0, 120.0, 140.0));
        put(tree, "sair", Rect::new(0.0, 50.0, 120.0, 94.0));
        put(tree, "excluir", Rect::new(0.0, 96.0, 120.0, 140.0));
    }

    #[test]
    fn menu_abre_com_foco_no_primeiro_item_e_volta_ao_fechar() {
        let escolhas = Rc::default();
        with_tree(com_menu(Rc::clone(&escolhas)), |tree| {
            place_menu(tree);
            let menu = tree.find("menu").unwrap();
            assert!(!tree.shown(menu));
            tree.set_focus(tree.find("mais"), FocusSource::Keyboard);
            tree.key_down(Key::Enter);
            frame(tree);
            assert!(tree.shown(menu));
            assert_eq!(tree.open_layers(), [menu]);
            assert_eq!(focused(tree).as_deref(), Some("sair"));

            tree.key_down(Key::Down);
            assert_eq!(focused(tree).as_deref(), Some("excluir"));
            tree.key_down(Key::Down);
            assert_eq!(focused(tree).as_deref(), Some("excluir"), "não dá a volta");
            tree.key_down(Key::Home);
            assert_eq!(focused(tree).as_deref(), Some("sair"));

            tree.key_down(Key::Escape);
            frame(tree);
            assert!(!tree.shown(menu));
            assert!(tree.open_layers().is_empty());
            assert_eq!(
                focused(tree).as_deref(),
                Some("mais"),
                "o foco volta à origem"
            );
            assert!(escolhas.borrow().is_empty());

            tree.key_down(Key::Enter);
            frame(tree);
            tree.key_down(Key::End);
            tree.key_down(Key::Enter);
            frame(tree);
            assert_eq!(*escolhas.borrow(), ["excluir"]);
            assert!(!tree.shown(menu), "escolher fecha o menu");
            assert_eq!(focused(tree).as_deref(), Some("mais"));
        });
    }

    #[test]
    fn clique_fora_fecha_sem_chegar_a_ninguem() {
        let escolhas = Rc::default();
        with_tree(com_menu(Rc::clone(&escolhas)), |tree| {
            place_menu(tree);
            click(tree, Point::new(50.0, 20.0));
            let menu = tree.find("menu").unwrap();
            assert!(tree.shown(menu));
            assert_eq!(focused(tree).as_deref(), Some("sair"));
            assert_eq!(
                tree.hit(Point::new(50.0, 120.0)),
                tree.find("excluir"),
                "o menu fica por cima do botão de baixo"
            );
            assert_eq!(tree.hit(Point::new(50.0, 20.0)), None, "fora do menu");

            // O botão "outro" está fora do menu: o clique só fecha.
            tree.pointer_down(Point::new(150.0, 120.0), MOUSE, 1);
            assert!(!tree.shown(menu));
            tree.pointer_up(Point::new(150.0, 120.0), MOUSE);
            frame(tree);
            assert_eq!(focused(tree).as_deref(), Some("mais"));

            click(tree, Point::new(50.0, 20.0));
            click(tree, Point::new(50.0, 70.0));
            assert_eq!(*escolhas.borrow(), ["sair"]);
            assert!(!tree.shown(menu));
        });
    }

    #[test]
    fn atalho_vale_enquanto_aparece_e_o_campo_com_foco_vem_antes() {
        let novos = Rc::new(Cell::new(0));
        let copias = Rc::new(Cell::new(0));
        let mostra = Rc::new(Cell::new(None));
        let (n, c, m) = (Rc::clone(&novos), Rc::clone(&copias), Rc::clone(&mostra));
        with_tree(
            move || {
                let visivel = signal(true);
                m.set(Some(visivel));
                Box::new(
                    Column::new()
                        .child(
                            Button::new("Novo")
                                .id("novo")
                                .visible(visivel)
                                .accelerator(Shortcut::ctrl('N'), move || n.set(n.get() + 1)),
                        )
                        .child(
                            Button::new("Copiar")
                                .id("copiar")
                                .accelerator(Shortcut::ctrl('c'), move || c.set(c.get() + 1)),
                        )
                        .child(Edit::bind(signal(String::new())).id("campo")),
                )
            },
            |tree| {
                tree.set_modifiers(Modifiers::CTRL);
                tree.key_down(Key::Char('n'));
                tree.text("n");
                assert_eq!(novos.get(), 1, "sem foco em ninguém, o atalho roda");

                tree.set_modifiers(Modifiers::CTRL | Modifiers::SHIFT);
                tree.key_down(Key::Char('n'));
                assert_eq!(novos.get(), 1, "o Shift conta");

                tree.set_modifiers(Modifiers::CTRL);
                mostra.get().unwrap().set(false);
                frame(tree);
                tree.key_down(Key::Char('n'));
                assert_eq!(novos.get(), 1, "escondido, não vale");

                tree.key_down(Key::Char('c'));
                assert_eq!(copias.get(), 1);
                tree.set_focus(tree.find("campo"), FocusSource::Keyboard);
                tree.key_down(Key::Char('c'));
                assert_eq!(copias.get(), 1, "o Ctrl+C fica com o campo");
            },
        );
    }

    #[test]
    fn atalho_de_um_signal_vale_na_hora_da_tecla() {
        let acionados = Rc::new(Cell::new(0));
        let atalho = Rc::new(Cell::new(None));
        let (a, s) = (Rc::clone(&acionados), Rc::clone(&atalho));
        with_tree(
            move || {
                let escolhido = signal(Shortcut::ctrl('N'));
                s.set(Some(escolhido));
                let a = Rc::clone(&a);
                Box::new(
                    Column::new().child(
                        Button::new("Novo")
                            .id("novo")
                            .accelerator(escolhido, move || a.set(a.get() + 1)),
                    ),
                )
            },
            |tree| {
                tree.set_modifiers(Modifiers::CTRL);
                tree.key_down(Key::Char('n'));
                assert_eq!(acionados.get(), 1);

                atalho.get().unwrap().set(Shortcut::ctrl('M'));
                tree.key_down(Key::Char('n'));
                assert_eq!(acionados.get(), 1, "o atalho antigo deixou de valer");
                tree.key_down(Key::Char('m'));
                assert_eq!(acionados.get(), 2, "o novo vale sem montar a tela de novo");
            },
        );
    }

    #[test]
    fn menu_de_contexto_no_ponteiro_e_junto_do_widget() {
        let escolhas = Rc::new(RefCell::new(Vec::new()));
        let e = Rc::clone(&escolhas);
        with_tree(
            move || {
                Box::new(
                    Column::new()
                        .child(
                            Button::new("Lista").id("lista").context_flyout(
                                crate::MenuFlyout::new().id("ctx").item(
                                    "abrir",
                                    "Abrir",
                                    move || e.borrow_mut().push("abrir"),
                                ),
                            ),
                        )
                        .child(Button::new("Outro").id("outro")),
                )
            },
            |tree| {
                put(tree, "lista", Rect::new(0.0, 0.0, 200.0, 100.0));
                put(tree, "outro", Rect::new(0.0, 200.0, 200.0, 244.0));
                let lista = tree.find("lista").unwrap();
                let ctx = tree.context_flyout(lista).unwrap();
                assert_eq!(tree.find("ctx"), Some(ctx));
                assert!(!tree.shown(ctx));

                tree.secondary_down(Point::new(30.0, 40.0));
                frame(tree);
                assert!(tree.shown(ctx));
                assert_eq!(tree.anchor(ctx), Some(Point::new(30.0, 40.0)));
                assert_eq!(focused(tree).as_deref(), Some("abrir"));
                assert_eq!(
                    tree.last_source,
                    FocusSource::Pointer,
                    "aberto pelo mouse, sem o anel de foco"
                );
                tree.key_down(Key::Escape);
                frame(tree);
                assert!(!tree.shown(ctx));

                tree.secondary_down(Point::new(30.0, 220.0));
                frame(tree);
                assert!(!tree.shown(ctx), "fora do dono, não abre");

                tree.set_focus(Some(lista), FocusSource::Keyboard);
                tree.set_modifiers(Modifiers::SHIFT);
                tree.key_down(Key::F(10));
                frame(tree);
                assert!(tree.shown(ctx), "Shift+F10 abre");
                assert_eq!(tree.anchor(ctx), None, "junto do widget");
                tree.set_modifiers(Modifiers::NONE);
                tree.key_down(Key::Enter);
                frame(tree);
                assert_eq!(*escolhas.borrow(), ["abrir"]);
                assert!(!tree.shown(ctx));
                assert_eq!(focused(tree).as_deref(), Some("lista"));

                tree.key_down(Key::ContextMenu);
                frame(tree);
                assert!(tree.shown(ctx), "a tecla Menu abre");
                tree.key_down(Key::Escape);
                frame(tree);

                let touch = PointerKind::Touch;
                tree.pointer_down(Point::new(50.0, 50.0), touch, 1);
                tree.advance(HOLD_DELAY / 2);
                assert!(!tree.shown(ctx));
                tree.advance(HOLD_DELAY);
                frame(tree);
                assert!(tree.shown(ctx), "o toque longo abre");
                assert_eq!(tree.anchor(ctx), Some(Point::new(50.0, 50.0)));
                tree.pointer_up(Point::new(50.0, 50.0), touch);
                frame(tree);
                assert!(tree.shown(ctx), "soltar o dedo não fecha");
            },
        );
    }

    #[test]
    fn cada_aba_tem_o_proprio_menu_de_contexto() {
        let escolhas: Rc<RefCell<Vec<String>>> = Rc::default();
        let e = Rc::clone(&escolhas);
        with_tree(
            move || {
                Box::new(
                    crate::TabView::bind(signal("a".to_string()))
                        .id("abas")
                        .tab("a", "A", Text::new("a"))
                        .tab("b", "B", Text::new("b"))
                        .tab_context_flyout(move |chave| {
                            let e = Rc::clone(&e);
                            let k = chave.clone();
                            crate::MenuFlyout::new().id(format!("menu-{chave}")).item(
                                format!("duplicar-{chave}"),
                                "Duplicar",
                                move || e.borrow_mut().push(k.clone()),
                            )
                        }),
                )
            },
            |tree| {
                put(tree, "abas", Rect::new(0.0, 0.0, 400.0, 300.0));
                put(tree, "abas-abas", Rect::new(8.0, 8.0, 392.0, 52.0));
                put(tree, "abas-a", Rect::new(8.0, 8.0, 108.0, 52.0));
                put(tree, "abas-b", Rect::new(108.0, 8.0, 208.0, 52.0));
                let b = tree.find("abas-b").unwrap();
                let menu_b = tree.find("menu-b").unwrap();
                assert_eq!(tree.context_flyout(b), Some(menu_b));

                tree.secondary_down(Point::new(150.0, 30.0));
                frame(tree);
                assert!(tree.shown(menu_b));
                assert!(!tree.shown(tree.find("menu-a").unwrap()));
                tree.key_down(Key::Enter);
                frame(tree);
                assert_eq!(*escolhas.borrow(), ["b"]);
            },
        );
    }

    #[test]
    fn atalho_de_item_de_menu_vale_com_o_menu_fechado() {
        let escolhas = Rc::new(RefCell::new(Vec::new()));
        let e = Rc::clone(&escolhas);
        with_tree(
            move || {
                Box::new(
                    Column::new().child(
                        Button::new("Mais").id("mais").flyout(
                            crate::MenuFlyout::new()
                                .id("menu")
                                .item("sair", "Sair", || {})
                                .accelerator(Shortcut::ctrl('q'), move || {
                                    e.borrow_mut().push("sair")
                                }),
                        ),
                    ),
                )
            },
            |tree| {
                let menu = tree.find("menu").unwrap();
                assert!(tree.accelerator_active(menu));
                tree.set_modifiers(Modifiers::CTRL);
                tree.key_down(Key::Char('q'));
                assert_eq!(*escolhas.borrow(), ["sair"]);
                assert!(!tree.shown(menu));
            },
        );
    }

    /// Botão "mais" com um menu: "novo", o submenu "exportar" (com "csv" e "json") e o item
    /// de marcar "quebra".
    fn com_submenu(
        escolhas: Rc<RefCell<Vec<&'static str>>>,
        quebra: Rc<Cell<Option<Signal<bool>>>>,
    ) -> impl FnOnce() -> Box<dyn Widget> {
        move || {
            let (a, b, c) = (Rc::clone(&escolhas), Rc::clone(&escolhas), escolhas);
            let marcado = signal(false);
            quebra.set(Some(marcado));
            Box::new(
                Column::new()
                    .child(
                        Button::new("Mais").id("mais").flyout(
                            crate::MenuFlyout::new()
                                .id("menu")
                                .item("novo", "Novo", move || a.borrow_mut().push("novo"))
                                .submenu(
                                    "exportar",
                                    "Exportar",
                                    crate::MenuFlyout::new()
                                        .id("exportar-menu")
                                        .item("csv", "CSV", move || b.borrow_mut().push("csv"))
                                        .item("json", "JSON", move || c.borrow_mut().push("json")),
                                )
                                .toggle("quebra", "Quebrar linhas", marcado),
                        ),
                    )
                    .child(Button::new("Outro").id("outro")),
            )
        }
    }

    fn place_submenu(tree: &mut Tree) {
        put(tree, "mais", Rect::new(0.0, 0.0, 100.0, 44.0));
        put(tree, "outro", Rect::new(0.0, 300.0, 100.0, 344.0));
        put(tree, "menu", Rect::new(0.0, 48.0, 160.0, 186.0));
        put(tree, "novo", Rect::new(1.0, 51.0, 159.0, 95.0));
        put(tree, "exportar", Rect::new(1.0, 95.0, 159.0, 139.0));
        put(tree, "quebra", Rect::new(1.0, 139.0, 159.0, 183.0));
        put(tree, "exportar-menu", Rect::new(160.0, 92.0, 280.0, 186.0));
        put(tree, "csv", Rect::new(161.0, 95.0, 279.0, 139.0));
        put(tree, "json", Rect::new(161.0, 139.0, 279.0, 183.0));
    }

    #[test]
    fn submenu_abre_com_a_seta_e_fecha_com_a_outra() {
        let escolhas = Rc::default();
        let quebra = Rc::default();
        with_tree(
            com_submenu(Rc::clone(&escolhas), Rc::clone(&quebra)),
            |tree| {
                place_submenu(tree);
                let sub = tree.find("exportar-menu").unwrap();
                tree.set_focus(tree.find("mais"), FocusSource::Keyboard);
                tree.key_down(Key::Enter);
                frame(tree);
                tree.key_down(Key::Down);
                assert_eq!(focused(tree).as_deref(), Some("exportar"));
                tree.key_down(Key::Right);
                frame(tree);
                assert!(tree.shown(sub));
                assert_eq!(focused(tree).as_deref(), Some("csv"));
                tree.key_down(Key::Down);
                assert_eq!(
                    focused(tree).as_deref(),
                    Some("json"),
                    "as setas ficam no submenu"
                );

                tree.key_down(Key::Left);
                frame(tree);
                assert!(!tree.shown(sub));
                assert!(
                    tree.shown(tree.find("menu").unwrap()),
                    "o menu de cima fica"
                );
                assert_eq!(focused(tree).as_deref(), Some("exportar"));

                tree.key_down(Key::Enter);
                frame(tree);
                assert!(tree.shown(sub), "Enter também abre");
                tree.key_down(Key::Escape);
                frame(tree);
                assert!(!tree.shown(sub), "Esc fecha só o submenu");
                assert_eq!(focused(tree).as_deref(), Some("exportar"));

                tree.key_down(Key::Right);
                frame(tree);
                tree.key_down(Key::End);
                tree.key_down(Key::Enter);
                frame(tree);
                assert_eq!(*escolhas.borrow(), ["json"]);
                assert!(
                    !tree.shown(tree.find("menu").unwrap()),
                    "escolher fecha tudo"
                );
                assert_eq!(focused(tree).as_deref(), Some("mais"));

                tree.key_down(Key::Enter);
                frame(tree);
                assert!(!tree.shown(sub), "reabrir o menu não reabre o submenu");
            },
        );
    }

    #[test]
    fn submenu_abre_com_o_ponteiro_parado_e_fecha_no_clique_fora() {
        let escolhas = Rc::default();
        let quebra: Rc<Cell<Option<Signal<bool>>>> = Rc::default();
        with_tree(
            com_submenu(Rc::clone(&escolhas), Rc::clone(&quebra)),
            |tree| {
                place_submenu(tree);
                let menu = tree.find("menu").unwrap();
                let sub = tree.find("exportar-menu").unwrap();
                click(tree, Point::new(50.0, 20.0));
                assert!(tree.shown(menu));

                tree.pointer_move(Point::new(50.0, 110.0), MOUSE);
                tree.advance(Duration::from_millis(300));
                frame(tree);
                assert!(!tree.shown(sub), "antes dos 400 ms");
                tree.advance(Duration::from_millis(100));
                frame(tree);
                assert!(tree.shown(sub), "parado 400 ms sobre o item");

                assert_eq!(
                    tree.hit(Point::new(50.0, 70.0)),
                    tree.find("novo"),
                    "o ponteiro chega ao menu de baixo"
                );
                tree.pointer_move(Point::new(50.0, 70.0), MOUSE);
                tree.advance(Duration::from_millis(400));
                frame(tree);
                assert!(!tree.shown(sub), "parado sobre outro item, fecha");
                assert!(tree.shown(menu));

                click(tree, Point::new(50.0, 110.0));
                assert!(tree.shown(sub), "o clique abre");
                click(tree, Point::new(200.0, 110.0));
                assert_eq!(*escolhas.borrow(), ["csv"]);
                assert!(!tree.shown(menu));

                click(tree, Point::new(50.0, 20.0));
                click(tree, Point::new(50.0, 110.0));
                assert!(tree.shown(sub));
                click(tree, Point::new(400.0, 400.0));
                assert!(!tree.shown(sub));
                assert!(!tree.shown(menu), "o clique fora fecha tudo");

                click(tree, Point::new(50.0, 20.0));
                click(tree, Point::new(50.0, 160.0));
                assert!(
                    quebra.get().unwrap().get_untracked(),
                    "o item de marcar troca o signal"
                );
                assert!(!tree.shown(menu));
            },
        );
    }

    #[test]
    fn dialogo_prende_o_tab_e_enter_confirma() {
        let excluiu = Rc::new(Cell::new(false));
        let aberto = Rc::new(Cell::new(None));
        let (e, a) = (Rc::clone(&excluiu), Rc::clone(&aberto));
        with_tree(
            move || {
                let open = signal(false);
                a.set(Some(open));
                Box::new(
                    Column::new()
                        .child(Button::new("Excluir conta").id("abrir"))
                        .child(
                            crate::ContentDialog::new("Excluir conta?", open)
                                .id("dialogo")
                                .content(Edit::bind(signal(String::new())).id("motivo"))
                                .primary("Excluir", move || e.set(true))
                                .close("Cancelar"),
                        ),
                )
            },
            |tree| {
                let open = aberto.get().unwrap();
                tree.set_focus(tree.find("abrir"), FocusSource::Keyboard);
                open.set(true);
                frame(tree);
                assert_eq!(focused(tree).as_deref(), Some("dialogo-primary"));
                let mut vistos = Vec::new();
                for _ in 0..4 {
                    tree.key_down(Key::Tab);
                    vistos.push(focused(tree).unwrap());
                }
                assert_eq!(
                    vistos,
                    [
                        "dialogo-close",
                        "motivo",
                        "dialogo-primary",
                        "dialogo-close"
                    ]
                );
                // O Enter que o campo não usa vai para o diálogo.
                tree.set_focus(tree.find("motivo"), FocusSource::Keyboard);
                tree.key_down(Key::Enter);
                frame(tree);
                assert!(excluiu.get());
                assert!(!open.get_untracked());
                assert_eq!(focused(tree).as_deref(), Some("abrir"));
            },
        );
    }

    #[test]
    fn dialogo_modal_ignora_clique_fora_e_fecha_com_esc() {
        let aberto = Rc::new(Cell::new(None));
        let a = Rc::clone(&aberto);
        with_tree(
            move || {
                let open = signal(true);
                a.set(Some(open));
                Box::new(
                    Column::new().child(Button::new("Atrás").id("atras")).child(
                        crate::ContentDialog::new("Aviso", open)
                            .id("aviso")
                            .close("OK"),
                    ),
                )
            },
            |tree| {
                let open = aberto.get().unwrap();
                put(tree, "atras", Rect::new(0.0, 0.0, 100.0, 44.0));
                put(tree, "aviso", Rect::new(200.0, 200.0, 520.0, 384.0));
                put(tree, "aviso-close", Rect::new(220.0, 310.0, 500.0, 354.0));
                frame(tree);
                click(tree, Point::new(50.0, 20.0));
                assert!(open.get_untracked(), "modal não fecha com o clique fora");
                assert_eq!(focused(tree).as_deref(), Some("aviso-close"));
                tree.key_down(Key::Escape);
                frame(tree);
                assert!(!open.get_untracked());
            },
        );
    }

    #[test]
    fn combo_troca_com_setas_fechado_e_abre_no_selecionado() {
        let valor = Rc::new(Cell::new(None));
        let v = Rc::clone(&valor);
        with_tree(
            move || {
                let uf = signal("RJ".to_owned());
                v.set(Some(uf));
                Box::new(
                    Column::new().child(crate::ComboBox::bind(uf, ["MG", "RJ", "SP"]).id("uf")),
                )
            },
            |tree| {
                let uf = valor.get().unwrap();
                tree.set_focus(tree.find("uf"), FocusSource::Keyboard);
                tree.key_down(Key::Down);
                assert_eq!(uf.get_untracked(), "SP");
                tree.key_down(Key::Down);
                assert_eq!(uf.get_untracked(), "SP", "para na última");
                tree.key_down(Key::Home);
                assert_eq!(uf.get_untracked(), "MG");

                tree.set_modifiers(Modifiers {
                    alt: true,
                    ..Modifiers::NONE
                });
                tree.key_down(Key::Down);
                tree.set_modifiers(Modifiers::NONE);
                frame(tree);
                assert!(tree.shown(tree.find("uf-lista").unwrap()));
                assert_eq!(focused(tree).as_deref(), Some("uf-mg"), "foco no escolhido");
                tree.key_down(Key::Down);
                tree.key_down(Key::Escape);
                frame(tree);
                assert_eq!(uf.get_untracked(), "MG", "Esc não muda");
                assert_eq!(focused(tree).as_deref(), Some("uf"));

                tree.key_down(Key::Space);
                tree.key_up(Key::Space);
                frame(tree);
                tree.key_down(Key::Down);
                tree.key_down(Key::Enter);
                frame(tree);
                assert_eq!(uf.get_untracked(), "RJ");
                assert!(!tree.shown(tree.find("uf-lista").unwrap()));
                assert_eq!(focused(tree).as_deref(), Some("uf"));
            },
        );
    }

    /// Uma sobreposição ([`Placement::Over`]): camada sem entrada nem ponteiro.
    struct Selo {
        common: crate::widget::Common,
    }

    impl Selo {
        fn new(id: &str, visible: Signal<bool>) -> Self {
            Self {
                common: crate::widget::Common {
                    id: Some(id.to_owned()),
                    visible: Some(crate::Prop::Dynamic(Rc::new(move || visible.get()))),
                    ..Default::default()
                },
            }
        }
    }

    impl Widget for Selo {
        fn kind(&self) -> &'static str {
            "selo"
        }

        fn common(&self) -> &crate::widget::Common {
            &self.common
        }

        fn look(&self) -> Look {
            Look::Column
        }

        fn layer(&self) -> Option<Layer> {
            Some(Layer {
                light_dismiss: false,
                input: false,
                placement: Placement::Over,
                ..Layer::default()
            })
        }
    }

    #[test]
    fn sobreposicao_pinta_antes_do_menu_e_nao_recebe_ponteiro_nem_foco() {
        let escolhas = Rc::new(RefCell::new(Vec::new()));
        let mostra = signal(false);
        let e = Rc::clone(&escolhas);
        with_tree(
            move || {
                let e = Rc::clone(&e);
                Box::new(
                    Column::new()
                        .child(
                            Button::new("Mais").id("mais").flyout(
                                crate::MenuFlyout::new().id("menu").item(
                                    "sair",
                                    "Sair",
                                    move || e.borrow_mut().push("sair"),
                                ),
                            ),
                        )
                        .child(Button::new("Outro").id("outro"))
                        .child(Selo::new("selo", mostra)),
                )
            },
            |tree| {
                put(tree, "mais", Rect::new(0.0, 0.0, 100.0, 44.0));
                put(tree, "outro", Rect::new(0.0, 100.0, 100.0, 144.0));
                put(tree, "menu", Rect::new(0.0, 48.0, 120.0, 92.0));
                put(tree, "sair", Rect::new(0.0, 50.0, 120.0, 90.0));
                put(tree, "selo", Rect::new(20.0, 100.0, 80.0, 140.0));
                let selo = tree.find("selo").unwrap();
                assert!(tree.open_layers().is_empty(), "fechada não pinta");

                mostra.set(true);
                frame(tree);
                assert_eq!(tree.open_layers(), [selo]);
                // O ponto do meio da sobreposição continua sendo do que está embaixo.
                let meio = Point::new(50.0, 120.0);
                assert_eq!(tree.hit(meio), tree.find("outro"));
                assert_eq!(tree.scrollers_at(meio), []);
                tree.focus_next();
                tree.focus_next();
                assert_ne!(tree.focused(), Some(selo));
                click(tree, meio);
                assert_eq!(tree.hit(meio), tree.find("outro"));

                // Com um menu aberto, a sobreposição fica atrás dele.
                tree.set_focus(tree.find("mais"), FocusSource::Keyboard);
                tree.key_down(Key::Enter);
                frame(tree);
                let menu = tree.find("menu").unwrap();
                assert_eq!(tree.open_layers(), [selo, menu]);

                mostra.set(false);
                frame(tree);
                assert_eq!(tree.open_layers(), [menu]);
            },
        );
    }

    #[test]
    fn dica_abre_depois_do_atraso_e_fecha_no_clique() {
        with_tree(
            || {
                Box::new(
                    Column::new()
                        .child(
                            Button::new("Salvar")
                                .id("salvar")
                                .tooltip("Salva o rascunho"),
                        )
                        .child(Button::new("Outro").id("outro")),
                )
            },
            |tree| {
                put(tree, "salvar", Rect::new(0.0, 0.0, 100.0, 44.0));
                put(tree, "outro", Rect::new(0.0, 50.0, 100.0, 94.0));
                let dica = tree.find("salvar-tooltip").unwrap();
                assert!(!tree.shown(dica));
                assert_eq!(tree.deadline(), None);

                tree.pointer_move(Point::new(50.0, 20.0), MOUSE);
                assert_eq!(tree.deadline(), Some(TOOLTIP_DELAY));
                tree.advance(Duration::from_millis(300));
                assert_eq!(tree.deadline(), Some(Duration::from_millis(200)));
                assert!(!tree.shown(dica));
                tree.advance(Duration::from_millis(200));
                assert!(tree.shown(dica));
                assert_eq!(tree.open_tooltip(), Some(dica));
                assert_eq!(tree.open_layers(), [dica]);
                assert_eq!(tree.hit(Point::new(50.0, 20.0)), tree.find("salvar"));

                tree.pointer_down(Point::new(50.0, 20.0), MOUSE, 1);
                assert!(!tree.shown(dica), "o clique fecha");
                tree.pointer_up(Point::new(50.0, 20.0), MOUSE);
                tree.advance(TOOLTIP_DELAY);
                assert!(!tree.shown(dica), "só volta depois de sair e entrar");

                tree.pointer_move(Point::new(50.0, 70.0), MOUSE);
                assert_eq!(tree.deadline(), None, "\"outro\" não tem dica");
                tree.pointer_move(Point::new(50.0, 20.0), MOUSE);
                assert_eq!(tree.deadline(), Some(TOOLTIP_DELAY));

                // Pelo teclado.
                tree.pointer_move(Point::new(500.0, 500.0), MOUSE);
                tree.set_focus(tree.find("outro"), FocusSource::Keyboard);
                tree.key_down(Key::Tab);
                assert_eq!(focused(tree).as_deref(), Some("salvar"));
                tree.advance(TOOLTIP_DELAY);
                assert!(tree.shown(dica));
                tree.key_down(Key::Tab);
                assert!(!tree.shown(dica), "o foco saiu");
            },
        );
    }

    fn text_of(tree: &Tree, name: &str) -> String {
        let widget = tree.widget(tree.find(name).unwrap());
        untrack(|| widget.text()).unwrap()
    }

    #[test]
    fn grupo_de_radio_e_uma_parada_so_do_tab() {
        with_tree(
            || {
                Box::new(
                    Column::new()
                        .child(Button::new("Antes").id("antes"))
                        .child(RadioButtons::bind(signal("B".to_owned()), ["A", "B", "C"]).id("op"))
                        .child(Button::new("Depois").id("depois")),
                )
            },
            |tree| {
                tree.set_focus(tree.find("antes"), FocusSource::Keyboard);
                tree.key_down(Key::Tab);
                assert_eq!(focused(tree).as_deref(), Some("op-b"), "chega na marcada");
                tree.key_down(Key::Tab);
                assert_eq!(focused(tree).as_deref(), Some("depois"));
                tree.set_modifiers(Modifiers::SHIFT);
                tree.key_down(Key::Tab);
                tree.set_modifiers(Modifiers::NONE);
                assert_eq!(focused(tree).as_deref(), Some("op-b"));

                tree.key_down(Key::Down);
                frame(tree);
                assert_eq!(focused(tree).as_deref(), Some("op-c"));
                assert_eq!(text_of(tree, "op"), "C", "a marca segue o foco");
                tree.key_down(Key::Tab);
                assert_eq!(focused(tree).as_deref(), Some("depois"));
            },
        );
    }

    #[test]
    fn ctrl_tab_sobe_do_campo_ate_o_tab_view() {
        with_tree(
            || {
                Box::new(
                    TabView::bind(signal("um".to_owned()))
                        .id("abas")
                        .tab("um", "Um", Edit::bind(signal(String::new())).id("campo"))
                        .tab("dois", "Dois", Button::new("Ok").id("ok")),
                )
            },
            |tree| {
                tree.set_focus(tree.find("campo"), FocusSource::Keyboard);
                tree.set_modifiers(Modifiers::CTRL);
                tree.key_down(Key::Tab);
                tree.set_modifiers(Modifiers::NONE);
                frame(tree);
                assert_eq!(text_of(tree, "abas"), "dois");
                assert_eq!(focused(tree).as_deref(), Some("abas-dois"));
                assert!(!tree.shown(tree.find("campo").unwrap()));

                // Sem TabView em volta, Ctrl+Tab move o foco como o Tab.
                tree.key_down(Key::Tab);
                assert_eq!(focused(tree).as_deref(), Some("ok"));
            },
        );
    }

    #[test]
    fn sugestoes_recebem_o_ponteiro_e_o_foco_fica_no_campo() {
        with_tree(
            || {
                Box::new(
                    Column::new()
                        .child(
                            AutoSuggestBox::bind(signal(String::new()), |t: &str| {
                                ["Belém", "Bento"]
                                    .into_iter()
                                    .filter(|c| !t.is_empty() && c.starts_with(t))
                                    .map(str::to_owned)
                                    .collect()
                            })
                            .id("cidade"),
                        )
                        .child(Button::new("Outro").id("outro")),
                )
            },
            |tree| {
                put(tree, "cidade", Rect::new(0.0, 0.0, 300.0, 44.0));
                put(tree, "outro", Rect::new(0.0, 200.0, 300.0, 244.0));
                tree.set_focus(tree.find("cidade"), FocusSource::Pointer);
                tree.text("Be");
                frame(tree);
                untrack(|| tree.reconcile());
                frame(tree);
                let lista = tree.find("cidade-lista").unwrap();
                assert!(tree.shown(lista));
                for (name, rect) in [
                    ("cidade-lista", Rect::new(0.0, 48.0, 300.0, 148.0)),
                    ("cidade-rolagem", Rect::new(0.0, 48.0, 300.0, 148.0)),
                    ("cidade-belém", Rect::new(0.0, 52.0, 300.0, 96.0)),
                    ("cidade-bento", Rect::new(0.0, 100.0, 300.0, 144.0)),
                ] {
                    put(tree, name, rect);
                }
                assert_eq!(
                    tree.hit(Point::new(150.0, 120.0)),
                    tree.find("cidade-bento")
                );
                assert_eq!(
                    tree.hit(Point::new(150.0, 220.0)),
                    tree.find("outro"),
                    "a lista não tapa o resto da tela"
                );

                tree.key_down(Key::Down);
                frame(tree);
                assert_eq!(
                    focused(tree).as_deref(),
                    Some("cidade"),
                    "↓ não tira o foco"
                );
                assert_eq!(text_of(tree, "cidade"), "Belém");

                click(tree, Point::new(150.0, 120.0));
                assert_eq!(text_of(tree, "cidade"), "Bento");
                assert!(!tree.shown(lista));
                assert_eq!(focused(tree).as_deref(), Some("cidade"));
            },
        );
    }

    #[test]
    fn indeterminado_anima_enquanto_aparece() {
        with_tree(
            || {
                let visivel = signal(true);
                Box::new(
                    Column::new()
                        .child(ProgressBar::new().id("barra").visible(visivel))
                        .child(ProgressBar::new().value(0.5).id("fixa"))
                        .child(
                            Button::new("Esconder")
                                .id("esconder")
                                .on_click(move || visivel.set(false)),
                        ),
                )
            },
            |tree| {
                assert_eq!(tree.deadline(), Some(ANIMATION_FRAME));
                tree.take_dirty();
                tree.advance(ANIMATION_FRAME);
                assert_eq!(tree.take_dirty(), [tree.find("barra").unwrap()]);

                put(tree, "esconder", Rect::new(0.0, 0.0, 100.0, 44.0));
                click(tree, Point::new(50.0, 20.0));
                assert_eq!(tree.deadline(), None, "oculto, não acorda o loop");
            },
        );
    }

    /// Barra com "Arquivo" (Alt+A), "Editar" (Alt+E) e "Exibir", e um botão "campo" embaixo.
    fn com_barra(escolhas: Rc<RefCell<Vec<&'static str>>>) -> impl FnOnce() -> Box<dyn Widget> {
        move || {
            let (a, b, c) = (Rc::clone(&escolhas), Rc::clone(&escolhas), escolhas);
            Box::new(
                Column::new()
                    .child(
                        crate::MenuBar::new()
                            .id("barra")
                            .menu(
                                "Arquivo",
                                crate::MenuFlyout::new()
                                    .id("arquivo-menu")
                                    .item("novo", "Novo", move || a.borrow_mut().push("novo"))
                                    .item("sair", "Sair", move || b.borrow_mut().push("sair")),
                            )
                            .access_key('A')
                            .menu(
                                "Editar",
                                crate::MenuFlyout::new().id("editar-menu").item(
                                    "copiar",
                                    "Copiar",
                                    move || c.borrow_mut().push("copiar"),
                                ),
                            )
                            .access_key('E')
                            .menu(
                                "Exibir",
                                crate::MenuFlyout::new().id("exibir-menu").item(
                                    "zoom",
                                    "Zoom",
                                    || {},
                                ),
                            ),
                    )
                    .child(Button::new("Campo").id("campo")),
            )
        }
    }

    fn place_barra(tree: &mut Tree) {
        put(tree, "barra", Rect::new(0.0, 0.0, 400.0, 40.0));
        put(tree, "barra-arquivo", Rect::new(4.0, 4.0, 70.0, 36.0));
        put(tree, "barra-editar", Rect::new(78.0, 4.0, 140.0, 36.0));
        put(tree, "barra-exibir", Rect::new(148.0, 4.0, 210.0, 36.0));
        put(tree, "arquivo-menu", Rect::new(4.0, 40.0, 164.0, 134.0));
        put(tree, "novo", Rect::new(5.0, 43.0, 163.0, 87.0));
        put(tree, "sair", Rect::new(5.0, 87.0, 163.0, 131.0));
        put(tree, "editar-menu", Rect::new(78.0, 40.0, 238.0, 90.0));
        put(tree, "copiar", Rect::new(79.0, 43.0, 237.0, 87.0));
        put(tree, "exibir-menu", Rect::new(148.0, 40.0, 308.0, 90.0));
        put(tree, "zoom", Rect::new(149.0, 43.0, 307.0, 87.0));
        put(tree, "campo", Rect::new(0.0, 300.0, 100.0, 344.0));
    }

    /// Os menus da barra que estão abertos, pelo id.
    fn abertos(tree: &Tree) -> Vec<&'static str> {
        ["arquivo-menu", "editar-menu", "exibir-menu"]
            .into_iter()
            .filter(|m| tree.shown(tree.find(m).unwrap()))
            .collect()
    }

    #[test]
    fn barra_de_menus_abre_no_clique_e_troca_com_o_ponteiro() {
        let escolhas = Rc::default();
        with_tree(com_barra(Rc::clone(&escolhas)), |tree| {
            place_barra(tree);
            assert_eq!(tree.hit(Point::new(30.0, 20.0)), tree.find("barra-arquivo"));
            click(tree, Point::new(30.0, 20.0));
            assert_eq!(abertos(tree), ["arquivo-menu"]);
            assert_eq!(
                tree.look(tree.find("barra-arquivo").unwrap()),
                Look::MenuBarItem {
                    label: "Arquivo".into(),
                    visual: crate::MenuBarItemVisual {
                        state: ButtonState::Hover,
                        open: true,
                        focus_ring: false,
                        density: crate::Density::Standard,
                    },
                },
                "o item aberto fica marcado"
            );

            tree.pointer_move(Point::new(100.0, 20.0), MOUSE);
            frame(tree);
            assert_eq!(abertos(tree), ["editar-menu"], "o ponteiro troca de menu");
            tree.pointer_move(Point::new(300.0, 20.0), MOUSE);
            frame(tree);
            assert_eq!(abertos(tree), ["editar-menu"], "fora dos itens, fica");

            click(tree, Point::new(100.0, 20.0));
            assert!(abertos(tree).is_empty(), "o clique no item aberto fecha");
            tree.pointer_move(Point::new(30.0, 20.0), MOUSE);
            frame(tree);
            assert!(
                abertos(tree).is_empty(),
                "sem menu aberto, o ponteiro não abre"
            );

            click(tree, Point::new(170.0, 20.0));
            assert_eq!(abertos(tree), ["exibir-menu"]);
            click(tree, Point::new(390.0, 390.0));
            assert!(abertos(tree).is_empty(), "o clique fora fecha");

            click(tree, Point::new(30.0, 20.0));
            click(tree, Point::new(50.0, 110.0));
            assert_eq!(*escolhas.borrow(), ["sair"]);
            assert!(abertos(tree).is_empty());
        });
    }

    #[test]
    fn barra_de_menus_pelo_teclado() {
        let escolhas = Rc::default();
        with_tree(com_barra(Rc::clone(&escolhas)), |tree| {
            place_barra(tree);
            tree.set_focus(tree.find("campo"), FocusSource::Keyboard);
            tree.key_down(Key::F(10));
            assert_eq!(focused(tree).as_deref(), Some("barra-arquivo"), "F10");
            tree.key_down(Key::Right);
            assert_eq!(focused(tree).as_deref(), Some("barra-editar"));
            tree.key_down(Key::Right);
            tree.key_down(Key::Left);
            assert_eq!(focused(tree).as_deref(), Some("barra-editar"));
            assert!(abertos(tree).is_empty(), "andar pelos itens não abre");

            tree.key_down(Key::Down);
            frame(tree);
            assert_eq!(abertos(tree), ["editar-menu"]);
            assert_eq!(focused(tree).as_deref(), Some("copiar"), "↓ entra no menu");
            tree.key_down(Key::Right);
            frame(tree);
            assert_eq!(abertos(tree), ["exibir-menu"], "→ troca de menu");
            assert_eq!(focused(tree).as_deref(), Some("zoom"));
            tree.key_down(Key::Right);
            frame(tree);
            assert_eq!(abertos(tree), ["arquivo-menu"], "e dá a volta");
            tree.key_down(Key::Left);
            frame(tree);
            assert_eq!(abertos(tree), ["exibir-menu"], "← também");

            tree.key_down(Key::Escape);
            frame(tree);
            assert!(abertos(tree).is_empty());
            assert_eq!(
                focused(tree).as_deref(),
                Some("barra-exibir"),
                "o Esc devolve o foco ao item"
            );
            tree.key_down(Key::Escape);
            frame(tree);
            assert_eq!(
                focused(tree).as_deref(),
                Some("campo"),
                "o outro Esc, a quem tinha antes"
            );

            tree.set_modifiers(Modifiers::ALT);
            tree.key_down(Key::Alt);
            tree.set_modifiers(Modifiers::NONE);
            tree.key_up(Key::Alt);
            assert_eq!(focused(tree).as_deref(), Some("barra-arquivo"), "Alt solto");
            tree.set_modifiers(Modifiers::ALT);
            tree.key_down(Key::Alt);
            tree.set_modifiers(Modifiers::NONE);
            tree.key_up(Key::Alt);
            assert_eq!(focused(tree).as_deref(), Some("campo"), "o Alt de novo sai");

            tree.key_down(Key::F(10));
            tree.key_down(Key::Down);
            frame(tree);
            tree.key_down(Key::Enter);
            frame(tree);
            assert_eq!(*escolhas.borrow(), ["novo"]);
            assert!(abertos(tree).is_empty());
            assert_eq!(
                focused(tree).as_deref(),
                Some("campo"),
                "escolher devolve o foco a quem tinha antes da barra"
            );
        });
    }

    #[test]
    fn alt_e_a_letra_abrem_o_menu_da_barra() {
        let escolhas = Rc::default();
        with_tree(com_barra(Rc::clone(&escolhas)), |tree| {
            place_barra(tree);
            tree.set_focus(tree.find("campo"), FocusSource::Keyboard);
            tree.set_modifiers(Modifiers::ALT);
            tree.key_down(Key::Alt);
            tree.key_down(Key::Char('e'));
            tree.set_modifiers(Modifiers::NONE);
            tree.key_up(Key::Alt);
            frame(tree);
            assert_eq!(abertos(tree), ["editar-menu"], "Alt+E");
            assert_eq!(focused(tree).as_deref(), Some("copiar"));

            tree.set_modifiers(Modifiers::ALT);
            tree.key_down(Key::Char('a'));
            tree.set_modifiers(Modifiers::NONE);
            frame(tree);
            assert_eq!(abertos(tree), ["arquivo-menu"], "com outro aberto, troca");
            tree.key_down(Key::Enter);
            frame(tree);
            assert_eq!(*escolhas.borrow(), ["novo"]);
            assert_eq!(focused(tree).as_deref(), Some("campo"));
        });
    }

    /// Widget de teste: arrasta, solta e conta o que recebeu.
    struct Peca {
        common: crate::widget::Common,
        arrasta: bool,
        recebe: bool,
        log: Rc<RefCell<Vec<String>>>,
        quer_foco: bool,
    }

    impl Peca {
        fn new(id: &str, log: &Rc<RefCell<Vec<String>>>) -> Self {
            Self {
                common: crate::widget::Common {
                    id: Some(id.to_owned()),
                    ..Default::default()
                },
                arrasta: false,
                recebe: false,
                log: Rc::clone(log),
                quer_foco: false,
            }
        }
    }

    impl Widget for Peca {
        fn kind(&self) -> &'static str {
            "peca"
        }

        fn common(&self) -> &crate::widget::Common {
            &self.common
        }

        fn look(&self) -> Look {
            Look::Column
        }

        fn interactive(&self) -> bool {
            true
        }

        fn focusable(&self) -> bool {
            true
        }

        fn drag_data(&self) -> Option<DragData> {
            self.arrasta.then(|| DragData {
                kind: "peca".into(),
                key: "p".into(),
                label: "P".into(),
            })
        }

        fn handle(&mut self, event: &Event, cx: &mut EventCx) -> bool {
            match event {
                Event::PointerDown { .. } => true,
                Event::DragOver(_) => self.recebe,
                Event::Drop(data) => {
                    self.log.borrow_mut().push(format!("drop {}", data.key));
                    true
                }
                Event::ContextOpening => {
                    self.log.borrow_mut().push("contexto".into());
                    self.quer_foco
                }
                Event::KeyDown(Key::Enter) => {
                    cx.focus_later("alvo");
                    true
                }
                _ => false,
            }
        }
    }

    #[test]
    fn arrasto_ativo_segue_sem_a_origem() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let l = Rc::clone(&log);
        let mostra = signal(true);
        with_tree(
            move || {
                let (a, b) = (Rc::clone(&l), Rc::clone(&l));
                Box::new(
                    Column::new()
                        .child(Show::when(mostra, move || {
                            let mut p = Peca::new("fonte", &a);
                            p.arrasta = true;
                            p
                        }))
                        .child({
                            let mut p = Peca::new("alvo", &b);
                            p.recebe = true;
                            p
                        }),
                )
            },
            |tree| {
                put(tree, "fonte", Rect::new(0.0, 0.0, 100.0, 40.0));
                put(tree, "alvo", Rect::new(0.0, 100.0, 100.0, 140.0));
                tree.pointer_down(Point::new(10.0, 10.0), MOUSE, 1);
                tree.pointer_move(Point::new(10.0, 60.0), MOUSE);
                assert!(tree.drag_label().is_some(), "o arrasto começou");
                mostra.set(false);
                frame(tree);
                untrack(|| tree.reconcile());
                assert!(tree.find("fonte").is_none(), "a origem saiu da árvore");
                assert!(tree.drag_label().is_some(), "o arrasto ativo segue");
                tree.pointer_move(Point::new(10.0, 120.0), MOUSE);
                tree.pointer_up(Point::new(10.0, 120.0), MOUSE);
                assert_eq!(*log.borrow(), ["drop p"]);
            },
        );
    }

    #[test]
    fn origem_removida_antes_do_limiar_cancela_o_arrasto() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let l = Rc::clone(&log);
        let mostra = signal(true);
        with_tree(
            move || {
                let (a, b) = (Rc::clone(&l), Rc::clone(&l));
                Box::new(
                    Column::new()
                        .child(Show::when(mostra, move || {
                            let mut p = Peca::new("fonte", &a);
                            p.arrasta = true;
                            p
                        }))
                        .child({
                            let mut p = Peca::new("alvo", &b);
                            p.recebe = true;
                            p
                        }),
                )
            },
            |tree| {
                put(tree, "fonte", Rect::new(0.0, 0.0, 100.0, 40.0));
                put(tree, "alvo", Rect::new(0.0, 100.0, 100.0, 140.0));
                tree.pointer_down(Point::new(10.0, 10.0), MOUSE, 1);
                mostra.set(false);
                frame(tree);
                untrack(|| tree.reconcile());
                tree.pointer_move(Point::new(10.0, 120.0), MOUSE);
                tree.pointer_up(Point::new(10.0, 120.0), MOUSE);
                assert!(tree.drag_label().is_none());
                assert!(log.borrow().is_empty(), "ninguém recebeu o item");
            },
        );
    }

    #[test]
    fn dono_do_menu_de_contexto_sabe_antes_de_abrir_e_pode_pedir_o_foco() {
        for quer_foco in [true, false] {
            let log = Rc::new(RefCell::new(Vec::new()));
            let l = Rc::clone(&log);
            with_tree(
                move || {
                    let mut dono = Peca::new("dono", &l);
                    dono.quer_foco = quer_foco;
                    dono.common
                        .set_context_flyout(crate::MenuFlyout::new().id("ctx").item(
                            "x",
                            "X",
                            || {},
                        ));
                    Box::new(Column::new().child(dono).child(Button::new("B").id("b")))
                },
                |tree| {
                    put(tree, "dono", Rect::new(0.0, 0.0, 100.0, 40.0));
                    put(tree, "b", Rect::new(0.0, 100.0, 100.0, 140.0));
                    tree.set_focus(tree.find("b"), FocusSource::Keyboard);
                    tree.secondary_down(Point::new(10.0, 10.0));
                    assert_eq!(*log.borrow(), ["contexto"]);
                    let ctx = tree.find("ctx").unwrap();
                    frame(tree);
                    assert!(tree.shown(ctx));
                    tree.key_down(Key::Escape);
                    frame(tree);
                    let esperado = if quer_foco { "dono" } else { "b" };
                    assert_eq!(
                        focused(tree).as_deref(),
                        Some(esperado),
                        "quer_foco = {quer_foco}"
                    );
                },
            );
        }
    }

    #[test]
    fn focus_later_espera_o_widget_aparecer() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let l = Rc::clone(&log);
        let mostra = signal(false);
        with_tree(
            move || {
                Box::new(
                    Column::new()
                        .child(Peca::new("pede", &l))
                        .child(Show::when(mostra, || Button::new("Alvo").id("alvo"))),
                )
            },
            |tree| {
                tree.set_focus(tree.find("pede"), FocusSource::Keyboard);
                tree.key_down(Key::Enter);
                mostra.set(true);
                frame(tree);
                untrack(|| tree.reconcile());
                put(tree, "alvo", Rect::new(0.0, 100.0, 100.0, 140.0));
                tree.reveal_pending();
                assert_eq!(focused(tree).as_deref(), Some("alvo"));
                assert!(tree.pending_focus.is_none());
            },
        );
    }

    #[test]
    fn focus_later_desiste_se_o_widget_nao_aparece() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let l = Rc::clone(&log);
        with_tree(
            move || Box::new(Column::new().child(Peca::new("pede", &l))),
            |tree| {
                tree.set_focus(tree.find("pede"), FocusSource::Keyboard);
                tree.key_down(Key::Enter);
                assert!(tree.pending_focus.is_some());
                for _ in 0..FOCUS_LATER_TRIES {
                    tree.reveal_pending();
                }
                assert!(tree.pending_focus.is_none(), "desistiu");
                assert_eq!(focused(tree).as_deref(), Some("pede"));
            },
        );
    }

    #[test]
    fn request_focus_foca_um_widget_que_aparece_depois() {
        let mostra = signal(false);
        with_tree(
            move || {
                Box::new(
                    Column::new()
                        .child(Button::new("Primeiro").id("primeiro"))
                        .child(Show::when(mostra, || {
                            Edit::bind(signal(String::new())).id("campo")
                        })),
                )
            },
            |tree| {
                tree.set_focus(tree.find("primeiro"), FocusSource::Keyboard);
                mostra.set(true);
                crate::request_focus("campo");
                frame(tree);
                untrack(|| tree.reconcile());
                put(tree, "campo", Rect::new(0.0, 100.0, 100.0, 140.0));
                tree.reveal_pending();
                assert_eq!(focused(tree).as_deref(), Some("campo"));
            },
        );
    }

    #[test]
    fn scroll_link_publica_o_deslocamento_vertical_e_atende_o_pedido() {
        let link = crate::ScrollLink::new(0.0);
        let l = Rc::clone(&link);
        with_tree(
            move || {
                Box::new(
                    Column::new().child(
                        ScrollViewer::new()
                            .id("rolagem")
                            .link(l)
                            .child(Column::new().child(Text::new("conteúdo").id("texto"))),
                    ),
                )
            },
            |tree| {
                let sv = tree.find("rolagem").unwrap();
                tree.set_scroll_extent(sv, Size::new(100.0, 1000.0), Size::new(100.0, 200.0));
                assert_eq!(link.viewport_height(), 200.0);
                assert_eq!(link.offset_y(), 0.0);
                link.scroll_by_y(150.0);
                tree.advance(Duration::from_millis(16));
                assert_eq!(link.offset_y(), 150.0);
                link.scroll_by_y(-50.0);
                tree.advance(Duration::from_millis(16));
                assert_eq!(link.offset_y(), 100.0);
            },
        );
    }
}
