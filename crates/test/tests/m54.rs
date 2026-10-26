//! Marco M54: painéis com alfinete (`DockHost`). O estado e a montagem, o auto-ocultar, o
//! painel flutuante, fechar, maximizar e o controlador.

mod comum;

use comum::theme_name;

use std::cell::Cell;
use std::rc::Rc;

use tomni_ui::DOCK_HOVER_DELAY;
use tomni_ui::DOCK_LEAVE_DELAY;
use tomni_ui::kurbo::Point;
use tomni_ui::prelude::*;
use tomni_ui::theme::Theme;
use tomni_ui::{Key, SPLIT_KEY_STEP, WindowHandle};
use tomni_ui_test::TestApp;

/// Uma tela com o `DockHost` `h`, o conteúdo `centro` e os painéis dados. Devolve também o
/// layout e o controlador.
fn app_com(
    layout: DockLayout,
    paineis: impl Fn() -> Vec<DockPane> + 'static,
    largura: u32,
    altura: u32,
) -> (TestApp, Signal<DockLayout>, DockController) {
    app_com_tema(layout, paineis, largura, altura, Theme::Light)
}

fn app_com_tema(
    layout: DockLayout,
    paineis: impl Fn() -> Vec<DockPane> + 'static,
    largura: u32,
    altura: u32,
    tema: Theme,
) -> (TestApp, Signal<DockLayout>, DockController) {
    let guarda = Rc::new(Cell::new(None));
    let g = Rc::clone(&guarda);
    let app = TestApp::new(move || {
        let sinal = signal(layout.clone());
        let controlador = DockController::new();
        g.set(Some((sinal, controlador.clone())));
        let mut host = DockHost::new(Text::new("Conteúdo").id("centro"))
            .id("h")
            .layout(sinal)
            .controller(&controlador);
        for painel in paineis() {
            host = host.pane(painel);
        }
        Form::new("Painéis").child(host)
    })
    .size(largura, altura)
    .theme(tema);
    let (sinal, controlador) = guarda.take().unwrap();
    (app, sinal, controlador)
}

fn painel(chave: &str, titulo: &str) -> DockPane {
    let id = format!("corpo-{chave}");
    DockPane::new(chave, Symbol::Folder, titulo.to_string(), move || {
        Column::new()
            .padding(12.0)
            .child(Button::new("Ação").id(id.clone()))
    })
}

fn dois_a_esquerda() -> Vec<DockPane> {
    vec![
        painel("arvore", "Árvore").size(240.0),
        painel("props", "Propriedades").size(240.0),
    ]
}

#[test]
fn o_layout_vazio_recebe_o_padrao_dos_paineis() {
    let (app, layout, _) = app_com(DockLayout::default(), dois_a_esquerda, 900, 500);
    let l = layout.get_untracked();
    let arvore = l.pane("arvore").expect("a árvore entra no layout");
    assert_eq!(arvore.mode, DockMode::Pinned);
    assert_eq!(arvore.side, DockSide::Left);
    assert_eq!(l.area(DockSide::Left).order, ["arvore", "props"]);
    assert!(app.visible("h-arvore"));
    assert!(app.visible("h-props"));
    assert!(app.rect("h-arvore").x1 <= app.rect("centro").x0);
}

fn auto_ocultar() -> Vec<DockPane> {
    vec![
        painel("arvore", "Árvore").size(240.0),
        painel("props", "Propriedades")
            .side(DockSide::Right)
            .mode(DockMode::AutoHide)
            .size(260.0),
        painel("saida", "Saída")
            .side(DockSide::Bottom)
            .mode(DockMode::AutoHide)
            .size(150.0),
        painel("ajuda", "Ajuda")
            .mode(DockMode::AutoHide)
            .size(200.0),
    ]
}

fn modo(layout: Signal<DockLayout>, chave: &str) -> DockMode {
    layout.get_untracked().pane(chave).unwrap().mode
}

#[test]
fn o_alfinete_solta_o_painel_para_a_faixa() {
    let (mut app, layout, _) = app_com(DockLayout::default(), dois_a_esquerda, 900, 500);
    app.click("h-arvore-alfinete");
    assert_eq!(modo(layout, "arvore"), DockMode::AutoHide);
    assert!(app.visible("h-arvore-faixa"));
    // O painel recolhido sai de cima e o outro ocupa a área toda.
    assert!(!app.visible("h-arvore"));
    assert!(app.rect("h-props").height() > 400.0);
}

