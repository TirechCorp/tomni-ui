//! `AutoSuggestBox`: campo de texto com uma lista de sugestões que acompanha o que se digita.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::rc::Rc;

use kurbo::Size;
use tomni_ui_reactive::{Get, GetUntracked, Set, Signal, With, WithUntracked, signal};

use crate::combo_box::{ComboBoxItemVisual, item_height, option_slug};
use crate::containers::Column;
use crate::cx::{EventCx, InputPurpose};
use crate::density::Density;
use crate::edit::{Edit, Part};
use crate::event::{Event, Key};
use crate::field_button::{FieldAction, FieldButton, FieldGlyph};
use crate::fragments::For;
use crate::press::Pressable;
use crate::prop::{IntoProp, Prop};
use crate::scroll_viewer::ScrollViewer;
use crate::shortcut::Shortcut;
use crate::symbol::Symbol;
use crate::widget::{Common, Flex, Insets, Layer, Look, PopupKind, Widget};

/// Margem da lista dentro do popup, como a do `ComboBox`.
const LIST_PADDING: Insets = Insets::new(5.0, 1.0, 5.0, 1.0);
/// Sugestões que a lista mostra antes de rolar.
const LIST_ROWS: f64 = 6.0;
/// Largura do botão de busca no fim do campo, a mesma do botão de limpar do `NumberBox`.
const QUERY_WIDTH: f64 = 40.0;

type Suggest = Rc<dyn Fn(&str) -> Vec<String>>;
type Submit = Rc<RefCell<Option<Box<dyn FnMut(String)>>>>;

/// O que o campo, a lista e os itens compartilham.
#[derive(Clone)]
struct Shared {
    value: Signal<String>,
    /// O que foi digitado: as sugestões vêm daqui, e Esc volta a ele.
    query: Signal<String>,
    open: Signal<bool>,
    /// Sugestão destacada pelas setas, já copiada para o campo.
    highlight: Signal<Option<String>>,
    suggest: Suggest,
    submit: Submit,
}

impl Shared {
    fn suggestions(&self) -> Vec<String> {
        (self.suggest)(&self.query.get())
    }

    fn suggestions_untracked(&self) -> Vec<String> {
        (self.suggest)(&self.query.get_untracked())
    }

    fn set_value(&self, text: &str) {
        if self.value.with_untracked(|v| v != text) {
            self.value.set(text.to_owned());
        }
    }

    fn close(&self) {
        if self.open.get_untracked() {
            self.open.set(false);
        }
        if self.highlight.with_untracked(Option::is_some) {
            self.highlight.set(None);
        }
    }

    /// Fecha a lista e entrega `text` ao `on_submit`.
    fn submit(&self, text: String) {
        self.close();
        if self.query.with_untracked(|q| *q != text) {
            self.query.set(text.clone());
        }
        if let Some(submit) = self.submit.borrow_mut().as_mut() {
            submit(text);
        }
    }
}

/// Campo de texto com sugestões, ligado a um `Signal<String>`.
///
/// ```
/// use tomni_ui_core::AutoSuggestBox;
/// use tomni_ui_reactive::prelude::*;
///
/// # let scope = Scope::root();
/// # scope.run(|| {
/// const CIDADES: [&str; 3] = ["Belo Horizonte", "Belém", "Recife"];
/// let cidade = signal(String::new());
/// let campo = AutoSuggestBox::bind(cidade, |texto: &str| {
///     let texto = texto.to_lowercase();
///     CIDADES
///         .iter()
///         .filter(|c| !texto.is_empty() && c.to_lowercase().starts_with(&texto))
///         .map(|c| c.to_string())
///         .collect()
/// })
/// .id("cidade")
/// .placeholder("Cidade")
/// .on_submit(|cidade| println!("buscar {cidade}"));
/// assert_eq!(campo.value(), "");
/// # });
/// # scope.clear();
/// ```
///
/// A cada mudança no texto, `suggest` recebe o que foi digitado e devolve as sugestões; a
/// lista abre embaixo do campo quando há alguma e fecha quando não há. ↓ e ↑ andam pela lista
/// e põem a sugestão no campo (↑ na primeira volta ao que foi digitado); Enter ou um clique
/// numa sugestão a escolhem, fecham a lista e chamam o `on_submit`; Esc fecha e volta ao que
/// foi digitado. O foco fica sempre no campo, e a lista fecha quando ele sai.
///
/// Com [`AutoSuggestBox::query_icon`], um botão no fim do campo (o `QueryIcon` do WinUI)
/// também chama o `on_submit` com o texto.
///
/// A lista tem o id `{id}-lista` e cada sugestão, `{id}-{sugestão}`, com a sugestão em
/// minúsculas e o que não é letra ou número trocado por `-` (`cidade-belo-horizonte`). O botão
/// de busca tem o id `{id}-buscar`.
pub struct AutoSuggestBox {
    edit: Edit,
    shared: Shared,
    query_icon: Option<Symbol>,
    /// Se um atalho de [`AutoSuggestBox::focus_shortcut`] pediu o foco.
    focus_wanted: Rc<Cell<bool>>,
}

