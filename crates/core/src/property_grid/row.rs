//! As partes montadas da `PropertyGrid`: a linha de propriedade, o cabeçalho de categoria, o
//! espaçador da coluna do nome e o fragmento que mantém as linhas na ordem certa.

use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use tomni_ui_reactive::prelude::*;

use super::editor::EditorHost;
use super::model::{Kind, RowKey};
use super::shared::Shared;
use super::visual::{PROPERTY_GRID_GRIP, PropertyGridCategoryVisual, PropertyGridRowVisual};
use crate::button::ButtonState;
use crate::cx::EventCx;
use crate::density::Density;
use crate::event::{Cursor, Event, FocusSource, Key};
use crate::press::Pressable;
use crate::tree_view::item_height;
use crate::widget::{Align, Common, Direction, Flex, Look, Slot, Widget};

/// O espaçador da coluna do nome: tem a largura dela (mais a linha de divisão), e o editor da
/// linha vem depois.
pub(crate) struct NameColumn {
    common: Common,
    shared: Rc<Shared>,
}

impl NameColumn {
    fn new(shared: &Rc<Shared>) -> Self {
        Self {
            common: Common::default(),
            shared: Rc::clone(shared),
        }
    }
}

impl Widget for NameColumn {
    fn kind(&self) -> &'static str {
        "property_grid_name"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::Row
    }

    fn dynamic_width(&self) -> Option<f64> {
        Some(self.shared.name_width() + 1.0)
    }
}

impl fmt::Debug for NameColumn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NameColumn").finish_non_exhaustive()
    }
}

/// Uma linha de propriedade.
pub(crate) struct PropertyRow {
    common: Common,
    shared: Rc<Shared>,
    index: usize,
    press: Pressable,
    density: Density,
    /// Se o ponteiro está perto da linha das colunas (o cursor vira `EwResize`).
    near_divider: bool,
    /// O arrasto da linha das colunas: o x do ponteiro e a fração do nome no começo.
    drag: Option<(f64, f64)>,
}

impl PropertyRow {
    pub(crate) fn new(shared: &Rc<Shared>, index: usize) -> Self {
        let id = shared.row_id(RowKey::Property(index));
        Self {
            common: Common::generated(id),
            shared: Rc::clone(shared),
            index,
            press: Pressable::default(),
            density: Density::Compact,
            near_divider: false,
            drag: None,
        }
    }

    fn key(&self) -> RowKey {
        RowKey::Property(self.index)
    }

    /// Se o ponteiro está a [`PROPERTY_GRID_GRIP`] px da linha que divide as colunas.
    fn pointer_near_divider(&self, cx: &EventCx) -> bool {
        let divider = cx.rect().x0 + self.shared.name_width_untracked();
        (cx.pointer().x - divider).abs() <= PROPERTY_GRID_GRIP
    }

    /// Se digitar na linha abre o editor com o texto: vale para o texto, a senha e o arquivo
    /// (o número só entra no editor com Enter, F2 ou o clique).
    fn typing_starts_edit(&self) -> bool {
        let properties = self.shared.properties.borrow();
        properties.get(self.index).is_some_and(|property| {
            property.editable()
                && matches!(
                    property.kind,
                    Kind::Text | Kind::Password | Kind::File(_) | Kind::Folder
                )
        })
    }
}

