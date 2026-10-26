//! Marco M20: o divisor (`Split`), a barra de menus, os menus de contexto e o arrastar e
//! soltar. Os mínimos dos dois lados, as setas, o tamanho no signal, a janela redimensionada,
//! a grade 2×2 de divisores aninhados e o cursor.

mod comum;

use std::cell::Cell;
use std::rc::Rc;

use tomni_ui::kurbo::{Point, Rect};
use tomni_ui::prelude::*;
use tomni_ui::theme::Theme;
use tomni_ui::{
    ButtonState, Cursor, HOLD_DELAY, InsertMark, Key, Look, PointerKind, SPLIT_KEY_STEP,
    SPLIT_SIZER,
};
use tomni_ui_test::TestApp;

/// Um `Split` lado a lado com os painéis "esq" e "dir", o tamanho em `largura`.
fn divisor(largura: Signal<f64>) -> Form {
    Form::new("Divisor").child(
        Split::row(
            Text::new("Arquivos").id("esq"),
            Text::new("Conteúdo").id("dir"),
        )
        .id("div")
        .size(largura)
        .min_first(100.0)
        .min_second(200.0),
    )
}

fn centro(r: Rect) -> Point {
    r.center()
}

#[test]
fn divisor_comeca_no_tamanho_e_as_partes_ficam_encostadas() {
    let mut app = TestApp::new(|| {
        Form::new("Divisor").child(Split::row(Text::new("A"), Text::new("B")).id("div"))
    })
    .size(800, 400);
    let primeiro = app.rect("div-primeiro");
    let sizer = app.rect("div-divisor");
    let segundo = app.rect("div-segundo");
    let todo = app.rect("div");
    assert_eq!(primeiro.width(), 240.0);
    assert_eq!(primeiro.x0, todo.x0);
    assert_eq!(sizer.x0, primeiro.x1);
    assert_eq!(sizer.width(), SPLIT_SIZER);
    assert_eq!(segundo.x0, sizer.x1);
    assert_eq!(segundo.x1, todo.x1);
    assert_eq!(app.value("div"), 240.0);
    assert_eq!(app.ui().cursor(centro(sizer)), Cursor::EwResize);
    assert_eq!(app.ui().cursor(centro(primeiro)), Cursor::Default);
}

#[test]
fn arrastar_o_divisor_muda_o_signal_e_respeita_os_minimos() {
    let largura = Rc::new(Cell::new(None));
    let guarda = Rc::clone(&largura);
    let mut app = TestApp::new(move || {
        let s = signal(300.0);
        guarda.set(Some(s));
        divisor(s)
    })
    .size(800, 400);
    let largura = largura.get().unwrap();
    let inicio = app.rect("esq").x0;
    let sizer = centro(app.rect("div-divisor"));

    app.drag(sizer, sizer + (50.0, 0.0));
    assert_eq!(largura.get_untracked(), 350.0);
    assert_eq!(app.rect("div-primeiro").width(), 350.0);
    assert_eq!(app.rect("esq").x0, inicio);

    // Para a esquerda, para no mínimo do primeiro.
    let sizer = centro(app.rect("div-divisor"));
    app.drag(sizer, Point::new(0.0, sizer.y));
    assert_eq!(largura.get_untracked(), 100.0);

    // Para a direita, para no mínimo do segundo.
    let sizer = centro(app.rect("div-divisor"));
    app.drag(sizer, Point::new(799.0, sizer.y));
    let todo = app.rect("div");
    assert_eq!(largura.get_untracked(), todo.width() - SPLIT_SIZER - 200.0);
    assert_eq!(app.rect("div-segundo").width(), 200.0);

    // O signal mudado pelo app move o divisor.
    largura.set(260.0);
    app.frame();
    assert_eq!(app.rect("div-primeiro").width(), 260.0);
    assert_eq!(app.rect("div-divisor").x0, todo.x0 + 260.0);
}

#[test]
fn setas_movem_o_divisor_de_8_px() {
    let mut app = TestApp::new(|| divisor(signal(240.0))).size(800, 400);
    app.tab();
    assert_eq!(app.focused().as_deref(), Some("div-divisor"));
    assert!(matches!(app.look("div-divisor"), Look::Sizer(v) if v.focus_ring));
    app.key(Key::Right);
    assert_eq!(app.value("div"), 240.0 + SPLIT_KEY_STEP);
    app.key(Key::Left).key(Key::Left);
    assert_eq!(app.value("div"), 240.0 - SPLIT_KEY_STEP);
    // ↑ e ↓ não mexem no divisor lado a lado.
    app.key(Key::Up);
    assert_eq!(app.value("div"), 240.0 - SPLIT_KEY_STEP);
    for _ in 0..30 {
        app.key(Key::Left);
    }
    assert_eq!(app.value("div"), 100.0);
}

