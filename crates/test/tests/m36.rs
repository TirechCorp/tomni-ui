//! Marco M36 (pedido pelo Tirech SSH): a tecla de acesso e o item em realce do `MenuFlyout`,
//! as camadas que não recebem entrada (`Placement::Over`) e os números dos painéis do
//! `TerminalPanes` (`identify` e `highlight_pane`).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use tomni_ui::kurbo::Point;
use tomni_ui::prelude::*;
use tomni_ui::terminal::{Loopback, PaneDirection, PaneLayout, Session, TerminalPanes};
use tomni_ui::theme::Theme;
use tomni_ui::{Brush, Key, Look, Modifiers, Token};
use tomni_ui_test::TestApp;

#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/terminal_panes.rs"]
mod terminal_panes;

/// Um botão "Menu" que abre um menu com a tecla de acesso nos itens; `log` anota o que rodou.
fn janela_com_menu(log: Rc<RefCell<Vec<String>>>, marcado: Signal<bool>) -> Form {
    let anota = |log: &Rc<RefCell<Vec<String>>>, o: &'static str| {
        let log = Rc::clone(log);
        move || log.borrow_mut().push(o.to_string())
    };
    let escondido = signal(false);
    Form::new("Menu").child(
        Button::new("Menu").id("botao").flyout(
            MenuFlyout::new()
                .id("menu")
                .item("um", "Um", anota(&log, "um"))
                .access_key('1')
                .item("dois", "Dois", anota(&log, "dois"))
                .access_key('2')
                .item("sumido", "Sumido", anota(&log, "sumido"))
                .access_key('3')
                .item_visible(escondido)
                .item("tres", "Três", anota(&log, "tres"))
                .access_key('3')
                .toggle("marca", "Marcar", marcado)
                .access_key('M')
                .submenu(
                    "mais",
                    "Mais",
                    MenuFlyout::new()
                        .id("mais-menu")
                        .item("filho", "Filho", anota(&log, "filho"))
                        .access_key('f')
                        .item("dois-filho", "Dois do filho", anota(&log, "dois-filho"))
                        .access_key('2'),
                )
                .access_key('s'),
        ),
    )
}

fn app_com_menu() -> (TestApp, Rc<RefCell<Vec<String>>>, Signal<bool>) {
    let log = Rc::new(RefCell::new(Vec::new()));
    let guarda: Rc<Cell<Option<Signal<bool>>>> = Rc::new(Cell::new(None));
    let (l, g) = (Rc::clone(&log), Rc::clone(&guarda));
    let app = TestApp::new(move || {
        let marcado = signal(false);
        g.set(Some(marcado));
        janela_com_menu(l, marcado)
    })
    .size(600, 400);
    (app, log, guarda.get().unwrap())
}

#[test]
fn a_tecla_de_acesso_aciona_o_item_e_fecha_o_menu() {
    let (mut app, log, _) = app_com_menu();
    app.click("botao");
    assert!(app.is_open("menu"));
    app.key(Key::Char('2'));
    assert_eq!(*log.borrow(), ["dois"]);
    assert!(!app.is_open("menu"));
}

#[test]
fn a_tecla_de_acesso_nao_diferencia_maiuscula_de_minuscula() {
    let (mut app, log, marcado) = app_com_menu();
    // `.access_key('M')` e a tecla chega em minúscula; o item de marcar troca o valor.
    app.click("botao").key(Key::Char('m'));
    assert!(marcado.get_untracked());
    assert!(!app.is_open("menu"));
    app.click("botao").key(Key::Char('M'));
    assert!(!marcado.get_untracked());
    assert!(log.borrow().is_empty());
}

#[test]
fn a_tecla_de_acesso_com_modificador_nao_aciona() {
    let (mut app, log, _) = app_com_menu();
    app.click("botao");
    app.key_with(Key::Char('2'), Modifiers::CTRL);
    app.key_with(Key::Char('2'), Modifiers::ALT);
    app.key_with(Key::Char('2'), Modifiers::SHIFT);
    assert!(log.borrow().is_empty());
    assert!(app.is_open("menu"), "o menu continua aberto");
}