impl Widget for PropertyRow {
    fn kind(&self) -> &'static str {
        "property_grid_row"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        let shared = &self.shared;
        let properties = shared.properties.borrow();
        let property = &properties[self.index];
        let value = property.value();
        let disabled = property.disabled.get();
        let selected = shared.selected.get() == Some(self.key());
        Look::PropertyGridRow {
            name: property.name.clone(),
            value: property.shown(&value),
            visual: PropertyGridRowVisual {
                changed: property.changed(&value),
                read_only: property.read_only,
                disabled,
                selected,
                editing: selected && !disabled && property.editable(),
                indent: property.category.is_some() && !shared.alphabetical.get(),
                name_width: shared.name_width(),
                state: if disabled {
                    ButtonState::Disabled
                } else {
                    self.press.state()
                },
                focus_ring: self.press.focus_ring(),
                density: self.density,
            },
        }
    }

    fn flex(&self) -> Option<Flex> {
        Some(Flex {
            direction: Direction::Row,
            align: Align::Center,
            ..Flex::default()
        })
    }

    fn set_density(&mut self, density: Density) {
        self.density = density;
        self.common.height = Some(item_height(density));
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        vec![
            Box::new(NameColumn::new(&self.shared)),
            Box::new(EditorHost::new(&self.shared, self.index)),
        ]
    }

    fn handle(&mut self, event: &Event, cx: &mut EventCx) -> bool {
        let shared = Rc::clone(&self.shared);
        let key = self.key();
        match event {
            Event::FocusIn(source) => {
                if *source == FocusSource::Keyboard {
                    shared.select(key);
                }
            }
            Event::PointerMove { kind, inside } => {
                self.near_divider = (*inside && kind.hovers() || self.drag.is_some())
                    && self.pointer_near_divider(cx);
                if let Some((from, ratio)) = self.drag {
                    shared.drag_name(ratio, cx.pointer().x - from);
                    return true;
                }
            }
            Event::PointerDown { .. } if self.pointer_near_divider(cx) => {
                self.drag = Some((cx.pointer().x, untrack(|| shared.name_ratio.get())));
                return true;
            }
            Event::PointerUp { .. } | Event::PointerCancel if self.drag.is_some() => {
                self.drag = None;
                return true;
            }
            Event::PointerDown { .. } if self.enabled() => {
                // O clique no valor de uma linha que não era a escolhida escolhe e leva o
                // foco ao editor (que só aparece depois de escolhida).
                let value_x = cx.rect().x0 + shared.name_width_untracked() + 1.0;
                if cx.pointer().x > value_x && untrack(|| shared.selected.get()) != Some(key) {
                    shared.select(key);
                    shared.edit(self.index, None, cx);
                }
            }
            Event::KeyDown(pressed)
                if self.press.is_focused()
                    && shared.key_down(key, *pressed, cx.modifiers(), cx) =>
            {
                return true;
            }
            // O Esc que sobe de um editor sem invólucro (a lista, a caixa de marcação) devolve
            // o foco à linha.
            Event::KeyDown(Key::Escape) if !self.press.is_focused() => {
                if let Some(id) = shared.row_id(key) {
                    cx.focus_later(id);
                }
                return true;
            }
            Event::Text(typed)
                if self.press.is_focused()
                    && !cx.modifiers().command()
                    && !cx.modifiers().alt
                    && self.typing_starts_edit() =>
            {
                shared.edit(self.index, Some(typed.clone()), cx);
                return true;
            }
            _ => {}
        }
        let (consumed, clicked) = self.press.handle(event, false);
        if clicked {
            shared.select(key);
        }
        consumed
    }

    fn interactive(&self) -> bool {
        true
    }

    fn cursor(&self) -> Option<Cursor> {
        (self.near_divider || self.drag.is_some()).then_some(Cursor::EwResize)
    }

    fn focusable(&self) -> bool {
        self.enabled()
    }

    fn tab_stop(&self) -> bool {
        self.shared.stop() == Some(self.key())
    }

    fn enabled(&self) -> bool {
        self.shared.enabled(self.key())
    }

    fn text(&self) -> Option<String> {
        untrack(|| {
            let properties = self.shared.properties.borrow();
            let property = &properties[self.index];
            Some(property.shown(&property.value()))
        })
    }
}

impl fmt::Debug for PropertyRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PropertyRow")
            .field("common", &self.common)
            .field("index", &self.index)
            .finish_non_exhaustive()
    }
}

/// O cabeçalho de uma categoria.
pub(crate) struct CategoryRow {
    common: Common,
    shared: Rc<Shared>,
    index: usize,
    press: Pressable,
    density: Density,
    /// Se o ponteiro está no quadrado do chevron.
    chevron_hover: bool,
    /// Se o botão foi pressionado no quadrado do chevron.
    down_chevron: bool,
}

impl CategoryRow {
    pub(crate) fn new(shared: &Rc<Shared>, index: usize) -> Self {
        let id = shared.row_id(RowKey::Category(index));
        Self {
            common: Common::generated(id),
            shared: Rc::clone(shared),
            index,
            press: Pressable::default(),
            density: Density::Compact,
            chevron_hover: false,
            down_chevron: false,
        }
    }

    fn key(&self) -> RowKey {
        RowKey::Category(self.index)
    }

