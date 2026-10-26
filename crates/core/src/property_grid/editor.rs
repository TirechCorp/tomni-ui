//! Os editores do valor: o que a linha escolhida mostra no lugar do texto em repouso, e o
//! fragmento que monta e desmonta esse editor.

use std::fmt;
use std::path::Path;
use std::rc::Rc;

use tomni_ui_reactive::prelude::*;

use super::model::{Filter, Kind, RowKey, Source};
use super::shared::Shared;
use crate::button::Button;
use crate::checkbox::CheckBox;
use crate::combo_box::ComboBox;
use crate::containers::Row;
use crate::cx::{EventCx, InputPurpose};
use crate::density::Density;
use crate::edit::Edit;
use crate::event::{Event, Key};
use crate::number_box::NumberBox;
use crate::symbol::Symbol;
use crate::tree_view::item_height;
use crate::widget::{Common, Flex, Look, Slot, Widget};

/// O invólucro do `Edit` e do `NumberBox` de uma propriedade: o valor só vai para o app
/// quando o editor confirma (Enter, Tab ou perder o foco); o Esc desfaz.
pub(crate) struct Editor<W: Widget> {
    inner: W,
    shared: Rc<Shared>,
    row: RowKey,
    /// Escreve o rascunho no signal do app, se o valor mudou.
    commit: Box<dyn Fn()>,
    /// O que fazer quando o editor recebe o foco: o texto digitado na linha, se houve.
    focus: fn(&mut W, Option<String>),
    /// Num editor só de leitura (o `.button`), o que Enter e espaço fazem; o texto não muda.
    activate: Option<Rc<dyn Fn()>>,
}

impl<W: Widget> Editor<W> {
    pub(crate) fn new(
        inner: W,
        shared: &Rc<Shared>,
        row: RowKey,
        commit: Box<dyn Fn()>,
        focus: fn(&mut W, Option<String>),
    ) -> Self {
        Self {
            inner,
            shared: Rc::clone(shared),
            row,
            commit,
            focus,
            activate: None,
        }
    }

    /// O editor passa a ser só de leitura: Enter e espaço chamam `click`, e digitar, apagar,
    /// colar e desfazer não fazem nada.
    pub(crate) fn activating(mut self, click: Rc<dyn Fn()>) -> Self {
        self.activate = Some(click);
        self
    }

    /// Devolve o foco à linha.
    fn back_to_row(&self, cx: &mut EventCx) {
        if let Some(id) = self.shared.row_id(self.row) {
            cx.focus_later(id);
        }
    }
}

impl<W: Widget> Widget for Editor<W> {
    fn kind(&self) -> &'static str {
        self.inner.kind()
    }

    fn common(&self) -> &Common {
        self.inner.common()
    }

    fn look(&self) -> Look {
        self.inner.look()
    }

    fn set_density(&mut self, density: Density) {
        self.inner.set_density(density);
    }

    fn flex(&self) -> Option<Flex> {
        self.inner.flex()
    }

    fn measured(&self) -> bool {
        self.inner.measured()
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        self.inner.take_children()
    }

    fn names_children(&self) -> bool {
        self.inner.names_children()
    }

    fn handle(&mut self, event: &Event, cx: &mut EventCx) -> bool {
        if let Some(click) = &self.activate {
            match event {
                Event::KeyDown(Key::Enter | Key::Space) => {
                    click();
                    return true;
                }
                Event::KeyDown(Key::Escape) => {
                    self.back_to_row(cx);
                    return true;
                }
                Event::Text(_) | Event::Ime(_) => return true,
                Event::KeyDown(Key::Backspace | Key::Delete) => return true,
                Event::KeyDown(Key::Char('x' | 'v' | 'z' | 'y')) if cx.modifiers().command() => {
                    return true;
                }
                _ => return self.inner.handle(event, cx),
            }
        }
        match event {
            Event::KeyDown(Key::Enter) => {
                self.inner.handle(event, cx);
                (self.commit)();
                self.back_to_row(cx);
                return true;
            }
            Event::KeyDown(Key::Escape) => {
                self.shared.revert();
                self.back_to_row(cx);
                return true;
            }
            _ => {}
        }
        let consumed = self.inner.handle(event, cx);
        match event {
            Event::FocusIn(_) => {
                let typed = self.shared.pending_text.borrow_mut().take();
                (self.focus)(&mut self.inner, typed);
            }
            Event::FocusOut => (self.commit)(),
            _ => {}
        }
        consumed
    }

    fn interactive(&self) -> bool {
        self.inner.interactive()
    }

    fn focusable(&self) -> bool {
        self.inner.focusable()
    }

    fn enabled(&self) -> bool {
        self.inner.enabled()
    }

    fn text(&self) -> Option<String> {
        self.inner.text()
    }

    fn text_input(&self) -> Option<InputPurpose> {
        self.inner.text_input()
    }
}

