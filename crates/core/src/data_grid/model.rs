//! O modelo do `DataGrid`: colunas, linhas, a ordenação natural, o filtro e as larguras.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;

use crate::symbol::Symbol;
use crate::tree_view::fold;

/// Largura mínima de uma coluna, em px: abaixo disso nem o arrasto do divisor desce.
pub const DATA_GRID_MIN_COLUMN: f64 = 40.0;

/// Largura de uma coluna.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ColumnWidth {
    /// Largura fixa, em px.
    Pixels(f64),
    /// Parte do que sobra depois das colunas fixas: `Star(2.0)` fica com o dobro de
    /// `Star(1.0)`.
    Star(f64),
}

/// O valor de uma célula para ordenar (o texto que aparece vem à parte, em
/// [`DataGridRow::cell`]).
#[derive(Clone, Debug, PartialEq)]
pub enum CellValue {
    /// Texto, em ordem natural sem diferenciar maiúsculas nem acentos: "arq2" vem antes de
    /// "arq10".
    Text(String),
    /// Número.
    Number(f64),
    /// Data e hora; `None` (desconhecida) vem antes de qualquer data.
    Time(Option<SystemTime>),
    /// Andamento de 0 a 1: a célula mostra uma `ProgressBar` no lugar do texto (o texto é só
    /// o nome acessível). Ordena como um número.
    Progress(f64),
}

/// Uma coluna.
#[derive(Clone, Debug, PartialEq)]
pub struct DataGridColumn {
    /// Chave da coluna: a do `sort`.
    pub key: String,
    /// Título do cabeçalho.
    pub header: String,
    /// Largura.
    pub width: ColumnWidth,
    /// Se o texto fica encostado à direita (números).
    pub align_end: bool,
    /// Se o clique no cabeçalho ordena por ela.
    pub sortable: bool,
}

impl DataGridColumn {
    /// Coluna `key` com o título `header`: `Star(1.0)`, à esquerda, ordenável.
    pub fn new(key: impl Into<String>, header: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            header: header.into(),
            width: ColumnWidth::Star(1.0),
            align_end: false,
            sortable: true,
        }
    }

    /// A largura da coluna.
    pub fn width(mut self, width: ColumnWidth) -> Self {
        self.width = width;
        self
    }

    /// Texto encostado à direita, como os números.
    pub fn align_end(mut self) -> Self {
        self.align_end = true;
        self
    }

    /// O cabeçalho não ordena.
    pub fn not_sortable(mut self) -> Self {
        self.sortable = false;
        self
    }
}

/// O tom de fundo de uma linha: o destaque que o app dá a ela (o que difere, o que é novo, o que
/// deu errado). Pinta o fundo com os tokens `SystemFillColor…Background` do WinUI, que foram
/// feitos para texto primário; a seleção e o hover vêm por cima, como uma camada.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RowTone {
    /// Verde (`SystemFillColorSuccessBackground`).
    Success,
    /// Amarelo (`SystemFillColorCautionBackground`).
    Caution,
    /// Vermelho (`SystemFillColorCriticalBackground`).
    Critical,
}

impl RowTone {
    /// O nome na inspeção e na acessibilidade: `"success"`, `"caution"` ou `"critical"`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Caution => "caution",
            Self::Critical => "critical",
        }
    }

    /// A descrição genérica para os leitores de tela: a lib não sabe o que o tom quer dizer
    /// para o app.
    pub fn description(self) -> &'static str {
        match self {
            Self::Success => "destaque: sucesso",
            Self::Caution => "destaque: atenção",
            Self::Critical => "destaque: crítico",
        }
    }
}

/// Uma linha: a chave, uma célula por coluna e o que a distingue na ordenação.
#[derive(Clone, Debug, PartialEq)]
pub struct DataGridRow {
    /// Chave da linha, única no grid.
    pub key: String,
    /// Uma célula por coluna, na ordem das colunas: o texto que aparece e o valor que ordena.
    pub cells: Vec<(String, CellValue)>,
    /// Ícone antes do texto da primeira coluna.
    pub icon: Option<Symbol>,
    /// Posição antes da coluna na ordenação: ".." (0), pastas (1) e arquivos (2), por exemplo.
    /// Não inverte com a ordem decrescente.
    pub rank: u8,
    /// Se aceita itens soltos sobre ela (uma pasta).
    pub drop_target: bool,
    /// O tom do fundo, se o app quer destacar a linha (padrão: nenhum).
    pub tone: Option<RowTone>,
}

