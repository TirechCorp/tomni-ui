//! `Edit`: campo ligado a um `Signal<String>`, com placeholder, senha e desabilitado.
//! `cargo run --example edit`.
//!
//! O texto digitado vai direto para o signal, e mudar o signal muda o campo.

use tomni_ui::prelude::*;

/// Campos de texto e um botão que limpa o nome.
pub fn campos() -> impl View {
    let nome = signal(String::new());
    let senha = signal(String::new());
    let travado = signal(String::from("Não dá para editar"));
    Form::new("Campos").child(
        Column::new()
            .gap(12)
            .padding(24)
            .child(Edit::bind(nome).id("nome").placeholder("Nome").height(44))
            .child(
                Edit::bind(senha)
                    .id("senha")
                    .placeholder("Senha")
                    .password()
                    .height(44),
            )
            .child(Edit::bind(travado).id("travado").disabled(true).height(44))
            .child(
                Button::new(move || format!("Limpar ({} letras)", nome.get().chars().count()))
                    .id("limpar")
                    .height(44)
                    .disabled(move || nome.get().is_empty())
                    .on_click(move || nome.set(String::new())),
            ),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(campos).size(390, 844).run()
}
