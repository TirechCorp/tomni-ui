//! Marco M44 (pedido pelo Tirech SSH): esconder e mostrar janelas (`WindowHandle::hide`,
//! `show` e `is_hidden`) e a bandeja do sistema (`tomni_ui::tray`).

use tomni_ui::prelude::*;
use tomni_ui::{WindowHandle, open_window};
use tomni_ui_test::TestApp;

fn tela() -> Form {
    Form::new("Principal").child(Column::new().child(Text::new("olá").id("texto")).child(
        Button::new("Abrir outra").id("abrir").on_click(|| {
            open_window(|| Form::new("Outra").child(Text::new("segunda")))
                .title("Outra")
                .open();
        }),
    ))
}

#[test]
fn a_janela_comeca_a_mostra_e_esconder_so_muda_o_estado() {
    let mut app = TestApp::new(tela);
    let principal = WindowHandle::main();
    assert!(!app.window_hidden(principal));

    principal.hide();
    app.frame();
    assert!(app.window_hidden(principal));
    assert!(principal.is_hidden());
    assert!(principal.is_open(), "esconder não fecha");
    // A árvore continua lá, com o texto.
    assert_eq!(app.text("texto"), "olá");

    principal.show();
    app.frame();
    assert!(!app.window_hidden(principal));
}

#[test]
fn mostrar_traz_a_janela_a_frente() {
    let mut app = TestApp::new(tela);
    app.click("abrir");
    let outra = *app.windows().last().unwrap();
    assert_ne!(outra, WindowHandle::main());

    outra.hide();
    app.frame();
    assert!(app.window_hidden(outra));
    app.set_active_window(WindowHandle::main());

    outra.show();
    app.frame();
    assert!(!app.window_hidden(outra));
    assert_eq!(tomni_ui::active_window().get_untracked(), outra);
}

#[test]
fn esconder_uma_janela_que_fechou_nao_faz_nada() {
    let mut app = TestApp::new(tela);
    app.click("abrir");
    let outra = *app.windows().last().unwrap();
    outra.close();
    app.frame();
    assert!(!outra.is_open());
    outra.hide();
    app.frame();
    assert!(!app.window_hidden(outra));
}

#[test]
fn fechar_uma_janela_escondida_limpa_o_estado() {
    let mut app = TestApp::new(tela);
    app.click("abrir");
    let outra = *app.windows().last().unwrap();
    outra.hide();
    app.frame();
    assert!(app.window_hidden(outra));
    outra.close();
    app.frame();
    assert!(!outra.is_open());
    assert!(!outra.is_hidden());
}

// ---- A bandeja do sistema ----

use tomni_ui::tray::{self, Tray, TrayImage};

/// Um app com a bandeja: dica, item, item com marca, item que se desabilita e separador.
fn tela_com_bandeja() -> Form {
    let cliques = signal(0);
    let ligado = signal(false);
    let liberado = signal(true);
    Tray::new(TrayImage::from_png(&[]))
        .tooltip(move || format!("Cliques: {}", cliques.get()))
        .on_activate(|| WindowHandle::main().show())
        .item(
            "somar",
            || "Somar".into(),
            move || cliques.update(|n| *n += 1),
        )
        .enabled(move || liberado.get())
        .check(
            "ligar",
            || "Ligar".into(),
            move || ligado.get(),
            move |valor| ligado.set(valor),
        )
        .separator()
        .item("travar", || "Travar".into(), move || liberado.set(false))
        .show()
        .keep();
    Form::new("Principal")
        .child(Text::new(move || format!("ligado: {}", ligado.get())).id("ligado"))
}

#[test]
fn a_bandeja_mostra_a_dica_e_os_itens_na_ordem() {
    let app = TestApp::new(tela_com_bandeja);
    let estado = app.tray().expect("a bandeja aparece");
    assert_eq!(estado.tooltip, "Cliques: 0");
    let ids: Vec<_> = estado.items.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(ids, ["somar", "ligar", "", "travar"]);
    assert_eq!(estado.items[0].label, "Somar");
    assert!(estado.items[1].toggle && !estado.items[1].checked);
    assert!(estado.items[2].separator);
    assert!(
        estado
            .items
            .iter()
            .filter(|i| !i.separator)
            .all(|i| i.enabled)
    );
}

#[test]
fn clicar_num_item_roda_a_acao_e_a_dica_acompanha() {
    let mut app = TestApp::new(tela_com_bandeja);
    app.tray_click("somar").tray_click("somar");
    assert_eq!(app.tray().unwrap().tooltip, "Cliques: 2");
}

#[test]
fn o_item_com_marca_recebe_o_valor_novo() {
    let mut app = TestApp::new(tela_com_bandeja);
    app.tray_click("ligar");
    assert_eq!(app.text("ligado"), "ligado: true");
    assert!(app.tray().unwrap().items[1].checked);
    app.tray_click("ligar");
    assert_eq!(app.text("ligado"), "ligado: false");
    assert!(!app.tray().unwrap().items[1].checked);
}

#[test]
fn item_desabilitado_nao_responde_ao_clique() {
    let mut app = TestApp::new(tela_com_bandeja);
    app.tray_click("travar");
    assert!(!app.tray().unwrap().items[0].enabled);
    app.tray_click("somar");
    assert_eq!(app.tray().unwrap().tooltip, "Cliques: 0");
}

#[test]
fn clicar_no_icone_roda_o_on_activate() {
    let mut app = TestApp::new(tela_com_bandeja);
    let principal = WindowHandle::main();
    principal.hide();
    app.frame();
    assert!(app.window_hidden(principal));
    app.tray_activate();
    assert!(!app.window_hidden(principal));
}

#[test]
fn sem_bandeja_no_sistema_o_estado_some_e_volta() {
    let mut app = TestApp::new(tela_com_bandeja);
    assert!(untrack(tray::available));
    app.set_tray_available(false);
    assert!(!untrack(tray::available));
    assert!(app.tray().is_none());
    app.set_tray_available(true);
    assert!(untrack(tray::available));
    assert_eq!(app.tray().unwrap().tooltip, "Cliques: 0");
}

#[test]
fn soltar_o_handle_tira_o_icone() {
    let handle = std::rc::Rc::new(std::cell::RefCell::new(None));
    let guardado = handle.clone();
    let mut app = TestApp::new(move || {
        *guardado.borrow_mut() = Some(Tray::new(TrayImage::from_png(&[])).show());
        let guardado = guardado.clone();
        Form::new("Principal").child(
            Button::new("Tirar")
                .id("tirar")
                .on_click(move || drop(guardado.borrow_mut().take())),
        )
    });
    assert!(app.tray().is_some());
    app.click("tirar");
    assert!(app.tray().is_none());
}

#[test]
#[should_panic(expected = "a bandeja não tem o item")]
fn clicar_num_item_que_nao_existe_avisa() {
    let mut app = TestApp::new(tela_com_bandeja);
    app.tray_click("nada");
}
