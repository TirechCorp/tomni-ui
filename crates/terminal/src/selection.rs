//! A seleção de texto: arrastando (contínua), com duplo clique (palavras), triplo clique
//! (linhas) e com o Alt (um bloco retangular).

use crate::cell::Flags;
use crate::emulator::Emulator;
use crate::grid::Row;

/// Os separadores de palavra do duplo clique, os mesmos do `wordDelimiters` do Windows
/// Terminal.
pub const WORD_DELIMITERS: &str = " /\\()\"'-.,:;<>~!@#$%^&*|+=[]{}~?\u{2502}";

/// Uma posição na grade: a linha absoluta (que não muda quando o histórico anda; ver
/// [`Emulator::view_line`]) e a coluna.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GridPoint {
    /// Linha absoluta.
    pub line: u64,
    /// Coluna.
    pub col: usize,
}

impl GridPoint {
    /// Posição na `line`, coluna `col`.
    pub const fn new(line: u64, col: usize) -> Self {
        Self { line, col }
    }
}

/// Como a seleção cresce.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SelectionKind {
    /// Do ponto em que começou até o ponteiro, na ordem de leitura.
    #[default]
    Simple,
    /// Palavras inteiras (duplo clique).
    Word,
    /// Linhas inteiras (triplo clique).
    Line,
    /// Um retângulo de colunas (arrastar com o Alt).
    Block,
}

/// Uma seleção.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Selection {
    /// Como cresce.
    pub kind: SelectionKind,
    /// Onde começou.
    pub anchor: GridPoint,
    /// Onde está o ponteiro.
    pub head: GridPoint,
}

fn is_word(c: char) -> bool {
    !WORD_DELIMITERS.contains(c)
}

impl Selection {
    /// Seleção que começa e termina em `point`.
    pub fn new(kind: SelectionKind, point: GridPoint) -> Self {
        Self {
            kind,
            anchor: point,
            head: point,
        }
    }

    /// Começo e fim (inclusive), em ordem e já crescidos até as palavras ou as linhas. No
    /// bloco, as colunas vão da menor à maior.
    pub fn bounds(&self, emu: &Emulator) -> (GridPoint, GridPoint) {
        let (mut start, mut end) = if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        };
        let last_col = emu.cols().saturating_sub(1);
        match self.kind {
            SelectionKind::Simple => {}
            SelectionKind::Block => {
                let (a, b) = (self.anchor.col, self.head.col);
                start.col = a.min(b);
                end.col = a.max(b);
            }
            SelectionKind::Word => {
                start = word_edge(emu, start, false);
                end = word_edge(emu, end, true);
            }
            SelectionKind::Line => {
                let (first, _) = emu.line_bounds();
                while start.line > first && emu.line_at(start.line - 1).is_some_and(|r| r.wrapped) {
                    start.line -= 1;
                }
                while emu.line_at(end.line).is_some_and(|r| r.wrapped)
                    && emu.line_at(end.line + 1).is_some()
                {
                    end.line += 1;
                }
                start.col = 0;
                end.col = last_col;
            }
        }
        (start, end.min(GridPoint::new(end.line, last_col)))
    }

    /// Colunas selecionadas na linha absoluta `line` (`start..end`), se alguma.
    pub fn columns(&self, emu: &Emulator, line: u64) -> Option<(usize, usize)> {
        let (start, end) = self.bounds(emu);
        if line < start.line || line > end.line {
            return None;
        }
        let cols = emu.cols();
        if self.kind == SelectionKind::Block {
            return Some((start.col, end.col + 1));
        }
        let from = if line == start.line { start.col } else { 0 };
        let to = if line == end.line { end.col + 1 } else { cols };
        (from < to).then_some((from, to))
    }

    /// Se a célula está selecionada.
    pub fn contains(&self, emu: &Emulator, point: GridPoint) -> bool {
        self.columns(emu, point.line)
            .is_some_and(|(from, to)| (from..to).contains(&point.col))
    }

    /// O texto selecionado: as linhas quebradas automaticamente se juntam, e cada linha perde
    /// os espaços do fim. No bloco, cada linha do retângulo é uma linha do texto.
    pub fn text(&self, emu: &Emulator) -> String {
        let (start, end) = self.bounds(emu);
        let mut text = String::new();
        for line in start.line..=end.line {
            let Some(row) = emu.line_at(line) else {
                continue;
            };
            let Some((from, to)) = self.columns(emu, line) else {
                continue;
            };
            let joined = self.kind != SelectionKind::Block && row.wrapped && line < end.line;
            text.push_str(&row_text(row, from, to, !joined));
            if line < end.line && !joined {
                text.push('\n');
            }
        }
        text
    }
}

