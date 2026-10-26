//! Marco M7: `CheckBox` nos 5 estados, marcada e desmarcada, nos 2 temas.

mod comum;

use comum::{State, theme_name};
use tomni_ui::prelude::*;
use tomni_ui::theme::Theme;
use tomni_ui_test::TestApp;

fn app(theme: Theme, checked: bool, state: State) -> TestApp {
    let view = move || {
        let aceito = signal(checked);
        Form::new("Caixa").child(
            Column::new().padding(12).child(
                CheckBox::bind(aceito, "Li e aceito os termos")
                    .id("termos")
                    .disabled(state == State::Disabled),
            ),
        )
    };
    let mut app = TestApp::new(view).size(220, 56).theme(theme);
    state.apply(&mut app, "termos");
    assert_eq!(app.checked("termos"), checked);
    app
}

fn check(theme: Theme) {
    for checked in [false, true] {
        let marked = if checked { "marcada" } else { "desmarcada" };
        for state in State::ALL {
            app(theme, checked, state).assert_snapshot(&format!(
                "checkbox_{}_{marked}_{}",
                state.name(),
                theme_name(theme)
            ));
        }
    }
}

#[test]
fn checkbox_claro() {
    check(Theme::Light);
}

#[test]
fn checkbox_escuro() {
    check(Theme::Dark);
}
