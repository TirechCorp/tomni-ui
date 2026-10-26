//! `Text`: a rampa tipográfica, um parágrafo que quebra linhas e um resumo cortado em duas.
//! `cargo run --example text`.
//!
//! Sem `.wrap()`, o texto fica numa linha e o lint acusa se ele for cortado; com `.wrap()`,
//! quebra na largura que o layout der; com `.max_lines(n)`, para na linha `n` com reticências
//! (de propósito, então o lint não acusa). `.secondary()` usa a cor de texto secundária. O
//! texto aceita valor fixo, closure ou signal.

use tomni_ui::prelude::*;

/// Os degraus da rampa e um parágrafo.
pub fn textos() -> impl View {
    let caracteres = signal(0usize);
    Form::new("Textos").child(
        Column::new()
            .gap(12)
            .padding(24)
            .child(Text::new("Criar conta").title_large().id("titulo"))
            .child(Text::new("Dados pessoais").subtitle().id("subtitulo"))
            .child(
                Text::new("Escolha uma senha forte")
                    .ramp(TextRamp::BodyStrong)
                    .id("destaque"),
            )
            .child(
                Text::new(
                    "A senha precisa de pelo menos 8 caracteres. Misture letras, números e \
                     símbolos; evite datas e nomes da família.",
                )
                .wrap()
                .id("paragrafo"),
            )
            .child(
                Text::new(
                    "Os dados ficam só neste aparelho até você entrar na conta. Depois, são \
                     sincronizados com os outros aparelhos em que você entrar, e dá para apagar \
                     tudo nas configurações.",
                )
                .secondary()
                .wrap()
                .max_lines(2)
                .id("resumo"),
            )
            .child(
                Text::new(move || format!("{} de 8 caracteres", caracteres.get()))
                    .caption()
                    .id("contagem"),
            )
            .child(
                Button::new("Mais um caractere")
                    .id("mais")
                    .height(44)
                    .on_click(move || caracteres.update(|n| *n += 1)),
            ),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(textos).size(390, 844).run()
}