impl DataGridRow {
    /// Linha `key`, sem células.
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            cells: Vec::new(),
            icon: None,
            rank: 0,
            drop_target: false,
            tone: None,
        }
    }

    /// A próxima célula: o texto que aparece e o valor que ordena.
    pub fn cell(mut self, text: impl Into<String>, value: CellValue) -> Self {
        self.cells.push((text.into(), value));
        self
    }

    /// O ícone da primeira coluna.
    pub fn icon(mut self, icon: Symbol) -> Self {
        self.icon = Some(icon);
        self
    }

    /// A posição antes da coluna na ordenação (menor vem antes).
    pub fn rank(mut self, rank: u8) -> Self {
        self.rank = rank;
        self
    }

    /// A linha aceita itens soltos sobre ela (`DataGridDrop::target`).
    pub fn drop_target(mut self) -> Self {
        self.drop_target = true;
        self
    }

    /// O tom do fundo da linha (`RowTone`). A seleção e o hover aparecem por cima; o realce de
    /// soltar sobre uma pasta vale mais que o tom.
    pub fn tone(mut self, tone: RowTone) -> Self {
        self.tone = Some(tone);
        self
    }

    /// O texto da primeira célula, que dá o nome da linha.
    pub fn label(&self) -> &str {
        self.cells.first().map_or("", |(text, _)| text)
    }
}

/// O sentido da ordenação.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SortDirection {
    /// Do menor ao maior.
    Ascending,
    /// Do maior ao menor.
    Descending,
}

impl SortDirection {
    /// O sentido contrário.
    pub fn flip(self) -> Self {
        match self {
            Self::Ascending => Self::Descending,
            Self::Descending => Self::Ascending,
        }
    }
}

/// Itens soltos num `DataGrid`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataGridDrop {
    /// O id do `DataGrid` de onde as linhas saíram.
    pub source_grid: String,
    /// As linhas arrastadas, na ordem em que aparecem no grid de origem.
    pub keys: Vec<String>,
    /// A linha sob o ponteiro (uma pasta) ou `None` (o fundo).
    pub target: Option<String>,
}

/// As linhas visíveis, já filtradas e ordenadas.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DataGridFlat {
    /// As linhas.
    pub rows: Vec<Arc<DataGridRow>>,
    /// Quantas linhas o app tem, com ou sem filtro.
    pub total: usize,
    index: HashMap<String, usize>,
}

impl DataGridFlat {
    /// A posição da chave nas linhas visíveis.
    pub fn position(&self, key: &str) -> Option<usize> {
        self.index.get(key).copied()
    }

    /// A linha da chave, se aparece.
    pub fn get(&self, key: &str) -> Option<&Arc<DataGridRow>> {
        self.position(key).map(|i| &self.rows[i])
    }
}

/// O valor que ordena uma célula, já preparado (o texto sem acentos nem maiúsculas).
#[derive(Clone, Debug, PartialEq)]
enum SortKey {
    Text(String),
    Number(f64),
    Time(Option<SystemTime>),
}

impl SortKey {
    fn of(row: &DataGridRow, column: usize) -> Self {
        match row.cells.get(column) {
            Some((_, CellValue::Text(text))) => Self::Text(fold(text)),
            Some((_, CellValue::Number(n) | CellValue::Progress(n))) => Self::Number(*n),
            Some((_, CellValue::Time(time))) => Self::Time(*time),
            None => Self::Text(String::new()),
        }
    }

    fn variant(&self) -> u8 {
        match self {
            Self::Text(_) => 0,
            Self::Number(_) => 1,
            Self::Time(_) => 2,
        }
    }