impl AutoSuggestBox {
    /// Campo que mostra e altera `value`, com as sugestões de `suggest` para o texto digitado.
    pub fn bind(value: Signal<String>, suggest: impl Fn(&str) -> Vec<String> + 'static) -> Self {
        Self {
            edit: Edit::bind(value),
            shared: Shared {
                value,
                query: signal(value.get_untracked()),
                open: signal(false),
                highlight: signal(None),
                suggest: Rc::new(suggest),
                submit: Rc::new(RefCell::new(None)),
            },
            query_icon: None,
            focus_wanted: Rc::new(Cell::new(false)),
        }
    }

    /// Atalho que leva o foco para o campo (o Ctrl+E de uma busca global). Vale na janela
    /// toda, enquanto o campo aparece, como o `.accelerator()` dos outros widgets.
    ///
    /// ```
    /// use tomni_ui_core::{AutoSuggestBox, Shortcut};
    /// use tomni_ui_reactive::prelude::*;
    ///
    /// # let scope = Scope::root();
    /// # scope.run(|| {
    /// let texto = signal(String::new());
    /// let busca = AutoSuggestBox::bind(texto, |_| Vec::new())
    ///     .id("busca")
    ///     .focus_shortcut(Shortcut::ctrl('E'));
    /// # let _ = busca;
    /// # });
    /// # scope.clear();
    /// ```
    pub fn focus_shortcut<M>(mut self, shortcut: impl IntoProp<Shortcut, M>) -> Self {
        let wanted = Rc::clone(&self.focus_wanted);
        self.edit = self.edit.accelerator(shortcut, move || wanted.set(true));
        self
    }

    /// Botão com o ícone `symbol` (em geral [`Symbol::Search`]) no fim do campo: um clique
    /// fecha a lista e chama o `on_submit` com o texto do campo, como o Enter. O foco fica no
    /// campo. Na árvore, é um `field_button` com o rótulo `Buscar`.
    pub fn query_icon(mut self, symbol: Symbol) -> Self {
        self.query_icon = Some(symbol);
        self
    }

    /// Texto mostrado enquanto o campo está vazio (fixo, closure ou signal).
    pub fn placeholder<M>(mut self, placeholder: impl IntoProp<String, M>) -> Self {
        self.edit = self.edit.placeholder(placeholder);
        self
    }

    /// Desabilita o campo (valor fixo, closure ou signal).
    pub fn disabled<M>(mut self, disabled: impl IntoProp<bool, M>) -> Self {
        self.edit = self.edit.disabled(disabled);
        self
    }

    /// Chamado com o texto escolhido: Enter no campo, um clique numa sugestão ou no botão do
    /// [`AutoSuggestBox::query_icon`].
    pub fn on_submit(self, submit: impl FnMut(String) + 'static) -> Self {
        *self.shared.submit.borrow_mut() = Some(Box::new(submit));
        self
    }

    /// Texto atual.
    pub fn value(&self) -> String {
        self.edit.value()
    }

    /// Se a lista de sugestões está aberta agora.
    pub fn is_open(&self) -> bool {
        self.shared.open.get() && !self.shared.suggestions().is_empty()
    }

    /// Anda `delta` (1 ou -1) pela lista.
    fn step(&self, delta: isize) -> bool {
        let s = &self.shared;
        let list = s.suggestions_untracked();
        if list.is_empty() {
            return false;
        }
        let current = s
            .highlight
            .with_untracked(|h| h.as_ref().and_then(|h| list.iter().position(|o| o == h)));
        let next = match (current, delta > 0) {
            (None, true) => Some(0),
            (None, false) => Some(list.len() - 1),
            (Some(i), true) => Some((i + 1).min(list.len() - 1)),
            (Some(0), false) => None,
            (Some(i), false) => Some(i - 1),
        };
        if !s.open.get_untracked() {
            s.open.set(true);
        }
        match next {
            Some(i) => {
                s.set_value(&list[i]);
                s.highlight.set(Some(list[i].clone()));
            }
            None => {
                s.set_value(&s.query.get_untracked());
                s.highlight.set(None);
            }
        }
        true
    }
}

