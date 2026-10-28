//! O widget [`Terminal`] e o visual que o `Drawer` pinta ([`TerminalVisual`]).

use std::any::Any;
use std::cell::{Cell, Ref, RefCell};
use std::fmt;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use tomni_ui_core::kurbo::{Point, Rect, Size, Vec2};
use tomni_ui_core::{
    Clipboard, ClipboardImage, Common, ContentDialog, Cursor, CustomLook, CustomVisual, Event,
    EventCx, FocusMove, FocusSource, Ime, InputPurpose, IntoProp, Key, Look, MenuFlyout, Modifiers,
    Prop, SCROLL_BAR_SIZE, SCROLL_LINE, ScrollAxis, ScrollBarState, ScrollMode, Scroller, Shortcut,
    Text, TextMetrics, Widget, clipboard,
};
use tomni_ui_reactive::{Get, Set, Signal, signal, untrack};
use tomni_ui_text::mono::{CellMetrics, cell_metrics};

use crate::emulator::{CursorShape, Emulator};
use crate::keys::encode_key;
use crate::links::{LinkHit, find_link};
use crate::mouse::{MouseAction, MouseButton, MouseEvent, encode_mouse, reports_mouse};
use crate::quote::{ShellQuote, shell_quote};
use crate::scheme::Scheme;
use crate::selection::{GridPoint, Selection, SelectionKind};
use crate::session::{Session, SessionCore};
use crate::transport::Transport;

/// Espaço entre a borda e a grade, o `padding` padrão.
pub const TERMINAL_PADDING: f64 = 8.0;
/// Tamanho da fonte padrão, em pixels (12 pt a 96 dpi).
pub const TERMINAL_FONT_SIZE: f32 = 16.0;
/// Menor tamanho de fonte que [`Terminal::font_size`] aplica: valores abaixo disto valem este.
pub const TERMINAL_FONT_MIN: f32 = 6.0;
/// Maior tamanho de fonte que [`Terminal::font_size`] aplica: valores acima disto valem este.
pub const TERMINAL_FONT_MAX: f32 = 72.0;
/// Pixels de roda por passo do zoom da fonte ([`Terminal::on_font_zoom`]): o de uma volta do
/// mouse. Um touchpad, que manda a roda em pixels aos poucos, soma até chegar a um passo.
pub const FONT_ZOOM_WHEEL: f64 = 48.0;
/// Colunas e linhas mínimas sem [`Terminal::cols_rows`]: o terminal cresce com `.grow(1)` ou
/// com `.width()` e `.height()`.
pub const TERMINAL_MIN_GRID: (u16, u16) = (20, 5);
/// Quanto dura o lampejo da campainha (BEL).
pub const BELL_FLASH: Duration = Duration::from_millis(150);
/// Tempo máximo de leitura por frame: o resto fica para o próximo, e a tela continua
/// respondendo com uma saída grande (um `cat` de 10 MB).
pub(crate) const READ_BUDGET: Duration = Duration::from_millis(8);

/// O que o clique direito faz num [`Terminal`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum RightClick {
    /// Abre o `.context_flyout()` do terminal. Sem menu, faz o mesmo que [`RightClick::CopyPaste`]
    /// (o padrão).
    #[default]
    Menu,
    /// O clique direito do PuTTY: com seleção, copia e limpa a seleção; sem ela, cola
    /// (passando pelo aviso de várias linhas). O `.context_flyout()` abre com Ctrl+clique
    /// direito, como no PuTTY, e com Shift+F10 e a tecla Menu.
    CopyPaste,
}

/// Como o terminal avisa a campainha (`BEL`, `Ctrl+G`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BellStyle {
    /// Pisca a tela por [`BELL_FLASH`] (o padrão).
    #[default]
    Flash,
    /// Não avisa.
    None,
}

/// Arrasto do polegar da barra de rolagem.
#[derive(Clone, Copy, Debug, PartialEq)]
struct BarDrag {
    /// Onde o ponteiro estava ao pressionar.
    from: f64,
    /// Linha do topo da vista ao pressionar, contando do começo do histórico.
    start: f64,
}

/// O que o botão pressionado está fazendo.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Press {
    /// Selecionando, a partir de `anchor`.
    Select {
        kind: SelectionKind,
        anchor: GridPoint,
    },
    /// Na barra de rolagem; `drag` quando pegou o polegar.
    Bar { drag: Option<BarDrag> },
    /// Rolando com o dedo, que estava em `y`.
    Pan { y: f64 },
}

type TitleHandler = Box<dyn FnMut(String)>;
/// O que o app faz com o Ctrl+roda (`Terminal::on_font_zoom`): os passos, positivos para aumentar.
type FontZoomHandler = Box<dyn FnMut(i32)>;
/// O que o app faz com um link aberto (`Terminal::on_link`).
type LinkHandler = Rc<RefCell<dyn FnMut(&LinkHit)>>;
/// O rótulo de abrir um link, ou `None` se ele não abre daqui (`Terminal::link_action`).
type LinkAction = Rc<dyn Fn(&LinkHit) -> Option<String>>;

/// Pedido do menu do terminal que só a próxima batida do relógio atende, porque quem tem a
/// seleção e a sessão é o widget.
enum Command {
    /// Copiar a seleção.
    Copy,
    /// Colar isto (já lido da área de transferência).
    Paste(Pasted),
}

/// O que a área de transferência tinha para colar.
enum Pasted {
    Text(String),
    Files(Vec<PathBuf>),
    Image(ClipboardImage),
}

/// Lê da área de transferência o que colar, na ordem: arquivos (se `rich`), texto e imagem
/// (se `rich`). Os gerenciadores de arquivos às vezes põem também o caminho como texto, e a
/// planilha e o editor de texto põem texto e imagem; em cada caso a pessoa quer o primeiro.
/// Com `only_rich`, o texto não é colado: devolve `None` quando ele existe.
fn read_pasted(clipboard: &mut dyn Clipboard, rich: bool, only_rich: bool) -> Option<Pasted> {
    if rich && let Some(files) = clipboard.read_files().filter(|files| !files.is_empty()) {
        return Some(Pasted::Files(files));
    }
    if let Some(text) = clipboard.read().filter(|text| !text.is_empty()) {
        return (!only_rich).then_some(Pasted::Text(text));
    }
    if rich {
        return clipboard.read_image().map(Pasted::Image);
    }
    None
}

/// O que chegou ao terminal além de texto.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RichPasteContent {
    /// Arquivos (e pastas) desta máquina, na ordem do sistema.
    Files(Vec<PathBuf>),
    /// Uma imagem sem arquivo (uma captura de tela).
    Image(ClipboardImage),
}

/// De onde veio uma [`RichPaste`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RichPasteSource {
    /// Ctrl+Shift+V, Shift+Insert, Cmd+V, o botão direito ou "Colar" do menu.
    Paste,
    /// O Ctrl+V sem Shift, com [`Terminal::rich_ctrl_v`].
    CtrlV,
    /// Arquivos soltos no terminal.
    Drop,
}

/// Uma colagem que o terminal não sabe resolver sozinho: imagem, ou arquivos.
#[derive(Clone, Debug)]
pub struct RichPaste {
    /// Quem recebe: a sessão do terminal e, com o envio para vários, as outras (na ordem do
    /// `TerminalPanes`).
    pub sessions: Vec<Session>,
    /// O que foi colado.
    pub content: RichPasteContent,
    /// De onde veio.
    pub source: RichPasteSource,
}

/// O que o terminal faz com a colagem rica.
type RichHandler = Rc<dyn Fn(RichPaste)>;

/// O link sob o ponteiro, como o `Drawer` o pinta.
#[derive(Clone, Debug, PartialEq)]
struct LinkMark {
    /// As células do link na vista, como (linha, coluna).
    cells: Vec<(usize, usize)>,
    /// Sublinhado contínuo (com o Ctrl apertado) em vez de pontilhado.
    solid: bool,
}

/// O link sob o ponteiro e o que a dica diz dele.
struct Hover {
    hit: LinkHit,
    ctrl: bool,
    /// A versão do emulador em que a busca foi feita.
    version: u64,
}

