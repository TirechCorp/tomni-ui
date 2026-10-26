//! `TextEditor`: editor de texto de várias linhas ligado a um signal, com quebra de linha,
//! rolagem, desfazer, realce por faixas, marcas de busca e um controlador para o app.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;

use kurbo::{Point, Rect, Vec2};
use tomni_ui_reactive::{Get, GetUntracked, Set, Signal, With, WithUntracked, signal, untrack};

use crate::clipboard;
use crate::color::ThemeColor;
use crate::cx::{Clipboard, ClipboardImage, EventCx, InputPurpose};
use crate::density::Density;
use crate::edit::{EditState, multi_line, word_after, word_at, word_before, word_modifier};
use crate::event::{Event, FocusSource, Ime, Key, Modifiers};
use crate::flyout::MenuFlyout;
use crate::prop::{IntoProp, Prop};
use crate::scroll_viewer::{
    Drag, ScrollAxis, ScrollBarParts, ScrollBarState, ScrollMode, Scroller, drag_bar, press_bar,
};
use crate::tree_view::fold_char;
use crate::widget::{Common, Look, Widget, common_builders};
use crate::windows::WindowHandle;

/// Quantos passos o desfazer guarda.
const UNDO_LIMIT: usize = 100;

/// Altura padrão do editor sem `.min_height`.
const DEFAULT_MIN_HEIGHT: f64 = 96.0;

/// Um trecho do texto com estilo próprio, devolvido pelo `highlight` do [`TextEditor`].
///
/// ```
/// use tomni_ui_core::StyleRange;
///
/// let titulo = StyleRange::new(0..6).bold();
/// assert!(titulo.bold && !titulo.italic);
/// ```
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StyleRange {
    /// Bytes do texto. Limites fora de um `char` e partes além do fim são ignorados.
    pub range: Range<usize>,
    /// Negrito.
    pub bold: bool,
    /// Itálico (inclinado por síntese, como no [`RichText`](crate::RichText)).
    pub italic: bool,
    /// Riscado.
    pub strike: bool,
    /// Cor do texto, no lugar da cor primária.
    pub color: Option<ThemeColor>,
    /// Cor de fundo atrás do trecho.
    pub background: Option<ThemeColor>,
}

impl StyleRange {
    /// Faixa sem estilo sobre `range`.
    pub fn new(range: Range<usize>) -> Self {
        Self {
            range,
            ..Self::default()
        }
    }

    /// Negrito.
    pub fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    /// Itálico.
    pub fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    /// Riscado.
    pub fn strike(mut self) -> Self {
        self.strike = true;
        self
    }

    /// Cor do texto.
    pub fn color(mut self, color: impl Into<ThemeColor>) -> Self {
        self.color = Some(color.into());
        self
    }

    /// Cor de fundo.
    pub fn background(mut self, color: impl Into<ThemeColor>) -> Self {
        self.background = Some(color.into());
        self
    }
}

/// Uma marca de busca: um trecho com fundo de destaque, mais forte no resultado atual.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mark {
    /// Bytes do texto.
    pub range: Range<usize>,
    /// Se é o resultado atual da busca.
    pub current: bool,
}

/// Uma tecla com os modificadores, como o `on_key` do [`TextEditor`] a recebe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    /// A tecla.
    pub key: Key,
    /// Os modificadores no momento do toque.
    pub modifiers: Modifiers,
}

/// Tudo o que o `Drawer` precisa para medir e pintar o [`TextEditor`]. Posições em bytes de
/// `text`, que já traz a composição do IME no cursor.
#[derive(Clone, Debug, PartialEq)]
pub struct TextEditorVisual {
    /// Texto exibido.
    pub text: Rc<str>,
    /// Mostrado quando `text` está vazio.
    pub placeholder: String,
    /// Cursor, só com foco e fora do modo só leitura.
    pub caret: Option<usize>,
    /// O cursor está no fim da linha visual de cima, no ponto onde uma quebra suave divide a
    /// linha (os dois lados têm o mesmo byte).
    pub caret_up: bool,
    /// Trecho selecionado, só com foco e nunca vazio.
    pub selection: Option<Range<usize>>,
    /// Trecho em composição no IME, sublinhado.
    pub composing: Option<Range<usize>>,
    /// Faixas de estilo, já limpas (dentro do texto, em limites de `char`, sem vazias).
    pub ranges: Rc<Vec<StyleRange>>,
    /// Marcas de busca, limpas como as faixas.
    pub marks: Rc<Vec<Mark>>,
    /// Quebra as linhas na largura do campo (senão rola na horizontal).
    pub wrap: bool,
    /// Fonte monoespaçada.
    pub mono: bool,
    /// Estado.
    pub state: EditState,
    /// Densidade.
    pub density: Density,
    /// Rolagem do texto.
    pub scroller: Scroller,
    /// Estado das barras de rolagem.
    pub bar: ScrollBarState,
    /// Altura mínima do campo.
    pub min_height: f64,
    /// Altura máxima: com ela o campo cresce com o texto até esse limite.
    pub max_height: Option<f64>,
    /// Só leitura.
    pub read_only: bool,
}

impl TextEditorVisual {
    /// Retângulo das barras de rolagem num editor que ocupa `rect`: por dentro da borda.
    pub fn bars_rect(rect: Rect) -> Rect {
        Rect::new(rect.x0 + 1.0, rect.y0 + 1.0, rect.x1 - 1.0, rect.y1 - 1.0)
    }
}

/// O texto a medir: o que o `Drawer` precisa para quebrar as linhas e achar os pontos do cursor.
#[derive(Clone, Copy, Debug)]
pub struct EditorText<'a> {
    /// Texto exibido.
    pub text: &'a str,
    /// Faixas de estilo (negrito muda a largura).
    pub ranges: &'a [StyleRange],
    /// Largura onde quebrar as linhas; `None` não quebra.
    pub wrap: Option<f64>,
    /// Fonte monoespaçada.
    pub mono: bool,
}

/// Os pontos onde o cursor pode parar num [`EditorText`], por linha visual.
///
/// `points` é `(byte, x, linha)` em ordem crescente de byte e, no mesmo byte, de linha: onde
/// uma quebra suave divide a linha há dois pontos com o mesmo byte, o fim da linha de cima e o
/// começo da de baixo. Um texto que termina em `\n` ganha o ponto final da linha vazia.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EditorPoints {
    /// `(byte, x, linha)`.
    pub points: Vec<(usize, f64, usize)>,
    /// Largura da linha mais comprida.
    pub width: f64,
    /// Quantas linhas visuais o texto tem (ao menos uma).
    pub lines: usize,
}

impl EditorPoints {
    /// O `x` e a linha onde o cursor em `byte` aparece. Num byte de quebra suave, `up` escolhe o
    /// fim da linha de cima; sem ponto exato, vale o último antes dele.
    pub fn locate(&self, byte: usize, up: bool) -> (f64, usize) {
        let first = self.points.partition_point(|p| p.0 < byte);
        let end = first + self.points[first..].partition_point(|p| p.0 == byte);
        let point = if first < end {
            Some(if up {
                self.points[first]
            } else {
                self.points[end - 1]
            })
        } else if first > 0 {
            Some(self.points[first - 1])
        } else {
            None
        };
        point.map_or((0.0, 0), |p| (p.1, p.2))
    }

    /// Os pontos da `line`.
    pub fn line_points(&self, line: usize) -> &[(usize, f64, usize)] {
        let lo = self.points.partition_point(|p| p.2 < line);
        let hi = self.points.partition_point(|p| p.2 <= line);
        &self.points[lo..hi]
    }

    /// O byte da `line` mais perto de `x` e se é o fim da linha de cima de uma quebra suave.
    pub fn nearest(&self, line: usize, x: f64) -> (usize, bool) {
        let points = self.line_points(line);
        let Some(best) = points
            .iter()
            .min_by(|a, b| (a.1 - x).abs().total_cmp(&(b.1 - x).abs()))
        else {
            return (self.points.last().map_or(0, |p| p.0), false);
        };
        let up = self.locate(best.0, false).1 != line;
        (best.0, up)
    }

    /// O maior ponto de parada antes de `byte`.
    pub fn prev_stop(&self, byte: usize) -> Option<usize> {
        let i = self.points.partition_point(|p| p.0 < byte);
        i.checked_sub(1).map(|i| self.points[i].0)
    }