    fn visual_base(&self) -> PropertyGridCategoryVisual {
        PropertyGridCategoryVisual {
            density: self.density,
            ..PropertyGridCategoryVisual::default()
        }
    }

    fn in_chevron(&self, cx: &EventCx) -> bool {
        self.visual_base()
            .chevron_square(cx.rect())
            .contains(cx.pointer())
    }
}

impl Widget for CategoryRow {
    fn kind(&self) -> &'static str {
        "property_grid_category"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        let shared = &self.shared;
        let label = shared
            .categories
            .borrow()
            .get(self.index)
            .map(|c| c.name.clone())
            .unwrap_or_default();
        Look::PropertyGridCategory {
            label,
            visual: PropertyGridCategoryVisual {
                expanded: shared.is_open(self.index),
                selected: shared.selected.get() == Some(self.key()),
                state: self.press.state(),
                chevron_state: if self.chevron_hover {
                    ButtonState::Hover
                } else {
                    ButtonState::Rest
                },
                focus_ring: self.press.focus_ring(),
                ..self.visual_base()
            },
        }
    }

    fn set_density(&mut self, density: Density) {
        self.density = density;
        self.common.height = Some(item_height(density));
    }

    fn handle(&mut self, event: &Event, cx: &mut EventCx) -> bool {
        let shared = Rc::clone(&self.shared);
        let key = self.key();
        match event {
            Event::PointerMove { kind, inside } => {
                self.chevron_hover = *inside && kind.hovers() && self.in_chevron(cx);
            }
            Event::PointerDown { clicks, .. } => {
                self.down_chevron = self.in_chevron(cx);
                if *clicks >= 2 && !self.down_chevron {
                    shared.toggle(self.index);
                }
            }
            Event::FocusIn(source) => {
                if *source == FocusSource::Keyboard {
                    shared.select(key);
                }
            }
            Event::KeyDown(pressed)
                if self.press.is_focused()
                    && shared.key_down(key, *pressed, cx.modifiers(), cx) =>
            {
                return true;
            }
            _ => {}
        }
        let (consumed, clicked) = self.press.handle(event, false);
        if clicked {
            if self.down_chevron {
                shared.toggle(self.index);
            } else {
                shared.select(key);
            }
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
        self.shared.stop() == Some(self.key())
    }

    fn text(&self) -> Option<String> {
        self.shared
            .categories
            .borrow()
            .get(self.index)
            .map(|c| c.name.clone())
    }
}

impl fmt::Debug for CategoryRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CategoryRow")
            .field("common", &self.common)
            .field("index", &self.index)
            .finish_non_exhaustive()
    }
}

/// O fragmento com as linhas visíveis, na ordem: trocar a ordem ou fechar uma categoria
/// mantém as linhas que continuam (e o foco delas).
pub(crate) struct GridRows {
    common: Common,
    shared: Rc<Shared>,
    keys: Option<Vec<RowKey>>,
}

impl GridRows {
    pub(crate) fn new(shared: &Rc<Shared>) -> Self {
        Self {
            common: Common::default(),
            shared: Rc::clone(shared),
            keys: None,
        }
    }
}

impl Widget for GridRows {
    fn kind(&self) -> &'static str {
        "property_grid_rows"
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
        let _ = self.shared.rows();
    }

    fn reconcile(&mut self) -> Option<Vec<Slot>> {
        let shared = Rc::clone(&self.shared);
        let keys = shared.rows_now();
        if self.keys.as_ref() == Some(&keys) {
            return None;
        }
        let mut old: HashMap<RowKey, usize> = self
            .keys
            .iter()
            .flatten()
            .enumerate()
            .map(|(i, key)| (*key, i))
            .collect();
        let slots = keys
            .iter()
            .map(|key| match old.remove(key) {
                Some(i) => Slot::Keep(i),
                None => {
                    let shared = Rc::clone(&shared);
                    let key = *key;
                    Slot::New(Box::new(move || match key {
                        RowKey::Property(i) => {
                            Box::new(PropertyRow::new(&shared, i)) as Box<dyn Widget>
                        }
                        RowKey::Category(c) => Box::new(CategoryRow::new(&shared, c)),
                    }))
                }
            })
            .collect();
        self.keys = Some(keys);
        Some(slots)
    }
}

impl fmt::Debug for GridRows {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GridRows")
            .field("rows", &self.keys.as_ref().map(Vec::len))
            .finish_non_exhaustive()
    }
}
