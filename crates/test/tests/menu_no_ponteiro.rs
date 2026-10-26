//! O menu de contexto aberto no ponteiro que não cabe nem abaixo nem acima dele: como o
//! `MenuFlyout` do WinUI, aparece inteiro, empurrado para dentro da janela, e só rola se for
//! maior que a janela toda.

use tomni_ui::prelude::*;
use tomni_ui_test::TestApp;

const ITENS: usize = 7;

fn tela() -> Form {
    let menu = (0..ITENS).fold(MenuFlyout::new().id("menu"), |menu, i| {
        menu.item(format!("item-{i}"), format!("Item {i}"), || {})
    });
    Form::new("Menu").child(
        Column::new()
            .child(Text::new("Acima").height(170))
            .child(Text::new("Alvo").id("alvo").height(44).context_flyout(menu))
            .child(Text::new("Abaixo").grow(1)),
    )
}

#[test]
fn menu_que_nao_cabe_em_nenhum_lado_aparece_inteiro_na_janela() {
    let (largura, altura) = (600, 400);
    let mut app = TestApp::new(tela).size(largura, altura);
    app.context_menu("alvo");
    assert!(app.is_open("menu"));
    let ponto = app.rect("alvo").center();
    let menu = app.rect("menu");
    let ultimo = app.rect(&format!("item-{}", ITENS - 1));
    // O caso do teste: o menu inteiro é mais alto que o espaço de cada lado do ponteiro.
    let altura_natural = ultimo.y1 - app.rect("item-0").y0;
    assert!(
        altura_natural > f64::from(altura) - ponto.y && altura_natural > ponto.y,
        "o menu precisa ser maior que os dois lados: {altura_natural} em {ponto:?}"
    );
    assert!(menu.y0 >= 0.0 && menu.y1 <= f64::from(altura), "{menu:?}");
    assert!(
        ultimo.y1 <= menu.y1 + 0.5,
        "o último item aparece sem rolar: {ultimo:?} em {menu:?}"
    );
    assert!(
        (menu.x0 - ponto.x).abs() < 1.0,
        "fica no x do ponteiro: {menu:?}"
    );
}

#[test]
fn menu_maior_que_a_janela_ocupa_a_altura_toda_e_rola() {
    let mut app = TestApp::new(tela).size(600, 260);
    app.context_menu("alvo");
    let menu = app.rect("menu");
    assert!(menu.y0 >= 0.0 && menu.y1 <= 260.0, "{menu:?}");
    assert!(menu.height() > 200.0, "usa a janela toda: {menu:?}");
}
