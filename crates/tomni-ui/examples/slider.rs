//! `Slider`: escolhe um número entre duas pontas. `cargo run --example slider`.
//!
//! O valor fica num `Signal<f64>`, sempre dentro da faixa e num múltiplo do passo (`.step`,
//! 1 por padrão). ←/→ andam um passo, PgUp/PgDn 10% da faixa e Home/End vão às pontas. No
//! `TestApp`, `app.set_value("volume", 30.0)` muda o valor e `app.value("volume")` lê.

use tomni_ui::prelude::*;

/// Volume e brilho; o texto embaixo mostra o volume.
pub fn som() -> impl View {
    let volume = signal(40.0);
    let brilho = signal(0.8);
    Form::new("Som e tela").child(
        Column::new()
            .gap(24)
            .padding(24)
            .child(
                Slider::bind(volume, 0..=100)
                    .id("volume")
                    .header("Volume")
                    .step(5.0),
            )
            .child(
                Slider::bind(brilho, 0.0..=1.0)
                    .id("brilho")
                    .header("Brilho (automático)")
                    .step(0.1)
                    .disabled(true),
            )
            .child(Text::new(move || format!("Volume: {}%", volume.get())).id("resumo")),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(som).size(390, 844).run()
}
