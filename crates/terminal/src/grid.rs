//! A grade de células: a tela, o histórico e o refluxo das linhas quando a largura muda.

use std::collections::VecDeque;

use crate::cell::{Cell, Flags, Pen};

/// Uma linha da grade.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Row {
    /// As células, uma por coluna.
    pub cells: Vec<Cell>,
    /// Se o texto continua na linha de baixo (a quebra automática na última coluna). Ao
    /// refluir, as linhas assim se juntam numa só antes de quebrar na largura nova.
    pub wrapped: bool,
}

impl Row {
    /// Linha vazia com `cols` colunas.
    pub fn blank(cols: usize) -> Self {
        Self {
            cells: vec![Cell::BLANK; cols],
            wrapped: false,
        }
    }

    /// Linha vazia com o fundo de `pen`.
    pub fn erased(cols: usize, pen: &Pen) -> Self {
        Self {
            cells: vec![Cell::erased(pen); cols],
            wrapped: false,
        }
    }

    /// Esvazia a linha com o fundo de `pen`, reaproveitando a memória.
    pub fn reset(&mut self, cols: usize, pen: &Pen) {
        self.cells.clear();
        self.cells.resize(cols, Cell::erased(pen));
        self.wrapped = false;
    }

    /// Colunas até a última célula que aparece (o resto é vazio).
    pub fn content_len(&self) -> usize {
        self.cells
            .iter()
            .rposition(|cell| !cell.is_blank())
            .map_or(0, |i| i + 1)
    }

    /// Se a linha não tem nada que apareça.
    pub fn is_blank(&self) -> bool {
        !self.wrapped && self.content_len() == 0
    }

    /// Texto das colunas `start..end`, sem os espaços do fim.
    pub fn text(&self, start: usize, end: usize) -> String {
        let end = end.min(self.cells.len());
        let mut text = String::new();
        for cell in self.cells.get(start..end).unwrap_or_default() {
            if cell.flags.intersects(Flags::WIDE_SPACER | Flags::WIDE_PAD) {
                continue;
            }
            text.push_str(&cell.text());
        }
        let trimmed = text.trim_end_matches(' ').len();
        text.truncate(trimmed);
        text
    }

    /// Muda para `cols` colunas sem refluir, cortando ou completando com células vazias. Um
    /// caractere largo cortado ao meio vira espaço.
    pub fn resize(&mut self, cols: usize) {
        self.cells.resize(cols, Cell::BLANK);
        if let Some(last) = self.cells.last_mut()
            && last.flags.contains(Flags::WIDE)
        {
            *last = Cell::BLANK;
        }
    }
}

/// A tela e o histórico.
#[derive(Clone, Debug)]
pub struct Grid {
    /// Colunas.
    pub cols: usize,
    /// Linhas da tela.
    pub rows: usize,
    /// As linhas da tela, de cima para baixo.
    pub lines: Vec<Row>,
    /// As linhas que saíram pelo topo, da mais antiga para a mais nova.
    pub history: VecDeque<Row>,
    /// Máximo de linhas no histórico.
    pub max_history: usize,
    /// Quantas linhas já saíram do começo do histórico (para as posições absolutas da
    /// seleção não mudarem quando o histórico anda).
    pub dropped: u64,
}

impl Grid {
    /// Muda o máximo do histórico; o que passar do limite sai do começo.
    pub fn set_max_history(&mut self, max_history: usize) {
        self.max_history = max_history;
        if self.history.len() > max_history {
            let extra = self.history.len() - max_history;
            self.history.drain(..extra);
            self.dropped += extra as u64;
        }
    }

    /// Grade vazia de `cols` por `rows`, com até `max_history` linhas de histórico.
    pub fn new(cols: usize, rows: usize, max_history: usize) -> Self {
        let (cols, rows) = (cols.max(1), rows.max(1));
        Self {
            cols,
            rows,
            lines: vec![Row::blank(cols); rows],
            history: VecDeque::new(),
            max_history,
            dropped: 0,
        }
    }

    /// Total de linhas: o histórico e a tela.
    pub fn total_lines(&self) -> usize {
        self.history.len() + self.rows
    }

