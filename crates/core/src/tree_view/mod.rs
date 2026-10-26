//! `TreeView`: árvore de pastas e itens, com seleção, filtro, renomear e arrastar.

mod item;
mod model;
mod rows;
mod shared;
mod visual;

use std::collections::BTreeSet;
use std::fmt;
use std::rc::Rc;
use std::time::Duration;

use kurbo::{Point, Rect};
use tomni_ui_reactive::prelude::*;

pub use model::{TreeItem, TreeItems};
pub(crate) use model::{fold, fold_char};
pub use rows::{TreeFlat, TreeRow};
pub(crate) use visual::item_height;
pub use visual::{
    DropMark, ItemStatus, TREE_VIEW_CHEVRON_COLUMN, TREE_VIEW_ICON, TREE_VIEW_ICON_GAP,
    TREE_VIEW_INDENT, TREE_VIEW_ITEM_HEIGHT, TREE_VIEW_ITEM_HEIGHT_COMPACT, TREE_VIEW_STATUS_BADGE,
    TreeViewItemVisual, TreeViewVisual,
};

use crate::containers::Column;
use crate::cx::EventCx;
use crate::data_grid::{DataGridDrop, decode_drag};
use crate::density::Density;
use crate::event::Event;
use crate::flyout::MenuFlyout;
use crate::scroll_viewer::ScrollViewer;
use crate::widget::{Common, Flex, Look, Widget, common_builders, density_builder};
use item::TreeRows;
use shared::Shared;

/// Velocidade máxima da rolagem automática do arrasto, nas bordas, em px/s.
const AUTO_SCROLL: f64 = 600.0;
/// Quanto o arrasto fica parado sobre uma pasta fechada até ela abrir.
const OPEN_DELAY: Duration = Duration::from_secs(1);

/// Árvore de pastas e itens, como o `TreeView` do WinUI.
///
/// ```
/// use tomni_ui_core::{TreeItem, TreeItems, TreeView};
/// use tomni_ui_reactive::prelude::*;
///
/// # let scope = Scope::root();
/// # scope.run(|| {
/// let itens = signal(vec![
///     TreeItem::folder("trabalho", "Trabalho")
///         .child(TreeItem::leaf("wiki", "Wiki"))
///         .child(TreeItem::leaf("agenda", "Agenda")),
///     TreeItem::leaf("blog", "Blog"),
/// ]);
/// let selecionado = signal(None::<String>);
/// let arvore = TreeView::bind(itens, selecionado)
///     .id("favoritos")
///     .grow(1)
///     .on_invoke(move |chave| println!("abrir {chave}"))
///     .on_rename(move |chave, texto| {
///         itens.update(|v| {
///             v.rename(&chave, texto);
///         });
///     });
/// # });
/// # scope.clear();
/// ```
///
/// Os itens e a seleção ficam em signals do app. As pastas abertas ficam no `TreeView`, a não
/// ser que o app passe um signal (`.expanded`). Só as linhas perto da área visível existem
/// como widgets, então uma árvore de milhares de itens continua leve.
///
/// O clique seleciona; o clique duplo e o Enter chamam `on_invoke` (e abrem ou fecham a
/// pasta). As setas, Home, End e as letras navegam; F2 renomeia e Delete chama `on_delete`.
/// Arrastar uma linha chama `on_move` com o novo pai e a posição, que o app aplica em
/// [`TreeItems::move_to`].
pub struct TreeView {
    common: Common,
    shared: Rc<Shared>,
    drag: Option<DragState>,
}

/// O arrasto sobre o `TreeView`.
struct DragState {
    source: Source,
    pointer: Point,
    rect: Rect,
    target: Option<Target>,
    held: Duration,
}

/// De onde vem o arrasto: uma linha da própria árvore ou linhas de um `DataGrid`.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Source {
    Row(String),
    Grid { grid: String, keys: Vec<String> },
}

