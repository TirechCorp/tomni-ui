//! As partes montadas do `DataGrid`: a linha, o campo de renomear e o fragmento que mantém só
//! as linhas perto da área visível.

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::rc::Rc;
use std::sync::Arc;

use kurbo::Rect;
use tomni_ui_reactive::prelude::*;

use super::model::DataGridRow;
use super::shared::{Click, Shared};
use super::visual::{DataGridRowVisual, row_height};
use crate::containers::Column;
use crate::cx::EventCx;
use crate::cx::InputPurpose;
use crate::density::Density;
use crate::drag::DragData;
use crate::edit::Edit;
use crate::event::{Event, Key};
use crate::flyout::ContextFlyout;
use crate::press::Pressable;
use crate::widget::{Align, Common, Direction, Flex, Insets, Justify, Look, Slot, Widget};

/// Sobra à direita do campo de renomear.
const RENAME_RIGHT: f64 = 8.0;
/// O campo começa um pouco antes do rótulo: o texto dele tem padding próprio.
const RENAME_LEFT: f64 = 4.0;

/// O campo de texto de uma linha em renomeação: um `Edit` que confirma com Enter ou quando
/// perde o foco e cancela com Esc.
struct RenameField {
    edit: Edit,
    shared: Rc<Shared>,
    key: String,
    original: String,
    value: Signal<String>,
    done: bool,
}

impl RenameField {
    fn new(shared: &Rc<Shared>, key: &str, label: String) -> Self {
        let value = signal(label.clone());
        let edit = Edit::bind(value)
            .generated_id(shared.sub(&format!("{key}-nome")))
            .grow(1);
        Self {
            edit,
            shared: Rc::clone(shared),
            key: key.to_owned(),
            original: label,
            value,
            done: false,
        }
    }

    /// Fecha o campo; `commit` confirma o texto (se for diferente do original e não vazio).
    /// Devolve `false`, sem fechar, se pediu para confirmar um texto vazio e `strict`.
    fn finish(&mut self, commit: bool, strict: bool) -> bool {
        if self.done {
            return true;
        }
        let text = self.value.get_untracked().trim().to_owned();
        let mut rename = None;
        if commit {
            if text.is_empty() {
                if strict {
                    return false;
                }
            } else if text != self.original {
                rename = Some(text);
            }
        }
        self.done = true;
        self.shared.renaming.get().set(None);
        if let Some(text) = rename {
            let callback = self.shared.on_rename.borrow().clone();
            if let Some(callback) = callback {
                callback(self.key.clone(), text);
            }
        }
        true
    }
}

impl Widget for RenameField {
    fn kind(&self) -> &'static str {
        self.edit.kind()
    }

    fn common(&self) -> &Common {
        self.edit.common()
    }

    fn look(&self) -> Look {
        self.edit.look()
    }

    fn set_density(&mut self, density: Density) {
        self.edit.set_density(density);
    }

    fn flex(&self) -> Option<Flex> {
        self.edit.flex()
    }

    fn measured(&self) -> bool {
        self.edit.measured()
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        self.edit.take_children()
    }

    fn names_children(&self) -> bool {
        self.edit.names_children()
    }

    fn handle(&mut self, event: &Event, cx: &mut EventCx) -> bool {
        match event {
            Event::KeyDown(Key::Enter) => {
                if self.finish(true, true)
                    && let Some(id) = self.shared.sub(&self.key)
                {
                    cx.focus_later(id);
                }
                return true;
            }
            Event::KeyDown(Key::Escape) => {
                self.finish(false, false);
                if let Some(id) = self.shared.sub(&self.key) {
                    cx.focus_later(id);
                }
                return true;
            }
            _ => {}
        }
        let consumed = self.edit.handle(event, cx);
        match event {
            Event::FocusIn(_) => self.edit.select_all(),
            Event::FocusOut => {
                self.finish(true, false);
            }
            _ => {}
        }
        consumed
    }

    fn interactive(&self) -> bool {
        self.edit.interactive()
    }

    fn focusable(&self) -> bool {
        self.edit.focusable()
    }

    fn enabled(&self) -> bool {
        self.edit.enabled()
    }

    fn text(&self) -> Option<String> {
        self.edit.text()
    }

    fn text_input(&self) -> Option<InputPurpose> {
        self.edit.text_input()
    }

    fn focus_on_mount(&self) -> bool {
        true
    }
}

impl fmt::Debug for RenameField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RenameField")
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

/// Uma linha do `DataGrid`.
pub(crate) struct GridRow {
    common: Common,
    shared: Rc<Shared>,
    key: String,
    press: Pressable,
    density: Density,
    /// O campo de renomear, até a montagem entregá-lo à árvore.
    rename: Option<RenameField>,
    renaming: bool,
    /// O que o botão solto ainda precisa fazer na seleção.
    pending: Click,
}

