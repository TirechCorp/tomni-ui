//! `Button`: padrão, de destaque, link, desabilitado e numa `Row`. `cargo run --example
//! button`.
//!
//! O rótulo e o `disabled` aceitam valor fixo ou closure; com closure, o botão repinta sozinho
//! quando os signals lidos mudam. `.hyperlink()` dá o visual de link: texto na cor de destaque,
//! sem fundo nem borda.

use tomni_ui::prelude::*;

/// Contador com os tipos de botão.
pub fn botoes() -> impl View {
    let cliques = signal(0);
    Form::new("Botões").child(
        Column::new()
            .gap(12)
            .padding(24)
            .child(
                Button::new(move || format!("Cliques: {}", cliques.get()))
                    .id("contar")
                    .height(44)
                    .on_click(move || cliques.update(|n| *n += 1)),
            )
            .child(
                Button::new("Zerar")
                    .id("zerar")
                    .accent()
                    .height(44)
                    .disabled(move || cliques.get() == 0)
                    .on_click(move || cliques.set(0)),
            )
            .child(
                Button::new("Sempre desabilitado")
                    .id("desabilitado")
                    .height(44)
                    .disabled(true),
            )
            .child(
                // Numa `Row`, `grow(1)` divide a largura entre os botões.
                Row::new()
                    .gap(8)
                    .child(
                        Button::new("-1")
                            .id("menos")
                            .grow(1)
                            .height(44)
                            .on_click(move || cliques.update(|n| *n -= 1)),
                    )
                    .child(
                        Button::new("+1")
                            .id("mais")
                            .grow(1)
                            .height(44)
                            .on_click(move || cliques.update(|n| *n += 1)),
                    ),
            )
            .child(
                // Numa `Row`, o link fica da largura do texto.
                Row::new().child(
                    Button::new("Começar do 10")
                        .id("dez")
                        .hyperlink()
                        .height(44)
                        .on_click(move || cliques.set(10)),
                ),
            ),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(botoes).size(390, 844).run()
}
