//! O emulador: interpreta os bytes do programa (com o `vte`) e mantém a grade, o cursor e os
//! modos. Cobre o VT100, o VT220 e o que o xterm tem de mais usado, com os links do OSC 8 e os
//! modos do relato do mouse ([`MouseTracking`], [`MouseEncoding`]; quem escreve os eventos é o
//! widget); o sixel, o bidi e as linhas de altura dupla ficam de fora.

use std::collections::HashMap;
use std::num::NonZeroU32;

use unicode_width::UnicodeWidthChar;
use vte::{Params, Parser, Perform};

use crate::cell::{Cell, Color, Flags, Pen};
use crate::grid::{Grid, Row};

/// Linhas do histórico por padrão.
pub const DEFAULT_SCROLLBACK: usize = 9001;

/// O progresso que o programa informou pelo `OSC 9;4` (o de `ESC ] 9 ; 4 ; estado ; valor ST`).
///
/// O valor, quando existe, vai de 0 a 100.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TermProgress {
    /// Sem progresso (o estado 0, o RIS e a sessão fechada).
    #[default]
    None,
    /// Progresso normal, em porcentagem (o estado 1).
    Normal(u8),
    /// Erro (o estado 2); sem valor, a porcentagem não se sabe.
    Error(Option<u8>),
    /// Trabalhando, sem porcentagem (o estado 3).
    Indeterminate,
    /// Pausado, um aviso (o estado 4); sem valor, a porcentagem não se sabe.
    Paused(Option<u8>),
}

impl TermProgress {
    /// A porcentagem, quando o estado tem uma.
    pub fn percent(self) -> Option<u8> {
        match self {
            Self::Normal(value) => Some(value),
            Self::Error(value) | Self::Paused(value) => value,
            Self::None | Self::Indeterminate => None,
        }
    }

    /// Lê os parâmetros que vêm depois do `4` do `OSC 9;4` (o estado e o valor), sabendo o que
    /// valia antes. `None` quando a sequência é malformada e deve ser ignorada.
    ///
    /// O estado ausente vale 0 (remove). O valor ausente, vazio ou que não é número vale 0 no
    /// estado 1 e mantém a porcentagem anterior nos estados 2 e 4; um número fora de 0 a 100 é
    /// limitado a ele.
    fn parse(params: &[&[u8]], before: Self) -> Option<Self> {
        let state = match params.first().copied() {
            None | Some(b"") => 0,
            Some(digits) => parse_integer(digits).filter(|n| (0..=4).contains(n))?,
        };
        let value = params
            .get(1)
            .copied()
            .and_then(parse_integer)
            .map(|n| n.clamp(0, 100) as u8);
        let kept = value.or(before.percent());
        Some(match state {
            0 => Self::None,
            1 => Self::Normal(value.unwrap_or(0)),
            2 => Self::Error(kept),
            3 => Self::Indeterminate,
            _ => Self::Paused(kept),
        })
    }
}

/// Um inteiro em ASCII, com sinal opcional. Um número grande demais para `i64` fica no extremo
/// dele, que o chamador limita. `None` se não é um número.
fn parse_integer(bytes: &[u8]) -> Option<i64> {
    let (negative, digits) = match bytes {
        [b'-', rest @ ..] => (true, rest),
        [b'+', rest @ ..] => (false, rest),
        _ => (false, bytes),
    };
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let magnitude = digits.iter().fold(0_i64, |acc, d| {
        acc.saturating_mul(10).saturating_add(i64::from(d - b'0'))
    });
    Some(if negative { -magnitude } else { magnitude })
}

/// Um aviso que o programa pediu por `OSC 9;texto` ou `OSC 777;notify;título;corpo`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Notice {
    /// O título, quando o programa deu um (só o `OSC 777`).
    pub title: Option<String>,
    /// O texto do aviso.
    pub body: String,
}

/// Forma do cursor (`ESC[n q`, o DECSCUSR).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CursorShape {
    /// Barra vertical.
    #[default]
    Bar,
    /// Bloco que cobre a célula.
    Block,
    /// Sublinhado.
    Underline,
}

/// Conjunto de caracteres de G0 ou G1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Charset {
    #[default]
    Ascii,
    /// Os desenhos de linha do VT100 (`ESC ( 0`).
    DecSpecial,
}

impl Charset {
    fn map(self, c: char) -> char {
        if self == Charset::Ascii {
            return c;
        }
        match c {
            '_' => ' ',
            '`' => '◆',
            'a' => '▒',
            'b' => '␉',
            'c' => '␌',
            'd' => '␍',
            'e' => '␊',
            'f' => '°',
            'g' => '±',
            'h' => '␤',
            'i' => '␋',
            'j' => '┘',
            'k' => '┐',
            'l' => '┌',
            'm' => '└',
            'n' => '┼',
            'o' => '⎺',
            'p' => '⎻',
            'q' => '─',
            'r' => '⎼',
            's' => '⎽',
            't' => '├',
            'u' => '┤',
            'v' => '┴',
            'w' => '┬',
            'x' => '│',
            'y' => '≤',
            'z' => '≥',
            '{' => 'π',
            '|' => '≠',
            '}' => '£',
            '~' => '·',
            c => c,
        }
    }
}

/// Posição e estado do cursor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Cursor {
    row: usize,
    col: usize,
    pen: Pen,
    /// O caractere foi escrito na última coluna: o próximo quebra a linha antes.
    pending_wrap: bool,
    charsets: [Charset; 2],
    /// Qual dos dois está em uso (SO e SI).
    shift: usize,
}

/// O que o DECSC guarda.
#[derive(Clone, Copy, Debug, Default)]
struct Saved {
    cursor: Cursor,
    origin: bool,
}

/// Modos ligados e desligados pelo programa.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Modes {
    /// Setas no modo aplicação (DECCKM, `ESC[?1h`): `ESC O A` em vez de `ESC [ A`.
    pub app_cursor: bool,
    /// Teclado numérico no modo aplicação (DECKPAM, `ESC =`).
    pub app_keypad: bool,
    /// Colar entre `ESC[200~` e `ESC[201~` (`ESC[?2004h`).
    pub bracketed_paste: bool,
    /// Cores da tela invertidas (DECSCNM, `ESC[?5h`).
    pub reverse_video: bool,
    /// Cursor visível (DECTCEM, `ESC[?25h`).
    pub cursor_visible: bool,
    /// Cursor piscando (`ESC[?12h` e o DECSCUSR).
    pub cursor_blink: bool,
    /// Quebra automática na última coluna (DECAWM).
    pub autowrap: bool,
    /// Posições relativas à região de rolagem (DECOM).
    pub origin: bool,
    /// Inserir em vez de sobrescrever (IRM, `ESC[4h`).
    pub insert: bool,
    /// O LF também volta à coluna 0 (LNM, `ESC[20h`).
    pub newline: bool,
    /// Que eventos do mouse o programa pediu (`ESC[?9h`, `?1000h`, `?1002h`, `?1003h`).
    pub mouse: MouseTracking,
    /// Como escrever as posições do mouse (`ESC[?1005h`, `?1006h`, `?1015h`).
    pub mouse_encoding: MouseEncoding,
    /// Na tela alternativa, sem o mouse pedido, a roda manda setas (`ESC[?1007h`, o padrão).
    pub alternate_scroll: bool,
    /// Avisar o programa quando o terminal ganha e perde o foco (`ESC[?1004h`): `ESC [ I` e
    /// `ESC [ O`.
    pub focus_events: bool,
}

/// Que eventos do mouse o programa recebe (o relato do mouse do xterm).
///
/// Os modos se excluem: ligar um troca o anterior, e desligar qualquer um desliga o relato.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MouseTracking {
    /// O mouse é do terminal: seleção, rolagem e menu.
    #[default]
    Off,
    /// Só o pressionar dos botões, sem as teclas modificadoras (X10, `ESC[?9h`).
    X10,
    /// Pressionar e soltar dos botões e a roda (`ESC[?1000h`).
    Buttons,
    /// Também o arrastar com um botão pressionado (`ESC[?1002h`).
    Drag,
    /// Também o movimento sem botão nenhum (`ESC[?1003h`).
    Motion,
}

/// Como as posições do mouse vão ao programa.
///
/// Os modos se excluem: ligar um troca o anterior, e desligar só vale para o que está ligado.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MouseEncoding {
    /// `ESC [ M` e três bytes (botão, coluna e linha somados a 32): só até a coluna e a linha
    /// 223; além disso o evento não vai.
    #[default]
    Default,
    /// Como o `Default`, com a coluna e a linha em UTF-8 (`ESC[?1005h`): até 2015.
    Utf8,
    /// `ESC [ < botão ; coluna ; linha M` (ou `m` ao soltar), sem limite (`ESC[?1006h`).
    Sgr,
    /// `ESC [ botão+32 ; coluna ; linha M`, sem limite (`ESC[?1015h`).
    Urxvt,
}

impl Default for Modes {
    fn default() -> Self {
        Self {
            app_cursor: false,
            app_keypad: false,
            bracketed_paste: false,
            reverse_video: false,
            cursor_visible: true,
            cursor_blink: true,
            autowrap: true,
            origin: false,
            insert: false,
            newline: false,
            mouse: MouseTracking::Off,
            mouse_encoding: MouseEncoding::Default,
            alternate_scroll: true,
            focus_events: false,
        }
    }
}

/// Um link marcado pelo programa com o `OSC 8`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hyperlink {
    uri: String,
    id: Option<String>,
}

impl Hyperlink {
    /// O destino, inteiro, como o programa escreveu.
    pub fn uri(&self) -> &str {
        &self.uri
    }

    /// O `id` que o programa deu ao link, se deu: trechos com o mesmo `id` e a mesma URI são
    /// um só link.
    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }
}

/// A tabela de links do `OSC 8`: as células guardam só o índice mais um.
#[derive(Debug, Default)]
struct Links {
    entries: Vec<Hyperlink>,
    /// O índice de cada par (id, URI), para o mesmo link voltar à mesma entrada.
    by_id: HashMap<(String, String), u32>,
    /// O link aberto agora: o que as células escritas recebem.
    open: Option<NonZeroU32>,
    /// Quantas entradas estavam vivas na última limpeza.
    live_at_last_sweep: usize,
}

/// A tabela só é limpa quando passa de 64 entradas e do dobro das vivas da última limpeza:
/// o gatilho é proporcional, não um teto.
const LINKS_SWEEP_FLOOR: usize = 64;

impl Links {
    /// Abre (ou reaproveita) o link `uri` com o `id` dado e devolve o índice mais um.
    fn open(&mut self, id: Option<String>, uri: String) -> NonZeroU32 {
        if let Some(id) = &id
            && let Some(&index) = self.by_id.get(&(id.clone(), uri.clone()))
        {
            return NonZeroU32::new(index + 1).unwrap_or(NonZeroU32::MIN);
        }
        let index = u32::try_from(self.entries.len()).unwrap_or(u32::MAX - 1);
        if let Some(id) = &id {
            self.by_id.insert((id.clone(), uri.clone()), index);
        }
        self.entries.push(Hyperlink { uri, id });
        NonZeroU32::new(index + 1).unwrap_or(NonZeroU32::MIN)
    }