#[test]
fn clicar_na_faixa_abre_por_cima_sem_mexer_no_conteudo() {
    let (mut app, _, controlador) = app_com(DockLayout::default(), auto_ocultar, 900, 500);
    let antes = app.rect("centro");
    app.click("h-props-faixa");
    assert!(controlador.is_open("props"));
    assert!(app.visible("h-props"));
    assert_eq!(app.rect("centro"), antes);
    // Encosta na faixa da direita.
    assert!((app.rect("h-props").x1 - app.rect("h-props-faixa").x0).abs() <= 8.0);
    assert!(app.rect("h-props").x0 > antes.x0);
}

#[test]
fn parar_o_mouse_na_faixa_abre_depois_do_atraso() {
    let (mut app, _, controlador) = app_com(DockLayout::default(), auto_ocultar, 900, 500);
    app.hover("h-props-faixa");
    assert!(!controlador.is_open("props"));
    app.wait(DOCK_HOVER_DELAY / 2);
    assert!(!controlador.is_open("props"));
    app.wait(DOCK_HOVER_DELAY);
    assert!(controlador.is_open("props"));
}

#[test]
fn sair_com_o_mouse_recolhe_depois_do_atraso() {
    let (mut app, _, controlador) = app_com(DockLayout::default(), auto_ocultar, 900, 500);
    app.hover("h-props-faixa");
    app.wait(DOCK_HOVER_DELAY * 2);
    assert!(controlador.is_open("props"));
    app.hover("h-props-corpo");
    app.wait(DOCK_LEAVE_DELAY * 2);
    assert!(controlador.is_open("props"));
    app.hover("centro");
    app.wait(DOCK_LEAVE_DELAY / 2);
    assert!(controlador.is_open("props"));
    app.wait(DOCK_LEAVE_DELAY);
    assert!(!controlador.is_open("props"));
}

#[test]
fn clicar_fora_recolhe_e_o_clique_nao_chega_ao_conteudo() {
    let cliques = Rc::new(Cell::new(0));
    let c = Rc::clone(&cliques);
    let (mut app, _, controlador) = {
        let guarda = Rc::new(Cell::new(None));
        let g = Rc::clone(&guarda);
        let app = TestApp::new(move || {
            let sinal = signal(DockLayout::default());
            let controlador = DockController::new();
            g.set(Some((sinal, controlador.clone())));
            let c = Rc::clone(&c);
            let mut host = DockHost::new(
                Button::new("Conteúdo")
                    .id("botao")
                    .on_click(move || c.set(c.get() + 1)),
            )
            .id("h")
            .layout(sinal)
            .controller(&controlador);
            for painel in auto_ocultar() {
                host = host.pane(painel);
            }
            Form::new("Painéis").child(host)
        })
        .size(900, 500);
        let (sinal, controlador) = guarda.take().unwrap();
        (app, sinal, controlador)
    };
    app.click("h-props-faixa");
    assert!(controlador.is_open("props"));
    app.click("botao");
    assert!(!controlador.is_open("props"));
    assert_eq!(cliques.get(), 0);
    app.click("botao");
    assert_eq!(cliques.get(), 1);
}

#[test]
fn com_o_foco_dentro_nao_recolhe_ao_sair() {
    let (mut app, _, controlador) = app_com(DockLayout::default(), auto_ocultar, 900, 500);
    app.click("h-props-faixa");
    assert_eq!(app.focused().as_deref(), Some("corpo-props"));
    app.hover("centro");
    app.wait(DOCK_LEAVE_DELAY * 3);
    assert!(controlador.is_open("props"));
}

#[test]
fn esc_recolhe_e_devolve_o_foco_ao_botao_da_faixa() {
    let (mut app, _, controlador) = app_com(DockLayout::default(), auto_ocultar, 900, 500);
    app.click("h-props-faixa");
    app.key(Key::Escape);
    assert!(!controlador.is_open("props"));
    assert_eq!(app.focused().as_deref(), Some("h-props-faixa"));
}

#[test]
fn recolher_e_abrir_nao_remonta() {
    let montagens = Rc::new(Cell::new(0));
    let m = Rc::clone(&montagens);
    let (mut app, _, controlador) = app_com(
        DockLayout::default(),
        move || {
            let m = Rc::clone(&m);
            vec![
                DockPane::new("props", Symbol::Folder, "Propriedades", move || {
                    m.set(m.get() + 1);
                    Text::new("corpo")
                })
                .mode(DockMode::AutoHide),
            ]
        },
        900,
        500,
    );
    let inicial = montagens.get();
    assert_eq!(inicial, 1);
    app.click("h-props-faixa");
    app.key(Key::Escape);
    app.click("h-props-faixa");
    controlador.hide("props");
    assert_eq!(montagens.get(), inicial);
}

