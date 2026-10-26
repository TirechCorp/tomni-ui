//! O estado que o `DockHost` e as partes dele dividem, e as operações sobre o layout.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tomni_ui_reactive::{
    Get, GetUntracked, Scope, Set, Signal, With, WithUntracked, effect, untrack,
};

use super::{Content, DOCK_PANE_SIZE, DockLayout, DockMode, DockPaneState, DockSide};
use crate::kurbo::Size;
use crate::prop::Prop;
use crate::symbol::Symbol;
use crate::windows::{WindowGeometry, WindowHandle};

/// Tamanho da área fixada de baixo, quando nenhum painel dá o dele.
const DEFAULT_BOTTOM_SIZE: f64 = 200.0;

/// Um painel, como o app o descreveu.
pub(crate) struct PaneSpec {
    pub key: String,
    pub icon: Symbol,
    pub title: Prop<String>,
    pub content: Content,
    pub side: DockSide,
    pub mode: DockMode,
    pub size: f64,
    pub min_size: f64,
    pub weight: f64,
    pub closable: bool,
    pub floatable: bool,
    pub actions: Option<Content>,
}

/// Onde o painel está montado agora.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Placement {
    /// Na área fixada do lado.
    Fixed(DockSide),
    /// Por cima do conteúdo, aberto pelo botão da faixa.
    Overlay(DockSide),
    /// Ocupando o `DockHost` inteiro.
    Maximized,
    /// Numa janela própria.
    Floating,
}

/// Onde o painel `key` está montado, ou `None` se ele não aparece (fechado ou desconhecido).
pub(crate) fn placement(layout: &DockLayout, key: &str) -> Option<Placement> {
    let pane = layout.pane(key)?;
    match pane.mode {
        DockMode::Closed => None,
        DockMode::Floating => Some(Placement::Floating),
        DockMode::Pinned | DockMode::AutoHide if layout.maximized.as_deref() == Some(key) => {
            Some(Placement::Maximized)
        }
        DockMode::Pinned => Some(Placement::Fixed(pane.side)),
        DockMode::AutoHide => Some(Placement::Overlay(pane.side)),
    }
}

/// As chaves da área de `side` que estão em `want`, na ordem da área.
pub(crate) fn keys_where(
    layout: &DockLayout,
    side: DockSide,
    want: impl Fn(Placement) -> bool,
) -> Vec<String> {
    layout
        .area(side)
        .order
        .iter()
        .filter(|key| placement(layout, key).is_some_and(&want))
        .cloned()
        .collect()
}

/// Os painéis de `side` em modo auto-ocultar (com ou sem o painel aberto), na ordem da área.
pub(crate) fn strip_keys(layout: &DockLayout, side: DockSide) -> Vec<String> {
    layout
        .area(side)
        .order
        .iter()
        .filter(|key| {
            layout
                .pane(key)
                .is_some_and(|p| p.mode == DockMode::AutoHide)
        })
        .cloned()
        .collect()
}

fn pane_default(spec: &PaneSpec) -> DockPaneState {
    DockPaneState {
        mode: spec.mode,
        side: spec.side,
        weight: spec.weight,
        overlay_size: spec.size,
        floating: None,
        docked: match spec.mode {
            DockMode::AutoHide => DockMode::AutoHide,
            _ => DockMode::Pinned,
        },
    }
}

