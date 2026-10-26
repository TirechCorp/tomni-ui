//! `Edit`: campo de texto ligado a um signal, de uma linha ou, com `multiline`, de várias.

use std::fmt;
use std::rc::Rc;

use kurbo::Rect;
use tomni_ui_reactive::{Get, GetUntracked, Set, Signal, With, WithUntracked, signal, untrack};

use crate::cx::{EventCx, InputPurpose};
use crate::density::Density;
use crate::event::{Event, FocusSource, Ime, Key, Modifiers};
use crate::field_button::{FieldAction, FieldButton, FieldGlyph};
use crate::prop::{IntoProp, Prop};
use crate::widget::{Align, Common, Direction, Flex, Justify, Look, Widget, common_builders};

/// Caractere que esconde a senha. O `PasswordChar` do WinUI é o ● (U+25CF), que a Selawik
/// não tem; fica o • do `PasswordBox` do `fluent_ui`.
pub const PASSWORD_CHAR: char = '•';

/// Altura natural do campo (a `TextControlThemeMinHeight` do WinUI), para o olho da senha
/// num campo sem `.height`: 32 px, ou 24 na densidade compacta.
pub(crate) fn field_height(density: Density) -> f64 {
    match density {
        Density::Standard => 32.0,
        Density::Compact => 24.0,
    }
}

/// Quantos passos o undo guarda.
const UNDO_LIMIT: usize = 100;

/// Estado visual do `Edit`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum EditState {
    /// Parado.
    #[default]
    Rest,
    /// Ponteiro sobre o campo, sem foco.
    Hover,
    /// Com foco: fundo claro e linha inferior na cor accent. Vence o hover.
    Focused,
    /// Desabilitado.
    Disabled,
}

/// Como o campo de senha deixa ver o que foi digitado: o `PasswordRevealMode` do WinUI.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PasswordReveal {
    /// Botão de olho no fim do campo, que mostra a senha enquanto está pressionado (padrão). O
    /// botão some quando o campo perde o foco com texto e só volta quando o campo fica vazio:
    /// quem chega depois não vê a senha de outra pessoa.
    #[default]
    Peek,
    /// Sem botão: a senha nunca aparece.
    Hidden,
    /// Senha visível, sem botão.
    Visible,
}

/// Uma parte do campo: um botão no fim dele, montado como filho.
pub(crate) struct Part {
    pub(crate) widget: Box<dyn Widget>,
    /// Largura que ocupa no fim do campo, tirada da área do texto enquanto aparece.
    pub(crate) width: f64,
    /// Se aparece agora (lido de forma rastreada).
    pub(crate) shown: Rc<dyn Fn() -> bool>,
}

/// Monta uma [`Part`] na montagem do campo, quando o id dele já é conhecido.
pub(crate) type MakePart = Box<dyn FnOnce(&Edit) -> Part>;

/// Tudo o que o `Drawer` precisa para pintar o `Edit`. Posições em bytes de `text`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EditVisual {
    /// Texto exibido: com a máscara no modo senha e a composição do IME no cursor.
    pub text: String,
    /// Mostrado quando `text` está vazio.
    pub placeholder: String,
    /// Cursor, só com foco.
    pub caret: Option<usize>,
    /// Trecho selecionado `(início, fim)`, só com foco e nunca vazio.
    pub selection: Option<(usize, usize)>,
    /// Trecho em composição no IME, sublinhado.
    pub composing: Option<(usize, usize)>,
    /// Quanto o texto está rolado para a esquerda, em pixels.
    pub scroll: f64,
    /// Campo de várias linhas: o texto fica no alto, com as quebras `\n`.
    pub multiline: bool,
    /// Quanto o texto de várias linhas está rolado para cima, em pixels.
    pub scroll_y: f64,
    /// Largura no fim do campo ocupada pelos botões de dentro (revelar a senha, limpar), fora
    /// da área do texto.
    pub trailing: f64,
    /// Estado.
    pub state: EditState,
    /// Densidade: na compacta, o campo tem 24 px e o padding do `Compact.xaml` do WinUI.
    pub density: Density,
}

/// Classe de um caractere para o duplo clique e o Ctrl+setas.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Word,
    Space,
    Punct,
}

fn class(c: char) -> Class {
    if c.is_alphanumeric() || c == '_' {
        Class::Word
    } else if c.is_whitespace() {
        Class::Space
    } else {
        Class::Punct
    }
}

/// Estado guardado pelo undo.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Snapshot {
    text: String,
    cursor: usize,
    anchor: usize,
}

/// Campo de texto de uma linha, com ligação bidirecional ao signal.
///
/// ```
/// use tomni_ui_core::Edit;
/// use tomni_ui_reactive::prelude::*;
///
/// # let scope = Scope::root();
/// # scope.run(|| {
/// let email = signal(String::new());
/// let campo = Edit::bind(email).id("email").placeholder("E-mail");
/// let senha = Edit::bind(signal(String::new())).id("senha").password();
/// # });
/// # scope.clear();
/// ```
///
/// Regras (spec, seção Widgets):
///
/// - Cursor, seleção com mouse, Shift+setas e duplo clique para selecionar a palavra (triplo
///   seleciona tudo); Home e End; Ctrl+setas e Ctrl+Backspace andam e apagam por palavra
///   (Option no macOS).
/// - Ctrl/Cmd + A, C, X, V, Z, e Y ou Shift+Z para refazer. No modo senha não há como copiar,
///   mesmo com a senha à mostra.
/// - Quebras de linha coladas viram espaço; caracteres de controle digitados são ignorados.
/// - Placeholder quando vazio, rolagem horizontal quando o texto passa da largura.
/// - A composição do IME aparece sublinhada no cursor e some se o foco sair.
/// - Desabilitado não recebe foco nem eventos.
pub struct Edit {
    common: Common,
    value: Signal<String>,
    placeholder: Prop<String>,
    password: bool,
    /// Várias linhas: Enter quebra a linha, as setas sobem e descem.
    multiline: bool,
    /// Só mostra e deixa selecionar e copiar.
    read_only: bool,
    reveal: Prop<PasswordReveal>,
    /// O botão de revelar está pressionado.
    peeking: Signal<bool>,
    /// O modo `Peek` mostra o botão de revelar: some quando o campo perde o foco com texto e
    /// volta quando ele fica vazio.
    can_peek: Signal<bool>,
    pub(crate) disabled: Prop<bool>,
    /// Se está com o foco, para as partes que só aparecem com ele.
    pub(crate) focused: Signal<bool>,
    /// Partes a montar ([`Widget::take_children`]).
    makers: Vec<MakePart>,
    /// Largura e visibilidade das partes montadas.
    parts: Vec<(f64, Rc<dyn Fn() -> bool>)>,
    /// Cursor e a outra ponta da seleção, em bytes do texto real.
    cursor: usize,
    anchor: usize,
    /// O texto como o campo o deixou da última vez. Se o signal mudou por fora, o cursor vai
    /// para o fim.
    known: String,
    /// Composição do IME e o trecho destacado nela.
    preedit: Option<(String, Option<(usize, usize)>)>,
    hovered: bool,
    dragging: bool,
    focus: Option<FocusSource>,
    scroll: f64,
    scroll_y: f64,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// A última mudança foi digitação: a próxima entra no mesmo passo do undo.
    typing: bool,
    density: Density,
}

