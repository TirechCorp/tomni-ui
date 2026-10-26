//! O que o `Drawer` e as ferramentas precisam saber do `DataGrid`, do cabeçalho e das linhas, e
//! a geometria que o widget (cliques, divisores) e o tema (o desenho) dividem.

use std::sync::Arc;

use kurbo::Rect;

use super::model::{DataGridColumn, DataGridFlat, DataGridRow, SortDirection, resolve_widths};
use crate::button::ButtonState;
use crate::density::Density;

/// Altura de uma linha: o `ListViewItem` do WinUI compacto, com a margem de 2 px em cima e
/// embaixo da pílula. Fica abaixo dos 44 px de alvo de toque: o lint abre exceção para as
/// linhas, como para a barra de menus (o mouse e o teclado são a entrada de uma tabela).
pub const DATA_GRID_ROW_HEIGHT: f64 = 32.0;
/// Altura de uma linha no modo compacto.
pub const DATA_GRID_ROW_HEIGHT_COMPACT: f64 = 24.0;
/// Altura do cabeçalho (o `DataGridColumnHeader` do Toolkit).
pub const DATA_GRID_HEADER_HEIGHT: f64 = 32.0;
/// Altura do cabeçalho no modo compacto.
pub const DATA_GRID_HEADER_HEIGHT_COMPACT: f64 = 24.0;
/// Espaço entre a borda da célula e o texto, dos dois lados.
pub const DATA_GRID_CELL_PADDING: f64 = 12.0;
/// Lado do ícone da primeira coluna.
pub const DATA_GRID_ICON: f64 = 16.0;
/// Espaço entre o ícone e o texto.
pub const DATA_GRID_ICON_GAP: f64 = 8.0;
/// Quanto de cada lado de um divisor de coluna ainda o pega.
pub const DATA_GRID_DIVIDER_HIT: f64 = 4.0;
/// Margem lateral da pílula da linha no padrão (`ListViewItemMargin`); no compacto é zero.
const SIDE: f64 = 4.0;
/// Margem de cima e de baixo da pílula no padrão.
const VERTICAL: f64 = 2.0;

/// Altura de uma linha na densidade `density`.
pub(crate) fn row_height(density: Density) -> f64 {
    match density {
        Density::Standard => DATA_GRID_ROW_HEIGHT,
        Density::Compact => DATA_GRID_ROW_HEIGHT_COMPACT,
    }
}

/// Altura do cabeçalho na densidade `density`.
pub(crate) fn header_height(density: Density) -> f64 {
    match density {
        Density::Standard => DATA_GRID_HEADER_HEIGHT,
        Density::Compact => DATA_GRID_HEADER_HEIGHT_COMPACT,
    }
}

fn margin(density: Density) -> (f64, f64) {
    match density {
        Density::Standard => (SIDE, VERTICAL),
        Density::Compact => (0.0, 0.0),
    }
}

/// A faixa das células: a largura do retângulo menos a margem lateral. As células do
/// cabeçalho e das linhas saem dela, por isso as colunas alinham.
fn band(rect: Rect, density: Density) -> Rect {
    let (x, _) = margin(density);
    Rect::new(
        rect.x0 + x,
        rect.y0,
        (rect.x1 - x).max(rect.x0 + x),
        rect.y1,
    )
}

/// Os retângulos das células de `columns` na faixa `band`.
pub fn data_grid_cells(columns: &[DataGridColumn], band: Rect) -> Vec<Rect> {
    let mut x = band.x0;
    resolve_widths(columns, band.width())
        .into_iter()
        .map(|w| {
            let cell = Rect::new(x, band.y0, x + w, band.y1);
            x += w;
            cell
        })
        .collect()
}

/// Tudo o que o `Drawer` precisa saber de uma linha do `DataGrid` para pintá-la.
#[derive(Clone, Debug)]
pub struct DataGridRowVisual {
    /// A linha.
    pub row: Arc<DataGridRow>,
    /// As colunas, com as larguras que o usuário deu.
    pub columns: Arc<Vec<DataGridColumn>>,
    /// Se é uma das linhas selecionadas.
    pub selected: bool,
    /// Estado.
    pub state: ButtonState,
    /// Anel de foco: só com foco vindo do teclado.
    pub focus_ring: bool,
    /// Se o nome foi trocado por um campo de texto (o `Edit` é filho da linha).
    pub renaming: bool,
    /// Se o arrasto vai soltar nesta linha (uma pasta).
    pub drop: bool,
    /// Densidade.
    pub density: Density,
}

