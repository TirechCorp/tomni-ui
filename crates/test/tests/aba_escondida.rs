//! `TabView::tab_visible`: a aba que some sai da faixa e do Ctrl+Tab, o painel dela não
//! aparece, e a escolha passa para a vizinha (pedido do Tirech Omni para a aba Sensores, que
//! some sem sensor).

use std::cell::Cell;
use std::rc::Rc;

use tomni_ui::prelude::*;
use tomni_ui::{Key, Modifiers};
use tomni_ui_test::TestApp;

/// Um `TabView` `abas` com `a`, `b` (fixas) e `c` (de uma lista), em que `b` aparece só com o
/// signal `mostrar`.
fn abas() -> (TestApp, Signal<bool>, Signal<String>) {
    let guardado = Rc::new(Cell::new(None));
    let g = Rc::clone(&guardado);
    let app = TestApp::new(move || {
        let mostrar = signal(false);
        let aba = signal("a".to_owned());
        g.set(Some((mostrar, aba)));
        Form::new("Abas").child(
            TabView::bind(aba)
                .id("abas")
                .tab("a", "A", Text::new("painel a"))
                .tab("b", "B", Text::new("painel b").id("texto-b"))
                .items(
                    || vec!["c".to_owned()],
                    |k| k.clone(),
                    |k| k.clone(),
                    Text::new,
                )
                .tab_visible(move |chave| chave != "b" || mostrar.get())
                .height(160),
        )
    })
    .size(600, 300);
    let (mostrar, aba) = guardado.get().unwrap();
    (app, mostrar, aba)
}

#[test]
fn a_aba_escondida_nao_aparece_nem_no_ctrl_tab() {
    let (mut app, mostrar, aba) = abas();
    assert!(app.visible("abas-a"));
    assert!(!app.visible("abas-b"));
    assert!(app.visible("abas-c"));

    // Ctrl+Tab vai de "a" direto a "c".
    app.click("abas-a");
    app.key_with(Key::Tab, Modifiers::CTRL);
    assert_eq!(aba.get_untracked(), "c");

    // Escolher "b" pelo código, escondida, mostra a primeira.
    aba.set("b".to_owned());
    app.frame();
    assert!(!app.visible("texto-b"));
    assert_eq!(app.text("abas"), "a");

    // A aba volta: a faixa e o painel aparecem.
    mostrar.set(true);
    app.frame();
    assert!(app.visible("abas-b"));
    assert!(app.visible("texto-b"));
}

#[test]
fn a_escolhida_que_some_passa_a_escolha_para_a_vizinha() {
    let (mut app, mostrar, aba) = abas();
    mostrar.set(true);
    aba.set("b".to_owned());
    app.frame();
    assert!(app.visible("texto-b"));

    mostrar.set(false);
    app.frame();
    assert_eq!(aba.get_untracked(), "c");
    assert!(!app.visible("abas-b"));
    assert!(!app.visible("texto-b"));

    // Quando volta, não é escolhida de novo.
    mostrar.set(true);
    app.frame();
    assert_eq!(aba.get_untracked(), "c");
}
