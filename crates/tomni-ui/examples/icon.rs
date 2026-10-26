//! `Icon` e os botões com ícone: `Button::icon`, `Button::icon_only` e
//! `ToggleButton::icon_only`. `cargo run --example icon`.
//!
//! O ícone tem 20 px e a cor do texto; `.size` escolhe 12, 16, 20 ou 24 e `.color`, outra cor.
//! Ao lado de um texto que já diz o mesmo, ele é só enfeite: `.decorative()` dispensa o lint de
//! contraste. O botão só de ícone é um quadrado de 44 px e pede `.label`, que vira a dica e o
//! nome na árvore e nos testes.

use tomni_ui::prelude::*;

/// Barra de um editor de documento: desfazer, refazer e favorito só com ícone, salvar e
/// compartilhar com ícone e rótulo, e uma linha de status com a última ação.
pub fn editor() -> impl View {
    let favorito = signal(false);
    let status = signal(String::from("Nenhuma alteração"));
    let acao = move |texto: &'static str| move || status.set(texto.to_owned());
    Form::new("Documento").child(
        Column::new()
            .gap(16)
            .padding(24)
            .child(
                Row::new()
                    .gap(8)
                    .align(Align::Center)
                    .child(Icon::new(Symbol::DocumentText).size(24).decorative())
                    .child(Text::new("Relatório anual").subtitle()),
            )
            .child(
                Row::new()
                    .gap(4)
                    .child(
                        Button::icon_only(Symbol::ArrowUndo)
                            .id("desfazer")
                            .label("Desfazer")
                            .on_click(acao("Desfeito")),
                    )
                    .child(
                        Button::icon_only(Symbol::ArrowRedo)
                            .id("refazer")
                            .label("Refazer")
                            .on_click(acao("Refeito")),
                    )
                    .child(
                        ToggleButton::icon_only(favorito, Symbol::Star)
                            .id("favorito")
                            .label("Favorito"),
                    ),
            )
            .child(
                Row::new()
                    .gap(8)
                    .child(
                        Button::new("Salvar")
                            .icon(Symbol::Save)
                            .accent()
                            .id("salvar")
                            .height(44)
                            .grow(1)
                            .on_click(acao("Salvo")),
                    )
                    .child(
                        Button::new("Compartilhar")
                            .icon(Symbol::Share)
                            .id("compartilhar")
                            .height(44)
                            .grow(1)
                            .on_click(acao("Link copiado")),
                    ),
            )
            .child(
                Row::new()
                    .gap(8)
                    .align(Align::Center)
                    .child(
                        Icon::new(move || {
                            if favorito.get() {
                                Symbol::StarFilled
                            } else {
                                Symbol::Info
                            }
                        })
                        .id("status-icone")
                        .size(16)
                        .color(Token::AccentTextFillColorPrimary),
                    )
                    .child(Text::new(move || status.get()).id("status")),
            ),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(editor).size(390, 844).run()
}