    fn needs_sweep(&self) -> bool {
        self.entries.len() > LINKS_SWEEP_FLOOR
            && self.entries.len() > self.live_at_last_sweep.saturating_mul(2)
    }
}

/// Lê o `id` da lista `chave=valor` separada por `:` do `OSC 8`.
fn link_id(params: &[u8]) -> Option<String> {
    params
        .split(|&b| b == b':')
        .find_map(|pair| pair.strip_prefix(b"id="))
        .filter(|id| !id.is_empty())
        .map(|id| String::from_utf8_lossy(id).into_owned())
}

/// O estado do terminal, separado do parser para o `Perform` poder emprestar só ele.
#[derive(Debug)]
struct State {
    primary: Grid,
    alternate: Grid,
    alt_active: bool,
    cursor: Cursor,
    saved: [Option<Saved>; 2],
    top: usize,
    bottom: usize,
    tabs: Vec<bool>,
    modes: Modes,
    shape: CursorShape,
    /// A forma escolhida pelo app, que o `ESC[0 q` e o RIS restauram.
    default_shape: CursorShape,
    title: String,
    title_changed: bool,
    progress: TermProgress,
    progress_changed: bool,
    notices: Vec<Notice>,
    /// O último diretório que o programa informou pelo `OSC 7`, até alguém o pegar.
    cwd: Option<String>,
    /// Os textos que o programa pediu para pôr na área de transferência (`OSC 52`).
    clipboard: Vec<String>,
    responses: Vec<u8>,
    bell: bool,
    display_offset: usize,
    last_char: Option<char>,
    links: Links,
}

/// O emulador de terminal.
pub struct Emulator {
    parser: Parser,
    state: State,
    version: u64,
}

impl std::fmt::Debug for Emulator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Emulator")
            .field("cols", &self.cols())
            .field("rows", &self.rows())
            .field("cursor", &self.cursor())
            .finish_non_exhaustive()
    }
}

fn default_tabs(cols: usize) -> Vec<bool> {
    (0..cols).map(|c| c % 8 == 0 && c > 0).collect()
}

impl Emulator {
    /// Terminal vazio de `cols` por `rows`, com até `scrollback` linhas de histórico.
    pub fn new(cols: usize, rows: usize, scrollback: usize) -> Self {
        let (cols, rows) = (cols.max(1), rows.max(1));
        Self {
            parser: Parser::new(),
            state: State {
                primary: Grid::new(cols, rows, scrollback),
                alternate: Grid::new(cols, rows, 0),
                alt_active: false,
                cursor: Cursor::default(),
                saved: [None, None],
                top: 0,
                bottom: rows - 1,
                tabs: default_tabs(cols),
                modes: Modes::default(),
                shape: CursorShape::default(),
                default_shape: CursorShape::default(),
                title: String::new(),
                title_changed: false,
                progress: TermProgress::None,
                progress_changed: false,
                notices: Vec::new(),
                cwd: None,
                clipboard: Vec::new(),
                responses: Vec::new(),
                bell: false,
                display_offset: 0,
                last_char: None,
                links: Links::default(),
            },
            version: 0,
        }
    }

    /// Interpreta os bytes que o programa escreveu.
    pub fn feed(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.parser.advance(&mut self.state, bytes);
        self.version += 1;
    }

    /// Muda o tamanho. Na tela principal o texto reflui; na alternativa só é cortado.
    pub fn resize(&mut self, cols: usize, rows: usize) {
        let (cols, rows) = (cols.max(1), rows.max(1));
        let s = &mut self.state;
        if cols == s.grid().cols && rows == s.grid().rows {
            return;
        }
        // A tela que não está em uso também muda, com o cursor que ela guardou.
        let alt_cursor_row = if s.alt_active { s.cursor.row } else { 0 };
        let main_cursor = if s.alt_active {
            s.saved[0].map_or((0, 0), |saved| (saved.cursor.row, saved.cursor.col))
        } else {
            (s.cursor.row, s.cursor.col)
        };
        let main_cursor = s.primary.reflow(cols, rows, main_cursor);
        let alt_row = s.alternate.resize_truncate(cols, rows, alt_cursor_row);
        if s.alt_active {
            s.cursor.row = alt_row;
            if let Some(saved) = &mut s.saved[0] {
                (saved.cursor.row, saved.cursor.col) = main_cursor;
            }
        } else {
            (s.cursor.row, s.cursor.col) = main_cursor;
        }
        s.cursor.col = s.cursor.col.min(cols - 1);
        s.cursor.pending_wrap = false;
        for saved in s.saved.iter_mut().flatten() {
            saved.cursor.row = saved.cursor.row.min(rows - 1);
            saved.cursor.col = saved.cursor.col.min(cols - 1);
        }
        s.top = 0;
        s.bottom = rows - 1;
        let old = s.tabs.len();
        s.tabs.resize(cols, false);
        for c in old..cols {
            s.tabs[c] = c % 8 == 0;
        }
        s.display_offset = s.display_offset.min(s.grid().history.len());
        self.version += 1;
    }

    /// Colunas.
    pub fn cols(&self) -> usize {
        self.state.grid().cols
    }

    /// Linhas da tela.
    pub fn rows(&self) -> usize {
        self.state.grid().rows
    }

    /// Linha e coluna do cursor na tela, a partir de 0.
    pub fn cursor(&self) -> (usize, usize) {
        (self.state.cursor.row, self.state.cursor.col)
    }

    /// Muda o máximo de linhas de histórico, sem apagar o que já está na tela: o que passar do
    /// novo limite sai do começo.
    pub fn set_scrollback(&mut self, lines: usize) {
        self.state.primary.set_max_history(lines);
    }

    /// Forma do cursor pedida pelo programa.
    pub fn cursor_shape(&self) -> CursorShape {
        self.state.shape
    }

    /// Muda a forma padrão do cursor, que vale até o programa pedir outra e volta com
    /// `ESC[0 q` e com o RIS (`ESC c`).
    pub fn set_default_cursor_shape(&mut self, shape: CursorShape) {
        self.state.default_shape = shape;
        self.state.shape = shape;
        self.version += 1;
    }

    /// Muda a forma padrão do cursor com o terminal em uso: a forma mostrada só acompanha se o
    /// programa não pediu outra com o `DECSCUSR`.
    pub fn change_default_cursor_shape(&mut self, shape: CursorShape) {
        if self.state.shape == self.state.default_shape {
            self.state.shape = shape;
        }
        self.state.default_shape = shape;
        self.version += 1;
    }

    /// Os modos ligados.
    pub fn modes(&self) -> Modes {
        self.state.modes
    }

    /// Se a tela alternativa (a do vim e do htop) está em uso.
    pub fn alt_screen(&self) -> bool {
        self.state.alt_active
    }

    /// Título posto pelo programa (`OSC 0` ou `OSC 2`).
    pub fn title(&self) -> &str {
        &self.state.title
    }

    /// O título, se mudou desde a última chamada.
    pub fn take_title_change(&mut self) -> Option<String> {
        std::mem::take(&mut self.state.title_changed).then(|| self.state.title.clone())
    }

    /// O progresso que o programa informou pelo `OSC 9;4` e ainda vale.
    pub fn progress(&self) -> TermProgress {
        self.state.progress
    }

    /// O progresso, se mudou desde a última chamada (uma sequência que repete o valor não
    /// conta).
    pub fn take_progress_change(&mut self) -> Option<TermProgress> {
        std::mem::take(&mut self.state.progress_changed).then_some(self.state.progress)
    }

    /// O link (`OSC 8`) da célula da linha `row` da vista (como em [`Emulator::view_row`]) e
    /// da coluna `col`.
    pub fn link_at(&self, row: usize, col: usize) -> Option<&Hyperlink> {
        self.link(self.view_row(row).cells.get(col)?)
    }

    /// O link a que `cell` pertence.
    pub fn link(&self, cell: &Cell) -> Option<&Hyperlink> {
        let index = usize::try_from(cell.link?.get() - 1).ok()?;
        self.state.links.entries.get(index)
    }

    /// Quantas entradas a tabela de links tem agora (as vivas e as que ainda não foram
    /// limpas).
    pub fn links_len(&self) -> usize {
        self.state.links.entries.len()
    }

    /// Os avisos que o programa pediu (`OSC 9` com texto e `OSC 777;notify`) desde a última
    /// chamada, na ordem.
    pub fn take_notices(&mut self) -> Vec<Notice> {
        std::mem::take(&mut self.state.notices)
    }

    /// O diretório que o programa informou pelo `OSC 7` (`ESC ] 7 ; file://máquina/caminho`),
    /// se informou outro desde a última chamada. A máquina é ignorada, os `%XX` do caminho são
    /// decodificados e só o esquema `file://` vale.
    pub fn take_cwd(&mut self) -> Option<String> {
        self.state.cwd.take()
    }

    /// Os textos que o programa pediu para copiar (`OSC 52`) desde a última chamada, na ordem.
    /// O pedido de leitura (`?`), o base64 torto e o que passa de 1 MiB são ignorados.
    pub fn take_clipboard(&mut self) -> Vec<String> {
        std::mem::take(&mut self.state.clipboard)
    }

    /// As respostas que o terminal deve mandar ao programa (o DA e a posição do cursor).
    pub fn take_responses(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.state.responses)
    }

    /// Se tocou a campainha (BEL) desde a última chamada.
    pub fn take_bell(&mut self) -> bool {
        std::mem::take(&mut self.state.bell)
    }

    /// Conta as mudanças: muda a cada `feed`, `resize` e rolagem do histórico.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// A grade em uso.
    pub fn grid(&self) -> &Grid {
        self.state.grid()
    }

    /// Linhas do histórico da tela em uso.
    pub fn history_len(&self) -> usize {
        self.state.grid().history.len()
    }

    /// Quantas linhas acima do fim a vista está (0 é a tela do programa).
    pub fn display_offset(&self) -> usize {
        self.state.display_offset
    }

    /// Rola a vista pelo histórico: positivo sobe. Devolve se mudou.
    pub fn scroll_display(&mut self, delta: isize) -> bool {
        let max = self.history_len() as isize;
        let offset = (self.state.display_offset as isize + delta).clamp(0, max) as usize;
        self.set_display_offset(offset)
    }

    /// Põe a vista `offset` linhas acima do fim. Devolve se mudou.
    pub fn set_display_offset(&mut self, offset: usize) -> bool {
        let offset = offset.min(self.history_len());
        if offset == self.state.display_offset {
            return false;
        }
        self.state.display_offset = offset;
        self.version += 1;
        true
    }

    /// A linha `row` da vista (o histórico, se rolado para cima).
    pub fn view_row(&self, row: usize) -> &Row {
        let grid = self.state.grid();
        let index = grid.history.len() - self.state.display_offset + row;
        grid.line(index).unwrap_or(&grid.lines[grid.rows - 1])
    }

    /// Posição absoluta da linha `row` da vista: não muda quando o histórico anda.
    pub fn view_line(&self, row: usize) -> u64 {
        let grid = self.state.grid();
        grid.dropped + (grid.history.len() - self.state.display_offset + row) as u64
    }

    /// A linha na posição absoluta `line`, se ainda existe.
    pub fn line_at(&self, line: u64) -> Option<&Row> {
        let grid = self.state.grid();
        let index = line.checked_sub(grid.dropped)?;
        grid.line(usize::try_from(index).ok()?)
    }

    /// Posição absoluta da primeira linha do histórico e da última da tela.
    pub fn line_bounds(&self) -> (u64, u64) {
        let grid = self.state.grid();
        (grid.dropped, grid.dropped + grid.total_lines() as u64 - 1)
    }

    /// A célula da tela do programa (sem o histórico).
    pub fn cell(&self, row: usize, col: usize) -> Option<&Cell> {
        self.state.grid().lines.get(row)?.cells.get(col)
    }

    /// Texto de uma linha da tela do programa, sem os espaços do fim.
    pub fn line_text(&self, row: usize) -> String {
        let grid = self.state.grid();
        grid.lines
            .get(row)
            .map(|r| r.text(0, grid.cols))
            .unwrap_or_default()
    }

    /// Texto da tela do programa: uma linha por linha da tela, sem os espaços do fim e sem as
    /// linhas vazias do fim.
    pub fn screen_text(&self) -> String {
        let mut lines: Vec<String> = (0..self.rows()).map(|r| self.line_text(r)).collect();
        while lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        lines.join("\n")
    }

    /// Texto do histórico e da tela, como linhas lógicas (as quebras automáticas juntadas).
    pub fn all_text(&self) -> String {
        let grid = self.state.grid();
        let mut text = String::new();
        for index in 0..grid.total_lines() {
            let Some(row) = grid.line(index) else { break };
            text.push_str(&row.text(0, grid.cols));
            if !row.wrapped {
                text.push('\n');
            }
        }
        text.trim_end_matches('\n').to_owned()
    }
}