#[test]
fn item_escondido_nao_responde_e_a_tecla_repetida_vale_para_o_primeiro_que_aparece() {
    let (mut app, log, _) = app_com_menu();
    // "Sumido" e "Três" usam o `3`: o escondido não conta, e o "Três" responde.
    app.click("botao").key(Key::Char('3'));
    assert_eq!(*log.borrow(), ["tres"]);
}

#[test]
fn tecla_sem_item_nao_faz_nada() {
    let (mut app, log, _) = app_com_menu();
    app.click("botao").key(Key::Char('9'));
    assert!(log.borrow().is_empty());
    assert!(app.is_open("menu"));
}

#[test]
fn a_tecla_de_um_submenu_fechado_nao_faz_nada_e_a_do_submenu_aberto_aciona() {
    let (mut app, log, _) = app_com_menu();
    // O `f` é do item do submenu, que está fechado.
    app.click("botao").key(Key::Char('f'));
    assert!(log.borrow().is_empty());
    assert!(!app.is_open("mais-menu"));
    // A tecla do próprio submenu abre-o.
    app.key(Key::Char('s'));
    assert!(app.is_open("mais-menu"));
    assert!(app.is_open("menu"), "o menu de onde ele saiu fica aberto");
    // Com o submenu aberto, o `2` é o do filho (e não o "Dois" do menu de cima).
    app.key(Key::Char('2'));
    assert_eq!(*log.borrow(), ["dois-filho"]);
    assert!(!app.is_open("mais-menu"));
    assert!(!app.is_open("menu"), "escolher fecha a cadeia toda");
}

#[test]
fn a_letra_da_tecla_de_acesso_nao_e_digitada_em_quem_ganha_o_foco() {
    let digitado = Rc::new(RefCell::new(String::new()));
    let d = Rc::clone(&digitado);
    let mut app = TestApp::new(move || {
        let texto = signal(String::new());
        let d = Rc::clone(&d);
        effect(move || *d.borrow_mut() = texto.get());
        Form::new("Menu").child(
            Edit::bind(texto).id("campo").context_flyout(
                MenuFlyout::new()
                    .id("menu")
                    .item("dois", "Dois", || {})
                    .access_key('2'),
            ),
        )
    })
    .size(600, 400);
    app.click("campo").right_click("campo");
    assert!(app.is_open("menu"));
    app.key(Key::Char('2'));
    assert!(!app.is_open("menu"));
    assert_eq!(app.focused().as_deref(), Some("campo"));
    // A plataforma entrega o texto da mesma tecla depois do `key_down`.
    app.type_text("2");
    assert_eq!(*digitado.borrow(), "");
}

// ---------------------------------------------------------------------------------------------
// `MenuFlyout::highlighted`

/// Menu com submenu, gravando o item em realce.
fn janela_com_realce(realce: Signal<Option<String>>) -> Form {
    Form::new("Realce").child(
        Button::new("Menu").id("botao").flyout(
            MenuFlyout::new()
                .id("menu")
                .highlighted(realce)
                .item("abrir", "Abrir", || {})
                .submenu(
                    "abrir-em",
                    "Abrir em",
                    MenuFlyout::new()
                        .id("abrir-em-menu")
                        .item("painel-0", "Painel 1", || {})
                        .item("painel-1", "Painel 2", || {}),
                )
                .item("apagar", "Apagar", || {}),
        ),
    )
}

type Historico = Rc<RefCell<Vec<Option<String>>>>;

/// O app, o signal do realce e tudo o que o signal já valeu (um efeito anota cada valor).
fn app_com_realce() -> (TestApp, Signal<Option<String>>, Historico) {
    let guarda: Rc<Cell<Option<Signal<Option<String>>>>> = Rc::new(Cell::new(None));
    let historico: Historico = Rc::new(RefCell::new(Vec::new()));
    let (g, h) = (Rc::clone(&guarda), Rc::clone(&historico));
    let app = TestApp::new(move || {
        let realce = signal(None);
        g.set(Some(realce));
        effect(move || h.borrow_mut().push(realce.get()));
        janela_com_realce(realce)
    })
    .size(600, 400);
    (app, guarda.get().unwrap(), historico)
}

fn realce(signal: Signal<Option<String>>) -> Option<String> {
    signal.get_untracked()
}

