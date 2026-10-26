//! Marco M7: `Text` na rampa tipográfica e num parágrafo que quebra linhas, nos 2 temas.

mod comum;

use comum::theme_name;
use tomni_ui::prelude::*;
use tomni_ui::theme::Theme;
use tomni_ui_test::TestApp;

fn rampa() -> impl View {
    Form::new("Rampa").child(
        Column::new()
            .padding(12)
            .gap(8)
            .child(Text::new("Título").title().id("titulo"))
            .child(Text::new("Subtítulo").subtitle())
            .child(Text::new("Corpo em destaque").ramp(TextRamp::BodyStrong))
            .child(Text::new("Corpo do texto, com acentuação"))
            .child(Text::new("Legenda pequena").caption()),
    )
}

fn paragrafo() -> impl View {
    Form::new("Parágrafo").child(
        Column::new().padding(12).child(
            Text::new(
                "Ao criar a conta, você concorda com os termos de uso e com a política de \
                 privacidade. Pode cancelar quando quiser, sem custo.",
            )
            .wrap()
            .id("paragrafo"),
        ),
    )
}

fn check(theme: Theme) {
    let name = theme_name(theme);
    TestApp::new(rampa)
        .size(320, 176)
        .theme(theme)
        .assert_snapshot(&format!("text_rampa_{name}"));
    TestApp::new(paragrafo)
        .size(240, 120)
        .theme(theme)
        .assert_snapshot(&format!("text_paragrafo_{name}"));
}

#[test]
fn text_claro() {
    check(Theme::Light);
}

#[test]
fn text_escuro() {
    check(Theme::Dark);
}
