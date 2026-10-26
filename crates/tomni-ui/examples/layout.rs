//! `Column` e `Row`: espaçamento (`gap`), margem interna (`padding`), alinhamento no eixo
//! cruzado (`align`) e distribuição no eixo principal (`justify`). Duas telas:
//! `cargo run --example layout` abre a `coluna`; `tomni-ui screenshot --example layout --view
//! linha` mostra a outra.

use tomni_ui::prelude::*;

/// `Column` com botões centralizados, cada um da própria largura, no meio da tela.
pub fn coluna() -> impl View {
    Form::new("Coluna").child(
        Column::new()
            .gap(12)
            .padding(24)
            .align(Align::Center)
            .justify(Justify::Center)
            .child(Button::new("Primeiro").id("primeiro").height(44))
            .child(Button::new("Segundo, mais largo").id("segundo").height(44))
            .child(Button::new("Terceiro").id("terceiro").accent().height(44)),
    )
}

/// `Row`s com `Justify::SpaceBetween` e `Justify::End`, e uma barra de ações no fim da tela.
pub fn linha() -> impl View {
    Form::new("Linha").child(
        Column::new()
            .gap(16)
            .padding(24)
            .child(
                Row::new()
                    .justify(Justify::SpaceBetween)
                    .child(Button::new("Voltar").id("voltar").height(44))
                    .child(Button::new("Ajuda").id("ajuda").height(44)),
            )
            .child(
                Row::new()
                    .gap(8)
                    .justify(Justify::End)
                    .child(Button::new("Cancelar").id("cancelar").height(44))
                    .child(Button::new("OK").id("ok").accent().width(88).height(44)),
            ),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(coluna).view(linha).size(390, 844).run()
}