/// Terminal: mostra o que um programa escreve e manda
/// a ele o que se digita, pela conexão ([`Transport`]) dada.
///
/// ```
/// use tomni_ui_terminal::{Loopback, Terminal};
///
/// let terminal = Terminal::new(Loopback::new().echo()).id("console").grow(1);
/// ```
///
/// Os transportes prontos são o [`Loopback`](crate::Loopback) (testes e exemplos), o `Telnet`,
/// o `Process` (um programa local num pseudoterminal; feature `pty`), o `Ssh` (feature `ssh`)
/// e o `Serial` (feature `serial`).
///
/// Comportamento:
///
/// - **Teclado**: as teclas vão ao programa nos códigos do xterm, inclusive o Tab; o Ctrl+Tab
///   continua trocando de aba, e o **F6** (Shift+F6) leva o foco ao próximo (ao anterior)
///   widget. O IME compõe no lugar do cursor.
/// - **Seleção**: arrastar seleciona; duplo clique, a palavra; triplo clique, a linha; com o
///   Alt, um bloco. Ctrl+Shift+C (ou Ctrl+C com seleção, ou Ctrl+Insert) copia; Ctrl+Shift+V (ou
///   Shift+Insert) cola, e o clique direito copia a seleção ou, sem seleção, cola. Colar
///   várias linhas pede confirmação, exceto quando o programa ligou o modo de colar entre
///   marcas.
/// - **Rolagem**: 9001 linhas de histórico, pela barra, pela roda, com Shift+PageUp e
///   Shift+PageDown e com Ctrl+Shift+↑/↓/Home/End. A saída nova não puxa a vista de quem
///   rolou para cima; digitar volta ao fim. Na tela alternativa (vim, htop, less), a roda
///   manda setas, a menos que o programa desligue (`ESC[?1007l`).
/// - **Zoom da fonte**: com [`Terminal::on_font_zoom`], o Ctrl+roda chama o app com os passos
///   e não rola nem vai ao programa; o app devolve o tamanho por [`Terminal::font_size`].
/// - **Mouse do programa**: quando o programa pede o mouse (o relato do xterm: `ESC[?9h`,
///   `?1000h`, `?1002h` ou `?1003h`, nas formas padrão, `?1005h`, `?1006h` (SGR) e `?1015h`), a
///   roda, os três botões, o arrastar (1002) e o movimento (1003) vão a ele, com a célula sob
///   o ponteiro e o Shift, o Alt e o Ctrl. Enquanto isso, o clique não seleciona, o Ctrl+clique
///   não abre link e o clique direito não abre o menu nem cola. **Com o Shift pressionado, o
///   mouse volta ao terminal**: seleciona, rola o histórico, abre o menu e, com Ctrl+Shift,
///   abre o link. A barra de rolagem é sempre do terminal. Com `ESC[?1004h`, o programa sabe
///   quando o terminal ganha (`ESC [ I`) e perde (`ESC [ O`) o foco.
///
/// Sem [`Terminal::cols_rows`], a grade acompanha o tamanho do widget: use `.grow(1)` (ou
/// `.width()` e `.height()`), porque o tamanho natural é só o mínimo, de 20 × 5 células.
pub struct Terminal {
    common: Common,
    session: Session,
    core: Rc<RefCell<SessionCore>>,
    /// Se o terminal criou a sessão (`Terminal::new`) e a descarta ao sair.
    owned: bool,
    font_size: Prop<f64>,
    scheme: Prop<Scheme>,
    cursor_shape: Prop<CursorShape>,
    cursor_blink: Prop<bool>,
    /// O que as propriedades reativas já aplicaram (lido e escrito em `look`, que é `&self`).
    applied: Applied,
    fixed: Option<(u16, u16)>,
    /// Último `title_seq` e `bell` da sessão que o widget já tratou.
    seen_title: u64,
    seen_bell: u64,
    on_title: Option<TitleHandler>,
    /// Do app: o Ctrl+roda muda o tamanho da fonte (`on_font_zoom`).
    on_font_zoom: Option<FontZoomHandler>,
    /// Do `TerminalPanes`: o terminal ganhou (`true`) ou perdeu (`false`) o foco.
    on_focus: Option<Rc<dyn Fn(Session, bool)>>,
    /// Do `TerminalPanes`: se o terminal pede o foco ao montar.
    focus_mount: Option<Rc<dyn Fn(Session) -> bool>>,
    /// Do `TerminalPanes`: para quem vai a entrada desta sessão (o envio para vários).
    router: Option<Router>,
    copy_on_select: Prop<bool>,
    paste_warning: Prop<bool>,
    right_click: Prop<RightClick>,
    bell: Prop<BellStyle>,
    links: Prop<bool>,
    on_link: Option<LinkHandler>,
    link_action: Option<LinkAction>,
    /// O menu do app (`context_flyout`), que o terminal monta junto dos itens do link.
    app_menu: Option<MenuFlyout>,
    /// Se o app deu um menu (`app_menu` sai dele na montagem).
    has_app_menu: bool,
    /// Se o terminal põe "Copiar" e "Colar" antes dos itens do menu do app.
    app_menu_edit: bool,
    /// O link sob o ponteiro e o retângulo e o ponto do último movimento dentro do terminal.
    hover: Option<Hover>,
    pointer: Option<(Rect, Point)>,
    /// A dica do link sob o ponteiro (vazia sem link).
    link_tip: Signal<String>,
    /// O link sob o clique direito (ou sob o cursor do texto, no teclado) que abriu o menu.
    ctx_link: Signal<Option<LinkHit>>,
    commands: Rc<RefCell<Vec<Command>>>,
    focused: bool,
    focus_ring: bool,
    selection: Option<Selection>,
    press: Option<Press>,
    bar_hover: bool,
    /// Pixels de roda ainda sem formar uma linha (na vertical e na horizontal).
    wheel: i32,
    wheel_x: i32,
    /// Pixels do Ctrl+roda ainda sem formar um passo do zoom da fonte.
    wheel_zoom: i32,
    /// O botão que desceu indo ao programa (o relato do mouse) e ainda não subiu.
    held: Option<MouseButton>,
    /// A última célula (coluna e linha da tela) contada ao programa: o movimento só vai
    /// quando ela muda.
    mouse_cell: Option<(usize, usize)>,
    flash: Option<Duration>,
    preedit: Option<String>,
    dialog_open: Signal<bool>,
    /// A mensagem do aviso de colar: muda com o número de terminais que recebem.
    dialog_text: Signal<String>,
    pending_paste: Rc<RefCell<Option<String>>>,
    /// Do app: o que fazer com imagem e arquivos colados ou soltos (`on_rich_paste`).
    rich_paste: Option<RichHandler>,
    /// Do app: quando o Ctrl+clique direito fica com ele, e o que faz (`on_ctrl_secondary`).
    ctrl_secondary: Option<(Prop<bool>, Action)>,
    rich_ctrl_v: Prop<bool>,
    osc52: Prop<bool>,
    mounted: bool,
}

/// O estado que as propriedades reativas de [`Terminal`] (`font_size`, `scheme`,
/// `cursor_shape`) já aplicaram, e o último tamanho do widget (para refazer a grade quando a
/// fonte muda).
struct Applied {
    metrics: Cell<CellMetrics>,
    font_size: Cell<f32>,
    scheme: Cell<Scheme>,
    shape: Cell<CursorShape>,
    size: Cell<Option<Size>>,
}

/// Lê `prop` sem registrar dependência (eventos e quadros fora de um efeito).
fn now<T: Clone>(prop: &Prop<T>) -> T {
    untrack(|| prop.get())
}

/// Uma ação do app, sem argumentos.
type Action = Rc<dyn Fn()>;

/// Decide para quais sessões vai a entrada digitada numa sessão.
type Router = Rc<dyn Fn(Session) -> Vec<Session>>;

/// Os núcleos das sessões que recebem a entrada de `origin`: sem `router`, só o próprio.
fn route(
    router: Option<&Router>,
    origin: Session,
    own: &Rc<RefCell<SessionCore>>,
) -> Vec<Rc<RefCell<SessionCore>>> {
    let Some(router) = router else {
        return vec![Rc::clone(own)];
    };
    let cores: Vec<_> = router(origin)
        .into_iter()
        .filter_map(Session::core)
        .collect();
    if cores.is_empty() {
        vec![Rc::clone(own)]
    } else {
        cores
    }
}

