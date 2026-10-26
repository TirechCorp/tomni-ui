//! Várias janelas no mesmo processo: o botão "Nova janela" abre outra, as abas dos terminais
//! passam de uma janela para a outra arrastando, e soltar uma aba fora de todas as janelas abre
//! uma janela nova com ela. `cargo run -p tomni-ui --example janelas --features terminal`.
//!
//! As sessões são `Loopback` com eco, sem servidor nenhum. A sessão que muda de janela é a
//! mesma: o histórico continua e nada reconecta. O contador é um signal criado na janela
//! principal e capturado pelas outras, então mudar numa aparece nas duas. No Wayland o sistema
//! não informa a posição das janelas e a aba não vai de uma janela para outra arrastando: use
//! o menu da aba ("Mover para uma janela nova"); soltar fora das janelas funciona.

use tomni_ui::prelude::*;
use tomni_ui::terminal::{Loopback, PaneLayout, Session, TerminalPanes};

/// O grupo de arrasto das abas: todos os `TerminalPanes` dele trocam abas entre si.
const GRUPO: &str = "sessoes";

/// Uma sessão de brincadeira: o eco das teclas, com um prompt e um aviso de boas-vindas.
fn sessao(nome: &str) -> Session {
    let sessao = Session::new(Loopback::new().echo()).key(nome).name(nome);
    sessao.feed(format!("Conectado a \x1b[1m{nome}\x1b[0m.\r\n").as_bytes());
    sessao.feed(format!("\x1b[1;32muser@{nome}\x1b[0m:\x1b[1;34m~\x1b[0m$ ").as_bytes());
    sessao
}

/// O contador e o botão que o aumenta: o mesmo signal em todas as janelas.
fn barra(contador: Signal<i32>) -> Row {
    Row::new()
        .gap(8)
        .child(Text::new(move || format!("Contador: {}", contador.get())).id("contador"))
        .child(
            Button::new("Contar")
                .height(44)
                .id("contar")
                .on_click(move || contador.update(|n| *n += 1)),
        )
}

/// Abre uma janela nova com `sessao` nela. A sessão sai de `origem` antes, sem ser descartada.
fn levar_para_janela_nova(contador: Signal<i32>, origem: Signal<PaneLayout>, sessao: Session) {
    origem.update(|layout| {
        layout.remove(sessao);
    });
    open_window(move || secundaria(contador, sessao))
        .title(format!("Sessão {}", sessao.key_str()))
        .size(640, 420)
        .open();
}

/// Os painéis de uma janela: do grupo comum, com o tear-off e o item "Mover para uma janela
/// nova" no menu das abas.
fn paineis(contador: Signal<i32>, layout: Signal<PaneLayout>) -> TerminalPanes {
    TerminalPanes::bind(layout)
        .id("paineis")
        .drag_group(GRUPO)
        .on_tear_off(move |sessao, _ponto| levar_para_janela_nova(contador, layout, sessao))
        .tab_menu_extra(move |sessao, menu| {
            let id = format!("janela-nova-{}", sessao.key_str());
            menu.item(id, "Mover para uma janela nova", move || {
                levar_para_janela_nova(contador, layout, sessao);
            })
        })
        .grow(1)
}

/// A tela de uma janela aberta depois da principal, com `sessao` na primeira aba. Quando a
/// última aba sai, a janela fecha.
fn secundaria(contador: Signal<i32>, sessao: Session) -> impl View {
    let eu = WindowHandle::current();
    let layout = signal(PaneLayout::with([sessao]));
    effect(move || {
        if layout.with(|l| l.sessions().is_empty()) {
            eu.close();
        }
    });
    Form::new("Outra janela").child(
        Column::new()
            .gap(8)
            .padding(16)
            .child(barra(contador))
            .child(paineis(contador, layout)),
    )
}

/// A tela da janela principal: o contador, o botão "Nova janela" e dois terminais em abas.
pub fn principal() -> impl View {
    let contador = signal(0);
    let layout = signal(PaneLayout::with([sessao("web1"), sessao("web2")]));
    let abertas = signal(2);
    Form::new("Várias janelas").child(
        Column::new()
            .gap(8)
            .padding(16)
            .child(
                Row::new()
                    .gap(8)
                    .child(
                        Button::new("Nova janela")
                            .id("nova-janela")
                            .on_click(move || {
                                abertas.update(|n| *n += 1);
                                let numero = abertas.get_untracked();
                                let nova = sessao(&format!("web{numero}"));
                                open_window(move || secundaria(contador, nova))
                                    .title(format!("Janela com web{numero}"))
                                    .size(640, 420)
                                    .open();
                            }),
                    )
                    .child(barra(contador)),
            )
            .child(paineis(contador, layout)),
    )
}

/// A tela de uma janela secundária, para o lint e os testes: um contador próprio e uma sessão.
pub fn janela_secundaria() -> impl View {
    secundaria(signal(0), sessao("extra"))
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(principal)
        .view(janela_secundaria)
        .size(900, 600)
        .run()
}
