//! `TerminalPanes`: vários terminais lado a lado, em abas e em grade, com o envio do que se
//! digita para mais de um. `cargo run -p tomni-ui --example terminal_panes --features terminal`.
//!
//! As três sessões aqui são `Loopback` com eco, sem servidor nenhum: o que se digita volta na
//! tela. Num app de verdade, troque por `Ssh`, `Process`, `Telnet` ou `Serial`. Ctrl+Shift+D
//! divide o painel ativo, Alt+setas mudam de painel e Ctrl+Shift+Z amplia o painel ativo. O
//! menu da aba (botão direito) fecha, duplica e reinicia a sessão. O botão "Identificar" mostra
//! no centro de cada painel o número dele (`identify`); a lib não tem temporizador, então o
//! botão liga e desliga, e um app que queira os números por alguns segundos agenda o desligar.

use tomni_ui::prelude::*;
use tomni_ui::terminal::{Broadcast, Loopback, PaneLayout, Session, TerminalPanes};

/// Uma sessão de brincadeira: o eco das teclas, com um prompt e um aviso de boas-vindas.
fn sessao(numero: usize) -> Session {
    let nome = format!("web{numero}");
    let sessao = Session::new(Loopback::new().echo())
        .key(nome.clone())
        .name(nome.clone());
    sessao.feed(format!("Conectado a \x1b[1m{nome}\x1b[0m.\r\n").as_bytes());
    sessao.feed(format!("\x1b[1;32muser@{nome}\x1b[0m:\x1b[1;34m~\x1b[0m$ ").as_bytes());
    sessao
}

/// A tela: a barra de layout e de envio em cima, e os painéis ocupando o resto.
pub fn console() -> impl View {
    let layout = signal(PaneLayout::with((1..=3).map(sessao)));
    let envio = signal(Broadcast::None);
    let identificar = signal(false);
    let grade =
        move |colunas: usize, linhas: usize| move || layout.update(|l| l.grid(colunas, linhas));
    // Botões que dividem a linha, com a altura mínima do alvo de toque.
    let botao =
        |texto: &'static str, id: &'static str| Button::new(texto).id(id).grow(1).height(44);
    Form::new("Terminais em painéis").child(
        Column::new()
            .gap(8)
            .padding(16)
            .child(
                Row::new()
                    .gap(8)
                    .child(botao("1", "layout-1").on_click(grade(1, 1)))
                    .child(botao("2 × 2", "layout-2x2").on_click(grade(2, 2)))
                    .child(botao("3 × 3", "layout-3x3").on_click(grade(3, 3)))
                    .child(
                        ToggleButton::bind(identificar, "Identificar")
                            .id("identificar")
                            .grow(1)
                            .height(44),
                    ),
            )
            .child(
                Row::new()
                    .gap(8)
                    .child(
                        botao("Nenhum", "envio-nenhum")
                            .on_click(move || envio.set(Broadcast::None)),
                    )
                    .child(
                        botao("Visíveis", "envio-visiveis")
                            .on_click(move || envio.set(Broadcast::Visible)),
                    )
                    .child(
                        botao("Todos", "envio-todos").on_click(move || envio.set(Broadcast::All)),
                    ),
            )
            .child(Text::new(move || format!("Envio: {}", envio.get().name())).id("modo-envio"))
            .child(
                TerminalPanes::bind(layout)
                    .broadcast(envio)
                    .identify(identificar)
                    .id("paineis")
                    .grow(1),
            ),
    )
}

/// A grade 2 × 2 com os números ligados e o painel 3 em destaque: o que o app mostra quando o
/// ponteiro passa pelo item "Painel 3" de um menu. Para o teste e o snapshot.
pub fn identificando() -> impl View {
    let layout = signal(PaneLayout::with((1..=4).map(sessao)));
    layout.update(|l| l.grid(2, 2));
    Form::new("Painéis identificados").child(
        TerminalPanes::bind(layout)
            .identify(true)
            .highlight_pane(Some(2))
            .id("paineis")
            .grow(1),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(console).size(1000, 700).run()
}
