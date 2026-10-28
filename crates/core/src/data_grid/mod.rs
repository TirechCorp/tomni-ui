//! `DataGrid`: tabela de linhas e colunas com cabeçalho que ordena, seleção múltipla, filtro,
//! renomear e arrastar.

mod header;
mod item;
mod model;
mod shared;
mod visual;

use std::collections::BTreeSet;
use std::fmt;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use kurbo::{Point, Rect};
use tomni_ui_reactive::prelude::*;

pub use model::{
    CellValue, ColumnWidth, DATA_GRID_MIN_COLUMN, DataGridColumn, DataGridDrop, DataGridFlat,
    DataGridRow, RowTone, SortDirection,
};
pub use visual::{
    DATA_GRID_CELL_PADDING, DATA_GRID_DIVIDER_HIT, DATA_GRID_HEADER_HEIGHT,
    DATA_GRID_HEADER_HEIGHT_COMPACT, DATA_GRID_ICON, DATA_GRID_ICON_GAP, DATA_GRID_ROW_HEIGHT,
    DATA_GRID_ROW_HEIGHT_COMPACT, DataGridHeaderVisual, DataGridRowVisual, DataGridVisual,
    data_grid_cells,
};

use crate::containers::Column;
use crate::cx::EventCx;
use crate::density::Density;
use crate::drag::DragData;
use crate::event::{Event, Key, Modifiers};
use crate::flyout::MenuFlyout;
use crate::scroll_viewer::ScrollViewer;
use crate::widget::{Common, Flex, Look, Widget, common_builders, density_builder};
use header::GridHeader;
use item::GridRows;
use shared::Shared;
pub(crate) use shared::{decode_drag, drag_kind_of_group};
use visual::header_height;

/// Velocidade máxima da rolagem automática do arrasto, nas bordas, em px/s.
const AUTO_SCROLL: f64 = 600.0;

/// Tabela de linhas e colunas, como o `DataGrid` do Windows Community Toolkit com a seleção
/// do `ListView` do WinUI.
///
/// ```
/// use std::collections::BTreeSet;
/// use tomni_ui_core::{CellValue, DataGrid, DataGridColumn, DataGridRow, SortDirection};
/// use tomni_ui_reactive::prelude::*;
///
/// # let scope = Scope::root();
/// # scope.run(|| {
/// let linhas = signal(vec![
///     DataGridRow::new("a")
///         .cell("Alfa", CellValue::Text("Alfa".into()))
///         .cell("12", CellValue::Number(12.0)),
///     DataGridRow::new("b")
///         .cell("Beta", CellValue::Text("Beta".into()))
///         .cell("3", CellValue::Number(3.0)),
/// ]);
/// let selecionadas = signal(BTreeSet::new());
/// let ordem = signal(Some(("nome".to_owned(), SortDirection::Ascending)));
/// let tabela = DataGrid::bind(linhas, selecionadas)
///     .id("tabela")
///     .grow(1)
///     .columns([
///         DataGridColumn::new("nome", "Nome"),
///         DataGridColumn::new("valor", "Valor").align_end(),
///     ])
///     .sort(ordem)
///     .on_invoke(move |chave| println!("abrir {chave}"));
/// # });
/// # scope.clear();
/// ```
///
/// As linhas, a seleção, a ordem, o filtro e o nome em edição ficam em signals do app. Cada
/// linha traz o texto de cada célula e o valor que ordena (ordem natural: "arq2" antes de
/// "arq10"), e o `DataGrid` ordena e filtra sozinho. Só as linhas perto da área visível existem
/// como widgets, então dezenas de milhares de linhas continuam leves.
///
/// O clique seleciona, com Ctrl soma ou tira e com Shift marca a faixa; o clique duplo e o
/// Enter chamam `on_invoke`. As setas, Home, End, PageUp, PageDown e as letras navegam (com
/// Shift marcam a faixa), Ctrl+A seleciona tudo, F2 renomeia e Delete chama `on_delete`.
/// Arrastar uma linha leva a seleção; `on_drop` recebe o que caiu neste grid. As células são
/// de texto, ícone (na primeira coluna), barra de andamento ([`CellValue::Progress`]) e
/// minissérie ([`CellValue::Sparkline`]), e cada uma pode ter um fundo de calor
/// ([`DataGridRow::heat`]); não há célula com widget.
pub struct DataGrid {
    common: Common,
    shared: Rc<Shared>,
    drag: Option<DragState>,
}

/// O arrasto sobre o `DataGrid`.
struct DragState {
    data: DragData,
    pointer: Point,
    rect: Rect,
}