#[test]
fn o_realce_acompanha_o_ponteiro_no_menu_raiz() {
    let (mut app, r, _) = app_com_realce();
    assert_eq!(realce(r), None);
    // Aberto pelo ponteiro, o menu dá o foco ao primeiro item, mas sem realce nenhum.
    app.click("botao");
    assert_eq!(realce(r), None);
    app.hover("apagar");
    assert_eq!(realce(r).as_deref(), Some("apagar"));
    app.hover("abrir");
    assert_eq!(realce(r).as_deref(), Some("abrir"));
    app.move_to(Point::new(580.0, 390.0));
    assert_eq!(realce(r), None);
    // Pelo teclado, o item com o foco fica em realce.
    app.key(Key::Down);
    assert_eq!(realce(r).as_deref(), Some("abrir-em"));
}

#[test]
fn abrir_o_submenu_pelo_ponteiro_grava_o_pai_e_depois_o_item_dele() {
    let (mut app, r, historico) = app_com_realce();
    app.click("botao").hover("abrir-em");
    app.wait(std::time::Duration::from_millis(450));
    assert!(app.is_open("abrir-em-menu"));
    assert_eq!(realce(r).as_deref(), Some("abrir-em"));
    app.hover("painel-1");
    assert_eq!(realce(r).as_deref(), Some("painel-1"));
    app.hover("painel-0");
    assert_eq!(realce(r).as_deref(), Some("painel-0"));
    // Do submenu para o pai, sem passar por um `None`.
    let antes = historico.borrow().len();
    app.hover("abrir-em");
    assert_eq!(realce(r).as_deref(), Some("abrir-em"));
    assert!(
        historico.borrow()[antes..].iter().all(Option::is_some),
        "passar de um menu a outro não deixa `None` no meio: {:?}",
        historico.borrow()
    );
}

#[test]
fn as_setas_mudam_o_realce_no_submenu_e_o_esc_e_o_fechar_gravam_none() {
    let (mut app, r, _) = app_com_realce();
    app.click("botao").key(Key::Down);
    assert_eq!(realce(r).as_deref(), Some("abrir-em"));
    app.key(Key::Right);
    assert!(app.is_open("abrir-em-menu"));
    assert_eq!(realce(r).as_deref(), Some("painel-0"));
    app.key(Key::Down);
    assert_eq!(realce(r).as_deref(), Some("painel-1"));
    // Esc fecha só o submenu: volta o item que o abriu.
    app.key(Key::Escape);
    assert!(!app.is_open("abrir-em-menu"));
    assert_eq!(realce(r).as_deref(), Some("abrir-em"));
    // Esc de novo fecha o menu.
    app.key(Key::Escape);
    assert!(!app.is_open("menu"));
    assert_eq!(realce(r), None);
}

#[test]
fn escolher_um_item_grava_none() {
    let (mut app, r, _) = app_com_realce();
    app.click("botao").hover("apagar");
    assert_eq!(realce(r).as_deref(), Some("apagar"));
    app.click("apagar");
    assert!(!app.is_open("menu"));
    assert_eq!(realce(r), None);
}

// ---------------------------------------------------------------------------------------------
// `TerminalPanes::identify` e `highlight_pane`

/// Um `TerminalPanes` de id `paineis` com `n` sessões `{prefixo}-1`… em abas de um painel só
/// (use `PaneLayout::grid`/`split` por `ajusta` para formar os painéis).
struct Paineis {
    app: TestApp,
    layout: Signal<PaneLayout>,
    identificar: Signal<bool>,
    destaque: Signal<Option<usize>>,
    sessoes: Vec<Session>,
}

