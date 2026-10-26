//! `.tooltip()`: dica que aparece depois de meio segundo com o ponteiro parado em cima do
//! widget, ou na hora quando o foco chega pelo teclado. `cargo run --example tooltip`.
//!
//! Qualquer widget aceita `.tooltip(...)`. A dica tem o id `{id}-tooltip`, fecha quando o
//! ponteiro sai, o foco sai, com Esc ou com um clique, e não ocupa espaço no layout. Use para
//! explicar, nunca para esconder o único jeito de saber o que um botão faz.

use tomni_ui::prelude::*;

/// Barra com três botões de ação, cada um com a dica do que faz.
pub fn barra() -> impl View {
    let copias = signal(0);
    Form::new("Arquivo").child(
        Column::new()
            .gap(12)
            .padding(24)
            .child(
                Row::new()
                    .gap(8)
                    .child(
                        Button::new("Copiar")
                            .id("copiar")
                            .height(44)
                            .tooltip("Copia o link do arquivo")
                            .on_click(move || copias.update(|n| *n += 1)),
                    )
                    .child(
                        Button::new("Mover")
                            .id("mover")
                            .height(44)
                            .tooltip("Leva o arquivo para outra pasta"),
                    )
                    .child(
                        Button::new("Arquivar")
                            .id("arquivar")
                            .height(44)
                            .tooltip("Tira o arquivo da lista, sem apagar"),
                    ),
            )
            .child(Text::new(move || format!("Links copiados: {}", copias.get())).id("copias")),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(barra).size(390, 844).run()
}
