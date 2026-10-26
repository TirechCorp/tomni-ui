//! Marco M21: o `TreeView`. Os snapshots (árvore, filtro, renomear e os dois arrastos, nos dois
//! temas e nas duas densidades), o teclado, o mouse, o filtro, renomear, apagar, o menu de
//! contexto, o arrasto, os `TreeItems` que preservam o estado, a virtualização e as ferramentas.

mod comum;

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;
use std::time::Duration;

use tomni_ui::kurbo::Point;
use tomni_ui::prelude::*;
use tomni_ui::theme::Theme;
use tomni_ui::{Look, Modifiers};
use tomni_ui_test::TestApp;

use comum::theme_name;

/// Tudo o que o teste mexe por fora da árvore.
#[derive(Clone)]
struct Estado {
    itens: Signal<Vec<TreeItem>>,
    selecionado: Signal<Option<String>>,
    aberto: Signal<BTreeSet<String>>,
    filtro: Signal<String>,
    renomeando: Signal<Option<String>>,
    /// O que os callbacks viram, em ordem: `invoke:chave`, `rename:chave:texto`,
    /// `delete:chave`, `move:chave:pai:posição`.
    log: Rc<RefCell<Vec<String>>>,
}

type Guardado = Rc<RefCell<Option<Estado>>>;

/// Favoritos (aberta) → Trabalho (aberta) → Servidores (fechada) → web-01, db-01; Wiki;
/// Notícias; Ações (fechada) → Bolsa; Leitura.
fn itens() -> Vec<TreeItem> {
    vec![
        TreeItem::folder("favoritos", "Favoritos")
            .child(
                TreeItem::folder("trabalho", "Trabalho")
                    .child(
                        TreeItem::folder("servidores", "Servidores")
                            .child(TreeItem::leaf("web-01", "web-01"))
                            .child(TreeItem::leaf("db-01", "db-01")),
                    )
                    .child(TreeItem::leaf("wiki", "Wiki")),
            )
            .child(TreeItem::leaf("noticias", "Notícias")),
        TreeItem::folder("acoes", "Ações").child(TreeItem::leaf("bolsa", "Bolsa")),
        TreeItem::leaf("leitura", "Leitura"),
    ]
}

fn tela(guardado: &Guardado, densidade: Density, pode_soltar: bool) -> Form {
    let itens = signal(itens());
    let selecionado = signal(None::<String>);
    let aberto = signal(BTreeSet::from([
        "favoritos".to_owned(),
        "trabalho".to_owned(),
    ]));
    let filtro = signal(String::new());
    let renomeando = signal(None::<String>);
    let log = Rc::new(RefCell::new(Vec::new()));
    *guardado.borrow_mut() = Some(Estado {
        itens,
        selecionado,
        aberto,
        filtro,
        renomeando,
        log: Rc::clone(&log),
    });
    let (l1, l2, l3, l4) = (
        Rc::clone(&log),
        Rc::clone(&log),
        Rc::clone(&log),
        Rc::clone(&log),
    );
    Form::new("Favoritos").child(
        TreeView::bind(itens, selecionado)
            .id("arv")
            .grow(1)
            .density(densidade)
            .expanded(aberto)
            .filter(filtro)
            .renaming(renomeando)
            .on_invoke(move |chave| l1.borrow_mut().push(format!("invoke:{chave}")))
            .on_rename(move |chave, texto| {
                l2.borrow_mut().push(format!("rename:{chave}:{texto}"));
                renomeando.set(None);
            })
            .on_delete(move |chave| l3.borrow_mut().push(format!("delete:{chave}")))
            .on_move(move |chave, pai, posicao| {
                l4.borrow_mut().push(format!(
                    "move:{chave}:{}:{posicao}",
                    pai.as_deref().unwrap_or("-")
                ));
            })
            .can_drop(move |_, pai| pode_soltar || pai != Some("acoes"))
            .item_context_flyout(|chave| {
                MenuFlyout::new()
                    .id(format!("menu-{chave}"))
                    .item("abrir", "Abrir", || {})
            }),
    )
}

fn arvore() -> (TestApp, Estado) {
    arvore_com(Theme::Light, Density::Standard, true)
}

fn arvore_com(tema: Theme, densidade: Density, pode_soltar: bool) -> (TestApp, Estado) {
    let guardado: Guardado = Rc::default();
    let g = Rc::clone(&guardado);
    let app = TestApp::new(move || tela(&g, densidade, pode_soltar))
        .size(320, 400)
        .theme(tema);
    let estado = guardado.borrow().clone().expect("a tela foi montada");
    (app, estado)
}

