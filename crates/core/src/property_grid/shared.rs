//! O estado que a `PropertyGrid`, as linhas e os divisores dividem, e o que o teclado e o mouse
//! fazem nele.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::rc::Rc;

use tomni_ui_reactive::prelude::*;

use super::model::{
    Category, Kind, Property, RowKey, Source, category_hidden, property_hidden, visible,
};
use super::visual::{CategoryInfo, PROPERTY_GRID_MIN_COLUMN, PropertyGridVisual, PropertyInfo};
use crate::cx::EventCx;
use crate::density::Density;
use crate::event::{Key, Modifiers};
use crate::tree_view::item_height;

/// Fração da largura que a coluna do nome ocupa no começo.
pub(crate) const NAME_RATIO: f64 = 0.5;
/// Menor altura do painel de descrição.
pub(crate) const MIN_DESCRIPTION: f64 = 32.0;
/// O padding da barra (4 px em cima e embaixo): conta no máximo do painel de descrição.
const BAR_PADDING: f64 = 8.0;
/// Altura do divisor da descrição.
const SIZER_HEIGHT: f64 = 8.0;
/// Altura inicial do painel de descrição.
pub(crate) const DESCRIPTION_HEIGHT: f64 = 72.0;

pub(crate) struct Shared {
    pub id: RefCell<Option<String>>,
    pub properties: RefCell<Vec<Property>>,
    pub categories: RefCell<Vec<Category>>,
    /// A linha escolhida.
    pub selected: Signal<Option<RowKey>>,
    /// Os nomes das categorias fechadas.
    pub collapsed: Signal<BTreeSet<String>>,
    pub alphabetical: Signal<bool>,
    /// Fração da largura da grade que a coluna do nome ocupa.
    pub name_ratio: Signal<f64>,
    /// Largura da grade, do `resized`.
    pub width: Signal<f64>,
    /// Altura da grade, do `resized`.
    pub height: Signal<f64>,
    /// Altura do painel de descrição.
    pub description_height: Signal<f64>,
    pub density: Cell<Density>,
    /// Estado da lista da `.choice` da linha escolhida: o F4 e o Alt+↓ na linha a abrem.
    pub choice_open: Signal<bool>,
    /// Muda quando o Esc desfaz o rascunho: o editor da linha escolhida é montado de novo.
    pub epoch: Signal<u64>,
    /// O texto digitado na linha, que o editor mostra ao receber o foco.
    pub pending_text: RefCell<Option<String>>,
}

impl Shared {
    pub fn new() -> Self {
        Self {
            id: RefCell::new(None),
            properties: RefCell::new(Vec::new()),
            categories: RefCell::new(Vec::new()),
            selected: signal(None),
            collapsed: signal(BTreeSet::new()),
            alphabetical: signal(false),
            name_ratio: signal(NAME_RATIO),
            width: signal(0.0),
            height: signal(0.0),
            description_height: signal(DESCRIPTION_HEIGHT),
            density: Cell::new(Density::Compact),
            choice_open: signal(false),
            epoch: signal(0),
            pending_text: RefCell::new(None),
        }
    }

    /// Id de uma parte: `{id}-{suffix}`.
    pub fn sub(&self, suffix: &str) -> Option<String> {
        self.id.borrow().as_ref().map(|id| format!("{id}-{suffix}"))
    }

    /// O id de uma linha.
    pub fn row_id(&self, key: RowKey) -> Option<String> {
        match key {
            RowKey::Property(i) => {
                let name = self.properties.borrow().get(i)?.name.clone();
                self.sub(&name)
            }
            RowKey::Category(c) => {
                let name = self.categories.borrow().get(c)?.name.clone();
                self.sub(&format!("categoria-{name}"))
            }
        }
    }

    /// As linhas visíveis, em ordem; lê `collapsed` e `alphabetical` de forma rastreada.
    pub fn rows(&self) -> Vec<RowKey> {
        let alphabetical = self.alphabetical.get();
        self.collapsed.with(|collapsed| {
            visible(
                &self.properties.borrow(),
                &self.categories.borrow(),
                collapsed,
                alphabetical,
            )
        })
    }

