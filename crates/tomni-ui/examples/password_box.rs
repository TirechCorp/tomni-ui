//! Senha: `Edit::password()` com o botão de revelar, como o `PasswordBox` do WinUI.
//! `cargo run --example password_box`.
//!
//! O olho aparece no fim do campo depois que a pessoa digita, e mostra a senha só enquanto
//! está pressionado. `.password_reveal(...)` escolhe o modo: `Peek` (padrão; o olho some
//! quando o campo perde o foco com texto, até apagar tudo), `Hidden` (sem olho) e `Visible`
//! (a senha à vista), os três do `PasswordRevealMode` do WinUI. O botão tem o id `{id}-revelar`; no `TestApp`:
//! `app.press("senha-revelar")`, `app.edit("senha").text` e `app.release()`.

use tomni_ui::prelude::*;

/// Login com o olho no campo e um PIN que a caixa de seleção mostra ou esconde.
pub fn senhas() -> impl View {
    let senha = signal(String::new());
    let pin = signal(String::new());
    let mostrar_pin = signal(false);
    Form::new("Senhas").child(
        Column::new()
            .gap(12)
            .padding(24)
            .child(
                Edit::bind(senha)
                    .id("senha")
                    .placeholder("Senha")
                    .password()
                    .height(44),
            )
            .child(
                Edit::bind(pin)
                    .id("pin")
                    .placeholder("PIN")
                    .password()
                    .password_reveal(move || {
                        if mostrar_pin.get() {
                            PasswordReveal::Visible
                        } else {
                            PasswordReveal::Hidden
                        }
                    })
                    .height(44),
            )
            .child(
                CheckBox::bind(mostrar_pin, "Mostrar PIN")
                    .id("mostrar-pin")
                    .height(44),
            )
            .child(
                Text::new(move || {
                    format!("A senha tem {} caracteres", senha.get().chars().count())
                })
                .id("tamanho")
                .caption(),
            ),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(senhas).size(390, 844).run()
}
