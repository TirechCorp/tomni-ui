//! `RadioButtons`: escolhe uma opção entre poucas, todas à vista. `cargo run --example
//! radio_buttons`.
//!
//! O valor fica num `Signal<String>` com o texto da opção. O grupo é uma parada só de Tab, e
//! ↑/↓ trocam a escolha. Cada opção tem o id `{id}-{opção}` (`tema-do-sistema`); no
//! `TestApp`, `app.select("tema", "Escuro")` (ou `app.click("tema-escuro")`) escolhe e
//! `app.text("tema")` lê o valor.

use tomni_ui::prelude::*;

/// Tema e tamanho do texto; o texto embaixo mostra o que foi escolhido.
pub fn aparencia() -> impl View {
    let tema = signal("Do sistema".to_owned());
    let tamanho = signal("Médio".to_owned());
    Form::new("Aparência").child(
        Column::new()
            .gap(24)
            .padding(24)
            .child(
                RadioButtons::bind(tema, ["Claro", "Escuro", "Do sistema"])
                    .id("tema")
                    .header("Tema"),
            )
            .child(
                RadioButtons::bind(tamanho, ["Pequeno", "Médio", "Grande"])
                    .id("tamanho")
                    .header("Tamanho do texto")
                    .disabled(true),
            )
            .child(Text::new(move || format!("Tema: {}", tema.get())).id("resumo")),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(aparencia).size(390, 844).run()
}
