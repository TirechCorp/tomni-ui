//! `ProgressBar` e `ProgressRing`: mostram que algo está em andamento. `cargo run --example
//! progress`.
//!
//! Com `.value(...)` (de 0 a 1, fixo, closure ou signal) mostram quanto já foi; sem
//! `.value` ficam indeterminados, animados. Nos screenshots a animação fica parada no
//! primeiro quadro; no `TestApp`, anda com `app.wait(...)`. `app.value("barra")` lê o
//! andamento. `ProgressBar::paused(..)` e `error(..)` pintam a barra de alerta e de erro (o
//! `ShowPaused` e o `ShowError` do WinUI).

use tomni_ui::prelude::*;

/// Um envio em partes: o botão avança, a barra e o anel acompanham.
pub fn envio() -> impl View {
    let enviado = signal(0.25);
    Form::new("Envio").child(
        Column::new()
            .gap(16)
            .padding(24)
            .child(Text::new("Enviando fotos"))
            .child(ProgressBar::new().id("barra").value(enviado))
            .child(
                Row::new()
                    .gap(16)
                    .align(Align::Center)
                    .child(ProgressRing::new().id("anel").value(enviado))
                    .child(
                        Text::new(move || format!("{:.0}% enviado", enviado.get() * 100.0))
                            .id("resumo"),
                    ),
            )
            .child(
                Button::new("Avançar")
                    .id("avancar")
                    .height(44)
                    .disabled(move || enviado.get() >= 1.0)
                    .on_click(move || enviado.update(|v| *v = (*v + 0.25).min(1.0))),
            )
            .child(Text::new("Envio pausado"))
            .child(ProgressBar::new().id("pausada").value(0.6).paused(true))
            .child(Text::new("Envio com erro"))
            .child(ProgressBar::new().id("com-erro").value(0.3).error(true))
            .child(Text::new("Procurando atualizações"))
            .child(ProgressBar::new().id("procurando"))
            .child(
                Row::new()
                    .gap(16)
                    .align(Align::Center)
                    .child(ProgressRing::new().id("carregando"))
                    .child(Text::new("Carregando a lista")),
            ),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(envio).size(390, 844).run()
}
