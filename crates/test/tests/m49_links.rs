//! Marco M49 (pedido pelo Tirech SSH), os links do terminal: o sublinhado e a mão com o Ctrl, a
//! dica do `OSC 8`, o Ctrl+clique, `links(false)`, `link_action` e o menu do clique direito.

mod comum;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use comum::theme_name;
use tomni_ui::kurbo::Point;
use tomni_ui::prelude::*;
use tomni_ui::terminal::{LinkHit, LoopbackHandle, TerminalVisual};
use tomni_ui::theme::Theme;
use tomni_ui::{Cursor, Look, Modifiers, TOOLTIP_DELAY};
use tomni_ui_test::TestApp;

const URL: &str = "https://exemplo.com/a?b=1";

/// O que o `on_link` recebeu.
type Aberto = Rc<RefCell<Vec<LinkHit>>>;

/// Um terminal de 80 × 24 com `Loopback`; o texto das duas primeiras linhas tem uma URL no
/// meio da linha e um link do `OSC 8` (`file:` com espaço) no começo da seguinte.
fn app_com(
    configurar: impl FnOnce(Terminal) -> Terminal + 'static,
    tema: Theme,
) -> (TestApp, LoopbackHandle, Aberto) {
    let conexao = Loopback::new();
    let handle = conexao.handle();
    let aberto = Aberto::default();
    let destino = aberto.clone();
    let mut app = TestApp::new(move || {
        let terminal = Terminal::new(conexao)
            .id("console")
            .cols_rows(80, 24)
            .paste_warning(false)
            .on_link(move |link| destino.borrow_mut().push(link.clone()));
        Form::new("Console").child(
            Column::new()
                .padding(16)
                .gap(8)
                .child(configurar(terminal))
                .child(Button::new("Depois").id("depois").height(44)),
        )
    })
    .size(780, 560)
    .theme(tema);
    app.feed(
        "console",
        format!(
            "veja {URL} fim\r\n\x1b]8;;file:///tmp/x%20y\x1b\\arquivo\x1b]8;;\x1b\\ resto\r\nsem link aqui\r\n"
        ),
    );
    (app, handle, aberto)
}

fn tela(tema: Theme) -> (TestApp, LoopbackHandle, Aberto) {
    app_com(|t| t, tema)
}

fn visual(app: &TestApp) -> TerminalVisual {
    match app.look("console") {
        Look::Custom(custom) => custom
            .downcast::<TerminalVisual>()
            .expect("o visual do terminal")
            .clone(),
        outro => panic!("não é o terminal: {outro:?}"),
    }
}

/// Centro da célula (`col`, `linha`) na janela.
fn celula(app: &TestApp, col: usize, linha: usize) -> Point {
    visual(app)
        .cell_rect(app.rect("console"), linha, col, 1)
        .center()
}

fn sublinhado(app: &TestApp) -> Option<(usize, bool)> {
    visual(app)
        .link()
        .map(|(celulas, continuo)| (celulas.len(), continuo))
}

fn texto_da_dica(app: &TestApp) -> String {
    let arvore = app.tree();
    let no = arvore.find("console-tooltip").expect("a dica do terminal");
    no.children
        .first()
        .and_then(|texto| texto.text.clone())
        .unwrap_or_default()
}

fn com_ctrl(app: &mut TestApp, col: usize, linha: usize) {
    let ponto = celula(app, col, linha);
    app.ui().set_modifiers(Modifiers::CTRL);
    app.move_to(ponto);
}

fn solta_ctrl(app: &mut TestApp) {
    app.ui().set_modifiers(Modifiers::NONE);
}

fn clique_direito(app: &mut TestApp, col: usize, linha: usize) {
    let ponto = celula(app, col, linha);
    app.move_to(ponto);
    app.ui().secondary_down(ponto);
    app.frame();
}

#[test]
fn ctrl_sobre_a_url_detectada_sublinha_e_mostra_a_mao() {
    let (mut app, _, _) = tela(Theme::Light);
    let na_url = celula(&app, 10, 0);
    app.move_to(na_url);
    assert_eq!(
        sublinhado(&app),
        None,
        "sem o Ctrl a URL do texto fica quieta"
    );
    assert_ne!(app.ui().cursor(na_url), Cursor::Hand);
    com_ctrl(&mut app, 10, 0);
    assert_eq!(sublinhado(&app), Some((URL.len(), true)));
    assert_eq!(app.ui().cursor(na_url), Cursor::Hand);
    // Soltar o Ctrl (um movimento adiante) apaga.
    solta_ctrl(&mut app);
    app.move_to(celula(&app, 11, 0));
    assert_eq!(sublinhado(&app), None);
    // Fora da URL, com o Ctrl, nada.
    com_ctrl(&mut app, 1, 0);
    assert_eq!(sublinhado(&app), None);
    let fora = celula(&app, 1, 0);
    assert_ne!(app.ui().cursor(fora), Cursor::Hand);
}