impl State {
    fn grid(&self) -> &Grid {
        if self.alt_active {
            &self.alternate
        } else {
            &self.primary
        }
    }

    fn grid_mut(&mut self) -> &mut Grid {
        if self.alt_active {
            &mut self.alternate
        } else {
            &mut self.primary
        }
    }

    fn cols(&self) -> usize {
        self.grid().cols
    }

    fn rows(&self) -> usize {
        self.grid().rows
    }

    fn row_mut(&mut self, row: usize) -> &mut Row {
        &mut self.grid_mut().lines[row]
    }

    /// Na coluna 0 de uma linha que continua a de cima (quebra automática), volta para a última
    /// coluna da de cima, como o PuTTY. Quem apaga com backspaces o que escreveu depois de uma
    /// barra que quebrou (o 7z por trás do pv) redesenha no mesmo lugar em vez de descer uma linha
    /// a cada volta.
    fn backspace(&mut self) {
        let row = self.cursor.row;
        if self.cursor.col == 0 && !self.cursor.pending_wrap {
            if self.modes.autowrap && row > 0 && self.grid().lines[row - 1].wrapped {
                self.cursor.row = row - 1;
                self.cursor.col = self.cols() - 1;
            }
        } else {
            self.cursor.col = self.cursor.col.saturating_sub(1);
        }
        self.cursor.pending_wrap = false;
    }

    /// Sobe a região de rolagem; as linhas que saem vão para o histórico se a região começa no
    /// topo da tela principal. Quem rolou a vista para cima continua vendo as mesmas linhas.
    fn scroll_up(&mut self, n: usize) {
        let (top, bottom, pen) = (self.top, self.bottom, self.cursor.pen);
        let keep = !self.alt_active && top == 0;
        let kept = self.grid_mut().scroll_up(top, bottom, n, &pen, keep);
        if self.display_offset > 0 {
            self.display_offset = (self.display_offset + kept).min(self.grid().history.len());
        }
    }

    fn scroll_down(&mut self, n: usize) {
        let (top, bottom, pen) = (self.top, self.bottom, self.cursor.pen);
        self.grid_mut().scroll_down(top, bottom, n, &pen);
    }

    fn linefeed(&mut self) {
        self.cursor.pending_wrap = false;
        if self.cursor.row == self.bottom {
            self.scroll_up(1);
        } else if self.cursor.row + 1 < self.rows() {
            self.cursor.row += 1;
        }
    }

    fn reverse_index(&mut self) {
        self.cursor.pending_wrap = false;
        if self.cursor.row == self.top {
            self.scroll_down(1);
        } else if self.cursor.row > 0 {
            self.cursor.row -= 1;
        }
    }

    fn goto(&mut self, row: usize, col: usize) {
        let (min_row, max_row) = if self.modes.origin {
            (self.top, self.bottom)
        } else {
            (0, self.rows() - 1)
        };
        let row = if self.modes.origin {
            row + self.top
        } else {
            row
        };
        self.cursor.row = row.clamp(min_row, max_row);
        self.cursor.col = col.min(self.cols() - 1);
        self.cursor.pending_wrap = false;
    }

    fn goto_row(&mut self, row: usize) {
        let col = self.cursor.col;
        self.goto(row, col);
    }

    /// Apaga as colunas `start..end` da linha `row` com o fundo da caneta, desfazendo os
    /// caracteres largos que ficarem partidos nas pontas.
    fn erase(&mut self, row: usize, start: usize, end: usize) {
        let pen = self.cursor.pen;
        let cols = self.cols();
        let end = end.min(cols);
        if start >= end {
            return;
        }
        let line = self.row_mut(row);
        let start = if line.cells[start].flags.contains(Flags::WIDE_SPACER) && start > 0 {
            start - 1
        } else {
            start
        };
        let end = if end < cols && line.cells[end].flags.contains(Flags::WIDE_SPACER) {
            end + 1
        } else {
            end
        };
        for cell in &mut line.cells[start..end] {
            *cell = Cell::erased(&pen);
        }
    }

    /// Antes de escrever em `col..col+width`, desfaz os largos que seriam partidos.
    fn clear_wide_around(&mut self, row: usize, col: usize, width: usize) {
        let cols = self.cols();
        let line = self.row_mut(row);
        if line.cells[col].flags.contains(Flags::WIDE_SPACER) && col > 0 {
            line.cells[col - 1] = Cell::BLANK;
        }
        let last = (col + width - 1).min(cols - 1);
        if line.cells[last].flags.contains(Flags::WIDE) && last + 1 < cols {
            line.cells[last + 1] = Cell::BLANK;
        }
    }

    fn print_char(&mut self, c: char) {
        let c = self.cursor.charsets[self.cursor.shift].map(c);
        let width = if (c as u32) < 0x7f {
            1
        } else {
            match c.width() {
                Some(w) => w,
                None => return,
            }
        };
        let cols = self.cols();
        if width == 0 {
            self.combine(c);
            return;
        }
        if self.cursor.pending_wrap && self.modes.autowrap {
            let row = self.cursor.row;
            self.row_mut(row).wrapped = true;
            self.linefeed();
            self.cursor.col = 0;
        }
        self.cursor.pending_wrap = false;
        if width == 2 && cols < 2 {
            return;
        }
        if width == 2 && self.cursor.col == cols - 1 {
            if !self.modes.autowrap {
                return;
            }
            let (row, pen) = (self.cursor.row, self.cursor.pen);
            let line = self.row_mut(row);
            let mut pad = Cell::erased(&pen);
            pad.flags = Flags::WIDE_PAD;
            line.cells[cols - 1] = pad;
            line.wrapped = true;
            self.linefeed();
            self.cursor.col = 0;
        }
        let (row, col, pen) = (self.cursor.row, self.cursor.col, self.cursor.pen);
        let link = self.links.open;
        if self.modes.insert {
            self.insert_blanks(width);
        }
        self.clear_wide_around(row, col, width);
        let line = self.row_mut(row);
        let mut cell = Cell::with(c, &pen);
        cell.link = link;
        if width == 2 {
            cell.flags |= Flags::WIDE;
            let mut spacer = Cell::with(' ', &pen);
            spacer.link = link;
            spacer.flags = (pen.flags & Flags::SGR) | Flags::WIDE_SPACER;
            line.cells[col + 1] = spacer;
        }
        line.cells[col] = cell;
        self.last_char = Some(c);
        if col + width >= cols {
            self.cursor.col = cols - 1;
            self.cursor.pending_wrap = self.modes.autowrap;
        } else {
            self.cursor.col = col + width;
        }
    }

    /// Junta um acento combinado ao último caractere escrito.
    fn combine(&mut self, c: char) {
        let (row, mut col) = (self.cursor.row, self.cursor.col);
        if !self.cursor.pending_wrap {
            if col == 0 {
                return;
            }
            col -= 1;
        }
        let line = self.row_mut(row);
        if line.cells[col].flags.contains(Flags::WIDE_SPACER) && col > 0 {
            col -= 1;
        }
        line.cells[col].push_combining(c);
    }

    fn insert_blanks(&mut self, n: usize) {
        let (row, col, pen) = (self.cursor.row, self.cursor.col, self.cursor.pen);
        let cols = self.cols();
        let n = n.min(cols - col);
        let line = self.row_mut(row);
        line.cells[col..].rotate_right(n);
        for cell in &mut line.cells[col..col + n] {
            *cell = Cell::erased(&pen);
        }
        if line.cells[cols - 1].flags.contains(Flags::WIDE) {
            line.cells[cols - 1] = Cell::erased(&pen);
        }
    }

    fn delete_chars(&mut self, n: usize) {
        let (row, col, pen) = (self.cursor.row, self.cursor.col, self.cursor.pen);
        let cols = self.cols();
        let n = n.min(cols - col);
        self.clear_wide_around(row, col, 1);
        let line = self.row_mut(row);
        line.cells[col..].rotate_left(n);
        for cell in &mut line.cells[cols - n..] {
            *cell = Cell::erased(&pen);
        }
        if line.cells[col].flags.contains(Flags::WIDE_SPACER) {
            line.cells[col] = Cell::erased(&pen);
        }
    }

    fn insert_lines(&mut self, n: usize) {
        let row = self.cursor.row;
        if row < self.top || row > self.bottom {
            return;
        }
        let (bottom, pen) = (self.bottom, self.cursor.pen);
        self.grid_mut().scroll_down(row, bottom, n, &pen);
        self.cursor.col = 0;
        self.cursor.pending_wrap = false;
    }

    fn delete_lines(&mut self, n: usize) {
        let row = self.cursor.row;
        if row < self.top || row > self.bottom {
            return;
        }
        let (bottom, pen) = (self.bottom, self.cursor.pen);
        self.grid_mut().scroll_up(row, bottom, n, &pen, false);
        self.cursor.col = 0;
        self.cursor.pending_wrap = false;
    }

    fn erase_display(&mut self, mode: u16) {
        let (row, col) = (self.cursor.row, self.cursor.col);
        let (rows, cols) = (self.rows(), self.cols());
        match mode {
            0 => {
                self.erase(row, col, cols);
                self.row_mut(row).wrapped = false;
                for r in row + 1..rows {
                    self.erase(r, 0, cols);
                    self.row_mut(r).wrapped = false;
                }
            }
            1 => {
                for r in 0..row {
                    self.erase(r, 0, cols);
                    self.row_mut(r).wrapped = false;
                }
                self.erase(row, 0, col + 1);
            }
            2 => {
                for r in 0..rows {
                    self.erase(r, 0, cols);
                    self.row_mut(r).wrapped = false;
                }
            }
            3 => {
                self.grid_mut().clear_history();
                self.display_offset = 0;
            }
            _ => {}
        }
    }