    /// As linhas visíveis, sem registrar dependência.
    pub fn rows_now(&self) -> Vec<RowKey> {
        untrack(|| self.rows())
    }

    /// Se a linha não aparece por causa do `.hidden` (a dela, a da categoria, ou a categoria
    /// sem linhas visíveis). Lê as condições de forma rastreada.
    pub fn is_hidden(&self, key: RowKey) -> bool {
        let properties = self.properties.borrow();
        let categories = self.categories.borrow();
        match key {
            RowKey::Property(i) => {
                i < properties.len() && property_hidden(&properties, &categories, i)
            }
            RowKey::Category(c) => {
                c < categories.len() && category_hidden(&properties, &categories, c)
            }
        }
    }

    /// Se a linha existe e está habilitada (o cabeçalho sempre está).
    pub fn enabled(&self, key: RowKey) -> bool {
        match key {
            RowKey::Category(_) => true,
            RowKey::Property(i) => self
                .properties
                .borrow()
                .get(i)
                .is_some_and(Property::enabled_untracked),
        }
    }

    /// A linha onde o Tab para: a escolhida, se aparece e está habilitada; senão, a primeira
    /// habilitada.
    pub fn stop(&self) -> Option<RowKey> {
        let rows = self.rows_now();
        let selected = untrack(|| self.selected.get());
        selected
            .filter(|key| rows.contains(key) && self.enabled(*key))
            .or_else(|| rows.into_iter().find(|key| self.enabled(*key)))
    }

    /// A linha escolhida que aparece.
    pub fn visible_selected(&self) -> Option<RowKey> {
        let selected = untrack(|| self.selected.get())?;
        self.rows_now().contains(&selected).then_some(selected)
    }

    pub fn select(&self, key: RowKey) {
        if untrack(|| self.selected.get()) != Some(key) {
            self.selected.set(Some(key));
        }
    }

    /// Escolhe a linha e leva o foco a ela.
    pub fn go(&self, key: RowKey, cx: &mut EventCx) {
        self.select(key);
        if let Some(id) = self.row_id(key) {
            cx.focus_later(id);
        }
    }

    /// Largura da coluna do nome: a posição da linha que divide as colunas. Lê os signals de
    /// forma rastreada.
    pub fn name_width(&self) -> f64 {
        let (ratio, width) = (self.name_ratio.get(), self.width.get());
        clamp_name(ratio * width, width)
    }

    /// Muda a altura do painel de descrição: no mínimo `MIN_DESCRIPTION`, e no máximo o que
    /// deixa a barra e duas linhas à vista.
    pub fn set_description_height(&self, wanted: f64) {
        let height = untrack(|| self.height.get());
        let row = item_height(self.density.get());
        let max = (height - (row + BAR_PADDING) - SIZER_HEIGHT - 2.0 * row).max(MIN_DESCRIPTION);
        let clamped = wanted.clamp(MIN_DESCRIPTION, max);
        if untrack(|| self.description_height.get()) != clamped {
            self.description_height.set(clamped);
        }
    }

    /// Arrasta a linha das colunas `dx` px a partir da fração `ratio` do começo do arrasto.
    pub fn drag_name(&self, ratio: f64, dx: f64) {
        let width = untrack(|| self.width.get());
        if width <= 0.0 {
            return;
        }
        let name = clamp_name(ratio * width + dx, width);
        self.name_ratio.set(name / width);
    }

    pub fn name_width_untracked(&self) -> f64 {
        untrack(|| self.name_width())
    }

    /// Se a categoria `c` está aberta (rastreado).
    pub fn is_open(&self, c: usize) -> bool {
        let categories = self.categories.borrow();
        categories
            .get(c)
            .is_some_and(|category| !self.collapsed.with(|set| set.contains(&category.name)))
    }

