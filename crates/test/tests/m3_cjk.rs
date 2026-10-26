//! Marco M3: IME de japonês com a Noto Sans JP. Só roda com `--features cjk`.

#![cfg(feature = "cjk")]

use tomni_ui::prelude::*;
use tomni_ui::theme::Theme;
use tomni_ui_test::TestApp;

fn campo() -> impl View {
    let texto = signal(String::new());
    Form::new("Campo").child(
        Column::new()
            .padding(8)
            .child(Edit::bind(texto).id("campo").placeholder("名前")),
    )
}

#[test]
fn ime_japones() {
    let mut app = TestApp::new(campo).size(200, 48);
    app.type_into("campo", "名前:");

    // Composição em hiragana, sublinhada, antes da conversão.
    app.ime_preedit("にほんご");
    let visual = app.edit("campo");
    assert_eq!(visual.text, "名前:にほんご");
    assert_eq!(visual.composing, Some(("名前:".len(), visual.text.len())));
    assert_eq!(app.text("campo"), "名前:");
    app.assert_snapshot("edit_ime_cjk_claro");

    app.ime_commit("日本語");
    assert_eq!(app.text("campo"), "名前:日本語");
    assert_eq!(app.edit("campo").composing, None);

    // O cursor anda um ideograma por vez.
    app.key(tomni_ui::Key::Left);
    assert_eq!(app.edit("campo").caret, Some("名前:日本".len()));
}

#[test]
fn ime_japones_escuro() {
    let mut app = TestApp::new(campo).size(200, 48).theme(Theme::Dark);
    app.click("campo");
    app.ime_preedit("にほんご");
    app.assert_snapshot("edit_ime_cjk_escuro");
}