impl Paineis {
    fn novo(prefixo: &str, n: usize, largura: u32, altura: u32, tema: Theme) -> Self {
        let sessoes: Vec<Session> = (1..=n)
            .map(|i| {
                let nome = format!("{prefixo}-{i}");
                Session::new(Loopback::new().echo())
                    .key(nome.clone())
                    .name(nome)
            })
            .collect();
        let guarda = Rc::new(Cell::new(None));
        let (g, s) = (Rc::clone(&guarda), sessoes.clone());
        let app = TestApp::new(move || {
            let layout = signal(PaneLayout::with(s.clone()));
            let identificar = signal(false);
            let destaque = signal(None);
            g.set(Some((layout, identificar, destaque)));
            Form::new("Painéis").child(
                Column::new()
                    .child(
                        Button::new("Menu")
                            .id("botao")
                            .height(44)
                            .flyout(MenuFlyout::new().id("menu").item("sair", "Sair", || {})),
                    )
                    .child(
                        TerminalPanes::bind(layout)
                            .id("paineis")
                            .identify(identificar)
                            .highlight_pane(destaque)
                            .grow(1),
                    ),
            )
        })
        .theme(tema)
        .size(largura, altura);
        let (layout, identificar, destaque) = guarda.get().unwrap();
        Self {
            app,
            layout,
            identificar,
            destaque,
            sessoes,
        }
    }

    fn grade(prefixo: &str, colunas: usize, linhas: usize, largura: u32, altura: u32) -> Self {
        let mut paineis = Self::novo(prefixo, colunas * linhas, largura, altura, Theme::Light);
        paineis.layout.update(|l| l.grid(colunas, linhas));
        paineis.app.frame();
        paineis
    }
}

impl Drop for Paineis {
    fn drop(&mut self) {
        for sessao in &self.sessoes {
            sessao.dispose();
        }
    }
}

fn crachas(app: &TestApp, quantos: usize) -> Vec<String> {
    (0..quantos)
        .filter(|n| {
            app.tree()
                .find(&format!("paineis-painel-{n}-numero"))
                .is_some()
                && app.is_open(&format!("paineis-painel-{n}-numero"))
                && app.rect(&format!("paineis-painel-{n}-numero")).width() > 0.0
        })
        .map(|n| app.text(&format!("paineis-painel-{n}-numero-texto")))
        .collect()
}

#[test]
fn grade_2x2_tem_um_cracha_por_painel_centrado_nele() {
    let mut p = Paineis::grade("g22", 2, 2, 900, 700);
    assert!(crachas(&p.app, 4).is_empty(), "desligado: nenhum crachá");
    p.identificar.set(true);
    p.app.frame();
    assert_eq!(crachas(&p.app, 4), ["1", "2", "3", "4"]);
    for n in 0..4 {
        let cracha = p.app.rect(&format!("paineis-painel-{n}-numero"));
        let painel = p.app.rect(&format!("paineis-painel-{n}"));
        assert!(
            (cracha.center().x - painel.center().x).abs() <= 1.0
                && (cracha.center().y - painel.center().y).abs() <= 1.0,
            "painel {n}: crachá {cracha:?} fora do centro de {painel:?}"
        );
        assert!(cracha.width() >= 120.0 && cracha.height() >= 120.0);
    }
    let info = p.app.panes("paineis");
    assert!(info.identifying);
    assert_eq!(info.highlighted, None);
}

#[test]
fn a_numeracao_segue_a_ordem_das_folhas_numa_divisao_irregular() {
    let mut p = Paineis::novo("irr", 3, 900, 700, Theme::Light);
    let sessoes = p.sessoes.clone();
    p.layout.update(|l| {
        let direita = l.split(0, PaneDirection::Right);
        let baixo = l.split(direita, PaneDirection::Down);
        // Um painel vazio não mostra o `TabView`: cada painel fica com uma sessão.
        l.move_tab(sessoes[1], direita);
        l.move_tab(sessoes[2], baixo);
    });
    p.identificar.set(true);
    p.app.frame();
    let forma = p.app.panes("paineis").layout;
    assert_eq!(crachas(&p.app, 3), ["1", "2", "3"], "layout {forma}");
    // A ordem das folhas é a do `pane(i)`: o crachá `n` fica no painel `n`.
    for n in 0..3 {
        let cracha = p.app.rect(&format!("paineis-painel-{n}-numero"));
        let painel = p.app.rect(&format!("paineis-painel-{n}"));
        assert!(painel.contains(cracha.center()), "painel {n}");
    }
}

