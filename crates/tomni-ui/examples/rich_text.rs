//! `RichText`: negrito, itálico, riscado, código e links no mesmo parágrafo.
//! `cargo run --example rich_text`.
//!
//! Cada trecho é um `Span`. O link é um `Span` com `.link(url)`: fica na cor accent, ganha
//! sublinhado com o ponteiro em cima, entra na ordem do Tab (e o Tab percorre os links do
//! texto antes de seguir) e abre com Enter ou clique, por `.on_link`. Na árvore das
//! ferramentas, cada link é um filho de id `<id>-link-<n>`.

use tomni_ui::prelude::*;

/// Um parágrafo com todos os estilos, e o último link clicado.
pub fn texto_rico() -> impl View {
    let aberto = signal(String::new());
    Form::new("Texto rico").child(
        Column::new()
            .gap(12)
            .padding(24)
            .child(
                RichText::new(vec![
                    Span::new("Para entrar, rode "),
                    Span::new("ssh usuario@servidor").code(),
                    Span::new(" e veja o "),
                    Span::new("manual")
                        .bold()
                        .link("https://exemplo.com/manual"),
                    Span::new(". O que ficou "),
                    Span::new("para depois").italic(),
                    Span::new(" e o que foi "),
                    Span::new("descartado").strike(),
                    Span::new(" estão na "),
                    Span::new("lista").link("https://exemplo.com/lista"),
                    Span::new("."),
                ])
                .wrap()
                .id("paragrafo")
                .on_link(move |url| aberto.set(url)),
            )
            .child(
                RichText::new(vec![
                    Span::new("Resumo em duas linhas: "),
                    Span::new("texto longo").bold(),
                    Span::new(
                        " que passa do espaço que o layout deu e termina em reticências, \
                         porque o limite de linhas corta o resto do parágrafo.",
                    ),
                ])
                .secondary()
                .max_lines(2)
                .id("resumo"),
            )
            .child(Text::new(move || aberto.get()).caption().id("aberto"))
            .child(
                Button::new("Limpar")
                    .id("limpar")
                    .height(44)
                    .on_click(move || aberto.set(String::new())),
            ),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(texto_rico).size(390, 844).run()
}