#[test]
fn abrir_um_recolhe_o_outro() {
    let (mut app, _, controlador) = app_com(DockLayout::default(), auto_ocultar, 900, 500);
    app.click("h-props-faixa");
    app.click("h-saida-faixa");
    assert!(!controlador.is_open("props"));
    assert!(controlador.is_open("saida"));
    assert!(!app.visible("h-props"));
}

#[test]
fn o_painel_da_direita_encosta_na_borda_direita() {
    let (mut app, _, _) = app_com(DockLayout::default(), auto_ocultar, 900, 500);
    app.click("h-props-faixa");
    let faixa = app.rect("h-props-faixa");
    let painel = app.rect("h-props");
    assert!(painel.x1 <= faixa.x0 + 1.0 && painel.x1 >= faixa.x0 - 8.0);
    assert!((painel.width() - 260.0).abs() <= 1.0);
}

#[test]
fn o_painel_de_baixo_encosta_na_faixa() {
    let (mut app, _, _) = app_com(DockLayout::default(), auto_ocultar, 900, 500);
    app.click("h-saida-faixa");
    let faixa = app.rect("h-saida-faixa");
    let painel = app.rect("h-saida");
    assert!(painel.y1 <= faixa.y0 + 1.0 && painel.y1 >= faixa.y0 - 8.0);
    assert!((painel.height() - 150.0).abs() <= 1.0);
}

#[test]
fn a_borda_do_painel_por_cima_grava_overlay_size() {
    let (mut app, layout, _) = app_com(DockLayout::default(), auto_ocultar, 900, 500);
    app.click("h-props-faixa");
    let borda = app.rect("h-props-borda").center();
    app.drag("h-props-borda", Point::new(borda.x - 60.0, borda.y));
    let tamanho = layout.get_untracked().pane("props").unwrap().overlay_size;
    assert!((tamanho - 320.0).abs() <= 2.0, "{tamanho}");
    assert!((app.rect("h-props").width() - 320.0).abs() <= 2.0);
}

/// Passa o foco por Tab até chegar em `id`.
fn foco_em(app: &mut TestApp, id: &str) {
    for _ in 0..30 {
        if app.focused().as_deref() == Some(id) {
            return;
        }
        app.tab();
    }
    panic!("o Tab não chegou em {id:?}");
}

fn layout_com(chaves: &[(&str, DockSide, DockMode)]) -> DockLayout {
    let mut layout = DockLayout::default();
    for (chave, lado, modo) in chaves {
        layout.panes.insert(
            (*chave).to_string(),
            tomni_ui::DockPaneState {
                mode: *modo,
                side: *lado,
                weight: 1.0,
                overlay_size: 300.0,
                floating: None,
                docked: if *modo == DockMode::Floating {
                    DockMode::Pinned
                } else {
                    *modo
                },
            },
        );
        layout.area_mut(*lado).order.push((*chave).to_string());
    }
    layout
}

#[test]
fn uma_chave_gravada_que_sumiu_e_ignorada() {
    let layout = layout_com(&[
        ("arvore", DockSide::Left, DockMode::Pinned),
        ("fantasma", DockSide::Left, DockMode::Pinned),
    ]);
    let (app, sinal, _) = app_com(layout, dois_a_esquerda, 900, 500);
    let l = sinal.get_untracked();
    assert!(l.pane("fantasma").is_none());
    assert_eq!(l.area(DockSide::Left).order, ["arvore", "props"]);
    assert!(app.visible("h-arvore"));
}

#[test]
fn um_painel_novo_entra_com_o_padrao() {
    let layout = layout_com(&[("arvore", DockSide::Left, DockMode::AutoHide)]);
    let (app, sinal, _) = app_com(layout, dois_a_esquerda, 900, 500);
    let l = sinal.get_untracked();
    // O que o layout já tinha continua como estava.
    assert_eq!(l.pane("arvore").unwrap().mode, DockMode::AutoHide);
    // O novo entra Fixado no lado padrão.
    assert_eq!(l.pane("props").unwrap().mode, DockMode::Pinned);
    assert!(app.visible("h-props"));
    assert!(app.visible("h-arvore-faixa"));
}