/// O rótulo de abrir `hit`: o do `action` ou, sem ele, "Abrir link" para `http` e `https`.
fn label_of(action: Option<&LinkAction>, hit: &LinkHit) -> Option<String> {
    match action {
        Some(action) => untrack(|| action(hit)),
        None => {
            let scheme = hit.uri.split_once(':').map_or("", |(scheme, _)| scheme);
            (scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
                .then(|| tomni_ui_core::strings().terminal_link_open.to_string())
        }
    }
}

/// A largura, na fonte que `TextMetrics` mede, de uma linha da dica: a largura máxima da dica
/// (320 px) menos o espaço de dentro, e do tamanho do texto de campo (14) para o da dica (12).
const TIP_LINE_WIDTH: f64 = (320.0 - 18.0) * 14.0 / 12.0;

/// `uri` inteira se cabe numa linha da dica; senão, o começo e o fim com "…" no meio.
fn shorten(uri: &str, metrics: &mut dyn TextMetrics) -> String {
    let carets = metrics.carets(uri);
    let total = carets.last().map_or(0.0, |&(_, x)| x);
    if total <= TIP_LINE_WIDTH {
        return uri.to_owned();
    }
    let ellipsis = metrics.carets("…").last().map_or(0.0, |&(_, x)| x);
    let half = ((TIP_LINE_WIDTH - ellipsis) / 2.0).max(0.0);
    let head = carets
        .iter()
        .rev()
        .find(|&&(_, x)| x <= half)
        .map_or(0, |&(byte, _)| byte);
    let tail = carets
        .iter()
        .find(|&&(_, x)| total - x <= half)
        .map_or(uri.len(), |&(byte, _)| byte);
    if tail <= head {
        return uri.to_owned();
    }
    format!("{}…{}", &uri[..head], &uri[tail..])
}

impl Terminal {
    /// Terminal ligado a `transport`. Cria uma [`Session`] só dele e a descarta ao sair da
    /// árvore; para a sessão viver mais que o widget, use [`Terminal::session`].
    pub fn new(transport: impl Transport + 'static) -> Self {
        let mut terminal = Self::session(Session::new(transport));
        terminal.owned = true;
        terminal
    }

    /// Terminal que mostra `session`. A conexão, o histórico e o título ficam na sessão: o
    /// terminal pode sair da árvore e a sessão continua (e outro terminal pode mostrá-la).
    /// Mostre uma sessão em **um** terminal por vez.
    ///
    /// Se `session` já foi descartada ([`Session::dispose`]), o terminal mostra uma sessão
    /// encerrada e vazia.
    ///
    /// ```
    /// use tomni_ui_terminal::{Loopback, Session, Terminal};
    ///
    /// let sessao = Session::new(Loopback::new().echo()).key("web-01");
    /// let terminal = Terminal::session(sessao).id("web-01").grow(1);
    /// assert_eq!(terminal.session_ref(), sessao);
    /// ```
    pub fn session(session: Session) -> Self {
        let core = session.core().unwrap_or_else(|| {
            let mut dead = SessionCore::new(Box::new(crate::Loopback::new()), String::new());
            dead.closed = true;
            Rc::new(RefCell::new(dead))
        });
        let seen_bell = core.borrow().bell;
        Self {
            common: Common::default(),
            session,
            core,
            owned: false,
            font_size: Prop::Value(f64::from(TERMINAL_FONT_SIZE)),
            scheme: Prop::Value(Scheme::default()),
            cursor_shape: Prop::Value(CursorShape::default()),
            cursor_blink: Prop::Value(true),
            applied: Applied {
                metrics: Cell::new(cell_metrics(TERMINAL_FONT_SIZE)),
                font_size: Cell::new(TERMINAL_FONT_SIZE),
                scheme: Cell::new(Scheme::default()),
                shape: Cell::new(CursorShape::default()),
                size: Cell::new(None),
            },
            fixed: None,
            // O título que a sessão já tem chega ao `on_title` na montagem.
            seen_title: 0,
            seen_bell,
            on_title: None,
            on_font_zoom: None,
            on_focus: None,
            focus_mount: None,
            router: None,
            copy_on_select: Prop::Value(false),
            paste_warning: Prop::Value(true),
            right_click: Prop::Value(RightClick::default()),
            bell: Prop::Value(BellStyle::default()),
            links: Prop::Value(true),
            on_link: None,
            link_action: None,
            app_menu: None,
            has_app_menu: false,
            app_menu_edit: false,
            hover: None,
            pointer: None,
            link_tip: signal(String::new()),
            ctx_link: signal(None),
            commands: Rc::default(),
            focused: false,
            focus_ring: false,
            selection: None,
            press: None,
            bar_hover: false,
            wheel: 0,
            wheel_x: 0,
            wheel_zoom: 0,
            held: None,
            mouse_cell: None,
            flash: None,
            preedit: None,
            dialog_open: signal(false),
            dialog_text: signal(tomni_ui_core::strings().paste_warning.to_string()),
            pending_paste: Rc::default(),
            rich_paste: None,
            ctrl_secondary: None,
            rich_ctrl_v: Prop::Value(false),
            osc52: Prop::Value(true),
            mounted: false,
        }
    }

    /// A sessão que o terminal mostra (a que o [`Terminal::new`] criou, por exemplo).
    pub fn session_ref(&self) -> Session {
        self.session
    }

    /// Id estável, usado em testes, no MCP e no lint. A caixa de aviso de colar fica com o id
    /// `{id}-paste` (a mensagem é `{id}-paste-mensagem`, e os botões, `{id}-paste-primary` e
    /// `{id}-paste-close`).
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.common.id = Some(id.into());
        self
    }

    /// Ocupa o espaço do eixo principal do pai, como o `flex: n` do CSS.
    pub fn grow(mut self, grow: impl Into<f64>) -> Self {
        self.common.grow = grow.into();
        self
    }

    /// Largura fixa, em pixels.
    pub fn width(mut self, width: impl Into<f64>) -> Self {
        self.common.width = Some(width.into());
        self
    }

    /// Altura fixa, em pixels.
    pub fn height(mut self, height: impl Into<f64>) -> Self {
        self.common.height = Some(height.into());
        self
    }

    /// Mostra ou esconde o terminal (valor fixo, closure ou signal). Oculto, ele continua
    /// lendo a conexão.
    pub fn visible<M>(mut self, visible: impl IntoProp<bool, M>) -> Self {
        self.common.visible = Some(visible.into_prop());
        self
    }

    /// Dica (texto fixo, closure ou signal) com o ponteiro parado sobre o terminal.
    pub fn tooltip<M>(mut self, text: impl IntoProp<String, M>) -> Self {
        self.common.tooltip = Some(text.into_prop());
        self
    }

    /// Atalho `shortcut` que roda `action` enquanto o terminal aparece. Com o foco no
    /// terminal, os atalhos do app vêm antes dele: a tecla de um atalho não chega ao programa.
    pub fn accelerator<M>(
        mut self,
        shortcut: impl IntoProp<Shortcut, M>,
        action: impl FnMut() + 'static,
    ) -> Self {
        self.common.add_accelerator(shortcut, action);
        self
    }

    /// Menu do clique direito, de Shift+F10, da tecla Menu e do toque longo. Com ele, o clique
    /// direito abre o menu em vez de copiar ou colar (ponha "Copiar" e "Colar" no menu). Sobre
    /// um link, o menu ganha antes dos itens do app os do link ("Abrir…" e "Copiar link") e um
    /// separador.
    pub fn context_flyout(mut self, menu: MenuFlyout) -> Self {
        self.app_menu = Some(menu);
        self.has_app_menu = true;
        self
    }

    /// Com o [`Terminal::context_flyout`], o menu ganha "Copiar" (só com seleção) e "Colar" antes
    /// dos itens do app, e um separador depois. Eles fazem o mesmo que Ctrl+Shift+C e
    /// Ctrl+Shift+V: o "Colar" passa pelo aviso de várias linhas e pelo envio para vários
    /// terminais. Sem efeito sem `.context_flyout()`.
    pub fn context_copy_paste(mut self) -> Self {
        self.app_menu_edit = true;
        self
    }

    /// Chamado no Ctrl+clique (botão esquerdo, sem Alt) num link: o endereço que o programa
    /// marcou (`OSC 8`) ou uma URL `http://` ou `https://` do texto. O clique não seleciona e
    /// não vai ao programa. Sem handler, ou se [`Terminal::link_action`] devolve `None` para o
    /// link, o Ctrl+clique faz o que faria sem links. O `OSC 8` com outros esquemas (`file:`,
    /// `mailto:`) também chega aqui: quem decide o que abrir é o app.
    pub fn on_link(mut self, handler: impl FnMut(&LinkHit) + 'static) -> Self {
        self.on_link = Some(Rc::new(RefCell::new(handler)));
        self
    }

    /// Liga (padrão) ou desliga o uso dos links com o ponteiro: sublinhado, mão, dica e
    /// Ctrl+clique. Desligado, o terminal continua guardando os links do `OSC 8` e o menu do
    /// clique direito continua oferecendo "Copiar link". Aceita um signal.
    pub fn links<M>(mut self, on: impl IntoProp<bool, M>) -> Self {
        self.links = on.into_prop();
        self
    }

    /// O rótulo do item de abrir de cada link no menu e na dica ("Abrir no navegador"), ou
    /// `None` para um link que não abre daqui: o menu só oferece copiar e o Ctrl+clique não
    /// chama o [`Terminal::on_link`]. Sem `link_action`, vale "Abrir link" para `http` e
    /// `https`, e `None` para o resto.
    pub fn link_action(mut self, action: impl Fn(&LinkHit) -> Option<String> + 'static) -> Self {
        self.link_action = Some(Rc::new(action));
        self
    }

    /// Grade fixa de `cols` × `rows` células (o tamanho natural do widget). Sem isto, a grade
    /// acompanha o tamanho do widget.
    pub fn cols_rows(mut self, cols: u16, rows: u16) -> Self {
        let (cols, rows) = (cols.max(1), rows.max(1));
        self.fixed = Some((cols, rows));
        self.core
            .borrow_mut()
            .emulator
            .resize(usize::from(cols), usize::from(rows));
        self
    }

    /// Tamanho da fonte (Cascadia Mono), em pixels, de 6 a 72
    /// ([`TERMINAL_FONT_MIN`] e [`TERMINAL_FONT_MAX`]). O padrão é 16. Com uma closure ou um
    /// signal, a mudança vale na hora: a grade é medida de novo e a sessão recebe o tamanho
    /// novo (colunas e linhas novas, e o `SIGWINCH` no programa).
    pub fn font_size<M>(mut self, size: impl IntoProp<f64, M>) -> Self {
        self.font_size = size.into_prop();
        untrack(|| self.sync_props());
        self
    }

    /// Liga o zoom da fonte pela roda: com o **Ctrl** pressionado (e sem Shift nem Alt), a roda
    /// chama `handler` com o número de passos e **não** rola o histórico, não manda setas e não
    /// vai ao programa, mesmo que ele tenha pedido o mouse. Um passo é uma volta do mouse
    /// ([`FONT_ZOOM_WHEEL`] pixels): o touchpad, que manda a roda aos poucos, soma os pixels e
    /// só chama quando completa um passo. O sinal é do zoom: **positivo aumenta** (roda para
    /// cima), negativo diminui.
    ///
    /// O terminal não guarda o tamanho: o app soma os passos ao que quiser (uma configuração,
    /// um deslocamento por aba) e devolve o resultado por [`Terminal::font_size`], que refaz a
    /// grade. Os limites de 6 a 72 valem sempre. Sem este handler, o Ctrl+roda faz o que fazia.
    /// Não confundir com o zoom do painel do `TerminalPanes`, que amplia o painel.
    ///
    /// ```
    /// use tomni_ui_reactive::{Get, Set, signal};
    /// use tomni_ui_terminal::{Loopback, Terminal};
    ///
    /// let tamanho = signal(16.0_f64);
    /// let terminal = Terminal::new(Loopback::new())
    ///     .font_size(move || tamanho.get())
    ///     .on_font_zoom(move |passos| {
    ///         tamanho.set((tamanho.get() + f64::from(passos)).clamp(6.0, 72.0));
    ///     });
    /// ```
    pub fn on_font_zoom(mut self, handler: impl FnMut(i32) + 'static) -> Self {
        self.on_font_zoom = Some(Box::new(handler));
        self
    }

    /// Cores: o [`Scheme::Campbell`] (o padrão), as do tema
    /// ([`Scheme::Theme`]) ou próprias. Aceita um signal, e a mudança vale na hora.
    pub fn scheme<M>(mut self, scheme: impl IntoProp<Scheme, M>) -> Self {
        self.scheme = scheme.into_prop();
        untrack(|| self.sync_props());
        self
    }

    /// Linhas do histórico (9001 por padrão; 0 desliga). O que a
    /// sessão já tem na tela e no histórico fica: só o que passar do limite sai.
    pub fn scrollback(self, lines: usize) -> Self {
        self.core.borrow_mut().emulator.set_scrollback(lines);
        self
    }

    /// Forma do cursor até o programa pedir outra: barra (o padrão), bloco
    /// ou sublinhado. Aceita um signal: a mudança vale na hora, sem passar por cima de uma forma
    /// que o programa pediu (`DECSCUSR`).
    pub fn cursor_shape<M>(mut self, shape: impl IntoProp<CursorShape, M>) -> Self {
        self.cursor_shape = shape.into_prop();
        untrack(|| self.sync_props());
        self
    }

    /// Se o cursor pisca quando o programa deixa (padrão `true`). Com `false`, o cursor nunca
    /// pisca, mesmo que o programa peça (`ESC[?12h`, `DECSCUSR`): a escolha de quem usa vale
    /// mais que a do programa. Com `true`, quem decide é o programa, e o cursor pisca por
    /// padrão. Aceita um signal: a mudança vale na hora. Uma sessão encerrada nunca pisca: o
    /// cursor fica fixo e vazado (só o contorno da célula), e volta ao normal com o
    /// [`Session::restart`].
    pub fn cursor_blink<M>(mut self, blink: impl IntoProp<bool, M>) -> Self {
        self.cursor_blink = blink.into_prop();
        self
    }

    /// Aplica `font_size`, `scheme` e `cursor_shape`, lidos agora: o `look` chama dentro do
    /// efeito do widget, e a mudança de um signal chega por aqui.
    fn sync_props(&self) {
        let size = (self.font_size.get() as f32).clamp(TERMINAL_FONT_MIN, TERMINAL_FONT_MAX);
        if size != self.applied.font_size.get() {
            self.applied.font_size.set(size);
            self.applied.metrics.set(cell_metrics(size));
            // A fonte mudou e o widget não: a grade tem outro tamanho em colunas e linhas.
            if self.fixed.is_none()
                && let Some(area) = self.applied.size.get()
            {
                let (cols, rows) = fit(area, &self.applied.metrics.get());
                self.core.borrow_mut().resize(cols, rows);
            }
        }
        self.applied.scheme.set(self.scheme.get());
        let shape = self.cursor_shape.get();
        if shape != self.applied.shape.get() {
            self.applied.shape.set(shape);
            self.core
                .borrow_mut()
                .emulator
                .change_default_cursor_shape(shape);
        }
    }

    /// Chamado quando o programa muda o título (`OSC 0` e `OSC 2`), para pôr na aba ou na
    /// janela.
    pub fn on_title(mut self, handler: impl FnMut(String) + 'static) -> Self {
        self.on_title = Some(Box::new(handler));
        self
    }

    /// Avisa o `TerminalPanes` de quem tem o foco.
    pub(crate) fn on_focus(mut self, handler: Rc<dyn Fn(Session, bool)>) -> Self {
        self.on_focus = Some(handler);
        self
    }

    /// O `TerminalPanes` decide se o terminal pede o foco ao montar.
    pub(crate) fn focus_mount(mut self, decide: Rc<dyn Fn(Session) -> bool>) -> Self {
        self.focus_mount = Some(decide);
        self
    }

    /// O `TerminalPanes` diz para quem vai o que se digita neste terminal.
    pub(crate) fn router(mut self, router: Router) -> Self {
        self.router = Some(router);
        self
    }

    /// Os núcleos que recebem o que se digita aqui: este e, com o envio ligado, os outros.
    fn targets(&self) -> Vec<Rc<RefCell<SessionCore>>> {
        route(self.router.as_ref(), self.session, &self.core)
    }

    /// Chamado quando se cola uma imagem ou arquivos, ou quando se soltam arquivos no terminal.
    /// Quem trata decide o texto e cola com [`Session::paste`]; a colagem rica não passa pelos
    /// avisos de várias linhas nem de vários terminais. Sem este handler, arquivos soltos colam
    /// os caminhos locais entre aspas ([`shell_quote`]) e imagem não faz nada.
    ///
    /// Arquivos vêm antes do texto, e texto antes de imagem: só o que sobra na área de
    /// transferência chega aqui.
    pub fn on_rich_paste(mut self, handler: impl Fn(RichPaste) + 'static) -> Self {
        self.rich_paste = Some(Rc::new(handler));
        self
    }

    /// O Ctrl+clique direito fica com o app enquanto `when` for verdadeiro: `action` roda e o
    /// clique não abre o menu, não copia e não cola (o programa que pediu o mouse continua
    /// recebendo o clique, como antes). Com `when` falso, o Ctrl+clique direito faz o de sempre.
    /// É o "baixar o arquivo marcado" do WinSSHTerm. Aceita um signal.
    pub fn on_ctrl_secondary<M>(
        mut self,
        when: impl IntoProp<bool, M>,
        action: impl Fn() + 'static,
    ) -> Self {
        self.ctrl_secondary = Some((when.into_prop(), Rc::new(action)));
        self
    }

    /// Se o Ctrl+clique direito com `modifiers` vai ao [`Terminal::on_ctrl_secondary`].
    fn ctrl_secondary_taken(&self, modifiers: Modifiers) -> bool {
        modifiers.ctrl
            && self
                .ctrl_secondary
                .as_ref()
                .is_some_and(|(when, _)| now(when))
    }

    /// Com [`Terminal::on_rich_paste`], o Ctrl+V sem Shift também cola imagem e arquivos quando
    /// a área de transferência não tem texto; com texto, ele continua indo ao programa como
    /// `^V`. Desligado por padrão. Aceita um signal.
    pub fn rich_ctrl_v<M>(mut self, on: impl IntoProp<bool, M>) -> Self {
        self.rich_ctrl_v = on.into_prop();
        self
    }

    /// Se o programa pode pôr texto na área de transferência do sistema (`OSC 52`, o que o `tmux`,
    /// o `vim` e o `ssh` remoto usam para copiar). Ligado por padrão; o pedido de ler a área de
    /// transferência nunca é atendido. Cada pedido troca o anterior, e o texto que passa de 1 MiB é
    /// ignorado. Aceita um signal.
    pub fn osc52<M>(mut self, on: impl IntoProp<bool, M>) -> Self {
        self.osc52 = on.into_prop();
        self
    }

    /// Copia ao terminar de selecionar, sem o Ctrl+C.
    /// Aceita um signal.
    pub fn copy_on_select<M>(mut self, on: impl IntoProp<bool, M>) -> Self {
        self.copy_on_select = on.into_prop();
        self
    }

    /// Pede confirmação antes de colar texto com várias linhas (ligado por padrão).
    /// Aceita um signal.
    pub fn paste_warning<M>(mut self, on: impl IntoProp<bool, M>) -> Self {
        self.paste_warning = on.into_prop();
        self
    }

    /// O que o clique direito faz ([`RightClick::Menu`] por padrão). Aceita um signal.
    pub fn right_click<M>(mut self, action: impl IntoProp<RightClick, M>) -> Self {
        self.right_click = action.into_prop();
        self
    }

    /// Como a campainha (`BEL`) avisa ([`BellStyle::Flash`] por padrão). Aceita um signal.
    pub fn bell<M>(mut self, style: impl IntoProp<BellStyle, M>) -> Self {
        self.bell = style.into_prop();
        self
    }

    /// O emulador, para ler a grade, o cursor e o título.
    ///
    /// # Panics
    ///
    /// Se chamado de dentro de uma ação que muda o terminal.
    pub fn emulator(&self) -> Ref<'_, Emulator> {
        Ref::map(self.core.borrow(), |core| &core.emulator)
    }

    /// Interpreta `bytes` como se viessem do programa, na hora (o `feed` do `TestApp`). As
    /// respostas do terminal vão pela conexão.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.core.borrow_mut().feed(bytes);
        self.sync();
    }

    /// Lê tudo o que chegou pela conexão, sem o limite de tempo do frame. O app não precisa
    /// chamar: o terminal lê sozinho quando a conexão acorda a interface. É o que o `TestApp`
    /// faz a cada passo, para o eco aparecer na hora.
    pub fn poll(&mut self) {
        self.core.borrow_mut().pump(None);
        self.sync();
    }

    /// Se a conexão foi fechada.
    pub fn closed(&self) -> bool {
        self.core.borrow().closed
    }

    /// O visual atual.
    pub fn visual(&self) -> TerminalVisual {
        let core = self.core.borrow();
        let emulator = &core.emulator;
        let modes = emulator.modes();
        TerminalVisual {
            core: Rc::clone(&self.core),
            state: VisualState {
                version: emulator.version(),
                metrics: self.applied.metrics.get(),
                scheme: self.applied.scheme.get(),
                fixed: self.fixed,
                focused: self.focused,
                focus_ring: self.focus_ring,
                cursor_on: true,
                blink: self.cursor_blink.get() && modes.cursor_blink && !core.closed,
                closed: core.closed,
                selection: self.selection,
                preedit: self.preedit.clone(),
                flash: self.flash.is_some(),
                link: self.hover.as_ref().map(|hover| LinkMark {
                    cells: hover.hit.cells.clone(),
                    solid: hover.ctrl,
                }),
                bar: if matches!(self.press, Some(Press::Bar { .. })) {
                    ScrollBarState::Pressed
                } else if self.bar_hover {
                    ScrollBarState::Hover
                } else {
                    ScrollBarState::Rest
                },
            },
        }
    }

    /// Depois de a sessão receber bytes: avisa o título novo e acende a campainha.
    fn sync(&mut self) {
        let (title, bell) = {
            let core = self.core.borrow();
            (
                (core.title_seq != self.seen_title)
                    .then(|| (core.title_seq, core.emulator.title().to_owned())),
                core.bell != self.seen_bell,
            )
        };
        if let Some((seq, title)) = title {
            self.seen_title = seq;
            if let Some(handler) = &mut self.on_title {
                handler(title);
            }
        }
        let copies = std::mem::take(&mut self.core.borrow_mut().clipboard_out);
        if now(&self.osc52) {
            // Só o último vale: cada `OSC 52` troca o que o anterior pôs.
            if let Some(text) = copies.into_iter().next_back() {
                clipboard::write(text);
            }
        }
        if bell {
            self.seen_bell = self.core.borrow().bell;
            if now(&self.bell) == BellStyle::Flash {
                self.flash = Some(BELL_FLASH);
            }
        }
    }

    fn app_menu_exists(&self) -> bool {
        self.has_app_menu
    }

    /// Tira a seleção e devolve o texto dela, ou `None` sem seleção.
    fn take_selection_text(&mut self) -> Option<String> {
        let selection = self.selection.take()?;
        Some(selection.text(&self.core.borrow().emulator))
    }

    fn copy(&mut self, cx: &mut EventCx) -> bool {
        let Some(text) = self.take_selection_text() else {
            return false;
        };
        if !text.is_empty() {
            cx.clipboard().write(text);
        }
        true
    }

    fn paste(&mut self, cx: &mut EventCx) {
        let rich = self.rich_paste.is_some();
        if let Some(pasted) = read_pasted(cx.clipboard(), rich, false) {
            self.paste_pasted(pasted, RichPasteSource::Paste);
        }
    }

    /// O Ctrl+V sem Shift: só arquivos e imagem. Devolve `false` quando não há nada disso ou
    /// quando há texto, e então a tecla segue para o programa.
    fn paste_rich_only(&mut self, cx: &mut EventCx, source: RichPasteSource) -> bool {
        match read_pasted(cx.clipboard(), true, true) {
            Some(pasted) => {
                self.paste_pasted(pasted, source);
                true
            }
            None => false,
        }
    }

    fn paste_pasted(&mut self, pasted: Pasted, source: RichPasteSource) {
        match pasted {
            Pasted::Text(text) => self.paste_text(text),
            Pasted::Files(files) => self.paste_rich(RichPasteContent::Files(files), source),
            Pasted::Image(image) => self.paste_rich(RichPasteContent::Image(image), source),
        }
    }

    /// As sessões que recebem a colagem: a mesma lista de `targets`.
    fn target_sessions(&self) -> Vec<Session> {
        let sessions: Vec<Session> = match &self.router {
            Some(router) => router(self.session)
                .into_iter()
                .filter(|session| session.core().is_some())
                .collect(),
            None => Vec::new(),
        };
        if sessions.is_empty() {
            vec![self.session]
        } else {
            sessions
        }
    }

    /// Entrega ao app uma colagem de imagem ou arquivos. Sem handler, nada acontece.
    fn paste_rich(&mut self, content: RichPasteContent, source: RichPasteSource) {
        let Some(handler) = self.rich_paste.clone() else {
            return;
        };
        self.selection = None;
        handler(RichPaste {
            sessions: self.target_sessions(),
            content,
            source,
        });
    }

    /// Arquivos soltos no terminal: o app trata ou, sem handler, os caminhos locais são colados
    /// entre aspas, separados por espaço e com um espaço no fim.
    fn files_dropped(&mut self, paths: &[PathBuf]) {
        if paths.is_empty() {
            return;
        }
        if self.rich_paste.is_some() {
            self.paste_rich(
                RichPasteContent::Files(paths.to_vec()),
                RichPasteSource::Drop,
            );
            return;
        }
        let shell = if cfg!(windows) {
            ShellQuote::PowerShell
        } else {
            ShellQuote::Posix
        };
        let mut text = String::new();
        for path in paths {
            text.push_str(&shell_quote(&path.to_string_lossy(), shell));
            text.push(' ');
        }
        self.selection = None;
        for core in self.targets() {
            core.borrow_mut().paste(&text);
        }
    }

    /// Cola `text`, passando pelo aviso de várias linhas e pelo envio para vários.
    fn paste_text(&mut self, text: String) {
        if text.is_empty() {
            return;
        }
        self.selection = None;
        let targets = self.targets();
        let many = targets.len() > 1;
        let multiline = text.trim_end_matches(['\r', '\n']).contains(['\r', '\n']);
        let bracketed = self.core.borrow().emulator.modes().bracketed_paste;
        let ask = if many {
            true
        } else {
            multiline && now(&self.paste_warning) && !bracketed
        };
        if ask && self.mounted {
            let message = if many {
                tomni_ui_core::strings()
                    .paste_warning_many
                    .replace("{count}", &targets.len().to_string())
            } else {
                tomni_ui_core::strings().paste_warning.to_string()
            };
            self.dialog_text.set(message);
            *self.pending_paste.borrow_mut() = Some(text);
            self.dialog_open.set(true);
        } else {
            for core in targets {
                core.borrow_mut().paste(&text);
            }
        }
    }

    /// A célula da vista sob `point` (linha e coluna), ou `None` fora da grade: na borda, depois
    /// da última coluna ou na barra de rolagem.
    fn cell_at(&self, rect: Rect, point: Point) -> Option<(usize, usize)> {
        let origin = grid_origin(rect);
        let metrics = self.applied.metrics.get();
        let x = (point.x - origin.x) / f64::from(metrics.width);
        let y = (point.y - origin.y) / f64::from(metrics.height);
        if x < 0.0 || y < 0.0 {
            return None;
        }
        let core = self.core.borrow();
        let (col, row) = (x as usize, y as usize);
        (col < core.emulator.cols() && row < core.emulator.rows()).then_some((row, col))
    }

    /// O link sob `point` (o detectado só com `detect`).
    fn link_at(&self, rect: Rect, point: Point, detect: bool) -> Option<LinkHit> {
        let (row, col) = self.cell_at(rect, point)?;
        find_link(&self.core.borrow().emulator, row, col, detect)
    }

    /// O link sob o cursor do texto: o que a tecla Menu e o Shift+F10 oferecem no menu.
    fn link_at_text_cursor(&self) -> Option<LinkHit> {
        let core = self.core.borrow();
        let emulator = &core.emulator;
        let (row, col) = emulator.cursor();
        let row = row + emulator.display_offset();
        if row >= emulator.rows() {
            return None;
        }
        find_link(emulator, row, col, true)
    }

    /// O rótulo de abrir `hit`: o do app (`link_action`) ou, sem ele, "Abrir link" para `http`
    /// e `https`. `None` quando o link não abre daqui.
    fn link_label(&self, hit: &LinkHit) -> Option<String> {
        label_of(self.link_action.as_ref(), hit)
    }

    /// Se o Ctrl+clique em `hit` chama o `on_link`.
    fn link_opens(&self, hit: &LinkHit) -> bool {
        self.on_link.is_some() && self.link_label(hit).is_some()
    }

    /// Acha o link sob o ponteiro (ou nenhum, com `point` `None`) e atualiza o desenho, a mão e
    /// a dica. `metrics` mede o destino para cortá-lo na dica; sem ele (o relógio), a dica só
    /// fica se o link for o mesmo.
    fn refresh_hover(
        &mut self,
        at: Option<(Rect, Point)>,
        ctrl: bool,
        metrics: Option<&mut dyn TextMetrics>,
    ) {
        let enabled = now(&self.links) && self.press.is_none() && self.held.is_none();
        let found = at.filter(|_| enabled).and_then(|(rect, point)| {
            let (row, col) = self.cell_at(rect, point)?;
            let version = self.core.borrow().emulator.version();
            if let Some(hover) = &self.hover
                && hover.ctrl == ctrl
                && hover.version == version
                && hover.hit.cells.contains(&(row, col))
            {
                return Some(None);
            }
            let hit = find_link(&self.core.borrow().emulator, row, col, ctrl)?;
            Some(Some(Hover { hit, ctrl, version }))
        });
        match found {
            // O mesmo link, no mesmo estado: nada a refazer.
            Some(None) => {}
            Some(Some(hover)) => {
                let same = self
                    .hover
                    .as_ref()
                    .is_some_and(|old| old.hit.uri == hover.hit.uri);
                let tip = match metrics {
                    Some(metrics) => Some(self.tip_text(&hover.hit, metrics)),
                    None if same => None,
                    None => Some(String::new()),
                };
                if let Some(tip) = tip {
                    self.link_tip.set(tip);
                }
                self.hover = Some(hover);
            }
            None => {
                if self.hover.take().is_some() {
                    self.link_tip.set(String::new());
                }
            }
        }
    }

    /// O texto da dica de `hit`: o destino, cortado no meio se não couber, e o que o
    /// Ctrl+clique faz.
    fn tip_text(&self, hit: &LinkHit, metrics: &mut dyn TextMetrics) -> String {
        let strings = tomni_ui_core::strings();
        let action = match self.link_label(hit) {
            Some(label) if self.on_link.is_some() => {
                strings.terminal_link_hint.replace("{action}", &label)
            }
            _ => strings.terminal_link_no_open.to_string(),
        };
        format!("{}\n{action}", shorten(&hit.uri, metrics))
    }

    /// Apertar ou soltar o Ctrl com o ponteiro parado muda o sublinhado e a mão.
    fn refresh_modifiers(&mut self, cx: &mut EventCx) {
        let (at, ctrl) = (self.pointer, self.link_modifier(cx.modifiers()));
        self.refresh_hover(at, ctrl, Some(cx.metrics()));
    }

    /// Se o Ctrl de `mods` vale para os links: com o mouse indo ao programa, o Ctrl+clique é
    /// dele, e o link só abre com Ctrl+Shift.
    fn link_modifier(&self, mods: Modifiers) -> bool {
        mods.ctrl && (mods.shift || !self.reports_mouse())
    }

    /// Se o programa pediu o mouse (o relato do xterm, `ESC[?1000h` e os outros).
    fn reports_mouse(&self) -> bool {
        reports_mouse(&self.core.borrow().emulator.modes())
    }

    /// Se o mouse vai ao programa: ele pediu o relato e o Shift não está pressionado. Com o
    /// Shift, o mouse volta ao terminal (seleção, histórico e menu).
    fn mouse_to_program(&self, mods: Modifiers) -> bool {
        !mods.shift && self.reports_mouse()
    }

    /// Se `point` está na barra de rolagem do terminal em `rect`.
    fn on_bar(&self, rect: Rect, point: Point) -> bool {
        scroller(
            &self.core.borrow().emulator,
            &self.applied.metrics.get(),
            rect,
        )
        .bar(rect, ScrollAxis::Y)
        .is_some_and(|parts| parts.bar.contains(point))
    }

    /// A célula da tela sob `point` (coluna e linha), presa à grade: arrastando para fora,
    /// vale a borda.
    fn screen_cell(&self, rect: Rect, point: Point) -> (usize, usize) {
        let origin = grid_origin(rect);
        let metrics = self.applied.metrics.get();
        let core = self.core.borrow();
        let (cols, rows) = (core.emulator.cols(), core.emulator.rows());
        let col = ((point.x - origin.x) / f64::from(metrics.width))
            .floor()
            .clamp(0.0, (cols - 1) as f64) as usize;
        let row = ((point.y - origin.y) / f64::from(metrics.height))
            .floor()
            .clamp(0.0, (rows - 1) as f64) as usize;
        (col, row)
    }

    /// Conta ao programa um evento do mouse em `point`, na forma que ele pediu. Vai só a esta
    /// sessão, nunca ao envio para vários; o movimento só vai quando muda de célula.
    fn report_mouse(
        &mut self,
        button: MouseButton,
        action: MouseAction,
        rect: Rect,
        point: Point,
        mods: Modifiers,
    ) {
        let (col, row) = self.screen_cell(rect, point);
        if action == MouseAction::Motion && self.mouse_cell == Some((col, row)) {
            return;
        }
        self.mouse_cell = Some((col, row));
        let modes = self.core.borrow().emulator.modes();
        let event = MouseEvent {
            button,
            action,
            col,
            row,
            mods,
        };
        if let Some(bytes) = encode_mouse(event, &modes) {
            self.core.borrow_mut().write(&bytes);
        }
    }

    /// Um botão desceu: vai ao programa se o mouse é dele (e o ponteiro não está na barra de
    /// rolagem, que é sempre do terminal).
    fn report_press(&mut self, button: MouseButton, cx: &EventCx) -> bool {
        let (rect, point, mods) = (cx.rect(), cx.pointer(), cx.modifiers());
        if !self.mouse_to_program(mods) || self.on_bar(rect, point) {
            return false;
        }
        self.selection = None;
        self.held = Some(button);
        self.report_mouse(button, MouseAction::Press, rect, point, mods);
        true
    }

    /// Avisa o programa que pediu (`ESC[?1004h`) que o terminal ganhou ou perdeu o foco.
    fn report_focus(&mut self, focused: bool) {
        let wanted = self.core.borrow().emulator.modes().focus_events;
        if wanted {
            self.core
                .borrow_mut()
                .write(if focused { b"\x1b[I" } else { b"\x1b[O" });
        }
    }

    /// O botão `button` subiu: conta ao programa se a descida foi a ele.
    fn report_release(&mut self, button: MouseButton, cx: &EventCx) -> bool {
        if self.held != Some(button) {
            return false;
        }
        self.held = None;
        let (rect, point, mods) = (cx.rect(), cx.pointer(), cx.modifiers());
        self.report_mouse(button, MouseAction::Release, rect, point, mods);
        true
    }

    /// Atende o que o menu pediu.
    fn run_commands(&mut self) {
        let commands = std::mem::take(&mut *self.commands.borrow_mut());
        for command in commands {
            match command {
                Command::Copy => {
                    if let Some(text) = self.take_selection_text()
                        && !text.is_empty()
                    {
                        clipboard::write(text);
                    }
                }
                Command::Paste(pasted) => self.paste_pasted(pasted, RichPasteSource::Paste),
            }
        }
    }

    /// O menu de contexto: os itens do link (só com um link sob o clique), um separador e o menu
    /// do app ou, sem ele, "Copiar" e "Colar". Com [`Terminal::context_copy_paste`], "Copiar" e
    /// "Colar" vêm antes dos itens do app.
    fn build_menu(&mut self) -> MenuFlyout {
        let strings = tomni_ui_core::strings();
        let id = self
            .common
            .id
            .clone()
            .unwrap_or_else(|| "terminal".to_owned());
        let rich = self.rich_paste.is_some();
        let ctx = self.ctx_link;
        let has_link = move || ctx.get().is_some();
        let on_link = self.on_link.clone();
        let action = self.link_action.clone();
        let opens = {
            let (on_link, action) = (on_link.clone(), action.clone());
            move || {
                on_link.is_some()
                    && ctx
                        .get()
                        .is_some_and(|hit| label_of(action.as_ref(), &hit).is_some())
            }
        };
        let label = {
            let action = action.clone();
            move || {
                ctx.get()
                    .and_then(|hit| label_of(action.as_ref(), &hit))
                    .unwrap_or_default()
            }
        };
        let open = move || {
            if let Some(handler) = &on_link
                && let Some(hit) = untrack(|| ctx.get())
                && let Ok(mut handler) = handler.try_borrow_mut()
            {
                handler(&hit);
            }
        };
        let copy_link = move || {
            if let Some(hit) = untrack(|| ctx.get()) {
                clipboard::write(hit.uri);
            }
        };
        let head = MenuFlyout::new()
            .item(format!("{id}-link-abrir"), label, open)
            .item_visible(opens)
            .item(
                format!("{id}-link-copiar"),
                strings.terminal_link_copy.to_string(),
                copy_link,
            )
            .item_visible(has_link);
        match self.app_menu.take() {
            Some(menu) if self.app_menu_edit => {
                let session = self.session;
                let commands = Rc::clone(&self.commands);
                let for_paste = Rc::clone(&self.commands);
                let edit = head
                    .separator_visible(has_link)
                    .item(
                        format!("{id}-menu-copiar"),
                        strings.terminal_copy.to_string(),
                        move || commands.borrow_mut().push(Command::Copy),
                    )
                    .item_visible(move || session.has_selection())
                    .item(
                        format!("{id}-menu-colar"),
                        strings.terminal_paste.to_string(),
                        move || {
                            let commands = Rc::clone(&for_paste);
                            clipboard::with(move |clipboard| {
                                if let Some(pasted) = read_pasted(clipboard, rich, false) {
                                    commands.borrow_mut().push(Command::Paste(pasted));
                                }
                            });
                        },
                    )
                    .separator();
                menu.prepend(edit)
            }
            Some(menu) => menu.prepend(head.separator_visible(has_link)),
            None => {
                let commands = Rc::clone(&self.commands);
                let for_paste = Rc::clone(&self.commands);
                head.separator_visible(has_link)
                    .item(
                        format!("{id}-menu-copiar"),
                        strings.terminal_copy.to_string(),
                        move || commands.borrow_mut().push(Command::Copy),
                    )
                    .item_visible(has_link)
                    .item(
                        format!("{id}-menu-colar"),
                        strings.terminal_paste.to_string(),
                        move || {
                            let commands = Rc::clone(&for_paste);
                            clipboard::with(move |clipboard| {
                                if let Some(pasted) = read_pasted(clipboard, rich, false) {
                                    commands.borrow_mut().push(Command::Paste(pasted));
                                }
                            });
                        },
                    )
                    .item_visible(has_link)
                    .id(format!("{id}-menu"))
            }
        }
    }

    /// Rola a vista `lines` linhas (positivo sobe, para o histórico).
    fn scroll_lines(&mut self, lines: isize) {
        self.core.borrow_mut().emulator.scroll_display(lines);
    }

    fn rows(&self) -> usize {
        self.core.borrow().emulator.rows()
    }

    fn key_down(&mut self, key: Key, cx: &mut EventCx) -> bool {
        let mods = cx.modifiers();
        let apple = cfg!(any(target_os = "macos", target_os = "ios"));
        let ctrl_shift = mods.ctrl && mods.shift && !mods.alt;
        let cmd = apple && mods.meta;
        match key {
            Key::F(6) if !mods.ctrl && !mods.alt => {
                cx.move_focus(if mods.shift {
                    FocusMove::Backward
                } else {
                    FocusMove::Forward
                });
                return true;
            }
            Key::Char('c') if ctrl_shift || cmd => {
                self.copy(cx);
                return true;
            }
            Key::Char('c') if mods.ctrl && !mods.alt && self.selection.is_some() => {
                return self.copy(cx);
            }
            Key::Insert if mods.ctrl && !mods.shift => {
                self.copy(cx);
                return true;
            }
            Key::Char('v')
                if mods.ctrl
                    && !mods.shift
                    && !mods.alt
                    && !apple
                    && self.rich_paste.is_some()
                    && now(&self.rich_ctrl_v)
                    && self.paste_rich_only(cx, RichPasteSource::CtrlV) =>
            {
                return true;
            }
            Key::Char('v') if ctrl_shift || cmd => {
                self.paste(cx);
                return true;
            }
            Key::Insert if mods.shift && !mods.ctrl => {
                self.paste(cx);
                return true;
            }
            Key::PageUp | Key::PageDown if mods.shift && !mods.ctrl && !mods.alt => {
                let page = self.rows() as isize;
                self.scroll_lines(if key == Key::PageUp { page } else { -page });
                return true;
            }
            Key::Up if ctrl_shift => {
                self.scroll_lines(1);
                return true;
            }
            Key::Down if ctrl_shift => {
                self.scroll_lines(-1);
                return true;
            }
            Key::Home if ctrl_shift => {
                self.scroll_lines(isize::MAX / 2);
                return true;
            }
            Key::End if ctrl_shift => {
                self.core.borrow_mut().emulator.set_display_offset(0);
                return true;
            }
            _ => {}
        }
        if cmd {
            // Os atalhos com Cmd são do sistema, não do programa.
            return false;
        }
        let mut sent = false;
        for core in self.targets() {
            sent |= core.borrow_mut().key(key, mods);
        }
        if sent {
            self.selection = None;
        }
        sent
    }

    /// A célula sob `point`, para o terminal em `rect`: a linha absoluta e a coluna, presas à
    /// grade. Acima ou abaixo da grade, rola a vista uma linha (arrastando uma seleção).
    fn point_at(&mut self, rect: Rect, point: Point, autoscroll: bool) -> GridPoint {
        let origin = grid_origin(rect);
        let (cw, ch) = (
            f64::from(self.applied.metrics.get().width),
            f64::from(self.applied.metrics.get().height),
        );
        let mut core = self.core.borrow_mut();
        let (cols, rows) = (core.emulator.cols(), core.emulator.rows());
        let row = ((point.y - origin.y) / ch).floor();
        if autoscroll {
            if row < 0.0 {
                core.emulator.scroll_display(1);
            } else if row >= rows as f64 {
                core.emulator.scroll_display(-1);
            }
        }
        let row = row.clamp(0.0, (rows - 1) as f64) as usize;
        let col = ((point.x - origin.x) / cw)
            .floor()
            .clamp(0.0, (cols - 1) as f64) as usize;
        GridPoint::new(core.emulator.view_line(row), col)
    }

    fn pointer_down(&mut self, kind_touch: bool, clicks: u8, cx: &mut EventCx) -> bool {
        let (rect, point) = (cx.rect(), cx.pointer());
        let scroller = scroller(
            &self.core.borrow().emulator,
            &self.applied.metrics.get(),
            rect,
        );
        if let Some(parts) = scroller.bar(rect, ScrollAxis::Y)
            && parts.bar.contains(point)
        {
            let mut drag = None;
            if parts.decrease.contains(point) {
                self.scroll_lines(1);
            } else if parts.increase.contains(point) {
                self.scroll_lines(-1);
            } else if parts.thumb.contains(point) {
                drag = Some(BarDrag {
                    from: point.y,
                    start: scroller.offset().y,
                });
            } else if point.y < parts.thumb.y0 {
                let page = self.rows() as isize;
                self.scroll_lines(page);
            } else {
                let page = self.rows() as isize;
                self.scroll_lines(-page);
            }
            self.press = Some(Press::Bar { drag });
            return true;
        }
        if kind_touch {
            self.press = Some(Press::Pan { y: point.y });
            return true;
        }
        if self.report_press(MouseButton::Left, cx) {
            self.press = None;
            return true;
        }
        let mods = cx.modifiers();
        // Com o mouse indo ao programa, o Shift só devolve o clique ao terminal: o
        // Ctrl+Shift+clique abre o link.
        let shift = mods.shift && !self.reports_mouse();
        if mods.ctrl && !mods.alt && !shift && now(&self.links) {
            // Ctrl+clique num link: o app abre, e o clique não seleciona nem vai ao programa.
            if let Some(hit) = self.link_at(rect, point, true)
                && self.link_opens(&hit)
                && let Some(handler) = &self.on_link
            {
                let handler = Rc::clone(handler);
                self.selection = None;
                self.press = None;
                if let Ok(mut handler) = handler.try_borrow_mut() {
                    handler(&hit);
                }
                return true;
            }
        }
        let alt = mods.alt;
        let kind = match clicks {
            2 => SelectionKind::Word,
            n if n >= 3 => SelectionKind::Line,
            _ if alt => SelectionKind::Block,
            _ => SelectionKind::Simple,
        };
        let anchor = self.point_at(rect, point, false);
        self.selection = match kind {
            SelectionKind::Word | SelectionKind::Line => Some(Selection::new(kind, anchor)),
            _ => None,
        };
        self.press = Some(Press::Select { kind, anchor });
        true
    }

    fn pointer_move(&mut self, cx: &mut EventCx) {
        let (rect, point) = (cx.rect(), cx.pointer());
        if let Some(button) = self.held {
            // Arrastando com o botão no programa: vai com o 1002 e o 1003.
            self.report_mouse(button, MouseAction::Motion, rect, point, cx.modifiers());
            return;
        }
        match self.press {
            Some(Press::Select { kind, anchor }) => {
                let head = self.point_at(rect, point, true);
                if self.selection.is_none() && head == anchor {
                    return;
                }
                let selection = self
                    .selection
                    .get_or_insert_with(|| Selection::new(kind, anchor));
                selection.head = head;
            }
            Some(Press::Bar {
                drag: Some(BarDrag { from, start }),
            }) => {
                let scroller = scroller(
                    &self.core.borrow().emulator,
                    &self.applied.metrics.get(),
                    rect,
                );
                let Some(parts) = scroller.bar(rect, ScrollAxis::Y) else {
                    return;
                };
                if parts.travel <= 0.0 {
                    return;
                }
                let max = scroller.max_offset().y;
                let top = (start + (point.y - from) * max / parts.travel).clamp(0.0, max);
                let line = (top / f64::from(self.applied.metrics.get().height)).round() as usize;
                let mut core = self.core.borrow_mut();
                let history = core.emulator.history_len();
                core.emulator
                    .set_display_offset(history.saturating_sub(line));
            }
            Some(Press::Pan { y }) => {
                let ch = f64::from(self.applied.metrics.get().height);
                let lines = ((point.y - y) / ch).trunc();
                if lines != 0.0 {
                    self.scroll_lines(lines as isize);
                    self.press = Some(Press::Pan { y: y + lines * ch });
                }
            }
            _ => {
                let scroller = scroller(
                    &self.core.borrow().emulator,
                    &self.applied.metrics.get(),
                    rect,
                );
                self.bar_hover = scroller
                    .bar(rect, ScrollAxis::Y)
                    .is_some_and(|parts| parts.bar.contains(point));
                // Sem botão, o movimento sobre a grade vai ao programa com o 1003.
                let mods = cx.modifiers();
                if self.press.is_none()
                    && self.mouse_to_program(mods)
                    && self.cell_at(rect, point).is_some()
                {
                    self.report_mouse(MouseButton::None, MouseAction::Motion, rect, point, mods);
                }
            }
        }
    }

    fn pointer_up(&mut self, cx: &mut EventCx) {
        if self.report_release(MouseButton::Left, cx) {
            return;
        }
        let press = self.press.take();
        if matches!(press, Some(Press::Select { .. }))
            && now(&self.copy_on_select)
            && let Some(selection) = self.selection
        {
            let text = selection.text(&self.core.borrow().emulator);
            if !text.is_empty() {
                cx.clipboard().write(text);
            }
        }
    }

    /// A roda: `x` e `y` em pixels (positivo para a direita e para baixo).
    fn scroll(&mut self, x: i32, y: i32, cx: &EventCx) -> bool {
        let mods = cx.modifiers();
        // O Ctrl+roda é o zoom da fonte do app: não rola nem vai ao programa.
        if self.on_font_zoom.is_some() && mods.ctrl && !mods.shift && !mods.alt {
            // A roda para cima (`y` negativo) aumenta.
            let steps = -take_units(&mut self.wheel_zoom, y, FONT_ZOOM_WHEEL as i32);
            if steps != 0
                && let Some(handler) = self.on_font_zoom.as_mut()
            {
                handler(steps);
            }
            return true;
        }
        self.wheel_zoom = 0;
        if self.mouse_to_program(mods) {
            // O programa pediu o mouse: cada linha da roda é um clique dos botões 64 a 67.
            let lines_y = take_lines(&mut self.wheel, y);
            let lines_x = take_lines(&mut self.wheel_x, x);
            let (rect, point) = (cx.rect(), cx.pointer());
            for (lines, less, more) in [
                (lines_y, MouseButton::WheelUp, MouseButton::WheelDown),
                (lines_x, MouseButton::WheelLeft, MouseButton::WheelRight),
            ] {
                let button = if lines < 0 { less } else { more };
                for _ in 0..lines.unsigned_abs() {
                    self.report_mouse(button, MouseAction::Press, rect, point, mods);
                }
            }
            return true;
        }
        // O sistema pode virar a roda com Shift em rolagem horizontal.
        let y = if mods.shift && y == 0 { x } else { y };
        let lines = take_lines(&mut self.wheel, y);
        if lines == 0 {
            return true;
        }
        let (alt_screen, modes) = {
            let core = self.core.borrow();
            (core.emulator.alt_screen(), core.emulator.modes())
        };
        if alt_screen {
            // Na tela alternativa não há histórico: a roda vira setas (o "alternate scroll
            // mode" do xterm), a menos que o programa tenha desligado (`ESC[?1007l`).
            if !modes.alternate_scroll {
                return true;
            }
            let key = if lines > 0 { Key::Down } else { Key::Up };
            let mut bytes = Vec::new();
            for _ in 0..lines.unsigned_abs() {
                bytes.extend(encode_key(key, Default::default(), &modes).unwrap_or_default());
            }
            self.core.borrow_mut().write(&bytes);
        } else {
            self.scroll_lines(-(lines as isize));
        }
        true
    }
}

