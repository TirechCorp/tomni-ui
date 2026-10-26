//! `ComboBox`: escolhe um valor numa lista. `cargo run --example combo_box`.
//!
//! O valor fica num `Signal<String>` (vazio enquanto nada foi escolhido). A lista abre embaixo
//! da caixa, ou em cima se não couber, e não ocupa espaço no layout. Cada item tem o id
//! `{id}-{opção}` (`uf-sp`); no `TestApp`, `app.select("uf", "SP")` faz o caminho todo.

use tomni_ui::prelude::*;

/// UF e cidade; o texto embaixo mostra o que foi escolhido.
pub fn endereco() -> impl View {
    let uf = signal(String::new());
    let cidade = signal("Belo Horizonte".to_owned());
    Form::new("Endereço").child(
        Column::new()
            .gap(12)
            .padding(24)
            .child(
                ComboBox::bind(uf, ["MG", "RJ", "SP"])
                    .id("uf")
                    .placeholder("Escolha a UF")
                    .height(44),
            )
            .child(
                ComboBox::bind(cidade, ["Belo Horizonte", "Rio de Janeiro", "São Paulo"])
                    .id("cidade")
                    .height(44),
            )
            .child(
                ComboBox::bind(signal("Brasil".to_owned()), ["Brasil"])
                    .id("pais")
                    .disabled(true)
                    .height(44),
            )
            .child(
                Text::new(move || {
                    let uf = uf.get();
                    if uf.is_empty() {
                        "Nenhuma UF escolhida".to_owned()
                    } else {
                        format!("{}, {uf}", cidade.get())
                    }
                })
                .id("resumo"),
            ),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(endereco).size(390, 844).run()
}