impl Edit {
    /// Campo que mostra e altera `value`.
    pub fn bind(value: Signal<String>) -> Self {
        Self {
            common: Common::default(),
            value,
            placeholder: Prop::default(),
            password: false,
            multiline: false,
            read_only: false,
            reveal: Prop::Value(PasswordReveal::Peek),
            peeking: signal(false),
            can_peek: signal(true),
            disabled: Prop::Value(false),
            focused: signal(false),
            makers: Vec::new(),
            parts: Vec::new(),
            cursor: 0,
            anchor: 0,
            known: value.get_untracked(),
            preedit: None,
            hovered: false,
            dragging: false,
            focus: None,
            scroll: 0.0,
            scroll_y: 0.0,
            undo: Vec::new(),
            redo: Vec::new(),
            density: Density::Standard,
            typing: false,
        }
    }

    /// Texto mostrado enquanto o campo está vazio (fixo, closure ou signal).
    pub fn placeholder<M>(mut self, placeholder: impl IntoProp<String, M>) -> Self {
        self.placeholder = placeholder.into_prop();
        self
    }

    /// Modo senha: mostra `•` no lugar de cada caractere, não deixa copiar e pede à
    /// plataforma um teclado de senha. Com texto, um botão de olho no fim do campo (id
    /// `{id}-revelar`) mostra a senha enquanto está pressionado; veja
    /// [`Edit::password_reveal`].
    pub fn password(mut self) -> Self {
        self.password = true;
        self
    }

    /// Como o campo de senha deixa ver o texto (fixo, closure ou signal): [`PasswordReveal`].
    /// O padrão é o `Peek`. Só vale com [`Edit::password`].
    pub fn password_reveal<M>(mut self, reveal: impl IntoProp<PasswordReveal, M>) -> Self {
        self.reveal = reveal.into_prop();
        self
    }

    /// Várias linhas: Enter quebra a linha, ↑ e ↓ andam entre elas, Home e End vão às pontas da
    /// linha (com Ctrl, do texto) e o que se cola mantém as quebras. O texto fica no alto do
    /// campo, que não cresce: dê uma altura com `.height` e o texto rola. Não quebra linhas
    /// compridas: rola na horizontal.
    pub fn multiline(mut self) -> Self {
        self.multiline = true;
        self
    }