fn chaves(app: &TestApp) -> Vec<String> {
    app.tree_items("arv").into_iter().map(|l| l.key).collect()
}

fn log(estado: &Estado) -> Vec<String> {
    estado.log.borrow().clone()
}

fn id(chave: &str) -> String {
    format!("arv-{chave}")
}

// ---------------------------------------------------------------------------------------
// Snapshots

fn cena_arvore(app: &mut TestApp) {
    app.click(&id("wiki"));
    app.key(Key::Up).key(Key::Down);
    app.hover(&id("leitura"));
}

fn cena_filtro(app: &mut TestApp, estado: &Estado) {
    estado.filtro.set("acoes".to_owned());
    app.frame();
}

fn cena_renomear(app: &mut TestApp, estado: &Estado) {
    estado.selecionado.set(Some("wiki".to_owned()));
    estado.renomeando.set(Some("wiki".to_owned()));
    app.frame();
    app.frame();
}

fn cena_arrasto_linha(app: &mut TestApp) {
    let wiki = app.rect(&id("wiki")).center();
    let alvo = app.rect(&id("noticias"));
    app.press(&id("wiki"));
    app.move_to(wiki + (0.0, 8.0));
    app.move_to(Point::new(alvo.center().x, alvo.y0 + 4.0));
}

fn cena_arrasto_pasta(app: &mut TestApp) {
    let wiki = app.rect(&id("wiki")).center();
    let alvo = app.rect(&id("acoes")).center();
    app.press(&id("wiki"));
    app.move_to(wiki + (0.0, 8.0));
    app.move_to(alvo);
}

#[test]
fn snapshots() {
    for tema in [Theme::Light, Theme::Dark] {
        for (densidade, nome_densidade) in [
            (Density::Standard, "padrao"),
            (Density::Compact, "compacta"),
        ] {
            let nome = |cena: &str| format!("m21-{cena}-{}-{nome_densidade}", theme_name(tema));
            let (mut app, _) = arvore_com(tema, densidade, true);
            cena_arvore(&mut app);
            app.assert_snapshot(&nome("arvore"));
            let (mut app, estado) = arvore_com(tema, densidade, true);
            cena_filtro(&mut app, &estado);
            app.assert_snapshot(&nome("filtro"));
            let (mut app, estado) = arvore_com(tema, densidade, true);
            cena_renomear(&mut app, &estado);
            app.assert_snapshot(&nome("renomear"));
            let (mut app, _) = arvore_com(tema, densidade, true);
            cena_arrasto_linha(&mut app);
            app.assert_snapshot(&nome("arrasto-linha"));
            let (mut app, _) = arvore_com(tema, densidade, true);
            cena_arrasto_pasta(&mut app);
            app.assert_snapshot(&nome("arrasto-pasta"));
        }
    }
}

// ---------------------------------------------------------------------------------------
// Estrutura

#[test]
fn linhas_visiveis_seguem_as_pastas_abertas() {
    let (mut app, estado) = arvore();
    assert_eq!(
        chaves(&app),
        [
            "favoritos",
            "trabalho",
            "servidores",
            "wiki",
            "noticias",
            "acoes",
            "leitura"
        ]
    );
    let linhas = app.tree_items("arv");
    assert_eq!(linhas[3].depth, 2);
    assert!(linhas[0].expanded && !linhas[2].expanded);
    app.expand("arv", "servidores");
    assert_eq!(chaves(&app)[3], "web-01");
    app.collapse("arv", "favoritos");
    assert_eq!(chaves(&app), ["favoritos", "acoes", "leitura"]);
    assert!(estado.aberto.get_untracked().contains("servidores"));
    app.assert_lint();
}

#[test]
fn a_linha_tem_44_px_no_padrao_e_24_no_compacto() {
    let (app, _) = arvore();
    assert_eq!(app.rect(&id("wiki")).height(), 44.0);
    let (app, _) = arvore_com(Theme::Light, Density::Compact, true);
    assert_eq!(app.rect(&id("wiki")).height(), 24.0);
}