#[test]
fn dois_fixados_no_mesmo_lado_dividem_pelo_peso() {
    let (_app, layout, _) = app_com(DockLayout::default(), dois_a_esquerda, 900, 500);
    layout.update(|l| {
        l.panes.get_mut("arvore").unwrap().weight = 3.0;
        l.panes.get_mut("props").unwrap().weight = 1.0;
    });
    let (mut app, layout, _) = app_com(layout.get_untracked(), dois_a_esquerda, 900, 500);
    let a = app.rect("h-arvore").height();
    let b = app.rect("h-props").height();
    assert!((a / b - 3.0).abs() < 0.2, "{a} / {b}");
    // Arrastar o divisor entre os dois passa peso de um para o outro.
    let centro = app.rect("h-arvore-divisor").center();
    app.drag("h-arvore-divisor", Point::new(centro.x, centro.y - 100.0));
    let l = layout.get_untracked();
    assert!(l.pane("arvore").unwrap().weight < 3.0 * l.pane("props").unwrap().weight);
    assert!(app.rect("h-arvore").height() < a);
}

#[test]
fn o_divisor_da_area_grava_o_tamanho() {
    let (mut app, layout, _) = app_com(DockLayout::default(), dois_a_esquerda, 900, 500);
    let largura = app.rect("h-arvore").width();
    let centro = app.rect("h-esquerda-divisor").center();
    app.drag("h-esquerda-divisor", Point::new(centro.x + 60.0, centro.y));
    let tamanho = layout.get_untracked().area(DockSide::Left).size;
    assert!((tamanho - (largura + 60.0)).abs() <= 2.0, "{tamanho}");
    assert!((app.rect("h-arvore").width() - (largura + 60.0)).abs() <= 2.0);
}

#[test]
fn as_setas_movem_o_divisor_da_area() {
    let (mut app, layout, _) = app_com(DockLayout::default(), dois_a_esquerda, 900, 500);
    let antes = app.rect("h-arvore").width();
    foco_em(&mut app, "h-esquerda-divisor");
    app.key(Key::Right);
    assert!((app.rect("h-arvore").width() - (antes + SPLIT_KEY_STEP)).abs() <= 1.0);
    assert!(
        (layout.get_untracked().area(DockSide::Left).size - (antes + SPLIT_KEY_STEP)).abs() <= 1.0
    );
    app.key(Key::Left);
    assert!((app.rect("h-arvore").width() - antes).abs() <= 1.0);
}

#[test]
fn com_a_janela_pequena_o_centro_encolhe_ate_o_minimo_e_depois_as_areas() {
    let (mut app, _, _) = app_com(DockLayout::default(), dois_a_esquerda, 900, 500);
    let area = app.rect("h-arvore").width();
    app.resize(400, 500);
    let centro = app.rect("centro");
    assert!(centro.width() >= 100.0 - 1.0, "{centro:?}");
    assert!(app.rect("h-arvore").width() <= area);
    app.resize(200, 500);
    assert!(app.rect("h-arvore").width() < area);
}

fn flutuante() -> DockLayout {
    layout_com(&[
        ("arvore", DockSide::Left, DockMode::Pinned),
        ("props", DockSide::Right, DockMode::Floating),
    ])
}

#[test]
fn flutuar_abre_uma_janela_com_o_titulo_e_tira_o_painel_daqui() {
    let (mut app, layout, _) = app_com(DockLayout::default(), dois_a_esquerda, 900, 500);
    assert_eq!(app.windows().len(), 1);
    app.click("h-props-flutuar");
    assert_eq!(modo(layout, "props"), DockMode::Floating);
    let janelas = app.windows();
    assert_eq!(janelas.len(), 2);
    assert_eq!(janelas[1].title(), "Propriedades");
    app.switch_to(janelas[1]);
    assert!(app.visible("h-props"));
    assert!(app.visible("h-props-flutuar"));
}

#[test]
fn fechar_a_janela_flutuante_devolve_o_painel_ao_lugar() {
    let (mut app, layout, _) = app_com(DockLayout::default(), dois_a_esquerda, 900, 500);
    app.click("h-props-flutuar");
    let janela = app.windows()[1];
    assert!(app.close_window(janela));
    assert_eq!(app.windows().len(), 1);
    assert_eq!(modo(layout, "props"), DockMode::Pinned);
    app.switch_to(WindowHandle::main());
    assert!(app.visible("h-props"));
}