    fn erase_line(&mut self, mode: u16) {
        let (row, col, cols) = (self.cursor.row, self.cursor.col, self.cols());
        match mode {
            0 => {
                self.erase(row, col, cols);
                self.row_mut(row).wrapped = false;
            }
            1 => self.erase(row, 0, col + 1),
            2 => {
                self.erase(row, 0, cols);
                self.row_mut(row).wrapped = false;
            }
            _ => {}
        }
    }

    fn tab_forward(&mut self, n: usize) {
        let cols = self.cols();
        for _ in 0..n {
            let next = (self.cursor.col + 1..cols).find(|&c| self.tabs[c]);
            self.cursor.col = next.unwrap_or(cols - 1);
        }
        self.cursor.pending_wrap = false;
    }

    fn tab_backward(&mut self, n: usize) {
        for _ in 0..n {
            let prev = (0..self.cursor.col).rev().find(|&c| self.tabs[c]);
            self.cursor.col = prev.unwrap_or(0);
        }
        self.cursor.pending_wrap = false;
    }

    fn save_cursor(&mut self) {
        let index = usize::from(self.alt_active);
        self.saved[index] = Some(Saved {
            cursor: self.cursor,
            origin: self.modes.origin,
        });
    }

    fn restore_cursor(&mut self) {
        let index = usize::from(self.alt_active);
        let saved = self.saved[index].unwrap_or_default();
        self.cursor = saved.cursor;
        self.modes.origin = saved.origin;
        self.cursor.row = self.cursor.row.min(self.rows() - 1);
        self.cursor.col = self.cursor.col.min(self.cols() - 1);
    }

    fn set_alt_screen(&mut self, on: bool, clear: bool) {
        if on == self.alt_active {
            return;
        }
        if on {
            self.alt_active = true;
            if clear {
                let pen = self.cursor.pen;
                let cols = self.cols();
                for row in &mut self.alternate.lines {
                    row.reset(cols, &pen);
                }
            }
        } else {
            if clear {
                let cols = self.cols();
                for row in &mut self.alternate.lines {
                    row.reset(cols, &Pen::default());
                }
            }
            self.alt_active = false;
        }
        self.display_offset = 0;
        self.cursor.row = self.cursor.row.min(self.rows() - 1);
    }

    fn set_mode(&mut self, mode: u16, private: bool, on: bool) {
        if !private {
            match mode {
                4 => self.modes.insert = on,
                20 => self.modes.newline = on,
                _ => {}
            }
            return;
        }
        match mode {
            1 => self.modes.app_cursor = on,
            5 => self.modes.reverse_video = on,
            6 => {
                self.modes.origin = on;
                self.goto(0, 0);
            }
            7 => self.modes.autowrap = on,
            12 => self.modes.cursor_blink = on,
            25 => self.modes.cursor_visible = on,
            47 => self.set_alt_screen(on, false),
            1047 => self.set_alt_screen(on, !on),
            1048 => {
                if on {
                    self.save_cursor();
                } else {
                    self.restore_cursor();
                }
            }
            1049 => {
                if on {
                    self.save_cursor();
                    self.set_alt_screen(true, true);
                } else {
                    self.set_alt_screen(false, false);
                    self.restore_cursor();
                }
            }
            9 => self.set_mouse(MouseTracking::X10, on),
            1000 => self.set_mouse(MouseTracking::Buttons, on),
            1002 => self.set_mouse(MouseTracking::Drag, on),
            1003 => self.set_mouse(MouseTracking::Motion, on),
            1004 => self.modes.focus_events = on,
            1005 => self.set_mouse_encoding(MouseEncoding::Utf8, on),
            1006 => self.set_mouse_encoding(MouseEncoding::Sgr, on),
            1007 => self.modes.alternate_scroll = on,
            1015 => self.set_mouse_encoding(MouseEncoding::Urxvt, on),
            2004 => self.modes.bracketed_paste = on,
            _ => {}
        }
    }

    /// Liga um modo do mouse (troca o anterior) ou desliga o relato, qualquer que seja o
    /// modo desligado.
    fn set_mouse(&mut self, tracking: MouseTracking, on: bool) {
        self.modes.mouse = if on { tracking } else { MouseTracking::Off };
    }

    /// Liga uma forma de escrever as posições (troca a anterior) ou, se `encoding` é a ligada,
    /// volta à padrão.
    fn set_mouse_encoding(&mut self, encoding: MouseEncoding, on: bool) {
        if on {
            self.modes.mouse_encoding = encoding;
        } else if self.modes.mouse_encoding == encoding {
            self.modes.mouse_encoding = MouseEncoding::Default;
        }
    }

    fn reset(&mut self) {
        let (cols, rows) = (self.cols(), self.rows());
        let scrollback = self.primary.max_history;
        let dropped = self.primary.dropped + self.primary.total_lines() as u64;
        self.primary = Grid::new(cols, rows, scrollback);
        self.primary.dropped = dropped;
        self.alternate = Grid::new(cols, rows, 0);
        self.alt_active = false;
        self.cursor = Cursor::default();
        self.saved = [None, None];
        self.top = 0;
        self.bottom = rows - 1;
        self.tabs = default_tabs(cols);
        self.modes = Modes::default();
        self.shape = self.default_shape;
        self.display_offset = 0;
        self.last_char = None;
        self.links = Links::default();
        self.set_progress(TermProgress::None);
    }

    /// Abre o link `uri` (ou fecha o aberto, com a URI vazia).
    fn set_link(&mut self, params: &[u8], uri: String) {
        if uri.is_empty() {
            self.links.open = None;
            return;
        }
        let open = self.links.open(link_id(params), uri);
        self.links.open = Some(open);
        if self.links.needs_sweep() {
            self.sweep_links();
        }
    }

    /// Tira da tabela os links que nenhuma célula (da tela, do histórico, nas duas grades) e
    /// nem o link aberto usam, e reescreve os índices das células.
    fn sweep_links(&mut self) {
        let len = self.links.entries.len();
        let mut used = vec![false; len];
        let index = |link: NonZeroU32| usize::try_from(link.get() - 1).ok().filter(|&i| i < len);
        if let Some(i) = self.links.open.and_then(index) {
            used[i] = true;
        }
        for grid in [&self.primary, &self.alternate] {
            for row in grid.history.iter().chain(&grid.lines) {
                for cell in &row.cells {
                    if let Some(i) = cell.link.and_then(index) {
                        used[i] = true;
                    }
                }
            }
        }
        let mut remap = vec![0u32; len];
        let mut kept = Vec::new();
        for (old, entry) in std::mem::take(&mut self.links.entries)
            .into_iter()
            .enumerate()
        {
            if used[old] {
                kept.push(entry);
                remap[old] = u32::try_from(kept.len()).unwrap_or(u32::MAX);
            }
        }
        let new_link =
            |link: Option<NonZeroU32>| link.and_then(index).and_then(|i| NonZeroU32::new(remap[i]));
        for grid in [&mut self.primary, &mut self.alternate] {
            for row in grid.history.iter_mut().chain(&mut grid.lines) {
                for cell in &mut row.cells {
                    if cell.link.is_some() {
                        cell.link = new_link(cell.link);
                    }
                }
            }
        }
        self.links.open = new_link(self.links.open);
        self.links.by_id = kept
            .iter()
            .enumerate()
            .filter_map(|(i, entry)| {
                let id = entry.id.clone()?;
                Some(((id, entry.uri.clone()), u32::try_from(i).ok()?))
            })
            .collect();
        self.links.live_at_last_sweep = kept.len();
        self.links.entries = kept;
    }

    /// Grava o progresso e marca a mudança só se o valor é outro.
    fn set_progress(&mut self, progress: TermProgress) {
        if progress != self.progress {
            self.progress = progress;
            self.progress_changed = true;
        }
    }

    fn soft_reset(&mut self) {
        // Como no xterm, o DECSTR não mexe no mouse nem no aviso de foco: só o RIS os zera.
        self.modes = Modes {
            reverse_video: self.modes.reverse_video,
            bracketed_paste: self.modes.bracketed_paste,
            mouse: self.modes.mouse,
            mouse_encoding: self.modes.mouse_encoding,
            alternate_scroll: self.modes.alternate_scroll,
            focus_events: self.modes.focus_events,
            ..Modes::default()
        };
        self.top = 0;
        self.bottom = self.rows() - 1;
        self.cursor.pen = Pen::default();
        self.cursor.charsets = [Charset::Ascii; 2];
        self.cursor.shift = 0;
        self.cursor.pending_wrap = false;
        self.saved = [None, None];
    }

    fn sgr(&mut self, params: &Params) {
        let items: Vec<&[u16]> = params.iter().collect();
        if items.is_empty() {
            self.cursor.pen = Pen::default();
            return;
        }
        let pen = &mut self.cursor.pen;
        let mut i = 0;
        while i < items.len() {
            let item = items[i];
            let code = item[0];
            match code {
                0 => *pen = Pen::default(),
                1 => pen.flags |= Flags::BOLD,
                2 => pen.flags |= Flags::DIM,
                3 => pen.flags |= Flags::ITALIC,
                4 => {
                    pen.flags
                        .set(Flags::UNDERLINE | Flags::DOUBLE_UNDERLINE, false);
                    match item.get(1).copied() {
                        Some(0) => {}
                        Some(2) => pen.flags |= Flags::DOUBLE_UNDERLINE,
                        _ => pen.flags |= Flags::UNDERLINE,
                    }
                }
                5 | 6 => pen.flags |= Flags::BLINK,
                7 => pen.flags |= Flags::INVERSE,
                8 => pen.flags |= Flags::HIDDEN,
                9 => pen.flags |= Flags::STRIKE,
                21 => {
                    pen.flags.set(Flags::UNDERLINE, false);
                    pen.flags |= Flags::DOUBLE_UNDERLINE;
                }
                22 => pen.flags.set(Flags::BOLD | Flags::DIM, false),
                23 => pen.flags.set(Flags::ITALIC, false),
                24 => pen
                    .flags
                    .set(Flags::UNDERLINE | Flags::DOUBLE_UNDERLINE, false),
                25 => pen.flags.set(Flags::BLINK, false),
                27 => pen.flags.set(Flags::INVERSE, false),
                28 => pen.flags.set(Flags::HIDDEN, false),
                29 => pen.flags.set(Flags::STRIKE, false),
                30..=37 => pen.fg = Color::Indexed((code - 30) as u8),
                39 => pen.fg = Color::Default,
                40..=47 => pen.bg = Color::Indexed((code - 40) as u8),
                49 => pen.bg = Color::Default,
                90..=97 => pen.fg = Color::Indexed((code - 90 + 8) as u8),
                100..=107 => pen.bg = Color::Indexed((code - 100 + 8) as u8),
                38 | 48 | 58 => {
                    let (color, used) = extended_color(&items[i..]);
                    if let Some(color) = color {
                        match code {
                            38 => pen.fg = color,
                            48 => pen.bg = color,
                            _ => {}
                        }
                    }
                    i += used;
                    continue;
                }
                _ => {}
            }
            i += 1;
        }
    }