#[test]
fn a_ferramenta_mostra_as_linhas() {
    let (mut app, estado) = arvore();
    app.click(&id("wiki"));
    let no = app.tree();
    let arvore = no.find("arv").expect("o TreeView");
    assert_eq!(arvore.kind, "tree_view");
    assert_eq!(arvore.items_total, Some(7));
    let itens = arvore.items.as_ref().unwrap();
    assert_eq!(itens[3].key, "wiki");
    assert!(itens[3].selected && itens[0].expanded);
    let linha = no.find(&id("wiki")).expect("a linha");
    assert_eq!(linha.kind, "tree_view_item");
    assert_eq!(linha.selected, Some(true));
    assert_eq!(linha.depth, Some(2));
    assert_eq!(arvore.filter, None);
    estado.filtro.set("wi".to_owned());
    app.frame();
    assert_eq!(
        app.tree().find("arv").unwrap().filter.as_deref(),
        Some("wi")
    );
}

// ---------------------------------------------------------------------------------------
// Mouse

#[test]
fn clique_seleciona_e_o_duplo_clique_invoca() {
    let (mut app, estado) = arvore();
    app.click(&id("wiki"));
    assert_eq!(estado.selecionado.get_untracked().as_deref(), Some("wiki"));
    assert!(log(&estado).is_empty());
    app.double_click(&id("wiki"));
    assert_eq!(log(&estado), ["invoke:wiki"]);
    // Na pasta, o duplo clique também alterna.
    app.double_click(&id("trabalho"));
    assert_eq!(log(&estado), ["invoke:wiki", "invoke:trabalho"]);
    assert!(!chaves(&app).contains(&"wiki".to_owned()));
}

#[test]
fn clique_no_chevron_alterna_sem_selecionar() {
    let (mut app, estado) = arvore();
    let linha = app.rect(&id("servidores"));
    // A coluna do chevron: depois do recuo de 2 níveis.
    let chevron = Point::new(linha.x0 + 4.0 + 32.0 + 20.0, linha.center().y);
    app.click_at(chevron, 1);
    assert!(chaves(&app).contains(&"web-01".to_owned()));
    assert_eq!(estado.selecionado.get_untracked(), None);
    app.click_at(chevron, 1);
    assert!(!chaves(&app).contains(&"web-01".to_owned()));
}

// ---------------------------------------------------------------------------------------
// Teclado

#[test]
fn setas_abrem_fecham_e_andam() {
    let (mut app, estado) = arvore();
    app.click(&id("servidores"));
    // → na fechada abre; → na aberta vai ao primeiro filho.
    app.key(Key::Right);
    assert!(chaves(&app).contains(&"web-01".to_owned()));
    assert_eq!(
        estado.selecionado.get_untracked().as_deref(),
        Some("servidores")
    );
    app.key(Key::Right);
    assert_eq!(
        estado.selecionado.get_untracked().as_deref(),
        Some("web-01")
    );
    // ← num filho vai ao pai; ← na aberta fecha.
    app.key(Key::Left);
    assert_eq!(
        estado.selecionado.get_untracked().as_deref(),
        Some("servidores")
    );
    app.key(Key::Left);
    assert!(!chaves(&app).contains(&"web-01".to_owned()));
    // Home e End.
    app.key(Key::End);
    assert_eq!(
        estado.selecionado.get_untracked().as_deref(),
        Some("leitura")
    );
    app.key(Key::Home);
    assert_eq!(
        estado.selecionado.get_untracked().as_deref(),
        Some("favoritos")
    );
    app.key(Key::Down).key(Key::Down);
    assert_eq!(
        estado.selecionado.get_untracked().as_deref(),
        Some("servidores")
    );
    app.key(Key::Up);
    assert_eq!(
        estado.selecionado.get_untracked().as_deref(),
        Some("trabalho")
    );
}

#[test]
fn a_letra_inicial_da_a_volta_e_ignora_acento() {
    let (mut app, estado) = arvore();
    app.click(&id("leitura"));
    app.type_text("a");
    assert_eq!(estado.selecionado.get_untracked().as_deref(), Some("acoes"));
    app.type_text("w");
    assert_eq!(estado.selecionado.get_untracked().as_deref(), Some("wiki"));
}

#[test]
fn enter_invoca_e_alterna_a_pasta() {
    let (mut app, estado) = arvore();
    app.click(&id("servidores")).key(Key::Enter);
    assert_eq!(log(&estado), ["invoke:servidores"]);
    assert!(chaves(&app).contains(&"web-01".to_owned()));
}