impl PartialEq for DataGridRowVisual {
    fn eq(&self, other: &Self) -> bool {
        self.selected == other.selected
            && self.state == other.state
            && self.focus_ring == other.focus_ring
            && self.renaming == other.renaming
            && self.drop == other.drop
            && self.density == other.density
            && (Arc::ptr_eq(&self.row, &other.row) || self.row == other.row)
            && (Arc::ptr_eq(&self.columns, &other.columns) || self.columns == other.columns)
    }
}

impl DataGridRowVisual {
    /// Altura da linha.
    pub fn height(&self) -> f64 {
        row_height(self.density)
    }

    /// O fundo da linha: o retângulo menos as margens.
    pub fn pill(&self, rect: Rect) -> Rect {
        let (x, y) = margin(self.density);
        Rect::new(
            rect.x0 + x,
            rect.y0 + y,
            (rect.x1 - x).max(rect.x0 + x),
            (rect.y0 + self.height() - y).max(rect.y0 + y),
        )
    }

    /// As células, de cima a baixo da pílula.
    pub fn cells(&self, rect: Rect) -> Vec<Rect> {
        let pill = self.pill(rect);
        data_grid_cells(&self.columns, pill)
    }

    /// Onde o nome começa na primeira coluna (depois do ícone, se a linha tem).
    pub fn label_x(&self, rect: Rect) -> f64 {
        let first = self.cells(rect).first().map_or(rect.x0, |c| c.x0);
        let icon = if self.row.icon.is_some() {
            DATA_GRID_ICON + DATA_GRID_ICON_GAP
        } else {
            0.0
        };
        first + DATA_GRID_CELL_PADDING + icon
    }

    /// O quadrado do ícone.
    pub fn icon_square(&self, rect: Rect) -> Rect {
        let pill = self.pill(rect);
        let x = self.cells(rect).first().map_or(rect.x0, |c| c.x0) + DATA_GRID_CELL_PADDING;
        let y = pill.y0 + ((pill.height() - DATA_GRID_ICON) / 2.0).floor();
        Rect::new(x, y, x + DATA_GRID_ICON, y + DATA_GRID_ICON)
    }
}

/// Tudo o que o `Drawer` precisa saber do cabeçalho.
#[derive(Clone, Debug)]
pub struct DataGridHeaderVisual {
    /// As colunas, com as larguras que o usuário deu.
    pub columns: Arc<Vec<DataGridColumn>>,
    /// A coluna ordenada e o sentido.
    pub sort: Option<(usize, SortDirection)>,
    /// A coluna sob o ponteiro.
    pub hovered: Option<usize>,
    /// A coluna pressionada.
    pub pressed: Option<usize>,
    /// O divisor (pela coluna à esquerda dele) que o usuário arrasta.
    pub resizing: Option<usize>,
    /// Se o ponteiro está sobre um divisor (que ganha o realce).
    pub divider_hover: Option<usize>,
    /// Anel de foco do cabeçalho inteiro: só com foco vindo do teclado.
    pub focus_ring: Option<usize>,
    /// Densidade.
    pub density: Density,
}

impl PartialEq for DataGridHeaderVisual {
    fn eq(&self, other: &Self) -> bool {
        self.sort == other.sort
            && self.hovered == other.hovered
            && self.pressed == other.pressed
            && self.resizing == other.resizing
            && self.divider_hover == other.divider_hover
            && self.focus_ring == other.focus_ring
            && self.density == other.density
            && (Arc::ptr_eq(&self.columns, &other.columns) || self.columns == other.columns)
    }
}

impl DataGridHeaderVisual {
    /// Altura do cabeçalho.
    pub fn height(&self) -> f64 {
        header_height(self.density)
    }

    /// As células do cabeçalho em `rect`.
    pub fn cells(&self, rect: Rect) -> Vec<Rect> {
        data_grid_cells(&self.columns, band(rect, self.density))
    }