/// O texto das colunas `from..to`, sem os espaços do fim se `trim`.
fn row_text(row: &Row, from: usize, to: usize, trim: bool) -> String {
    let mut text = String::new();
    let to = to.min(row.cells.len());
    for cell in row.cells.get(from..to).unwrap_or_default() {
        if cell.flags.intersects(Flags::WIDE_SPACER | Flags::WIDE_PAD) {
            continue;
        }
        text.push_str(&cell.text());
    }
    if trim {
        let len = text.trim_end_matches(' ').len();
        text.truncate(len);
    }
    text
}

/// O caractere em `point`; o da primeira metade nos largos.
fn char_at(emu: &Emulator, point: GridPoint) -> Option<char> {
    let row = emu.line_at(point.line)?;
    let mut col = point.col.min(row.cells.len().checked_sub(1)?);
    if row.cells[col].flags.contains(Flags::WIDE_SPACER) && col > 0 {
        col -= 1;
    }
    Some(row.cells[col].c)
}

/// Anda de `point` até a ponta da palavra (ou do trecho de separadores) em que ele está,
/// passando para a linha vizinha se a quebra foi automática.
fn word_edge(emu: &Emulator, point: GridPoint, forward: bool) -> GridPoint {
    let Some(c) = char_at(emu, point) else {
        return point;
    };
    let class = is_word(c);
    let cols = emu.cols();
    let mut current = point;
    loop {
        let next = if forward {
            if current.col + 1 < cols {
                GridPoint::new(current.line, current.col + 1)
            } else if emu.line_at(current.line).is_some_and(|r| r.wrapped) {
                GridPoint::new(current.line + 1, 0)
            } else {
                return current;
            }
        } else if current.col > 0 {
            GridPoint::new(current.line, current.col - 1)
        } else if current.line > 0 && emu.line_at(current.line - 1).is_some_and(|r| r.wrapped) {
            GridPoint::new(current.line - 1, cols - 1)
        } else {
            return current;
        };
        match char_at(emu, next) {
            Some(c) if is_word(c) == class => current = next,
            _ => return current,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emu(text: &[u8]) -> Emulator {
        let mut emu = Emulator::new(10, 3, 100);
        emu.feed(text);
        emu
    }

    fn p(line: u64, col: usize) -> GridPoint {
        GridPoint::new(line, col)
    }

    #[test]
    fn simples_na_ordem_de_leitura() {
        let emu = emu(b"abc def\r\nghi jkl");
        let mut sel = Selection::new(SelectionKind::Simple, p(1, 2));
        sel.head = p(0, 4);
        assert_eq!(sel.text(&emu), "def\nghi");
        assert!(sel.contains(&emu, p(0, 9)));
        assert!(!sel.contains(&emu, p(1, 3)));
    }

    #[test]
    fn palavra_com_duplo_clique() {
        let emu = emu(b"ls /usr/bin");
        let sel = Selection::new(SelectionKind::Word, p(0, 5));
        assert_eq!(sel.text(&emu), "usr");
        let sel = Selection::new(SelectionKind::Word, p(0, 0));
        assert_eq!(sel.text(&emu), "ls");
    }

    #[test]
    fn palavra_atravessa_a_quebra_automatica() {
        let emu = emu(b"aaaa bbbbbbcc dd");
        let sel = Selection::new(SelectionKind::Word, p(1, 0));
        assert_eq!(sel.text(&emu), "bbbbbbcc");
    }

    #[test]
    fn linha_inteira_com_triplo_clique() {
        let emu = emu(b"0123456789abc\r\nxyz");
        let sel = Selection::new(SelectionKind::Line, p(1, 1));
        assert_eq!(sel.text(&emu), "0123456789abc");
    }

    #[test]
    fn bloco_com_alt() {
        let emu = emu(b"abcdef\r\nghijkl\r\nmnopqr");
        let mut sel = Selection::new(SelectionKind::Block, p(0, 4));
        sel.head = p(2, 1);
        assert_eq!(sel.text(&emu), "bcde\nhijk\nnopq");
        assert!(!sel.contains(&emu, p(1, 5)));
    }

    #[test]
    fn largos_e_acentos_no_texto() {
        let emu = emu("日本 é".as_bytes());
        let mut sel = Selection::new(SelectionKind::Simple, p(0, 0));
        sel.head = p(0, 9);
        assert_eq!(sel.text(&emu), "日本 é");
    }
}