#[test]
fn tab_entra_na_selecionada_e_o_proximo_sai() {
    let mut app = TestApp::new(move || {
        let itens = signal(itens());
        let escolhido = signal(Some("wiki".to_owned()));
        let aberto = signal(BTreeSet::from([
            "favoritos".to_owned(),
            "trabalho".to_owned(),
        ]));
        Form::new("Favoritos").child(
            Column::new()
                .grow(1)
                .child(Button::new("Antes").id("antes"))
                .child(
                    TreeView::bind(itens, escolhido)
                        .id("arv")
                        .grow(1)
                        .expanded(aberto),
                )
                .child(Button::new("Depois").id("depois")),
        )
    })
    .size(320, 400);
    app.tab();
    assert_eq!(app.focused().as_deref(), Some("antes"));
    app.tab();
    assert_eq!(app.focused().as_deref(), Some("arv-wiki"));
    app.tab();
    assert_eq!(app.focused().as_deref(), Some("depois"));
    app.shift_tab();
    assert_eq!(app.focused().as_deref(), Some("arv-wiki"));
}

#[test]
fn sem_selecao_o_tab_cai_na_primeira_linha() {
    let (mut app, _) = arvore();
    app.tab();
    assert_eq!(app.focused().as_deref(), Some("arv-favoritos"));
}

// ---------------------------------------------------------------------------------------
// Filtro

#[test]
fn filtro_mantem_os_ancestrais_e_ao_limpar_volta_ao_de_antes() {
    let (mut app, estado) = arvore();
    estado.filtro.set("ACOES".to_owned());
    app.frame();
    assert_eq!(chaves(&app), ["acoes"]);
    estado.filtro.set("db".to_owned());
    app.frame();
    assert_eq!(
        chaves(&app),
        ["favoritos", "trabalho", "servidores", "db-01"]
    );
    // Fechar uma pasta durante o filtro não mexe no `expanded` do app.
    let antes = estado.aberto.get_untracked();
    app.collapse("arv", "trabalho");
    assert_eq!(chaves(&app), ["favoritos", "trabalho"]);
    assert_eq!(estado.aberto.get_untracked(), antes);
    estado.filtro.set(String::new());
    app.frame();
    assert_eq!(estado.aberto.get_untracked(), antes);
    assert_eq!(chaves(&app).len(), 7);
}

// ---------------------------------------------------------------------------------------
// Renomear, apagar e menu de contexto

#[test]
fn f2_texto_e_enter_renomeiam() {
    let (mut app, estado) = arvore();
    app.click(&id("wiki")).key(Key::F(2));
    assert_eq!(app.focused().as_deref(), Some("arv-wiki-nome"));
    app.shortcut('a').type_text("Docs").key(Key::Enter);
    assert_eq!(log(&estado), ["rename:wiki:Docs"]);
    assert_eq!(estado.renomeando.get_untracked(), None);
}

#[test]
fn esc_cancela_e_vazio_e_recusado() {
    let (mut app, estado) = arvore();
    app.click(&id("wiki")).key(Key::F(2));
    app.type_text("x").key(Key::Escape);
    assert!(log(&estado).is_empty());
    assert_eq!(estado.renomeando.get_untracked(), None);
    app.key(Key::F(2));
    app.shortcut('a').key(Key::Backspace).key(Key::Enter);
    assert!(log(&estado).is_empty(), "vazio é recusado");
    assert_eq!(estado.renomeando.get_untracked().as_deref(), Some("wiki"));
}

#[test]
fn perder_o_foco_confirma_e_o_app_pode_abrir_o_campo() {
    let (mut app, estado) = arvore();
    estado.renomeando.set(Some("noticias".to_owned()));
    app.frame();
    app.frame();
    assert_eq!(app.focused().as_deref(), Some("arv-noticias-nome"));
    app.shortcut('a').type_text("Jornal");
    app.click(&id("leitura"));
    assert_eq!(log(&estado), ["rename:noticias:Jornal"]);
}

#[test]
fn delete_chama_on_delete() {
    let (mut app, estado) = arvore();
    app.click(&id("leitura")).key(Key::Delete);
    assert_eq!(log(&estado), ["delete:leitura"]);
}

#[test]
fn clique_direito_seleciona_e_abre_o_menu() {
    let (mut app, estado) = arvore();
    app.right_click(&id("wiki"));
    assert_eq!(estado.selecionado.get_untracked().as_deref(), Some("wiki"));
    assert!(app.is_open("menu-wiki"));
    app.key(Key::Escape);
    app.context_menu(&id("noticias"));
    assert_eq!(
        estado.selecionado.get_untracked().as_deref(),
        Some("noticias")
    );
    assert!(app.is_open("menu-noticias"));
}