#[test]
fn janela_redimensionada_mantem_o_primeiro_painel() {
    let mut app = TestApp::new(|| divisor(signal(300.0))).size(800, 400);
    assert_eq!(app.rect("div-primeiro").width(), 300.0);
    app.resize(1200, 400);
    assert_eq!(app.rect("div-primeiro").width(), 300.0);
    assert_eq!(app.rect("div-segundo").x1, app.rect("div").x1);
    app.resize(640, 400);
    assert_eq!(app.rect("div-primeiro").width(), 300.0);
    // Pequena demais: o segundo fica no mínimo e o primeiro encolhe.
    app.resize(480, 400);
    assert_eq!(app.rect("div-segundo").width(), 200.0);
    assert!(app.rect("div-primeiro").width() < 300.0);
    assert!(app.rect("div-primeiro").width() >= 100.0);
}

#[test]
fn divisor_em_coluna_e_grade_2x2() {
    let mut app = TestApp::new(|| {
        Form::new("Grade").child(
            Split::row(
                Split::column(Text::new("1").id("a"), Text::new("2").id("b"))
                    .id("esq")
                    .size(signal(120.0)),
                Split::column(Text::new("3").id("c"), Text::new("4").id("d"))
                    .id("dir")
                    .size(signal(80.0)),
            )
            .id("grade")
            .size(signal(200.0)),
        )
    })
    .size(800, 600);
    let esq = app.rect("esq");
    let dir = app.rect("dir");
    assert_eq!(esq.width(), 200.0);
    assert_eq!(dir.x0, esq.x1 + SPLIT_SIZER);
    assert_eq!(app.rect("esq-primeiro").height(), 120.0);
    assert_eq!(app.rect("dir-primeiro").height(), 80.0);
    assert_eq!(app.rect("esq-divisor").height(), SPLIT_SIZER);
    assert_eq!(app.rect("esq-divisor").width(), esq.width());
    assert_eq!(app.rect("esq-segundo").y1, esq.y1);
    assert_eq!(esq.height(), dir.height());
    let sizer = centro(app.rect("dir-divisor"));
    assert_eq!(app.ui().cursor(sizer), Cursor::NsResize);
    app.drag(sizer, sizer + (0.0, 40.0));
    assert_eq!(app.value("dir"), 120.0);
    assert_eq!(app.value("esq"), 120.0);
    // No divisor deitado, ↓ e ↑ movem.
    app.click("esq-divisor");
    app.key(Key::Down);
    assert_eq!(app.value("esq"), 128.0);
}

#[test]
fn hover_e_arrasto_no_visual_do_divisor() {
    let mut app = TestApp::new(|| divisor(signal(240.0))).size(800, 400);
    let estado = |app: &TestApp| match app.look("div-divisor") {
        Look::Sizer(v) => v.state,
        look => panic!("{look:?}"),
    };
    assert_eq!(estado(&app), ButtonState::Rest);
    app.hover("div-divisor");
    assert_eq!(estado(&app), ButtonState::Hover);
    app.press("div-divisor");
    assert_eq!(estado(&app), ButtonState::Pressed);
    app.release();
    assert_eq!(estado(&app), ButtonState::Hover);
    assert!(app.lint().is_empty(), "{:?}", app.lint());
}

/// Um `TabView` com as abas "app", "erro" e "acesso", que reordena pelo `on_reorder` e anota
/// cada chamada em `chamadas`.
fn abas(chamadas: Rc<std::cell::RefCell<Vec<(usize, usize)>>>) -> Form {
    let logs = signal(vec![
        "app".to_string(),
        "erro".to_string(),
        "acesso".to_string(),
    ]);
    let aberto = signal("app".to_string());
    Form::new("Logs").child(
        TabView::bind(aberto)
            .id("logs")
            .items(logs, |l| l.clone(), |l| l.clone(), Text::new)
            .tab_context_flyout(|chave| {
                MenuFlyout::new().id(format!("menu-{chave}")).item(
                    format!("fechar-{chave}"),
                    "Fechar",
                    || {},
                )
            })
            .on_reorder(move |de, para| {
                chamadas.borrow_mut().push((de, para));
                logs.update(|v| {
                    let l = v.remove(de);
                    v.insert(para, l);
                });
            }),
    )
}

