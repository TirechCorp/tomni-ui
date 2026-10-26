//! O estado que o `TreeView`, as linhas e o fragmento dividem, e o que o teclado e o mouse
//! fazem nele.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::rc::Rc;
use std::sync::Arc;

use tomni_ui_reactive::prelude::*;

use super::model::{TreeItem, fold};
use super::rows::{TreeFlat, flatten};
use super::visual::{DropMark, ItemStatus, item_height};
use crate::cx::EventCx;
use crate::data_grid::DataGridDrop;
use crate::density::Density;
use crate::event::{Key, Modifiers};
use crate::flyout::MenuFlyout;
use crate::scroll_viewer::ScrollLink;

pub(crate) type KeyFn = Rc<dyn Fn(String)>;
pub(crate) type RenameFn = Rc<dyn Fn(String, String)>;
pub(crate) type MoveFn = Rc<dyn Fn(String, Option<String>, usize)>;
pub(crate) type CanDropFn = Rc<dyn Fn(&str, Option<&str>) -> bool>;
pub(crate) type MenuFn = Rc<dyn Fn(&str) -> MenuFlyout>;
pub(crate) type StatusFn = Rc<dyn Fn(&str) -> Option<ItemStatus>>;
pub(crate) type TooltipFn = Rc<dyn Fn(&str) -> Option<String>>;
pub(crate) type GridDropFn = Rc<dyn Fn(DataGridDrop)>;
pub(crate) type CanGridDropFn = Rc<dyn Fn(&DataGridDrop) -> bool>;

/// O tipo do `DragData` das linhas.
pub(crate) const DRAG_KIND: &str = "tree_view_item";

/// Onde o arrasto vai cair: a linha (pelo índice nas linhas visíveis) e a marca nela.
pub(crate) type DropTarget = Option<(usize, DropMark)>;

pub(crate) struct Shared {
    pub id: RefCell<Option<String>>,
    pub items: Signal<Vec<TreeItem>>,
    pub selected: Signal<Option<String>>,
    /// As pastas abertas: a do app ou, sem ela, uma interna.
    pub expanded: Cell<Signal<BTreeSet<String>>>,
    pub filter: Cell<Signal<String>>,
    pub renaming: Cell<Signal<Option<String>>>,
    /// Com o filtro, as pastas que o usuário fechou; zerado quando o filtro muda.
    pub filter_collapsed: Signal<BTreeSet<String>>,
    /// As linhas visíveis.
    pub flat: Cell<Memo<Arc<TreeFlat>>>,
    pub link: Rc<ScrollLink>,
    pub density: Cell<Density>,
    pub on_invoke: RefCell<Option<KeyFn>>,
    pub on_rename: RefCell<Option<RenameFn>>,
    pub on_delete: RefCell<Option<KeyFn>>,
    pub on_move: RefCell<Option<MoveFn>>,
    pub can_drop: RefCell<Option<CanDropFn>>,
    /// O grupo de arrasto dos `DataGrid`s aceitos e o callback de linhas soltas sobre as pastas.
    pub grid_drop: RefCell<Option<(String, GridDropFn)>>,
    pub can_grid_drop: RefCell<Option<CanGridDropFn>>,
    pub item_menu: RefCell<Option<MenuFn>>,
    /// O selo de estado e a dica de cada linha.
    pub item_status: RefCell<Option<StatusFn>>,
    pub item_tooltip: RefCell<Option<TooltipFn>>,
    /// Onde o arrasto vai cair, para as linhas pintarem a marca.
    pub drop: Signal<DropTarget>,
    /// A linha com o foco, para a janela mantê-la montada.
    pub focus_key: Signal<Option<String>>,
    /// A origem do arrasto em andamento.
    pub drag_key: Signal<Option<String>>,
    /// As linhas montadas: `[início, fim)` em blocos de [`BLOCK`].
    pub window: Signal<(usize, usize)>,
}

/// Quantas linhas a janela do fragmento anda de cada vez.
pub(crate) const BLOCK: usize = 32;

