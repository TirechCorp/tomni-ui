//! O estado que o `DataGrid`, o cabeçalho, as linhas e o fragmento dividem, e o que o teclado
//! e o mouse fazem nele.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use tomni_ui_reactive::prelude::*;

use super::model::{
    DataGridColumn, DataGridDrop, DataGridFlat, DataGridRow, SortDirection, arrange,
};
use super::visual::row_height;
use crate::cx::EventCx;
use crate::density::Density;
use crate::drag::DragData;
use crate::event::{Key, Modifiers};
use crate::flyout::MenuFlyout;
use crate::scroll_viewer::ScrollLink;
use crate::tree_view::fold;

pub(crate) type KeyFn = Rc<dyn Fn(String)>;
pub(crate) type KeysFn = Rc<dyn Fn(Vec<String>)>;
pub(crate) type RenameFn = Rc<dyn Fn(String, String)>;
pub(crate) type OnKeyFn = Rc<dyn Fn(Key, Modifiers) -> bool>;
pub(crate) type MenuFn = Rc<dyn Fn(&str) -> MenuFlyout>;
pub(crate) type DropFn = Rc<dyn Fn(DataGridDrop)>;
pub(crate) type CanDropFn = Rc<dyn Fn(&DataGridDrop) -> bool>;
pub(crate) type FilesFn = Rc<dyn Fn(Vec<PathBuf>, Option<String>)>;

/// O prefixo do tipo do `DragData` das linhas; o grupo vem depois.
const DRAG_KIND: &str = "data_grid_rows";
/// Separa o id do grid e as chaves no `DragData::key`.
const SEP: char = '\u{1f}';

/// O tipo do `DragData` das linhas de um grid do grupo `group`: o `TreeView` usa para aceitar
/// linhas soltas sobre as pastas dele.
pub(crate) fn drag_kind_of_group(group: &str) -> String {
    format!("{DRAG_KIND}:{group}")
}

/// O que um arrasto de linhas leva: o grid de origem, a linha que foi pressionada e as
/// chaves arrastadas.
pub(crate) fn decode_drag(data: &DragData) -> (String, String, Vec<String>) {
    let mut parts = data.key.split(SEP);
    let grid = parts.next().unwrap_or_default().to_owned();
    let pressed = parts.next().unwrap_or_default().to_owned();
    (grid, pressed, parts.map(str::to_owned).collect())
}

/// Quantas linhas a janela do fragmento anda de cada vez.
pub(crate) const BLOCK: usize = 32;

/// O que o clique que soltou a linha ainda precisa fazer (o que o botão pressionado deixou
/// para o fim, para o arrasto da seleção inteira começar sem desfazê-la).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Click {
    /// Nada.
    None,
    /// Clique simples numa linha da seleção: fica só ela.
    Collapse,
    /// Ctrl+clique numa linha da seleção: sai dela.
    ToggleOff,
}

pub(crate) struct Shared {
    pub id: RefCell<Option<String>>,
    pub rows: Signal<Vec<DataGridRow>>,
    pub selected: Signal<BTreeSet<String>>,
    pub columns: Signal<Arc<Vec<DataGridColumn>>>,
    /// As larguras que o usuário deu, por coluna.
    pub widths: Signal<Vec<Option<f64>>>,
    pub sort: Cell<Signal<Option<(String, SortDirection)>>>,
    pub filter: Cell<Signal<String>>,
    pub renaming: Cell<Signal<Option<String>>>,
    /// As colunas com as larguras do usuário.
    pub layout: Memo<Arc<Vec<DataGridColumn>>>,
    /// As linhas visíveis.
    pub flat: Cell<Memo<Arc<DataGridFlat>>>,
    pub link: Rc<ScrollLink>,
    pub density: Cell<Density>,
    pub group: RefCell<String>,
    pub empty_text: RefCell<String>,
    pub on_invoke: RefCell<Option<KeyFn>>,
    pub on_rename: RefCell<Option<RenameFn>>,
    pub on_delete: RefCell<Option<KeysFn>>,
    pub on_key: RefCell<Option<OnKeyFn>>,
    pub item_menu: RefCell<Option<MenuFn>>,
    pub on_drop: RefCell<Option<DropFn>>,
    pub can_drop: RefCell<Option<CanDropFn>>,
    pub on_files: RefCell<Option<FilesFn>>,
    /// A âncora das faixas (Shift).
    pub anchor: RefCell<Option<String>>,
    /// A linha do teclado: a que tem o foco ou, sem foco, a que o teclado volta a ter.
    pub cursor: Signal<Option<String>>,
    /// A linha que recebe o arrasto (uma pasta), para ela pintar a marca.
    pub drop: Signal<Option<String>>,
    /// Se o arrasto vai soltar no fundo.
    pub drop_background: Signal<bool>,
    /// A linha com o foco, para a janela mantê-la montada.
    pub focus_key: Signal<Option<String>>,
    /// A linha que começou um arrasto, para ela continuar montada.
    pub drag_key: Signal<Option<String>>,
    /// As linhas montadas: `[início, fim)` em blocos de [`BLOCK`].
    pub window: Signal<(usize, usize)>,
}

