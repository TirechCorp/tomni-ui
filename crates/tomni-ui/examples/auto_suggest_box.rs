//! `AutoSuggestBox`: campo de texto com sugestões embaixo. `cargo run --example
//! auto_suggest_box`.
//!
//! A closure recebe o texto digitado e devolve as sugestões; a lista abre quando há alguma.
//! ↓/↑ andam pela lista, Enter ou um clique escolhem e chamam o `.on_submit`. Cada sugestão
//! tem o id `{id}-{sugestão}` (`cidade-belo-horizonte`). No `TestApp`:
//! `app.type_into("cidade", "Be").click("cidade-belo-horizonte")`, e `app.is_open("cidade")`
//! diz se a lista está aberta. `.query_icon(Symbol::Search)` põe uma lupa no fim do campo
//! (id `{id}-buscar`) que chama o `.on_submit` com o texto digitado, como o Enter.

use tomni_ui::prelude::*;

const CIDADES: [&str; 8] = [
    "Belo Horizonte",
    "Belém",
    "Brasília",
    "Curitiba",
    "Fortaleza",
    "Recife",
    "Rio de Janeiro",
    "São Paulo",
];

/// Sugere as cidades que começam com o texto, sem diferença de maiúsculas.
pub fn sugerir(texto: &str) -> Vec<String> {
    let texto = texto.trim().to_lowercase();
    if texto.is_empty() {
        return Vec::new();
    }
    CIDADES
        .iter()
        .filter(|c| c.to_lowercase().starts_with(&texto))
        .map(|c| (*c).to_owned())
        .collect()
}

/// Busca de cidade; o texto embaixo mostra a última escolhida.
pub fn busca() -> impl View {
    let cidade = signal(String::new());
    let escolhida = signal(String::new());
    Form::new("Previsão do tempo").child(
        Column::new()
            .gap(12)
            .padding(24)
            .child(
                AutoSuggestBox::bind(cidade, sugerir)
                    .id("cidade")
                    .placeholder("Cidade")
                    .query_icon(Symbol::Search)
                    .height(44)
                    .on_submit(move |c| escolhida.set(c)),
            )
            .child(
                Text::new(move || {
                    let c = escolhida.get();
                    if c.is_empty() {
                        "Nenhuma cidade escolhida".to_owned()
                    } else {
                        format!("Previsão para {c}")
                    }
                })
                .id("resultado"),
            ),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(busca).size(390, 844).run()
}
