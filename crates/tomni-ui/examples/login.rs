//! Login da spec numa janela: `cargo run --example login`. No Android, o `android_main` é o
//! ponto de entrada da `NativeActivity` (`cargo apk run --example login`).
//!
//! "Entrar" só habilita com um e-mail e uma senha de 6 caracteres ou mais.

use tomni_ui::prelude::*;

/// Login com e-mail, senha e o botão que só habilita com os dois válidos.
pub fn login() -> impl View {
    let email = signal(String::new());
    let senha = signal(String::new());
    let entrou = signal(false);
    Form::new("Login").child(
        Column::new()
            .gap(12)
            .padding(24)
            .child(
                Edit::bind(email)
                    .id("email")
                    .placeholder("E-mail")
                    .height(44),
            )
            .child(
                Edit::bind(senha)
                    .id("senha")
                    .placeholder("Senha")
                    .password()
                    .height(44),
            )
            .child(
                Button::new(move || {
                    String::from(if entrou.get() { "Bem-vindo!" } else { "Entrar" })
                })
                .id("entrar")
                .accent()
                .height(44)
                .disabled(move || !email.get().contains('@') || senha.get().chars().count() < 6)
                .on_click(move || entrou.set(true)),
            ),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(login).size(390, 844).run()
}

#[cfg(target_os = "android")]
#[allow(
    unsafe_code,
    reason = "a NativeActivity procura o símbolo `android_main`"
)]
#[unsafe(no_mangle)]
fn android_main(app: tomni_ui::platform::AndroidApp) {
    if let Err(e) = tomni_ui::App::new(login).android(app).run() {
        eprintln!("{e}");
    }
}