type Chamadas = Rc<std::cell::RefCell<Vec<(usize, usize)>>>;

fn app_abas() -> (TestApp, Chamadas) {
    let chamadas = Rc::new(std::cell::RefCell::new(Vec::new()));
    let c = Rc::clone(&chamadas);
    let app = TestApp::new(move || abas(c)).size(600, 300);
    (app, chamadas)
}

fn escolhida(app: &TestApp, id: &str) -> bool {
    match app.look(id) {
        Look::TabHeader { visual, .. } => visual.selected,
        look => panic!("{id:?} não é uma aba: {look:?}"),
    }
}

fn marca(app: &TestApp, id: &str) -> Option<InsertMark> {
    match app.look(id) {
        Look::TabHeader { visual, .. } => visual.insert,
        look => panic!("{id:?} não é uma aba: {look:?}"),
    }
}

/// Ponto na metade da esquerda (`lado` < 0.5) ou da direita da aba `id`.
fn na_aba(app: &TestApp, id: &str, lado: f64) -> Point {
    let r = app.rect(id);
    Point::new(r.x0 + r.width() * lado, r.center().y)
}

#[test]
fn arrasto_so_comeca_depois_de_4_px_e_o_clique_continua() {
    let (mut app, chamadas) = app_abas();
    let erro = centro(app.rect("logs-erro"));
    app.press("logs-erro").move_to(erro + (3.0, 0.0));
    assert_eq!(app.drag_label(), None, "3 px ainda é um clique");
    app.release();
    assert!(escolhida(&app, "logs-erro"), "o clique escolheu a aba");
    assert!(chamadas.borrow().is_empty());

    let app_ = centro(app.rect("logs-app"));
    app.press("logs-app").move_to(app_ + (5.0, 0.0));
    assert_eq!(app.drag_label().as_deref(), Some("app"));
    // A etiqueta segue o ponteiro, com o visual do `Flyout`.
    let (_, ponto) = app.ui().tree().drag_label().unwrap();
    assert_eq!(ponto, app_ + (5.0, 0.0));
    app.release();
    assert_eq!(app.drag_label(), None);
    assert!(
        escolhida(&app, "logs-erro"),
        "o arrasto não é um clique: a aba escolhida não muda"
    );
    assert!(chamadas.borrow().is_empty(), "soltou no mesmo lugar");
}

#[test]
fn reordenar_passa_os_indices_certos() {
    let (mut app, chamadas) = app_abas();
    let destino = na_aba(&app, "logs-acesso", 0.75);
    app.drag("logs-app", destino);
    assert_eq!(*chamadas.borrow(), [(0, 2)]);
    assert_eq!(app.tab_titles("logs"), ["erro", "acesso", "app"]);

    let destino = na_aba(&app, "logs-erro", 0.25);
    app.drag("logs-app", destino);
    assert_eq!(chamadas.borrow()[1], (2, 0));
    assert_eq!(app.tab_titles("logs"), ["app", "erro", "acesso"]);

    // Para o vão de um dos lados da própria aba: nada muda.
    let destino = na_aba(&app, "logs-acesso", 0.25);
    app.drag("logs-erro", destino);
    let destino = na_aba(&app, "logs-app", 0.75);
    app.drag("logs-erro", destino);
    assert_eq!(chamadas.borrow().len(), 2);
    assert_eq!(app.tab_titles("logs"), ["app", "erro", "acesso"]);
}

#[test]
fn a_linha_mostra_onde_a_aba_vai_cair() {
    let (mut app, _) = app_abas();
    let erro = na_aba(&app, "logs-erro", 0.25);
    app.press("logs-acesso").move_to(erro);
    assert_eq!(marca(&app, "logs-erro"), Some(InsertMark::Before));
    assert_eq!(marca(&app, "logs-app"), None);
    let fim = na_aba(&app, "logs-acesso", 0.75);
    app.move_to(fim);
    assert_eq!(marca(&app, "logs-erro"), None);
    assert_eq!(marca(&app, "logs-acesso"), Some(InsertMark::After));
    // Fora da faixa, a marca some.
    let fora = Point::new(fim.x, 250.0);
    app.move_to(fora);
    assert_eq!(marca(&app, "logs-acesso"), None);
    assert_eq!(app.drag_label().as_deref(), Some("acesso"));
    app.release();
}