    /// Abre ou fecha a categoria `c`. Fechar com uma propriedade dela escolhida passa a
    /// escolha para o cabeçalho.
    pub fn set_open(&self, c: usize, open: bool) {
        let Some(name) = self.categories.borrow().get(c).map(|c| c.name.clone()) else {
            return;
        };
        if untrack(|| self.collapsed.with(|set| set.contains(&name))) == open {
            self.collapsed.update(|set| {
                if open {
                    set.remove(&name);
                } else {
                    set.insert(name);
                }
            });
        }
        if !open
            && let Some(RowKey::Property(i)) = untrack(|| self.selected.get())
            && self.properties.borrow().get(i).and_then(|p| p.category) == Some(c)
        {
            self.select(RowKey::Category(c));
        }
    }

    pub fn toggle(&self, c: usize) {
        let open = untrack(|| self.is_open(c));
        self.set_open(c, !open);
    }

    /// O id do editor (ou da caixa) do valor de uma propriedade.
    pub fn editor_id(&self, index: usize) -> Option<String> {
        let name = self.properties.borrow().get(index)?.name.clone();
        self.sub(&format!("{name}-valor"))
    }

    /// Desfaz o rascunho: o editor da linha escolhida é montado de novo, com o valor do app.
    pub fn revert(&self) {
        self.epoch.update(|n| *n += 1);
    }

    /// Inverte a `.check` da propriedade `index`.
    fn flip(&self, index: usize) {
        let properties = self.properties.borrow();
        if let Some(Source::Bool(prop)) = properties.get(index).map(|p| &p.source) {
            let on = untrack(|| prop.get());
            on.update(|value| *value = !*value);
        }
    }

    /// O foco vai para o editor da propriedade `index`, com `typed` (o texto digitado na
    /// linha) já nele.
    pub fn edit(&self, index: usize, typed: Option<String>, cx: &mut EventCx) {
        *self.pending_text.borrow_mut() = typed;
        if let Some(id) = self.editor_id(index) {
            cx.focus_later(id);
        }
    }

    /// Teclas que mexem no valor de uma propriedade com foco: F4 e Alt+↓ abrem a lista da
    /// `.choice`, Espaço inverte a `.check`, e Enter e F2 entram no editor (na `.check`,
    /// invertem; na `.choice`, abrem a lista).
    fn value_key(&self, index: usize, pressed: Key, mods: Modifiers, cx: &mut EventCx) -> bool {
        let (kind, read_only) = {
            let properties = self.properties.borrow();
            let Some(property) = properties.get(index) else {
                return false;
            };
            let kind = match &property.kind {
                Kind::Check => Value::Check,
                Kind::Choice(_) => Value::Choice,
                Kind::Button(click) => Value::Button(Rc::clone(click)),
                _ => Value::Other,
            };
            (kind, property.read_only)
        };
        let open =
            matches!(pressed, Key::F(4)) && !mods.ctrl || matches!(pressed, Key::Down) && mods.alt;
        let enter = matches!(pressed, Key::Enter | Key::F(2)) && !mods.command() && !mods.alt;
        match kind {
            Value::Check if matches!(pressed, Key::Space) && !mods.command() && !mods.alt => {
                if !read_only {
                    self.flip(index);
                }
                true
            }
            Value::Check if enter => {
                if !read_only {
                    self.flip(index);
                }
                true
            }
            Value::Choice if open || enter => {
                if !read_only {
                    self.choice_open.set(true);
                    self.edit(index, None, cx);
                }
                true
            }
            Value::Button(click)
                if enter || matches!(pressed, Key::Space) && !mods.command() && !mods.alt =>
            {
                if !read_only {
                    click();
                }
                true
            }
            Value::Other if enter => {
                if !read_only {
                    self.edit(index, None, cx);
                }
                true
            }
            _ => false,
        }
    }