impl Shared {
    pub fn new(items: Signal<Vec<TreeItem>>, selected: Signal<Option<String>>) -> Rc<Self> {
        let expanded = signal(BTreeSet::new());
        let filter = signal(String::new());
        let renaming = signal(None);
        let filter_collapsed = signal(BTreeSet::new());
        let shared = Rc::new(Self {
            id: RefCell::new(None),
            items,
            selected,
            expanded: Cell::new(expanded),
            filter: Cell::new(filter),
            renaming: Cell::new(renaming),
            filter_collapsed,
            flat: Cell::new(flat_memo(items, expanded, filter, filter_collapsed)),
            link: ScrollLink::new(0.0),
            density: Cell::new(Density::Standard),
            on_invoke: RefCell::new(None),
            on_rename: RefCell::new(None),
            on_delete: RefCell::new(None),
            on_move: RefCell::new(None),
            can_drop: RefCell::new(None),
            grid_drop: RefCell::new(None),
            can_grid_drop: RefCell::new(None),
            item_menu: RefCell::new(None),
            item_status: RefCell::new(None),
            item_tooltip: RefCell::new(None),
            drop: signal(None),
            focus_key: signal(None),
            drag_key: signal(None),
            window: signal((0, BLOCK)),
        });
        shared.watch_filter();
        shared
    }

    /// Refaz o que depende do `expanded` e do `filter`, depois que o app troca um deles.
    pub fn rebuild(&self) {
        self.flat.set(flat_memo(
            self.items,
            self.expanded.get(),
            self.filter.get(),
            self.filter_collapsed,
        ));
        self.watch_filter();
    }

    /// Zera `filter_collapsed` quando o texto do filtro muda.
    fn watch_filter(&self) {
        let (filter, collapsed) = (self.filter.get(), self.filter_collapsed);
        effect(move || {
            let _ = filter.get();
            if !collapsed.get_untracked().is_empty() {
                collapsed.set(BTreeSet::new());
            }
        });
    }

    /// O estado do item `key` (rastreado: lê os signals da closure do app).
    pub fn status_of(&self, key: &str) -> Option<ItemStatus> {
        let f = self.item_status.borrow().clone();
        f.and_then(|f| f(key))
    }

    pub fn flat_now(&self) -> Arc<TreeFlat> {
        self.flat.get().get_untracked()
    }

    pub fn flat_tracked(&self) -> Arc<TreeFlat> {
        self.flat.get().get()
    }

    /// Se há um filtro ativo (texto que não é só espaços).
    pub fn filtering(&self) -> bool {
        !self.filter.get().get_untracked().trim().is_empty()
    }

    pub fn item_height(&self) -> f64 {
        item_height(self.density.get())
    }

    pub fn set_drop(&self, target: DropTarget) {
        if self.drop.get_untracked() != target {
            self.drop.set(target);
        }
    }

    /// Tipo do item no arrasto: só este `TreeView` aceita as linhas dele.
    pub fn drag_kind(&self) -> String {
        format!("{DRAG_KIND}@{:p}", self as *const Self)
    }

    /// O tipo do arrasto de linhas de `DataGrid` que este `TreeView` aceita, se aceita.
    pub fn grid_kind(&self) -> Option<String> {
        let grid_drop = self.grid_drop.borrow();
        grid_drop
            .as_ref()
            .map(|(group, _)| crate::data_grid::drag_kind_of_group(group))
    }

    /// Refaz a janela de linhas montadas a partir da rolagem.
    pub fn update_window(&self) {
        let (offset, viewport) = (self.link.offset_y(), self.link.viewport_height());
        let height = self.item_height();
        let first = ((offset - viewport) / height).floor().max(0.0) as usize;
        let last = ((offset + 2.0 * viewport) / height).ceil().max(0.0) as usize;
        let window = (first / BLOCK * BLOCK, last.div_ceil(BLOCK).max(1) * BLOCK);
        if self.window.get_untracked() != window {
            self.window.set(window);
        }
    }

    /// Id de uma parte: `{id}-{suffix}`.
    pub fn sub(&self, suffix: &str) -> Option<String> {
        self.id.borrow().as_ref().map(|id| format!("{id}-{suffix}"))
    }

    /// A chave selecionada, se a linha aparece (a que sumiu vale `None`, sem mexer no signal).
    pub fn visible_selected(&self) -> Option<String> {
        let key = self.selected.get_untracked()?;
        self.flat_now().position(&key).map(|_| key)
    }

    /// O mesmo, rastreado: para o desenho.
    pub fn visible_selected_tracked(&self) -> Option<String> {
        let key = self.selected.get()?;
        self.flat_tracked().position(&key).map(|_| key)
    }

    pub fn select(&self, key: &str) {
        if self.selected.get_untracked().as_deref() != Some(key) {
            self.selected.set(Some(key.to_owned()));
        }
    }