impl Shared {
    pub fn new(rows: Signal<Vec<DataGridRow>>, selected: Signal<BTreeSet<String>>) -> Rc<Self> {
        let columns = signal(Arc::new(Vec::new()));
        let widths = signal(Vec::new());
        let sort = signal(None);
        let filter = signal(String::new());
        let layout = memo(move || {
            let columns: Arc<Vec<DataGridColumn>> = columns.get();
            let widths = widths.get();
            Arc::new(
                columns
                    .iter()
                    .enumerate()
                    .map(|(i, column)| {
                        let mut column = column.clone();
                        if let Some(Some(w)) = widths.get(i) {
                            column.width = super::ColumnWidth::Pixels(*w);
                        }
                        column
                    })
                    .collect(),
            )
        });
        let shared = Rc::new(Self {
            id: RefCell::new(None),
            rows,
            selected,
            columns,
            widths,
            sort: Cell::new(sort),
            filter: Cell::new(filter),
            renaming: Cell::new(signal(None)),
            layout,
            flat: Cell::new(flat_memo(rows, sort, filter, columns)),
            link: ScrollLink::new(0.0),
            density: Cell::new(Density::Standard),
            group: RefCell::new(String::new()),
            empty_text: RefCell::new(String::new()),
            on_invoke: RefCell::new(None),
            on_rename: RefCell::new(None),
            on_delete: RefCell::new(None),
            on_key: RefCell::new(None),
            item_menu: RefCell::new(None),
            on_drop: RefCell::new(None),
            can_drop: RefCell::new(None),
            on_files: RefCell::new(None),
            anchor: RefCell::new(None),
            cursor: signal(None),
            drop: signal(None),
            drop_background: signal(false),
            focus_key: signal(None),
            drag_key: signal(None),
            window: signal((0, BLOCK)),
        });
        shared.watch_columns();
        shared
    }

    /// Refaz as linhas visíveis, depois que o app troca o `sort` ou o `filter`.
    pub fn rebuild(&self) {
        self.flat.set(flat_memo(
            self.rows,
            self.sort.get(),
            self.filter.get(),
            self.columns,
        ));
    }

    /// Zera as larguras do usuário quando o app troca as colunas.
    fn watch_columns(&self) {
        let (columns, widths) = (self.columns, self.widths);
        effect(move || {
            let n = columns.get().len();
            if widths.get_untracked().len() != n {
                widths.set(vec![None; n]);
            }
        });
    }

    pub fn flat_now(&self) -> Arc<DataGridFlat> {
        self.flat.get().get_untracked()
    }

    pub fn flat_tracked(&self) -> Arc<DataGridFlat> {
        self.flat.get().get()
    }

    pub fn row_height(&self) -> f64 {
        row_height(self.density.get())
    }

    /// Id de uma parte: `{id}-{suffix}`.
    pub fn sub(&self, suffix: &str) -> Option<String> {
        self.id.borrow().as_ref().map(|id| format!("{id}-{suffix}"))
    }