    /// A linha `index` contando do começo do histórico.
    pub fn line(&self, index: usize) -> Option<&Row> {
        if index < self.history.len() {
            self.history.get(index)
        } else {
            self.lines.get(index - self.history.len())
        }
    }

    /// Guarda `row` no fim do histórico. Devolve a linha que saiu do começo, se o histórico
    /// estava cheio, para reaproveitar a memória.
    fn push_history(&mut self, row: Row) -> Option<Row> {
        if self.max_history == 0 {
            self.dropped += 1;
            return Some(row);
        }
        let old = if self.history.len() >= self.max_history {
            self.dropped += 1;
            self.history.pop_front()
        } else {
            None
        };
        self.history.push_back(row);
        old
    }

    /// Sobe as linhas `top..=bottom` em `n`, com linhas vazias embaixo. Com `keep`, as que
    /// saem pelo topo vão para o histórico. Devolve quantas foram para o histórico.
    pub fn scroll_up(
        &mut self,
        top: usize,
        bottom: usize,
        n: usize,
        pen: &Pen,
        keep: bool,
    ) -> usize {
        let n = n.min(bottom + 1 - top);
        if n == 0 {
            return 0;
        }
        let cols = self.cols;
        self.lines[top..=bottom].rotate_left(n);
        let mut kept = 0;
        for i in bottom + 1 - n..=bottom {
            if keep {
                let spare = std::mem::take(&mut self.lines[i]);
                kept += 1;
                if let Some(mut old) = self.push_history(spare) {
                    old.reset(cols, pen);
                    self.lines[i] = old;
                    continue;
                }
            }
            self.lines[i].reset(cols, pen);
        }
        kept
    }

    /// Desce as linhas `top..=bottom` em `n`, com linhas vazias em cima.
    pub fn scroll_down(&mut self, top: usize, bottom: usize, n: usize, pen: &Pen) {
        let n = n.min(bottom + 1 - top);
        if n == 0 {
            return;
        }
        let cols = self.cols;
        self.lines[top..=bottom].rotate_right(n);
        for row in &mut self.lines[top..top + n] {
            row.reset(cols, pen);
        }
    }

    /// Apaga o histórico.
    pub fn clear_history(&mut self) {
        self.dropped += self.history.len() as u64;
        self.history.clear();
    }

    /// Muda o tamanho sem refluir (a tela alternativa): as linhas são cortadas ou completadas,
    /// e as de cima saem se o cursor ficaria fora. Devolve a linha nova do cursor.
    pub fn resize_truncate(&mut self, cols: usize, rows: usize, cursor_row: usize) -> usize {
        let (cols, rows) = (cols.max(1), rows.max(1));
        let mut cursor_row = cursor_row;
        if rows < self.rows {
            let from_top = (cursor_row + 1).saturating_sub(rows);
            self.lines.drain(..from_top);
            self.lines.truncate(rows);
            cursor_row -= from_top;
        }
        for row in &mut self.lines {
            row.resize(cols);
        }
        self.lines.resize(rows, Row::blank(cols));
        self.cols = cols;
        self.rows = rows;
        cursor_row
    }