impl GridRow {
    pub(crate) fn new(shared: &Rc<Shared>, key: String, renaming: bool) -> Self {
        let mut common = Common::generated(shared.sub(&key));
        let density = shared.density.get();
        common.height = Some(row_height(density));
        let menu = shared.item_menu.borrow().clone();
        if let Some(menu) = menu {
            common.context_flyout = Some(ContextFlyout::new(menu(&key)));
        }
        let rename = renaming.then(|| {
            let label = shared
                .flat_now()
                .get(&key)
                .map(|row| row.label().to_owned())
                .unwrap_or_default();
            RenameField::new(shared, &key, label)
        });
        Self {
            common,
            shared: Rc::clone(shared),
            key,
            press: Pressable::default(),
            density,
            rename,
            renaming,
            pending: Click::None,
        }
    }

    fn visual(&self, row: Arc<DataGridRow>, selected: bool, drop: bool) -> DataGridRowVisual {
        DataGridRowVisual {
            row,
            columns: self.shared.layout.get(),
            selected,
            state: self.press.state(),
            focus_ring: self.press.focus_ring(),
            renaming: self.renaming,
            drop,
            density: self.density,
        }
    }
}

impl Widget for GridRow {
    fn kind(&self) -> &'static str {
        "data_grid_row"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        let shared = &self.shared;
        let flat = shared.flat_tracked();
        let selected = shared.selected.with(|s| s.contains(&self.key));
        let drop = shared.drop.get().as_deref() == Some(self.key.as_str());
        let row = flat
            .get(&self.key)
            .cloned()
            .unwrap_or_else(|| Arc::new(DataGridRow::new(self.key.clone())));
        Look::DataGridRow(self.visual(row, selected, drop))
    }

    fn flex(&self) -> Option<Flex> {
        if !self.renaming {
            return None;
        }
        let shared = &self.shared;
        let width = untrack(|| shared.link.viewport_width()).max(200.0);
        let rect = Rect::new(0.0, 0.0, width, row_height(self.density));
        let row = untrack(|| shared.flat_now().get(&self.key).cloned())
            .unwrap_or_else(|| Arc::new(DataGridRow::new(self.key.clone())));
        let visual = untrack(|| self.visual(row, false, false));
        let first = visual.cells(rect).first().copied().unwrap_or(rect);
        let left = visual.label_x(rect) - RENAME_LEFT;
        Some(Flex {
            direction: Direction::Row,
            padding: Insets::new(0.0, (width - first.x1).max(0.0) + RENAME_RIGHT, 0.0, left),
            align: Align::Center,
            justify: Justify::Start,
            ..Flex::default()
        })
    }

    fn set_density(&mut self, density: Density) {
        self.density = density;
        self.common.height = Some(row_height(density));
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        self.rename
            .take()
            .map(|field| Box::new(field) as Box<dyn Widget>)
            .into_iter()
            .collect()
    }

    fn handle(&mut self, event: &Event, cx: &mut EventCx) -> bool {
        let shared = Rc::clone(&self.shared);
        match event {
            Event::ContextOpening => {
                if !shared.is_selected(&self.key) {
                    shared.select_only(&self.key);
                }
                shared.set_cursor(&self.key);
                return true;
            }
            // O alvo do arrasto é o `DataGrid`: a linha só deixa passar.
            Event::DragOver(_)
            | Event::DragLeave
            | Event::Drop(_)
            | Event::DropOutside { .. }
            | Event::FilesDropped { .. } => return false,
            Event::PointerDown { clicks, .. } => {
                let mods = cx.modifiers();
                self.pending = shared.pointer_down(&self.key, mods);
                if *clicks >= 2 && !mods.shift && !mods.command() {
                    self.pending = Click::None;
                    shared.invoke(&self.key);
                }
            }
            Event::PointerCancel => self.pending = Click::None,
            Event::FocusIn(_) => {
                shared.focus_key.set(Some(self.key.clone()));
                shared.set_cursor(&self.key);
            }
            Event::FocusOut => {
                if shared.focus_key.get_untracked().as_deref() == Some(self.key.as_str()) {
                    shared.focus_key.set(None);
                }
            }
            Event::KeyDown(key) if self.press.is_focused() => {
                if shared.key_down(&self.key, *key, cx.modifiers(), cx) {
                    return true;
                }
            }
            Event::Text(text)
                if self.press.is_focused()
                    && !cx.modifiers().command()
                    && !cx.modifiers().alt
                    && shared.type_ahead(&self.key, text, cx) =>
            {
                return true;
            }
            _ => {}
        }
        let (consumed, clicked) = self.press.handle(event, false);
        if clicked {
            let pending = std::mem::replace(&mut self.pending, Click::None);
            shared.clicked(&self.key, pending);
        }
        consumed
    }

    fn interactive(&self) -> bool {
        true
    }

    fn focusable(&self) -> bool {
        true
    }

    fn tab_stop(&self) -> bool {
        self.shared.tab_key().as_deref() == Some(self.key.as_str())
    }

    fn text(&self) -> Option<String> {
        untrack(|| {
            self.shared
                .flat_now()
                .get(&self.key)
                .map(|row| row.label().to_owned())
        })
    }

    fn drag_data(&self) -> Option<DragData> {
        if self.renaming {
            return None;
        }
        self.shared.drag_data(&self.key)
    }
}

