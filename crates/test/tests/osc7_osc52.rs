//! Marco M46 (pedido pelo Tirech Omni): o diretório que o programa informa (`OSC 7`, por
//! `Session::cwd`) e o texto que ele pede para copiar (`OSC 52`, por `Terminal::osc52`).

use tomni_ui::prelude::*;
use tomni_ui::terminal::{Loopback, LoopbackHandle, Session};
use tomni_ui_test::TestApp;

fn app_com(
    configurar: impl FnOnce(Terminal) -> Terminal + 'static,
) -> (TestApp, LoopbackHandle, Session) {
    let conexao = Loopback::new();
    let handle = conexao.handle();
    let sessao = Session::new(conexao);
    let dela = sessao;
    let app = TestApp::new(move || {
        Form::new("Console").child(
            configurar(Terminal::session(dela).id("console").cols_rows(80, 24))
                .paste_warning(false),
        )
    })
    .size(780, 560);
    (app, handle, sessao)
}

#[test]
fn cwd_acompanha_o_osc_7() {
    let (mut app, handle, sessao) = app_com(|t| t);
    assert_eq!(sessao.cwd(), None);
    handle.feed("\x1b]7;file://srv/home/ana/meu%20projeto\x07");
    app.frame();
    assert_eq!(sessao.cwd().as_deref(), Some("/home/ana/meu projeto"));
    handle.feed("\x1b]7;file://srv/tmp\x1b\\");
    app.frame();
    assert_eq!(sessao.cwd().as_deref(), Some("/tmp"));
    // O que não é `file://` não troca o diretório.
    handle.feed("\x1b]7;/etc\x07");
    app.frame();
    assert_eq!(sessao.cwd().as_deref(), Some("/tmp"));
}

#[test]
fn restart_limpa_o_cwd() {
    let (mut app, handle, sessao) = app_com(|t| t);
    handle.feed("\x1b]7;file://srv/tmp\x07");
    app.frame();
    assert_eq!(sessao.cwd().as_deref(), Some("/tmp"));
    handle.close(Some("caiu".into()));
    Session::poll_all();
    assert!(sessao.restart());
    assert_eq!(sessao.cwd(), None);
}

#[test]
fn osc_52_copia_para_a_area_de_transferencia() {
    let (mut app, handle, _) = app_com(|t| t);
    handle.feed("\x1b]52;c;aGVsbG8=\x07");
    app.frame();
    app.frame();
    assert_eq!(app.clipboard().read().as_deref(), Some("hello"));
}

#[test]
fn osc_52_so_o_ultimo_pedido_vale() {
    let (mut app, handle, _) = app_com(|t| t);
    handle.feed("\x1b]52;c;b25l\x07\x1b]52;c;dHdv\x07");
    app.frame();
    app.frame();
    assert_eq!(app.clipboard().read().as_deref(), Some("two"));
}

#[test]
fn osc_52_ignora_leitura_e_base64_torto() {
    let (mut app, handle, _) = app_com(|t| t);
    handle.feed("\x1b]52;c;?\x07\x1b]52;c;@@@@\x07\x1b]52;c;aGVsbG8\x07");
    app.frame();
    app.frame();
    assert_eq!(app.clipboard().read(), None);
}

#[test]
fn osc_52_desligado_nao_copia() {
    let ligado = signal(false);
    let (mut app, handle, _) = app_com(move |t| t.osc52(ligado));
    handle.feed("\x1b]52;c;aGVsbG8=\x07");
    app.frame();
    app.frame();
    assert_eq!(app.clipboard().read(), None);
    // O pedido que chegou desligado não fica esperando a hora de ligar.
    ligado.set(true);
    app.frame();
    app.frame();
    assert_eq!(app.clipboard().read(), None);
    handle.feed("\x1b]52;c;b3V0cm8=\x07");
    app.frame();
    app.frame();
    assert_eq!(app.clipboard().read().as_deref(), Some("outro"));
}