    /// Teclas de uma linha com foco. Devolve se usou.
    pub fn key_down(&self, key: RowKey, pressed: Key, mods: Modifiers, cx: &mut EventCx) -> bool {
        if let RowKey::Property(i) = key
            && self.value_key(i, pressed, mods, cx)
        {
            return true;
        }
        if mods.command() || mods.alt {
            return false;
        }
        let rows: Vec<RowKey> = self
            .rows_now()
            .into_iter()
            .filter(|row| self.enabled(*row))
            .collect();
        let Some(pos) = rows.iter().position(|row| *row == key) else {
            return false;
        };
        match pressed {
            Key::Up if pos > 0 => self.go(rows[pos - 1], cx),
            Key::Down if pos + 1 < rows.len() => self.go(rows[pos + 1], cx),
            Key::Up | Key::Down => {}
            Key::Home => self.go(rows[0], cx),
            Key::End => self.go(rows[rows.len() - 1], cx),
            Key::Left => match key {
                RowKey::Category(c) => self.set_open(c, false),
                RowKey::Property(i) => {
                    let category = self.properties.borrow().get(i).and_then(|p| p.category);
                    if let Some(c) = category
                        && !untrack(|| self.alphabetical.get())
                    {
                        self.go(RowKey::Category(c), cx);
                    }
                }
            },
            Key::Right => {
                if let RowKey::Category(c) = key {
                    self.set_open(c, true);
                }
            }
            Key::Enter => {
                if let RowKey::Category(c) = key {
                    self.toggle(c);
                } else {
                    return false;
                }
            }
            _ => return false,
        }
        true
    }

    /// O nome e o texto do painel de descrição: os da linha escolhida (o cabeçalho só tem o
    /// nome). Lê os signals de forma rastreada.
    pub fn description(&self) -> (String, String) {
        let Some(selected) = self.selected.get() else {
            return (String::new(), String::new());
        };
        // A linha escolhida que ficou escondida (ou numa categoria escondida) não descreve nada.
        if self.is_hidden(selected) {
            return (String::new(), String::new());
        }
        match selected {
            RowKey::Property(i) => self
                .properties
                .borrow()
                .get(i)
                .map(|p| (p.name.clone(), p.description.clone()))
                .unwrap_or_default(),
            RowKey::Category(c) => (
                self.categories
                    .borrow()
                    .get(c)
                    .map(|c| c.name.clone())
                    .unwrap_or_default(),
                String::new(),
            ),
        }
    }

    /// O visual das ferramentas; lê os valores de forma rastreada.
    pub fn visual(&self) -> PropertyGridVisual {
        let selected = self.selected.get();
        let properties = self.properties.borrow();
        let categories = self.categories.borrow();
        let infos = properties
            .iter()
            .enumerate()
            .map(|(i, property)| {
                let value = property.value();
                PropertyInfo {
                    category: property
                        .category
                        .and_then(|c| categories.get(c))
                        .map(|c| c.name.clone()),
                    name: property.name.clone(),
                    value: property.described(&value),
                    changed: property.changed(&value),
                    selected: selected == Some(RowKey::Property(i)),
                    read_only: property.read_only,
                    enabled: !property.disabled.get(),
                    hidden: property_hidden(&properties, &categories, i),
                }
            })
            .collect();
        let collapsed = self.collapsed.get();
        PropertyGridVisual {
            properties: infos,
            categories: categories
                .iter()
                .enumerate()
                .map(|(i, c)| CategoryInfo {
                    name: c.name.clone(),
                    expanded: !collapsed.contains(&c.name),
                    hidden: category_hidden(&properties, &categories, i),
                })
                .collect(),
            alphabetical: self.alphabetical.get(),
        }
    }
}

/// O que o tipo da propriedade decide nas teclas do valor.
enum Value {
    Check,
    Choice,
    Button(Rc<dyn Fn()>),
    Other,
}

/// A coluna do nome com `wanted` px numa grade de `width`: pelo menos 40 px de cada lado.
pub(crate) fn clamp_name(wanted: f64, width: f64) -> f64 {
    let max = (width - PROPERTY_GRID_MIN_COLUMN).max(PROPERTY_GRID_MIN_COLUMN);
    wanted.clamp(PROPERTY_GRID_MIN_COLUMN, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coluna_do_nome_respeita_o_minimo_dos_dois_lados() {
        assert_eq!(clamp_name(10.0, 300.0), 40.0);
        assert_eq!(clamp_name(150.0, 300.0), 150.0);
        assert_eq!(clamp_name(290.0, 300.0), 260.0);
        // Grade estreita demais: fica no mínimo.
        assert_eq!(clamp_name(30.0, 50.0), 40.0);
    }
}