/// Tira de `wheel` as linhas inteiras que ele já soma com mais `delta` pixels.
fn take_lines(wheel: &mut i32, delta: i32) -> i32 {
    take_units(wheel, delta, SCROLL_LINE as i32)
}

/// Tira de `wheel` as unidades inteiras de `step` pixels que ele já soma com mais `delta`.
fn take_units(wheel: &mut i32, delta: i32, step: i32) -> i32 {
    *wheel += delta;
    let units = *wheel / step;
    *wheel -= units * step;
    units
}

impl Terminal {
    /// Passa a seleção à sessão, para [`Session::selection_text`] e o menu do app a verem.
    fn mirror_selection(&self) {
        let changed = self.core.borrow_mut().set_selection(self.selection);
        if let Some((signal, has)) = changed
            && untrack(|| signal.get()) != has
        {
            signal.set(has);
        }
    }

    fn handle_event(&mut self, event: &Event, cx: &mut EventCx) -> bool {
        match event {
            Event::FocusIn(source) => {
                self.focused = true;
                self.focus_ring = *source == FocusSource::Keyboard;
                self.report_focus(true);
                if let Some(on_focus) = &self.on_focus {
                    on_focus(self.session, true);
                }
                true
            }
            Event::FocusOut => {
                self.focused = false;
                self.focus_ring = false;
                self.preedit = None;
                self.report_focus(false);
                if let Some(on_focus) = &self.on_focus {
                    on_focus(self.session, false);
                }
                true
            }
            Event::KeyDown(key) => {
                self.focus_ring = self.focus_ring || *key == Key::Tab;
                if *key == Key::Other {
                    self.refresh_modifiers(cx);
                }
                self.key_down(*key, cx)
            }
            Event::KeyUp(Key::Other) => {
                self.refresh_modifiers(cx);
                false
            }
            Event::Text(text) => {
                let mods = cx.modifiers();
                if (mods.ctrl && !mods.alt) || text.chars().all(char::is_control) {
                    return false;
                }
                self.selection = None;
                for core in self.targets() {
                    core.borrow_mut().text(text, mods);
                }
                true
            }
            Event::Ime(Ime::Preedit { text, .. }) => {
                self.preedit = (!text.is_empty()).then(|| text.clone());
                true
            }
            Event::Ime(Ime::Commit(text)) => {
                self.preedit = None;
                self.selection = None;
                for core in self.targets() {
                    core.borrow_mut().commit(text);
                }
                true
            }
            Event::PointerDown { kind, clicks } => {
                self.focus_ring = false;
                self.pointer_down(*kind == tomni_ui_core::PointerKind::Touch, *clicks, cx)
            }
            Event::PointerMove { inside, .. } => {
                self.pointer_move(cx);
                self.pointer = inside.then(|| (cx.rect(), cx.pointer()));
                let (at, ctrl) = (self.pointer, self.link_modifier(cx.modifiers()));
                self.refresh_hover(at, ctrl, Some(cx.metrics()));
                true
            }
            Event::PointerUp { .. } => {
                self.pointer_up(cx);
                true
            }
            Event::PointerCancel => {
                self.press = None;
                self.held = None;
                self.bar_hover = false;
                self.pointer = None;
                self.refresh_hover(None, false, None);
                true
            }
            Event::SecondaryDown => {
                if self.report_press(MouseButton::Right, cx) {
                    return true;
                }
                if self.ctrl_secondary_taken(cx.modifiers())
                    && let Some((_, action)) = &self.ctrl_secondary
                {
                    action();
                    return true;
                }
                // Com seleção, o clique direito copia; sem ela, ou
                // com `copy_on_select` (que já copiou), cola. Com `.context_flyout()`, o clique
                // abre o menu e não chega aqui.
                if now(&self.copy_on_select) || !self.copy(cx) {
                    self.paste(cx);
                }
                true
            }
            Event::SecondaryUp => self.report_release(MouseButton::Right, cx),
            Event::MiddleDown => self.report_press(MouseButton::Middle, cx),
            Event::MiddleUp => self.report_release(MouseButton::Middle, cx),
            Event::Scroll { x, y } => self.scroll(*x, *y, cx),
            Event::FilesDropped { paths } => {
                self.files_dropped(paths);
                true
            }
            _ => false,
        }
    }
}

