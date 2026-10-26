//! Marco M25: os três acréscimos da lib para o cofre de senhas do Tirech Omni: a escrita no
//! clipboard fora de um widget, o motivo do encerramento da sessão e a lista de opções que o
//! app calcula na hora (`choice_from`).

use tomni_ui::kurbo::Point;
use tomni_ui::prelude::*;
use tomni_ui::terminal::{Loopback, Session};
use tomni_ui::{Key, clipboard};
use tomni_ui_test::TestApp;

fn clicar_no_nome(app: &mut TestApp, nome: &str) {
    let r = app.rect(&format!("props-{nome}"));
    app.click_at(Point::new(r.x0 + 30.0, r.center().y), 1);
}

#[test]
fn clipboard_write_chega_ao_clipboard_no_proximo_quadro() {
    let mut app = TestApp::new(|| {
        Form::new("Copiar").child(Button::new("Copiar").id("copiar").on_click(|| {
            clipboard::write("segredo");
        }))
    });
    assert_eq!(app.clipboard().read(), None);
    app.click("copiar");
    app.frame();
    assert_eq!(app.clipboard().read().as_deref(), Some("segredo"));
}

#[test]
fn clipboard_write_aceita_string_e_a_ultima_vence() {
    let mut app = TestApp::new(|| Form::new("Vazio").child(Text::new("x")));
    clipboard::write(String::from("um"));
    clipboard::write("dois");
    app.frame();
    assert_eq!(app.clipboard().read().as_deref(), Some("dois"));
}

#[test]
fn close_reason_guarda_o_motivo_e_e_reativo() {
    let lb = Loopback::new();
    let handle = lb.handle();
    let sessao = Session::new(lb);
    let visto = signal(String::from("aberta"));
    effect(move || {
        visto.set(format!("{} / {:?}", sessao.closed(), sessao.close_reason()));
    });
    run_effects();
    assert_eq!(sessao.close_reason(), None);

    handle.close(Some("autenticação recusada para ana".into()));
    Session::poll_all();
    run_effects();
    assert_eq!(
        sessao.close_reason().as_deref(),
        Some("autenticação recusada para ana")
    );
    assert_eq!(
        visto.get_untracked(),
        "true / Some(\"autenticação recusada para ana\")"
    );

    assert!(sessao.restart());
    run_effects();
    assert_eq!(sessao.close_reason(), None);
    sessao.dispose();
}

#[test]
fn choice_from_acompanha_as_opcoes() {
    let opcoes = signal(vec!["Alfa".to_owned(), "Beta".to_owned()]);
    let valor = signal(String::from("Alfa"));
    let mut app = TestApp::new(move || {
        Form::new("Escolha").child(
            PropertyGrid::new()
                .id("props")
                .choice_from("Item", valor, move || opcoes.get())
                .grow(1),
        )
    })
    .size(480, 240);

    clicar_no_nome(&mut app, "Item");
    app.key(Key::F(4));
    assert!(app.is_open("props-Item-valor-lista"));
    assert!(app.visible("props-Item-valor-alfa"));
    assert!(app.visible("props-Item-valor-beta"));
    app.key(Key::Escape);

    // Muda a lista com a linha ainda escolhida: o editor se refaz com as opções novas.
    opcoes.set(vec!["Gama".to_owned()]);
    app.frame();
    clicar_no_nome(&mut app, "Item");
    app.key(Key::F(4));
    assert!(app.is_open("props-Item-valor-lista"));
    assert!(app.visible("props-Item-valor-gama"));
    assert!(app.tree().find("props-Item-valor-alfa").is_none());
    app.click("props-Item-valor-gama");
    assert_eq!(valor.get_untracked(), "Gama");
}

#[test]
fn choice_de_sempre_continua_igual() {
    let valor = signal(String::from("SSH"));
    let mut app = TestApp::new(move || {
        Form::new("Escolha").child(
            PropertyGrid::new()
                .id("props")
                .choice("Protocolo", valor, ["SSH", "Telnet"])
                .grow(1),
        )
    })
    .size(480, 240);
    clicar_no_nome(&mut app, "Protocolo");
    app.key(Key::F(4));
    app.click("props-Protocolo-valor-telnet");
    assert_eq!(valor.get_untracked(), "Telnet");
}