    /// O menor ponto de parada depois de `byte`.
    pub fn next_stop(&self, byte: usize) -> Option<usize> {
        let i = self.points.partition_point(|p| p.0 <= byte);
        self.points.get(i).map(|p| p.0)
    }

    /// A linha do último ponto: onde o cursor fica no fim do texto.
    pub fn last_line(&self) -> usize {
        self.lines.saturating_sub(1)
    }
}

/// Um passo do desfazer: o trecho `removed` que estava em `start` virou `added`.
#[derive(Clone, Debug)]
struct Step {
    start: usize,
    removed: String,
    added: String,
    /// Cursor e âncora antes e depois.
    before: (usize, usize),
    after: (usize, usize),
}

type KeyHandler = Rc<dyn Fn(&KeyEvent, &TextEditorController) -> bool>;
type FilesHandler = Rc<dyn Fn(Vec<PathBuf>, &TextEditorController)>;
type ImageHandler = Rc<dyn Fn(ClipboardImage, &TextEditorController)>;

#[derive(Clone, Default)]
struct Handlers {
    on_key: Option<KeyHandler>,
    on_copy: Option<Rc<dyn Fn(String)>>,
    on_change: Option<Rc<dyn Fn()>>,
    on_paste_files: Option<FilesHandler>,
    on_paste_image: Option<ImageHandler>,
    on_files_dropped: Option<FilesHandler>,
}

/// Todo o estado do editor que o controlador alcança. O widget só guarda o visual e o mouse.
struct Shared {
    value: Option<Signal<String>>,
    /// Muda quando o app mexe no editor por fora de um evento, para o visual ser relido.
    version: Option<Signal<u64>>,
    id: Option<String>,
    read_only: Prop<bool>,
    disabled: Prop<bool>,
    cursor: usize,
    anchor: usize,
    /// O texto como o editor o deixou da última vez: se o signal mudou por fora, o cursor vai
    /// para o fim e o desfazer zera.
    known: Rc<str>,
    undo: Vec<Step>,
    redo: Vec<Step>,
    /// A última mudança foi digitação: a próxima entra no mesmo passo.
    typing: bool,
    /// Coluna que ↑ e ↓ procuram.
    goal_x: Option<f64>,
    caret_up: bool,
    pending_scroll: Option<usize>,
    /// Mostrar o cursor no próximo quadro.
    reveal: bool,
    /// Pedido de foco antes de o editor ter id.
    want_focus: bool,
    handlers: Handlers,
}

impl Shared {
    fn new() -> Self {
        Self {
            value: None,
            version: None,
            id: None,
            read_only: Prop::Value(false),
            disabled: Prop::Value(false),
            cursor: 0,
            anchor: 0,
            known: Rc::from(""),
            undo: Vec::new(),
            redo: Vec::new(),
            typing: false,
            goal_x: None,
            caret_up: false,
            pending_scroll: None,
            reveal: false,
            want_focus: false,
            handlers: Handlers::default(),
        }
    }

    /// O texto de `v` como `Rc`, reaproveitando o último. Se mudou por fora, o cursor vai para
    /// o fim e o desfazer zera.
    fn current(&mut self, v: &str) -> Rc<str> {
        if *self.known != *v {
            self.known = Rc::from(v);
            self.cursor = v.len();
            self.anchor = v.len();
            self.undo.clear();
            self.redo.clear();
            self.typing = false;
            self.caret_up = false;
            self.goal_x = None;
        } else {
            if !v.is_char_boundary(self.cursor) {
                self.cursor = v.len();
            }
            if !v.is_char_boundary(self.anchor) {
                self.anchor = v.len();
            }
        }
        Rc::clone(&self.known)
    }

    fn selection(&self) -> Range<usize> {
        self.cursor.min(self.anchor)..self.cursor.max(self.anchor)
    }

    fn move_to(&mut self, at: usize, up: bool, extend: bool) {
        self.cursor = at;
        if !extend {
            self.anchor = at;
        }
        self.caret_up = up;
        self.typing = false;
        self.goal_x = None;
    }

