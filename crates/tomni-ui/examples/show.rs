//! `Show`: troca uma view por outra quando a condição muda. `cargo run --example show`.
//!
//! A view que sai é descartada (o texto digitado nela some) e a que entra é criada de novo.
//! Para só esconder, mantendo o estado, use `.visible()`: compare os dois campos.

use tomni_ui::prelude::*;

/// "Entrar" mostra o campo de apelido no lugar do aviso; "Sair" volta. Embaixo, o mesmo
/// campo com `.visible()`, que guarda o texto enquanto está escondido.
pub fn conta() -> impl View {
    let logado = signal(false);
    let notas = signal(true);
    Form::new("Conta").child(
        Column::new()
            .gap(12)
            .padding(24)
            .child(
                Button::new(move || if logado.get() { "Sair" } else { "Entrar" }.to_owned())
                    .id("entrar")
                    .accent()
                    .height(44)
                    .on_click(move || logado.update(|l| *l = !*l)),
            )
            .child(
                Show::when(logado, || {
                    let apelido = signal(String::new());
                    Edit::bind(apelido)
                        .id("apelido")
                        .placeholder("Apelido")
                        .height(44)
                })
                .otherwise(|| Text::new("Entre para escolher um apelido.").id("aviso")),
            )
            .child(
                Button::new(move || {
                    if notas.get() {
                        "Esconder notas"
                    } else {
                        "Mostrar notas"
                    }
                    .to_owned()
                })
                .id("notas")
                .height(44)
                .on_click(move || notas.update(|n| *n = !*n)),
            )
            .child(
                Edit::bind(signal(String::new()))
                    .id("rascunho")
                    .placeholder("Notas (ficam guardadas)")
                    .height(44)
                    .visible(notas),
            ),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(conta).size(390, 844).run()
}
