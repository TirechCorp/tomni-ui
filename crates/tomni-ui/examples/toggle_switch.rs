//! `ToggleSwitch`: chave ligada a um `Signal<bool>`, com "Ligado"/"Desligado" ao lado e um
//! rótulo opcional em cima (`.header`). `cargo run --example toggle_switch`.
//!
//! Clique e Espaço alternam. Com o rótulo, a chave tem 64 px de altura; sem, 40 (use
//! `.height(44)` para o lint de touch-target).

use tomni_ui::prelude::*;

/// Duas chaves: a segunda só habilita com a primeira ligada.
pub fn chaves() -> impl View {
    let notificacoes = signal(true);
    let som = signal(false);
    Form::new("Chaves").child(
        Column::new()
            .gap(12)
            .padding(24)
            .child(
                ToggleSwitch::bind(notificacoes)
                    .header("Notificações")
                    .id("notificacoes"),
            )
            .child(
                ToggleSwitch::bind(som)
                    .header("Tocar um som")
                    .id("som")
                    .disabled(move || !notificacoes.get()),
            )
            .child(
                Text::new(move || {
                    if notificacoes.get() && som.get() {
                        "Avisos com som".to_owned()
                    } else if notificacoes.get() {
                        "Avisos em silêncio".to_owned()
                    } else {
                        "Sem avisos".to_owned()
                    }
                })
                .id("resumo"),
            ),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(chaves).size(390, 844).run()
}