/// Onde o arrasto cairia.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Target {
    /// A linha da marca, nas linhas visíveis.
    row: usize,
    /// A marca; `None` quando soltar ali não mudaria nada.
    mark: Option<DropMark>,
    /// O pai de destino e a posição na convenção do `move_to`.
    parent: Option<String>,
    position: usize,
    /// A pasta fechada, com filhos, sobre a qual o arrasto está parado no meio.
    closed_folder: Option<String>,
    noop: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Zone {
    Top,
    Into,
    Bottom,
    BelowAll,
}

impl TreeView {
    /// `TreeView` dos itens `items`, com a chave selecionada em `selected`.
    pub fn bind(items: Signal<Vec<TreeItem>>, selected: Signal<Option<String>>) -> Self {
        let shared = Shared::new(items, selected);
        let watch = Rc::clone(&shared);
        effect(move || watch.update_window());
        Self {
            common: Common::default(),
            shared,
            drag: None,
        }
    }

    /// As chaves das pastas abertas, no app. Sem isto, o `TreeView` guarda as dele.
    pub fn expanded(self, expanded: Signal<BTreeSet<String>>) -> Self {
        self.shared.expanded.set(expanded);
        self.shared.rebuild();
        self
    }

    /// O texto do filtro: só os itens cujo rótulo contém o texto (e os ancestrais deles)
    /// aparecem, com as pastas abertas. Sem acentos nem maiúsculas.
    pub fn filter(self, filter: Signal<String>) -> Self {
        self.shared.filter.set(filter);
        self.shared.rebuild();
        self
    }

    /// A chave do item em renomeação, no app: pôr uma chave aqui troca o rótulo dela por um
    /// campo de texto, e `None` volta ao normal. O F2 também usa. Exige `.on_rename`.
    pub fn renaming(self, renaming: Signal<Option<String>>) -> Self {
        self.shared.renaming.set(renaming);
        self
    }

    /// Enter ou clique duplo num item, com a chave dele.
    pub fn on_invoke(self, callback: impl Fn(String) + 'static) -> Self {
        *self.shared.on_invoke.borrow_mut() = Some(Rc::new(callback));
        self
    }

    /// O usuário confirmou um nome novo, com a chave e o texto. O app troca o rótulo
    /// (`TreeItems::rename`); sem este callback não se renomeia.
    pub fn on_rename(self, callback: impl Fn(String, String) + 'static) -> Self {
        *self.shared.on_rename.borrow_mut() = Some(Rc::new(callback));
        self
    }

    /// Delete numa linha, com a chave dela. O app decide se apaga.
    pub fn on_delete(self, callback: impl Fn(String) + 'static) -> Self {
        *self.shared.on_delete.borrow_mut() = Some(Rc::new(callback));
        self
    }

    /// Uma linha foi arrastada e solta: a chave, o pai de destino (`None` é a raiz) e a
    /// posição na lista do pai depois de tirar o item, a que o [`TreeItems::move_to`] espera.
    /// Sem este callback as linhas não arrastam.
    pub fn on_move(self, callback: impl Fn(String, Option<String>, usize) + 'static) -> Self {
        *self.shared.on_move.borrow_mut() = Some(Rc::new(callback));
        self
    }

    /// Recusa destinos de arrasto: recebe a chave arrastada e o pai de destino. O próprio
    /// item e os descendentes dele já são recusados.
    pub fn can_drop(self, can_drop: impl Fn(&str, Option<&str>) -> bool + 'static) -> Self {
        *self.shared.can_drop.borrow_mut() = Some(Rc::new(can_drop));
        self
    }

    /// Linhas de um [`DataGrid`](crate::DataGrid) do grupo `group` (`.drag_group(group)`)
    /// soltas sobre uma pasta da árvore (um [`TreeItem::folder`]). `DataGridDrop::target` é a
    /// chave da pasta. A pasta recebe a marca de "para dentro", e uma pasta fechada com filhos
    /// abre depois de o arrasto ficar parado sobre ela. Soltar numa folha ou entre as linhas
    /// não faz nada. Não muda o arrasto das linhas da própria árvore (`on_move`).
    pub fn on_grid_drop(
        self,
        group: impl Into<String>,
        callback: impl Fn(DataGridDrop) + 'static,
    ) -> Self {
        *self.shared.grid_drop.borrow_mut() = Some((group.into(), Rc::new(callback)));
        self
    }

