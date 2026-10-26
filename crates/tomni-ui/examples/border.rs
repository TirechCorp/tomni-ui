//! `Border`: um contêiner de um filho com fundo, borda, raio e padding. `cargo run --example
//! border`.
//!
//! O fundo e a borda são um `Brush`: um token do tema (`Token::CardStrokeColorDefault`, que
//! acompanha a troca de tema), uma cor fixa (`Color::hex`) ou um gradiente linear
//! (`Brush::linear`, com as pontas relativas ao retângulo). O lint confere o contraste dos
//! textos contra o fundo do `Border`; num gradiente, contra a ponta mais perto do texto.

use tomni_ui::prelude::*;

/// Faixa de destaque com gradiente e um quadro com a cor de camada do tema.
pub fn destaque() -> impl View {
    Form::new("Destaque").child(
        Column::new()
            .gap(16)
            .padding(24)
            .child(
                Border::new()
                    .id("faixa")
                    .background(Brush::linear(
                        (0.0, 0.0),
                        (1.0, 1.0),
                        [(0.0, Color::hex(0x003E92)), (1.0, Color::hex(0x6B2FA0))],
                    ))
                    .corner_radius(8)
                    .padding(24)
                    .child(
                        Column::new()
                            .gap(4)
                            .child(Text::new("Tomni UI").title_large().color(Color::WHITE))
                            .child(
                                Text::new(
                                    "Interfaces nativas em Rust, com o visual do Windows 11.",
                                )
                                .wrap()
                                .color(Color::hex(0xE8E8F8)),
                            ),
                    ),
            )
            .child(
                Border::new()
                    .id("quadro")
                    .background(Token::LayerFillColorDefault)
                    .border(Token::CardStrokeColorDefault, 1)
                    .corner_radius(4)
                    .padding(12)
                    .child(
                        Text::new("Um quadro com a cor de camada do tema e borda de 1 px.").wrap(),
                    ),
            ),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(destaque).size(390, 844).run()
}