    /// Troca `range` por `insert`. Devolve o texto novo (`None` se nada muda).
    fn replace_range(
        &mut self,
        text: &str,
        range: Range<usize>,
        insert: &str,
        typing: bool,
    ) -> Option<String> {
        if range.is_empty() && insert.is_empty() {
            return None;
        }
        let before = (self.cursor, self.anchor);
        let after = range.start + insert.len();
        let merge = typing
            && self.typing
            && range.is_empty()
            && self
                .undo
                .last()
                .is_some_and(|s| s.start + s.added.len() == range.start);
        if merge {
            if let Some(last) = self.undo.last_mut() {
                last.added.push_str(insert);
                last.after = (after, after);
            }
        } else {
            self.undo.push(Step {
                start: range.start,
                removed: text[range.clone()].to_owned(),
                added: insert.to_owned(),
                before,
                after: (after, after),
            });
            if self.undo.len() > UNDO_LIMIT {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.typing = typing;
        let mut new = String::with_capacity(text.len() + insert.len());
        new.push_str(&text[..range.start]);
        new.push_str(insert);
        new.push_str(&text[range.end..]);
        self.cursor = after;
        self.anchor = after;
        self.caret_up = false;
        self.goal_x = None;
        self.known = Rc::from(new.as_str());
        Some(new)
    }

    /// Desfaz o último passo.
    fn undo_step(&mut self, text: &str) -> Option<String> {
        let step = self.undo.pop()?;
        let end = step.start + step.added.len();
        if text.get(step.start..end) != Some(step.added.as_str()) {
            self.undo.clear();
            self.redo.clear();
            return None;
        }
        let new = format!("{}{}{}", &text[..step.start], step.removed, &text[end..]);
        (self.cursor, self.anchor) = step.before;
        self.redo.push(step);
        self.after_history(&new);
        Some(new)
    }

    /// Refaz o último passo desfeito.
    fn redo_step(&mut self, text: &str) -> Option<String> {
        let step = self.redo.pop()?;
        let end = step.start + step.removed.len();
        if text.get(step.start..end) != Some(step.removed.as_str()) {
            self.undo.clear();
            self.redo.clear();
            return None;
        }
        let new = format!("{}{}{}", &text[..step.start], step.added, &text[end..]);
        (self.cursor, self.anchor) = step.after;
        self.undo.push(step);
        self.after_history(&new);
        Some(new)
    }

    fn after_history(&mut self, new: &str) {
        self.cursor = self.cursor.min(new.len());
        self.anchor = self.anchor.min(new.len());
        self.typing = false;
        self.caret_up = false;
        self.goal_x = None;
        self.known = Rc::from(new);
    }
}

/// O texto sem passar de `max` bytes e em limite de `char`, para fixar uma posição.
fn floor_boundary(text: &str, mut at: usize) -> usize {
    at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// O limite de `char` a partir de `at`, para a frente.
fn ceil_boundary(text: &str, mut at: usize) -> usize {
    at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at += 1;
    }
    at
}

/// Onde `needle` aparece em `text`, sem sobrepor. Com `ignore_case`, sem diferenciar maiúsculas
/// nem acentos (a mesma regra do filtro do `TreeView`).
fn find_in(text: &str, needle: &str, ignore_case: bool) -> Vec<Range<usize>> {
    if needle.is_empty() {
        return Vec::new();
    }
    if !ignore_case {
        return text
            .match_indices(needle)
            .map(|(i, m)| i..i + m.len())
            .collect();
    }
    let combining = |c: char| ('\u{300}'..='\u{36f}').contains(&c);
    let needle: Vec<char> = needle
        .chars()
        .filter(|&c| !combining(c))
        .flat_map(char::to_lowercase)
        .map(fold_char)
        .collect();
    if needle.is_empty() {
        return Vec::new();
    }
    // Cada caractere dobrado com os bytes de onde veio; o acento solto estende o anterior.
    let mut hay: Vec<(char, usize, usize)> = Vec::with_capacity(text.len());
    for (i, c) in text.char_indices() {
        let end = i + c.len_utf8();
        if combining(c) {
            if let Some(last) = hay.last_mut() {
                last.2 = end;
            }
            continue;
        }
        for l in c.to_lowercase() {
            hay.push((fold_char(l), i, end));
        }
    }
    let mut found = Vec::new();
    let mut i = 0;
    while i + needle.len() <= hay.len() {
        let hit = hay[i..i + needle.len()]
            .iter()
            .map(|h| h.0)
            .eq(needle.iter().copied());
        if hit {
            let end = hay[i + needle.len() - 1].2;
            found.push(hay[i].1..end);
            i += needle.len();
            while i < hay.len() && hay[i].1 < end {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    found
}

/// Um pedido de [`request_focus`], [`request_focus_inside`] ou [`start_bring_into_view`]: o id do
/// widget e a janela de onde veio. Com várias janelas, o mesmo id pode existir em mais de uma
/// (a mesma tela montada duas vezes): a janela de origem atende primeiro e, se ela não tem um
/// widget que sirva, o pedido passa a valer para qualquer janela.
struct WindowRequest {
    id: String,
    origin: WindowHandle,
    /// A origem já procurou e não achou: qualquer janela pode atender.
    any: bool,
    /// Quantos quadros da origem (ou, fechada ela, de qualquer janela) ainda procuram.
    tries: u8,
}

/// Quantos quadros um pedido global procura o widget antes de desistir.
const REQUEST_TRIES: u8 = 8;

type Request = RefCell<Option<WindowRequest>>;

fn put_request(cell: &'static std::thread::LocalKey<Request>, id: String) {
    put_request_in(cell, WindowHandle::current(), id);
}

/// Como [`put_request`], com a janela de origem dada.
fn put_request_in(cell: &'static std::thread::LocalKey<Request>, origin: WindowHandle, id: String) {
    let request = WindowRequest {
        id,
        origin,
        any: false,
        tries: REQUEST_TRIES,
    };
    cell.with(|f| *f.borrow_mut() = Some(request));
    crate::wake();
}

/// Entrega o pedido à árvore da janela atual se for a vez dela e `found` disser que ela tem o
/// widget. Senão, conta um quadro (só na origem, ou em qualquer janela se a origem fechou) e
/// deixa o pedido para as outras.
fn claim_request(
    cell: &'static std::thread::LocalKey<Request>,
    found: impl Fn(&str) -> bool,
) -> Option<String> {
    cell.with(|f| {
        let mut slot = f.borrow_mut();
        let request = slot.as_mut()?;
        let here = WindowHandle::current();
        let origin = request.origin == here || !request.origin.is_open();
        if !origin && !request.any {
            return None;
        }
        if found(&request.id) {
            return slot.take().map(|r| r.id);
        }
        if origin {
            request.any = true;
            request.tries = request.tries.saturating_sub(1);
            if request.tries == 0 {
                *slot = None;
            }
        }
        None
    })
}

thread_local! {
    static FOCUS: Request = const { RefCell::new(None) };
    static FOCUS_INSIDE: Request = const { RefCell::new(None) };
    static REVEAL: Request = const { RefCell::new(None) };
    static NEXT_ID: Cell<u32> = const { Cell::new(0) };
}

/// Pede o foco para o widget com `id` (um `Edit`, um botão, qualquer um que aceite foco): a
/// árvore atende no quadro seguinte e, se o widget ainda não existe (acabou de aparecer num
/// `Show`), tenta de novo por alguns quadros. Serve a quem abre uma barra de busca e quer o
/// cursor nela. É o mesmo pedido de [`TextEditorController::focus`], sem precisar de um
/// controlador. Chame de um evento ou de um efeito, não da construção da view.
///
/// Com várias janelas, a de onde veio o pedido ([`WindowHandle::current`]: a do evento) atende
/// primeiro, mesmo que o id exista também em outra (a mesma tela montada nas duas); se ela não
/// tem um widget com esse id que aceite o foco, o pedido passa a valer para as outras.
pub fn request_focus(id: impl Into<String>) {
    put_request(&FOCUS, id.into());
}

/// Como [`request_focus`], com a janela que atende primeiro dada: para um efeito, que roda no
/// quadro de qualquer janela, quando quem o disparou foi a janela `window` (a ativa, por
/// exemplo, de [`active_window`](crate::active_window)).
pub fn request_focus_in(window: WindowHandle, id: impl Into<String>) {
    put_request_in(&FOCUS, window, id.into());
}

/// Como [`request_focus`], mas para a primeira parada de Tab **dentro** do widget com `id`
/// (um painel, um contêiner): o foco vai para o primeiro filho que aceita foco, como se viesse
/// do teclado. Se o widget ainda não existe, a árvore tenta de novo por alguns quadros; se ele
/// não tem nada que aceite foco, o pedido é descartado.
pub fn request_focus_inside(id: impl Into<String>) {
    put_request(&FOCUS_INSIDE, id.into());
}

/// Pede para rolar os `ScrollViewer` em volta do widget com `id` até ele aparecer inteiro,
/// sem mexer no foco, como o `StartBringIntoView` do WinUI. A árvore atende no quadro seguinte,
/// depois do layout, e tenta de novo por alguns quadros se o widget ainda não existe. Serve a
/// quem escolhe uma linha de um `TreeView` pelo código (a linha escolhida fica sempre montada,
/// com o id `{id da árvore}-{chave}`). Chame de um evento ou de um efeito.
pub fn start_bring_into_view(id: impl Into<String>) {
    put_request(&REVEAL, id.into());
}

/// Pedido do [`start_bring_into_view`], para a árvore da janela atual atender se `found`.
pub(crate) fn claim_reveal_request(found: impl Fn(&str) -> bool) -> Option<String> {
    claim_request(&REVEAL, found)
}

/// Pedido de foco do [`request_focus_inside`], para a árvore da janela atual atender se `found`.
pub(crate) fn claim_focus_inside_request(found: impl Fn(&str) -> bool) -> Option<String> {
    claim_request(&FOCUS_INSIDE, found)
}

/// Pedido de foco do [`request_focus`] e do [`TextEditorController::focus`], para a árvore da
/// janela atual atender se `found`.
pub(crate) fn claim_focus_request(found: impl Fn(&str) -> bool) -> Option<String> {
    claim_request(&FOCUS, found)
}

/// O controlador de um [`TextEditor`]: o app o cria, passa ao editor com
/// [`TextEditor::controller`] e o guarda para mexer na seleção, trocar trechos, rolar e
/// buscar. Cópias apontam para o mesmo editor. Sem editor ligado, nada acontece.
///
/// ```
/// use tomni_ui_core::{TextEditor, TextEditorController};
/// use tomni_ui_reactive::prelude::*;
///
/// # let scope = Scope::root();
/// # scope.run(|| {
/// let texto = signal(String::from("olá mundo"));
/// let controle = TextEditorController::new();
/// let editor = TextEditor::bind(texto).id("nota").controller(controle.clone());
/// controle.set_selection(0..4); // bytes: o `á` ocupa dois
/// controle.replace_selection("oi");
/// assert_eq!(texto.get_untracked(), "oi mundo");
/// assert_eq!(controle.find("MUNDO", true), vec![3..8]);
/// # drop(editor);
/// # });
/// # scope.clear();
/// ```
#[derive(Clone)]
pub struct TextEditorController(Rc<RefCell<Shared>>);

impl Default for TextEditorController {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for TextEditorController {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextEditorController")
            .finish_non_exhaustive()
    }
}

impl TextEditorController {
    /// Controlador solto, para passar a um editor.
    pub fn new() -> Self {
        Self(Rc::new(RefCell::new(Shared::new())))
    }

    /// Roda `f` com o estado e o texto atual; se `f` devolve o texto novo, grava no signal e
    /// avisa o `on_change`, **sem** segurar o estado nessas chamadas.
    fn mutate(&self, f: impl FnOnce(&mut Shared, &str) -> Option<String>) -> bool {
        let value = self.0.borrow().value;
        let Some(value) = value else {
            return false;
        };
        let new = {
            let mut shared = self.0.borrow_mut();
            let text = value.with_untracked(|v| shared.current(v));
            f(&mut shared, &text)
        };
        self.bump();
        let Some(new) = new else {
            return false;
        };
        value.set(new);
        let on_change = self.0.borrow().handlers.on_change.clone();
        if let Some(on_change) = on_change {
            on_change();
        }
        true
    }

    /// Roda `f` com o estado e o texto atual sem mexer no texto.
    fn read<R>(&self, f: impl FnOnce(&mut Shared, &str) -> R) -> Option<R> {
        let value = self.0.borrow().value?;
        let mut shared = self.0.borrow_mut();
        let text = value.with_untracked(|v| shared.current(v));
        Some(f(&mut shared, &text))
    }

    /// Faz o visual ser relido: o app mexeu por fora de um evento do editor.
    fn bump(&self) {
        let version = self.0.borrow().version;
        if let Some(version) = version {
            version.set(version.get_untracked().wrapping_add(1));
        }
        crate::wake();
    }

    fn flags(&self) -> (bool, bool) {
        let (read_only, disabled) = {
            let shared = self.0.borrow();
            (shared.read_only.clone(), shared.disabled.clone())
        };
        untrack(|| (read_only.get(), disabled.get()))
    }

    /// O texto do editor.
    pub fn value(&self) -> String {
        let value = self.0.borrow().value;
        value.map(|v| v.get_untracked()).unwrap_or_default()
    }

    /// A seleção em bytes, do começo ao fim; o cursor sem seleção é um trecho vazio.
    pub fn selection(&self) -> Range<usize> {
        self.read(|s, _| s.selection()).unwrap_or(0..0)
    }

    /// Seleciona `range` (limitado ao texto e a limites de `char`), com o cursor no fim, e
    /// mostra o cursor no próximo quadro.
    pub fn set_selection(&self, range: Range<usize>) {
        self.read(|s, text| {
            let start = floor_boundary(text, range.start.min(range.end));
            let end = floor_boundary(text, range.end.max(range.start));
            s.anchor = start;
            s.move_to(end, false, true);
            s.reveal = true;
        });
        self.bump();
    }

    /// Troca `range` por `text` num passo só do desfazer, com o cursor no fim do texto novo.
    pub fn replace(&self, range: Range<usize>, text: &str) {
        self.mutate(|s, current| {
            let start = floor_boundary(current, range.start.min(range.end));
            let end = ceil_boundary(current, range.end.max(range.start));
            let new = s.replace_range(current, start..end, text, false);
            s.reveal = true;
            new
        });
    }

    /// Troca a seleção (ou insere no cursor) por `text`.
    pub fn replace_selection(&self, text: &str) {
        self.mutate(|s, current| {
            let range = s.selection();
            let range = floor_boundary(current, range.start)..ceil_boundary(current, range.end);
            let new = s.replace_range(current, range, text, false);
            s.reveal = true;
            new
        });
    }

    /// Pede o foco para o editor (precisa ter `.id`).
    pub fn focus(&self) {
        let id = self.0.borrow().id.clone();
        match id {
            Some(id) => put_request(&FOCUS, id),
            None => self.0.borrow_mut().want_focus = true,
        }
    }

    /// Rola até a posição `byte`: se a linha dela não está inteira à vista, vai para o meio.
    pub fn scroll_to(&self, byte: usize) {
        self.0.borrow_mut().pending_scroll = Some(byte);
        self.bump();
    }

    /// Onde `needle` aparece no texto, sem sobrepor. Com `ignore_case`, sem diferenciar
    /// maiúsculas nem acentos. Texto vazio não acha nada.
    pub fn find(&self, needle: &str, ignore_case: bool) -> Vec<Range<usize>> {
        let text = self.value();
        find_in(&text, needle, ignore_case)
    }

    /// Esquece o desfazer e o refazer (o app trocou de nota).
    pub fn clear_history(&self) {
        let mut shared = self.0.borrow_mut();
        shared.undo.clear();
        shared.redo.clear();
        shared.typing = false;
    }

    /// Seleciona o texto todo.
    pub fn select_all(&self) {
        self.read(|s, text| {
            s.anchor = 0;
            s.move_to(text.len(), false, true);
        });
        self.bump();
    }

    /// Desfaz o último passo.
    pub fn undo(&self) {
        if !self.flags().0 {
            self.mutate(|s, text| s.undo_step(text));
        }
    }

    /// Refaz o último passo desfeito.
    pub fn redo(&self) {
        if !self.flags().0 {
            self.mutate(|s, text| s.redo_step(text));
        }
    }

    /// O texto selecionado, se há seleção.
    fn selected_text(&self) -> Option<String> {
        self.read(|s, text| {
            let range = s.selection();
            (!range.is_empty()).then(|| text[range].to_owned())
        })
        .flatten()
    }

    /// Copia a seleção para `clipboard`, ou a entrega ao `on_copy`, que a substitui.
    pub fn copy(&self, clipboard: &mut dyn Clipboard) {
        let Some(selected) = self.selected_text() else {
            return;
        };
        let on_copy = self.0.borrow().handlers.on_copy.clone();
        match on_copy {
            Some(on_copy) => on_copy(selected),
            None => clipboard.write(selected),
        }
    }

    /// Copia a seleção e a apaga (a apagar só fora do modo só leitura).
    pub fn cut(&self, clipboard: &mut dyn Clipboard) {
        self.copy(clipboard);
        let (read_only, disabled) = self.flags();
        if !read_only && !disabled {
            self.mutate(|s, text| {
                let range = s.selection();
                s.replace_range(text, range, "", false)
            });
        }
    }

    /// Cola: os arquivos copiados (só com `on_paste_files`), senão o texto, senão a imagem (só
    /// com `on_paste_image`). Só leitura e desabilitado não colam.
    pub fn paste(&self, clipboard: &mut dyn Clipboard) {
        let (read_only, disabled) = self.flags();
        if read_only || disabled {
            return;
        }
        let handlers = self.0.borrow().handlers.clone();
        if let Some(on_files) = &handlers.on_paste_files
            && let Some(files) = clipboard.read_files().filter(|f| !f.is_empty())
        {
            on_files(files, self);
            return;
        }
        if let Some(pasted) = clipboard.read().filter(|t| !t.is_empty()) {
            let pasted = multi_line(&pasted);
            self.mutate(|s, text| {
                let range = s.selection();
                s.replace_range(text, range, &pasted, false)
            });
            return;
        }
        if let Some(on_image) = &handlers.on_paste_image
            && let Some(image) = clipboard.read_image()
        {
            on_image(image, self);
        }
    }
}

/// O realce do app: dado o texto, devolve as faixas de estilo.
type Highlight = Rc<dyn Fn(&str) -> Vec<StyleRange>>;

/// Editor de texto de várias linhas, com ligação bidirecional ao signal.
///
/// ```
/// use tomni_ui_core::TextEditor;
/// use tomni_ui_reactive::prelude::*;
///
/// # let scope = Scope::root();
/// # scope.run(|| {
/// let nota = signal(String::new());
/// let editor = TextEditor::bind(nota).id("nota").placeholder("Escreva aqui").monospace();
/// # drop(editor);
/// # });
/// # scope.clear();
/// ```
///
/// Regras:
///
/// - Quebra as linhas na largura do campo ([`TextEditor::wrap`]); sem quebra, rola na
///   horizontal. Rola na vertical com a roda, as barras (por dentro da borda) e o teclado.
/// - Setas, Ctrl+setas (por palavra), Home e End (da linha visual), Ctrl+Home e Ctrl+End,
///   PageUp e PageDown, com Shift para selecionar; Ctrl+A, C, X, V, Z, e Y ou Shift+Z.
/// - Enter insere `\n`. **Tab sai do campo**, como no `Edit`: o app o intercepta no
///   [`TextEditor::on_key`] quando quer outro uso (recuar uma lista, por exemplo).
/// - Mouse: clique, arrastar (rola na borda enquanto o ponteiro mexe), duplo clique na palavra,
///   triplo na linha, Shift+clique estende, e o menu de contexto com Recortar, Copiar, Colar,
///   Desfazer e Selecionar tudo.
/// - Desfazer guarda 100 passos e junta a digitação seguida; cada chamada do
///   [`TextEditorController`] é um passo. Um texto mudado por fora do editor zera o histórico.
/// - O texto todo é medido de uma vez (não por parágrafo): é feito para notas, não para
///   arquivos de megabytes.
pub struct TextEditor {
    common: Common,
    shared: Rc<RefCell<Shared>>,
    placeholder: Prop<String>,
    wrap: bool,
    mono: bool,
    min_height: f64,
    max_height: Option<f64>,
    highlight: Option<Highlight>,
    marks: Option<Signal<Vec<Mark>>>,
    /// O realce do último texto: só roda de novo quando o texto muda.
    styles: RefCell<(Rc<str>, Rc<Vec<StyleRange>>)>,
    preedit: Option<(String, Option<(usize, usize)>)>,
    hovered: bool,
    dragging: bool,
    focus: Option<FocusSource>,
    scroller: Scroller,
    bar: ScrollBarState,
    bar_drag: Option<Drag>,
    density: Density,
}

/// O que o editor mostra agora: o texto real, o exibido (com a composição) e o que vale em cima
/// dele.
struct Frame {
    /// Texto do signal.
    text: Rc<str>,
    /// Texto exibido: com a composição do IME no cursor.
    shown: Rc<str>,
    ranges: Rc<Vec<StyleRange>>,
    marks: Rc<Vec<Mark>>,
    cursor: usize,
    anchor: usize,
    /// Cursor no texto exibido.
    caret: usize,
    composing: Option<Range<usize>>,
}

impl TextEditor {
    /// Editor que mostra e altera `value`.
    pub fn bind(value: Signal<String>) -> Self {
        let mut shared = Shared::new();
        shared.value = Some(value);
        shared.version = Some(signal(0));
        shared.known = Rc::from(value.get_untracked());
        let common = Common {
            grow: 1.0,
            ..Common::default()
        };
        Self {
            common,
            shared: Rc::new(RefCell::new(shared)),
            placeholder: Prop::default(),
            wrap: true,
            mono: false,
            min_height: DEFAULT_MIN_HEIGHT,
            max_height: None,
            highlight: None,
            marks: None,
            styles: RefCell::new((Rc::from(""), Rc::default())),
            preedit: None,
            hovered: false,
            dragging: false,
            focus: None,
            scroller: Scroller::new(ScrollMode::Vertical),
            bar: ScrollBarState::Rest,
            bar_drag: None,
            density: Density::Standard,
        }
    }

    /// Texto mostrado enquanto o editor está vazio (fixo, closure ou signal).
    pub fn placeholder<M>(mut self, placeholder: impl IntoProp<String, M>) -> Self {
        self.placeholder = placeholder.into_prop();
        self
    }

    /// Quebra as linhas na largura do campo (o padrão). Com `false`, as linhas compridas rolam
    /// na horizontal.
    pub fn wrap(mut self, wrap: bool) -> Self {
        self.wrap = wrap;
        self.scroller = Scroller::new(if wrap {
            ScrollMode::Vertical
        } else {
            ScrollMode::Both
        });
        self
    }

    /// Fonte monoespaçada (a do terminal, com a feature `mono`; sem ela, a do texto comum).
    pub fn monospace(mut self) -> Self {
        self.mono = true;
        self
    }

    /// Só leitura (fixo, closure ou signal): recebe foco, seleciona e copia, mas não muda o
    /// texto.
    pub fn read_only<M>(self, read_only: impl IntoProp<bool, M>) -> Self {
        self.shared.borrow_mut().read_only = read_only.into_prop();
        self
    }

    /// Desabilita o editor (fixo, closure ou signal).
    pub fn disabled<M>(self, disabled: impl IntoProp<bool, M>) -> Self {
        self.shared.borrow_mut().disabled = disabled.into_prop();
        self
    }

    /// Altura mínima, e a altura do campo quando não há `.max_height` nem `.height`
    /// (padrão 96 px).
    pub fn min_height(mut self, height: f64) -> Self {
        self.min_height = height;
        self
    }

    /// Altura máxima: o campo cresce com o texto até esse limite e então rola. Sem ela, ele
    /// ocupa o espaço que o pai der (`grow`) e rola dentro dele. Define `grow` como zero:
    /// chame `.grow()` depois para mudar.
    pub fn max_height(mut self, height: f64) -> Self {
        self.max_height = Some(height);
        self.common.grow = 0.0;
        self
    }

    /// Realce: devolve, para o texto atual, as faixas com estilo. Roda de novo só quando o
    /// texto muda, fora de qualquer rastreamento de signal. Faixas fora do texto ou fora de um
    /// limite de `char` são ajustadas ou descartadas.
    pub fn highlight(mut self, highlight: impl Fn(&str) -> Vec<StyleRange> + 'static) -> Self {
        self.highlight = Some(Rc::new(highlight));
        self
    }

    /// Marcas de busca ligadas a um signal.
    pub fn marks(mut self, marks: Signal<Vec<Mark>>) -> Self {
        self.marks = Some(marks);
        self
    }

    /// Liga o controlador `controller` a este editor.
    pub fn controller(mut self, controller: TextEditorController) -> Self {
        {
            let old = self.shared.borrow();
            let mut new = controller.0.borrow_mut();
            new.value = old.value;
            new.version = old.version;
            new.id.clone_from(&old.id);
            new.read_only = old.read_only.clone();
            new.disabled = old.disabled.clone();
            new.known = Rc::clone(&old.known);
            new.cursor = new.cursor.min(new.known.len());
            new.anchor = new.anchor.min(new.known.len());
            let handlers = &old.handlers;
            macro_rules! mover {
                ($($campo:ident),*) => {
                    $(if handlers.$campo.is_some() {
                        new.handlers.$campo.clone_from(&handlers.$campo);
                    })*
                };
            }
            mover!(
                on_key,
                on_copy,
                on_change,
                on_paste_files,
                on_paste_image,
                on_files_dropped
            );
        }
        self.shared = controller.0;
        self
    }

    /// Chamado antes do tratamento padrão de cada tecla (com a composição do IME fechada):
    /// devolva `true` para consumir. É o lugar de recuar uma lista com Tab ou de continuar uma
    /// com Enter.
    pub fn on_key(
        self,
        handler: impl Fn(&KeyEvent, &TextEditorController) -> bool + 'static,
    ) -> Self {
        self.shared.borrow_mut().handlers.on_key = Some(Rc::new(handler));
        self
    }

    /// Chamado com o texto copiado (Ctrl+C, Ctrl+X, menu), **no lugar** de gravá-lo na área de
    /// transferência: o app decide o que copiar. Recortar ainda apaga a seleção.
    pub fn on_copy(self, handler: impl Fn(String) + 'static) -> Self {
        self.shared.borrow_mut().handlers.on_copy = Some(Rc::new(handler));
        self
    }

    /// Chamado depois de cada mudança do texto feita pelo editor ou pelo controlador.
    pub fn on_change(self, handler: impl Fn() + 'static) -> Self {
        self.shared.borrow_mut().handlers.on_change = Some(Rc::new(handler));
        self
    }

    /// Chamado quando se cola e a área de transferência tem arquivos copiados, antes do texto.
    pub fn on_paste_files(
        self,
        handler: impl Fn(Vec<PathBuf>, &TextEditorController) + 'static,
    ) -> Self {
        self.shared.borrow_mut().handlers.on_paste_files = Some(Rc::new(handler));
        self
    }

    /// Chamado quando se cola e a área de transferência não tem texto, só uma imagem. Sem este
    /// método, a imagem nem é lida.
    pub fn on_paste_image(
        self,
        handler: impl Fn(ClipboardImage, &TextEditorController) + 'static,
    ) -> Self {
        self.shared.borrow_mut().handlers.on_paste_image = Some(Rc::new(handler));
        self
    }

    /// Chamado quando arquivos do sistema são soltos sobre o editor: o cursor vai ao ponto
    /// sob o ponteiro e o evento é consumido. Sem este método (ou só leitura, ou
    /// desabilitado), o evento sobe aos ancestrais.
    pub fn on_files_dropped(
        self,
        handler: impl Fn(Vec<PathBuf>, &TextEditorController) + 'static,
    ) -> Self {
        self.shared.borrow_mut().handlers.on_files_dropped = Some(Rc::new(handler));
        self
    }

    fn controller_handle(&self) -> TextEditorController {
        TextEditorController(Rc::clone(&self.shared))
    }

    /// Se é só leitura e se está desabilitado agora (sem rastrear).
    fn flags(&self) -> (bool, bool) {
        self.controller_handle().flags()
    }

    /// As faixas de estilo de `text`, do cache ou do `highlight`.
    fn styles(&self, text: &Rc<str>) -> Rc<Vec<StyleRange>> {
        let Some(highlight) = &self.highlight else {
            return Rc::default();
        };
        {
            let cache = self.styles.borrow();
            if Rc::ptr_eq(&cache.0, text) || *cache.0 == **text {
                return Rc::clone(&cache.1);
            }
        }
        let ranges: Vec<StyleRange> = highlight(text)
            .into_iter()
            .filter_map(|mut r| {
                r.range = clean_range(text, &r.range)?;
                Some(r)
            })
            .collect();
        let ranges = Rc::new(ranges);
        *self.styles.borrow_mut() = (Rc::clone(text), Rc::clone(&ranges));
        ranges
    }

    /// O que o editor mostra agora, sem rastrear signals.
    fn frame(&self) -> Frame {
        untrack(|| {
            let value = self.shared.borrow().value;
            let text: Rc<str> = match value {
                Some(value) => value.with_untracked(|v| self.shared.borrow_mut().current(v)),
                None => Rc::from(""),
            };
            let (cursor, anchor) = {
                let shared = self.shared.borrow();
                (shared.cursor, shared.anchor)
            };
            let ranges = self.styles(&text);
            let marks: Vec<Mark> = self
                .marks
                .map(|m| m.get_untracked())
                .unwrap_or_default()
                .into_iter()
                .filter_map(|mut m| {
                    m.range = clean_range(&text, &m.range)?;
                    Some(m)
                })
                .collect();
            let mut frame = Frame {
                shown: Rc::clone(&text),
                text,
                ranges,
                marks: Rc::new(marks),
                cursor,
                anchor,
                caret: cursor,
                composing: None,
            };
            if self.focus.is_some()
                && let Some((preedit, highlight)) = &self.preedit
            {
                let by = preedit.len();
                let mut shown = String::with_capacity(frame.text.len() + by);
                shown.push_str(&frame.text[..cursor]);
                shown.push_str(preedit);
                shown.push_str(&frame.text[cursor..]);
                frame.shown = Rc::from(shown);
                frame.composing = Some(cursor..cursor + by);
                frame.caret = cursor + highlight.map_or(by, |(_, end)| end.min(by));
                frame.ranges = Rc::new(
                    frame
                        .ranges
                        .iter()
                        .cloned()
                        .map(|mut r| {
                            r.range = shift(&r.range, cursor, by);
                            r
                        })
                        .collect(),
                );
                frame.marks = Rc::new(
                    frame
                        .marks
                        .iter()
                        .cloned()
                        .map(|mut m| {
                            m.range = shift(&m.range, cursor, by);
                            m
                        })
                        .collect(),
                );
            }
            frame
        })
    }

    /// Estado para o `Drawer`. Lê o texto, o placeholder, as marcas, o `disabled`, o
    /// `read_only` e a versão do controlador de forma rastreada.
    pub fn visual(&self) -> TextEditorVisual {
        let (value, version, read_only, disabled) = {
            let shared = self.shared.borrow();
            (
                shared.value,
                shared.version,
                shared.read_only.clone(),
                shared.disabled.clone(),
            )
        };
        if let Some(value) = value {
            value.with(|_| ());
        }
        if let Some(version) = version {
            version.get();
        }
        if let Some(marks) = self.marks {
            marks.with(|_| ());
        }
        let read_only = read_only.get();
        let disabled = disabled.get();
        let placeholder = self.placeholder.get();
        let frame = self.frame();
        let focused = self.focus.is_some() && !disabled;
        let selected = frame.cursor != frame.anchor;
        let mut visual = TextEditorVisual {
            text: Rc::clone(&frame.shown),
            placeholder,
            caret: None,
            caret_up: self.shared.borrow().caret_up,
            selection: None,
            composing: None,
            ranges: Rc::clone(&frame.ranges),
            marks: Rc::clone(&frame.marks),
            wrap: self.wrap,
            mono: self.mono,
            state: if disabled {
                EditState::Disabled
            } else if focused {
                EditState::Focused
            } else if self.hovered {
                EditState::Hover
            } else {
                EditState::Rest
            },
            density: self.density,
            scroller: self.scroller,
            bar: self.bar,
            min_height: self.min_height,
            max_height: self.max_height,
            read_only,
        };
        if focused {
            visual.caret = (!read_only).then_some(frame.caret);
            if frame.composing.is_some() {
                visual.composing.clone_from(&frame.composing);
            } else if selected {
                visual.selection =
                    Some(frame.cursor.min(frame.anchor)..frame.cursor.max(frame.anchor));
            }
        }
        visual
    }

    // ---- Geometria ----

    /// A área do texto e os pontos do cursor de `frame`.
    fn layout(&self, frame: &Frame, cx: &mut EventCx) -> (Rect, Rc<EditorPoints>) {
        let rect = cx.rect();
        let area = cx.metrics().editor_area(rect, self.density);
        let wrap = self.wrap.then(|| area.width());
        let points = cx.metrics().editor_points(&EditorText {
            text: &frame.shown,
            ranges: &frame.ranges,
            wrap,
            mono: self.mono,
        });
        (area, points)
    }

    /// Acerta o tamanho do conteúdo e da área visível na rolagem. Devolve `true` se mudou.
    fn sync_extent(&mut self, frame: &Frame, cx: &mut EventCx) -> bool {
        let (area, points) = self.layout(frame, cx);
        let line = cx.metrics().line_height();
        let viewport = area.size();
        let content = kurbo::Size::new(
            if self.wrap {
                viewport.width
            } else {
                points.width + 1.0
            },
            points.lines as f64 * line,
        );
        self.scroller.set_extent(content, viewport)
    }

    /// Rola o mínimo para o cursor ficar à vista. Devolve `true` se rolou.
    fn ensure_visible(&mut self, frame: &Frame, cx: &mut EventCx) -> bool {
        let (area, points) = self.layout(frame, cx);
        let line_height = cx.metrics().line_height();
        let up = self.shared.borrow().caret_up;
        let (x, line) = points.locate(frame.caret, up);
        let offset = self.scroller.offset();
        let top = line as f64 * line_height;
        let mut target = offset;
        if top < target.y {
            target.y = top;
        }
        if top + line_height > target.y + area.height() {
            target.y = top + line_height - area.height();
        }
        if !self.wrap {
            if x < target.x {
                target.x = x;
            }
            if x + 1.0 > target.x + area.width() {
                target.x = x + 1.0 - area.width();
            }
        }
        self.scroller.scroll_to(target)
    }

    /// O pedido de rolar até `byte`: se a linha não está inteira à vista, vai para o meio.
    fn scroll_to_byte(&mut self, frame: &Frame, cx: &mut EventCx, byte: usize) -> bool {
        let (area, points) = self.layout(frame, cx);
        let line_height = cx.metrics().line_height();
        let (_, line) = points.locate(byte.min(frame.shown.len()), false);
        let top = line as f64 * line_height;
        let offset = self.scroller.offset();
        if top >= offset.y && top + line_height <= offset.y + area.height() {
            return false;
        }
        let target = Vec2::new(offset.x, top - (area.height() - line_height) / 2.0);
        self.scroller.scroll_to(target)
    }

    /// O byte mais perto do ponteiro, e se é o fim da linha de cima de uma quebra suave.
    fn hit(&self, frame: &Frame, cx: &mut EventCx) -> (usize, bool) {
        let (area, points) = self.layout(frame, cx);
        let line_height = cx.metrics().line_height();
        let offset = self.scroller.offset();
        let pointer = cx.pointer();
        let x = pointer.x - area.x0 + offset.x;
        let y = pointer.y - area.y0 + offset.y;
        let line = ((y / line_height).floor().max(0.0) as usize).min(points.last_line());
        points.nearest(line, x)
    }

    /// A barra de rolagem sob `point`, se houver.
    fn bar_at(&self, rect: Rect, point: Point) -> Option<ScrollBarParts> {
        [ScrollAxis::Y, ScrollAxis::X]
            .into_iter()
            .filter_map(|axis| self.scroller.bar(rect, axis))
            .find(|parts| parts.bar.contains(point))
    }

    // ---- Eventos ----

    fn set_focus(&mut self, focus: Option<FocusSource>) {
        self.focus = focus;
    }

    fn event(&mut self, frame: &Frame, event: &Event, cx: &mut EventCx) -> bool {
        let controller = self.controller_handle();
        let (read_only, _) = self.flags();
        match event {
            Event::PointerMove { kind, inside } => {
                self.hovered = *inside && kind.hovers();
                let bars = TextEditorVisual::bars_rect(cx.rect());
                if let Some(drag) = self.bar_drag {
                    drag_bar(&mut self.scroller, drag, bars, cx.pointer());
                } else if self.dragging {
                    self.drag_select(frame, cx);
                } else {
                    let over = self.bar_at(bars, cx.pointer()).is_some();
                    self.bar = if over {
                        ScrollBarState::Hover
                    } else {
                        ScrollBarState::Rest
                    };
                }
                true
            }
            Event::PointerDown { kind, clicks } => {
                self.hovered = kind.hovers();
                let bars = TextEditorVisual::bars_rect(cx.rect());
                if let Some(parts) = self.bar_at(bars, cx.pointer()) {
                    self.bar_drag = press_bar(&mut self.scroller, parts, cx.pointer());
                    self.bar = ScrollBarState::Pressed;
                    return true;
                }
                self.preedit = None;
                let frame = self.frame();
                let (at, up) = self.hit(&frame, cx);
                let text = &frame.text;
                let shift = cx.modifiers().shift;
                let mut shared = self.shared.borrow_mut();
                match clicks {
                    2 => {
                        let (start, end) = word_at(text, at);
                        shared.anchor = start;
                        shared.move_to(end, false, true);
                    }
                    3.. => {
                        let start = text[..at].rfind('\n').map_or(0, |i| i + 1);
                        let end = text[at..].find('\n').map_or(text.len(), |i| at + i);
                        shared.anchor = start;
                        shared.move_to(end, false, true);
                    }
                    _ => {
                        shared.move_to(at, up, shift);
                        drop(shared);
                        self.dragging = true;
                    }
                }
                true
            }
            Event::PointerUp { kind, inside } => {
                self.hovered = *inside && kind.hovers();
                self.dragging = false;
                self.bar_drag = None;
                self.bar = if self
                    .bar_at(TextEditorVisual::bars_rect(cx.rect()), cx.pointer())
                    .is_some()
                {
                    ScrollBarState::Hover
                } else {
                    ScrollBarState::Rest
                };
                true
            }
            Event::PointerCancel => {
                self.hovered = false;
                self.dragging = false;
                self.bar_drag = None;
                self.bar = ScrollBarState::Rest;
                true
            }
            Event::FocusIn(source) => {
                self.set_focus(Some(*source));
                self.shared.borrow_mut().reveal = true;
                true
            }
            Event::FocusOut => {
                self.set_focus(None);
                self.preedit = None;
                self.dragging = false;
                self.bar_drag = None;
                self.shared.borrow_mut().typing = false;
                true
            }
            Event::ContextOpening => true,
            _ if self.focus.is_none() => false,
            Event::KeyDown(key) => {
                if self.preedit.is_none() {
                    let handler = self.shared.borrow().handlers.on_key.clone();
                    if let Some(handler) = handler {
                        let key_event = KeyEvent {
                            key: *key,
                            modifiers: cx.modifiers(),
                        };
                        if handler(&key_event, &controller) {
                            return true;
                        }
                    }
                }
                self.key(frame, *key, cx)
            }
            Event::Text(typed) => {
                if read_only {
                    return false;
                }
                let typed: String = typed
                    .chars()
                    .filter(|&c| !c.is_control() || c == '\n')
                    .collect();
                if typed.is_empty() {
                    return false;
                }
                self.preedit = None;
                controller.mutate(|s, text| {
                    let range = s.selection();
                    s.replace_range(text, range, &typed, true)
                });
                true
            }
            Event::Ime(Ime::Preedit {
                text: preedit,
                cursor,
            }) => {
                if read_only {
                    return false;
                }
                if frame.cursor != frame.anchor {
                    controller.mutate(|s, text| {
                        let range = s.selection();
                        s.replace_range(text, range, "", true)
                    });
                }
                self.preedit = (!preedit.is_empty()).then(|| (preedit.clone(), *cursor));
                true
            }
            Event::Ime(Ime::Commit(commit)) => {
                self.preedit = None;
                if read_only {
                    return false;
                }
                controller.mutate(|s, text| {
                    let range = s.selection();
                    s.replace_range(text, range, commit, true)
                });
                true
            }
            _ => false,
        }
    }

    /// O arrasto com o botão pressionado: estende a seleção até o ponteiro, rolando quando ele
    /// está além da borda (a cada movimento, proporcional à distância).
    fn drag_select(&mut self, frame: &Frame, cx: &mut EventCx) {
        let rect = cx.rect();
        let area = cx.metrics().editor_area(rect, self.density);
        let pointer = cx.pointer();
        let push = |outside: f64| (outside / 4.0 + 4.0).min(48.0);
        let mut delta = Vec2::ZERO;
        if pointer.y < area.y0 {
            delta.y = -push(area.y0 - pointer.y);
        } else if pointer.y > area.y1 {
            delta.y = push(pointer.y - area.y1);
        }
        if !self.wrap {
            if pointer.x < area.x0 {
                delta.x = -push(area.x0 - pointer.x);
            } else if pointer.x > area.x1 {
                delta.x = push(pointer.x - area.x1);
            }
        }
        if delta != Vec2::ZERO {
            self.scroller.scroll_by(delta);
        }
        let (at, up) = self.hit(frame, cx);
        self.shared.borrow_mut().move_to(at, up, true);
    }

    fn key(&mut self, frame: &Frame, key: Key, cx: &mut EventCx) -> bool {
        if self.preedit.is_some() {
            // Com a composição aberta, as teclas são do IME.
            return false;
        }
        let mods = cx.modifiers();
        let shift = mods.shift;
        let word = word_modifier(mods);
        let controller = self.controller_handle();
        let (read_only, _) = self.flags();
        let text = &frame.text;
        let (start, end) = (
            frame.cursor.min(frame.anchor),
            frame.cursor.max(frame.anchor),
        );
        let selected = start != end;
        let cursor = frame.cursor;
        let up = self.shared.borrow().caret_up;
        let (area, points) = self.layout(frame, cx);
        let line_height = cx.metrics().line_height();
        let (x, line) = points.locate(cursor, up);
        let move_to = |at: usize, up: bool| {
            controller.0.borrow_mut().move_to(at, up, shift);
        };
        match key {
            Key::Left | Key::Right => {
                let left = key == Key::Left;
                let edge = cfg!(any(target_os = "macos", target_os = "ios")) && mods.meta;
                let at = if edge {
                    if left { 0 } else { text.len() }
                } else if selected && !shift && !word {
                    if left { start } else { end }
                } else if word {
                    if left {
                        word_before(text, cursor)
                    } else {
                        word_after(text, cursor)
                    }
                } else if left {
                    points.prev_stop(cursor).unwrap_or(0)
                } else {
                    points.next_stop(cursor).unwrap_or(text.len())
                };
                move_to(at, false);
                true
            }
            Key::Up | Key::Down | Key::PageUp | Key::PageDown => {
                let page = ((area.height() / line_height).floor() as usize).max(1);
                let step = match key {
                    Key::Up | Key::Down => 1,
                    _ => page,
                };
                let backwards = matches!(key, Key::Up | Key::PageUp);
                let goal = controller.0.borrow().goal_x.unwrap_or(x);
                let last = points.last_line();
                let (at, new_up) = if backwards {
                    if line == 0 {
                        (0, false)
                    } else {
                        points.nearest(line.saturating_sub(step), goal)
                    }
                } else if line >= last {
                    (text.len(), false)
                } else {
                    points.nearest((line + step).min(last), goal)
                };
                if matches!(key, Key::PageUp | Key::PageDown) {
                    let by = step as f64 * line_height;
                    self.scroller
                        .scroll_by(Vec2::new(0.0, if backwards { -by } else { by }));
                }
                move_to(at, new_up);
                controller.0.borrow_mut().goal_x = Some(goal);
                true
            }
            Key::Home | Key::End => {
                if mods.ctrl || mods.meta {
                    if key == Key::Home {
                        move_to(0, false);
                    } else {
                        move_to(text.len(), false);
                    }
                    return true;
                }
                let on_line = points.line_points(line);
                let point = if key == Key::Home {
                    on_line.first()
                } else {
                    on_line.last()
                };
                if let Some(&(byte, _, _)) = point {
                    let up = points.locate(byte, false).1 != line;
                    move_to(byte, up);
                }
                true
            }
            Key::Enter if !read_only => {
                controller.mutate(|s, current| {
                    let range = s.selection();
                    s.replace_range(current, range, "\n", false)
                });
                true
            }
            Key::Backspace | Key::Delete if !read_only => {
                let back = key == Key::Backspace;
                let range = if selected {
                    start..end
                } else if back {
                    let at = if word {
                        word_before(text, cursor)
                    } else {
                        points.prev_stop(cursor).unwrap_or(0)
                    };
                    at..cursor
                } else {
                    let at = if word {
                        word_after(text, cursor)
                    } else {
                        points.next_stop(cursor).unwrap_or(text.len())
                    };
                    cursor..at
                };
                controller.mutate(|s, current| s.replace_range(current, range, "", false));
                true
            }
            Key::Insert if shift && !read_only => {
                controller.paste(cx.clipboard());
                true
            }
            Key::Char(c) if mods.command() => match c.to_ascii_lowercase() {
                'a' => {
                    let mut shared = controller.0.borrow_mut();
                    shared.anchor = 0;
                    shared.move_to(text.len(), false, true);
                    true
                }
                'c' => {
                    controller.copy(cx.clipboard());
                    true
                }
                'x' => {
                    controller.cut(cx.clipboard());
                    true
                }
                'v' if !read_only => {
                    controller.paste(cx.clipboard());
                    true
                }
                'z' if read_only => false,
                'z' if shift => {
                    controller.redo();
                    true
                }
                'z' => {
                    controller.undo();
                    true
                }
                'y' if !read_only => {
                    controller.redo();
                    true
                }
                _ => false,
            },
            _ => false,
        }
    }

    /// Solta arquivos sobre o editor: o cursor vai ao ponto sob o ponteiro.
    fn files_dropped(&mut self, paths: &[PathBuf], cx: &mut EventCx) -> bool {
        let (read_only, disabled) = self.flags();
        let handler = self.shared.borrow().handlers.on_files_dropped.clone();
        let Some(handler) = handler else {
            return false;
        };
        if read_only || disabled {
            return false;
        }
        let frame = self.frame();
        let (at, up) = self.hit(&frame, cx);
        self.shared.borrow_mut().move_to(at, up, false);
        handler(paths.to_vec(), &self.controller_handle());
        true
    }

    /// Trata um evento. Devolve `true` se o editor o consumiu.
    pub fn handle(&mut self, event: &Event, cx: &mut EventCx) -> bool {
        let frame = self.frame();
        self.sync_extent(&frame, cx);
        if let Event::Scroll { x, y } = event {
            return self
                .scroller
                .scroll_by(Vec2::new(f64::from(*x), f64::from(*y)));
        }
        let (_, disabled) = self.flags();
        if disabled {
            self.hovered = false;
            self.dragging = false;
            self.bar_drag = None;
            self.set_focus(None);
            self.preedit = None;
            return false;
        }
        let consumed = if let Event::FilesDropped { paths } = event {
            self.files_dropped(paths, cx)
        } else {
            self.event(&frame, event, cx)
        };
        if consumed {
            let frame = self.frame();
            self.sync_extent(&frame, cx);
            if !matches!(
                event,
                Event::PointerMove { .. } | Event::PointerUp { .. } | Event::PointerDown { .. }
            ) || self.dragging
            {
                self.ensure_visible(&frame, cx);
            }
        }
        consumed
    }

    /// Acerta o tamanho do conteúdo e aplica os pedidos de rolar e de mostrar o cursor.
    fn place(&mut self, cx: &mut EventCx) -> bool {
        let frame = self.frame();
        let mut changed = self.sync_extent(&frame, cx);
        let (scroll, reveal) = {
            let mut shared = self.shared.borrow_mut();
            (
                shared.pending_scroll.take(),
                std::mem::take(&mut shared.reveal),
            )
        };
        if let Some(byte) = scroll {
            changed |= self.scroll_to_byte(&frame, cx, byte);
        }
        if reveal && self.focus.is_some() {
            changed |= self.ensure_visible(&frame, cx);
        }
        changed
    }

    /// O menu de contexto: Recortar, Copiar, Colar, Desfazer e Selecionar tudo.
    fn context_menu(&self) -> MenuFlyout {
        let base = self.shared.borrow().id.clone().unwrap_or_else(|| {
            let n = NEXT_ID.with(|n| {
                n.set(n.get() + 1);
                n.get()
            });
            format!("editor-{n}")
        });
        let c = self.controller_handle();
        let (cut, copy, paste, undo, all) = (c.clone(), c.clone(), c.clone(), c.clone(), c);
        MenuFlyout::new()
            .item(format!("{base}-recortar"), "Recortar", move || {
                let c = cut.clone();
                clipboard::with(move |cb| c.cut(cb));
            })
            .item(format!("{base}-copiar"), "Copiar", move || {
                let c = copy.clone();
                clipboard::with(move |cb| c.copy(cb));
            })
            .item(format!("{base}-colar"), "Colar", move || {
                let c = paste.clone();
                clipboard::with(move |cb| c.paste(cb));
            })
            .item(format!("{base}-desfazer"), "Desfazer", move || undo.undo())
            .item(
                format!("{base}-selecionar-tudo"),
                "Selecionar tudo",
                move || all.select_all(),
            )
            .id(format!("{base}-menu"))
    }
}

/// Limita `range` ao texto e a limites de `char`; `None` se sobra um trecho vazio.
fn clean_range(text: &str, range: &Range<usize>) -> Option<Range<usize>> {
    let start = ceil_boundary(text, range.start);
    let end = floor_boundary(text, range.end);
    (start < end).then_some(start..end)
}

/// Desloca `range` em `by` bytes para o que está depois de `at`, onde entrou a composição.
fn shift(range: &Range<usize>, at: usize, by: usize) -> Range<usize> {
    let start = if range.start >= at {
        range.start + by
    } else {
        range.start
    };
    let end = if range.end > at {
        range.end + by
    } else {
        range.end
    };
    start..end
}

common_builders!(TextEditor);

impl Widget for TextEditor {
    fn kind(&self) -> &'static str {
        "text_editor"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::TextEditor(self.visual())
    }

    fn set_density(&mut self, density: Density) {
        self.density = density;
    }

    fn measured(&self) -> bool {
        true
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        let want_focus = {
            let mut shared = self.shared.borrow_mut();
            shared.id.clone_from(&self.common.id);
            std::mem::take(&mut shared.want_focus)
        };
        if want_focus {
            self.controller_handle().focus();
        }
        if self.common.context_flyout.is_none() {
            let menu = self.context_menu();
            self.common.set_context_flyout(menu);
        }
        Vec::new()
    }

    fn handle(&mut self, event: &Event, cx: &mut EventCx) -> bool {
        TextEditor::handle(self, event, cx)
    }

    fn places(&self) -> bool {
        true
    }

    fn placed(&mut self, _rect: Rect, cx: &mut EventCx) -> bool {
        self.place(cx)
    }

    fn interactive(&self) -> bool {
        true
    }

    fn wants_scroll(&self) -> bool {
        true
    }

    fn captures_tab(&self) -> bool {
        true
    }

    fn focusable(&self) -> bool {
        !self.flags().1
    }

    fn enabled(&self) -> bool {
        !self.flags().1
    }

    fn text(&self) -> Option<String> {
        Some(self.controller_handle().value())
    }

    fn text_input(&self) -> Option<InputPurpose> {
        Some(InputPurpose::Normal)
    }
}

impl fmt::Debug for TextEditor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let shared = self.shared.borrow();
        f.debug_struct("TextEditor")
            .field("common", &self.common)
            .field("cursor", &shared.cursor)
            .field("anchor", &shared.anchor)
            .field("wrap", &self.wrap)
            .finish_non_exhaustive()
    }
}