#[test]
fn esc_cancela_e_soltar_fora_nao_faz_nada() {
    let (mut app, chamadas) = app_abas();
    let destino = na_aba(&app, "logs-acesso", 0.75);
    app.press("logs-app").move_to(destino);
    assert_eq!(marca(&app, "logs-acesso"), Some(InsertMark::After));
    app.key(Key::Escape);
    assert_eq!(app.drag_label(), None);
    assert_eq!(marca(&app, "logs-acesso"), None);
    app.release();
    assert!(chamadas.borrow().is_empty(), "o Esc cancelou");
    assert!(escolhida(&app, "logs-app"));

    app.drag("logs-app", Point::new(300.0, 250.0));
    assert!(chamadas.borrow().is_empty(), "soltou fora da faixa");
    app.drag("logs-app", Point::new(700.0, 20.0));
    assert!(chamadas.borrow().is_empty(), "soltou fora da janela");
    assert_eq!(app.tab_titles("logs"), ["app", "erro", "acesso"]);
    assert_eq!(app.drag_label(), None);
}

#[test]
fn no_toque_o_arrasto_comeca_ao_segurar() {
    let (mut app, chamadas) = app_abas();
    let app_ = centro(app.rect("logs-app"));
    let destino = na_aba(&app, "logs-acesso", 0.75);

    // Sem segurar, o dedo andando é um pan, não um arrasto.
    app.ui().pointer_move(app_, PointerKind::Touch);
    app.ui().pointer_down(app_, PointerKind::Touch, 1);
    app.frame();
    app.ui().pointer_move(destino, PointerKind::Touch);
    app.frame();
    assert_eq!(app.drag_label(), None);
    app.ui().pointer_up(destino, PointerKind::Touch);
    app.frame();
    assert!(chamadas.borrow().is_empty());

    // Segurando, o arrasto arma; andando, começa.
    app.ui().pointer_move(app_, PointerKind::Touch);
    app.ui().pointer_down(app_, PointerKind::Touch, 1);
    app.frame();
    app.wait(HOLD_DELAY);
    assert!(!app.is_open("menu-app"), "o menu espera o dedo sair");
    app.ui().pointer_move(destino, PointerKind::Touch);
    app.frame();
    assert_eq!(app.drag_label().as_deref(), Some("app"));
    app.ui().pointer_up(destino, PointerKind::Touch);
    app.frame();
    assert_eq!(*chamadas.borrow(), [(0, 2)]);
    assert!(!app.is_open("menu-app"));

    // Segurando sem andar, o dedo sai e o menu de contexto abre.
    let erro = centro(app.rect("logs-erro"));
    app.ui().pointer_move(erro, PointerKind::Touch);
    app.ui().pointer_down(erro, PointerKind::Touch, 1);
    app.frame();
    app.wait(HOLD_DELAY);
    app.ui().pointer_up(erro, PointerKind::Touch);
    app.frame();
    assert!(app.is_open("menu-erro"));
    assert_eq!(chamadas.borrow().len(), 1);
}

#[test]
fn sem_on_reorder_a_aba_nao_arrasta() {
    let mut app = TestApp::new(|| {
        let aberto = signal("a".to_string());
        Form::new("Abas").child(
            TabView::bind(aberto)
                .id("abas")
                .tab("a", "Primeira", Text::new("1"))
                .tab("b", "Segunda", Text::new("2")),
        )
    })
    .size(600, 300);
    let b = centro(app.rect("abas-b"));
    app.press("abas-a").move_to(b);
    assert_eq!(app.drag_label(), None);
    app.release();
    assert_eq!(app.tab_titles("abas"), ["Primeira", "Segunda"]);
}