    fn compare(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Text(a), Self::Text(b)) => natural_cmp(a, b),
            (Self::Number(a), Self::Number(b)) => a.total_cmp(b),
            (Self::Time(a), Self::Time(b)) => a.cmp(b),
            _ => self.variant().cmp(&other.variant()),
        }
    }
}

/// Compara em ordem natural: os trechos de dígitos valem pelo número que formam. Espera o
/// texto já sem acentos nem maiúsculas.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let (da, db) = (digits(&mut a), digits(&mut b));
                let (da, db) = (da.trim_start_matches('0'), db.trim_start_matches('0'));
                match da.len().cmp(&db.len()).then_with(|| da.cmp(db)) {
                    Ordering::Equal => {}
                    other => return other,
                }
            }
            (Some(x), Some(y)) => match x.cmp(&y) {
                Ordering::Equal => {
                    a.next();
                    b.next();
                }
                other => return other,
            },
        }
    }
}

fn digits(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut out = String::new();
    while let Some(c) = chars.next_if(char::is_ascii_digit) {
        out.push(c);
    }
    out
}

/// Filtra e ordena `rows`: só as que têm `filter` (sem acentos nem maiúsculas) no texto da
/// primeira célula; depois por `rank` e, se há `sort`, pelo valor da coluna `column` (empate
/// pela chave).
pub fn arrange(
    rows: &[DataGridRow],
    filter: &str,
    sort: Option<(usize, SortDirection)>,
) -> DataGridFlat {
    #[cfg(debug_assertions)]
    {
        let mut seen = std::collections::HashSet::new();
        for row in rows {
            assert!(
                seen.insert(row.key.as_str()),
                "chave repetida no DataGrid: {:?} (as chaves são únicas no grid)",
                row.key
            );
        }
    }
    let needle = fold(filter.trim());
    let mut order: Vec<usize> = (0..rows.len())
        .filter(|&i| needle.is_empty() || fold(rows[i].label()).contains(&needle))
        .collect();
    match sort {
        Some((column, direction)) => {
            let keys: Vec<SortKey> = order
                .iter()
                .map(|&i| SortKey::of(&rows[i], column))
                .collect();
            // `order` e `keys` andam juntos: ordena os pares.
            let mut pairs: Vec<(usize, SortKey)> = order.into_iter().zip(keys).collect();
            pairs.sort_by(|(i, a), (j, b)| {
                let (x, y) = (&rows[*i], &rows[*j]);
                x.rank
                    .cmp(&y.rank)
                    .then_with(|| match direction {
                        SortDirection::Ascending => a.compare(b),
                        SortDirection::Descending => b.compare(a),
                    })
                    .then_with(|| x.key.cmp(&y.key))
            });
            order = pairs.into_iter().map(|(i, _)| i).collect();
        }
        None => order.sort_by_key(|&i| rows[i].rank),
    }
    let rows_out: Vec<Arc<DataGridRow>> =
        order.iter().map(|&i| Arc::new(rows[i].clone())).collect();
    let index = rows_out
        .iter()
        .enumerate()
        .map(|(i, row)| (row.key.clone(), i))
        .collect();
    DataGridFlat {
        rows: rows_out,
        total: rows.len(),
        index,
    }
}