impl<W: Widget> fmt::Debug for Editor<W> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Editor")
            .field("inner", &self.inner.kind())
            .finish_non_exhaustive()
    }
}

/// O `Edit` ao receber o foco: mostra o texto digitado na linha, com o cursor no fim, ou
/// seleciona o texto todo.
fn focus_edit(edit: &mut Edit, typed: Option<String>) {
    match typed {
        Some(text) => edit.replace_text(text),
        None => edit.select_all(),
    }
}

/// O `NumberBox` seleciona o texto sozinho; o texto digitado na linha não vale para ele.
fn focus_number(_: &mut NumberBox, _: Option<String>) {}

/// O `Edit` do `.button` não seleciona nada ao receber o foco.
fn focus_none(_: &mut Edit, _: Option<String>) {}

/// Um resumo das opções de uma `.choice`, para saber se a lista mudou.
fn options_digest(options: &[String]) -> u64 {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    options.hash(&mut hasher);
    hasher.finish()
}

/// O que identifica o editor montado: outro signal ligado, ou o Esc que desfez o rascunho,
/// monta o editor de novo.
#[derive(Clone, Copy, PartialEq)]
enum EditorKey {
    Text(Signal<String>, u64),
    Number(Signal<f64>, u64),
    /// O valor e um resumo das opções: o editor se refaz quando elas mudam.
    Choice(Signal<String>, u64),
    Check(Signal<bool>),
    /// O resumo do `.button`.
    Button(Signal<String>),
}

/// O fragmento com o editor de uma linha: nenhum filho, ou um.
pub(crate) struct EditorHost {
    common: Common,
    shared: Rc<Shared>,
    index: usize,
    mounted: Option<Option<EditorKey>>,
}

impl EditorHost {
    pub(crate) fn new(shared: &Rc<Shared>, index: usize) -> Self {
        Self {
            common: Common::default(),
            shared: Rc::clone(shared),
            index,
            mounted: None,
        }
    }

    /// O editor que a linha pede agora; lê os signals de forma rastreada.
    ///
    /// A `.check` fica à vista em toda linha. Os outros tipos só têm editor na linha
    /// escolhida e habilitada.
    fn wanted(&self) -> Option<EditorKey> {
        let shared = &self.shared;
        let properties = shared.properties.borrow();
        let property = properties.get(self.index)?;
        if let (Kind::Check, Source::Bool(prop)) = (&property.kind, &property.source) {
            return Some(EditorKey::Check(prop.get()));
        }
        if shared.selected.get() != Some(RowKey::Property(self.index))
            || property.disabled.get()
            || !property.editable()
        {
            return None;
        }
        let epoch = shared.epoch.get();
        match (&property.kind, &property.source) {
            (Kind::Choice(options), Source::Text(prop)) => {
                Some(EditorKey::Choice(prop.get(), options_digest(&options())))
            }
            (Kind::Number { .. }, Source::Number(prop)) => {
                Some(EditorKey::Number(prop.get(), epoch))
            }
            (Kind::Button(_), Source::Text(prop)) => Some(EditorKey::Button(prop.get())),
            (_, Source::Text(prop)) => Some(EditorKey::Text(prop.get(), epoch)),
            _ => None,
        }
    }

