//! `TabView::on_tab_click`: o clique numa aba avisa o app mesmo quando ela já era a escolhida
//! (o `Tapped` do `TabViewItem` do WinUI), que não muda o signal. O teclado não avisa.

use std::cell::RefCell;
use std::rc::Rc;

use tomni_ui::prelude::*;
use tomni_ui_test::TestApp;

fn montar() -> (TestApp, Signal<String>, Rc<RefCell<Vec<String>>>) {
    let cliques = Rc::new(RefCell::new(Vec::new()));
    let aba = signal("x".to_owned());
    let anotar = Rc::clone(&cliques);
    let app = TestApp::new(move || {
        let anotar = Rc::clone(&anotar);
        Form::new("Abas").child(
            TabView::bind(aba)
                .id("abas")
                .tab("x", "Primeira", Text::new("um"))
                .tab("y", "Segunda", Text::new("dois"))
                .on_tab_click(move |chave| anotar.borrow_mut().push(chave))
                .height(120),
        )
    })
    .size(600, 300);
    (app, aba, cliques)
}

#[test]
fn o_clique_avisa_tambem_na_aba_ja_escolhida() {
    let (mut app, aba, cliques) = montar();
    app.click("abas-x");
    assert_eq!(*cliques.borrow(), ["x"], "a aba já escolhida também avisa");
    assert_eq!(aba.get_untracked(), "x");
    app.click("abas-y");
    assert_eq!(aba.get_untracked(), "y", "o aviso vem depois de escolher");
    assert_eq!(*cliques.borrow(), ["x", "y"]);
}

#[test]
fn escolher_pelo_teclado_nao_avisa() {
    let (mut app, aba, cliques) = montar();
    app.click("abas-x");
    cliques.borrow_mut().clear();
    app.key(Key::Right);
    assert_eq!(aba.get_untracked(), "y");
    assert!(cliques.borrow().is_empty());
}