/// Põe o layout em ordem para os painéis `panes`: tira o que sumiu, preenche o que falta e
/// conserta o que não faz sentido (tamanho zero, chave de outro lado na ordem).
pub(crate) fn normalize(layout: &mut DockLayout, panes: &[Rc<PaneSpec>]) {
    layout
        .panes
        .retain(|key, _| panes.iter().any(|spec| spec.key == *key));
    for spec in panes {
        let state = layout
            .panes
            .entry(spec.key.clone())
            .or_insert_with(|| pane_default(spec));
        if !(state.weight.is_finite() && state.weight > 0.0) {
            state.weight = spec.weight;
        }
        if !(state.overlay_size.is_finite() && state.overlay_size > 0.0) {
            state.overlay_size = spec.size;
        }
        if !matches!(state.docked, DockMode::Pinned | DockMode::AutoHide) {
            state.docked = DockMode::Pinned;
        }
    }
    for side in DockSide::ALL {
        let mut keys: Vec<String> = Vec::new();
        for key in &layout.area(side).order {
            let here = layout.pane(key).is_some_and(|p| p.side == side);
            if here && !keys.contains(key) {
                keys.push(key.clone());
            }
        }
        for spec in panes {
            let here = layout.pane(&spec.key).is_some_and(|p| p.side == side);
            if here && !keys.contains(&spec.key) {
                keys.push(spec.key.clone());
            }
        }
        let first = panes
            .iter()
            .find(|spec| layout.pane(&spec.key).is_some_and(|p| p.side == side));
        let area = layout.area_mut(side);
        area.order = keys;
        if !(area.size.is_finite() && area.size > 0.0) {
            area.size = first.map_or(
                if side == DockSide::Bottom {
                    DEFAULT_BOTTOM_SIZE
                } else {
                    DOCK_PANE_SIZE
                },
                |spec| spec.size,
            );
        }
    }
    let valid = layout.maximized.as_ref().is_some_and(|key| {
        layout
            .pane(key)
            .is_some_and(|p| matches!(p.mode, DockMode::Pinned | DockMode::AutoHide))
    });
    if !valid {
        layout.maximized = None;
    }
}

/// O ponteiro, o foco e os temporizadores que abrem e recolhem os painéis auto-ocultar.
#[derive(Default)]
pub(crate) struct Timing {
    /// O painel cujo botão da faixa está sob o ponteiro.
    pub hover_key: Option<String>,
    /// O painel cujo botão da faixa está sob o ponteiro, mesmo depois de um clique.
    pub strip_hover: Option<String>,
    /// Quanto o ponteiro está parado nesse botão.
    pub hover_elapsed: Duration,
    /// Quanto o ponteiro e o foco estão fora do painel aberto e da faixa.
    pub leave_elapsed: Duration,
    /// O botão que acabou de ser clicado: o ponteiro precisa sair dele antes de abrir de novo
    /// ao parar.
    pub suppress: Option<String>,
    /// Os painéis com o ponteiro dentro.
    pub pointer_in: HashSet<String>,
    /// Os painéis com o foco dentro.
    pub focus_in: HashSet<String>,
}

/// Tudo o que as partes do `DockHost` dividem.
pub(crate) struct Shared {
    pub id: String,
    pub layout: Signal<DockLayout>,
    pub min_content: f64,
    pub panes: Vec<Rc<PaneSpec>>,
    /// O painel auto-ocultar aberto por cima do conteúdo (um só em todo o `DockHost`).
    pub opened: Signal<Option<String>>,
    /// O tamanho da área do conteúdo (entre as áreas fixadas), do último layout.
    pub center: Signal<Size>,
    pub timing: RefCell<Timing>,
    /// As janelas dos painéis flutuando.
    pub windows: RefCell<HashMap<String, WindowHandle>>,
    /// O último tamanho medido da casca de cada painel.
    pub sizes: RefCell<HashMap<String, Size>>,
    /// Se a tela do `DockHost` já saiu da árvore (as janelas flutuantes não mexem mais no
    /// layout).
    pub disposed: Arc<AtomicBool>,
    /// O escopo dos efeitos que ligam os signals do controlador ao layout.
    pub wiring: RefCell<Option<Scope>>,
}

impl Shared {
    pub fn new(
        id: String,
        layout: Signal<DockLayout>,
        min_content: f64,
        panes: Vec<Rc<PaneSpec>>,
        opened: Signal<Option<String>>,
        center: Signal<Size>,
    ) -> Self {
        Self {
            id,
            layout,
            min_content,
            panes,
            opened,
            center,
            timing: RefCell::default(),
            windows: RefCell::default(),
            sizes: RefCell::default(),
            disposed: Arc::new(AtomicBool::new(false)),
            wiring: RefCell::new(None),
        }
    }