    fn csi(&mut self, params: &Params, intermediates: &[u8], action: char) {
        let first: Vec<u16> = params.iter().map(|p| p[0]).collect();
        let arg = |i: usize, default: u16| -> usize {
            match first.get(i).copied() {
                None | Some(0) => usize::from(default),
                Some(v) => usize::from(v),
            }
        };
        let raw = |i: usize| first.get(i).copied().unwrap_or(0);
        let private = intermediates.first() == Some(&b'?');
        let (rows, cols) = (self.rows(), self.cols());
        match (action, intermediates) {
            ('@', []) => self.insert_blanks(arg(0, 1)),
            ('A', []) => {
                let n = arg(0, 1);
                let min = if self.cursor.row >= self.top {
                    self.top
                } else {
                    0
                };
                self.cursor.row = self.cursor.row.saturating_sub(n).max(min);
                self.cursor.pending_wrap = false;
            }
            ('B' | 'e', []) => {
                let n = arg(0, 1);
                let max = if self.cursor.row <= self.bottom {
                    self.bottom
                } else {
                    rows - 1
                };
                self.cursor.row = (self.cursor.row + n).min(max);
                self.cursor.pending_wrap = false;
            }
            ('C' | 'a', []) => {
                self.cursor.col = (self.cursor.col + arg(0, 1)).min(cols - 1);
                self.cursor.pending_wrap = false;
            }
            ('D', []) => {
                self.cursor.col = self.cursor.col.saturating_sub(arg(0, 1));
                self.cursor.pending_wrap = false;
            }
            ('E', []) => {
                let max = if self.cursor.row <= self.bottom {
                    self.bottom
                } else {
                    rows - 1
                };
                self.cursor.row = (self.cursor.row + arg(0, 1)).min(max);
                self.cursor.col = 0;
                self.cursor.pending_wrap = false;
            }
            ('F', []) => {
                let min = if self.cursor.row >= self.top {
                    self.top
                } else {
                    0
                };
                self.cursor.row = self.cursor.row.saturating_sub(arg(0, 1)).max(min);
                self.cursor.col = 0;
                self.cursor.pending_wrap = false;
            }
            ('G' | '`', []) => {
                self.cursor.col = (arg(0, 1) - 1).min(cols - 1);
                self.cursor.pending_wrap = false;
            }
            ('H' | 'f', []) => self.goto(arg(0, 1) - 1, arg(1, 1) - 1),
            ('I', []) => self.tab_forward(arg(0, 1)),
            ('J', [] | [b'?']) => self.erase_display(raw(0)),
            ('K', [] | [b'?']) => self.erase_line(raw(0)),
            ('L', []) => self.insert_lines(arg(0, 1)),
            ('M', []) => self.delete_lines(arg(0, 1)),
            ('P', []) => self.delete_chars(arg(0, 1)),
            ('S', []) => self.scroll_up(arg(0, 1)),
            ('T', []) if first.len() <= 1 => self.scroll_down(arg(0, 1)),
            ('X', []) => {
                let (row, col) = (self.cursor.row, self.cursor.col);
                self.erase(row, col, col + arg(0, 1));
            }
            ('Z', []) => self.tab_backward(arg(0, 1)),
            ('b', []) => {
                if let Some(c) = self.last_char {
                    for _ in 0..arg(0, 1).min(65535) {
                        self.print_char(c);
                    }
                }
            }
            ('c', []) if raw(0) == 0 => self.responses.extend_from_slice(b"\x1b[?62;22c"),
            ('c', [b'>']) if raw(0) == 0 => self.responses.extend_from_slice(b"\x1b[>0;10;1c"),
            ('d', []) => self.goto_row(arg(0, 1) - 1),
            ('g', []) => match raw(0) {
                0 => {
                    let col = self.cursor.col;
                    self.tabs[col] = false;
                }
                3 => self.tabs.iter_mut().for_each(|t| *t = false),
                _ => {}
            },
            ('h' | 'l', [] | [b'?']) => {
                for &mode in &first {
                    self.set_mode(mode, private, action == 'h');
                }
            }
            ('m', []) => self.sgr(params),
            ('n', []) => match raw(0) {
                5 => self.responses.extend_from_slice(b"\x1b[0n"),
                6 => {
                    let row = if self.modes.origin {
                        self.cursor.row - self.top
                    } else {
                        self.cursor.row
                    };
                    let reply = format!("\x1b[{};{}R", row + 1, self.cursor.col + 1);
                    self.responses.extend_from_slice(reply.as_bytes());
                }
                _ => {}
            },
            ('r', []) => {
                let top = arg(0, 1) - 1;
                let bottom = arg(1, rows as u16).min(rows) - 1;
                if top < bottom {
                    self.top = top;
                    self.bottom = bottom;
                    self.goto(0, 0);
                }
            }
            ('s', []) => self.save_cursor(),
            ('u', []) => self.restore_cursor(),
            ('q', [b' ']) => {
                // O 0 volta à forma escolhida pelo app (no xterm é o bloco
                // piscando).
                let (shape, blink) = match raw(0) {
                    0 => (self.default_shape, true),
                    1 => (CursorShape::Block, true),
                    2 => (CursorShape::Block, false),
                    3 => (CursorShape::Underline, true),
                    4 => (CursorShape::Underline, false),
                    5 => (CursorShape::Bar, true),
                    6 => (CursorShape::Bar, false),
                    _ => return,
                };
                self.shape = shape;
                self.modes.cursor_blink = blink;
            }
            ('p', [b'!']) => self.soft_reset(),
            _ => {}
        }
    }
}

/// Lê a cor do `38`, `48` ou `58` do SGR, nas duas formas: `38;5;n` e `38;2;r;g;b` (com `;`)
/// e `38:5:n`, `38:2:r:g:b` e `38:2:espaço:r:g:b` (com `:`). Devolve a cor e quantos itens
/// usou.
fn extended_color(items: &[&[u16]]) -> (Option<Color>, usize) {
    let byte = |v: u16| v.min(255) as u8;
    let head = items[0];
    if head.len() > 1 {
        let color = match head.get(1) {
            Some(5) => head.get(2).map(|&n| Color::Indexed(byte(n))),
            Some(2) => {
                let rgb = if head.len() >= 6 {
                    &head[3..6]
                } else {
                    head.get(2..5).unwrap_or(&[])
                };
                (rgb.len() == 3).then(|| Color::Rgb(byte(rgb[0]), byte(rgb[1]), byte(rgb[2])))
            }
            _ => None,
        };
        return (color, 1);
    }
    let value = |i: usize| items.get(i).map(|item| item[0]);
    match value(1) {
        Some(5) => match value(2) {
            Some(n) => (Some(Color::Indexed(byte(n))), 3),
            None => (None, 2),
        },
        Some(2) => match (value(2), value(3), value(4)) {
            (Some(r), Some(g), Some(b)) => (Some(Color::Rgb(byte(r), byte(g), byte(b))), 5),
            _ => (None, items.len()),
        },
        _ => (None, 1),
    }
}

