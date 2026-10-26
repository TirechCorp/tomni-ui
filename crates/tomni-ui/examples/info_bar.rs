//! `InfoBar`: barra de aviso nas quatro severidades, com o botão de fechar e o de ação.
//! `cargo run --example info_bar`.
//!
//! A barra não se esconde sozinha: o `.on_close` muda um signal e o `.visible` lê o mesmo
//! signal. Oculta, ela sai do layout, do Tab e do lint. O `.action` põe um botão depois da
//! mensagem, com o id `<id da barra>-acao`.

use tomni_ui::prelude::*;

/// As quatro severidades; o botão "Mostrar de novo" reabre a de erro.
pub fn avisos() -> impl View {
    let erro = signal(true);
    Form::new("Avisos").child(
        Column::new()
            .gap(12)
            .padding(24)
            .child(
                InfoBar::new(InfoBarSeverity::Informational)
                    .id("info")
                    .title("Novidade")
                    .message("Há uma versão nova do app."),
            )
            .child(
                InfoBar::new(InfoBarSeverity::Success)
                    .id("sucesso")
                    .title("Pronto")
                    .message("Conta criada."),
            )
            .child(
                InfoBar::new(InfoBarSeverity::Success)
                    .id("atualizado")
                    .title("Instalado")
                    .message("A versão nova está pronta.")
                    .action("Reiniciar agora", || {}),
            )
            .child(
                InfoBar::new(InfoBarSeverity::Warning)
                    .id("aviso")
                    .title("Conexão instável")
                    .message("As alterações serão salvas quando a conexão voltar."),
            )
            .child(
                InfoBar::new(InfoBarSeverity::Error)
                    .id("erro")
                    .title("Erro")
                    .message("Este e-mail já está cadastrado.")
                    .visible(erro)
                    .on_close(move || erro.set(false)),
            )
            .child(
                Button::new("Mostrar de novo")
                    .id("reabrir")
                    .height(44)
                    .visible(move || !erro.get())
                    .on_click(move || erro.set(true)),
            ),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(avisos).size(390, 844).run()
}