// ---------------------------------------------------------------------------------------
// Arrasto

fn arrastar_para(app: &mut TestApp, de: &str, ponto: Point) {
    let origem = app.rect(&id(de)).center();
    app.press(&id(de));
    app.move_to(origem + (0.0, 8.0));
    app.move_to(ponto);
    app.release();
}

#[test]
fn soltar_no_meio_da_pasta_move_para_dentro() {
    let (mut app, estado) = arvore();
    let alvo = app.rect(&id("acoes")).center();
    arrastar_para(&mut app, "wiki", alvo);
    assert_eq!(log(&estado), ["move:wiki:acoes:1"]);
}

#[test]
fn soltar_na_borda_de_cima_insere_antes_no_pai_da_linha() {
    let (mut app, estado) = arvore();
    let r = app.rect(&id("noticias"));
    arrastar_para(&mut app, "leitura", Point::new(r.center().x, r.y0 + 3.0));
    // Antes de Notícias, dentro de Favoritos (posição 1: depois de Trabalho).
    assert_eq!(log(&estado), ["move:leitura:favoritos:1"]);
}

#[test]
fn mesmo_pai_para_baixo_desconta_o_item_que_sai() {
    let (mut app, estado) = arvore();
    // Favoritos → [Trabalho, Notícias]; Notícias para antes de Trabalho.
    let r = app.rect(&id("trabalho"));
    arrastar_para(&mut app, "noticias", Point::new(r.center().x, r.y0 + 3.0));
    assert_eq!(log(&estado), ["move:noticias:favoritos:0"]);
    // Trabalho para depois de Notícias: a posição final é 1 (Notícias ficou em 0).
    let (mut app, estado) = arvore();
    let r = app.rect(&id("noticias"));
    arrastar_para(&mut app, "trabalho", Point::new(r.center().x, r.y1 - 3.0));
    assert_eq!(log(&estado), ["move:trabalho:favoritos:1"]);
}

#[test]
fn can_drop_barra_o_destino() {
    let (mut app, estado) = arvore_com(Theme::Light, Density::Standard, false);
    let alvo = app.rect(&id("acoes")).center();
    arrastar_para(&mut app, "wiki", alvo);
    assert!(log(&estado).is_empty());
}

#[test]
fn nao_solta_em_si_mesmo_nem_num_descendente() {
    let (mut app, estado) = arvore();
    let propria = app.rect(&id("trabalho")).center();
    arrastar_para(&mut app, "trabalho", propria);
    let filha = app.rect(&id("wiki")).center();
    arrastar_para(&mut app, "trabalho", filha);
    assert!(log(&estado).is_empty());
}

#[test]
fn pasta_fechada_abre_depois_de_um_segundo_parada() {
    let (mut app, estado) = arvore();
    let alvo = app.rect(&id("acoes")).center();
    let origem = app.rect(&id("wiki")).center();
    app.press(&id("wiki"));
    app.move_to(origem + (0.0, 8.0));
    app.move_to(alvo);
    assert!(!chaves(&app).contains(&"bolsa".to_owned()));
    app.wait(Duration::from_millis(1100));
    assert!(chaves(&app).contains(&"bolsa".to_owned()));
    app.release();
    assert!(estado.aberto.get_untracked().contains("acoes"));
}

#[test]
fn esc_cancela_o_arrasto() {
    let (mut app, estado) = arvore();
    let alvo = app.rect(&id("acoes")).center();
    let origem = app.rect(&id("wiki")).center();
    app.press(&id("wiki"));
    app.move_to(origem + (0.0, 8.0));
    app.move_to(alvo);
    app.key(Key::Escape);
    app.release();
    assert!(log(&estado).is_empty());
}

// ---------------------------------------------------------------------------------------
// TreeItems preservam o estado

#[test]
fn inserir_mover_subir_descer_e_remover_mantem_pastas_e_selecao() {
    let (mut app, estado) = arvore();
    app.click(&id("wiki"));
    let abertos = estado.aberto.get_untracked();
    estado.itens.update(|v| {
        assert!(v.insert_item(Some("favoritos"), 0, TreeItem::leaf("novo", "Novo")));
    });
    app.frame();
    assert_eq!(chaves(&app)[1], "novo");
    estado.itens.update(|v| {
        assert!(v.move_to("novo", Some("trabalho"), 0));
        assert!(v.move_down("novo"));
        assert!(v.move_up("novo"));
    });
    app.frame();
    assert_eq!(estado.selecionado.get_untracked().as_deref(), Some("wiki"));
    assert_eq!(estado.aberto.get_untracked(), abertos);
    estado.itens.update(|v| {
        v.remove_item("novo");
    });
    app.frame();
    assert!(
        app.tree_items("arv")
            .iter()
            .any(|l| l.key == "wiki" && l.selected)
    );
    assert_eq!(app.text("arv"), "wiki");
}

