//! `ScrollViewer`: área rolável com barra no estilo do WinUI. `cargo run --example
//! scroll_viewer`.
//!
//! Rola com a roda do mouse, arrastando com o dedo, pela barra (que expande com o mouse em
//! cima), com PageUp/PageDown, Home/End e as setas, e quando o Tab leva o foco a um botão
//! escondido. Com `.grow(1)` numa `Column`, ocupa o espaço que sobra.

use tomni_ui::prelude::*;

/// Categorias numa faixa horizontal e 40 avisos numa lista vertical que ocupa o resto.
pub fn rolagem() -> impl View {
    let categorias = [
        "Todos", "Trabalho", "Casa", "Estudos", "Saúde", "Viagem", "Compras",
    ];
    let escolhida = signal("Todos");
    Form::new("Avisos").child(
        Column::new()
            .gap(12)
            .padding(24)
            .child(Text::new(move || format!("Categoria: {}", escolhida.get())).id("categoria"))
            .child(
                ScrollViewer::horizontal().id("categorias").child(
                    Row::new()
                        .gap(8)
                        .padding(Insets::new(0.0, 0.0, 12.0, 0.0))
                        .child(For::each(
                            categorias.to_vec(),
                            |c| *c,
                            move |c| {
                                Button::new(c)
                                    .id(format!("cat-{}", c.to_lowercase()))
                                    .height(44)
                                    .on_click(move || escolhida.set(c))
                            },
                        )),
                ),
            )
            .child(
                ScrollViewer::new().id("avisos").grow(1).child(
                    Column::new()
                        .gap(8)
                        .padding(Insets::new(0.0, 12.0, 0.0, 0.0))
                        .child(For::each(
                            (1..=40).collect::<Vec<u32>>(),
                            |&i| i,
                            |i| {
                                Button::new(format!("Aviso {i}"))
                                    .id(format!("aviso-{i}"))
                                    .height(44)
                            },
                        )),
                ),
            ),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(rolagem).size(390, 844).run()
}
