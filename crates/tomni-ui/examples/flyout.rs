//! `Flyout`: popup leve com qualquer conteúdo, aberto por um botão. `cargo run --example flyout`.
//!
//! O popup entra no botão com `.flyout(...)`, abre embaixo dele (em cima, se não couber) e
//! não ocupa espaço no layout. Esc ou um clique fora fecham; para fechar por um botão de
//! dentro, passe um signal seu em `.open(...)` e ponha `false` nele.

use tomni_ui::prelude::*;

/// Botão "Ajuda" com uma explicação e um "Entendi" que fecha o popup.
pub fn ajuda() -> impl View {
    let aberto = signal(false);
    Form::new("Senha").child(
        Column::new()
            .gap(12)
            .padding(24)
            .child(
                Edit::bind(signal(String::new()))
                    .id("senha")
                    .password()
                    .placeholder("Senha nova")
                    .height(44),
            )
            .child(
                Button::new("Ajuda").id("ajuda").height(44).flyout(
                    Flyout::new(
                        Column::new()
                            .gap(12)
                            .child(
                                Text::new(
                                    "A senha precisa de 8 caracteres, com uma letra e um número.",
                                )
                                .wrap(),
                            )
                            .child(
                                Button::new("Entendi")
                                    .id("entendi")
                                    .height(44)
                                    .on_click(move || aberto.set(false)),
                            ),
                    )
                    .id("ajuda-popup")
                    .open(aberto),
                ),
            ),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(ajuda).size(390, 844).run()
}