    /// Muda o tamanho refluindo o texto: as linhas quebradas automaticamente se juntam e
    /// quebram de novo na largura nova, inclusive as do histórico. O cursor acompanha o
    /// caractere em que estava. Se a tela estava cheia, o fim do texto continua no fim da
    /// tela; senão, o topo fica onde estava, e crescer traz linhas do histórico de volta.
    /// Devolve a posição nova do cursor.
    pub fn reflow(&mut self, cols: usize, rows: usize, cursor: (usize, usize)) -> (usize, usize) {
        let (cols, rows) = (cols.max(1), rows.max(1));
        let old_rows = self.rows;
        let old_history = self.history.len();
        let mut all: Vec<Row> = self.history.drain(..).collect();
        all.append(&mut self.lines);
        let cursor_abs = old_history + cursor.0.min(old_rows - 1);
        let content_end = all
            .iter()
            .rposition(|row| !row.is_blank())
            .map_or(0, |i| i + 1);
        let end = content_end.max(cursor_abs + 1);
        let screen_full = end == old_history + old_rows;
        all.truncate(end);

        // As linhas lógicas: cada uma junta as linhas da grade até a que não quebrou.
        let mut logical: Vec<Vec<Cell>> = Vec::new();
        let mut current: Vec<Cell> = Vec::new();
        let mut cursor_line = (0, 0);
        let mut top_line = (0, 0);
        for (index, mut row) in all.into_iter().enumerate() {
            if index == cursor_abs {
                cursor_line = (logical.len(), current.len() + cursor.1);
            }
            if index == old_history {
                top_line = (logical.len(), current.len());
            }
            if row.wrapped {
                if row
                    .cells
                    .last()
                    .is_some_and(|c| c.flags.contains(Flags::WIDE_PAD))
                {
                    row.cells.pop();
                }
            } else {
                let len = row.content_len();
                row.cells.truncate(len);
            }
            current.append(&mut row.cells);
            if !row.wrapped {
                logical.push(std::mem::take(&mut current));
            }
        }
        if !current.is_empty() {
            logical.push(current);
        }

        // Quebra de novo, anotando onde ficam o cursor e o antigo topo da tela.
        let mut out: Vec<Row> = Vec::new();
        let mut new_cursor = (0, 0);
        let mut new_top = 0;
        for (line_index, cells) in logical.into_iter().enumerate() {
            let first = out.len();
            let mut row = Row::blank(0);
            let mut positions = Vec::with_capacity(cells.len());
            for cell in cells {
                if cell.flags.contains(Flags::WIDE_SPACER) {
                    positions.push((out.len(), row.cells.len()));
                    row.cells.push(cell);
                    continue;
                }
                let width = if cell.flags.contains(Flags::WIDE) {
                    2
                } else {
                    1
                };
                if row.cells.len() + width > cols {
                    if width == 2 && row.cells.len() < cols {
                        let mut pad = Cell::BLANK;
                        pad.flags = Flags::WIDE_PAD;
                        row.cells.push(pad);
                    }
                    row.wrapped = true;
                    out.push(std::mem::replace(&mut row, Row::blank(0)));
                }
                positions.push((out.len(), row.cells.len()));
                row.cells.push(cell);
            }
            let tail = (out.len(), row.cells.len());
            out.push(row);
            let place = |offset: usize| -> (usize, usize) {
                if let Some(&pos) = positions.get(offset) {
                    return pos;
                }
                let past = tail.1 + (offset - positions.len());
                (tail.0 + past / cols, past % cols)
            };
            if line_index == cursor_line.0 {
                new_cursor = place(cursor_line.1);
                while out.len() <= new_cursor.0 {
                    if let Some(last) = out.last_mut() {
                        last.wrapped = true;
                    }
                    out.push(Row::blank(0));
                }
            }
            if line_index == top_line.0 {
                new_top = place(top_line.1).0.max(first);
            }
        }
        for row in &mut out {
            row.cells.resize(cols, Cell::BLANK);
        }

        let mut top = if screen_full {
            out.len().saturating_sub(rows)
        } else {
            let grown = rows.saturating_sub(old_rows);
            new_top.saturating_sub(grown)
        };
        top = top
            .max((new_cursor.0 + 1).saturating_sub(rows))
            .min(new_cursor.0);
        let mut screen = out.split_off(top.min(out.len()));
        screen.truncate(rows);
        screen.resize(rows, Row::blank(cols));
        let mut history: VecDeque<Row> = out.into();
        if history.len() > self.max_history {
            let extra = history.len() - self.max_history;
            history.drain(..extra);
            self.dropped += extra as u64;
        }
        self.history = history;
        self.lines = screen;
        self.cols = cols;
        self.rows = rows;
        (new_cursor.0 - top, new_cursor.1.min(cols - 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid_with(cols: usize, rows: usize, lines: &[(&str, bool)]) -> Grid {
        let mut grid = Grid::new(cols, rows, 100);
        for (i, (text, wrapped)) in lines.iter().enumerate() {
            for (col, c) in text.chars().enumerate() {
                grid.lines[i].cells[col] = Cell::with(c, &Pen::default());
            }
            grid.lines[i].wrapped = *wrapped;
        }
        grid
    }

    fn screen(grid: &Grid) -> Vec<String> {
        grid.lines.iter().map(|r| r.text(0, grid.cols)).collect()
    }

    #[test]
    fn rolar_guarda_no_historico() {
        let mut grid = grid_with(4, 2, &[("ab", false), ("cd", false)]);
        assert_eq!(grid.scroll_up(0, 1, 1, &Pen::default(), true), 1);
        assert_eq!(screen(&grid), ["cd", ""]);
        assert_eq!(grid.history[0].text(0, 4), "ab");
        grid.scroll_down(0, 1, 1, &Pen::default());
        assert_eq!(screen(&grid), ["", "cd"]);
    }

    #[test]
    fn historico_tem_teto() {
        let mut grid = Grid::new(2, 1, 3);
        for c in ['a', 'b', 'c', 'd', 'e'] {
            grid.lines[0].cells[0] = Cell::with(c, &Pen::default());
            grid.scroll_up(0, 0, 1, &Pen::default(), true);
        }
        let texto: Vec<String> = grid.history.iter().map(|r| r.text(0, 2)).collect();
        assert_eq!(texto, ["c", "d", "e"]);
        assert_eq!(grid.dropped, 2);
    }

    #[test]
    fn estreitar_quebra_e_alargar_junta() {
        let mut grid = grid_with(6, 3, &[("abcdef", true), ("gh", false), ("$", false)]);
        let cursor = grid.reflow(4, 3, (2, 1));
        assert_eq!(screen(&grid), ["abcd", "efgh", "$"]);
        assert_eq!(cursor, (2, 1));
        let cursor = grid.reflow(3, 3, cursor);
        assert_eq!(screen(&grid), ["def", "gh", "$"]);
        assert_eq!(grid.history[0].text(0, 3), "abc");
        assert!(grid.history[0].wrapped);
        assert_eq!(cursor, (2, 1));
        let cursor = grid.reflow(10, 3, cursor);
        assert_eq!(screen(&grid), ["abcdefgh", "$", ""]);
        assert!(grid.history.is_empty());
        assert_eq!(cursor, (1, 1));
    }

    #[test]
    fn crescer_traz_o_historico_de_volta() {
        let mut grid = grid_with(4, 2, &[("um", false), ("dois", false)]);
        grid.scroll_up(0, 1, 1, &Pen::default(), true);
        grid.lines[1].cells[0] = Cell::with('$', &Pen::default());
        let cursor = grid.reflow(4, 3, (1, 1));
        assert_eq!(screen(&grid), ["um", "dois", "$"]);
        assert_eq!(cursor, (2, 1));
    }

    #[test]
    fn encolher_sem_tela_cheia_mantem_o_topo() {
        let mut grid = grid_with(4, 5, &[("$", false)]);
        let cursor = grid.reflow(4, 3, (0, 2));
        assert_eq!(screen(&grid), ["$", "", ""]);
        assert_eq!(cursor, (0, 2));
        assert!(grid.history.is_empty());
    }

    #[test]
    fn largo_nao_fica_partido_ao_refluir() {
        let mut grid = Grid::new(4, 2, 10);
        let pen = Pen::default();
        grid.lines[0].cells[0] = Cell::with('a', &pen);
        grid.lines[0].cells[1] = Cell::with('b', &pen);
        let mut wide = Cell::with('日', &pen);
        wide.flags = Flags::WIDE;
        grid.lines[0].cells[2] = wide;
        let mut spacer = Cell::BLANK;
        spacer.flags = Flags::WIDE_SPACER;
        grid.lines[0].cells[3] = spacer;
        let cursor = grid.reflow(3, 2, (1, 0));
        assert_eq!(grid.history[0].text(0, 3), "ab");
        assert!(grid.history[0].cells[2].flags.contains(Flags::WIDE_PAD));
        assert_eq!(grid.lines[0].text(0, 3), "日");
        assert_eq!(cursor, (1, 0));
        grid.reflow(4, 2, cursor);
        assert_eq!(grid.lines[0].text(0, 4), "ab日");
    }

    #[test]
    fn tela_alternativa_so_corta() {
        let mut grid = grid_with(4, 3, &[("abcd", false), ("x", false), ("y", false)]);
        let row = grid.resize_truncate(2, 2, 2);
        assert_eq!(screen(&grid), ["x", "y"]);
        assert_eq!(row, 1);
    }
}