    /// Recusa pastas de destino das linhas do `DataGrid` (`on_grid_drop`): recebe o que seria
    /// solto, com a pasta em `target`.
    pub fn can_grid_drop(self, can_drop: impl Fn(&DataGridDrop) -> bool + 'static) -> Self {
        *self.shared.can_grid_drop.borrow_mut() = Some(Rc::new(can_drop));
        self
    }

    /// O menu de contexto de cada linha, pela chave. O clique direito seleciona a linha antes
    /// de abrir.
    pub fn item_context_flyout(self, menu: impl Fn(&str) -> MenuFlyout + 'static) -> Self {
        *self.shared.item_menu.borrow_mut() = Some(Rc::new(menu));
        self
    }

    /// Desenha um selo de estado no canto do ícone dos itens em que `f(chave)` é `Some`, como o
    /// `InfoBadge` do WinUI. Sem ícone, o selo ocupa o espaço do ícone. A closure é reativa
    /// (lê os signals do app): só a linha é redesenhada quando ela muda, sem remontar os itens.
    /// O estado entra também no nome acessível da linha.
    pub fn item_status(self, f: impl Fn(&str) -> Option<ItemStatus> + 'static) -> Self {
        *self.shared.item_status.borrow_mut() = Some(Rc::new(f));
        self
    }

    /// A dica de cada linha, pela chave; reativa, como o `TabView::tab_tooltip`. A dica tem o
    /// id `{id da árvore}-{chave}-tooltip`. `None` ou texto vazio não abre.
    pub fn item_tooltip(self, f: impl Fn(&str) -> Option<String> + 'static) -> Self {
        *self.shared.item_tooltip.borrow_mut() = Some(Rc::new(f));
        self
    }

    /// A chave selecionada que aparece.
    pub fn selected(&self) -> Option<String> {
        self.shared.visible_selected()
    }

    /// Abre ou fecha a pasta `key`, como o chevron: o `TestApp` usa. Com filtro, vale só
    /// enquanto o filtro durar.
    pub fn set_open(&self, key: &str, open: bool) {
        self.shared.set_open(key, open);
    }

    fn clear_drag(&mut self) {
        self.drag = None;
        self.shared.set_drop(None);
        if self.shared.drag_key.get_untracked().is_some() {
            self.shared.drag_key.set(None);
        }
    }

    /// Mostra a marca do alvo e guarda o estado do arrasto.
    fn show_target(&mut self, source: &Source, pointer: Point, rect: Rect, target: Option<Target>) {
        let state = self.drag.get_or_insert(DragState {
            source: source.clone(),
            pointer,
            rect,
            target: None,
            held: Duration::ZERO,
        });
        state.pointer = pointer;
        state.rect = rect;
        if state.target != target {
            state.held = Duration::ZERO;
        }
        let mark = target
            .as_ref()
            .and_then(|t| t.mark.map(|mark| (t.row, mark)));
        state.target = target;
        self.shared.set_drop(mark);
    }
}

/// O destino de linhas de um `DataGrid` com o ponteiro em `pointer`: só uma pasta, para dentro.
fn grid_target_at(
    shared: &Shared,
    pointer: Point,
    rect: Rect,
    grid: &str,
    keys: &[String],
) -> Option<Target> {
    let flat = shared.flat_now();
    let height = shared.item_height();
    let offset = untrack(|| shared.link.offset_y());
    let y = (pointer.y - rect.y0 + offset).max(0.0);
    let index = (y / height) as usize;
    let row = flat.rows.get(index).filter(|row| row.folder)?;
    let can_grid_drop = shared.can_grid_drop.borrow().clone();
    if let Some(can_grid_drop) = can_grid_drop {
        let drop = DataGridDrop {
            source_grid: grid.to_owned(),
            keys: keys.to_vec(),
            target: Some(row.key.clone()),
        };
        if !can_grid_drop(&drop) {
            return None;
        }
    }
    Some(Target {
        row: index,
        mark: Some(DropMark::Into),
        parent: Some(row.key.clone()),
        position: 0,
        closed_folder: (row.has_children && !row.expanded).then(|| row.key.clone()),
        noop: false,
    })
}

