//! `set_strings` (pedido pelo M43 do Tirech SSH): os textos próprios da lib saem na tabela da
//! thread, em português por padrão. Cada teste roda na sua thread, e o `thread_local` isola.

use tomni_ui::prelude::*;
use tomni_ui::terminal::{Loopback, PaneLayout, TerminalPanes};
use tomni_ui::theme::Theme;
use tomni_ui::{Key, Modifiers, SpinButtons, Strings, set_strings};
use tomni_ui_test::TestApp;

/// Troca os textos da lib para o inglês, com um marcador que não existe em português.
fn em_ingles() {
    let mut s = Strings::default();
    s.tab_close = "Close tab".into();
    s.tab_add = "New tab".into();
    s.tab_scroll_left = "Scroll left".into();
    s.tab_scroll_right = "Scroll right".into();
    s.back = "Back".into();
    s.navigation_close = "Close navigation".into();
    s.navigation_open = "Open navigation".into();
    s.navigation_search = "Search".into();
    s.navigation_settings = "Settings".into();
    s.minimize = "Minimize".into();
    s.maximize = "Maximize".into();
    s.restore = "Restore".into();
    s.close = "Close".into();
    s.show_password = "Show password".into();
    s.increase = "Increase".into();
    s.decrease = "Decrease".into();
    s.clear = "Clear".into();
    s.sort_alphabetical = "Alphabetical order".into();
    s.paste_title = "Warning".into();
    s.paste_warning = "You are about to paste several lines.".into();
    s.paste_anyway = "Paste anyway".into();
    s.cancel = "Cancel".into();
    s.pane_empty = "Drag a tab here".into();
    s.pane_close = "Close the pane".into();
    s.pane_name = "Pane {n}".into();
    s.pane_move_to = "Move to pane".into();
    s.pane_new_right = "New pane to the right".into();
    s.pane_new_below = "New pane below".into();
    s.pane_restart = "Restart session".into();
    s.pane_close_tab = "Close tab".into();
    s.connection_closed = "[connection closed]".into();
    s.connection_closed_reason = "[connection closed: {reason}]".into();
    s.page_failed = "Couldn't open the page".into();
    s.try_again = "Try again".into();
    s.query = "Search".into();
    s.toggle_on = "On".into();
    s.toggle_off = "Off".into();
    s.tab_progress_working = "working".into();
    s.tab_progress_paused = "paused".into();
    s.tab_progress_error = "error".into();
    s.tab_attention = "needs attention".into();
    s.terminal_copy = "Copy".into();
    s.terminal_paste = "Paste".into();
    s.terminal_link_open = "Open link".into();
    s.terminal_link_copy = "Copy link".into();
    s.terminal_link_hint = "Ctrl+click: {action}".into();
    s.terminal_link_no_open =
        "This link can't be opened from here; use Copy link in the menu".into();
    set_strings(s);
}

/// O texto do nó `id`: `text` ou, se não houver, `label`.
fn texto(app: &TestApp, id: &str) -> String {
    let arvore = app.tree();
    let no = arvore.find(id).unwrap_or_else(|| panic!("sem {id:?}"));
    no.text
        .clone()
        .or_else(|| no.label.clone())
        .unwrap_or_default()
}

#[test]
fn sem_set_strings_os_textos_continuam_em_portugues() {
    let s = Strings::default();
    assert_eq!(s.tab_close, "Fechar aba");
    assert_eq!(s.connection_closed_reason, "[conexão encerrada: {reason}]");
    let app = TestApp::new(|| {
        Form::new("Padrão").child(
            Column::new().padding(16).child(
                NumberBox::bind(signal(1.0))
                    .id("q")
                    .spin_buttons(SpinButtons::Inline)
                    .height(44),
            ),
        )
    })
    .size(390, 400);
    assert_eq!(texto(&app, "q-aumentar"), "Aumentar");
    assert_eq!(texto(&app, "q-diminuir"), "Diminuir");
}

#[test]
fn campos_em_ingles() {
    em_ingles();
    let senha = signal("segredo".to_owned());
    let mut app = TestApp::new(move || {
        Form::new("Campos").child(
            Column::new()
                .padding(16)
                .gap(8)
                .child(
                    NumberBox::bind(signal(1.0))
                        .id("q")
                        .spin_buttons(SpinButtons::Inline)
                        .height(44),
                )
                .child(Edit::bind(senha).id("senha").password().height(44)),
        )
    })
    .size(390, 400);
    assert_eq!(texto(&app, "q-aumentar"), "Increase");
    assert_eq!(texto(&app, "q-diminuir"), "Decrease");
    app.click("senha");
    assert_eq!(texto(&app, "senha-revelar"), "Show password");
    app.assert_lint();
}