impl fmt::Debug for GridRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GridRow")
            .field("common", &self.common)
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

/// O que ocupa um lugar no fragmento: uma linha (com o estado de renomear) ou o vão que
/// substitui as linhas fora da janela.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum SlotKey {
    Row(String, bool),
    Gap(usize, usize),
}

/// Fragmento com as linhas montadas: a janela em volta da área visível, as linhas fixadas
/// (com foco, do teclado, em renomeação, origem do arrasto) e os vãos entre elas.
pub(crate) struct GridRows {
    common: Common,
    shared: Rc<Shared>,
    slots: Option<Vec<SlotKey>>,
}

impl GridRows {
    pub(crate) fn new(shared: &Rc<Shared>) -> Self {
        Self {
            common: Common::default(),
            shared: Rc::clone(shared),
            slots: None,
        }
    }

    /// Os índices que ficam montados, em ordem.
    fn mounted(&self) -> BTreeSet<usize> {
        let shared = &self.shared;
        let flat = shared.flat_now();
        let n = flat.rows.len();
        let (start, end) = shared.window.get_untracked();
        let mut mounted: BTreeSet<usize> = (start.min(n)..end.min(n)).collect();
        let pins = [
            shared.tab_key(),
            shared.cursor.get_untracked(),
            shared.focus_key.get_untracked(),
            shared.renaming.get().get_untracked(),
            shared.drag_key.get_untracked(),
        ];
        for key in pins.into_iter().flatten() {
            mounted.extend(flat.position(&key));
        }
        mounted
    }
}

impl Widget for GridRows {
    fn kind(&self) -> &'static str {
        "data_grid_rows"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::Fragment
    }

    fn fragment(&self) -> bool {
        true
    }

    fn track_children(&self) {
        let shared = &self.shared;
        let _ = shared.flat_tracked();
        let _ = shared.window.get();
        let _ = shared.cursor.get();
        let _ = shared.renaming.get().get();
        let _ = shared.focus_key.get();
        let _ = shared.drag_key.get();
    }

    fn reconcile(&mut self) -> Option<Vec<Slot>> {
        let shared = Rc::clone(&self.shared);
        let flat = shared.flat_now();
        let n = flat.rows.len();
        let renaming = shared.renaming.get().get_untracked();
        let height = shared.row_height();

        let mut keys = Vec::new();
        let mut next = 0;
        for index in self.mounted() {
            if index > next {
                keys.push(SlotKey::Gap(next, index));
            }
            let row = &flat.rows[index];
            keys.push(SlotKey::Row(
                row.key.clone(),
                renaming.as_deref() == Some(row.key.as_str()),
            ));
            next = index + 1;
        }
        if next < n {
            keys.push(SlotKey::Gap(next, n));
        }
        if self.slots.as_ref() == Some(&keys) {
            return None;
        }

        let mut old: HashMap<&SlotKey, usize> = self
            .slots
            .iter()
            .flatten()
            .enumerate()
            .map(|(i, key)| (key, i))
            .collect();
        let slots = keys
            .iter()
            .map(|key| match old.remove(key) {
                Some(i) => Slot::Keep(i),
                None => {
                    let shared = Rc::clone(&shared);
                    let key = key.clone();
                    Slot::New(Box::new(move || match key {
                        SlotKey::Row(key, renaming) => {
                            Box::new(GridRow::new(&shared, key, renaming)) as Box<dyn Widget>
                        }
                        SlotKey::Gap(a, b) => {
                            Box::new(Column::new().height((b - a) as f64 * height))
                        }
                    }))
                }
            })
            .collect();
        self.slots = Some(keys);
        Some(slots)
    }
}

impl fmt::Debug for GridRows {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GridRows")
            .field("slots", &self.slots.as_ref().map(Vec::len))
            .finish_non_exhaustive()
    }
}