impl DataGrid {
    /// `DataGrid` das linhas `rows`, com as chaves selecionadas em `selected`.
    pub fn bind(rows: Signal<Vec<DataGridRow>>, selected: Signal<BTreeSet<String>>) -> Self {
        let shared = Shared::new(rows, selected);
        let watch = Rc::clone(&shared);
        effect(move || watch.update_window());
        Self {
            common: Common::default(),
            shared,
            drag: None,
        }
    }

    /// As colunas, na ordem em que aparecem.
    pub fn columns(self, columns: impl IntoIterator<Item = DataGridColumn>) -> Self {
        self.shared
            .columns
            .set(std::sync::Arc::new(columns.into_iter().collect()));
        self
    }

    /// A coluna ordenada (a chave) e o sentido, no app. O clique no cabeçalho troca o valor.
    /// Sem isto, as linhas ficam na ordem em que o app as pôs.
    pub fn sort(self, sort: Signal<Option<(String, SortDirection)>>) -> Self {
        self.shared.sort.set(sort);
        self.shared.rebuild();
        self
    }

    /// O texto do filtro: só as linhas cuja primeira célula contém o texto aparecem. Sem
    /// acentos nem maiúsculas.
    pub fn filter(self, filter: Signal<String>) -> Self {
        self.shared.filter.set(filter);
        self.shared.rebuild();
        self
    }

    /// A chave da linha em renomeação, no app: pôr uma chave aqui troca o nome dela por um
    /// campo de texto, e `None` volta ao normal. O F2 também usa. Exige `.on_rename`.
    pub fn renaming(self, renaming: Signal<Option<String>>) -> Self {
        self.shared.renaming.set(renaming);
        self
    }

    /// Enter ou clique duplo numa linha, com a chave dela.
    pub fn on_invoke(self, callback: impl Fn(String) + 'static) -> Self {
        *self.shared.on_invoke.borrow_mut() = Some(Rc::new(callback));
        self
    }

    /// O usuário confirmou um nome novo, com a chave e o texto. O app troca o nome; sem este
    /// callback não se renomeia.
    pub fn on_rename(self, callback: impl Fn(String, String) + 'static) -> Self {
        *self.shared.on_rename.borrow_mut() = Some(Rc::new(callback));
        self
    }

    /// Delete numa linha com o foco: as chaves da seleção, se a linha está nela, ou só a da
    /// linha. O app decide se apaga.
    pub fn on_delete(self, callback: impl Fn(Vec<String>) + 'static) -> Self {
        *self.shared.on_delete.borrow_mut() = Some(Rc::new(callback));
        self
    }

    /// Uma tecla que o grid não usa, com uma linha em foco (Ctrl+C, Ctrl+V, F5, Backspace...).
    /// Devolve `true` se o app usou.
    pub fn on_key(self, callback: impl Fn(Key, Modifiers) -> bool + 'static) -> Self {
        *self.shared.on_key.borrow_mut() = Some(Rc::new(callback));
        self
    }

    /// O menu de contexto de cada linha, pela chave. O clique direito seleciona a linha antes
    /// de abrir, a menos que ela já esteja na seleção. O menu vale para a linha como foi
    /// criada: monte-o lendo a seleção só quando um item for acionado.
    pub fn item_context_flyout(self, menu: impl Fn(&str) -> MenuFlyout + 'static) -> Self {
        *self.shared.item_menu.borrow_mut() = Some(Rc::new(menu));
        self
    }

    /// O grupo de arrasto: linhas arrastadas de um grid só caem em grids do mesmo grupo. Sem
    /// grupo, só o próprio grid aceita as dele.
    pub fn drag_group(self, group: impl Into<String>) -> Self {
        *self.shared.group.borrow_mut() = group.into();
        self
    }

    /// Linhas soltas neste grid (`DataGridDrop::target` é a pasta sob o ponteiro ou `None`
    /// para o fundo). Sem este callback o grid não recebe linhas.
    pub fn on_drop(self, callback: impl Fn(DataGridDrop) + 'static) -> Self {
        *self.shared.on_drop.borrow_mut() = Some(Rc::new(callback));
        self
    }

    /// Recusa destinos de arrasto.
    pub fn can_drop(self, can_drop: impl Fn(&DataGridDrop) -> bool + 'static) -> Self {
        *self.shared.can_drop.borrow_mut() = Some(Rc::new(can_drop));
        self
    }

    /// Arquivos do sistema soltos sobre o grid, com a chave da pasta sob o ponteiro (ou
    /// `None`). Vem do sistema de janelas, que não avisa no Wayland.
    pub fn on_files_dropped(
        self,
        callback: impl Fn(Vec<PathBuf>, Option<String>) + 'static,
    ) -> Self {
        *self.shared.on_files.borrow_mut() = Some(Rc::new(callback));
        self
    }

    /// O texto de quando não há linhas (a lista vazia ou o filtro sem resultado).
    pub fn empty_text(self, text: impl Into<String>) -> Self {
        *self.shared.empty_text.borrow_mut() = text.into();
        self
    }

