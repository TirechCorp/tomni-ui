//! Marco M12: `ToggleButton` nos 5 estados, ligado e desligado, nos 2 temas, e o
//! comportamento pelo `TestApp`.

mod comum;

use comum::{State, theme_name};
use tomni_ui::Key;
use tomni_ui::prelude::*;
use tomni_ui::theme::Theme;
use tomni_ui_test::TestApp;

fn app(theme: Theme, on: bool, state: State) -> TestApp {
    let view = move || {
        let negrito = signal(on);
        Form::new("Botão liga/desliga").child(
            Column::new().padding(12).child(
                ToggleButton::bind(negrito, "Negrito")
                    .id("negrito")
                    .height(44)
                    .disabled(state == State::Disabled),
            ),
        )
    };
    let mut app = TestApp::new(view).size(160, 68).theme(theme);
    state.apply(&mut app, "negrito");
    assert_eq!(app.checked("negrito"), on);
    app
}

fn check(theme: Theme) {
    for on in [false, true] {
        let value = if on { "ligado" } else { "desligado" };
        for state in State::ALL {
            app(theme, on, state).assert_snapshot(&format!(
                "toggle_button_{}_{value}_{}",
                state.name(),
                theme_name(theme)
            ));
        }
    }
}

#[test]
fn toggle_button_claro() {
    check(Theme::Light);
}

#[test]
fn toggle_button_escuro() {
    check(Theme::Dark);
}

fn editor(theme: Theme) -> TestApp {
    TestApp::new(|| {
        let negrito = signal(false);
        let italico = signal(true);
        Form::new("Editor").child(
            Column::new()
                .padding(24)
                .gap(12)
                .child(
                    Row::new()
                        .gap(8)
                        .child(
                            ToggleButton::bind(negrito, "Negrito")
                                .id("negrito")
                                .height(44),
                        )
                        .child(
                            ToggleButton::bind(italico, "Itálico")
                                .id("italico")
                                .height(44),
                        ),
                )
                .child(
                    Text::new(move || {
                        format!(
                            "Negrito {}, itálico {}.",
                            if negrito.get() { "ligado" } else { "desligado" },
                            if italico.get() { "ligado" } else { "desligado" },
                        )
                    })
                    .id("resumo"),
                ),
        )
    })
    .theme(theme)
}

#[test]
fn clique_enter_e_espaco_alternam_o_signal() {
    let mut app = editor(Theme::Light);
    assert!(!app.checked("negrito"));
    assert!(app.checked("italico"));
    app.click("negrito");
    assert!(app.checked("negrito"));
    assert_eq!(app.text("resumo"), "Negrito ligado, itálico ligado.");
    assert_eq!(
        app.focused().as_deref(),
        Some("negrito"),
        "o clique dá o foco"
    );
    app.key(Key::Enter);
    assert!(!app.checked("negrito"), "Enter alterna, como no botão");
    app.key(Key::Space);
    assert!(app.checked("negrito"), "Espaço também");
    app.tab().key(Key::Space);
    assert!(!app.checked("italico"));
    assert_eq!(app.text("resumo"), "Negrito ligado, itálico desligado.");
}

#[test]
fn arvore_mostra_o_estado_e_o_lint_passa() {
    for theme in [Theme::Light, Theme::Dark] {
        let mut app = editor(theme);
        app.click("italico");
        let tree = app.tree();
        let node = tree.find("italico").expect("o botão está na árvore");
        assert_eq!(node.kind, "toggle_button");
        assert_eq!(node.label.as_deref(), Some("Itálico"));
        assert_eq!(node.checked, Some(false));
        assert_eq!(app.label("negrito"), "Negrito");
        // Ligado, o rótulo fica no fundo de destaque: o lint confere o contraste.
        app.click("negrito");
        app.assert_lint();
    }
}