/// O destino do arrasto da linha `source` com o ponteiro em `pointer`.
fn target_at(shared: &Shared, pointer: Point, rect: Rect, source: &str) -> Option<Target> {
    let flat = shared.flat_now();
    let n = flat.rows.len();
    if n == 0 {
        return None;
    }
    let height = shared.item_height();
    let offset = untrack(|| shared.link.offset_y());
    let y = (pointer.y - rect.y0 + offset).max(0.0);
    let src = flat.get(source)?;
    let (src_parent, src_index) = (src.parent.clone(), src.index);

    let (index, zone) = if y >= n as f64 * height {
        (n - 1, Zone::BelowAll)
    } else {
        let index = (y / height) as usize;
        let fraction = (y - index as f64 * height) / height;
        let row = &flat.rows[index];
        let zone = if row.folder {
            match fraction {
                f if f < 0.25 => Zone::Top,
                f if f > 0.75 => Zone::Bottom,
                _ => Zone::Into,
            }
        } else if fraction < 0.5 {
            Zone::Top
        } else {
            Zone::Bottom
        };
        (index, zone)
    };
    let row = &flat.rows[index];
    if row.key == source && zone != Zone::BelowAll {
        return None;
    }
    let (parent, raw, mark) = match zone {
        Zone::Top => (row.parent.clone(), row.index, DropMark::Top(row.depth)),
        Zone::Bottom if row.has_children && row.expanded => {
            (Some(row.key.clone()), 0, DropMark::Bottom(row.depth + 1))
        }
        Zone::Bottom => (
            row.parent.clone(),
            row.index + 1,
            DropMark::Bottom(row.depth),
        ),
        Zone::Into => {
            let count = shared
                .items
                .with_untracked(|items| items.find(&row.key).map_or(0, |i| i.children.len()));
            (Some(row.key.clone()), count, DropMark::Into)
        }
        Zone::BelowAll => (
            None,
            shared.items.with_untracked(Vec::len),
            DropMark::Bottom(0),
        ),
    };
    // Não cai dentro de si nem de um descendente.
    let mut ancestor = parent.clone();
    while let Some(key) = ancestor {
        if key == source {
            return None;
        }
        ancestor = flat.get(&key).and_then(|r| r.parent.clone());
    }
    let can_drop = shared.can_drop.borrow().clone();
    if let Some(can_drop) = can_drop
        && !can_drop(source, parent.as_deref())
    {
        return None;
    }
    let position = if parent == src_parent && src_index < raw {
        raw - 1
    } else {
        raw
    };
    let noop = parent == src_parent && position == src_index;
    let closed_folder =
        (zone == Zone::Into && row.has_children && !row.expanded).then(|| row.key.clone());
    Some(Target {
        row: index,
        mark: (!noop).then_some(mark),
        parent,
        position,
        closed_folder,
        noop,
    })
}

common_builders!(TreeView);
density_builder!(TreeView);

impl Widget for TreeView {
    fn kind(&self) -> &'static str {
        "tree_view"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn names_children(&self) -> bool {
        true
    }

    fn look(&self) -> Look {
        let shared = &self.shared;
        Look::TreeView(TreeViewVisual {
            rows: shared.flat_tracked(),
            selected: shared.visible_selected_tracked(),
            filter: shared.filter.get().get(),
        })
    }

    fn flex(&self) -> Option<Flex> {
        Some(Flex::default())
    }

    fn set_density(&mut self, density: Density) {
        self.shared.density.set(density);
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        let shared = &self.shared;
        *shared.id.borrow_mut() = self.common.id.clone();
        let mut scroll = ScrollViewer::new()
            .link_with_bars(Rc::clone(&shared.link))
            .generated()
            .grow(1)
            .child(Column::new().child(TreeRows::new(shared)));
        if let Some(id) = shared.sub("rolagem") {
            scroll = scroll.id(id);
        }
        vec![Box::new(scroll)]
    }

