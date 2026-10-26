//! Marco M7: `InfoBar` nas 4 severidades, nos 2 temas. A de aviso tem a mensagem longa, que
//! desce para baixo do título; as outras cabem numa linha.

mod comum;

use comum::theme_name;
use tomni_ui::Modifiers;
use tomni_ui::prelude::*;
use tomni_ui::theme::Theme;
use tomni_ui_test::TestApp;

const CASES: [(InfoBarSeverity, &str, &str); 4] = [
    (
        InfoBarSeverity::Informational,
        "Novidade",
        "Há uma versão nova.",
    ),
    (InfoBarSeverity::Success, "Pronto", "Conta criada."),
    (
        InfoBarSeverity::Warning,
        "Conexão instável",
        "As alterações serão salvas quando a conexão voltar.",
    ),
    (
        InfoBarSeverity::Error,
        "Erro",
        "Este e-mail já está cadastrado.",
    ),
];

fn check(theme: Theme) {
    for (severity, title, message) in CASES {
        let view = move || {
            let aberta = signal(true);
            Form::new("Aviso").child(
                Column::new().padding(12).child(
                    InfoBar::new(severity)
                        .id("aviso")
                        .title(title)
                        .message(message)
                        .visible(aberta)
                        .on_close(move || aberta.set(false)),
                ),
            )
        };
        let app = TestApp::new(view).size(390, 124).theme(theme);
        app.assert_snapshot(&format!(
            "info_bar_{}_{}",
            severity.name(),
            theme_name(theme)
        ));
    }
}

#[test]
fn info_bar_claro() {
    check(Theme::Light);
}

#[test]
fn info_bar_escuro() {
    check(Theme::Dark);
}

/// Uma barra com o botão de ação e o de fechar; `acoes` e `fechadas` contam os cliques.
fn com_acao(
    mensagem: &'static str,
    fechavel: bool,
    acoes: std::rc::Rc<std::cell::Cell<u32>>,
    fechadas: std::rc::Rc<std::cell::Cell<u32>>,
) -> impl View {
    let aberta = signal(true);
    let barra = InfoBar::new(InfoBarSeverity::Success)
        .id("aviso")
        .title("Instalado")
        .message(mensagem)
        .visible(aberta)
        .action("Abrir agora", move || acoes.set(acoes.get() + 1));
    let barra = if fechavel {
        barra.on_close(move || {
            fechadas.set(fechadas.get() + 1);
            aberta.set(false);
        })
    } else {
        barra
    };
    Form::new("Aviso").child(Column::new().padding(12).child(barra))
}

fn contadores() -> (
    std::rc::Rc<std::cell::Cell<u32>>,
    std::rc::Rc<std::cell::Cell<u32>>,
) {
    Default::default()
}

#[test]
fn o_botao_de_acao_clica_sem_fechar_a_barra() {
    let (acoes, fechadas) = contadores();
    let (a, f) = (acoes.clone(), fechadas.clone());
    let mut app = TestApp::new(move || com_acao("Claude Code 2.1.211.", true, a, f)).size(560, 124);
    app.click("aviso-acao");
    assert_eq!((acoes.get(), fechadas.get()), (1, 0));
    assert!(app.visible("aviso"));
    // O id da barra continua apontando o botão de fechar.
    app.click("aviso");
    assert_eq!((acoes.get(), fechadas.get()), (1, 1));
    assert!(!app.visible("aviso"));
}

#[test]
fn a_arvore_mostra_o_botao_de_acao() {
    let (acoes, fechadas) = contadores();
    let app = TestApp::new(move || com_acao("Pronto.", true, acoes, fechadas)).size(560, 124);
    let arvore = app.tree();
    let botao = arvore
        .find("aviso-acao")
        .expect("o botão de ação na árvore");
    assert_eq!(botao.kind, "button");
    assert_eq!(botao.label.as_deref(), Some("Abrir agora"));
    assert_eq!(botao.state, Some("rest"));
}

#[test]
fn o_teclado_passa_da_acao_para_o_fechar_e_volta() {
    let (acoes, fechadas) = contadores();
    let (a, f) = (acoes.clone(), fechadas.clone());
    let mut app = TestApp::new(move || com_acao("Pronto.", true, a, f)).size(560, 124);
    app.tab();
    assert_eq!(app.focused().as_deref(), Some("aviso"));
    assert!(app.tree().find("aviso-acao").unwrap().focused);
    app.key(Key::Enter);
    app.key(Key::Space);
    assert_eq!((acoes.get(), fechadas.get()), (2, 0));

    // O Tab seguinte vai ao botão de fechar, na mesma parada da barra.
    app.tab();
    assert_eq!(app.focused().as_deref(), Some("aviso"));
    assert!(!app.tree().find("aviso-acao").unwrap().focused);
    // Shift+Tab volta ao de ação.
    app.key_with(Key::Tab, Modifiers::SHIFT);
    assert!(app.tree().find("aviso-acao").unwrap().focused);
    app.tab();
    app.key(Key::Enter);
    assert_eq!((acoes.get(), fechadas.get()), (2, 1));
}

#[test]
fn so_com_a_acao_a_barra_inteira_recebe_o_ponteiro() {
    let (acoes, fechadas) = contadores();
    let (a, f) = (acoes.clone(), fechadas.clone());
    let mut app = TestApp::new(move || com_acao("Pronto.", false, a, f)).size(560, 124);
    app.click("aviso-acao");
    assert_eq!(acoes.get(), 1);
    app.tab();
    app.key(Key::Enter);
    assert_eq!((acoes.get(), fechadas.get()), (2, 0));
}

fn check_acao(theme: Theme) {
    // Cabe numa linha, e a de baixo é estreita: a mensagem longa desce e leva o botão junto.
    for (nome, largura, altura, mensagem) in [
        ("linha", 560, 124, "Claude Code 2.1.211."),
        (
            "quebrada",
            390,
            176,
            "Claude Code 2.1.211 está pronto para abrir no servidor web-01.",
        ),
    ] {
        let (acoes, fechadas) = contadores();
        let mut app = TestApp::new(move || com_acao(mensagem, true, acoes, fechadas))
            .size(largura, altura)
            .theme(theme);
        app.assert_snapshot(&format!("info_bar_acao_{nome}_{}", theme_name(theme)));
        app.assert_lint();
    }
}

#[test]
fn info_bar_com_acao_claro() {
    check_acao(Theme::Light);
}

#[test]
fn info_bar_com_acao_escuro() {
    check_acao(Theme::Dark);
}