impl Widget for Terminal {
    fn kind(&self) -> &'static str {
        "terminal"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        self.sync_props();
        Look::Custom(CustomLook::new(self.visual()))
    }

    fn context_on_secondary(&self) -> bool {
        self.app_menu_exists() && now(&self.right_click) == RightClick::Menu
    }

    fn context_on_secondary_at(&self, rect: Rect, point: Point, modifiers: Modifiers) -> bool {
        // O clique direito do programa que pediu o mouse não abre o menu (com Shift, abre).
        if self.mouse_to_program(modifiers) && !self.on_bar(rect, point) {
            return false;
        }
        // O Ctrl+clique direito que o app pegou chega como `SecondaryDown`.
        if self.ctrl_secondary_taken(modifiers) {
            return false;
        }
        // No `RightClick::CopyPaste`, o Ctrl+clique direito abre o menu, como no PuTTY.
        self.link_at(rect, point, true).is_some()
            || self.context_on_secondary()
            || (modifiers.ctrl && self.app_menu_exists())
    }

    fn tip_anchor(&self, rect: Rect) -> Option<Rect> {
        let (_, point) = self.hover.as_ref().and(self.pointer)?;
        let origin = grid_origin(rect);
        let height = f64::from(self.applied.metrics.get().height);
        let row = ((point.y - origin.y) / height).floor();
        let top = origin.y + row * height;
        Some(Rect::new(point.x, top, point.x, top + height))
    }

    fn context_available(&self, rect: Rect, at: Option<Point>) -> bool {
        if self.app_menu_exists() {
            return true;
        }
        match at {
            Some(point) => self.link_at(rect, point, true).is_some(),
            None => self.link_at_text_cursor().is_some(),
        }
    }

    fn context_opening_at(&mut self, rect: Rect, at: Option<Point>) {
        let hit = match at {
            Some(point) => self.link_at(rect, point, true),
            None => self.link_at_text_cursor(),
        };
        self.ctx_link.set(hit);
    }

    fn cursor(&self) -> Option<Cursor> {
        let hover = self.hover.as_ref()?;
        (hover.ctrl && self.link_opens(&hover.hit)).then_some(Cursor::Hand)
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        self.mounted = true;
        self.sync();
        let id = self
            .common
            .id
            .as_deref()
            .map_or_else(|| "terminal-paste".to_owned(), |id| format!("{id}-paste"));
        let core = Rc::clone(&self.core);
        let router = self.router.clone();
        let session = self.session;
        let pending = Rc::clone(&self.pending_paste);
        let menu = self.build_menu();
        self.common.set_context_flyout(menu);
        // A dica do link vem antes da do app, que continua valendo fora dos links.
        let link_tip = self.link_tip;
        let own = self.common.tooltip.take();
        self.common.tooltip = Some(
            (move || {
                let tip = link_tip.get();
                match &own {
                    Some(own) if tip.is_empty() => own.get(),
                    _ => tip,
                }
            })
            .into_prop(),
        );
        let strings = tomni_ui_core::strings();
        let dialog = ContentDialog::new(strings.paste_title.to_string(), self.dialog_open)
            .content(
                Text::new(self.dialog_text)
                    .id(format!("{id}-mensagem"))
                    .wrap(),
            )
            .id(id)
            .primary(strings.paste_anyway.to_string(), move || {
                if let Some(text) = pending.borrow_mut().take() {
                    for core in route(router.as_ref(), session, &core) {
                        core.borrow_mut().paste(&text);
                    }
                }
            })
            .close(strings.cancel.to_string());
        vec![Box::new(dialog)]
    }

    fn handle(&mut self, event: &Event, cx: &mut EventCx) -> bool {
        let handled = self.handle_event(event, cx);
        self.mirror_selection();
        handled
    }

    fn interactive(&self) -> bool {
        true
    }

    fn focusable(&self) -> bool {
        true
    }

    fn accelerators_first(&self) -> bool {
        true
    }

    fn text(&self) -> Option<String> {
        Some(self.core.borrow().emulator.screen_text())
    }

    fn text_input(&self) -> Option<InputPurpose> {
        Some(InputPurpose::Normal)
    }

    fn animating(&self) -> bool {
        true
    }

    fn ticking(&self) -> bool {
        if self.flash.is_some() || !self.commands.borrow().is_empty() {
            return true;
        }
        self.core.borrow().needs_poll()
    }

    fn tick(&mut self, dt: Duration) {
        let _terminal = tomni_ui_core::stages::stage("terminal");
        self.flash = self
            .flash
            .and_then(|left| left.checked_sub(dt))
            .filter(|left| !left.is_zero());
        self.run_commands();
        self.core.borrow_mut().pump(Some(READ_BUDGET));
        self.sync();
        // A saída nova e a roda mexem no que está sob um ponteiro parado.
        if self.pointer.is_some() {
            let (at, ctrl) = (self.pointer, self.hover.as_ref().is_some_and(|h| h.ctrl));
            self.refresh_hover(at, ctrl, None);
        }
    }

    fn resized(&mut self, size: Size) {
        // Um painel escondido mede 0 × 0: a sessão guarda o último tamanho.
        if size.width <= 0.0 || size.height <= 0.0 {
            return;
        }
        self.applied.size.set(Some(size));
        let (cols, rows) = self
            .fixed
            .unwrap_or_else(|| fit(size, &self.applied.metrics.get()));
        self.core.borrow_mut().resize(cols, rows);
    }

    fn focus_on_mount(&self) -> bool {
        self.focus_mount
            .as_ref()
            .is_some_and(|decide| decide(self.session))
    }

    fn captures_tab(&self) -> bool {
        true
    }

    fn wants_scroll(&self) -> bool {
        true
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        if self.owned {
            self.session.dispose();
        }
    }
}