#[test]
fn encaixar_pelo_cabecalho_da_janela_flutuante() {
    let (mut app, layout, _) = app_com(DockLayout::default(), dois_a_esquerda, 900, 500);
    app.click("h-props-flutuar");
    let janela = app.windows()[1];
    app.switch_to(janela);
    app.click("h-props-flutuar");
    assert_eq!(modo(layout, "props"), DockMode::Pinned);
    assert_eq!(app.windows().len(), 1);
}

#[test]
fn um_layout_gravado_flutuando_abre_a_janela_na_montagem() {
    let (app, _, _) = app_com(flutuante(), dois_a_esquerda, 900, 500);
    assert_eq!(app.windows().len(), 2);
}

#[test]
fn fechar_some_e_show_volta_no_modo_de_antes() {
    let (mut app, layout, controlador) = app_com(
        layout_com(&[
            ("arvore", DockSide::Left, DockMode::Pinned),
            ("props", DockSide::Right, DockMode::AutoHide),
        ]),
        dois_a_esquerda,
        900,
        500,
    );
    app.click("h-arvore-fechar");
    assert_eq!(modo(layout, "arvore"), DockMode::Closed);
    assert!(controlador.close("props"));
    assert!(!app.visible("h-props-faixa"));
    assert!(controlador.show("props"));
    assert_eq!(modo(layout, "props"), DockMode::AutoHide);
    assert!(controlador.show("arvore"));
    assert_eq!(modo(layout, "arvore"), DockMode::Pinned);
}

#[test]
fn closable_false_tira_o_botao_e_o_item_do_menu() {
    let (mut app, _, controlador) = app_com(
        DockLayout::default(),
        || vec![painel("arvore", "Árvore").closable(false)],
        900,
        500,
    );
    assert!(app.ui().find("h-arvore-fechar").is_none());
    app.right_click("h-arvore-cabecalho");
    assert!(app.ui().find("h-arvore-menu-fechar").is_none());
    // O controlador continua fechando.
    assert!(controlador.close("arvore"));
}

#[test]
fn floatable_false_tira_o_botao_e_o_item_do_menu() {
    let (mut app, _, _) = app_com(
        DockLayout::default(),
        || vec![painel("arvore", "Árvore").floatable(false)],
        900,
        500,
    );
    assert!(!app.visible("h-arvore-flutuar"));
    app.right_click("h-arvore-cabecalho");
    assert!(!app.visible("h-arvore-menu-flutuar"));
}

#[test]
fn duplo_clique_no_titulo_maximiza_e_esc_restaura() {
    let (mut app, layout, _) = app_com(DockLayout::default(), dois_a_esquerda, 900, 500);
    app.double_click("h-arvore-titulo");
    assert_eq!(layout.get_untracked().maximized.as_deref(), Some("arvore"));
    assert!(app.rect("h-arvore").width() > 800.0);
    assert!(!app.visible("centro"));
    app.key(Key::Escape);
    assert_eq!(layout.get_untracked().maximized, None);
    assert!(app.visible("centro"));
    app.double_click("h-arvore-titulo");
    assert_eq!(layout.get_untracked().maximized.as_deref(), Some("arvore"));
    app.double_click("h-arvore-titulo");
    assert_eq!(layout.get_untracked().maximized, None);
}

#[test]
fn o_menu_do_cabecalho_troca_o_lado() {
    let (mut app, layout, _) = app_com(DockLayout::default(), dois_a_esquerda, 900, 500);
    app.right_click("h-arvore-cabecalho");
    assert!(!app.visible("h-arvore-menu-esquerda"));
    app.click("h-arvore-menu-direita");
    let l = layout.get_untracked();
    assert_eq!(l.pane("arvore").unwrap().side, DockSide::Right);
    assert_eq!(l.area(DockSide::Right).order, ["arvore"]);
    assert!(app.rect("h-arvore").x0 > app.rect("centro").x1 - 1.0);
}