/// As larguras, em px, das colunas `columns` num grid de `total` px: as fixas ficam como
/// são e as `Star` dividem o que sobra, sem passar de [`DATA_GRID_MIN_COLUMN`] para baixo.
pub fn resolve_widths(columns: &[DataGridColumn], total: f64) -> Vec<f64> {
    let fixed: f64 = columns
        .iter()
        .map(|c| match c.width {
            ColumnWidth::Pixels(w) => w.max(DATA_GRID_MIN_COLUMN),
            ColumnWidth::Star(_) => 0.0,
        })
        .sum();
    let stars: f64 = columns
        .iter()
        .map(|c| match c.width {
            ColumnWidth::Star(s) => s.max(0.0),
            ColumnWidth::Pixels(_) => 0.0,
        })
        .sum();
    let rest = (total - fixed).max(0.0);
    columns
        .iter()
        .map(|c| match c.width {
            ColumnWidth::Pixels(w) => w.max(DATA_GRID_MIN_COLUMN),
            ColumnWidth::Star(s) if stars > 0.0 => {
                (rest * s.max(0.0) / stars).max(DATA_GRID_MIN_COLUMN)
            }
            ColumnWidth::Star(_) => DATA_GRID_MIN_COLUMN,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linha(chave: &str, rank: u8, nome: &str, tamanho: f64) -> DataGridRow {
        DataGridRow::new(chave)
            .rank(rank)
            .cell(nome, CellValue::Text(nome.to_owned()))
            .cell(format!("{tamanho}"), CellValue::Number(tamanho))
    }

    fn chaves(flat: &DataGridFlat) -> Vec<&str> {
        flat.rows.iter().map(|r| r.key.as_str()).collect()
    }

    #[test]
    fn ordem_natural_compara_numeros_pelo_valor() {
        assert_eq!(natural_cmp("arq2", "arq10"), Ordering::Less);
        assert_eq!(natural_cmp("arq10", "arq2"), Ordering::Greater);
        assert_eq!(natural_cmp("a", "a"), Ordering::Equal);
        assert_eq!(natural_cmp("a1", "a"), Ordering::Greater);
        assert_eq!(natural_cmp("x007", "x7"), Ordering::Equal);
    }

    #[test]
    fn ordena_por_rank_e_depois_pela_coluna() {
        let linhas = vec![
            linha("b", 1, "Beta", 3.0),
            linha("a", 1, "alfa", 9.0),
            linha("f10", 2, "arq10", 1.0),
            linha("f2", 2, "arq2", 2.0),
            linha("pai", 0, "..", 0.0),
        ];
        let flat = arrange(&linhas, "", Some((0, SortDirection::Ascending)));
        assert_eq!(chaves(&flat), ["pai", "a", "b", "f2", "f10"]);
        let flat = arrange(&linhas, "", Some((0, SortDirection::Descending)));
        // O rank não inverte.
        assert_eq!(chaves(&flat), ["pai", "b", "a", "f10", "f2"]);
        let flat = arrange(&linhas, "", Some((1, SortDirection::Descending)));
        assert_eq!(chaves(&flat), ["pai", "a", "b", "f2", "f10"]);
    }

    #[test]
    fn sem_ordem_mantem_a_do_app_dentro_do_rank() {
        let linhas = vec![
            linha("z", 1, "z", 0.0),
            linha("y", 0, "y", 0.0),
            linha("x", 1, "x", 0.0),
        ];
        assert_eq!(chaves(&arrange(&linhas, "", None)), ["y", "z", "x"]);
    }

    #[test]
    fn filtro_ignora_acentos_e_maiusculas() {
        let linhas = vec![
            linha("1", 0, "Relatório final.pdf", 1.0),
            linha("2", 0, "fotos", 1.0),
        ];
        let flat = arrange(&linhas, " RELATORIO ", None);
        assert_eq!(chaves(&flat), ["1"]);
        assert_eq!(flat.total, 2);
        assert_eq!(flat.position("1"), Some(0));
        assert!(flat.get("2").is_none());
    }

    #[test]
    fn data_desconhecida_vem_antes() {
        let agora = SystemTime::now();
        let linhas = vec![
            DataGridRow::new("a").cell("", CellValue::Time(Some(agora))),
            DataGridRow::new("b").cell("", CellValue::Time(None)),
        ];
        let flat = arrange(&linhas, "", Some((0, SortDirection::Ascending)));
        assert_eq!(chaves(&flat), ["b", "a"]);
    }

    #[test]
    fn larguras_dividem_o_que_sobra() {
        let colunas = vec![
            DataGridColumn::new("nome", "Nome"),
            DataGridColumn::new("data", "Data").width(ColumnWidth::Pixels(100.0)),
            DataGridColumn::new("tipo", "Tipo").width(ColumnWidth::Star(2.0)),
        ];
        let w = resolve_widths(&colunas, 400.0);
        assert_eq!(w, [100.0, 100.0, 200.0]);
        let apertado = resolve_widths(&colunas, 100.0);
        assert_eq!(apertado, [40.0, 100.0, 40.0]);
    }
}