impl Perform for State {
    fn print(&mut self, c: char) {
        self.print_char(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            0x07 => self.bell = true,
            0x08 => self.backspace(),
            0x09 => self.tab_forward(1),
            0x0A..=0x0C => {
                self.linefeed();
                if self.modes.newline {
                    self.cursor.col = 0;
                }
            }
            0x0D => {
                self.cursor.col = 0;
                self.cursor.pending_wrap = false;
            }
            0x0E => self.cursor.shift = 1,
            0x0F => self.cursor.shift = 0,
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignore: bool, action: char) {
        if !ignore {
            self.csi(params, intermediates, action);
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], ignore: bool, byte: u8) {
        if ignore {
            return;
        }
        match (intermediates, byte) {
            ([], b'7') => self.save_cursor(),
            ([], b'8') => self.restore_cursor(),
            ([], b'D') => self.linefeed(),
            ([], b'E') => {
                self.linefeed();
                self.cursor.col = 0;
            }
            ([], b'H') => {
                let col = self.cursor.col;
                self.tabs[col] = true;
            }
            ([], b'M') => self.reverse_index(),
            ([], b'c') => self.reset(),
            ([], b'=') => self.modes.app_keypad = true,
            ([], b'>') => self.modes.app_keypad = false,
            ([b'#'], b'8') => {
                let cols = self.cols();
                let pen = Pen::default();
                for row in &mut self.grid_mut().lines {
                    row.cells.clear();
                    row.cells.resize(cols, Cell::with('E', &pen));
                    row.wrapped = false;
                }
                self.top = 0;
                self.bottom = self.rows() - 1;
                self.goto(0, 0);
            }
            ([b'('], c) | ([b')'], c) => {
                let index = usize::from(intermediates[0] == b')');
                self.cursor.charsets[index] = if c == b'0' {
                    Charset::DecSpecial
                } else {
                    Charset::Ascii
                };
            }
            _ => {}
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        match params {
            [b"0" | b"2", rest @ ..] => {
                let title = join(rest);
                if title != self.title {
                    self.title = title;
                    self.title_changed = true;
                }
            }
            [b"7", rest @ ..] => {
                if let Some(path) = cwd_of(&join(rest)) {
                    self.cwd = Some(path);
                }
            }
            [b"8", params, uri @ ..] => self.set_link(params, join(uri)),
            [b"52", _target, data] => {
                if let Some(text) = osc52_text(data) {
                    self.clipboard.push(text);
                }
            }
            [b"9", b"4", rest @ ..] => {
                if let Some(progress) = TermProgress::parse(rest, self.progress) {
                    self.set_progress(progress);
                }
            }
            // `9;texto` é um aviso. Um segundo parâmetro só de dígitos (`9;9;/diretório`, que
            // alguns shells emitem) é outro comando da família e não vira aviso.
            [b"9", first, ..] => {
                if !first.is_empty() && !first.iter().all(u8::is_ascii_digit) {
                    self.notices.push(Notice {
                        title: None,
                        body: join(&params[1..]),
                    });
                }
            }
            [b"777", b"notify", title, rest @ ..] => {
                let notice = if rest.is_empty() {
                    Notice {
                        title: None,
                        body: join(&[title]),
                    }
                } else {
                    Notice {
                        title: Some(join(&[title])).filter(|t| !t.is_empty()),
                        body: join(rest),
                    }
                };
                self.notices.push(notice);
            }
            _ => {}
        }
    }
}

/// O maior texto que o `OSC 52` pode pôr na área de transferência (o base64 vem 4/3 maior).
const OSC52_LIMIT: usize = 1024 * 1024;

/// O caminho de um `file://máquina/caminho` do `OSC 7`, com os `%XX` decodificados.
fn cwd_of(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    // A máquina vai até a primeira barra e não importa: o caminho é o que vale.
    let path = &rest[rest.find('/')?..];
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| char::from(b).to_digit(16);
        match (
            bytes[i],
            bytes.get(i + 1).and_then(|&b| hex(b)),
            bytes.get(i + 2).and_then(|&b| hex(b)),
        ) {
            (b'%', Some(hi), Some(lo)) => {
                out.push((hi * 16 + lo) as u8);
                i += 3;
            }
            (b, ..) => {
                out.push(b);
                i += 1;
            }
        }
    }
    Some(String::from_utf8_lossy(&out).into_owned())
}

/// O texto de um `OSC 52` (`base64`), ou `None` para o pedido de leitura (`?`), o base64 torto
/// e o que passa do limite.
fn osc52_text(data: &[u8]) -> Option<String> {
    use base64::Engine as _;
    if data == b"?" || data.len() > OSC52_LIMIT / 3 * 4 + 4 {
        return None;
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .ok()
        .filter(|b| b.len() <= OSC52_LIMIT)?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Os parâmetros de um OSC emendados com `;`, como o programa os escreveu (o texto pode ter
/// `;`, que o parser dividiu).
fn join(params: &[&[u8]]) -> String {
    params
        .iter()
        .map(|p| String::from_utf8_lossy(p))
        .collect::<Vec<_>>()
        .join(";")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn term(cols: usize, rows: usize) -> Emulator {
        Emulator::new(cols, rows, 100)
    }

    #[test]
    fn texto_e_quebra_automatica() {
        let mut t = term(5, 3);
        t.feed(b"abcdefg");
        assert_eq!(t.screen_text(), "abcde\nfg");
        assert!(t.grid().lines[0].wrapped);
        assert_eq!(t.cursor(), (1, 2));
    }

    #[test]
    fn backspace_na_coluna_zero_volta_para_a_linha_que_quebrou() {
        let mut t = term(5, 3);
        t.feed(b"abcdefg\r\x08\x08XY");
        assert_eq!(t.screen_text(), "abcXY\nfg");
        assert_eq!(t.cursor(), (0, 4));
        // Sem quebra na linha de cima, o backspace fica na coluna 0.
        t.feed(b"\r\n\n\x08Z");
        assert_eq!(t.screen_text(), "abcXY\nfg\nZ");
        // Na espera da última coluna, anda uma para trás, como no xterm.
        t.feed(b"\rabcde\x08W");
        assert_eq!(t.screen_text(), "abcXY\nfg\nabcWe");
    }

    #[test]
    fn a_barra_do_pv_com_o_andamento_do_7z_nao_desce_a_cada_volta() {
        // O 7z escreve o andamento dele, o pv desenha a barra logo depois (e quebra), volta com
        // `\r`, e o 7z apaga com backspaces o que escreveu: tudo fica nas mesmas duas linhas.
        let mut t = term(20, 6);
        for volta in 0..5 {
            t.feed(format!("\x08\x08\x08\x08   \x08\x08\x08 {volta}M").as_bytes());
            t.feed(format!("{:<19}\r", format!("barra {volta}")).as_bytes());
        }
        assert_eq!(t.cursor().0, 1);
        assert_eq!(t.history_len(), 0);
    }

    #[test]
    fn ultima_coluna_espera_o_proximo_caractere() {
        let mut t = term(5, 3);
        t.feed(b"abcde");
        assert_eq!(t.cursor(), (0, 4));
        t.feed(b"\r\n");
        assert_eq!(t.cursor(), (1, 0));
        assert!(!t.grid().lines[0].wrapped);
    }

    #[test]
    fn mudar_o_historico_mantem_a_tela_e_so_corta_o_que_passa_do_limite() {
        let mut t = term(10, 2);
        t.feed(b"um\r\ndois\r\ntres\r\nquatro\r\ncinco");
        assert_eq!(t.history_len(), 3);
        let tela = t.screen_text();

        t.set_scrollback(5000);
        assert_eq!(t.history_len(), 3, "aumentar o limite não apaga nada");
        assert_eq!(t.screen_text(), tela);

        t.set_scrollback(1);
        assert_eq!(t.history_len(), 1, "só o excedente sai, do começo");
        assert_eq!(t.screen_text(), tela, "a tela fica como estava");

        t.set_scrollback(0);
        assert_eq!(t.history_len(), 0);
        assert_eq!(t.screen_text(), tela);
    }

    #[test]
    fn cr_lf_e_rolagem_para_o_histórico() {
        let mut t = term(10, 2);
        t.feed(b"um\r\ndois\r\ntres");
        assert_eq!(t.screen_text(), "dois\ntres");
        assert_eq!(t.history_len(), 1);
        assert_eq!(t.all_text(), "um\ndois\ntres");
    }

    #[test]
    fn posicionar_e_apagar() {
        let mut t = term(10, 3);
        t.feed(b"0123456789\r\nabcdefghij\r\nABCDEFGHIJ");
        t.feed(b"\x1b[2;5H\x1b[K");
        assert_eq!(t.line_text(1), "abcd");
        t.feed(b"\x1b[1;3H\x1b[1K");
        assert_eq!(t.line_text(0), "   3456789");
        t.feed(b"\x1b[3;2H\x1b[2X");
        assert_eq!(t.line_text(2), "A  DEFGHIJ");
        t.feed(b"\x1b[2J");
        assert_eq!(t.screen_text(), "");
    }

    #[test]
    fn apagar_usa_a_cor_de_fundo() {
        let mut t = term(4, 1);
        t.feed(b"\x1b[44m\x1b[2K");
        assert_eq!(t.cell(0, 3).unwrap().bg, Color::Indexed(4));
    }

    #[test]
    fn sgr_cores_e_atributos() {
        let mut t = term(20, 1);
        t.feed(b"\x1b[1;31ma\x1b[0;38;5;208mb\x1b[38;2;1;2;3;48:2::4:5:6mc\x1b[94;7md\x1b[m");
        let cell = |c| t.cell(0, c).unwrap().clone();
        assert_eq!(cell(0).fg, Color::Indexed(1));
        assert!(cell(0).flags.contains(Flags::BOLD));
        assert_eq!(cell(1).fg, Color::Indexed(208));
        assert!(!cell(1).flags.contains(Flags::BOLD));
        assert_eq!(cell(2).fg, Color::Rgb(1, 2, 3));
        assert_eq!(cell(2).bg, Color::Rgb(4, 5, 6));
        assert_eq!(cell(3).fg, Color::Indexed(12));
        assert!(cell(3).flags.contains(Flags::INVERSE));
        t.feed(b"\x1b[4:3mx\x1b[4:0my");
        assert!(t.cell(0, 4).unwrap().flags.contains(Flags::UNDERLINE));
        assert!(!t.cell(0, 5).unwrap().flags.contains(Flags::UNDERLINE));
    }

    #[test]
    fn regiao_de_rolagem() {
        let mut t = term(5, 4);
        t.feed(b"a\r\nb\r\nc\r\nd");
        t.feed(b"\x1b[2;3r\x1b[3;1H\n");
        assert_eq!(t.screen_text(), "a\nc\n\nd");
        assert_eq!(t.history_len(), 0, "a região não começa no topo");
        t.feed(b"\x1b[2;1H\x1bM");
        assert_eq!(t.screen_text(), "a\n\nc\nd");
        t.feed(b"\x1b[r");
        assert_eq!(t.cursor(), (0, 0));
    }

    #[test]
    fn inserir_e_apagar_linhas_e_caracteres() {
        let mut t = term(6, 3);
        t.feed(b"abcdef\r\nghijkl\r\nmnopqr");
        t.feed(b"\x1b[2;1H\x1b[L");
        assert_eq!(t.screen_text(), "abcdef\n\nghijkl");
        t.feed(b"\x1b[M");
        assert_eq!(t.screen_text(), "abcdef\nghijkl");
        t.feed(b"\x1b[1;2H\x1b[2P");
        assert_eq!(t.line_text(0), "adef");
        t.feed(b"\x1b[2@");
        assert_eq!(t.line_text(0), "a  def");
        t.feed(b"\x1b[4hXY\x1b[4l");
        assert_eq!(t.line_text(0), "aXY  d");
    }

    #[test]
    fn tela_alternativa_guarda_a_principal() {
        let mut t = term(10, 3);
        t.feed(b"shell$ ");
        t.feed(b"\x1b[?1049h\x1b[Hvim");
        assert!(t.alt_screen());
        assert_eq!(t.screen_text(), "vim");
        t.feed(b"\x1b[?1049l");
        assert!(!t.alt_screen());
        assert_eq!(t.screen_text(), "shell$");
        assert_eq!(t.cursor(), (0, 7));
    }

    #[test]
    fn titulo_por_osc() {
        let mut t = term(10, 1);
        t.feed(b"\x1b]0;meu titulo\x07");
        assert_eq!(t.take_title_change().as_deref(), Some("meu titulo"));
        assert_eq!(t.take_title_change(), None);
        t.feed("\x1b]2;ação\x1b\\".as_bytes());
        assert_eq!(t.title(), "ação");
    }

    #[test]
    fn respostas_do_da_e_da_posicao() {
        let mut t = term(10, 5);
        t.feed(b"\x1b[3;4H\x1b[6n\x1b[c");
        assert_eq!(t.take_responses(), b"\x1b[3;4R\x1b[?62;22c");
    }

    #[test]
    fn largos_ocupam_duas_celulas() {
        let mut t = term(5, 2);
        t.feed("日本".as_bytes());
        assert_eq!(t.cursor(), (0, 4));
        assert!(t.cell(0, 0).unwrap().flags.contains(Flags::WIDE));
        assert!(t.cell(0, 1).unwrap().flags.contains(Flags::WIDE_SPACER));
        t.feed("語".as_bytes());
        assert_eq!(t.screen_text(), "日本\n語");
        assert!(t.cell(0, 4).unwrap().flags.contains(Flags::WIDE_PAD));
        // Escrever em cima da metade de um largo apaga a outra metade.
        t.feed(b"\x1b[1;2Hx");
        assert_eq!(t.line_text(0), " x本");
    }

    #[test]
    fn acento_combinado_junta_na_celula() {
        let mut t = term(5, 1);
        t.feed("e\u{301}a".as_bytes());
        assert_eq!(t.line_text(0), "e\u{301}a");
        assert_eq!(t.cursor(), (0, 2));
    }

    #[test]
    fn desenhos_de_linha_do_vt100() {
        let mut t = term(10, 1);
        t.feed(b"\x1b(0lqk\x1b(Bq");
        assert_eq!(t.line_text(0), "┌─┐q");
        t.feed(b"\x1b)0\x0eq\x0fq");
        assert_eq!(t.line_text(0), "┌─┐q─q");
    }

    #[test]
    fn tabulacao() {
        let mut t = term(20, 1);
        t.feed(b"a\tb\x1b[3gc\td");
        assert_eq!(t.line_text(0), "a       bc         d");
    }

    #[test]
    fn decaln_e_origem() {
        let mut t = term(3, 3);
        t.feed(b"\x1b#8");
        assert_eq!(t.screen_text(), "EEE\nEEE\nEEE");
        t.feed(b"\x1b[2;3r\x1b[?6h\x1b[1;1HX\x1b[6n");
        assert_eq!(t.line_text(1), "XEE");
        assert_eq!(t.take_responses(), b"\x1b[1;2R");
    }

    #[test]
    fn modos_das_teclas_e_do_cursor() {
        let mut t = term(5, 1);
        t.feed(b"\x1b[?1h\x1b=\x1b[?2004h\x1b[?25l\x1b[2 q");
        let modes = t.modes();
        assert!(modes.app_cursor && modes.app_keypad && modes.bracketed_paste);
        assert!(!modes.cursor_visible && !modes.cursor_blink);
        assert_eq!(t.cursor_shape(), CursorShape::Block);
        t.feed(b"\x1bc");
        assert_eq!(t.modes(), Modes::default());
    }

    #[test]
    fn modos_do_mouse_se_trocam_e_desligam() {
        let mut t = term(5, 1);
        assert_eq!(t.modes().mouse, MouseTracking::Off);
        assert!(t.modes().alternate_scroll && !t.modes().focus_events);
        t.feed(b"\x1b[?1000h");
        assert_eq!(t.modes().mouse, MouseTracking::Buttons);
        t.feed(b"\x1b[?1002h");
        assert_eq!(t.modes().mouse, MouseTracking::Drag);
        t.feed(b"\x1b[?1003h");
        assert_eq!(t.modes().mouse, MouseTracking::Motion);
        // Desligar qualquer um desliga o relato, como no xterm.
        t.feed(b"\x1b[?1000l");
        assert_eq!(t.modes().mouse, MouseTracking::Off);
        t.feed(b"\x1b[?9h");
        assert_eq!(t.modes().mouse, MouseTracking::X10);
        t.feed(b"\x1b[?9l");
        assert_eq!(t.modes().mouse, MouseTracking::Off);
        t.feed(b"\x1b[?1000;1006h");
        assert_eq!(t.modes().mouse, MouseTracking::Buttons);
        assert_eq!(t.modes().mouse_encoding, MouseEncoding::Sgr);
    }

    #[test]
    fn codificacoes_do_mouse_se_excluem() {
        let mut t = term(5, 1);
        t.feed(b"\x1b[?1006h");
        assert_eq!(t.modes().mouse_encoding, MouseEncoding::Sgr);
        t.feed(b"\x1b[?1015h");
        assert_eq!(t.modes().mouse_encoding, MouseEncoding::Urxvt);
        // Desligar a que não está ligada não muda nada.
        t.feed(b"\x1b[?1006l");
        assert_eq!(t.modes().mouse_encoding, MouseEncoding::Urxvt);
        t.feed(b"\x1b[?1015l");
        assert_eq!(t.modes().mouse_encoding, MouseEncoding::Default);
        t.feed(b"\x1b[?1005h");
        assert_eq!(t.modes().mouse_encoding, MouseEncoding::Utf8);
    }

    #[test]
    fn modos_do_mouse_valem_nas_duas_telas_e_o_ris_zera() {
        let mut t = term(5, 2);
        t.feed(b"\x1b[?1049h\x1b[?1002h\x1b[?1006h\x1b[?1004h\x1b[?1007l");
        t.feed(b"\x1b[?1049l");
        let modes = t.modes();
        assert_eq!(modes.mouse, MouseTracking::Drag);
        assert_eq!(modes.mouse_encoding, MouseEncoding::Sgr);
        assert!(modes.focus_events && !modes.alternate_scroll);
        // O DECSTR guarda o mouse, como o xterm; o RIS zera tudo.
        t.feed(b"\x1b[!p");
        assert_eq!(t.modes().mouse, MouseTracking::Drag);
        assert!(t.modes().focus_events);
        t.feed(b"\x1bc");
        assert_eq!(t.modes(), Modes::default());
        assert_eq!(t.modes().mouse, MouseTracking::Off);
        assert!(t.modes().alternate_scroll);
    }

    #[test]
    fn vista_rolada_nao_e_puxada_pela_saida() {
        let mut t = term(5, 2);
        for i in 0..10 {
            t.feed(format!("{i}\r\n").as_bytes());
        }
        t.scroll_display(3);
        let antes = t.view_row(0).text(0, 5);
        t.feed(b"x\r\ny\r\n");
        assert_eq!(t.view_row(0).text(0, 5), antes);
        assert_eq!(t.display_offset(), 5);
        t.set_display_offset(0);
        assert_eq!(t.view_row(1).text(0, 5), "");
    }

    #[test]
    fn redimensionar_reflui_a_principal() {
        let mut t = term(10, 3);
        t.feed(b"0123456789abc\r\n$ ");
        t.resize(5, 3);
        assert_eq!(t.all_text(), "0123456789abc\n$");
        assert_eq!(t.screen_text(), "56789\nabc\n$");
        assert_eq!(t.cursor(), (2, 2));
        t.resize(20, 3);
        assert_eq!(t.screen_text(), "0123456789abc\n$");
        assert_eq!(t.cursor(), (1, 2));
    }

    #[test]
    fn rep_repete_o_ultimo() {
        let mut t = term(10, 1);
        t.feed(b"a\x1b[4b");
        assert_eq!(t.line_text(0), "aaaaa");
    }

    #[test]
    fn ed3_apaga_o_histórico() {
        let mut t = term(5, 1);
        t.feed(b"a\r\nb\r\nc");
        assert_eq!(t.history_len(), 2);
        t.feed(b"\x1b[3J");
        assert_eq!(t.history_len(), 0);
    }

    /// O progresso que sobra depois de `bytes`.
    fn progresso(bytes: &[u8]) -> TermProgress {
        let mut t = term(10, 2);
        t.feed(bytes);
        t.progress()
    }

    #[test]
    fn osc_9_4_em_cada_estado() {
        for fim in ["\x07", "\x1b\\"] {
            let osc = |corpo: &str| format!("\x1b]9;4;{corpo}{fim}");
            assert_eq!(progresso(osc("0").as_bytes()), TermProgress::None);
            assert_eq!(progresso(osc("1;45").as_bytes()), TermProgress::Normal(45));
            assert_eq!(progresso(osc("2").as_bytes()), TermProgress::Error(None));
            assert_eq!(
                progresso(osc("2;30").as_bytes()),
                TermProgress::Error(Some(30))
            );
            assert_eq!(progresso(osc("3").as_bytes()), TermProgress::Indeterminate);
            assert_eq!(progresso(osc("4").as_bytes()), TermProgress::Paused(None));
            assert_eq!(
                progresso(osc("4;80").as_bytes()),
                TermProgress::Paused(Some(80))
            );
        }
    }

    #[test]
    fn osc_9_4_malformado() {
        // Ignorados: o progresso anterior fica.
        let antes = "\x1b]9;4;1;20\x07";
        for corpo in ["9;4;7;10", "9;4;x", "9;4;5", "9;4;-1;10"] {
            let bytes = format!("{antes}\x1b]{corpo}\x07");
            assert_eq!(
                progresso(bytes.as_bytes()),
                TermProgress::Normal(20),
                "{corpo}"
            );
        }
        let osc = |corpo: &str| progresso(format!("\x1b]{corpo}\x07").as_bytes());
        assert_eq!(osc("9;4;1;abc"), TermProgress::Normal(0));
        assert_eq!(osc("9;4;1;150"), TermProgress::Normal(100));
        assert_eq!(osc("9;4;1;-5"), TermProgress::Normal(0));
        assert_eq!(osc("9;4;1;99999999999999999999"), TermProgress::Normal(100));
        assert_eq!(osc("9;4;1;-99999999999999999999"), TermProgress::Normal(0));
        assert_eq!(osc("9;4;1;"), TermProgress::Normal(0));
        assert_eq!(osc("9;4;1"), TermProgress::Normal(0));
        assert_eq!(osc("9;4;2"), TermProgress::Error(None));
        // Sem estado, remove.
        let bytes = format!("{antes}\x1b]9;4\x07");
        assert_eq!(progresso(bytes.as_bytes()), TermProgress::None);
        let bytes = format!("{antes}\x1b]9;4;\x07");
        assert_eq!(progresso(bytes.as_bytes()), TermProgress::None);
    }

    #[test]
    fn pausar_ou_errar_sem_valor_mantem_a_porcentagem() {
        assert_eq!(
            progresso(b"\x1b]9;4;1;40\x07\x1b]9;4;4\x07"),
            TermProgress::Paused(Some(40))
        );
        assert_eq!(
            progresso(b"\x1b]9;4;1;40\x07\x1b]9;4;2\x07"),
            TermProgress::Error(Some(40))
        );
        assert_eq!(
            progresso(b"\x1b]9;4;1;40\x07\x1b]9;4;4;10\x07"),
            TermProgress::Paused(Some(10))
        );
        // Depois de remover não há o que manter.
        assert_eq!(
            progresso(b"\x1b]9;4;1;40\x07\x1b]9;4;0\x07\x1b]9;4;4\x07"),
            TermProgress::Paused(None)
        );
    }

    #[test]
    fn o_progresso_so_marca_mudanca_quando_muda() {
        let mut t = term(10, 2);
        assert_eq!(t.take_progress_change(), None);
        t.feed(b"\x1b]9;4;1;10\x07\x1b]9;4;1;10\x07");
        assert_eq!(t.take_progress_change(), Some(TermProgress::Normal(10)));
        assert_eq!(t.take_progress_change(), None);
        t.feed(b"\x1b]9;4;1;10\x07");
        assert_eq!(
            t.take_progress_change(),
            None,
            "o mesmo valor não é mudança"
        );
        t.feed(b"\x1b]9;4;1;11\x07");
        assert_eq!(t.take_progress_change(), Some(TermProgress::Normal(11)));
        // Sem progresso, remover de novo não muda nada.
        t.feed(b"\x1b]9;4;0\x07\x1b]9;4;0\x07");
        assert_eq!(t.take_progress_change(), Some(TermProgress::None));
        t.feed(b"\x1b]9;4;0\x07");
        assert_eq!(t.take_progress_change(), None);
    }

    #[test]
    fn o_ris_limpa_o_progresso() {
        let mut t = term(10, 2);
        t.feed(b"\x1b]9;4;3\x07");
        assert_eq!(t.take_progress_change(), Some(TermProgress::Indeterminate));
        t.feed(b"\x1bc");
        assert_eq!(t.progress(), TermProgress::None);
        assert_eq!(t.take_progress_change(), Some(TermProgress::None));
        // Sem progresso o RIS não marca mudança.
        t.feed(b"\x1bc");
        assert_eq!(t.take_progress_change(), None);
    }

    #[test]
    fn osc_9_com_texto_e_um_aviso() {
        let mut t = term(10, 2);
        t.feed("\x1b]9;Compilação terminada\x07".as_bytes());
        assert_eq!(
            t.take_notices(),
            [Notice {
                title: None,
                body: "Compilação terminada".to_owned()
            }]
        );
        assert!(t.take_notices().is_empty());
        // O texto pode ter `;`.
        t.feed(b"\x1b]9;um;dois\x1b\\");
        assert_eq!(t.take_notices()[0].body, "um;dois");
        // Os outros comandos da família, que só têm dígitos no segundo parâmetro, não são aviso.
        t.feed(b"\x1b]9;9;/home/x\x07\x1b]9;1;500\x07\x1b]9;\x07\x1b]9\x07");
        assert!(t.take_notices().is_empty());
        // `9;4` é o progresso, não um aviso.
        t.feed(b"\x1b]9;4;1;10\x07");
        assert!(t.take_notices().is_empty());
    }

    #[test]
    fn osc_777_notify_tem_titulo_e_corpo() {
        let mut t = term(10, 2);
        t.feed(b"\x1b]777;notify;Build;Pronto\x07");
        assert_eq!(
            t.take_notices(),
            [Notice {
                title: Some("Build".to_owned()),
                body: "Pronto".to_owned()
            }]
        );
        t.feed(b"\x1b]777;notify;Build;Pronto;sem erros\x07");
        assert_eq!(t.take_notices()[0].body, "Pronto;sem erros");
        // Só um parâmetro: é o corpo.
        t.feed(b"\x1b]777;notify;Pronto\x07");
        assert_eq!(
            t.take_notices(),
            [Notice {
                title: None,
                body: "Pronto".to_owned()
            }]
        );
        t.feed(b"\x1b]777;outra;coisa\x07\x1b]777;notify\x07");
        assert!(t.take_notices().is_empty());
    }

    #[test]
    fn osc_7_informa_o_diretorio() {
        let mut t = term(20, 3);
        assert_eq!(t.take_cwd(), None);
        t.feed(b"\x1b]7;file://host/home/ana/meu%20projeto\x07");
        assert_eq!(t.take_cwd().as_deref(), Some("/home/ana/meu projeto"));
        // Quem pegou não pega de novo.
        assert_eq!(t.take_cwd(), None);
        // O terminador ST e a máquina vazia valem também.
        t.feed(b"\x1b]7;file:///var/www\x1b\\");
        assert_eq!(t.take_cwd().as_deref(), Some("/var/www"));
        // Acentos vêm em UTF-8 percent-encoded.
        t.feed(b"\x1b]7;file://h/home/jo%C3%A3o\x07");
        assert_eq!(t.take_cwd().as_deref(), Some("/home/joão"));
    }

    #[test]
    fn osc_7_torto_nao_vira_diretorio() {
        let mut t = term(20, 3);
        t.feed(b"\x1b]7;/home/ana\x07\x1b]7;http://host/x\x07\x1b]7;file://host\x07\x1b]7;\x07");
        assert_eq!(t.take_cwd(), None);
        // Um `%` sem dois dígitos fica como veio.
        t.feed(b"\x1b]7;file://h/a%zz/100%\x07");
        assert_eq!(t.take_cwd().as_deref(), Some("/a%zz/100%"));
    }

    #[test]
    fn osc_52_pede_para_copiar() {
        let mut t = term(20, 3);
        t.feed(b"\x1b]52;c;aGVsbG8=\x07");
        assert_eq!(t.take_clipboard(), ["hello"]);
        assert!(t.take_clipboard().is_empty());
        // O alvo vazio e o ST também valem, e a ordem se mantém.
        t.feed(b"\x1b]52;;b25l\x1b\\\x1b]52;pc;dHdv\x07");
        assert_eq!(t.take_clipboard(), ["one", "two"]);
    }

    #[test]
    fn osc_52_ignora_leitura_base64_torto_e_exagero() {
        let mut t = term(20, 3);
        t.feed(b"\x1b]52;c;?\x07\x1b]52;c;@@@@\x07\x1b]52;c;aGVsbG8\x07\x1b]52;c\x07");
        assert!(t.take_clipboard().is_empty());
        // Mais de 1 MiB é ignorado; sem pânico.
        let grande = "QQ==".repeat(400_000);
        t.feed(format!("\x1b]52;c;{grande}\x07").as_bytes());
        assert!(t.take_clipboard().is_empty());
        // Bytes que não são UTF-8 viram o caractere de substituição.
        t.feed(b"\x1b]52;c;/w==\x07");
        assert_eq!(t.take_clipboard(), ["\u{fffd}"]);
    }

    fn uri_em(t: &Emulator, row: usize, col: usize) -> Option<String> {
        t.link_at(row, col).map(|l| l.uri().to_owned())
    }

    #[test]
    fn osc_8_marca_as_celulas_do_trecho() {
        let mut t = term(20, 2);
        t.feed(b"a\x1b]8;;https://x.org/p\x1b\\bc\x1b]8;;\x1b\\d");
        assert_eq!(uri_em(&t, 0, 0), None);
        assert_eq!(uri_em(&t, 0, 1).as_deref(), Some("https://x.org/p"));
        assert_eq!(uri_em(&t, 0, 2).as_deref(), Some("https://x.org/p"));
        assert_eq!(uri_em(&t, 0, 3), None);
        assert_eq!(t.line_text(0), "abcd");
    }

    #[test]
    fn osc_8_com_bel_e_com_ponto_e_virgula_na_uri() {
        let mut t = term(20, 2);
        t.feed(b"\x1b]8;;https://x.org/a;b=1;c\x07z\x1b]8;;\x07");
        assert_eq!(uri_em(&t, 0, 0).as_deref(), Some("https://x.org/a;b=1;c"));
    }

    #[test]
    fn osc_8_o_mesmo_id_e_uri_e_um_link_so() {
        let mut t = term(20, 3);
        t.feed(b"\x1b]8;id=k;https://x.org\x07a\x1b]8;;\x07 \x1b]8;id=k;https://x.org\x07b\x1b]8;;\x07");
        assert_eq!(t.links_len(), 1);
        assert_eq!(t.cell(0, 0).unwrap().link, t.cell(0, 2).unwrap().link);
        assert_eq!(t.link_at(0, 0).unwrap().id(), Some("k"));
        // Outra URI com o mesmo id é outro link; e sem id nada se junta.
        t.feed(b"\r\n\x1b]8;id=k;https://y.org\x07c\x1b]8;;\x07");
        assert_eq!(t.links_len(), 2);
        t.feed(b"\x1b]8;;https://z.org\x07d\x1b]8;;\x07\x1b]8;;https://z.org\x07e\x1b]8;;\x07");
        assert_eq!(t.links_len(), 4);
    }

    #[test]
    fn osc_8_o_id_vem_no_meio_dos_parametros() {
        let mut t = term(20, 2);
        t.feed(b"\x1b]8;foo=bar:id=meu:x=y;https://x.org\x07a\x1b]8;;\x07");
        assert_eq!(t.link_at(0, 0).unwrap().id(), Some("meu"));
    }

    #[test]
    fn osc_8_sobrevive_ao_sgr_zero_e_nao_marca_o_apagado() {
        let mut t = term(20, 2);
        t.feed(b"\x1b]8;;https://x.org\x07\x1b[1ma\x1b[0mb\x1b[K");
        assert!(t.cell(0, 0).unwrap().link.is_some());
        assert!(t.cell(0, 1).unwrap().link.is_some());
        t.feed(b"\r\x1b[2K");
        assert_eq!(t.cell(0, 0).unwrap().link, None, "apagar não marca");
    }

    #[test]
    fn osc_8_anda_com_a_celula_ao_rolar_e_ao_redimensionar() {
        let mut t = Emulator::new(6, 2, 10);
        t.feed(b"\x1b]8;;https://x.org\x07abcdef\x1b]8;;\x07\r\nxy\r\nz");
        // A primeira linha foi para o histórico.
        assert_eq!(t.history_len(), 1);
        t.set_display_offset(1);
        assert_eq!(uri_em(&t, 0, 3).as_deref(), Some("https://x.org"));
        t.set_display_offset(0);
        t.resize(3, 2);
        t.set_display_offset(t.history_len());
        assert!(
            (0..t.rows()).any(|r| uri_em(&t, r, 0).is_some()),
            "o link continua depois de refluir"
        );
    }

    #[test]
    fn osc_8_aberto_no_fim_de_uma_linha_marca_a_proxima() {
        let mut t = term(4, 3);
        t.feed(b"\x1b]8;;https://x.org\x07abcdef\x1b]8;;\x07");
        assert!(uri_em(&t, 1, 1).is_some());
        assert_eq!(uri_em(&t, 1, 2), None);
    }

    #[test]
    fn osc_8_malformado_nao_quebra() {
        let mut t = term(10, 2);
        t.feed(b"\x1b]8\x07a\x1b]8;\x07b\x1b]8;;\x07c");
        assert_eq!(t.link_at(0, 0), None);
        assert_eq!(t.line_text(0), "abc");
        assert_eq!(t.links_len(), 0);
    }

    #[test]
    fn osc_8_a_tabela_e_limpa_pelas_vivas_e_sem_teto() {
        let mut t = Emulator::new(10, 2, 0);
        // Cada link ocupa a linha inteira e some quando a tela rola: sem histórico, as
        // entradas anteriores ficam sem célula.
        for i in 0..1000 {
            t.feed(format!("\x1b]8;;https://x.org/{i}\x07l\x1b]8;;\x07\r\n").as_bytes());
        }
        assert!(t.links_len() < 200, "{}", t.links_len());
        // O último link continua certo, mesmo depois de os índices serem reescritos.
        t.feed(b"\x1b]8;;https://x.org/ultimo\x07u\x1b]8;;\x07");
        let row = t.cursor().0;
        assert_eq!(uri_em(&t, row, 0).as_deref(), Some("https://x.org/ultimo"));
    }

    #[test]
    fn osc_8_a_limpeza_guarda_o_link_ainda_aberto_e_o_do_histórico() {
        let mut t = Emulator::new(10, 2, 2000);
        t.feed(b"\x1b]8;id=a;https://x.org/primeiro\x07p\x1b]8;;\x07\r\n");
        t.feed(b"\x1b]8;id=b;https://x.org/aberto\x07");
        for i in 0..300 {
            // Entradas que ninguém usa mais: abertas e fechadas sem escrever.
            t.feed(format!("\x1b]8;;https://x.org/vazio{i}\x07\x1b]8;;\x07").as_bytes());
        }
        t.feed(b"\x1b]8;id=b;https://x.org/aberto\x07q");
        assert!(t.links_len() < 300);
        let cursor_row = t.cursor().0;
        assert_eq!(
            uri_em(&t, cursor_row, 0).as_deref(),
            Some("https://x.org/aberto")
        );
        assert_eq!(
            uri_em(&t, cursor_row - 1, 0).as_deref(),
            Some("https://x.org/primeiro")
        );
    }

    #[test]
    fn osc_8_o_ris_zera_a_tabela() {
        let mut t = term(10, 2);
        t.feed(b"\x1b]8;;https://x.org\x07a\x1bc");
        assert_eq!(t.links_len(), 0);
        t.feed(b"b");
        assert_eq!(t.link_at(0, 0), None);
    }

    #[test]
    fn osc_8_o_texto_largo_leva_as_duas_celulas() {
        let mut t = term(10, 2);
        t.feed("\x1b]8;;https://x.org\x07日\x1b]8;;\x07".as_bytes());
        assert!(t.cell(0, 0).unwrap().link.is_some());
        assert!(t.cell(0, 1).unwrap().link.is_some());
    }
}