    /// As chaves selecionadas que aparecem, na ordem das linhas.
    pub fn selected_keys(&self) -> Vec<String> {
        self.shared.selected_keys()
    }

    /// Troca a seleção pelas linhas `keys` (as que existem), como o clique com Ctrl: o
    /// `TestApp` usa.
    pub fn select_keys(&self, keys: &[String]) {
        let flat = self.shared.flat_now();
        let set: BTreeSet<String> = keys
            .iter()
            .filter(|key| flat.position(key).is_some())
            .cloned()
            .collect();
        *self.shared.anchor.borrow_mut() = keys.last().cloned();
        if self.shared.selected.with_untracked(|s| *s != set) {
            self.shared.selected.set(set);
        }
    }

    fn clear_drag(&mut self) {
        self.drag = None;
        self.shared.set_drop(None, false);
        if self.shared.drag_key.get_untracked().is_some() {
            self.shared.drag_key.set(None);
        }
    }

    /// Atualiza a marca do alvo para o ponteiro em `pointer`. Devolve se o grid aceita.
    fn show_target(&mut self, data: DragData, pointer: Point, rect: Rect) -> bool {
        let shared = Rc::clone(&self.shared);
        let (_, pressed, _) = Shared::decode(&data);
        if shared.drag_key.get_untracked().as_deref() != Some(pressed.as_str()) {
            shared.drag_key.set(Some(pressed));
        }
        let target = shared.drop_target(&data, pointer.y, rect.y0);
        let accepted = target.is_some() && shared.on_drop.borrow().is_some();
        match (&target, accepted) {
            (Some((_, row)), true) => shared.set_drop(row.clone(), row.is_none()),
            _ => shared.set_drop(None, false),
        }
        self.drag = Some(DragState {
            data,
            pointer,
            rect,
        });
        accepted
    }
}

common_builders!(DataGrid);
density_builder!(DataGrid);

impl Widget for DataGrid {
    fn kind(&self) -> &'static str {
        "data_grid"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn names_children(&self) -> bool {
        true
    }

    fn look(&self) -> Look {
        let shared = &self.shared;
        let flat = shared.flat_tracked();
        let selected = shared.selected.with(|selected| {
            flat.rows
                .iter()
                .filter(|row| selected.contains(&row.key))
                .map(|row| row.key.clone())
                .collect()
        });
        Look::DataGrid(DataGridVisual {
            rows: flat,
            columns: shared.layout.get(),
            sort: shared.sort.get().get(),
            selected,
            filter: shared.filter.get().get(),
            empty_text: shared.empty_text.borrow().clone(),
            drop_background: shared.drop_background.get(),
            density: shared.density.get(),
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
            .child(Column::new().child(GridRows::new(shared)));
        if let Some(id) = shared.sub("rolagem") {
            scroll = scroll.id(id);
        }
        vec![Box::new(GridHeader::new(shared)), Box::new(scroll)]
    }

    fn handle(&mut self, event: &Event, cx: &mut EventCx) -> bool {
        let shared = Rc::clone(&self.shared);
        match event {
            Event::DragOver(data) if data.kind == shared.drag_kind() => {
                self.show_target(data.clone(), cx.pointer(), cx.rect())
            }
            Event::DragLeave => {
                self.clear_drag();
                true
            }
            Event::Drop(data) if data.kind == shared.drag_kind() => {
                let target = shared.drop_target(data, cx.pointer().y, cx.rect().y0);
                self.clear_drag();
                let on_drop = shared.on_drop.borrow().clone();
                if let (Some((drop, _)), Some(on_drop)) = (target, on_drop) {
                    on_drop(drop);
                }
                true
            }
            Event::FilesDropped { paths } => {
                let callback = shared.on_files.borrow().clone();
                let Some(callback) = callback else {
                    return false;
                };
                let target = shared
                    .row_at(cx.pointer().y, cx.rect().y0)
                    .filter(|row| row.drop_target)
                    .map(|row| row.key.clone());
                callback(paths.clone(), target);
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
        let Some(state) = &self.drag else {
            return;
        };
        let (data, pointer, rect) = (state.data.clone(), state.pointer, state.rect);

        // Rolagem automática: a velocidade cresce da borda de uma linha para fora.
        let edge = shared.row_height();
        let top = rect.y0 + header_height(shared.density.get());
        let speed = if pointer.y < top + edge {
            -(1.0 - ((pointer.y - top) / edge).clamp(0.0, 1.0))
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
        self.show_target(data, pointer, rect);
    }

    fn text(&self) -> Option<String> {
        Some(self.shared.selected_keys().join(", "))
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

impl fmt::Debug for DataGrid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DataGrid")
            .field("common", &self.common)
            .finish_non_exhaustive()
    }
}