    pub fn spec(&self, key: &str) -> Option<&Rc<PaneSpec>> {
        self.panes.iter().find(|spec| spec.key == key)
    }

    /// O id gerado de uma parte: `{id}-{suffix}`.
    pub fn sub(&self, suffix: &str) -> String {
        format!("{}-{suffix}", self.id)
    }

    /// O id de uma parte de um painel: `{id}-{key}` ou `{id}-{key}-{suffix}`.
    pub fn pane_id(&self, key: &str, suffix: Option<&str>) -> String {
        match suffix {
            Some(suffix) => format!("{}-{key}-{suffix}", self.id),
            None => format!("{}-{key}", self.id),
        }
    }

    pub fn is_disposed(&self) -> bool {
        self.disposed.load(Ordering::Relaxed)
    }

    /// O estado do painel, sem registrar dependência.
    pub fn state(&self, key: &str) -> Option<DockPaneState> {
        self.layout
            .with_untracked(|layout| layout.pane(key).cloned())
    }

    /// O modo do painel (reativo).
    pub fn mode_of(&self, key: &str) -> Option<DockMode> {
        self.layout
            .with(|layout| layout.pane(key).map(|pane| pane.mode))
    }

    /// Muda o layout com `change`; só grava (e avisa) se `change` disse que mudou algo.
    pub fn edit(&self, change: impl FnOnce(&mut DockLayout) -> bool) -> bool {
        let before = self.layout.get_untracked();
        let mut layout = before.clone();
        if !change(&mut layout) || layout == before {
            return false;
        }
        self.layout.set(layout);
        true
    }

    /// Abre o painel auto-ocultar `key` por cima do conteúdo.
    pub(crate) fn open(&self, key: &str) {
        if self.opened.get_untracked().as_deref() != Some(key) {
            self.opened.set(Some(key.to_string()));
        }
        self.reset_leave();
    }

    /// Recolhe o painel aberto por cima, seja qual for.
    pub fn collapse(&self) {
        if self.opened.get_untracked().is_some() {
            self.opened.set(None);
        }
        let mut timing = self.timing.borrow_mut();
        timing.leave_elapsed = Duration::ZERO;
        timing.hover_key = None;
        timing.hover_elapsed = Duration::ZERO;
    }

    fn reset_leave(&self) {
        self.timing.borrow_mut().leave_elapsed = Duration::ZERO;
    }

    fn drop_opened(&self, key: &str) {
        if self.opened.get_untracked().as_deref() == Some(key) {
            self.collapse();
        }
    }

    pub fn show(&self, key: &str, focus: bool) -> bool {
        let Some(state) = self.state(key) else {
            return false;
        };
        // Outro painel maximizado cobriria o que se pede para mostrar.
        let other_maximized = self
            .layout
            .with_untracked(|l| l.maximized.as_deref().is_some_and(|m| m != key));
        if other_maximized {
            self.edit(|l| {
                l.maximized = None;
                true
            });
        }
        match state.mode {
            DockMode::Closed => {
                self.edit(|l| {
                    if let Some(pane) = l.panes.get_mut(key) {
                        pane.mode = pane.docked;
                    }
                    true
                });
                if state.docked == DockMode::AutoHide {
                    self.open(key);
                }
            }
            DockMode::AutoHide => self.open(key),
            DockMode::Pinned => {}
            DockMode::Floating => {
                if let Some(handle) = self.windows.borrow().get(key) {
                    handle.focus();
                }
                return true;
            }
        }
        if focus {
            crate::request_focus_inside(self.pane_id(key, Some("corpo")));
        }
        true
    }

    pub fn hide(&self, key: &str) -> bool {
        if self.opened.get_untracked().as_deref() == Some(key) {
            self.collapse();
            true
        } else {
            false
        }
    }