#[test]
fn abas_e_barra_de_titulo_em_ingles() {
    em_ingles();
    let aba = signal("a".to_owned());
    let pagina = signal("inicio".to_owned());
    let mut app = TestApp::new(move || {
        Form::new("Abas")
            .title_bar(TitleBar::new("Abas").back(true))
            .child(
                Column::new()
                    .gap(8)
                    .child(
                        TabView::bind(aba)
                            .id("abas")
                            .height(120)
                            .on_add(|| {})
                            .on_close(|_| {})
                            .tab("a", "A", Text::new("um"))
                            .tab("b", "B", Text::new("dois")),
                    )
                    .child(
                        NavigationView::bind(pagina)
                            .id("nav")
                            .grow(1)
                            .item("inicio", Symbol::Home, "Início", Text::new("x"))
                            .settings(Text::new("config")),
                    ),
            )
    })
    .size(1280, 800);
    assert_eq!(texto(&app, "barra-de-titulo-voltar"), "Back");
    assert_eq!(texto(&app, "abas-adicionar"), "New tab");
    assert_eq!(texto(&app, "abas-a-fechar"), "Close tab");
    assert_eq!(app.label("nav-configuracoes"), "Settings");
    app.assert_lint();
}

#[test]
fn property_grid_em_ingles() {
    em_ingles();
    let app = TestApp::new(|| {
        Form::new("Propriedades").child(
            PropertyGrid::new()
                .id("props")
                .grow(1)
                .category("Geral")
                .text("Nome", signal("x".to_owned())),
        )
    })
    .size(600, 400);
    assert_eq!(app.label("props-alfabetica"), "Alphabetical order");
}

#[test]
fn paineis_do_terminal_em_ingles() {
    em_ingles();
    let layout = signal(PaneLayout::new());
    let mut app = TestApp::new(move || {
        Form::new("Painéis").child(TerminalPanes::bind(layout).id("paineis").grow(1))
    })
    .size(800, 500)
    .theme(Theme::Light);
    let arvore = app.tree();
    let textos: Vec<String> = flat(&arvore)
        .into_iter()
        .filter_map(|n| n.text.clone().or_else(|| n.label.clone()))
        .collect();
    assert!(textos.iter().any(|t| t == "Drag a tab here"), "{textos:?}");
    app.assert_lint();
}

/// Todos os nós da árvore, em ordem.
fn flat(no: &tomni_ui::Node) -> Vec<&tomni_ui::Node> {
    let mut out = vec![no];
    for filho in &no.children {
        out.extend(flat(filho));
    }
    out
}

#[test]
fn aviso_de_colar_e_conexao_encerrada_em_ingles() {
    em_ingles();
    let conexao = Loopback::new();
    let handle = conexao.handle();
    let mut app = TestApp::new(move || {
        Form::new("Console").child(Terminal::new(conexao).id("console").cols_rows(80, 24))
    })
    .size(780, 560);
    app.click("console");
    app.clipboard().write("cd /tmp\nrm -rf *\n".into());
    app.key_with(Key::Char('v'), Modifiers::CTRL.with(Modifiers::SHIFT));
    assert!(app.is_open("console-paste"));
    assert_eq!(
        app.tree().find("console-paste").unwrap().title.as_deref(),
        Some("Warning")
    );
    assert_eq!(
        texto(&app, "console-paste-mensagem"),
        "You are about to paste several lines."
    );
    assert_eq!(app.label("console-paste-close"), "Cancel");
    app.click("console-paste-close");
    handle.close(Some("refused".into()));
    app.frame();
    assert!(
        app.terminal_text("console")
            .ends_with("[connection closed: refused]")
    );
}

#[test]
fn o_menu_do_link_do_terminal_em_ingles() {
    em_ingles();
    let conexao = Loopback::new();
    let mut app = TestApp::new(move || {
        Form::new("Console").child(Terminal::new(conexao).id("console").cols_rows(80, 24))
    })
    .size(780, 560);
    app.feed("console", "see https://example.com now\r\n");
    let ponto = app.rect("console").origin() + tomni_ui::kurbo::Vec2::new(60.0, 24.0);
    app.move_to(ponto);
    app.ui().secondary_down(ponto);
    app.frame();
    assert_eq!(app.label("console-link-abrir"), "Open link");
    assert_eq!(app.label("console-link-copiar"), "Copy link");
    assert_eq!(app.label("console-menu-copiar"), "Copy");
    assert_eq!(app.label("console-menu-colar"), "Paste");
    app.assert_lint();
}

#[test]
fn a_chave_liga_desliga_em_ingles() {
    em_ingles();
    let app = TestApp::new(|| {
        Form::new("Chave").child(
            Column::new()
                .padding(16)
                .gap(8)
                .child(ToggleSwitch::bind(signal(true)).id("ligada"))
                .child(ToggleSwitch::bind(signal(false)).id("desligada")),
        )
    })
    .size(240, 160);
    app.assert_snapshot("strings_toggle_switch_ingles");
}
