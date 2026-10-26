//! `ContentDialog`: caixa modal que pede uma decisão. `cargo run --example content_dialog`.
//!
//! A caixa abre quando o signal vira `true` e fecha sozinha quando um botão é escolhido. Ela
//! não ocupa espaço no layout: ponha em qualquer lugar da tela. Os botões têm os ids
//! `{id}-primary`, `{id}-secondary` e `{id}-close`; Enter faz o mesmo que o primário e Esc
//! fecha sem ação.

use tomni_ui::prelude::*;

/// "Sair sem salvar?" com Salvar, Descartar e Cancelar; o texto mostra o que aconteceu.
pub fn sair() -> impl View {
    let aberto = signal(false);
    let resultado = signal("Há alterações não salvas.".to_owned());
    Form::new("Editor").child(
        Column::new()
            .gap(12)
            .padding(24)
            .child(Text::new(resultado).id("resultado").wrap())
            .child(
                Button::new("Sair")
                    .id("sair")
                    .height(44)
                    .on_click(move || aberto.set(true)),
            )
            .child(
                ContentDialog::new("Sair sem salvar?", aberto)
                    .id("dialogo")
                    .content(
                        Text::new("As alterações feitas desde o último salvamento se perdem.")
                            .wrap(),
                    )
                    .primary("Salvar", move || resultado.set("Salvo.".to_owned()))
                    .secondary("Descartar", move || {
                        resultado.set("Alterações descartadas.".to_owned());
                    })
                    .close("Cancelar"),
            ),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(sair).size(390, 844).run()
}