    pub fn toggle(&self, key: &str) -> bool {
        let Some(state) = self.state(key) else {
            return false;
        };
        let open = match state.mode {
            DockMode::Closed => false,
            DockMode::AutoHide => self.opened.get_untracked().as_deref() == Some(key),
            DockMode::Pinned | DockMode::Floating => true,
        };
        if open {
            self.close(key)
        } else {
            self.show(key, true)
        }
    }

    pub fn pin(&self, key: &str) -> bool {
        let done = self.edit(|l| match l.panes.get_mut(key) {
            Some(pane) if pane.mode == DockMode::AutoHide => {
                pane.mode = DockMode::Pinned;
                pane.docked = DockMode::Pinned;
                true
            }
            _ => false,
        });
        if done {
            self.drop_opened(key);
        }
        done
    }

    pub fn unpin(&self, key: &str) -> bool {
        self.edit(|l| match l.panes.get_mut(key) {
            Some(pane) if pane.mode == DockMode::Pinned => {
                pane.mode = DockMode::AutoHide;
                pane.docked = DockMode::AutoHide;
                true
            }
            _ => false,
        })
    }

    pub fn float(&self, key: &str) -> bool {
        let done = self.edit(|l| {
            let Some(pane) = l.panes.get_mut(key) else {
                return false;
            };
            if !matches!(pane.mode, DockMode::Pinned | DockMode::AutoHide) {
                return false;
            }
            pane.docked = pane.mode;
            pane.mode = DockMode::Floating;
            if l.maximized.as_deref() == Some(key) {
                l.maximized = None;
            }
            true
        });
        if done {
            self.drop_opened(key);
        }
        done
    }

    pub fn dock(&self, key: &str) -> bool {
        self.edit(|l| match l.panes.get_mut(key) {
            Some(pane) if pane.mode == DockMode::Floating => {
                pane.mode = pane.docked;
                true
            }
            _ => false,
        })
    }

    pub fn close(&self, key: &str) -> bool {
        let done = self.edit(|l| {
            let Some(pane) = l.panes.get_mut(key) else {
                return false;
            };
            if pane.mode == DockMode::Closed {
                return false;
            }
            if matches!(pane.mode, DockMode::Pinned | DockMode::AutoHide) {
                pane.docked = pane.mode;
            }
            pane.mode = DockMode::Closed;
            if l.maximized.as_deref() == Some(key) {
                l.maximized = None;
            }
            true
        });
        if done {
            self.drop_opened(key);
        }
        done
    }

    pub fn move_to(&self, key: &str, side: DockSide) -> bool {
        self.edit(|l| {
            let Some(pane) = l.panes.get_mut(key) else {
                return false;
            };
            if pane.side == side {
                return false;
            }
            let from = pane.side;
            pane.side = side;
            l.area_mut(from).order.retain(|k| k != key);
            let order = &mut l.area_mut(side).order;
            order.retain(|k| k != key);
            order.push(key.to_string());
            true
        })
    }

    pub fn maximize(&self, key: &str) -> bool {
        let done = self.edit(|l| {
            let allowed = l
                .pane(key)
                .is_some_and(|p| matches!(p.mode, DockMode::Pinned | DockMode::AutoHide));
            if !allowed || l.maximized.as_deref() == Some(key) {
                return false;
            }
            l.maximized = Some(key.to_string());
            true
        });
        if done {
            self.collapse();
            // O painel monta de novo no lugar de maximizado: o foco vai para dentro dele, e o
            // Esc que restaura encontra o painel no caminho.
            crate::request_focus_inside(self.pane_id(key, Some("corpo")));
        }
        done
    }

    pub fn restore(&self) -> bool {
        self.edit(|l| l.maximized.take().is_some())
    }

