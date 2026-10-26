//! Marco M39 (fatia da lib): linhas de um `DataGrid` soltas sobre as pastas de um `TreeView`
//! (`on_grid_drop` e `can_grid_drop`), sem mudar o arrasto entre as linhas da própria árvore.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;
use std::time::Duration;

use tomni_ui::kurbo::Point;
use tomni_ui::prelude::*;
use tomni_ui_test::TestApp;

/// O que os callbacks viram: `grid:origem:chaves:destino` e `move:chave:pai:posição`.
type Log = Rc<RefCell<Vec<String>>>;

struct Cena {
    app: TestApp,
    log: Log,
    aberto: Signal<BTreeSet<String>>,
}

fn arquivo(chave: &str) -> DataGridRow {
    DataGridRow::new(chave).cell(chave, CellValue::Text(chave.to_owned()))
}

/// Uma árvore (`arvore`) e dois grids (`grid`, do grupo `cofre`, e `outro`, de outro grupo).
fn cena(grupo: &'static str, pode: bool) -> Cena {
    let log: Log = Rc::default();
    let guardado = Rc::new(RefCell::new(None));
    let (g, l) = (Rc::clone(&guardado), Rc::clone(&log));
    let app = TestApp::new(move || {
        let itens = signal(vec![
            TreeItem::folder("pasta", "Pasta"),
            TreeItem::folder("cheia", "Cheia").child(TreeItem::leaf("filha", "Filha")),
            TreeItem::leaf("folha", "Folha"),
        ]);
        let selecionado = signal(None::<String>);
        let aberto = signal(BTreeSet::new());
        *g.borrow_mut() = Some(aberto);
        let linhas = signal(vec![arquivo("a"), arquivo("b"), arquivo("c")]);
        let escolhidas = signal(BTreeSet::new());
        let outras = signal(vec![arquivo("x")]);
        let escolhidas_outras = signal(BTreeSet::new());
        let (l1, l2) = (Rc::clone(&l), Rc::clone(&l));
        Form::new("Cofre").child(
            Row::new()
                .grow(1)
                .gap(8)
                .child(
                    TreeView::bind(itens, selecionado)
                        .id("arvore")
                        .grow(1)
                        .expanded(aberto)
                        .on_move(move |chave, pai, posicao| {
                            l1.borrow_mut().push(format!(
                                "move:{chave}:{}:{posicao}",
                                pai.as_deref().unwrap_or("-")
                            ));
                        })
                        .on_grid_drop(grupo, move |soltou| {
                            l2.borrow_mut().push(format!(
                                "grid:{}:{}:{}",
                                soltou.source_grid,
                                soltou.keys.join(","),
                                soltou.target.as_deref().unwrap_or("-")
                            ));
                        })
                        .can_grid_drop(move |_| pode),
                )
                .child(
                    Column::new()
                        .grow(1)
                        .gap(8)
                        .child(
                            DataGrid::bind(linhas, escolhidas)
                                .id("grid")
                                .grow(1)
                                .columns(vec![DataGridColumn::new("nome", "Nome")])
                                .drag_group("cofre"),
                        )
                        .child(
                            DataGrid::bind(outras, escolhidas_outras)
                                .id("outro")
                                .grow(1)
                                .columns(vec![DataGridColumn::new("nome", "Nome")])
                                .drag_group("outro"),
                        ),
                ),
        )
    })
    .size(640, 400);
    let aberto = guardado.borrow().expect("a tela foi montada");
    Cena { app, log, aberto }
}

fn log(cena: &Cena) -> Vec<String> {
    cena.log.borrow().clone()
}

#[test]
fn soltar_linhas_do_grid_numa_pasta_chama_o_callback() {
    let mut cena = cena("cofre", true);
    cena.app
        .drag_rows("grid", &["a", "b"], "arvore", Some("pasta"));
    assert_eq!(log(&cena), ["grid:grid:a,b:pasta"]);
}

#[test]
fn soltar_numa_folha_nao_chama() {
    let mut cena = cena("cofre", true);
    cena.app.drag_rows("grid", &["a"], "arvore", Some("folha"));
    assert!(log(&cena).is_empty());
}

#[test]
fn can_grid_drop_que_recusa_nao_chama() {
    let mut cena = cena("cofre", false);
    cena.app.drag_rows("grid", &["a"], "arvore", Some("pasta"));
    assert!(log(&cena).is_empty());
}

#[test]
fn grid_de_outro_grupo_nao_e_aceito() {
    let mut cena = cena("cofre", true);
    cena.app.drag_rows("outro", &["x"], "arvore", Some("pasta"));
    assert!(log(&cena).is_empty());
}

#[test]
fn pasta_fechada_com_filhos_abre_depois_do_atraso() {
    let mut cena = cena("cofre", true);
    cena.app.grid_select("grid", &["a"]);
    let origem = cena.app.rect("grid-a").center();
    let alvo = cena.app.rect("arvore-cheia").center();
    cena.app.press("grid-a");
    cena.app.move_to(origem + (0.0, 8.0));
    cena.app.move_to(alvo);
    assert!(!cena.aberto.get_untracked().contains("cheia"));
    cena.app.wait(Duration::from_millis(1100));
    assert!(cena.aberto.get_untracked().contains("cheia"));
    cena.app.release();
    assert_eq!(log(&cena), ["grid:grid:a:cheia"]);
}

#[test]
fn o_arrasto_da_propria_arvore_continua_igual() {
    let mut cena = cena("cofre", true);
    let origem = cena.app.rect("arvore-folha").center();
    let alvo: Point = cena.app.rect("arvore-pasta").center();
    cena.app.press("arvore-folha");
    cena.app.move_to(origem + (0.0, -8.0));
    cena.app.move_to(alvo);
    cena.app.release();
    assert_eq!(log(&cena), ["move:folha:pasta:0"]);
}