/// Barra de menus com "Arquivo" (Novo, Exibir ▸ Quebra de linha, Sair) e "Ajuda", uma lista
/// de logs com menu de contexto e um botão com atalho. `log` anota o que rodou.
fn janela(log: Rc<std::cell::RefCell<Vec<String>>>) -> Form {
    let quebra = signal(true);
    let anota = |log: &Rc<std::cell::RefCell<Vec<String>>>, o: &'static str| {
        let log = Rc::clone(log);
        move || log.borrow_mut().push(o.to_string())
    };
    Form::new("Logs").child(
        Column::new()
            .child(
                MenuBar::new()
                    .id("barra")
                    .menu(
                        "Arquivo",
                        MenuFlyout::new()
                            .id("arquivo")
                            .item("novo", "Novo", anota(&log, "novo"))
                            .shortcut(Shortcut::ctrl('N'))
                            .submenu(
                                "exibir",
                                "Exibir",
                                MenuFlyout::new()
                                    .id("exibir-menu")
                                    .toggle("quebra", "Quebra de linha", quebra)
                                    .shortcut(Shortcut::ctrl_shift('W')),
                            )
                            .separator()
                            .item("sair", "Sair", anota(&log, "sair")),
                    )
                    .access_key('A')
                    .menu(
                        "Ajuda",
                        MenuFlyout::new()
                            .id("ajuda")
                            .item("sobre", "Sobre", anota(&log, "sobre")),
                    ),
            )
            .child(
                Column::new()
                    .id("lista")
                    .child(Text::new("app.log").id("app-log"))
                    .child(Text::new("erro.log").id("erro-log"))
                    .context_flyout(MenuFlyout::new().id("lista-menu").item(
                        "copiar",
                        "Copiar",
                        anota(&log, "copiar"),
                    )),
            )
            .child(
                Button::new("Recarregar")
                    .id("recarregar")
                    .height(44)
                    .on_click(anota(&log, "recarregar"))
                    .accelerator(Shortcut::key(Key::F(5)), anota(&log, "recarregar")),
            )
            .child(Text::new(move || format!("quebra: {}", quebra.get())).id("estado")),
    )
}

fn app_janela() -> (TestApp, Rc<std::cell::RefCell<Vec<String>>>) {
    let log = Rc::new(std::cell::RefCell::new(Vec::new()));
    let l = Rc::clone(&log);
    let app = TestApp::new(move || janela(l)).size(600, 400);
    (app, log)
}

#[test]
fn barra_de_menus_abre_troca_e_fecha() {
    let (mut app, log) = app_janela();
    assert!(!app.is_open("arquivo"));
    app.click("barra-arquivo");
    assert!(app.is_open("arquivo"));
    // Com um menu aberto, o ponteiro sobre outro item troca de menu.
    app.hover("barra-ajuda");
    assert!(!app.is_open("arquivo"));
    assert!(app.is_open("ajuda"));
    app.click("sobre");
    assert!(!app.is_open("ajuda"));
    assert_eq!(*log.borrow(), ["sobre"]);

    // O submenu abre ao lado, e o item de marcar troca o signal.
    app.click("barra-arquivo").click("exibir");
    assert!(app.is_open("exibir-menu"));
    assert_eq!(app.text("estado"), "quebra: true");
    app.click("quebra");
    assert_eq!(app.text("estado"), "quebra: false");
    assert!(!app.is_open("arquivo"), "escolher fecha os menus");
    assert!(!app.is_open("exibir-menu"));

    // Teclado: F10 foca a barra, ↓ abre, Esc fecha e devolve o foco ao item.
    app.key(Key::F(10));
    assert_eq!(app.focused().as_deref(), Some("barra-arquivo"));
    app.key(Key::Right);
    assert_eq!(app.focused().as_deref(), Some("barra-ajuda"));
    app.key(Key::Down);
    assert!(app.is_open("ajuda"));
    app.key(Key::Escape);
    assert!(!app.is_open("ajuda"));
    assert_eq!(app.focused().as_deref(), Some("barra-ajuda"));
}

#[test]
fn atalhos_rodam_com_o_menu_fechado() {
    let (mut app, log) = app_janela();
    app.shortcut('n');
    app.shortcut(Shortcut::ctrl('N'));
    app.shortcut(Shortcut::key(Key::F(5)));
    assert_eq!(*log.borrow(), ["novo", "novo", "recarregar"]);
    assert!(!app.is_open("arquivo"));
    app.shortcut(Shortcut::ctrl_shift('W'));
    assert_eq!(app.text("estado"), "quebra: false");
    // Alt e a letra abrem o menu da barra.
    app.shortcut(Shortcut::alt('A'));
    assert!(app.is_open("arquivo"));

    let arvore = app.tree();
    let novo = arvore.find("novo").unwrap();
    assert_eq!(novo.accelerators, ["Ctrl+N"]);
    assert_eq!(
        arvore.find("quebra").unwrap().accelerators,
        ["Ctrl+Shift+W"]
    );
    assert_eq!(arvore.find("quebra").unwrap().kind, "toggle_menu_item");
    assert_eq!(arvore.find("recarregar").unwrap().accelerators, ["F5"]);
    assert_eq!(
        arvore.find("barra-arquivo").unwrap().accelerators,
        ["Alt+A"]
    );
    assert_eq!(arvore.find("barra").unwrap().kind, "menu_bar");
}