    fn handle(&mut self, event: &Event, cx: &mut EventCx) -> bool {
        let shared = Rc::clone(&self.shared);
        match event {
            Event::DragOver(data) if data.kind == shared.drag_kind() => {
                if shared.drag_key.get_untracked().as_deref() != Some(data.key.as_str()) {
                    shared.drag_key.set(Some(data.key.clone()));
                }
                let (pointer, rect) = (cx.pointer(), cx.rect());
                let target = target_at(&shared, pointer, rect, &data.key);
                let accepted = target.is_some();
                self.show_target(&Source::Row(data.key.clone()), pointer, rect, target);
                accepted
            }
            Event::DragOver(data) if shared.grid_kind().as_deref() == Some(&data.kind) => {
                let (grid, _, keys) = decode_drag(data);
                let (pointer, rect) = (cx.pointer(), cx.rect());
                let target = grid_target_at(&shared, pointer, rect, &grid, &keys);
                let accepted = target.is_some();
                self.show_target(&Source::Grid { grid, keys }, pointer, rect, target);
                accepted
            }
            Event::DragLeave => {
                self.clear_drag();
                true
            }
            Event::Drop(data) if data.kind == shared.drag_kind() => {
                let target = target_at(&shared, cx.pointer(), cx.rect(), &data.key);
                self.clear_drag();
                let on_move = shared.on_move.borrow().clone();
                if let (Some(target), Some(on_move)) = (target, on_move)
                    && !target.noop
                {
                    on_move(data.key.clone(), target.parent, target.position);
                }
                true
            }
            Event::Drop(data) if shared.grid_kind().as_deref() == Some(&data.kind) => {
                let (grid, _, keys) = decode_drag(data);
                let target = grid_target_at(&shared, cx.pointer(), cx.rect(), &grid, &keys);
                self.clear_drag();
                let on_drop = shared
                    .grid_drop
                    .borrow()
                    .as_ref()
                    .map(|(_, f)| Rc::clone(f));
                if let (Some(target), Some(on_drop)) = (target, on_drop) {
                    on_drop(DataGridDrop {
                        source_grid: grid,
                        keys,
                        target: target.parent,
                    });
                }
                true
            }
            _ => false,
        }
    }

    fn animating(&self) -> bool {
        true
    }

    fn ticking(&self) -> bool {
        self.drag.is_some()
    }

    fn tick(&mut self, dt: Duration) {
        let shared = Rc::clone(&self.shared);
        let Some(state) = &mut self.drag else {
            return;
        };
        let (pointer, rect) = (state.pointer, state.rect);
        let source = state.source.clone();

        // Rolagem automática: a velocidade cresce da borda de uma linha para fora.
        let edge = shared.item_height();
        let speed = if pointer.y < rect.y0 + edge {
            -(1.0 - ((pointer.y - rect.y0) / edge).clamp(0.0, 1.0))
        } else if pointer.y > rect.y1 - edge {
            ((pointer.y - (rect.y1 - edge)) / edge).clamp(0.0, 1.0)
        } else {
            0.0
        };
        if speed != 0.0 {
            shared
                .link
                .scroll_by_y(speed * AUTO_SCROLL * dt.as_secs_f64());
        }

        // O conteúdo andou sob o ponteiro parado: o alvo muda.
        let target = match &source {
            Source::Row(key) => target_at(&shared, pointer, rect, key),
            Source::Grid { grid, keys } => grid_target_at(&shared, pointer, rect, grid, keys),
        };
        let mut held = state.held + dt;
        if state.target != target {
            held = Duration::ZERO;
        }
        let open = target
            .as_ref()
            .and_then(|t| t.closed_folder.clone())
            .filter(|_| held >= OPEN_DELAY);
        if open.is_some() {
            held = Duration::ZERO;
        }
        self.show_target(&source, pointer, rect, target);
        if let Some(state) = &mut self.drag {
            state.held = held;
        }
        if let Some(key) = open {
            shared.set_open(&key, true);
        }
    }

    fn text(&self) -> Option<String> {
        self.shared.visible_selected()
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

impl fmt::Debug for TreeView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TreeView")
            .field("common", &self.common)
            .finish_non_exhaustive()
    }
}
