//! `ToggleButton`: botão que fica ligado ou desligado, ligado a um `Signal<bool>`.
//! `cargo run --example toggle_button`.
//!
//! Clique, Enter e Espaço alternam. Desligado, tem o visual do botão padrão; ligado, o do
//! botão de destaque. Tem 32 px de altura, como o `Button`: use `.height(44)` para o lint de
//! touch-target.

use tomni_ui::prelude::*;

/// Barra de formatação: três botões que ligam e desligam, e um resumo do que está ligado.
pub fn formatacao() -> impl View {
    let negrito = signal(true);
    let italico = signal(false);
    let sublinhado = signal(false);
    Form::new("Formatação").child(
        Column::new()
            .gap(12)
            .padding(24)
            .child(Text::new("Formatação").subtitle())
            .child(
                Row::new()
                    .gap(8)
                    .child(
                        ToggleButton::bind(negrito, "Negrito")
                            .id("negrito")
                            .height(44)
                            .grow(1),
                    )
                    .child(
                        ToggleButton::bind(italico, "Itálico")
                            .id("italico")
                            .height(44)
                            .grow(1),
                    )
                    .child(
                        ToggleButton::bind(sublinhado, "Sublinhado")
                            .id("sublinhado")
                            .height(44)
                            .grow(1),
                    ),
            )
            .child(
                Text::new(move || {
                    let ligados: Vec<&str> = [
                        (negrito.get(), "negrito"),
                        (italico.get(), "itálico"),
                        (sublinhado.get(), "sublinhado"),
                    ]
                    .into_iter()
                    .filter_map(|(on, nome)| on.then_some(nome))
                    .collect();
                    if ligados.is_empty() {
                        "Sem formatação".to_owned()
                    } else {
                        format!("Ligado: {}", ligados.join(", "))
                    }
                })
                .id("resumo"),
            ),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(formatacao).size(390, 844).run()
}