impl fmt::Debug for Terminal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Terminal")
            .field("common", &self.common)
            .field("emulator", &self.core.borrow().emulator)
            .field("scheme", &self.applied.scheme.get())
            .finish_non_exhaustive()
    }
}

/// Canto superior esquerdo da grade, para o terminal em `rect`.
pub fn grid_origin(rect: Rect) -> Point {
    Point::new(rect.x0 + TERMINAL_PADDING, rect.y0 + TERMINAL_PADDING)
}

/// Colunas e linhas que cabem em `size`, com o espaço da borda e da barra de rolagem.
fn fit(size: Size, metrics: &CellMetrics) -> (u16, u16) {
    let width = size.width - 2.0 * TERMINAL_PADDING - SCROLL_BAR_SIZE;
    let height = size.height - 2.0 * TERMINAL_PADDING;
    let cols = (width / f64::from(metrics.width))
        .floor()
        .clamp(1.0, 2000.0) as u16;
    let rows = (height / f64::from(metrics.height))
        .floor()
        .clamp(1.0, 1000.0) as u16;
    (cols, rows)
}

/// A rolagem do histórico como um [`Scroller`], para a barra: o conteúdo é o histórico e a
/// tela, e a vista, as linhas da tela.
fn scroller(emulator: &Emulator, metrics: &CellMetrics, rect: Rect) -> Scroller {
    let ch = f64::from(metrics.height);
    let rows = emulator.rows() as f64;
    let history = emulator.history_len() as f64;
    let mut scroller = Scroller::new(ScrollMode::Vertical);
    scroller.set_extent(
        Size::new(rect.width(), (history + rows) * ch),
        Size::new(rect.width(), rows * ch),
    );
    scroller.scroll_to(Vec2::new(
        0.0,
        (history - emulator.display_offset() as f64) * ch,
    ));
    scroller
}

