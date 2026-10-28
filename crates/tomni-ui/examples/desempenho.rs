//! `AreaChart`, `Sparkline`, `LevelMeter` e o calor das células do `DataGrid`: uma página
//! Desempenho, como a do Gerenciador de Tarefas. `cargo run --example desempenho`.
//!
//! Os gráficos recebem a série inteira (`Vec<f64>`, fixa, closure ou signal), do ponto mais
//! antigo ao mais novo, e mostram os `.capacity(..)` mais novos (60 por padrão) na escala de 0
//! a `.max(..)` (100 por padrão): o mais novo fica na borda direita. O `LevelMeter` acende os
//! quadradinhos do amarelo-claro ao vermelho-escuro conforme o nível (de 0 a 1); com
//! `.on_click` vira um botão. No `DataGrid`, `.heat(..)` dá o calor da célula de antes, de 0
//! a 1, e `CellValue::Sparkline` desenha uma minissérie no lugar do texto. `app.value("cpu")`
//! lê o nível do medidor e o ponto mais novo de um gráfico.

use std::collections::BTreeSet;

use tomni_ui::prelude::*;

/// Os usos de CPU, em %, que o botão "Amostrar" põe na série, um de cada vez.
const AMOSTRAS: [f64; 8] = [12.0, 18.0, 35.0, 64.0, 92.0, 71.0, 40.0, 22.0];

/// Os programas da tabela: nome, CPU em % e a série recente (de 0 a 1).
fn programas(cpu: f64) -> Vec<DataGridRow> {
    let lista: [(&str, f64, [f64; 6]); 4] = [
        ("compilador", cpu * 0.6, [0.2, 0.4, 0.5, 0.7, 0.8, 0.6]),
        ("navegador", cpu * 0.25, [0.3, 0.2, 0.3, 0.2, 0.3, 0.25]),
        ("terminal", cpu * 0.1, [0.05, 0.1, 0.05, 0.1, 0.1, 0.1]),
        ("ocioso", 0.0, [0.0; 6]),
    ];
    lista
        .into_iter()
        .map(|(nome, uso, serie)| {
            DataGridRow::new(nome)
                .cell(nome, CellValue::Text(nome.to_owned()))
                .cell(format!("{uso:.0}%"), CellValue::Number(uso))
                .heat(uso / 100.0)
                .cell(
                    format!("histórico de {nome}"),
                    CellValue::Sparkline(serie.to_vec()),
                )
        })
        .collect()
}

/// A página: o medidor no alto, as miniaturas à esquerda, o gráfico grande à direita e a
/// tabela por programa embaixo.
pub fn painel() -> impl View {
    let serie = signal(vec![10.0, 14.0, 12.0, 20.0, 35.0]);
    let proxima = signal(0_usize);
    let memoria = signal(vec![40.0, 41.0, 41.0, 43.0, 44.0]);
    let detalhes = signal(true);
    let atual = move || serie.with(|s| s.last().copied().unwrap_or(0.0));
    let linhas = signal(programas(35.0));
    let selecionadas = signal(BTreeSet::<String>::new());
    let ordem = signal(Some(("cpu".to_owned(), SortDirection::Descending)));

    let amostrar = move || {
        let i = proxima.get_untracked();
        let uso = AMOSTRAS[i % AMOSTRAS.len()];
        proxima.set(i + 1);
        serie.update(|s| s.push(uso));
        memoria.update(|m| m.push(40.0 + (i % 5) as f64));
        linhas.set(programas(uso));
    };

    Form::new("Desempenho").child(
        Column::new()
            .grow(1)
            .padding(16)
            .gap(12)
            .child(
                Row::new()
                    .gap(12)
                    .align(Align::Center)
                    .child(Text::new("Desempenho").ramp(TextRamp::Subtitle).grow(1))
                    .child(
                        LevelMeter::new(move || atual() / 100.0)
                            .id("medidor")
                            .height(44)
                            .tooltip(move || format!("CPU: {:.0}%", atual()))
                            .on_click(move || detalhes.update(|d| *d = !*d)),
                    )
                    .child(
                        Button::new("Amostrar")
                            .id("amostrar")
                            .height(44)
                            .on_click(amostrar),
                    ),
            )
            .child(
                Show::when(
                    move || detalhes.get(),
                    move || {
                        Row::new()
                            .gap(12)
                            .height(180)
                            .child(
                                Column::new()
                                    .gap(8)
                                    .width(160)
                                    .child(
                                        Row::new()
                                            .gap(8)
                                            .align(Align::Center)
                                            .child(Sparkline::new(serie).id("spark-cpu"))
                                            .child(Text::new(move || {
                                                format!("CPU {:.0}%", atual())
                                            })),
                                    )
                                    .child(
                                        Row::new()
                                            .gap(8)
                                            .align(Align::Center)
                                            .child(
                                                Sparkline::new(memoria)
                                                    .id("spark-memoria")
                                                    .color(Token::SystemFillColorAttention),
                                            )
                                            .child(Text::new("Memória")),
                                    ),
                            )
                            .child(AreaChart::new(serie).id("cpu").grow(1))
                    },
                )
                .id("detalhes"),
            )
            .child(
                DataGrid::bind(linhas, selecionadas)
                    .id("programas")
                    .grow(1)
                    .columns([
                        DataGridColumn::new("nome", "Programa"),
                        DataGridColumn::new("cpu", "CPU")
                            .width(ColumnWidth::Pixels(80.0))
                            .align_end(),
                        DataGridColumn::new("historico", "Histórico")
                            .width(ColumnWidth::Pixels(120.0)),
                    ])
                    .sort(ordem),
            ),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(painel).size(720, 560).run()
}
