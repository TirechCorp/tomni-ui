//! `DataGrid`: tabela de arquivos com cabeçalho que ordena. `cargo run --example data_grid`.
//!
//! As linhas ficam num `Signal<Vec<DataGridRow>>` e a seleção, num `Signal<BTreeSet<String>>`;
//! a ordem (`.sort`), o filtro (`.filter`) e o nome em edição (`.renaming`) também são signals
//! do app. O clique no cabeçalho ordena (de novo, inverte), o divisor redimensiona a coluna (o
//! clique duplo ajusta ao texto), Ctrl e Shift marcam várias linhas, F2 renomeia, Delete chama
//! `.on_delete` e arrastar leva a seleção. A coluna do nome mostra o ícone; `.rank` mantém o
//! ".." e as pastas antes dos arquivos em qualquer ordem. `.tone(RowTone::..)` pinta o fundo da
//! linha (sucesso, atenção ou crítico), como uma comparação de pastas; a seleção e o ponteiro
//! aparecem por cima, e a inspeção diz o tom de cada linha. As linhas têm os ids `{id}-{chave}`.
//! No `TestApp`: `app.grid("arquivos")`, `app.grid_select(...)`, `app.drag_rows(...)` e
//! `app.drop_files(...)`.

use std::collections::BTreeSet;
use std::time::{Duration, UNIX_EPOCH};

use tomni_ui::prelude::*;

/// Segundos desde 1970 de 2026-10-01 00:00 UTC, de onde partem as datas fictícias.
const BASE: u64 = 1_790_812_800;

/// A data `segundos` (desde 1970) como `2026-10-18 14:30`.
fn data(segundos: u64) -> String {
    let dias = (segundos / 86_400) as i64;
    let resto = segundos % 86_400;
    // Dias desde 1970 para ano, mês e dia (calendário gregoriano).
    let z = dias + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let dia = doy - (153 * mp + 2) / 5 + 1;
    let mes = if mp < 10 { mp + 3 } else { mp - 9 };
    let ano = yoe + era * 400 + i64::from(mes <= 2);
    format!(
        "{ano}-{mes:02}-{dia:02} {:02}:{:02}",
        resto / 3_600,
        resto % 3_600 / 60
    )
}

/// O tamanho em bytes como `12,5 KB`.
fn tamanho(bytes: u64) -> String {
    const UNIDADES: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut valor = bytes as f64;
    let mut unidade = 0;
    while valor >= 1024.0 && unidade < UNIDADES.len() - 1 {
        valor /= 1024.0;
        unidade += 1;
    }
    match unidade {
        0 => format!("{bytes} B"),
        _ => format!("{valor:.1} {}", UNIDADES[unidade]).replace('.', ","),
    }
}

/// Uma linha de arquivo (ou de pasta, com `bytes` `None`).
fn linha(nome: &str, bytes: Option<u64>, minutos: u64, permissoes: &str) -> DataGridRow {
    let quando = BASE + minutos * 60;
    let momento = UNIX_EPOCH + Duration::from_secs(quando);
    let pasta = bytes.is_none();
    let mut linha = DataGridRow::new(nome)
        .cell(nome, CellValue::Text(nome.to_owned()))
        .cell(
            bytes.map_or_else(String::new, tamanho),
            CellValue::Number(bytes.unwrap_or(0) as f64),
        )
        .cell(data(quando), CellValue::Time(Some(momento)))
        .cell(permissoes, CellValue::Text(permissoes.to_owned()))
        .icon(if pasta {
            Symbol::Folder
        } else {
            Symbol::Document
        })
        .rank(if pasta { 1 } else { 2 });
    if pasta {
        linha = linha.drop_target();
    }
    linha
}