#[test]
fn o_controlador_faz_o_que_a_tabela_diz() {
    let (_app, layout, c) = app_com(DockLayout::default(), auto_ocultar, 900, 500);
    assert!(c.pin("ajuda"));
    assert_eq!(modo(layout, "ajuda"), DockMode::Pinned);
    assert!(c.unpin("ajuda"));
    assert_eq!(modo(layout, "ajuda"), DockMode::AutoHide);
    assert!(c.toggle("ajuda"));
    assert!(c.is_open("ajuda"));
    assert!(c.hide("ajuda"));
    assert!(!c.is_open("ajuda"));
    // Aberto, o toggle fecha de vez.
    assert!(c.show("ajuda"));
    assert!(c.toggle("ajuda"));
    assert_eq!(c.mode("ajuda"), Some(DockMode::Closed));
    assert!(c.show("ajuda"));
    assert!(c.hide("ajuda"));
    assert!(c.float("ajuda"));
    assert_eq!(c.mode("ajuda"), Some(DockMode::Floating));
    assert!(c.dock("ajuda"));
    assert_eq!(c.mode("ajuda"), Some(DockMode::AutoHide));
    assert!(c.move_to("ajuda", DockSide::Bottom));
    assert_eq!(
        layout.get_untracked().pane("ajuda").unwrap().side,
        DockSide::Bottom
    );
    assert!(c.maximize("arvore"));
    assert_eq!(layout.get_untracked().maximized.as_deref(), Some("arvore"));
    assert!(c.restore());
    assert!(!c.show("nao-existe"));
    assert_eq!(c.mode("nao-existe"), None);
}

#[test]
fn shown_acompanha_o_layout_e_o_set_abre_e_fecha() {
    let (mut app, layout, c) = app_com(DockLayout::default(), auto_ocultar, 900, 500);
    let visivel = c.shown("props");
    assert!(visivel.get_untracked());
    c.close("props");
    app.frame();
    assert!(!visivel.get_untracked());
    c.show("props");
    app.frame();
    assert!(visivel.get_untracked());
    visivel.set(false);
    app.frame();
    assert_eq!(modo(layout, "props"), DockMode::Closed);
    visivel.set(true);
    app.frame();
    assert_eq!(modo(layout, "props"), DockMode::AutoHide);
}

#[test]
fn os_botoes_usam_as_strings() {
    let (mut app, _, _) = app_com(DockLayout::default(), dois_a_esquerda, 900, 500);
    let s = tomni_ui::strings();
    assert_eq!(app.label("h-arvore-alfinete"), s.dock_unpin);
    assert_eq!(app.label("h-arvore-fechar"), s.dock_close);
    assert_eq!(app.label("h-arvore-flutuar"), s.dock_float);
    app.right_click("h-arvore-cabecalho");
    assert_eq!(app.label("h-arvore-menu-maximizar"), s.dock_maximize);
}

/// Os snapshots do dock: cada estado nos dois temas.
fn snapshots(nome: &str, passos: impl Fn(&mut TestApp)) {
    for tema in [Theme::Light, Theme::Dark] {
        let (mut app, _, _) = app_com_tema(DockLayout::default(), auto_ocultar, 900, 500, tema);
        passos(&mut app);
        app.assert_snapshot(&format!("{nome}_{}", theme_name(tema)));
    }
}

#[test]
fn snapshot_dock_fixado() {
    for tema in [Theme::Light, Theme::Dark] {
        let (app, _, _) = app_com_tema(DockLayout::default(), dois_a_esquerda, 900, 500, tema);
        app.assert_snapshot(&format!("dock_fixado_{}", theme_name(tema)));
    }
}

#[test]
fn snapshot_dock_faixas() {
    snapshots("dock_faixas", |_| {});
}

#[test]
fn snapshot_dock_por_cima_esquerda() {
    snapshots("dock_por_cima_esquerda", |app| {
        app.click("h-ajuda-faixa");
    });
}

#[test]
fn snapshot_dock_por_cima_direita() {
    snapshots("dock_por_cima_direita", |app| {
        app.click("h-props-faixa");
    });
}

#[test]
fn snapshot_dock_por_cima_embaixo() {
    snapshots("dock_por_cima_embaixo", |app| {
        app.click("h-saida-faixa");
    });
}

#[test]
fn snapshot_dock_maximizado() {
    snapshots("dock_maximizado", |app| {
        app.double_click("h-arvore-titulo");
    });
}

#[test]
fn snapshot_dock_foco_na_faixa() {
    snapshots("dock_foco_na_faixa", |app| {
        foco_em(app, "h-props-faixa");
    });
}

#[test]
fn snapshot_dock_menu_do_cabecalho() {
    snapshots("dock_menu_do_cabecalho", |app| {
        app.right_click("h-arvore-cabecalho");
    });
}
