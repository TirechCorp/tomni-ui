//! Marco M7: `ToggleSwitch` nos 5 estados, ligado e desligado, nos 2 temas.

mod comum;

use comum::{State, theme_name};
use tomni_ui::prelude::*;
use tomni_ui::theme::Theme;
use tomni_ui_test::TestApp;

fn app(theme: Theme, on: bool, state: State) -> TestApp {
    let view = move || {
        let novidades = signal(on);
        Form::new("Chave").child(
            Column::new().padding(12).child(
                ToggleSwitch::bind(novidades)
                    .header("Notificações")
                    .id("novidades")
                    .disabled(state == State::Disabled),
            ),
        )
    };
    let mut app = TestApp::new(view).size(200, 88).theme(theme);
    state.apply(&mut app, "novidades");
    assert_eq!(app.checked("novidades"), on);
    app
}

fn check(theme: Theme) {
    for on in [false, true] {
        let value = if on { "ligado" } else { "desligado" };
        for state in State::ALL {
            app(theme, on, state).assert_snapshot(&format!(
                "toggle_switch_{}_{value}_{}",
                state.name(),
                theme_name(theme)
            ));
        }
    }
}

#[test]
fn toggle_switch_claro() {
    check(Theme::Light);
}

#[test]
fn toggle_switch_escuro() {
    check(Theme::Dark);
}