/// Os arquivos de uma pasta do servidor: o "..", dez pastas e 189 arquivos fictícios.
pub fn arquivos_iniciais() -> Vec<DataGridRow> {
    let mut linhas = vec![
        DataGridRow::new("..")
            .cell("..", CellValue::Text("..".to_owned()))
            .cell("", CellValue::Number(0.0))
            .cell("", CellValue::Time(None))
            .cell("", CellValue::Text(String::new()))
            .icon(Symbol::FolderOpen)
            .rank(0)
            .drop_target(),
    ];
    for (i, nome) in [
        "backups", "config", "docs", "imagens", "logs", "scripts", "site", "tmp", "vendor", "www",
    ]
    .into_iter()
    .enumerate()
    {
        linhas.push(linha(nome, None, (i as u64) * 1_440 + 30, "drwxr-xr-x"));
    }
    let extensoes = ["txt", "log", "conf", "png", "sh", "json", "zip", "md", "rs"];
    // Os três primeiros arquivos ganham um tom, para ver as cores.
    let tons = [RowTone::Caution, RowTone::Success, RowTone::Critical];
    for i in 1..=189_u64 {
        let ext = extensoes[(i % 9) as usize];
        let nome = format!("arquivo{i}.{ext}");
        let bytes = (i * 7_919) % 5_000_000 + 40;
        let permissoes = if ext == "sh" {
            "-rwxr-xr-x"
        } else {
            "-rw-r--r--"
        };
        let mut arquivo = linha(&nome, Some(bytes), (i * 97) % 20_000, permissoes);
        if let Some(tom) = tons.get((i - 1) as usize) {
            arquivo = arquivo.tone(*tom);
        }
        linhas.push(arquivo);
    }
    linhas
}

/// Gerenciador de arquivos de mentira: filtro em cima, a tabela e, embaixo, o status.
pub fn arquivos() -> impl View {
    let linhas = signal(arquivos_iniciais());
    let selecionadas = signal(BTreeSet::<String>::new());
    let ordem = signal(Some(("nome".to_owned(), SortDirection::Ascending)));
    let filtro = signal(String::new());
    let renomeando = signal(None::<String>);
    let status = signal("Pronto".to_owned());

    let apagar = move |chaves: Vec<String>| {
        linhas.update(|v| v.retain(|l| !chaves.contains(&l.key)));
        selecionadas.update(BTreeSet::clear);
        status.set(format!("{} apagado(s)", chaves.len()));
    };
    let primeira = move || selecionadas.with_untracked(|s| s.iter().next().cloned());

    Form::new("Arquivos").child(
        Column::new()
            .grow(1)
            .padding(12)
            .gap(8)
            .child(
                Edit::bind(filtro)
                    .id("filtro")
                    .height(44)
                    .placeholder("Filtrar arquivos"),
            )
            .child(
                DataGrid::bind(linhas, selecionadas)
                    .id("arquivos")
                    .grow(1)
                    .columns([
                        DataGridColumn::new("nome", "Nome").width(ColumnWidth::Star(1.0)),
                        DataGridColumn::new("tamanho", "Tamanho")
                            .width(ColumnWidth::Pixels(110.0))
                            .align_end(),
                        DataGridColumn::new("modificado", "Modificado")
                            .width(ColumnWidth::Pixels(140.0)),
                        DataGridColumn::new("permissoes", "Permissões")
                            .width(ColumnWidth::Pixels(100.0))
                            .not_sortable(),
                    ])
                    .sort(ordem)
                    .filter(filtro)
                    .renaming(renomeando)
                    .empty_text("Nenhum arquivo")
                    .on_invoke(move |chave| {
                        let pasta = linhas
                            .with_untracked(|v| v.iter().any(|l| l.key == chave && l.drop_target));
                        status.set(match pasta {
                            true => format!("Abrindo a pasta {chave}"),
                            false => format!("Abrindo {chave}"),
                        });
                    })
                    .on_rename(move |chave, nome| {
                        linhas.update(|v| {
                            if let Some(l) = v.iter_mut().find(|l| l.key == chave) {
                                l.cells[0] = (nome.clone(), CellValue::Text(nome));
                            }
                        });
                        renomeando.set(None);
                        status.set("Renomeado".to_owned());
                    })
                    .on_delete(apagar)
                    .item_context_flyout(move |_| {
                        MenuFlyout::new()
                            .item("abrir", "Abrir", move || {
                                if let Some(chave) = primeira() {
                                    status.set(format!("Abrindo {chave}"));
                                }
                            })
                            .item("renomear", "Renomear", move || {
                                if let Some(chave) = primeira() {
                                    renomeando.set(Some(chave));
                                }
                            })
                            .separator()
                            .item("apagar", "Apagar", move || {
                                let chaves: Vec<String> =
                                    selecionadas.with_untracked(|s| s.iter().cloned().collect());
                                apagar(chaves);
                            })
                    })
                    .on_files_dropped(move |caminhos, _| {
                        status.set(format!("{} arquivo(s) para enviar", caminhos.len()));
                    }),
            )
            .child(Text::new(move || status.get()).id("status")),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(arquivos).size(620, 640).run()
}