    /// Refaz a janela de linhas montadas a partir da rolagem.
    pub fn update_window(&self) {
        let (offset, viewport) = (self.link.offset_y(), self.link.viewport_height());
        let height = self.row_height();
        let first = ((offset - viewport) / height).floor().max(0.0) as usize;
        let last = ((offset + 2.0 * viewport) / height).ceil().max(0.0) as usize;
        let window = (first / BLOCK * BLOCK, last.div_ceil(BLOCK).max(1) * BLOCK);
        if self.window.get_untracked() != window {
            self.window.set(window);
        }
    }

    // --- Ordenação -------------------------------------------------------------------------

    /// A coluna ordenada (o índice) e o sentido.
    pub fn sort_now(&self) -> Option<(usize, SortDirection)> {
        sort_index(
            &self.columns.get_untracked(),
            &self.sort.get().get_untracked(),
        )
    }

    /// O clique no cabeçalho da coluna `index`: ordena por ela ou inverte o sentido.
    pub fn sort_by(&self, index: usize) {
        let columns = self.columns.get_untracked();
        let Some(column) = columns.get(index).filter(|c| c.sortable) else {
            return;
        };
        let sort = self.sort.get();
        let next = match sort.get_untracked() {
            Some((key, direction)) if key == column.key => (key, direction.flip()),
            _ => (column.key.clone(), SortDirection::Ascending),
        };
        sort.set(Some(next));
    }

    // --- Seleção ---------------------------------------------------------------------------

    /// As chaves selecionadas que aparecem, na ordem das linhas.
    pub fn selected_keys(&self) -> Vec<String> {
        let flat = self.flat_now();
        self.selected.with_untracked(|selected| {
            flat.rows
                .iter()
                .filter(|row| selected.contains(&row.key))
                .map(|row| row.key.clone())
                .collect()
        })
    }

    pub fn is_selected(&self, key: &str) -> bool {
        self.selected.with_untracked(|s| s.contains(key))
    }

    fn set_selection(&self, set: BTreeSet<String>) {
        if self.selected.with_untracked(|s| *s != set) {
            self.selected.set(set);
        }
    }

    pub fn select_only(&self, key: &str) {
        self.set_selection(BTreeSet::from([key.to_owned()]));
        *self.anchor.borrow_mut() = Some(key.to_owned());
    }

    /// Seleciona a faixa entre a âncora e `key`.
    pub fn select_range(&self, key: &str) {
        let flat = self.flat_now();
        let anchor = self
            .anchor
            .borrow()
            .clone()
            .filter(|a| flat.position(a).is_some())
            .or_else(|| self.cursor.get_untracked())
            .unwrap_or_else(|| key.to_owned());
        let (Some(a), Some(b)) = (flat.position(&anchor), flat.position(key)) else {
            self.select_only(key);
            return;
        };
        let (from, to) = (a.min(b), a.max(b));
        self.set_selection(
            flat.rows[from..=to]
                .iter()
                .map(|row| row.key.clone())
                .collect(),
        );
        *self.anchor.borrow_mut() = Some(anchor);
    }

    pub fn select_all(&self) {
        let flat = self.flat_now();
        self.set_selection(flat.rows.iter().map(|row| row.key.clone()).collect());
    }

    /// Põe ou tira `key` da seleção.
    pub fn toggle(&self, key: &str) {
        let mut set = self.selected.get_untracked();
        if !set.remove(key) {
            set.insert(key.to_owned());
        }
        self.selected.set(set);
        *self.anchor.borrow_mut() = Some(key.to_owned());
    }

    pub fn set_cursor(&self, key: &str) {
        if self.cursor.get_untracked().as_deref() != Some(key) {
            self.cursor.set(Some(key.to_owned()));
        }
    }

    /// A linha que é a parada de Tab: a do teclado, se aparece, ou a primeira.
    pub fn tab_key(&self) -> Option<String> {
        let flat = self.flat_now();
        self.cursor
            .get_untracked()
            .filter(|key| flat.position(key).is_some())
            .or_else(|| flat.rows.first().map(|row| row.key.clone()))
    }

