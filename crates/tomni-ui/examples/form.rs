//! `Form`: a tela, com título (o da janela) e um filho. Um cadastro com validação:
//! `cargo run --example form`.
//!
//! "Criar conta" só habilita com nome, e-mail com `@` e as duas senhas iguais, com 6 caracteres
//! ou mais. Os valores derivados são closures que leem os signals.

use tomni_ui::prelude::*;

/// Cadastro com quatro campos e dois botões.
pub fn cadastro() -> impl View {
    let nome = signal(String::new());
    let email = signal(String::new());
    let senha = signal(String::new());
    let confirmar = signal(String::new());
    let criada = signal(false);
    let valido = move || {
        !nome.get().trim().is_empty()
            && email.get().contains('@')
            && senha.get().chars().count() >= 6
            && senha.get() == confirmar.get()
    };
    let campo = |valor, id: &str, placeholder: &str| {
        Edit::bind(valor)
            .id(id)
            .placeholder(placeholder.to_owned())
            .height(44)
    };
    Form::new("Cadastro").child(
        Column::new()
            .gap(12)
            .padding(24)
            .child(campo(nome, "nome", "Nome"))
            .child(campo(email, "email", "E-mail"))
            .child(campo(senha, "senha", "Senha (6 ou mais)").password())
            .child(campo(confirmar, "confirmar", "Repita a senha").password())
            .child(
                Row::new()
                    .gap(8)
                    .justify(Justify::End)
                    .child(
                        Button::new("Limpar")
                            .id("limpar")
                            .height(44)
                            .on_click(move || {
                                for valor in [nome, email, senha, confirmar] {
                                    valor.set(String::new());
                                }
                                criada.set(false);
                            }),
                    )
                    .child(
                        Button::new(move || {
                            String::from(if criada.get() {
                                "Conta criada"
                            } else {
                                "Criar conta"
                            })
                        })
                        .id("criar")
                        .accent()
                        .height(44)
                        .disabled(move || !valido())
                        .on_click(move || criada.set(true)),
                    ),
            ),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(cadastro).size(390, 844).run()
}