impl AutoSuggestBox {
    /// Id estável, usado em testes, no MCP e no lint.
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.edit = self.edit.id(id);
        self
    }

    /// Fator de crescimento no eixo principal do contêiner.
    pub fn grow(mut self, grow: impl Into<f64>) -> Self {
        self.edit = self.edit.grow(grow);
        self
    }

    /// Largura fixa, em pixels.
    pub fn width(mut self, width: impl Into<f64>) -> Self {
        self.edit = self.edit.width(width);
        self
    }

    /// Altura fixa, em pixels.
    pub fn height(mut self, height: impl Into<f64>) -> Self {
        self.edit = self.edit.height(height);
        self
    }

    /// Mostra ou esconde (valor fixo, closure ou signal).
    pub fn visible<M>(mut self, visible: impl IntoProp<bool, M>) -> Self {
        self.edit = self.edit.visible(visible);
        self
    }

    /// Dica que aparece com o ponteiro parado em cima.
    pub fn tooltip<M>(mut self, text: impl IntoProp<String, M>) -> Self {
        self.edit = self.edit.tooltip(text);
        self
    }
}

impl Widget for AutoSuggestBox {
    fn kind(&self) -> &'static str {
        "auto_suggest_box"
    }

    fn common(&self) -> &Common {
        self.edit.common()
    }

    fn look(&self) -> Look {
        self.edit.look()
    }

    fn flex(&self) -> Option<Flex> {
        self.edit.flex()
    }

    fn measured(&self) -> bool {
        self.edit.measured()
    }

    fn set_density(&mut self, density: Density) {
        self.edit.set_density(density);
    }

    fn names_children(&self) -> bool {
        self.edit.names_children()
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        if let Some(symbol) = self.query_icon {
            let shared = self.shared.clone();
            let disabled = self.edit.disabled.clone();
            self.edit.push_part(Box::new(move |edit: &Edit| {
                let shown: Rc<dyn Fn() -> bool> = Rc::new(|| true);
                let enabled = Prop::Dynamic(Rc::new(move || !disabled.get()));
                Part {
                    widget: Box::new(
                        FieldButton::new(
                            edit.part_id("buscar"),
                            crate::strings().query.to_string(),
                            FieldGlyph::Symbol(symbol),
                            QUERY_WIDTH,
                            Rc::clone(&shown),
                            FieldAction::Click(Rc::new(move || {
                                shared.submit(shared.value.get_untracked());
                            })),
                        )
                        .enabled(enabled),
                    ),
                    width: QUERY_WIDTH,
                    shown,
                }
            }));
        }
        let mut children = Widget::take_children(&mut self.edit);
        let id = self.common().id.clone();
        let shared = self.shared.clone();
        let items = shared.clone();
        let prefix = id.clone();
        let list = For::each(
            move || items.suggestions(),
            String::clone,
            move |label: String| SuggestionItem {
                common: Common::generated(
                    prefix
                        .as_ref()
                        .map(|id| format!("{id}-{}", option_slug(&label))),
                ),
                label,
                shared: shared.clone(),
                press: Pressable::default(),
                density: Density::Standard,
            },
        );
        let mut scroll = ScrollViewer::new()
            .child(Column::new().child(list))
            .generated();
        if let Some(id) = &id {
            scroll = scroll.id(format!("{id}-rolagem"));
        }
        children.push(Box::new(SuggestionList {
            common: Common::generated(id.map(|id| format!("{id}-lista"))),
            shared: self.shared.clone(),
            child: Some(Box::new(scroll)),
            density: Density::Standard,
        }));
        children
    }

    fn handle(&mut self, event: &Event, cx: &mut EventCx) -> bool {
        if self.edit.is_disabled() {
            self.shared.close();
            return Widget::handle(&mut self.edit, event, cx);
        }
        let focused = self.edit.is_focused();
        let plain = !cx.modifiers().command() && !cx.modifiers().alt;
        match event {
            Event::KeyDown(Key::Down) if focused && plain => {
                if self.step(1) {
                    return true;
                }
            }
            Event::KeyDown(Key::Up) if focused && plain => {
                if self.shared.open.get_untracked() && self.step(-1) {
                    return true;
                }
            }
            Event::KeyDown(Key::Enter) if focused => {
                let open = self.is_open_untracked();
                if open || self.shared.submit.borrow().is_some() {
                    self.shared.submit(self.shared.value.get_untracked());
                    return true;
                }
            }
            Event::KeyDown(Key::Escape) if focused && self.is_open_untracked() => {
                if self.shared.highlight.with_untracked(Option::is_some) {
                    self.shared.set_value(&self.shared.query.get_untracked());
                }
                self.shared.close();
                return true;
            }
            Event::FocusOut => self.shared.close(),
            _ => {}
        }
        let before = self.shared.value.get_untracked();
        let consumed = Widget::handle(&mut self.edit, event, cx);
        let after = self.shared.value.get_untracked();
        if after != before {
            // Mudou por digitação, colagem, desfazer...: é uma busca nova.
            self.shared.query.set(after.clone());
            if self.shared.highlight.with_untracked(Option::is_some) {
                self.shared.highlight.set(None);
            }
            let open = self.edit.is_focused() && !(self.shared.suggest)(&after).is_empty();
            if self.shared.open.get_untracked() != open {
                self.shared.open.set(open);
            }
        }
        consumed
    }

    fn interactive(&self) -> bool {
        true
    }

    fn focusable(&self) -> bool {
        self.edit.focusable()
    }

    fn enabled(&self) -> bool {
        self.edit.enabled()
    }

    fn text(&self) -> Option<String> {
        Some(self.edit.value())
    }

    fn text_input(&self) -> Option<InputPurpose> {
        self.edit.text_input()
    }

    fn take_focus_request(&mut self) -> Option<String> {
        if self.focus_wanted.replace(false) {
            self.common().id.clone()
        } else {
            None
        }
    }
}