    /// A coluna sob o ponto de x `x`.
    pub fn column_at(&self, rect: Rect, x: f64) -> Option<usize> {
        self.cells(rect)
            .iter()
            .position(|cell| x >= cell.x0 && x < cell.x1)
    }

    /// O divisor (pelo índice da coluna à esquerda dele) sob o ponto de x `x`.
    pub fn divider_at(&self, rect: Rect, x: f64) -> Option<usize> {
        self.cells(rect)
            .iter()
            .position(|cell| (cell.x1 - x).abs() <= DATA_GRID_DIVIDER_HIT)
    }
}

/// Tudo o que as ferramentas e o desenho precisam saber do `DataGrid`.
#[derive(Clone, Debug)]
pub struct DataGridVisual {
    /// As linhas visíveis, em ordem.
    pub rows: Arc<DataGridFlat>,
    /// As colunas, com as larguras que o usuário deu.
    pub columns: Arc<Vec<DataGridColumn>>,
    /// A chave da coluna ordenada e o sentido.
    pub sort: Option<(String, SortDirection)>,
    /// As chaves selecionadas que aparecem, na ordem das linhas.
    pub selected: Vec<String>,
    /// O texto do filtro.
    pub filter: String,
    /// O texto de quando não há linhas.
    pub empty_text: String,
    /// Se um arrasto vai soltar no fundo (o grid inteiro ganha o contorno).
    pub drop_background: bool,
    /// Densidade.
    pub density: Density,
}

impl PartialEq for DataGridVisual {
    fn eq(&self, other: &Self) -> bool {
        self.sort == other.sort
            && self.selected == other.selected
            && self.filter == other.filter
            && self.empty_text == other.empty_text
            && self.drop_background == other.drop_background
            && self.density == other.density
            && (Arc::ptr_eq(&self.columns, &other.columns) || self.columns == other.columns)
            && (Arc::ptr_eq(&self.rows, &other.rows) || self.rows == other.rows)
    }
}

impl DataGridVisual {
    /// A área das linhas, abaixo do cabeçalho.
    pub fn body(&self, rect: Rect) -> Rect {
        Rect::new(
            rect.x0,
            (rect.y0 + header_height(self.density)).min(rect.y1),
            rect.x1,
            rect.y1,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colunas() -> Arc<Vec<DataGridColumn>> {
        Arc::new(vec![
            DataGridColumn::new("nome", "Nome"),
            DataGridColumn::new("tam", "Tamanho").width(super::super::ColumnWidth::Pixels(100.0)),
        ])
    }

    fn linha(density: Density) -> DataGridRowVisual {
        DataGridRowVisual {
            row: Arc::new(DataGridRow::new("a")),
            columns: colunas(),
            selected: false,
            state: ButtonState::Rest,
            focus_ring: false,
            renaming: false,
            drop: false,
            density,
        }
    }

    #[test]
    fn geometria_da_linha() {
        let rect = Rect::new(0.0, 100.0, 304.0, 132.0);
        let v = linha(Density::Standard);
        assert_eq!(v.pill(rect), Rect::new(4.0, 102.0, 300.0, 130.0));
        let cells = v.cells(rect);
        assert_eq!(cells[0], Rect::new(4.0, 102.0, 200.0, 130.0));
        assert_eq!(cells[1], Rect::new(200.0, 102.0, 300.0, 130.0));
    }

    #[test]
    fn linha_compacta_nao_tem_margem() {
        let rect = Rect::new(0.0, 0.0, 300.0, 24.0);
        assert_eq!(linha(Density::Compact).pill(rect), rect);
    }

    #[test]
    fn divisores_do_cabecalho() {
        let header = DataGridHeaderVisual {
            columns: colunas(),
            sort: None,
            hovered: None,
            pressed: None,
            resizing: None,
            divider_hover: None,
            focus_ring: None,
            density: Density::Standard,
        };
        let rect = Rect::new(0.0, 0.0, 304.0, 32.0);
        assert_eq!(header.column_at(rect, 10.0), Some(0));
        assert_eq!(header.column_at(rect, 250.0), Some(1));
        assert_eq!(header.divider_at(rect, 199.0), Some(0));
        assert_eq!(header.divider_at(rect, 100.0), None);
        assert_eq!(header.divider_at(rect, 299.0), Some(1));
    }
}