#[test]
fn o_osc_8_sublinha_pontilhado_ao_passar_e_mostra_o_destino_na_dica() {
    let (mut app, _, _) = tela(Theme::Light);
    let no_link = celula(&app, 2, 1);
    app.move_to(no_link);
    assert_eq!(sublinhado(&app), Some(("arquivo".len(), false)));
    // Sem o Ctrl não vira mão: a seleção continua como sempre.
    assert_ne!(app.ui().cursor(no_link), Cursor::Hand);
    assert!(!app.is_open("console-tooltip"), "a dica espera");
    app.wait(TOOLTIP_DELAY + Duration::from_millis(10));
    assert!(app.is_open("console-tooltip"));
    let dica = texto_da_dica(&app);
    let (primeira, segunda) = dica.split_once('\n').expect("duas linhas");
    assert_eq!(primeira, "file:///tmp/x%20y");
    // Sem `link_action`, o `file:` não abre daqui.
    assert_eq!(segunda, "Este link não abre daqui; use Copiar link no menu");
    com_ctrl(&mut app, 2, 1);
    assert_eq!(sublinhado(&app), Some(("arquivo".len(), true)));
}

#[test]
fn a_dica_diz_o_que_o_ctrl_clique_faz() {
    let (mut app, _, _) = app_com(
        |t| t.link_action(|_| Some("Abrir no navegador".into())),
        Theme::Light,
    );
    app.move_to(celula(&app, 2, 1));
    app.wait(TOOLTIP_DELAY + Duration::from_millis(10));
    assert_eq!(
        texto_da_dica(&app),
        "file:///tmp/x%20y\nCtrl+clique: Abrir no navegador"
    );
}

#[test]
fn a_dica_corta_o_endereco_comprido_no_meio() {
    let conexao = Loopback::new();
    let longo = format!("https://exemplo.com/{}/fim", "caminho/".repeat(30));
    let mut app = TestApp::new(move || {
        Form::new("Console").child(Terminal::new(conexao).id("console").cols_rows(100, 5))
    })
    .size(900, 300);
    app.feed(
        "console",
        format!("\x1b]8;;{longo}\x1b\\link\x1b]8;;\x1b\\\r\n"),
    );
    app.move_to(celula(&app, 1, 0));
    app.wait(TOOLTIP_DELAY + Duration::from_millis(10));
    let dica = texto_da_dica(&app);
    let primeira = dica.lines().next().unwrap();
    assert!(primeira.contains('…'), "{primeira}");
    assert!(primeira.starts_with("https://exemplo.com/"));
    assert!(primeira.ends_with("/fim"));
    assert!(primeira.chars().count() < longo.chars().count());
}

#[test]
fn ctrl_clique_chama_on_link_com_o_endereco_inteiro_e_nao_seleciona() {
    let (mut app, handle, aberto) = tela(Theme::Light);
    app.ui().set_modifiers(Modifiers::CTRL);
    app.click_at(celula(&app, 10, 0), 1);
    solta_ctrl(&mut app);
    let recebidos = aberto.borrow();
    assert_eq!(recebidos.len(), 1);
    assert_eq!(recebidos[0].uri, URL);
    assert!(!recebidos[0].explicit);
    drop(recebidos);
    assert_eq!(visual(&app).selection(), None, "o clique não seleciona");
    assert!(handle.take_written().is_empty(), "e não vai ao programa");
    // No link do OSC 8, o `file:` também chega (sem `link_action`, o padrão só abre http/https).
    aberto.borrow_mut().clear();
    app.ui().set_modifiers(Modifiers::CTRL);
    app.click_at(celula(&app, 2, 1), 1);
    solta_ctrl(&mut app);
    assert!(
        aberto.borrow().is_empty(),
        "o `file:` não abre sem `link_action`"
    );
}

#[test]
fn ctrl_clique_no_osc_8_com_link_action_chama_on_link() {
    let (mut app, _, aberto) = app_com(
        |t| t.link_action(|_| Some("Abrir a pasta".into())),
        Theme::Light,
    );
    app.ui().set_modifiers(Modifiers::CTRL);
    app.click_at(celula(&app, 2, 1), 1);
    solta_ctrl(&mut app);
    let recebidos = aberto.borrow();
    assert_eq!(recebidos.len(), 1);
    assert_eq!(recebidos[0].uri, "file:///tmp/x%20y");
    assert!(recebidos[0].explicit);
}