    /// O ponteiro desceu na linha `key`: ajusta a seleção e diz o que o clique ainda faz.
    pub fn pointer_down(&self, key: &str, mods: Modifiers) -> Click {
        self.set_cursor(key);
        if mods.shift {
            self.select_range(key);
            return Click::None;
        }
        let selected = self.is_selected(key);
        if mods.command() {
            if selected {
                return Click::ToggleOff;
            }
            self.toggle(key);
            return Click::None;
        }
        if selected {
            let many = self.selected.with_untracked(|s| s.len() > 1);
            return if many { Click::Collapse } else { Click::None };
        }
        self.select_only(key);
        Click::None
    }

    /// O botão subiu sobre a linha sem arrastar.
    pub fn clicked(&self, key: &str, click: Click) {
        match click {
            Click::None => {}
            Click::Collapse => self.select_only(key),
            Click::ToggleOff => self.toggle(key),
        }
    }

    // --- Teclado ---------------------------------------------------------------------------

    pub fn invoke(&self, key: &str) {
        let callback = self.on_invoke.borrow().clone();
        if let Some(callback) = callback {
            callback(key.to_owned());
        }
    }

    /// Leva o foco à linha, que fica montada até receber.
    pub fn focus(&self, key: &str, cx: &mut EventCx) {
        self.set_cursor(key);
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

    /// Quantas linhas cabem numa página.
    fn page(&self) -> usize {
        let viewport = untrack(|| self.link.viewport_height());
        ((viewport / self.row_height()).floor() as usize).max(1)
    }

    /// Teclas de uma linha com foco. Devolve se usou.
    pub fn key_down(&self, key: &str, pressed: Key, mods: Modifiers, cx: &mut EventCx) -> bool {
        let flat = self.flat_now();
        let Some(pos) = flat.position(key) else {
            return false;
        };
        let last = flat.rows.len() - 1;
        let target = if mods.alt {
            None
        } else {
            match pressed {
                Key::Up => Some(pos.saturating_sub(1)),
                Key::Down => Some((pos + 1).min(last)),
                Key::Home => Some(0),
                Key::End => Some(last),
                Key::PageUp => Some(pos.saturating_sub(self.page())),
                Key::PageDown => Some((pos + self.page()).min(last)),
                _ => None,
            }
        };
        if let Some(target) = target {
            let to = flat.rows[target].key.clone();
            if mods.shift {
                self.select_range(&to);
            } else if !mods.command() {
                self.select_only(&to);
            }
            self.focus(&to, cx);
            return true;
        }
        if !mods.alt && mods.command() {
            match pressed {
                Key::Char('a' | 'A') => {
                    self.select_all();
                    return true;
                }
                Key::Space => {
                    self.toggle(key);
                    return true;
                }
                _ => {}
            }
        } else if !mods.alt {
            match pressed {
                Key::Space => {
                    self.select_only(key);
                    return true;
                }
                Key::Enter => {
                    self.invoke(key);
                    return true;
                }
                Key::F(2) => return self.start_rename(key),
                Key::Delete => return self.delete(key),
                _ => {}
            }
        }
        let on_key = self.on_key.borrow().clone();
        on_key.is_some_and(|on_key| on_key(pressed, mods))
    }

    /// Delete: apaga a seleção (se a linha do foco está nela) ou só a linha do foco.
    fn delete(&self, key: &str) -> bool {
        let callback = self.on_delete.borrow().clone();
        let Some(callback) = callback else {
            return false;
        };
        let keys = if self.is_selected(key) {
            self.selected_keys()
        } else {
            vec![key.to_owned()]
        };
        callback(keys);
        true
    }

    /// Letra digitada com o foco numa linha: vai para a próxima cujo nome começa com ela,
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
            .find(|row| fold(row.label()).starts_with(&needle));
        if let Some(row) = found {
            self.select_only(&row.key);
            self.focus(&row.key, cx);
        }
        true
    }

    // --- Arrasto ---------------------------------------------------------------------------