    fn build(shared: &Rc<Shared>, index: usize, key: EditorKey) -> Box<dyn Widget> {
        let properties = shared.properties.borrow();
        let property = &properties[index];
        let id = shared.editor_id(index);
        let row = RowKey::Property(index);
        // O editor tem a altura da linha (44 px na padrão, o alvo de toque).
        let height = item_height(shared.density.get());
        match key {
            EditorKey::Check(target) => {
                let disabled = property.disabled.clone();
                let read_only = property.read_only;
                Box::new(
                    CheckBox::bind(target, "")
                        .generated_id(id)
                        .height(height)
                        .disabled(move || read_only || disabled.get()),
                )
            }
            EditorKey::Choice(target, _) => {
                let options = match &property.kind {
                    Kind::Choice(options) => options(),
                    _ => Vec::new(),
                };
                shared.choice_open.set(false);
                Box::new(
                    ComboBox::bind(target, options)
                        .generated_id(id)
                        .open_signal(shared.choice_open)
                        .height(height)
                        .grow(1),
                )
            }
            EditorKey::Number(target, _) => {
                let (min, max) = match property.kind {
                    Kind::Number { min, max } => (min, max),
                    _ => (f64::MIN, f64::MAX),
                };
                let draft = signal(target.get_untracked());
                follow(target, draft);
                let commit = Box::new(move || {
                    let value = draft.get_untracked();
                    if target.get_untracked() != value {
                        target.set(value);
                    }
                });
                let number = NumberBox::bind(draft)
                    .min(min)
                    .max(max)
                    .generated_id(id)
                    .height(height)
                    .grow(1);
                Box::new(Editor::new(number, shared, row, commit, focus_number))
            }
            EditorKey::Button(target) => {
                let Kind::Button(click) = &property.kind else {
                    unreachable!("a chave do botão vem de um `Kind::Button`");
                };
                let edit = Edit::bind(target).generated_id(id).height(height).grow(1);
                let editor = Editor::new(edit, shared, row, Box::new(|| {}), focus_none)
                    .activating(Rc::clone(click));
                let click = Rc::clone(click);
                let browse = Button::icon_only(Symbol::MoreHorizontal)
                    .label(format!("Abrir {}", property.name))
                    .generated_id(shared.sub(&format!("{}-botao", property.name)))
                    .on_click(move || click());
                Box::new(
                    Row::new()
                        .gap(4)
                        .grow(1)
                        .height(height)
                        .child(editor)
                        .child(browse),
                )
            }
            EditorKey::Text(target, _) => {
                let draft = signal(target.get_untracked());
                follow(target, draft);
                let commit = Box::new(move || {
                    let value = draft.get_untracked();
                    if target.get_untracked() != value {
                        target.set(value);
                    }
                });
                let mut edit = Edit::bind(draft).generated_id(id).height(height).grow(1);
                if matches!(property.kind, Kind::Password) {
                    edit = edit.password();
                }
                let editor = Editor::new(edit, shared, row, commit, focus_edit);
                match &property.kind {
                    Kind::File(_) | Kind::Folder if crate::dialogs::is_supported() => {
                        let name = property.name.clone();
                        let filters = match &property.kind {
                            Kind::File(filters) => Some(filters.clone()),
                            _ => None,
                        };
                        let browse = Button::icon_only(Symbol::MoreHorizontal)
                            .label(format!("Procurar {name}"))
                            .generated_id(shared.sub(&format!("{name}-procurar")))
                            .on_click(browse(name, filters, target, draft));
                        Box::new(
                            Row::new()
                                .gap(4)
                                .grow(1)
                                .height(height)
                                .child(editor)
                                .child(browse),
                        )
                    }
                    _ => Box::new(editor),
                }
            }
        }
    }
}

/// O que o botão "…" de uma `.file` ou `.folder` faz: pede o arquivo (`filters` é `Some`) ou a
/// pasta (`None`) ao seletor do sistema, abrindo na pasta do valor atual (se ela existir). Com
/// um caminho, ele vira o valor; cancelar não muda nada.
fn browse(
    name: String,
    filters: Option<Vec<Filter>>,
    target: Signal<String>,
    draft: Signal<String>,
) -> impl FnMut() + 'static {
    move || {
        let mut dialog = match &filters {
            Some(filters) => {
                let mut dialog = crate::dialogs::open_file().title(name.clone());
                for filter in filters {
                    let extensions: Vec<&str> =
                        filter.extensions.iter().map(String::as_str).collect();
                    dialog = dialog.filter(filter.name.clone(), &extensions);
                }
                dialog
            }
            None => crate::dialogs::pick_folder().title(name.clone()),
        };
        let current = draft.get_untracked();
        let current = Path::new(&current);
        // Numa pasta, o diálogo abre nela; num arquivo, na pasta que o contém.
        let start = if filters.is_some() {
            current.parent()
        } else {
            Some(current)
        };
        if let Some(folder) = start.filter(|p| p.is_dir()) {
            dialog = dialog.directory(folder);
        }
        dialog.show(move |path| {
            if let Some(path) = path {
                let path = path.to_string_lossy().into_owned();
                draft.set(path.clone());
                if target.get_untracked() != path {
                    target.set(path);
                }
            }
        });
    }
}

/// O rascunho acompanha o valor que o app muda por código.
fn follow<T>(target: Signal<T>, draft: Signal<T>)
where
    T: Clone + PartialEq + Send + Sync + 'static,
{
    effect(move || {
        let value = target.get();
        if draft.get_untracked() != value {
            draft.set(value);
        }
    });
}

impl Widget for EditorHost {
    fn kind(&self) -> &'static str {
        "property_grid_editor"
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
        let _ = self.wanted();
    }

    fn reconcile(&mut self) -> Option<Vec<Slot>> {
        let wanted = untrack(|| self.wanted());
        if self.mounted == Some(wanted) {
            return None;
        }
        self.mounted = Some(wanted);
        let slots = match wanted {
            Some(key) => {
                let shared = Rc::clone(&self.shared);
                let index = self.index;
                vec![Slot::New(Box::new(move || {
                    Self::build(&shared, index, key)
                }))]
            }
            None => Vec::new(),
        };
        Some(slots)
    }
}

impl fmt::Debug for EditorHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EditorHost")
            .field("index", &self.index)
            .finish_non_exhaustive()
    }
}
