//! Marco M7: `CheckBox`, `ToggleSwitch`, `InfoBar` e `.visible()` pelo `TestApp`, e a regra
//! `missing-glyph` do lint.

use tomni_ui::Key;
use tomni_ui::lint::{Rule, Severity};
use tomni_ui::prelude::*;
use tomni_ui_test::TestApp;

/// Pedaço do cadastro: os termos habilitam "Criar conta"; a chave não depende de nada.
fn termos() -> impl View {
    let aceito = signal(false);
    let novidades = signal(false);
    Form::new("Cadastro").child(
        Column::new()
            .padding(16)
            .gap(12)
            .child(
                CheckBox::bind(aceito, "Li e aceito os termos")
                    .id("termos")
                    .height(44),
            )
            .child(
                ToggleSwitch::bind(novidades)
                    .header("Receber novidades por e-mail")
                    .id("novidades"),
            )
            .child(
                Button::new("Criar conta")
                    .id("criar")
                    .accent()
                    .height(44)
                    .disabled(move || !aceito.get()),
            ),
    )
}

#[test]
fn clique_e_espaco_alternam() {
    let mut app = TestApp::new(termos).size(390, 400);
    assert!(!app.checked("termos"));
    assert!(!app.enabled("criar"));

    app.click("termos");
    assert!(app.checked("termos"));
    assert!(app.enabled("criar"));
    app.click("termos");
    assert!(!app.checked("termos"));

    app.click("novidades");
    assert!(app.checked("novidades"));

    // Foco pelo teclado: Espaço alterna, Enter não (como no WinUI).
    app.tab();
    assert_eq!(app.focused().as_deref(), Some("termos"));
    app.key(Key::Space);
    assert!(app.checked("termos"));
    app.key(Key::Enter);
    assert!(app.checked("termos"));

    app.tab();
    assert_eq!(app.focused().as_deref(), Some("novidades"));
    app.key(Key::Space);
    assert!(!app.checked("novidades"));
    app.assert_lint();
}

#[test]
fn desabilitado_nao_recebe_foco_nem_clique() {
    let mut app = TestApp::new(|| {
        let a = signal(true);
        let b = signal(false);
        Form::new("Desabilitados").child(
            Column::new()
                .padding(16)
                .child(CheckBox::bind(a, "Bloqueada").id("caixa").disabled(true))
                .child(ToggleSwitch::bind(b).id("chave").disabled(true))
                .child(Button::new("OK").id("ok").height(44)),
        )
    });
    assert!(!app.enabled("caixa"));
    assert!(!app.enabled("chave"));
    app.tab();
    assert_eq!(app.focused().as_deref(), Some("ok"));

    // As ações encadeiam, como o `type_into`.
    app.click("caixa").click("chave");
    assert!(app.checked("caixa"));
    assert!(!app.checked("chave"));
}

#[test]
fn oculto_sai_do_tab_do_layout_e_do_lint() {
    let mut app = TestApp::new(|| {
        let detalhes = signal(false);
        Form::new("Oculto").child(
            Column::new()
                .padding(16)
                .gap(12)
                .child(
                    CheckBox::bind(detalhes, "Mostrar detalhes")
                        .id("mostrar")
                        .height(44),
                )
                // Pequeno demais para o touch-target, mas o lint só o vê quando aparece.
                .child(Button::new("Detalhes").id("detalhes").visible(detalhes))
                .child(Button::new("OK").id("ok").height(44)),
        )
    });
    assert!(!app.visible("detalhes"));
    assert!(app.rect("detalhes").is_zero_area());
    let ok = app.rect("ok");
    assert_eq!(
        ok.y0,
        app.rect("mostrar").y1 + 12.0,
        "o oculto não ocupa espaço"
    );
    app.tab();
    app.tab();
    assert_eq!(app.focused().as_deref(), Some("ok"), "o Tab pula o oculto");
    app.assert_lint();

    app.click("mostrar");
    assert!(app.visible("detalhes"));
    assert!(app.rect("ok").y0 > ok.y0, "o OK desce para dar lugar");
    assert_eq!(
        app.focused().as_deref(),
        Some("mostrar"),
        "o clique foca a caixa"
    );
    app.tab();
    assert_eq!(app.focused().as_deref(), Some("detalhes"));
    let issues = app.lint();
    assert!(
        issues
            .iter()
            .any(|i| i.rule == Rule::TouchTarget && i.widget == "detalhes"),
        "visível, o lint volta a vê-lo: {issues:?}"
    );
}

#[test]
#[should_panic(expected = "está oculto")]
fn clicar_no_oculto_falha() {
    let mut app = TestApp::new(|| {
        Form::new("Oculto").child(
            Column::new()
                .padding(16)
                .child(Button::new("Sumiu").id("sumiu").visible(false)),
        )
    });
    app.click("sumiu");
}

#[test]
fn marcar_repinta_so_quem_depende() {
    let mut app = TestApp::new(termos).size(390, 400);
    assert!(app.repaints_since_last().is_empty());
    app.click("termos");
    assert_eq!(app.repaints_since_last(), ["termos", "criar"]);
    app.click("novidades");
    // Sair da caixa tira o hover dela.
    assert_eq!(app.repaints_since_last(), ["termos", "novidades"]);
}

#[test]
fn fechar_a_info_bar() {
    let mut app = TestApp::new(|| {
        let aberta = signal(true);
        Form::new("Aviso").child(
            Column::new().padding(16).gap(12).child(
                InfoBar::new(InfoBarSeverity::Error)
                    .id("erro")
                    .title("Erro")
                    .message("Este e-mail já está cadastrado")
                    .visible(aberta)
                    .on_close(move || aberta.set(false)),
            ),
        )
    })
    .size(390, 200);
    assert!(app.visible("erro"));
    assert_eq!(app.text("erro"), "Este e-mail já está cadastrado");
    app.assert_lint();
    app.click("erro");
    assert!(!app.visible("erro"));
    assert!(!app.tree().children[0].children[0].visible);
}

fn texto(text: &'static str) -> TestApp {
    TestApp::new(move || {
        Form::new("Glifos").child(Column::new().padding(16).child(Text::new(text).id("texto")))
    })
    .size(390, 120)
}

fn missing_glyph(app: &mut TestApp) -> Vec<String> {
    app.lint()
        .into_iter()
        .filter(|i| i.rule == Rule::MissingGlyph)
        .inspect(|i| assert_eq!(i.severity, Severity::Error))
        .map(|i| i.message)
        .collect()
}

#[test]
fn portugues_acentuado_nao_falta() {
    let mut app = texto("Ação, coração, pão, avô, à beça: é pé, ü e Ç");
    assert_eq!(missing_glyph(&mut app), Vec::<String>::new());
}

#[test]
fn emoji_falta() {
    let mut app = texto("Conta criada 🎉");
    let issues = missing_glyph(&mut app);
    assert_eq!(issues.len(), 1, "{issues:?}");
    assert!(issues[0].contains("🎉"), "{issues:?}");
}

#[cfg(not(feature = "cjk"))]
#[test]
fn sinal_de_menos_falta() {
    let mut app = texto("Saldo: −10");
    let issues = missing_glyph(&mut app);
    assert_eq!(issues.len(), 1, "{issues:?}");
    assert!(issues[0].contains("U+2212"), "{issues:?}");
}
