//! `UniformGrid` e `Row::wrap`: a grade de cards da Gallery e uma linha de filtros que quebra.
//! `cargo run --example uniform_grid`.
//!
//! O `UniformGrid` põe os filhos em quantas colunas de pelo menos `.min_column_width` (200 px
//! por padrão) couberem, todas da mesma largura; numa mesma linha, os filhos têm a mesma
//! altura. A `Row` com `.wrap()` passa para a linha de baixo o filho que não cabe, com o
//! `.gap` nos dois eixos. Redimensione a janela para ver as colunas mudarem.

use tomni_ui::prelude::*;

const CONTROLES: [(&str, &str, &str); 6] = [
    (
        "botoes",
        "Botões",
        "Button, ToggleButton e o botão de link.",
    ),
    (
        "texto",
        "Texto",
        "A rampa de tipos, a cor e o texto cortado.",
    ),
    ("cards", "Cards", "Border e Card, clicáveis ou não."),
    (
        "listas",
        "Listas",
        "ComboBox, RadioButtons e AutoSuggestBox.",
    ),
    ("progresso", "Progresso", "ProgressBar e ProgressRing."),
    ("navegacao", "Navegação", "TabView e Expander."),
];

/// Uma galeria: filtros numa linha que quebra e os controles numa grade de cards.
pub fn galeria() -> impl View {
    let aberto = signal(String::new());
    let mut filtros = Row::new().id("filtros").wrap().gap(8);
    for (id, nome) in [
        ("todos", "Todos"),
        ("basicos", "Básicos"),
        ("colecoes", "Coleções"),
        ("status", "Status"),
    ] {
        filtros = filtros.child(
            ToggleButton::bind(signal(id == "todos"), nome)
                .id(id)
                .height(44),
        );
    }
    let mut grade = UniformGrid::new().id("grade").min_column_width(160).gap(8);
    for (id, titulo, descricao) in CONTROLES {
        grade = grade.child(
            Card::new()
                .id(id)
                .label(titulo)
                .on_click(move || aberto.set(titulo.to_owned()))
                .child(
                    Column::new()
                        .gap(4)
                        .child(Text::new(titulo).body_strong())
                        .child(Text::new(descricao).secondary().wrap().max_lines(2)),
                ),
        );
    }
    Form::new("Galeria").child(
        ScrollViewer::new().id("rolagem").child(
            Column::new()
                .gap(16)
                .padding(24)
                .child(Text::new("Galeria").title_large())
                .child(filtros)
                .child(grade)
                .child(
                    Text::new(move || {
                        let aberto = aberto.get();
                        if aberto.is_empty() {
                            String::new()
                        } else {
                            format!("Aberto: {aberto}")
                        }
                    })
                    .id("aberto"),
                ),
        ),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(galeria).size(390, 844).run()
}