#[test]
fn clicar_sem_o_ctrl_nao_chama_e_seleciona_como_sempre() {
    let (mut app, _, aberto) = tela(Theme::Light);
    app.click_at(celula(&app, 10, 0), 1);
    assert!(aberto.borrow().is_empty());
    // Arrastar sobre o link explícito seleciona.
    app.drag(celula(&app, 0, 1), celula(&app, 5, 1));
    assert_eq!(app.clipboard().read(), None);
    assert!(visual(&app).selection().is_some());
    assert!(aberto.borrow().is_empty());
}

#[test]
fn links_falso_nao_sublinha_nao_chama_e_nao_mostra_a_mao() {
    let (mut app, _, aberto) = app_com(|t| t.links(false), Theme::Light);
    com_ctrl(&mut app, 10, 0);
    assert_eq!(sublinhado(&app), None);
    let ponto = celula(&app, 10, 0);
    assert_ne!(app.ui().cursor(ponto), Cursor::Hand);
    app.move_to(celula(&app, 2, 1));
    assert_eq!(sublinhado(&app), None);
    app.wait(TOOLTIP_DELAY + Duration::from_millis(10));
    assert!(!app.is_open("console-tooltip"));
    app.click_at(celula(&app, 10, 0), 1);
    solta_ctrl(&mut app);
    assert!(aberto.borrow().is_empty());
}

#[test]
fn link_action_none_nao_chama_e_o_menu_so_tem_copiar_link() {
    let (mut app, _, aberto) = app_com(|t| t.link_action(|_| None), Theme::Light);
    app.ui().set_modifiers(Modifiers::CTRL);
    app.click_at(celula(&app, 10, 0), 1);
    solta_ctrl(&mut app);
    assert!(aberto.borrow().is_empty());
    clique_direito(&mut app, 10, 0);
    assert!(app.is_open("console-menu"));
    assert!(!app.visible("console-link-abrir"));
    assert!(app.visible("console-link-copiar"));
}

#[test]
fn o_menu_do_link_abre_e_copia() {
    let (mut app, _, aberto) = tela(Theme::Light);
    clique_direito(&mut app, 10, 0);
    assert!(app.is_open("console-menu"));
    assert!(app.visible("console-link-abrir"));
    assert!(app.visible("console-link-copiar"));
    app.click("console-link-copiar");
    app.frame();
    assert_eq!(app.clipboard().read().as_deref(), Some(URL));
    assert!(aberto.borrow().is_empty());
    clique_direito(&mut app, 10, 0);
    app.click("console-link-abrir");
    assert_eq!(aberto.borrow().len(), 1);
    assert_eq!(aberto.borrow()[0].uri, URL);
}

#[test]
fn com_links_desligados_o_menu_ainda_copia_o_link() {
    let (mut app, _, _) = app_com(|t| t.links(false), Theme::Light);
    clique_direito(&mut app, 2, 1);
    assert!(app.is_open("console-menu"));
    app.click("console-link-copiar");
    app.frame();
    assert_eq!(app.clipboard().read().as_deref(), Some("file:///tmp/x%20y"));
}

#[test]
fn clique_direito_fora_de_link_copia_e_cola_como_antes() {
    let (mut app, handle, _) = app_com(|t| t.copy_on_select(true), Theme::Light);
    app.drag(celula(&app, 0, 2), celula(&app, 2, 2));
    assert_eq!(app.clipboard().read().as_deref(), Some("sem"));
    app.right_click("console");
    assert!(!app.is_open("console-menu"));
    assert_eq!(String::from_utf8(handle.take_written()).unwrap(), "sem");
}

#[test]
fn snapshots_links() {
    for tema in [Theme::Light, Theme::Dark] {
        let nome = theme_name(tema);
        let (mut app, _, _) = tela(tema);
        com_ctrl(&mut app, 10, 0);
        app.assert_snapshot(&format!("terminal-link-ctrl_{nome}"));
        solta_ctrl(&mut app);

        let (mut app, _, _) = tela(tema);
        app.move_to(celula(&app, 2, 1));
        app.wait(TOOLTIP_DELAY + Duration::from_millis(10));
        app.assert_snapshot(&format!("terminal-link-osc8_{nome}"));

        let (mut app, _, _) = tela(tema);
        clique_direito(&mut app, 10, 0);
        app.assert_snapshot(&format!("terminal-link-menu_{nome}"));
    }
}