    /// Abre ou fecha a pasta, no `expanded` ou, com filtro, no `filter_collapsed`.
    pub fn set_open(&self, key: &str, open: bool) {
        if self.filtering() {
            let closed = self.filter_collapsed.get_untracked().contains(key);
            if closed == open {
                self.filter_collapsed.update(|set| {
                    if open {
                        set.remove(key);
                    } else {
                        set.insert(key.to_owned());
                    }
                });
            }
        } else {
            let expanded = self.expanded.get();
            if expanded.get_untracked().contains(key) != open {
                expanded.update(|set| {
                    if open {
                        set.insert(key.to_owned());
                    } else {
                        set.remove(key);
                    }
                });
            }
        }
    }

    /// Alterna a pasta, se tem filhos.
    pub fn toggle(&self, key: &str) {
        let flat = self.flat_now();
        if let Some(row) = flat.get(key).filter(|r| r.has_children) {
            self.set_open(key, !row.expanded);
        }
    }

    pub fn invoke(&self, key: &str) {
        let callback = self.on_invoke.borrow().clone();
        if let Some(callback) = callback {
            callback(key.to_owned());
        }
    }

    /// Seleciona a linha e leva o foco a ela.
    pub fn go(&self, key: &str, cx: &mut EventCx) {
        self.select(key);
        if let Some(id) = self.sub(key) {
            cx.focus_later(id);
        }
    }

    pub fn start_rename(&self, key: &str) -> bool {
        if self.on_rename.borrow().is_none() {
            return false;
        }
        self.renaming.get().set(Some(key.to_owned()));
        true
    }

    /// Teclas de uma linha com foco. Devolve se usou.
    pub fn key_down(&self, key: &str, pressed: Key, mods: Modifiers, cx: &mut EventCx) -> bool {
        if mods.command() || mods.alt {
            return false;
        }
        let flat = self.flat_now();
        let Some(pos) = flat.position(key) else {
            return false;
        };
        let rows = &flat.rows;
        let row = &rows[pos];
        match pressed {
            Key::Up if pos > 0 => self.go(&rows[pos - 1].key, cx),
            Key::Down if pos + 1 < rows.len() => self.go(&rows[pos + 1].key, cx),
            Key::Up | Key::Down => {}
            Key::Home => self.go(&rows[0].key, cx),
            Key::End => self.go(&rows[rows.len() - 1].key, cx),
            Key::Right => {
                if row.has_children && !row.expanded {
                    self.set_open(key, true);
                } else if row.has_children
                    && let Some(child) = rows
                        .get(pos + 1)
                        .filter(|next| next.parent.as_deref() == Some(key))
                {
                    self.go(&child.key, cx);
                }
            }
            Key::Left => {
                if row.has_children && row.expanded {
                    self.set_open(key, false);
                } else if let Some(parent) = row.parent.as_deref()
                    && flat.position(parent).is_some()
                {
                    self.go(parent, cx);
                }
            }
            Key::Enter => {
                self.invoke(key);
                self.toggle(key);
            }
            Key::F(2) => return self.start_rename(key),
            Key::Delete => {
                let callback = self.on_delete.borrow().clone();
                let Some(callback) = callback else {
                    return false;
                };
                callback(key.to_owned());
            }
            _ => return false,
        }
        true
    }

    /// Letra digitada com o foco numa linha: vai para a próxima cujo rótulo começa com ela,
    /// dando a volta.
    pub fn type_ahead(&self, key: &str, text: &str, cx: &mut EventCx) -> bool {
        let Some(first) = text.chars().next().filter(|c| c.is_alphanumeric()) else {
            return false;
        };
        let needle = fold(&first.to_string());
        let flat = self.flat_now();
        let Some(pos) = flat.position(key) else {
            return false;
        };
        let n = flat.rows.len();
        let found = (1..=n)
            .map(|step| &flat.rows[(pos + step) % n])
            .find(|row| fold(&row.label).starts_with(&needle));
        if let Some(row) = found {
            self.go(&row.key, cx);
        }
        true
    }
}

fn flat_memo(
    items: Signal<Vec<TreeItem>>,
    expanded: Signal<BTreeSet<String>>,
    filter: Signal<String>,
    collapsed: Signal<BTreeSet<String>>,
) -> Memo<Arc<TreeFlat>> {
    memo(move || {
        Arc::new(items.with(|items| {
            expanded.with(|expanded| {
                filter.with(|filter| {
                    collapsed.with(|collapsed| flatten(items, expanded, filter, collapsed))
                })
            })
        }))
    })
}
