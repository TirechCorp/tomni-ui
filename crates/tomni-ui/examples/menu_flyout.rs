//! `MenuFlyout`: menu de ações aberto por um botão. `cargo run --example menu_flyout`.
//!
//! O menu entra no botão com `.flyout(...)` e não ocupa espaço no layout. Cada item tem um id
//! próprio e uma ação; escolher um item roda a ação e fecha o menu. Esc ou um clique fora
//! fecham sem escolher. `.toggle(..)` marca e desmarca um valor, e `.radio(..)` escolhe um entre
//! vários de um mesmo `Signal` (o item do grupo marcado leva um ponto). Os dois reservam a
//! coluna da marca em todos os itens do menu. `.item_enabled(..)` desabilita um item: ele fica
//! esmaecido, não responde ao clique, ao atalho nem ao Enter, e o teclado passa por cima dele.

use tomni_ui::prelude::*;

/// Botão "Mais opções" com três ações; o texto mostra a última escolhida.
pub fn menu() -> impl View {
    let ultima = signal("Nenhuma ação ainda".to_owned());
    let ordem = signal("nome".to_owned());
    Form::new("Documento").child(
        Column::new()
            .gap(12)
            .padding(24)
            .child(
                Button::new("Mais opções").id("mais").height(44).flyout(
                    MenuFlyout::new()
                        .id("menu")
                        .item("copiar", "Copiar link", move || {
                            ultima.set("Link copiado".to_owned());
                        })
                        .item("renomear", "Renomear", move || {
                            ultima.set("Renomeando".to_owned());
                        })
                        .item("mover", "Mover para…", move || {
                            ultima.set("Movendo".to_owned());
                        })
                        .item_enabled(false)
                        .separator()
                        .item("apagar", "Apagar", move || {
                            ultima.set("Documento apagado".to_owned());
                        }),
                ),
            )
            .child(
                Button::new("Ordenar").id("ordenar").height(44).flyout(
                    MenuFlyout::new()
                        .id("ordem")
                        .radio("ordem-nome", "Por nome", ordem, "nome".to_owned())
                        .radio("ordem-data", "Por data", ordem, "data".to_owned())
                        .radio("ordem-tamanho", "Por tamanho", ordem, "tamanho".to_owned()),
                ),
            )
            .child(Text::new(move || format!("Ordem: {}", ordem.get())).id("ordem-atual"))
            .child(Text::new(ultima).id("ultima")),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(menu).size(390, 844).run()
}