#[test]
fn selecao_que_some_vale_nenhuma_sem_escrever_no_signal() {
    let (mut app, estado) = arvore();
    app.click(&id("wiki"));
    estado.itens.update(|v| {
        v.remove_item("wiki");
    });
    app.frame();
    assert_eq!(app.tree().find("arv").unwrap().text, None);
    assert_eq!(estado.selecionado.get_untracked().as_deref(), Some("wiki"));
    app.tab();
    assert_eq!(app.focused().as_deref(), Some("arv-favoritos"));
}

// ---------------------------------------------------------------------------------------
// Virtualização

#[test]
fn dez_mil_filhos_ficam_leves() {
    let filhos: Vec<TreeItem> = (0..10_000)
        .map(|i| TreeItem::leaf(format!("s{i}"), format!("servidor-{i:05}")))
        .collect();
    let guardado: Rc<RefCell<Option<Signal<Option<String>>>>> = Rc::default();
    let g = Rc::clone(&guardado);
    let mut app = TestApp::new(move || {
        let itens = signal(vec![
            TreeItem::folder("todos", "Todos").children(filhos.clone()),
        ]);
        let selecionado = signal(None::<String>);
        *g.borrow_mut() = Some(selecionado);
        Form::new("Muitos").child(TreeView::bind(itens, selecionado).id("arv").grow(1))
    })
    .size(320, 400);
    let selecionado = guardado.borrow().unwrap();
    app.expand("arv", "todos");
    assert_eq!(app.tree_items("arv").len(), 10_001);
    let montadas = |app: &TestApp| {
        let mut n = 0;
        let mut pilha = vec![app.tree()];
        while let Some(no) = pilha.pop() {
            n += usize::from(no.kind == "tree_view_item");
            pilha.extend(no.children);
        }
        n
    };
    assert!(montadas(&app) < 200, "montadas: {}", montadas(&app));
    let mut pior = Duration::ZERO;
    let mut medir = |app: &mut TestApp, f: &dyn Fn(&mut TestApp)| {
        let t = std::time::Instant::now();
        f(app);
        pior = pior.max(t.elapsed());
    };
    app.click(&id("todos"));
    medir(&mut app, &|app| {
        app.key(Key::End);
    });
    assert_eq!(selecionado.get_untracked().as_deref(), Some("s9999"));
    for _ in 0..50 {
        medir(&mut app, &|app| {
            app.wheel(
                Point::new(100.0, 100.0),
                tomni_ui::kurbo::Vec2::new(0.0, -2000.0),
            );
        });
    }
    medir(&mut app, &|app| {
        app.key(Key::Home);
    });
    assert_eq!(selecionado.get_untracked().as_deref(), Some("todos"));
    assert!(montadas(&app) < 200);
    assert!(pior < Duration::from_millis(50), "pior frame: {pior:?}");
}

#[test]
fn a_linha_fixada_segue_montada_fora_da_janela() {
    let (mut app, estado) = arvore();
    estado.selecionado.set(Some("wiki".to_owned()));
    app.frame();
    let _ = &mut app;
    assert!(matches!(app.look(&id("wiki")), Look::TreeViewItem { .. }));
    let _ = Modifiers::NONE;
}

#[test]
fn o_selo_do_item_aparece_depois_do_rotulo() {
    let mut app = TestApp::new(|| {
        let itens = signal(vec![
            TreeItem::leaf("com", "Com selo").badge(Symbol::Sparkle),
            TreeItem::leaf("sem", "Sem selo"),
        ]);
        let selecionado = signal(None::<String>);
        Form::new("Selo").child(TreeView::bind(itens, selecionado).id("arv").grow(1))
    })
    .size(320, 200);
    app.frame();
    assert!(matches!(
        app.look(&id("com")),
        Look::TreeViewItem {
            badge: Some(Symbol::Sparkle),
            ..
        }
    ));
    assert!(matches!(
        app.look(&id("sem")),
        Look::TreeViewItem { badge: None, .. }
    ));
}
