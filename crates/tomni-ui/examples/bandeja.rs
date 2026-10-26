//! O ícone do app na bandeja do sistema: um contador cuja dica mostra "Cliques: N", o item
//! "Somar" do menu e "Sair". Fechar a janela só a esconde (o clique no ícone a traz de volta);
//! sem bandeja no sistema, fechar encerra o app. `cargo run -p tomni-ui --example bandeja`.
//!
//! O ícone é um PNG embutido. O `TrayHandle` que `show()` devolve remove o ícone quando é
//! solto, então o exemplo o guarda com `keep()`, até o fim do app.

use tomni_ui::prelude::*;
use tomni_ui::tray::{Tray, TrayImage};
use tomni_ui::{WindowHandle, on_close_requested};

static ICONE: &[u8] = include_bytes!("imagens/bandeja.png");

/// A tela única: o contador, o botão que soma e o que esconde a janela.
pub fn principal() -> impl View {
    let cliques = signal(0);
    // `Sair` liga isto e fecha a janela; sem isso, fechar só esconderia.
    let saindo = signal(false);

    Tray::new(TrayImage::from_png(ICONE))
        .tooltip(move || format!("Cliques: {}", cliques.get()))
        .on_activate(|| WindowHandle::main().show())
        .item(
            "somar",
            || "Somar".to_string(),
            move || cliques.update(|n| *n += 1),
        )
        .separator()
        .item(
            "sair",
            || "Sair".to_string(),
            move || {
                saindo.set(true);
                WindowHandle::main().close();
            },
        )
        .show()
        .keep();

    // Com bandeja, fechar esconde. Sem ela, a pessoa não teria como trazer a janela de volta:
    // fecha de verdade.
    on_close_requested(move || {
        if saindo.get_untracked() || !untrack(tomni_ui::tray::available) {
            return true;
        }
        WindowHandle::main().hide();
        false
    });

    Form::new("Bandeja").child(
        Column::new()
            .gap(8)
            .padding(16)
            .child(Text::new(move || format!("Cliques: {}", cliques.get())).id("cliques"))
            .child(
                Button::new("Somar")
                    .id("somar")
                    .height(44)
                    .on_click(move || cliques.update(|n| *n += 1)),
            )
            .child(
                Button::new("Esconder")
                    .id("esconder")
                    .height(44)
                    .on_click(|| WindowHandle::main().hide()),
            ),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(principal).size(360, 240).run()
}