#[test]
fn ligar_e_desligar_nao_remonta_os_paineis() {
    let mut p = Paineis::grade("liga", 2, 2, 900, 700);
    p.app.click("paineis-terminal-liga-2");
    let foco = p.app.focused();
    assert_eq!(foco.as_deref(), Some("paineis-terminal-liga-2"));
    p.identificar.set(true);
    p.app.frame();
    assert_eq!(crachas(&p.app, 4).len(), 4);
    assert_eq!(p.app.focused(), foco, "ligar não remonta o terminal");
    p.identificar.set(false);
    p.app.frame();
    assert!(crachas(&p.app, 4).is_empty());
    assert_eq!(p.app.focused(), foco, "desligar também não");
}

#[test]
fn o_cracha_nao_pega_o_clique_nem_o_foco() {
    let mut p = Paineis::grade("clique", 2, 2, 900, 700);
    p.identificar.set(true);
    p.app.frame();
    let meio = p.app.rect("paineis-painel-3-numero").center();
    p.app.click_at(meio, 1);
    assert_eq!(
        p.app.focused().as_deref(),
        Some("paineis-terminal-clique-4"),
        "o clique no meio do crachá chega ao terminal embaixo"
    );
}

#[test]
fn o_realce_pinta_o_cracha_na_cor_de_destaque_e_ignora_indice_fora() {
    let mut p = Paineis::grade("real", 2, 2, 900, 700);
    p.identificar.set(true);
    p.destaque.set(Some(2));
    p.app.frame();
    let cor = |p: &Paineis, n: usize| match p.app.look(&format!("paineis-painel-{n}-numero")) {
        Look::Border(visual) => visual,
        look => panic!("{look:?}"),
    };
    let realcado = cor(&p, 2);
    let normal = cor(&p, 1);
    assert_eq!(
        realcado.background,
        Some(Brush::token(Token::AccentFillColorDefault))
    );
    assert!(realcado.border.is_none());
    assert_eq!(
        normal.background,
        Some(Brush::token(Token::SolidBackgroundFillColorBase))
    );
    assert!(normal.border.is_some());
    assert_eq!(p.app.panes("paineis").highlighted, Some(2));
    // Um índice que não existe não faz nada.
    p.destaque.set(Some(9));
    p.app.frame();
    assert_eq!(p.app.panes("paineis").highlighted, None);
    assert_eq!(crachas(&p.app, 4).len(), 4);
    for n in 0..4 {
        assert_eq!(cor(&p, n), normal, "painel {n}");
    }
}

#[test]
fn com_zoom_so_o_painel_ampliado_tem_cracha() {
    let mut p = Paineis::grade("zoom", 2, 2, 900, 700);
    p.identificar.set(true);
    p.layout.update(|l| l.zoom(1));
    p.app.frame();
    p.app.frame();
    assert_eq!(p.app.panes("paineis").zoomed, Some(1));
    assert_eq!(crachas(&p.app, 4), ["2"]);
}

#[test]
fn painel_pequeno_demais_fica_sem_cracha_e_sem_panico() {
    let mut p = Paineis::grade("peq", 3, 3, 300, 300);
    p.identificar.set(true);
    p.app.frame();
    p.app.frame();
    assert!(crachas(&p.app, 9).is_empty());
    // Num painel só, a janela cabe o crachá.
    p.layout.update(|l| l.single());
    p.app.frame();
    p.app.frame();
    assert_eq!(crachas(&p.app, 1), ["1"]);
}

#[test]
fn um_menu_aberto_fica_acima_do_cracha() {
    let mut p = Paineis::grade("menu", 2, 2, 900, 700);
    p.identificar.set(true);
    p.app.frame();
    p.app.click("botao");
    assert!(p.app.is_open("menu"));
    assert_eq!(crachas(&p.app, 4).len(), 4);
    p.app.assert_lint();
}

#[test]
fn snapshot_do_exemplo_identificando_nos_dois_temas() {
    for (tema, nome) in [(Theme::Light, "claro"), (Theme::Dark, "escuro")] {
        let mut app = TestApp::new(terminal_panes::identificando)
            .theme(tema)
            .size(900, 640);
        app.frame();
        app.assert_lint();
        app.assert_snapshot(&format!("m36_identificando_{nome}"));
        for chave in ["web1", "web2", "web3", "web4"] {
            Session::find(chave).unwrap().dispose();
        }
    }
}