    /// Tipo do item no arrasto: só os grids do mesmo grupo (por padrão, só este) aceitam.
    pub fn drag_kind(&self) -> String {
        let group = self.group.borrow();
        if group.is_empty() {
            format!("{DRAG_KIND}@{:p}", self as *const Self)
        } else {
            drag_kind_of_group(&group)
        }
    }

    /// O `DragData` de uma linha: leva a seleção inteira, se a linha está nela.
    pub fn drag_data(&self, key: &str) -> Option<DragData> {
        let flat = self.flat_now();
        let row = flat.get(key)?;
        let keys = if self.is_selected(key) {
            self.selected_keys()
        } else {
            vec![key.to_owned()]
        };
        let label = if keys.len() > 1 {
            format!("{} itens", keys.len())
        } else {
            row.label().to_owned()
        };
        let mut encoded = self.id.borrow().clone().unwrap_or_default();
        encoded.push(SEP);
        encoded.push_str(key);
        for key in &keys {
            encoded.push(SEP);
            encoded.push_str(key);
        }
        Some(DragData {
            kind: self.drag_kind(),
            key: encoded,
            label,
        })
    }

    /// O que um arrasto de linhas leva: o grid de origem, a linha que foi pressionada e as
    /// chaves arrastadas.
    pub fn decode(data: &DragData) -> (String, String, Vec<String>) {
        decode_drag(data)
    }

    pub fn set_drop(&self, row: Option<String>, background: bool) {
        if self.drop.get_untracked() != row {
            self.drop.set(row);
        }
        if self.drop_background.get_untracked() != background {
            self.drop_background.set(background);
        }
    }

    /// A linha sob o ponto de y `y` (na janela), se há uma, dado o retângulo do grid.
    pub fn row_at(&self, y: f64, grid_top: f64) -> Option<Arc<DataGridRow>> {
        let flat = self.flat_now();
        let header = super::visual::header_height(self.density.get());
        let offset = untrack(|| self.link.offset_y());
        let local = y - grid_top - header + offset;
        if local < 0.0 {
            return None;
        }
        let index = (local / self.row_height()) as usize;
        flat.rows.get(index).cloned()
    }

    /// O destino de um arrasto com o ponteiro em `y`: `None` se não aceita; senão a linha
    /// de destino (uma pasta) ou `None` para o fundo.
    pub fn drop_target(
        &self,
        data: &DragData,
        y: f64,
        grid_top: f64,
    ) -> Option<(DataGridDrop, Option<String>)> {
        let (source_grid, _, keys) = Self::decode(data);
        let own = self.id.borrow().clone().unwrap_or_default();
        let row = self
            .row_at(y, grid_top)
            .filter(|row| row.drop_target && !keys.contains(&row.key));
        let target = row.map(|row| row.key.clone());
        if target.is_none() && source_grid == own {
            // Soltar no fundo do próprio grid não muda nada.
            return None;
        }
        let drop = DataGridDrop {
            source_grid,
            keys,
            target: target.clone(),
        };
        let can_drop = self.can_drop.borrow().clone();
        if let Some(can_drop) = can_drop
            && !can_drop(&drop)
        {
            return None;
        }
        Some((drop, target))
    }
}

/// O índice da coluna de chave `key` nas `columns`.
pub(crate) fn sort_index(
    columns: &[DataGridColumn],
    sort: &Option<(String, SortDirection)>,
) -> Option<(usize, SortDirection)> {
    let (key, direction) = sort.as_ref()?;
    let index = columns.iter().position(|c| &c.key == key)?;
    Some((index, *direction))
}

fn flat_memo(
    rows: Signal<Vec<DataGridRow>>,
    sort: Signal<Option<(String, SortDirection)>>,
    filter: Signal<String>,
    columns: Signal<Arc<Vec<DataGridColumn>>>,
) -> Memo<Arc<DataGridFlat>> {
    memo(move || {
        let columns = columns.get();
        let sort = sort_index(&columns, &sort.get());
        Arc::new(rows.with(|rows| filter.with(|filter| arrange(rows, filter, sort))))
    })
}