impl AutoSuggestBox {
    fn is_open_untracked(&self) -> bool {
        self.shared.open.get_untracked() && !self.shared.suggestions_untracked().is_empty()
    }
}

impl fmt::Debug for AutoSuggestBox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AutoSuggestBox")
            .field("edit", &self.edit)
            .field("open", &self.shared.open.get_untracked())
            .field("query_icon", &self.query_icon)
            .finish_non_exhaustive()
    }
}

/// O popup com as sugestões de um [`AutoSuggestBox`]: não tira o foco do campo.
struct SuggestionList {
    common: Common,
    shared: Shared,
    child: Option<Box<dyn Widget>>,
    density: Density,
}

impl Widget for SuggestionList {
    fn kind(&self) -> &'static str {
        "auto_suggest_list"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn names_children(&self) -> bool {
        true
    }

    fn visible(&self) -> bool {
        self.shared.open.get() && !self.shared.suggestions().is_empty()
    }

    fn look(&self) -> Look {
        Look::Popup(PopupKind::ComboBox)
    }

    fn flex(&self) -> Option<Flex> {
        Some(Flex {
            padding: LIST_PADDING,
            ..Flex::default()
        })
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        self.child.take().into_iter().collect()
    }

    fn layer(&self) -> Option<Layer> {
        Some(Layer {
            input: false,
            pointer: true,
            light_dismiss: false,
            match_width: true,
            max_size: Size::new(
                f64::INFINITY,
                LIST_ROWS * item_height(self.density) + LIST_PADDING.top + LIST_PADDING.bottom,
            ),
            ..Layer::default()
        })
    }

    fn set_open(&mut self, open: bool) {
        if !open {
            self.shared.close();
        }
    }

    fn set_density(&mut self, density: Density) {
        self.density = density;
    }
}

/// Uma sugestão da lista.
struct SuggestionItem {
    common: Common,
    label: String,
    shared: Shared,
    press: Pressable,
    density: Density,
}