    /// Liga o signal `shown` do controlador ao layout: ele acompanha "não está fechado", e
    /// mudá-lo mostra ou fecha o painel. Os efeitos moram no escopo `wiring`.
    pub fn wire_shown(self: &Rc<Self>, key: &str, shown: Signal<bool>) {
        let Some(scope) = self.wiring.borrow().clone() else {
            return;
        };
        if self.spec(key).is_none() {
            return;
        }
        let open = self
            .state(key)
            .is_some_and(|state| state.mode != DockMode::Closed);
        if shown.get_untracked() != open {
            shown.set(open);
        }
        scope.run(|| {
            let (this, name) = (Rc::clone(self), key.to_string());
            effect(move || {
                let open = this
                    .mode_of(&name)
                    .is_some_and(|mode| mode != DockMode::Closed);
                untrack(|| {
                    if shown.get() != open {
                        shown.set(open);
                    }
                });
            });
            let (this, name) = (Rc::clone(self), key.to_string());
            effect(move || {
                let want = shown.get();
                untrack(|| {
                    let open = this
                        .state(&name)
                        .is_some_and(|state| state.mode != DockMode::Closed);
                    if want && !open {
                        this.show(&name, false);
                    } else if !want && open {
                        this.close(&name);
                    }
                });
            });
        });
    }

    /// Abre e fecha as janelas dos painéis flutuando, para que o layout e as janelas
    /// concordem: o layout manda.
    pub fn sync_windows(self: &Rc<Self>, layout: &DockLayout) {
        for spec in &self.panes {
            let key = spec.key.as_str();
            let floating = layout
                .pane(key)
                .is_some_and(|pane| pane.mode == DockMode::Floating);
            let handle = self.windows.borrow().get(key).copied();
            match (floating, handle) {
                (true, None) => self.open_window(spec, layout),
                (false, Some(handle)) => {
                    // Sai da tabela antes de fechar: o gancho de fechar da janela não mexe no
                    // layout de uma janela que o `DockHost` mesmo está fechando.
                    self.windows.borrow_mut().remove(key);
                    handle.close();
                }
                _ => {}
            }
        }
    }

    fn open_window(self: &Rc<Self>, spec: &Rc<PaneSpec>, layout: &DockLayout) {
        let key = spec.key.clone();
        let saved: Option<WindowGeometry> = layout.pane(&key).and_then(|p| p.floating);
        let measured = self.sizes.borrow().get(&key).copied();
        let title = untrack(|| spec.title.get());
        let shared = Rc::clone(self);
        let pane = Rc::clone(spec);
        let mut window =
            crate::windows::open_window(move || super::build::floating(shared, pane)).title(title);
        match (saved, measured) {
            (Some(geometry), _) => {
                window = window.size(geometry.size.0.max(1), geometry.size.1.max(1));
                if let Some((x, y)) = geometry.position {
                    window = window.position(x, y);
                }
            }
            (None, Some(size)) => {
                window = window.size(size.width.round().max(1.0) as u32, {
                    size.height.round().max(1.0) as u32
                });
            }
            (None, None) => {}
        }
        match window.open() {
            Some(handle) => {
                self.windows.borrow_mut().insert(key, handle);
            }
            // Onde só cabe uma janela (o Android), o painel volta ao lugar.
            None => {
                self.edit(|l| {
                    if let Some(pane) = l.panes.get_mut(&key) {
                        pane.mode = pane.docked;
                    }
                    true
                });
            }
        }
    }

    /// O gancho de fechar da janela flutuante de `key`. Devolve `true` (pode fechar).
    pub fn floating_closed(&self, key: &str, handle: WindowHandle) -> bool {
        if self.is_disposed() {
            return true;
        }
        let mine = self.windows.borrow().get(key) == Some(&handle);
        if !mine {
            return true;
        }
        let geometry = handle.geometry();
        let all = crate::closing_all_windows();
        if !all {
            self.windows.borrow_mut().remove(key);
        }
        self.edit(|l| {
            let Some(pane) = l.panes.get_mut(key) else {
                return false;
            };
            if geometry.is_some() {
                pane.floating = geometry;
            }
            if !all && pane.mode == DockMode::Floating {
                pane.mode = pane.docked;
            }
            true
        });
        true
    }
}

thread_local! {
    static NEXT_ID: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Um id gerado para o `DockHost` sem `.id()`.
pub(crate) fn next_id() -> String {
    NEXT_ID.with(|next| {
        let n = next.get() + 1;
        next.set(n);
        format!("dock{n}")
    })
}
