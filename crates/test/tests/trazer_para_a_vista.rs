//! `start_bring_into_view`: o app escolhe uma linha de um `TreeView` comprido pelo código e pede
//! para a árvore rolar até ela, sem tirar o foco de onde está (o `StartBringIntoView` do WinUI).

use tomni_ui::prelude::*;
use tomni_ui::start_bring_into_view;
use tomni_ui_test::TestApp;

const LINHAS: usize = 200;

fn tela() -> Form {
    let itens = signal(
        (0..LINHAS)
            .map(|i| TreeItem::leaf(format!("l{i}"), format!("Linha {i}")))
            .collect::<Vec<_>>(),
    );
    let selecionado = signal(None::<String>);
    Form::new("Árvore").child(
        Column::new()
            .gap(8)
            .child(Button::new("Ir para a 150").id("ir").on_click(move || {
                selecionado.set(Some("l150".to_owned()));
                start_bring_into_view("arvore-l150");
            }))
            .child(TreeView::bind(itens, selecionado).id("arvore").grow(1)),
    )
}

#[test]
fn a_linha_escolhida_pelo_codigo_aparece_sem_levar_o_foco() {
    let mut app = TestApp::new(tela).size(400, 300);
    app.click("ir");
    app.frame();
    let arvore = app.rect("arvore");
    let linha = app.rect("arvore-l150");
    assert!(
        linha.y0 >= arvore.y0 && linha.y1 <= arvore.y1,
        "a linha {linha:?} aparece inteira na árvore {arvore:?}"
    );
    assert_eq!(app.focused().as_deref(), Some("ir"));
    let escolhida: Vec<_> = app
        .tree_items("arvore")
        .into_iter()
        .filter(|item| item.selected)
        .map(|item| item.key)
        .collect();
    assert_eq!(escolhida, ["l150"]);
}

#[test]
fn id_que_nao_existe_nao_rola_nada() {
    let mut app = TestApp::new(tela).size(400, 300);
    let antes = app.rect("arvore-l0");
    start_bring_into_view("nao-existe");
    for _ in 0..12 {
        app.frame();
    }
    assert_eq!(app.rect("arvore-l0"), antes);
}
