//! `CheckBox`: ligada a um `Signal<bool>`, habilitando um botão. `cargo run --example checkbox`.
//!
//! Clique (na caixa ou no rótulo) e Espaço alternam. A caixa tem 32 px de altura: use
//! `.height(44)` para o lint de touch-target.

use tomni_ui::prelude::*;

/// Termos que habilitam o "Continuar", e uma caixa desabilitada.
pub fn caixas() -> impl View {
    let aceito = signal(false);
    let lembrar = signal(true);
    Form::new("Caixas").child(
        Column::new()
            .gap(12)
            .padding(24)
            .child(
                CheckBox::bind(aceito, "Li e aceito os termos")
                    .id("termos")
                    .height(44),
            )
            .child(
                CheckBox::bind(lembrar, "Lembrar de mim")
                    .id("lembrar")
                    .height(44),
            )
            .child(
                CheckBox::bind(signal(true), "Sempre marcada")
                    .id("bloqueada")
                    .height(44)
                    .disabled(true),
            )
            .child(
                Button::new("Continuar")
                    .id("continuar")
                    .accent()
                    .height(44)
                    .disabled(move || !aceito.get()),
            ),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(caixas).size(390, 844).run()
}