/// O que muda o desenho, fora a grade (que muda com a `version`).
#[derive(Clone, Debug, PartialEq)]
struct VisualState {
    version: u64,
    metrics: CellMetrics,
    scheme: Scheme,
    fixed: Option<(u16, u16)>,
    focused: bool,
    focus_ring: bool,
    cursor_on: bool,
    blink: bool,
    closed: bool,
    selection: Option<Selection>,
    preedit: Option<String>,
    flash: bool,
    link: Option<LinkMark>,
    bar: ScrollBarState,
}

/// O que o `Drawer` precisa para pintar um [`Terminal`]: a grade (lida do emulador
/// compartilhado), as cores, o cursor, a seleção e a barra de rolagem.
#[derive(Clone)]
pub struct TerminalVisual {
    core: Rc<RefCell<SessionCore>>,
    state: VisualState,
}

impl TerminalVisual {
    /// O emulador: a grade, o cursor, os modos e o título.
    ///
    /// # Panics
    ///
    /// Se o terminal estiver lendo da conexão (não acontece durante a pintura).
    pub fn emulator(&self) -> Ref<'_, Emulator> {
        Ref::map(self.core.borrow(), |core| &core.emulator)
    }

    /// Soma `elapsed` ao tempo de interface da sessão ([`crate::Session::busy`]): o `Drawer`
    /// chama depois de pintar o terminal.
    ///
    /// # Panics
    ///
    /// Se o emulador ainda estiver emprestado ([`TerminalVisual::emulator`]).
    pub fn add_busy(&self, elapsed: Duration) {
        self.core.borrow_mut().busy += elapsed;
    }

    /// Medidas da célula.
    pub fn metrics(&self) -> &CellMetrics {
        &self.state.metrics
    }

    /// Esquema de cores.
    pub fn scheme(&self) -> Scheme {
        self.state.scheme
    }

    /// Se tem o foco: sem ele, o cursor é só o contorno da célula.
    pub fn focused(&self) -> bool {
        self.state.focused
    }

    /// Se a sessão foi encerrada: o cursor fica fixo e vazado, como sem foco.
    pub fn closed(&self) -> bool {
        self.state.closed
    }

    /// Se mostra o anel de foco (foco vindo do teclado).
    pub fn focus_ring(&self) -> bool {
        self.state.focus_ring
    }

    /// A seleção.
    pub fn selection(&self) -> Option<&Selection> {
        self.state.selection.as_ref()
    }

    /// O texto em composição no IME, desenhado sublinhado no cursor.
    pub fn preedit(&self) -> Option<&str> {
        self.state.preedit.as_deref()
    }

    /// Se a campainha está acesa.
    pub fn flash(&self) -> bool {
        self.state.flash
    }

    /// As células do link sob o ponteiro, como (linha da vista, coluna), e se o sublinhado é
    /// contínuo (com o Ctrl apertado) em vez de pontilhado.
    pub fn link(&self) -> Option<(&[(usize, usize)], bool)> {
        self.state
            .link
            .as_ref()
            .map(|mark| (mark.cells.as_slice(), mark.solid))
    }

    /// Estado da barra de rolagem.
    pub fn bar_state(&self) -> ScrollBarState {
        self.state.bar
    }

    /// A rolagem do histórico, para desenhar a barra em `rect` com [`Scroller::bar`].
    pub fn scroller(&self, rect: Rect) -> Scroller {
        scroller(&self.emulator(), &self.state.metrics, rect)
    }

    /// Onde o cursor aparece, na vista (linha e coluna) e com que forma; `None` se o programa
    /// o escondeu, se a vista rolou para longe dele ou na fase apagada do pisca.
    pub fn cursor(&self) -> Option<(usize, usize, CursorShape)> {
        if !self.state.cursor_on {
            return None;
        }
        let emulator = self.emulator();
        if !emulator.modes().cursor_visible {
            return None;
        }
        let (row, col) = emulator.cursor();
        let row = row + emulator.display_offset();
        (row < emulator.rows()).then_some((row, col, emulator.cursor_shape()))
    }

    /// Tamanho natural: a grade fixa ou o mínimo, mais a borda e a barra.
    pub fn natural_size(&self) -> Size {
        let (cols, rows) = self.state.fixed.unwrap_or(TERMINAL_MIN_GRID);
        let m = &self.state.metrics;
        Size::new(
            f64::from(cols) * f64::from(m.width) + 2.0 * TERMINAL_PADDING + SCROLL_BAR_SIZE,
            f64::from(rows) * f64::from(m.height) + 2.0 * TERMINAL_PADDING,
        )
    }

    /// Retângulo da célula na linha `row` da vista e coluna `col`, de `cells` de largura.
    pub fn cell_rect(&self, rect: Rect, row: usize, col: usize, cells: usize) -> Rect {
        let origin = grid_origin(rect);
        let (cw, ch) = (
            f64::from(self.state.metrics.width),
            f64::from(self.state.metrics.height),
        );
        let x = origin.x + col as f64 * cw;
        let y = origin.y + row as f64 * ch;
        Rect::new(x, y, x + cells as f64 * cw, y + ch)
    }
}

impl fmt::Debug for TerminalVisual {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TerminalVisual")
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl CustomVisual for TerminalVisual {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn eq_visual(&self, other: &dyn CustomVisual) -> bool {
        other
            .as_any()
            .downcast_ref::<Self>()
            .is_some_and(|other| Rc::ptr_eq(&self.core, &other.core) && self.state == other.state)
    }

    fn hide_caret(&self) -> Option<CustomLook> {
        (self.state.focused && self.state.blink && self.state.preedit.is_none()).then(|| {
            let mut hidden = self.clone();
            hidden.state.cursor_on = false;
            CustomLook::new(hidden)
        })
    }
}