#[test]
fn atalho_repetido_e_erro_do_lint() {
    let mut app = TestApp::new(|| {
        Form::new("Atalhos").child(
            Column::new()
                .child(
                    Button::new("Salvar")
                        .id("salvar")
                        .accelerator(Shortcut::ctrl('S'), || {}),
                )
                .child(
                    Button::new("Enviar")
                        .id("enviar")
                        .accelerator(Shortcut::ctrl('S'), || {}),
                ),
        )
    });
    let problemas = app.lint();
    assert!(
        problemas
            .iter()
            .any(|p| p.rule.name() == "duplicate-accelerator" && p.message.contains("Ctrl+S")),
        "{problemas:?}"
    );
}

#[test]
fn menu_de_contexto_sobre_a_lista() {
    let (mut app, log) = app_janela();
    assert!(app.tree().find("lista").unwrap().context_flyout);
    assert!(!app.tree().find("app-log").unwrap().context_flyout);
    // O botão direito sobre um item da lista sobe até quem tem o menu.
    app.context_menu("erro-log");
    assert!(app.is_open("lista-menu"));
    let ponto = centro(app.rect("erro-log"));
    let menu = app.rect("lista-menu");
    assert!(
        (menu.x0 - ponto.x).abs() < 1.0 && (menu.y0 - ponto.y).abs() < 1.0,
        "o canto do menu fica no ponteiro: {menu:?}, {ponto:?}"
    );
    app.click("copiar");
    assert!(!app.is_open("lista-menu"));
    assert_eq!(*log.borrow(), ["copiar"]);
    // Esc fecha sem rodar nada.
    app.context_menu("app-log").key(Key::Escape);
    assert!(!app.is_open("lista-menu"));
    assert_eq!(log.borrow().len(), 1);
    app.assert_lint();
}

const TEMAS: [Theme; 2] = [Theme::Light, Theme::Dark];
const DENSIDADES: [Density; 2] = [Density::Standard, Density::Compact];

fn nome_da_densidade(densidade: Density) -> &'static str {
    match densidade {
        Density::Standard => "padrao",
        Density::Compact => "compacto",
    }
}

#[test]
fn snapshots_do_divisor() {
    for tema in TEMAS {
        for densidade in DENSIDADES {
            for estado in ["rest", "hover", "pressed", "focused"] {
                let mut app = TestApp::new(move || {
                    Form::new("Sessões").density(densidade).child(
                        Split::row(
                            Column::new()
                                .child(Text::new("Servidores").ramp(TextRamp::BodyStrong))
                                .child(Text::new("web-01"))
                                .child(Text::new("db-01")),
                            Text::new("Conectado a web-01."),
                        )
                        .id("div")
                        .size(signal(140.0))
                        .grow(1),
                    )
                })
                .size(360, 120)
                .theme(tema);
                match estado {
                    "hover" => {
                        app.hover("div-divisor");
                    }
                    "pressed" => {
                        let ponto = centro(app.rect("div-divisor"));
                        app.ui().pointer_move(ponto, PointerKind::Mouse);
                        app.ui().pointer_down(ponto, PointerKind::Mouse, 1);
                        app.frame();
                    }
                    "focused" => {
                        app.tab();
                        assert_eq!(app.focused().as_deref(), Some("div-divisor"));
                    }
                    _ => {}
                }
                app.assert_lint();
                app.assert_snapshot(&format!(
                    "m20_divisor_{estado}_{}_{}",
                    nome_da_densidade(densidade),
                    comum::theme_name(tema)
                ));
            }
        }
    }
}