    /// Só leitura: o campo recebe foco, seleciona e copia, mas não muda o texto.
    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }

    /// Desabilita o campo (valor fixo, closure ou signal).
    pub fn disabled<M>(mut self, disabled: impl IntoProp<bool, M>) -> Self {
        self.disabled = disabled.into_prop();
        self
    }

    /// Texto atual.
    pub fn value(&self) -> String {
        self.value.get_untracked()
    }

    /// Se está desabilitado agora.
    pub fn is_disabled(&self) -> bool {
        self.disabled.get()
    }

    /// Se está com o foco.
    pub fn is_focused(&self) -> bool {
        self.focus.is_some()
    }

    /// Se a senha está à mostra agora: o modo `Visible` ou o botão de revelar pressionado.
    pub fn is_revealed(&self) -> bool {
        self.password && (self.reveal.get() == PasswordReveal::Visible || self.peeking.get())
    }

    /// Se o texto aparece escondido agora (modo senha, sem a senha à mostra).
    fn masked(&self) -> bool {
        self.password && !self.is_revealed()
    }

    /// Id derivado do id do grupo que monta o campo (o de renomear do `TreeView`).
    pub(crate) fn generated_id(mut self, id: Option<String>) -> Self {
        self.common = Common::generated(id);
        self
    }

    /// Seleciona o texto todo (o `NumberBox` ao receber o foco, como o `SelectAll` do WinUI).
    pub(crate) fn select_all(&mut self) {
        let text = self.value.get_untracked();
        self.anchor = 0;
        self.move_to(text.len(), true);
        self.known = text;
    }

    /// Troca o texto todo por `text` e leva o cursor ao fim (a digitação que a `PropertyGrid`
    /// recebe na linha e passa ao editor).
    pub(crate) fn replace_text(&mut self, text: String) {
        let end = text.len();
        self.value.set(text.clone());
        self.known = text;
        self.anchor = end;
        self.move_to(end, false);
        self.undo.clear();
        self.redo.clear();
    }

    /// Acrescenta uma parte no fim do campo, montada junto com ele.
    pub(crate) fn push_part(&mut self, make: MakePart) {
        self.makers.push(make);
    }

    /// Id de uma parte: `{id}-{suffix}`.
    pub(crate) fn part_id(&self, suffix: &str) -> Option<String> {
        self.common.id.as_ref().map(|id| format!("{id}-{suffix}"))
    }

    /// Se tem partes, montadas ou a montar: o campo vira um contêiner.
    fn has_parts(&self) -> bool {
        self.password || !self.makers.is_empty() || !self.parts.is_empty()
    }

    /// Largura das partes que aparecem (lida de forma rastreada).
    fn trailing(&self) -> f64 {
        self.parts
            .iter()
            .filter(|(_, shown)| shown())
            .map(|(width, _)| width)
            .sum()
    }

    /// Retângulo do campo sem as partes do fim, onde fica o texto.
    fn text_rect(&self, rect: Rect) -> Rect {
        let trailing = untrack(|| self.trailing());
        Rect::new(rect.x0, rect.y0, (rect.x1 - trailing).max(rect.x0), rect.y1)
    }

    fn set_focus(&mut self, focus: Option<FocusSource>) {
        self.focus = focus;
        if self.focused.get_untracked() != focus.is_some() {
            self.focused.set(focus.is_some());
        }
    }

    /// O botão de revelar a senha. Como no `PasswordBox` do WinUI, é quadrado, com a altura
    /// do campo, e a margem de dentro: num campo de 44 px, o fundo tem 40×36.
    fn reveal_part(edit: &Edit) -> Part {
        let width = edit.common.height.unwrap_or(field_height(edit.density));
        let value = edit.value;
        let reveal = edit.reveal.clone();
        let can_peek = edit.can_peek;
        let disabled = edit.disabled.clone();
        let peeking = edit.peeking;
        let shown: Rc<dyn Fn() -> bool> = Rc::new(move || {
            !disabled.get()
                && value.with(|v| !v.is_empty())
                && match reveal.get() {
                    PasswordReveal::Peek => can_peek.get(),
                    PasswordReveal::Hidden | PasswordReveal::Visible => false,
                }
        });
        let widget = FieldButton::new(
            edit.part_id("revelar"),
            crate::strings().show_password.to_string(),
            FieldGlyph::Reveal,
            width,
            Rc::clone(&shown),
            FieldAction::Hold(Rc::new(move |on| {
                if peeking.get_untracked() != on {
                    peeking.set(on);
                }
            })),
        );
        Part {
            widget: Box::new(widget),
            width,
            shown,
        }
    }

    /// Seleção `(início, fim)` em bytes do texto, ou o cursor `(c, c)`.
    pub fn selection(&self) -> (usize, usize) {
        let (cursor, anchor) = self.positions(&self.value());
        (cursor.min(anchor), cursor.max(anchor))
    }

    /// Estado para o `Drawer`. Lê o texto, o placeholder e o `disabled` de forma rastreada.
    pub fn visual(&self) -> EditVisual {
        let text = self.value.get();
        let placeholder = self.placeholder.get();
        let trailing = self.trailing();
        if self.is_disabled() {
            return EditVisual {
                text: self.display(&text),
                placeholder,
                scroll: self.scroll,
                scroll_y: self.scroll_y,
                multiline: self.multiline,
                trailing,
                state: EditState::Disabled,
                density: self.density,
                ..EditVisual::default()
            };
        }
        let (cursor, anchor) = self.positions(&text);
        let mut shown = self.display(&text);
        let at = self.shown_byte(&text, cursor);
        let mut visual = EditVisual {
            placeholder,
            scroll: self.scroll,
            scroll_y: self.scroll_y,
            multiline: self.multiline,
            trailing,
            density: self.density,
            state: if self.is_focused() {
                EditState::Focused
            } else if self.hovered {
                EditState::Hover
            } else {
                EditState::Rest
            },
            ..EditVisual::default()
        };
        if self.is_focused() {
            visual.caret = (!self.read_only).then_some(at);
            if let Some((preedit, highlight)) = &self.preedit {
                shown.insert_str(at, preedit);
                visual.composing = Some((at, at + preedit.len()));
                visual.caret = Some(at + highlight.map_or(preedit.len(), |(_, end)| end));
            } else if cursor != anchor {
                let other = self.shown_byte(&text, anchor);
                visual.selection = Some((at.min(other), at.max(other)));
            }
        }
        visual.text = shown;
        visual
    }

    /// Cursor e âncora dentro de `text`. Se o signal mudou por fora (um `set` do app, uma
    /// sugestão do `AutoSuggestBox`), os dois vão para o fim do texto novo.
    fn positions(&self, text: &str) -> (usize, usize) {
        if text != self.known {
            return (text.len(), text.len());
        }
        let fix = |i: usize| {
            if text.is_char_boundary(i) {
                i
            } else {
                text.len()
            }
        };
        (fix(self.cursor), fix(self.anchor))
    }

    /// Texto exibido sem a composição.
    fn display(&self, text: &str) -> String {
        if self.masked() {
            std::iter::repeat_n(PASSWORD_CHAR, text.chars().count()).collect()
        } else {
            text.to_owned()
        }
    }

    /// Byte do texto real para o byte do texto exibido (sem a composição).
    fn shown_byte(&self, text: &str, byte: usize) -> usize {
        if self.masked() {
            text[..byte].chars().count() * PASSWORD_CHAR.len_utf8()
        } else {
            byte
        }
    }

    /// Byte do texto exibido (sem a composição) para o byte do texto real.
    fn real_byte(&self, text: &str, byte: usize) -> usize {
        if self.masked() {
            let n = byte / PASSWORD_CHAR.len_utf8();
            text.char_indices().nth(n).map_or(text.len(), |(i, _)| i)
        } else {
            byte
        }
    }

    /// Onde o cursor pode parar no texto real: entre clusters (um `é` decomposto é um só) ou,
    /// no modo senha, entre caracteres, que é o que aparece.
    fn stops(&self, text: &str, cx: &mut EventCx) -> Vec<usize> {
        if self.masked() {
            text.char_indices()
                .map(|(i, _)| i)
                .chain([text.len()])
                .collect()
        } else {
            cx.metrics()
                .carets(text)
                .into_iter()
                .map(|(i, _)| i)
                .collect()
        }
    }

    /// Começo da palavra antes de `at`. No modo senha, as palavras não aparecem: vai ao início.
    fn word_before(&self, text: &str, at: usize) -> usize {
        if self.masked() {
            return 0;
        }
        word_before(text, at)
    }

    /// Começo da próxima palavra depois de `at`, como no Windows. No modo senha, vai ao fim.
    fn word_after(&self, text: &str, at: usize) -> usize {
        if self.masked() {
            return text.len();
        }
        word_after(text, at)
    }

    /// Palavra (ou sequência de espaços, ou de pontuação) em volta de `at`.
    fn word_at(&self, text: &str, at: usize) -> (usize, usize) {
        if self.masked() {
            return (0, text.len());
        }
        word_at(text, at)
    }

    /// Byte do texto real mais perto do ponteiro.
    fn hit(&self, text: &str, cx: &mut EventCx) -> usize {
        let rect = self.text_rect(cx.rect());
        if self.multiline {
            let area = cx.metrics().multiline_area(rect, self.density);
            let (x, y) = (
                cx.pointer().x - area.x0 + self.scroll,
                cx.pointer().y - area.y0 + self.scroll_y,
            );
            let points = cx.metrics().caret_points(text);
            let last = points.last().map_or(0, |p| p.2);
            let line = ((y / cx.metrics().line_height()).floor().max(0.0) as usize).min(last);
            return nearest_in_line(&points, line, x).unwrap_or(0);
        }
        let area = cx.metrics().text_area(rect, self.density);
        let x = cx.pointer().x - area.x0 + self.scroll;
        let shown = self.display(text);
        let carets = cx.metrics().carets(&shown);
        let byte = carets
            .iter()
            .min_by(|a, b| (a.1 - x).abs().total_cmp(&(b.1 - x).abs()))
            .map_or(0, |&(i, _)| i);
        self.real_byte(text, byte)
    }

    /// Rola o mínimo para o cursor (ou o fim da composição) ficar visível.
    fn ensure_visible(&mut self, cx: &mut EventCx) {
        let visual = self.visual_untracked();
        let rect = self.text_rect(cx.rect());
        if self.multiline {
            let area = cx.metrics().multiline_area(rect, self.density);
            let points = cx.metrics().caret_points(&visual.text);
            let line_height = cx.metrics().line_height();
            let caret = visual.caret.unwrap_or(self.cursor);
            if let Some(&(_, x, line)) = points.iter().find(|p| p.0 >= caret).or(points.last()) {
                if x - self.scroll > area.width() - 1.0 {
                    self.scroll = x - area.width() + 1.0;
                }
                if x < self.scroll {
                    self.scroll = x;
                }
                let top = line as f64 * line_height;
                if top < self.scroll_y {
                    self.scroll_y = top;
                }
                if top + line_height > self.scroll_y + area.height() {
                    self.scroll_y = top + line_height - area.height();
                }
            }
            self.clamp_scroll(&points, area, line_height);
            return;
        }
        let width = cx.metrics().text_area(rect, self.density).width();
        let carets = cx.metrics().carets(&visual.text);
        let total = carets.last().map_or(0.0, |c| c.1);
        if let Some(caret) = visual.caret {
            let x = carets.iter().find(|c| c.0 >= caret).map_or(total, |c| c.1);
            if x - self.scroll > width - 1.0 {
                self.scroll = x - width + 1.0;
            }
            if x < self.scroll {
                self.scroll = x;
            }
        }
        self.scroll = self.scroll.clamp(0.0, (total + 1.0 - width).max(0.0));
    }

    /// Mantém a rolagem do texto de várias linhas dentro do conteúdo.
    fn clamp_scroll(&mut self, points: &[(usize, f64, usize)], area: Rect, line_height: f64) {
        let width = points.iter().map(|p| p.1).fold(0.0, f64::max);
        let height = (points.last().map_or(0, |p| p.2) + 1) as f64 * line_height;
        self.scroll = self
            .scroll
            .clamp(0.0, (width + 1.0 - area.width()).max(0.0));
        self.scroll_y = self.scroll_y.clamp(0.0, (height - area.height()).max(0.0));
    }

    /// A roda sobre o campo de várias linhas: rola o texto. Devolve `false` se não há para onde
    /// rolar, e a tela em volta rola no lugar.
    fn scroll_by(&mut self, dx: i32, dy: i32, cx: &mut EventCx) -> bool {
        let rect = self.text_rect(cx.rect());
        let area = cx.metrics().multiline_area(rect, self.density);
        let text = self.visual_untracked().text;
        let points = cx.metrics().caret_points(&text);
        let line_height = cx.metrics().line_height();
        let before = (self.scroll, self.scroll_y);
        self.scroll += f64::from(dx);
        self.scroll_y += f64::from(dy);
        self.clamp_scroll(&points, area, line_height);
        before != (self.scroll, self.scroll_y)
    }

    fn visual_untracked(&self) -> EditVisual {
        tomni_ui_reactive::untrack(|| self.visual())
    }

    fn snapshot(&self, text: &str) -> Snapshot {
        Snapshot {
            text: text.to_owned(),
            cursor: self.cursor,
            anchor: self.anchor,
        }
    }

    /// Troca a seleção por `insert` e guarda o passo no undo.
    fn replace(&mut self, text: &str, insert: &str, typing: bool) {
        if self.read_only {
            return;
        }
        let (start, end) = (self.cursor.min(self.anchor), self.cursor.max(self.anchor));
        if start == end && insert.is_empty() {
            return;
        }
        if !(typing && self.typing) {
            self.undo.push(self.snapshot(text));
            if self.undo.len() > UNDO_LIMIT {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.typing = typing;
        let mut new = String::with_capacity(text.len() + insert.len());
        new.push_str(&text[..start]);
        new.push_str(insert);
        new.push_str(&text[end..]);
        self.cursor = start + insert.len();
        self.anchor = self.cursor;
        self.known.clone_from(&new);
        self.value.set(new);
    }

    /// Move o cursor; com `extend`, a âncora fica e a seleção cresce.
    fn move_to(&mut self, at: usize, extend: bool) {
        self.cursor = at;
        if !extend {
            self.anchor = at;
        }
        self.typing = false;
    }

    fn restore(&mut self, snapshot: Snapshot) {
        self.cursor = snapshot.cursor;
        self.anchor = snapshot.anchor;
        self.typing = false;
        self.known.clone_from(&snapshot.text);
        self.value.set(snapshot.text);
    }

    fn undo(&mut self, text: &str) -> bool {
        if self.read_only {
            return false;
        }
        let Some(snapshot) = self.undo.pop() else {
            return false;
        };
        self.redo.push(self.snapshot(text));
        self.restore(snapshot);
        true
    }

    fn redo(&mut self, text: &str) -> bool {
        if self.read_only {
            return false;
        }
        let Some(snapshot) = self.redo.pop() else {
            return false;
        };
        self.undo.push(self.snapshot(text));
        self.restore(snapshot);
        true
    }

    /// Trata um evento. Devolve `true` se o campo o consumiu.
    ///
    /// Desabilitado, ignora tudo e esquece hover, arrasto, foco e composição.
    pub fn handle(&mut self, event: &Event, cx: &mut EventCx) -> bool {
        if self.is_disabled() {
            self.hovered = false;
            self.dragging = false;
            self.set_focus(None);
            self.preedit = None;
            return false;
        }
        if self.multiline
            && let Event::Scroll { x, y } = event
        {
            return self.scroll_by(*x, *y, cx);
        }
        let text = self.value();
        if text.is_empty() && self.password && !self.can_peek.get_untracked() {
            // Vazio, o botão de revelar volta: não há senha de ninguém para ver.
            self.can_peek.set(true);
        }
        (self.cursor, self.anchor) = self.positions(&text);
        if text != self.known {
            // Mudou por fora: a próxima digitação é outro passo do undo.
            self.typing = false;
            self.known.clone_from(&text);
        }
        let consumed = self.event(&text, event, cx);
        if consumed {
            self.ensure_visible(cx);
        }
        consumed
    }

    fn event(&mut self, text: &str, event: &Event, cx: &mut EventCx) -> bool {
        match event {
            Event::PointerMove { kind, inside } => {
                self.hovered = *inside && kind.hovers();
                if self.dragging {
                    let at = self.hit(text, cx);
                    self.move_to(at, true);
                }
                true
            }
            Event::PointerDown { kind, clicks } => {
                self.hovered = kind.hovers();
                self.preedit = None;
                let at = self.hit(text, cx);
                match clicks {
                    2 => {
                        let (start, end) = self.word_at(text, at);
                        self.anchor = start;
                        self.move_to(end, true);
                    }
                    3.. => {
                        self.anchor = 0;
                        self.move_to(text.len(), true);
                    }
                    _ => {
                        self.move_to(at, cx.modifiers().shift);
                        self.dragging = true;
                    }
                }
                true
            }
            Event::PointerUp { kind, inside } => {
                self.hovered = *inside && kind.hovers();
                self.dragging = false;
                true
            }
            Event::PointerCancel => {
                self.hovered = false;
                self.dragging = false;
                true
            }
            Event::FocusIn(source) => {
                self.set_focus(Some(*source));
                if *source == FocusSource::Keyboard {
                    self.move_to(text.len(), false);
                }
                true
            }
            Event::FocusOut => {
                self.set_focus(None);
                if self.password && !text.is_empty() && self.can_peek.get_untracked() {
                    self.can_peek.set(false);
                }
                self.preedit = None;
                self.dragging = false;
                self.typing = false;
                true
            }
            _ if !self.is_focused() => false,
            Event::KeyDown(key) => self.key(text, *key, cx),
            Event::KeyUp(_)
            | Event::Wheel { .. }
            | Event::Scroll { .. }
            | Event::SecondaryDown
            | Event::SecondaryUp
            | Event::ContextOpening
            | Event::MiddleClick
            | Event::MiddleDown
            | Event::MiddleUp
            | Event::DragOver(_)
            | Event::DragLeave
            | Event::Drop(_)
            | Event::DropOutside { .. }
            | Event::FilesDropped { .. } => false,
            Event::Text(typed) => {
                if self.read_only {
                    return false;
                }
                let multiline = self.multiline;
                let typed: String = typed
                    .chars()
                    .filter(|&c| !c.is_control() || (multiline && c == '\n'))
                    .collect();
                if typed.is_empty() {
                    return false;
                }
                self.preedit = None;
                self.replace(text, &typed, true);
                true
            }
            Event::Ime(Ime::Preedit {
                text: preedit,
                cursor,
            }) => {
                if self.password || self.read_only {
                    return false;
                }
                if self.cursor != self.anchor {
                    self.replace(text, "", true);
                }
                self.preedit = (!preedit.is_empty()).then(|| (preedit.clone(), *cursor));
                true
            }
            Event::Ime(Ime::Commit(commit)) => {
                self.preedit = None;
                if self.read_only {
                    return false;
                }
                self.replace(text, commit, true);
                true
            }
        }
    }

    fn key(&mut self, text: &str, key: Key, cx: &mut EventCx) -> bool {
        let mods = cx.modifiers();
        let shift = mods.shift;
        let word = word_modifier(mods);
        let (start, end) = (self.cursor.min(self.anchor), self.cursor.max(self.anchor));
        let selected = start != end;
        if self.preedit.is_some() {
            // Com a composição aberta, as teclas são do IME.
            return false;
        }
        match key {
            Key::Left | Key::Right => {
                let left = key == Key::Left;
                // Cmd+setas no macOS vão às pontas.
                let edge = cfg!(any(target_os = "macos", target_os = "ios")) && mods.meta;
                let at = if edge {
                    if left { 0 } else { text.len() }
                } else if selected && !shift && !word {
                    if left { start } else { end }
                } else if word {
                    if left {
                        self.word_before(text, self.cursor)
                    } else {
                        self.word_after(text, self.cursor)
                    }
                } else {
                    let stops = self.stops(text, cx);
                    if left {
                        stops
                            .iter()
                            .rev()
                            .find(|&&i| i < self.cursor)
                            .copied()
                            .unwrap_or(0)
                    } else {
                        stops
                            .iter()
                            .find(|&&i| i > self.cursor)
                            .copied()
                            .unwrap_or(text.len())
                    }
                };
                self.move_to(at, shift);
                true
            }
            Key::Up | Key::Down if self.multiline => {
                let points = cx.metrics().caret_points(text);
                let line = line_of(&points, self.cursor);
                let x = x_of(&points, self.cursor);
                let last = points.last().map_or(0, |p| p.2);
                let at = match (key == Key::Up, line) {
                    (true, 0) => 0,
                    (true, _) => nearest_in_line(&points, line - 1, x).unwrap_or(0),
                    (false, l) if l >= last => text.len(),
                    (false, _) => nearest_in_line(&points, line + 1, x).unwrap_or(text.len()),
                };
                self.move_to(at, shift);
                true
            }
            Key::Home | Key::End if self.multiline && !(mods.ctrl || mods.meta) => {
                let points = cx.metrics().caret_points(text);
                let line = line_of(&points, self.cursor);
                let mut bytes = points.iter().filter(|p| p.2 == line).map(|p| p.0);
                let at = if key == Key::Home {
                    bytes.next()
                } else {
                    bytes.next_back()
                };
                self.move_to(at.unwrap_or(0), shift);
                true
            }
            Key::Home => {
                self.move_to(0, shift);
                true
            }
            Key::End => {
                self.move_to(text.len(), shift);
                true
            }
            Key::Enter if self.multiline && !self.read_only => {
                self.replace(text, "\n", false);
                true
            }
            Key::Backspace | Key::Delete => {
                if !selected {
                    let back = key == Key::Backspace;
                    let at = match (back, word) {
                        (true, true) => self.word_before(text, self.cursor),
                        (false, true) => self.word_after(text, self.cursor),
                        (true, false) => {
                            let stops = self.stops(text, cx);
                            stops
                                .iter()
                                .rev()
                                .find(|&&i| i < self.cursor)
                                .copied()
                                .unwrap_or(0)
                        }
                        (false, false) => {
                            let stops = self.stops(text, cx);
                            stops
                                .iter()
                                .find(|&&i| i > self.cursor)
                                .copied()
                                .unwrap_or(text.len())
                        }
                    };
                    self.anchor = at;
                }
                self.replace(text, "", false);
                true
            }
            Key::Char(c) if mods.command() => match c.to_ascii_lowercase() {
                'a' => {
                    self.anchor = 0;
                    self.move_to(text.len(), true);
                    true
                }
                'c' | 'x' if self.password => false,
                'c' => {
                    if selected {
                        cx.clipboard().write(text[start..end].to_owned());
                    }
                    true
                }
                'x' => {
                    if selected {
                        cx.clipboard().write(text[start..end].to_owned());
                        self.replace(text, "", false);
                    }
                    true
                }
                'v' if self.read_only => false,
                'v' => {
                    if let Some(pasted) = cx.clipboard().read() {
                        let pasted = if self.multiline {
                            multi_line(&pasted)
                        } else {
                            single_line(&pasted)
                        };
                        self.replace(text, &pasted, false);
                    }
                    true
                }
                'z' if shift => self.redo(text),
                'z' => self.undo(text),
                'y' => self.redo(text),
                _ => false,
            },
            _ => false,
        }
    }
}

/// Começo da palavra antes de `at`.
pub(crate) fn word_before(text: &str, at: usize) -> usize {
    let mut chars = text[..at].char_indices().rev().peekable();
    while chars.next_if(|&(_, c)| class(c) == Class::Space).is_some() {}
    let Some(&(mut start, c)) = chars.peek() else {
        return 0;
    };
    let k = class(c);
    for (i, c) in chars {
        if class(c) != k {
            break;
        }
        start = i;
    }
    start
}

/// Começo da próxima palavra depois de `at`, como no Windows.
pub(crate) fn word_after(text: &str, at: usize) -> usize {
    let mut chars = text[at..]
        .char_indices()
        .map(|(i, c)| (at + i, c))
        .peekable();
    if let Some(&(_, c)) = chars.peek() {
        let k = class(c);
        if k != Class::Space {
            while chars.next_if(|&(_, c)| class(c) == k).is_some() {}
        }
    }
    while chars.next_if(|&(_, c)| class(c) == Class::Space).is_some() {}
    chars.peek().map_or(text.len(), |&(i, _)| i)
}

/// Palavra (ou sequência de espaços, ou de pontuação) em volta de `at`.
pub(crate) fn word_at(text: &str, at: usize) -> (usize, usize) {
    let Some(c) = text[at..]
        .chars()
        .next()
        .or_else(|| text[..at].chars().next_back())
    else {
        return (at, at);
    };
    let k = class(c);
    let start = text[..at]
        .char_indices()
        .rev()
        .take_while(|&(_, c)| class(c) == k)
        .last()
        .map_or(at, |(i, _)| i);
    let end = text[at..]
        .char_indices()
        .find(|&(_, c)| class(c) != k)
        .map_or(text.len(), |(i, _)| at + i);
    (start, end)
}

/// O modificador de andar por palavra: Option no macOS, Ctrl no resto.
pub(crate) fn word_modifier(mods: Modifiers) -> bool {
    if cfg!(any(target_os = "macos", target_os = "ios")) {
        mods.alt
    } else {
        mods.ctrl
    }
}

/// O ponto do cursor em `byte`, ou o último antes dele.
fn point_at(points: &[(usize, f64, usize)], byte: usize) -> Option<&(usize, f64, usize)> {
    points
        .iter()
        .find(|p| p.0 == byte)
        .or_else(|| points.iter().rev().find(|p| p.0 <= byte))
}

/// A linha de `byte` num texto de várias linhas.
fn line_of(points: &[(usize, f64, usize)], byte: usize) -> usize {
    point_at(points, byte).map_or(0, |p| p.2)
}

/// O x de `byte` na linha dele.
fn x_of(points: &[(usize, f64, usize)], byte: usize) -> f64 {
    point_at(points, byte).map_or(0.0, |p| p.1)
}

/// O byte da `line` mais perto de `x`.
fn nearest_in_line(points: &[(usize, f64, usize)], line: usize, x: f64) -> Option<usize> {
    points
        .iter()
        .filter(|p| p.2 == line)
        .min_by(|a, b| (a.1 - x).abs().total_cmp(&(b.1 - x).abs()))
        .map(|p| p.0)
}

/// Texto colado em várias linhas: as quebras ficam (`\r\n` e `\r` viram `\n`), a tabulação
/// vira espaço e os outros caracteres de controle saem.
pub(crate) fn multi_line(text: &str) -> String {
    text.replace("\r\n", "\n")
        .chars()
        .filter_map(|c| match c {
            '\n' | '\r' => Some('\n'),
            '\t' => Some(' '),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect()
}

/// Texto colado numa linha só: cada quebra de linha vira um espaço e os outros caracteres de
/// controle saem.
fn single_line(text: &str) -> String {
    text.replace("\r\n", "\n")
        .chars()
        .filter_map(|c| match c {
            '\n' | '\r' | '\t' => Some(' '),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect()
}

common_builders!(Edit);

impl Widget for Edit {
    fn kind(&self) -> &'static str {
        "edit"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::Edit(self.visual())
    }

    fn set_density(&mut self, density: Density) {
        self.density = density;
    }

    fn flex(&self) -> Option<Flex> {
        self.has_parts().then_some(Flex {
            direction: Direction::Row,
            justify: Justify::End,
            align: Align::Stretch,
            ..Flex::default()
        })
    }

    fn measured(&self) -> bool {
        true
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        let mut makers = std::mem::take(&mut self.makers);
        if self.password {
            makers.insert(0, Box::new(Self::reveal_part));
        }
        makers
            .into_iter()
            .map(|make| {
                let part = make(self);
                self.parts.push((part.width, part.shown));
                part.widget
            })
            .collect()
    }

    fn names_children(&self) -> bool {
        self.has_parts()
    }

    fn handle(&mut self, event: &Event, cx: &mut EventCx) -> bool {
        Edit::handle(self, event, cx)
    }

    fn interactive(&self) -> bool {
        true
    }

    fn wants_scroll(&self) -> bool {
        self.multiline
    }

    fn focusable(&self) -> bool {
        !self.is_disabled()
    }

    fn enabled(&self) -> bool {
        !self.is_disabled()
    }

    fn text(&self) -> Option<String> {
        Some(self.value())
    }

    fn text_input(&self) -> Option<InputPurpose> {
        Some(
            if self.password && self.reveal.get() != PasswordReveal::Visible {
                InputPurpose::Password
            } else {
                InputPurpose::Normal
            },
        )
    }
}

impl fmt::Debug for Edit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Edit")
            .field("common", &self.common)
            .field(
                "value",
                &self.value.with_untracked(|v| {
                    if self.password {
                        format!("<{} caracteres>", v.chars().count())
                    } else {
                        format!("{v:?}")
                    }
                }),
            )
            .field("password", &self.password)
            .field("cursor", &self.cursor)
            .field("anchor", &self.anchor)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use kurbo::{Point, Rect};
    use tomni_ui_reactive::prelude::*;

    use super::*;
    use crate::cx::{Clipboard, FixedMetrics, MemoryClipboard};
    use crate::event::PointerKind::Mouse;

    /// Andar por palavra: Option no macOS, Ctrl no resto.
    const WORD: Modifiers = if cfg!(any(target_os = "macos", target_os = "ios")) {
        Modifiers {
            alt: true,
            ..Modifiers::NONE
        }
    } else {
        Modifiers {
            ctrl: true,
            ..Modifiers::NONE
        }
    };

    /// Um `Edit` de 0..100 × 0..32 com 10 px por caractere, sem padding.
    struct Harness {
        edit: Edit,
        value: Signal<String>,
        metrics: FixedMetrics,
        clipboard: MemoryClipboard,
        rect: Rect,
        pointer: Point,
        mods: Modifiers,
    }

    impl Harness {
        fn new(edit: impl FnOnce(Signal<String>) -> Edit) -> Self {
            let value = signal(String::new());
            let mut h = Self {
                edit: edit(value),
                value,
                metrics: FixedMetrics::default(),
                clipboard: MemoryClipboard::default(),
                rect: Rect::new(0.0, 0.0, 100.0, 32.0),
                pointer: Point::ZERO,
                mods: Modifiers::NONE,
            };
            h.send(Event::FocusIn(FocusSource::Keyboard));
            h
        }

        fn send(&mut self, event: Event) -> bool {
            let mut cx = EventCx::new(
                self.rect,
                self.pointer,
                self.mods,
                &mut self.metrics,
                &mut self.clipboard,
            );
            self.edit.handle(&event, &mut cx)
        }

        fn typed(&mut self, text: &str) {
            for c in text.chars() {
                self.send(Event::Text(c.into()));
            }
        }

        fn key(&mut self, key: Key) -> bool {
            self.key_with(key, Modifiers::NONE)
        }

        fn key_with(&mut self, key: Key, mods: Modifiers) -> bool {
            self.mods = mods;
            let consumed = self.send(Event::KeyDown(key));
            self.mods = Modifiers::NONE;
            consumed
        }

        fn shortcut(&mut self, c: char) -> bool {
            self.key_with(Key::Char(c), Modifiers::COMMAND)
        }

        fn click(&mut self, x: f64, clicks: u8) {
            self.pointer = Point::new(x, 16.0);
            self.send(Event::PointerDown {
                kind: Mouse,
                clicks,
            });
            self.send(Event::PointerUp {
                kind: Mouse,
                inside: true,
            });
        }

        fn text(&self) -> String {
            self.value.get_untracked()
        }

        fn visual(&self) -> EditVisual {
            self.edit.visual()
        }
    }

    fn with(test: impl FnOnce()) {
        let scope = Scope::root();
        scope.run(test);
        scope.clear();
    }

    #[test]
    fn digita_no_signal_e_ignora_controle() {
        with(|| {
            let mut h = Harness::new(Edit::bind);
            h.typed("ana");
            assert_eq!(h.text(), "ana");
            assert!(!h.send(Event::Text("\r".into())), "Enter não é texto");
            h.send(Event::Text("@x\t".into()));
            assert_eq!(h.text(), "ana@x");
            assert_eq!(h.edit.selection(), (5, 5));
            assert_eq!(h.visual().caret, Some(5));
            assert_eq!(h.visual().state, EditState::Focused);
        });
    }

    #[test]
    fn signal_mudado_por_fora() {
        with(|| {
            let mut h = Harness::new(Edit::bind);
            h.typed("abcdef");
            h.value.set("ab".into());
            assert_eq!(h.visual().caret, Some(2), "cursor fora do texto vai ao fim");
            h.typed("c");
            assert_eq!(h.text(), "abc");
            h.send(Event::KeyDown(Key::Home));
            h.value.set("xyz".into());
            assert_eq!(h.visual().caret, Some(3), "texto novo: cursor no fim");
            h.typed("!");
            assert_eq!(h.text(), "xyz!");
        });
    }

    #[test]
    fn setas_shift_home_end() {
        with(|| {
            let mut h = Harness::new(Edit::bind);
            h.typed("abcd");
            h.key(Key::Left);
            h.key(Key::Left);
            assert_eq!(h.edit.selection(), (2, 2));
            h.key_with(Key::Left, Modifiers::SHIFT);
            assert_eq!(h.visual().selection, Some((1, 2)));
            h.key_with(Key::End, Modifiers::SHIFT);
            assert_eq!(h.visual().selection, Some((2, 4)), "a âncora fica no lugar");
            // Sem Shift, a seta recolhe a seleção para o lado dela.
            h.key(Key::Left);
            assert_eq!(h.edit.selection(), (2, 2));
            h.key(Key::Home);
            assert_eq!(h.edit.selection(), (0, 0));
            h.key(Key::Left);
            assert_eq!(h.edit.selection(), (0, 0));
            assert!(!h.key(Key::Up), "uma linha só");
            assert!(!h.key(Key::Enter));
        });
    }

    #[test]
    fn palavras() {
        with(|| {
            let mut h = Harness::new(Edit::bind);
            h.typed("ola, mundo novo");
            h.key_with(Key::Left, WORD);
            assert_eq!(h.edit.selection(), (11, 11));
            h.key_with(Key::Left, WORD);
            assert_eq!(h.edit.selection(), (5, 5));
            h.key_with(Key::Left, WORD);
            assert_eq!(h.edit.selection(), (3, 3), "a vírgula é uma palavra");
            h.key_with(Key::Right, WORD);
            assert_eq!(h.edit.selection(), (5, 5));
            h.key_with(Key::End, Modifiers::NONE);
            h.key_with(Key::Backspace, WORD);
            assert_eq!(h.text(), "ola, mundo ");
            h.key(Key::Home);
            h.key_with(Key::Delete, WORD);
            assert_eq!(h.text(), ", mundo ");
        });
    }

    #[test]
    fn backspace_e_delete() {
        with(|| {
            let mut h = Harness::new(Edit::bind);
            h.typed("ação");
            h.key(Key::Backspace);
            assert_eq!(h.text(), "açã");
            h.key(Key::Home);
            h.key(Key::Delete);
            assert_eq!(h.text(), "çã");
            h.key_with(Key::End, Modifiers::SHIFT);
            h.key(Key::Delete);
            assert_eq!(h.text(), "");
            assert!(h.key(Key::Backspace), "apagar o vazio não faz nada");
        });
    }

    #[test]
    fn clipboard() {
        with(|| {
            let mut h = Harness::new(Edit::bind);
            h.typed("ana@x.com");
            h.shortcut('a');
            assert_eq!(h.visual().selection, Some((0, 9)));
            h.shortcut('c');
            assert_eq!(h.clipboard.read().as_deref(), Some("ana@x.com"));
            h.key(Key::Home);
            h.key_with(Key::Right, Modifiers::SHIFT);
            h.key_with(Key::Right, Modifiers::SHIFT);
            h.key_with(Key::Right, Modifiers::SHIFT);
            h.shortcut('x');
            assert_eq!(h.text(), "@x.com");
            assert_eq!(h.clipboard.read().as_deref(), Some("ana"));
            h.key(Key::End);
            h.clipboard.write("linha 1\r\nlinha 2\n".into());
            h.shortcut('v');
            assert_eq!(h.text(), "@x.comlinha 1 linha 2 ");
        });
    }

    #[test]
    fn undo_agrupa_a_digitacao() {
        with(|| {
            let mut h = Harness::new(Edit::bind);
            h.typed("ana");
            h.key(Key::Backspace);
            h.typed("x");
            assert_eq!(h.text(), "anx");
            h.shortcut('z');
            assert_eq!(h.text(), "an");
            h.shortcut('z');
            assert_eq!(h.text(), "ana");
            h.shortcut('z');
            assert_eq!(h.text(), "");
            assert!(!h.shortcut('z'), "nada mais a desfazer");
            h.shortcut('y');
            assert_eq!(h.text(), "ana");
            h.key_with(Key::Char('z'), Modifiers::COMMAND | Modifiers::SHIFT);
            assert_eq!(h.text(), "an");
            assert_eq!(h.edit.selection(), (2, 2), "o cursor volta junto");
            // Uma mudança nova descarta o que havia para refazer.
            h.typed("!");
            assert!(!h.shortcut('y'));
        });
    }

    #[test]
    fn undo_tem_limite() {
        with(|| {
            let mut h = Harness::new(Edit::bind);
            for _ in 0..UNDO_LIMIT + 10 {
                h.typed("a");
                h.key(Key::Left);
                h.key(Key::Right);
            }
            let mut passos = 0;
            while h.shortcut('z') {
                passos += 1;
            }
            assert_eq!(passos, UNDO_LIMIT);
            assert_eq!(h.text().len(), 10);
        });
    }

    #[test]
    fn mouse_posiciona_arrasta_e_seleciona_palavra() {
        with(|| {
            let mut h = Harness::new(Edit::bind);
            h.typed("um dois");
            h.click(24.0, 1);
            assert_eq!(h.edit.selection(), (2, 2));

            h.pointer = Point::new(11.0, 16.0);
            h.send(Event::PointerDown {
                kind: Mouse,
                clicks: 1,
            });
            h.pointer = Point::new(58.0, 16.0);
            h.send(Event::PointerMove {
                kind: Mouse,
                inside: true,
            });
            h.send(Event::PointerUp {
                kind: Mouse,
                inside: true,
            });
            assert_eq!(h.visual().selection, Some((1, 6)));
            h.pointer = Point::new(90.0, 16.0);
            h.send(Event::PointerMove {
                kind: Mouse,
                inside: true,
            });
            assert_eq!(
                h.visual().selection,
                Some((1, 6)),
                "soltou: não arrasta mais"
            );

            h.click(45.0, 2);
            assert_eq!(h.visual().selection, Some((3, 7)));
            h.click(25.0, 2);
            assert_eq!(h.visual().selection, Some((2, 3)), "o espaço é uma palavra");
            h.click(5.0, 3);
            assert_eq!(h.visual().selection, Some((0, 7)));

            // Shift+clique estende a seleção.
            h.click(10.0, 1);
            h.mods = Modifiers::SHIFT;
            h.click(50.0, 1);
            assert_eq!(h.visual().selection, Some((1, 5)));
        });
    }

    #[test]
    fn rola_para_mostrar_o_cursor() {
        with(|| {
            let mut h = Harness::new(Edit::bind);
            h.rect = Rect::new(0.0, 0.0, 50.0, 32.0);
            h.typed("0123456789");
            assert_eq!(h.visual().scroll, 51.0, "o cursor de 1 px cabe no fim");
            h.key(Key::Home);
            assert_eq!(h.visual().scroll, 0.0);
            h.key(Key::End);
            assert_eq!(h.visual().scroll, 51.0);
            // O clique leva a rolagem em conta.
            h.click(4.0, 1);
            assert_eq!(h.edit.selection(), (5, 5));
            // Apagar tudo volta ao começo.
            h.shortcut('a');
            h.key(Key::Delete);
            assert_eq!(h.visual().scroll, 0.0);
        });
    }

    #[test]
    fn senha() {
        with(|| {
            let mut h = Harness::new(|s| Edit::bind(s).password());
            h.typed("sé1");
            assert_eq!(h.text(), "sé1");
            let visual = h.visual();
            assert_eq!(visual.text, "•••");
            assert_eq!(visual.caret, Some(9));
            h.key(Key::Left);
            assert_eq!(h.edit.selection(), (3, 3), "anda por caractere");
            assert_eq!(h.visual().caret, Some(6));
            h.key_with(Key::Left, WORD);
            assert_eq!(h.edit.selection(), (0, 0), "palavras não aparecem");

            h.shortcut('a');
            assert_eq!(h.visual().selection, Some((0, 9)));
            assert!(!h.shortcut('c'));
            assert!(!h.shortcut('x'));
            assert_eq!(h.clipboard.read(), None);
            assert_eq!(h.text(), "sé1");

            h.click(15.0, 1);
            assert_eq!(h.edit.selection(), (1, 1), "o clique cai entre as bolinhas");
            h.click(15.0, 2);
            assert_eq!(h.visual().selection, Some((0, 9)), "duplo clique pega tudo");

            assert!(
                !h.send(Event::Ime(Ime::Preedit {
                    text: "か".into(),
                    cursor: None,
                })),
                "sem composição na senha"
            );
            assert_eq!(h.edit.text_input(), Some(InputPurpose::Password));
            assert!(!format!("{:?}", h.edit).contains("sé1"));
        });
    }

    #[test]
    fn revelar_senha_enquanto_pressiona() {
        with(|| {
            let mut h = Harness::new(|s| Edit::bind(s).id("senha").password());
            let mut partes = Widget::take_children(&mut h.edit);
            assert_eq!(partes.len(), 1);
            let olho = &mut partes[0];
            assert_eq!(olho.common().id.as_deref(), Some("senha-revelar"));
            assert!(!olho.visible(), "vazio, sem botão");
            h.typed("abc");
            assert!(olho.visible());
            assert_eq!(h.visual().trailing, 32.0, "quadrado, da altura do campo");

            let mut metrics = FixedMetrics::default();
            let mut clipboard = MemoryClipboard::default();
            let mut cx = EventCx::new(
                Rect::new(68.0, 0.0, 100.0, 32.0),
                Point::ZERO,
                Modifiers::NONE,
                &mut metrics,
                &mut clipboard,
            );
            let down = Event::PointerDown {
                kind: Mouse,
                clicks: 1,
            };
            olho.handle(&down, &mut cx);
            assert!(h.edit.is_revealed());
            assert_eq!(h.visual().text, "abc");
            assert!(!h.shortcut('c'), "à mostra, continua sem copiar");
            let up = Event::PointerUp {
                kind: Mouse,
                inside: true,
            };
            olho.handle(&up, &mut cx);
            assert_eq!(h.visual().text, "•••");

            // Perdeu o foco com texto: o botão some até o campo ficar vazio.
            h.send(Event::FocusOut);
            assert!(!olho.visible());
            h.send(Event::FocusIn(FocusSource::Keyboard));
            assert!(!olho.visible());
            h.shortcut('a');
            h.key(Key::Delete);
            h.typed("x");
            assert!(olho.visible());
        });
    }

    #[test]
    fn modos_de_revelar() {
        with(|| {
            let modo = signal(PasswordReveal::Peek);
            let mut h = Harness::new(|s| Edit::bind(s).password().password_reveal(modo));
            let partes = Widget::take_children(&mut h.edit);
            h.typed("abc");
            assert!(partes[0].visible());
            modo.set(PasswordReveal::Hidden);
            assert!(!partes[0].visible());
            assert_eq!(h.visual().text, "•••");
            modo.set(PasswordReveal::Visible);
            assert!(!partes[0].visible());
            assert_eq!(h.visual().text, "abc");
            assert_eq!(h.edit.text_input(), Some(InputPurpose::Normal));
        });
    }

    #[test]
    fn campo_comum_continua_folha() {
        with(|| {
            let mut edit = Edit::bind(signal(String::new()));
            assert!(edit.flex().is_none());
            assert!(Widget::take_children(&mut edit).is_empty());
            let senha = Edit::bind(signal(String::new())).password();
            assert!(senha.flex().is_some());
        });
    }

    #[test]
    fn composicao_do_ime() {
        with(|| {
            let mut h = Harness::new(Edit::bind);
            h.typed("ab");
            h.key(Key::Left);
            h.send(Event::Ime(Ime::Preedit {
                text: "´".into(),
                cursor: None,
            }));
            let visual = h.visual();
            assert_eq!(visual.text, "a´b");
            assert_eq!(visual.composing, Some((1, 3)));
            assert_eq!(visual.caret, Some(3));
            assert_eq!(h.text(), "ab", "a composição não vai ao signal");
            assert!(!h.key(Key::Left), "as teclas são do IME");

            h.send(Event::Ime(Ime::Commit("é".into())));
            assert_eq!(h.text(), "aéb");
            assert_eq!(h.visual().composing, None);
            assert_eq!(h.edit.selection(), (3, 3));

            // A composição substitui a seleção e some quando o foco sai.
            h.shortcut('a');
            h.send(Event::Ime(Ime::Preedit {
                text: "にほ".into(),
                cursor: Some((3, 6)),
            }));
            assert_eq!(h.text(), "");
            assert_eq!(h.visual().caret, Some(6));
            h.send(Event::FocusOut);
            h.send(Event::FocusIn(FocusSource::Pointer));
            assert_eq!(h.visual().text, "");
            h.shortcut('z');
            assert_eq!(h.text(), "aéb");
        });
    }

    #[test]
    fn hover_e_foco() {
        with(|| {
            let mut h = Harness::new(Edit::bind);
            h.send(Event::FocusOut);
            assert_eq!(h.visual().state, EditState::Rest);
            assert_eq!(h.visual().caret, None);
            h.send(Event::PointerMove {
                kind: Mouse,
                inside: true,
            });
            assert_eq!(h.visual().state, EditState::Hover);
            h.send(Event::FocusIn(FocusSource::Pointer));
            assert_eq!(h.visual().state, EditState::Focused, "o foco vence o hover");
            assert!(!h.send(Event::KeyUp(Key::Space)));
        });
    }

    #[test]
    fn sem_foco_ignora_teclado() {
        with(|| {
            let mut h = Harness::new(Edit::bind);
            h.send(Event::FocusOut);
            assert!(!h.send(Event::Text("a".into())));
            assert!(!h.key(Key::Backspace));
            assert_eq!(h.text(), "");
        });
    }

    #[test]
    fn desabilitado() {
        with(|| {
            let off = signal(false);
            let mut h = Harness::new(|s| Edit::bind(s).placeholder("E-mail").disabled(off));
            h.typed("ana");
            off.set(true);
            assert!(!h.edit.focusable());
            assert!(!h.send(Event::Text("x".into())));
            assert_eq!(h.text(), "ana");
            assert_eq!(
                h.visual(),
                EditVisual {
                    text: "ana".into(),
                    placeholder: "E-mail".into(),
                    state: EditState::Disabled,
                    ..EditVisual::default()
                }
            );
            off.set(false);
            assert_eq!(h.visual().state, EditState::Rest, "perdeu o foco");
        });
    }
}
