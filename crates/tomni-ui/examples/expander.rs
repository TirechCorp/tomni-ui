//! `Expander`: cabeçalho que mostra e esconde um conteúdo (sanfona). `cargo run --example
//! expander`.
//!
//! Aberto ou fechado fica num `Signal<bool>`; um clique no cabeçalho, Enter ou Espaço
//! alternam. Fechado, o conteúdo some do layout e o que vem embaixo sobe. O painel do
//! conteúdo tem o id `{id}-conteudo`. No `TestApp`, `app.click("sons")` alterna e
//! `app.is_open("sons")` diz se está aberto.

use tomni_ui::prelude::*;

/// Duas seções recolhíveis, uma já aberta.
pub fn notificacoes() -> impl View {
    let sons = signal(true);
    let avancado = signal(false);
    let vibrar = signal(true);
    let resumo = signal(false);
    Form::new("Notificações").child(
        Column::new()
            .gap(8)
            .padding(24)
            .child(
                Expander::new("Sons", sons).id("sons").content(
                    CheckBox::bind(vibrar, "Vibrar ao tocar")
                        .id("vibrar")
                        .height(44),
                ),
            )
            .child(
                Expander::new("Avançado", avancado).id("avancado").content(
                    Column::new()
                        .gap(8)
                        .child(Text::new("Junta as notificações das 8h às 18h.").wrap())
                        .child(
                            CheckBox::bind(resumo, "Resumo diário")
                                .id("resumo-diario")
                                .height(44),
                        ),
                ),
            )
            .child(
                Text::new(move || {
                    if avancado.get() {
                        "Opções avançadas à mostra".to_owned()
                    } else {
                        "Opções avançadas recolhidas".to_owned()
                    }
                })
                .id("estado"),
            ),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(notificacoes).size(390, 844).run()
}