/// Janela com a barra de menus de um cliente SSH.
fn com_barra(densidade: Density) -> Form {
    let quebra = signal(true);
    let barra = signal(false);
    Form::new("Sessões").density(densidade).child(
        Column::new()
            .child(
                MenuBar::new()
                    .id("barra")
                    .menu(
                        "Arquivo",
                        MenuFlyout::new()
                            .id("arquivo")
                            .item("nova", "Nova sessão", || {})
                            .icon(Symbol::Add)
                            .shortcut(Shortcut::ctrl('N'))
                            .item("abrir", "Abrir...", || {})
                            .icon(Symbol::FolderOpen)
                            .shortcut(Shortcut::ctrl('O'))
                            .submenu(
                                "exibir",
                                "Exibir",
                                MenuFlyout::new()
                                    .id("exibir-menu")
                                    .toggle("quebra", "Quebra de linha", quebra)
                                    .toggle("status", "Barra de status", barra),
                            )
                            .separator()
                            .item("sair", "Sair", || {}),
                    )
                    .access_key('A')
                    .menu(
                        "Editar",
                        MenuFlyout::new()
                            .id("editar")
                            .item("copiar", "Copiar", || {}),
                    )
                    .menu(
                        "Ajuda",
                        MenuFlyout::new().id("ajuda").item("sobre", "Sobre", || {}),
                    ),
            )
            .child(Text::new("Nenhuma sessão aberta.")),
    )
}

#[test]
fn snapshots_da_barra_de_menus() {
    for tema in TEMAS {
        for densidade in DENSIDADES {
            let nome = |s: &str| {
                format!(
                    "m20_barra_{s}_{}_{}",
                    nome_da_densidade(densidade),
                    comum::theme_name(tema)
                )
            };
            let mut app = TestApp::new(move || com_barra(densidade))
                .size(420, 280)
                .theme(tema);
            app.hover("barra-editar");
            app.assert_lint();
            app.assert_snapshot(&nome("fechada"));
            app.click("barra-arquivo");
            app.hover("abrir");
            app.assert_lint();
            app.assert_snapshot(&nome("aberta"));
            app.click("exibir");
            app.hover("status");
            assert!(app.is_open("exibir-menu"));
            app.assert_lint();
            app.assert_snapshot(&nome("submenu"));
        }
    }
}

#[test]
fn snapshots_do_menu_de_contexto() {
    for tema in TEMAS {
        for densidade in DENSIDADES {
            let mut app = TestApp::new(move || {
                Form::new("Logs").density(densidade).child(
                    Column::new()
                        .id("lista")
                        .child(Text::new("app.log").id("l1"))
                        .child(Text::new("erro.log").id("l2"))
                        .child(Text::new("acesso.log").id("l3"))
                        .context_flyout(
                            MenuFlyout::new()
                                .id("lista-menu")
                                .item("abrir", "Abrir", || {})
                                .item("copiar", "Copiar o caminho", || {})
                                .shortcut(Shortcut::ctrl_shift('C'))
                                .separator()
                                .item("remover", "Remover da lista", || {})
                                .icon(Symbol::Delete),
                        ),
                )
            })
            .size(360, 260)
            .theme(tema);
            app.context_menu("l2");
            app.hover("copiar");
            app.assert_lint();
            app.assert_snapshot(&format!(
                "m20_contexto_{}_{}",
                nome_da_densidade(densidade),
                comum::theme_name(tema)
            ));
        }
    }
}

#[test]
fn snapshots_do_arrasto_de_aba() {
    for tema in TEMAS {
        for densidade in DENSIDADES {
            let mut app = TestApp::new(move || {
                let logs = signal(
                    ["app.log", "erro.log", "acesso.log", "cron.log"]
                        .map(String::from)
                        .to_vec(),
                );
                let aberto = signal("app.log".to_string());
                Form::new("Logs").density(densidade).child(
                    TabView::bind(aberto)
                        .id("logs")
                        .grow(1)
                        .items(logs, |l| l.clone(), |l| l.clone(), Text::new)
                        .on_reorder(move |de, para| {
                            logs.update(|v| {
                                let l = v.remove(de);
                                v.insert(para, l);
                            })
                        }),
                )
            })
            .size(560, 140)
            .theme(tema);
            let destino = na_aba(&app, "logs-acesso-log", 0.7);
            app.press("logs-app-log").move_to(destino);
            assert_eq!(app.drag_label().as_deref(), Some("app.log"));
            app.assert_snapshot(&format!(
                "m20_arrasto_{}_{}",
                nome_da_densidade(densidade),
                comum::theme_name(tema)
            ));
            app.release();
            assert_eq!(
                app.tab_titles("logs"),
                ["erro.log", "acesso.log", "app.log", "cron.log"]
            );
        }
    }
}