impl Widget for SuggestionItem {
    fn kind(&self) -> &'static str {
        "auto_suggest_item"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::ComboBoxItem {
            label: self.label.clone(),
            visual: ComboBoxItemVisual {
                state: self.press.state(),
                selected: self
                    .shared
                    .highlight
                    .with(|h| h.as_deref() == Some(self.label.as_str())),
                focus_ring: false,
                density: self.density,
            },
        }
    }

    fn set_density(&mut self, density: Density) {
        self.density = density;
    }

    fn handle(&mut self, event: &Event, _: &mut EventCx) -> bool {
        let (consumed, clicked) = self.press.handle(event, false);
        if clicked {
            self.shared.set_value(&self.label);
            self.shared.submit(self.label.clone());
        }
        consumed
    }

    fn interactive(&self) -> bool {
        true
    }

    fn text(&self) -> Option<String> {
        Some(self.label.clone())
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use kurbo::{Point, Rect};
    use tomni_ui_reactive::prelude::*;

    use super::*;
    use crate::cx::{FixedMetrics, MemoryClipboard};
    use crate::event::{FocusSource, Modifiers, PointerKind::Mouse};

    const CIDADES: [&str; 3] = ["Belo Horizonte", "Belém", "Recife"];

    fn sugestoes(texto: &str) -> Vec<String> {
        let texto = texto.to_lowercase();
        CIDADES
            .iter()
            .filter(|c| !texto.is_empty() && c.to_lowercase().starts_with(&texto))
            .map(|c| c.to_string())
            .collect()
    }

    fn send(w: &mut dyn Widget, event: Event) -> bool {
        let mut metrics = FixedMetrics::default();
        let mut clipboard = MemoryClipboard::default();
        let mut cx = EventCx::new(
            Rect::new(0.0, 0.0, 200.0, 32.0),
            Point::ZERO,
            Modifiers::NONE,
            &mut metrics,
            &mut clipboard,
        );
        w.handle(&event, &mut cx)
    }

    fn with(test: impl FnOnce(Signal<String>, AutoSuggestBox, Rc<RefCell<Vec<String>>>)) {
        let scope = Scope::root();
        scope.run(|| {
            let s = signal(String::new());
            let enviados = Rc::new(RefCell::new(Vec::new()));
            let log = Rc::clone(&enviados);
            let mut campo = AutoSuggestBox::bind(s, sugestoes)
                .id("cidade")
                .on_submit(move |c| log.borrow_mut().push(c));
            send(&mut campo, Event::FocusIn(FocusSource::Keyboard));
            test(s, campo, enviados);
        });
        scope.clear();
    }

    #[test]
    fn digitar_abre_e_setas_destacam() {
        with(|s, mut campo, enviados| {
            assert!(!campo.is_open());
            send(&mut campo, Event::Text("be".into()));
            assert!(campo.is_open());
            assert!(send(&mut campo, Event::KeyDown(Key::Down)));
            assert_eq!(s.get(), "Belo Horizonte");
            send(&mut campo, Event::KeyDown(Key::Down));
            send(&mut campo, Event::KeyDown(Key::Down));
            assert_eq!(s.get(), "Belém", "para na última");
            send(&mut campo, Event::KeyDown(Key::Up));
            send(&mut campo, Event::KeyDown(Key::Up));
            assert_eq!(s.get(), "be", "acima da primeira volta ao digitado");
            assert!(campo.is_open());
            send(&mut campo, Event::KeyDown(Key::Up));
            send(&mut campo, Event::KeyDown(Key::Enter));
            assert_eq!(s.get(), "Belém");
            assert!(!campo.is_open());
            assert_eq!(*enviados.borrow(), ["Belém"]);
        });
    }

    #[test]
    fn esc_volta_ao_digitado_e_foco_fora_fecha() {
        with(|s, mut campo, _| {
            send(&mut campo, Event::Text("re".into()));
            send(&mut campo, Event::KeyDown(Key::Down));
            assert_eq!(s.get(), "Recife");
            assert!(send(&mut campo, Event::KeyDown(Key::Escape)));
            assert_eq!(s.get(), "re");
            assert!(!campo.is_open());
            assert!(
                !send(&mut campo, Event::KeyDown(Key::Escape)),
                "fechada, Esc passa"
            );
            send(&mut campo, Event::Text("c".into()));
            assert!(campo.is_open());
            send(&mut campo, Event::FocusOut);
            assert!(!campo.is_open());
            send(&mut campo, Event::FocusIn(FocusSource::Keyboard));
            send(&mut campo, Event::Text("x".into()));
            assert!(!campo.is_open(), "sem sugestões, não abre");
        });
    }

    #[test]
    fn clique_na_sugestao_escolhe() {
        with(|s, mut campo, enviados| {
            let mut lista = campo.take_children().remove(0);
            assert_eq!(lista.common().id.as_deref(), Some("cidade-lista"));
            send(&mut campo, Event::Text("rec".into()));
            assert!(lista.visible());
            let mut item = SuggestionItem {
                common: Common::default(),
                label: "Recife".into(),
                shared: campo.shared.clone(),
                press: Pressable::default(),
                density: Density::Standard,
            };
            send(
                &mut item,
                Event::PointerDown {
                    kind: Mouse,
                    clicks: 1,
                },
            );
            send(
                &mut item,
                Event::PointerUp {
                    kind: Mouse,
                    inside: true,
                },
            );
            assert_eq!(s.get(), "Recife");
            assert!(!lista.visible());
            assert_eq!(*enviados.borrow(), ["Recife"]);
            lista.set_open(false);
        });
    }
}
