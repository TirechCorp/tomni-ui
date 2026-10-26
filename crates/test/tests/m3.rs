//! Marco M3: `Edit` com teclado, mouse, seleção, clipboard, undo, rolagem, senha e IME.
//!
//! Os testes passam pelo `Ui` com o `FluentDrawer`: o cursor e o clique usam as medidas da
//! Selawik, como no dispositivo.

use tomni_ui::kurbo::Point;
use tomni_ui::prelude::*;
use tomni_ui::theme::Theme;
use tomni_ui::{EditState, InputPurpose, Key, Modifiers, PASSWORD_CHAR};
use tomni_ui_test::TestApp;

/// Modificador que anda e apaga por palavra: Ctrl, ou Option no macOS.
const WORD: Modifiers = if cfg!(any(target_os = "macos", target_os = "ios")) {
    Modifiers {
        alt: true,
        ..Modifiers::NONE
    }
} else {
    Modifiers {
        ctrl: true,
        ..Modifiers::NONE
    }
};

/// Login da spec: "Entrar" só habilita com um e-mail e uma senha de 6 caracteres ou mais.
fn login() -> impl View {
    let email = signal(String::new());
    let senha = signal(String::new());
    Form::new("Login").child(
        Column::new()
            .gap(12)
            .padding(24)
            .child(Edit::bind(email).id("email").placeholder("E-mail"))
            .child(
                Edit::bind(senha)
                    .id("senha")
                    .placeholder("Senha")
                    .password(),
            )
            .child(
                Button::new("Entrar")
                    .id("entrar")
                    .accent()
                    .disabled(move || {
                        !email.get().contains('@') || senha.get().chars().count() < 6
                    }),
            ),
    )
}

/// Um campo só, com `edit` para ajustar.
fn campo(edit: impl Fn(Signal<String>) -> Edit + 'static) -> impl Fn() -> Form {
    move || {
        let texto = signal(String::new());
        Form::new("Campo").child(Column::new().padding(8).child(edit(texto).id("campo")))
    }
}

/// Ponto no campo `id` a `x` px do começo do texto (borda de 1 px mais padding de 10 px).
fn no_texto(app: &TestApp, id: &str, x: f64) -> Point {
    let rect = app.rect(id);
    Point::new(rect.x0 + 11.0 + x, rect.center().y)
}

#[test]
fn login_valido() {
    let mut app = TestApp::new(login).size(390, 844);
    assert!(!app.enabled("entrar"));

    app.type_into("email", "ana@x.com")
        .type_into("senha", "123456");
    assert!(app.enabled("entrar"));
    // Com texto na senha aparece o botão de revelar dentro do campo.
    assert_eq!(
        app.repaints_since_last(),
        &["email", "senha", "senha-revelar", "entrar"]
    );
    assert_eq!(app.text("email"), "ana@x.com");
    assert_eq!(app.text("senha"), "123456");
    app.assert_snapshot("login_valido");
}

#[test]
fn teclado_e_selecao() {
    let mut app = TestApp::new(campo(Edit::bind));
    app.type_into("campo", "olá mundo");
    assert_eq!(app.edit("campo").caret, Some("olá mundo".len()));

    app.key(Key::Home);
    for _ in 0..3 {
        app.key_with(Key::Right, Modifiers::SHIFT);
    }
    assert_eq!(app.edit("campo").selection, Some((0, "olá".len())));
    app.type_text("oi");
    assert_eq!(app.text("campo"), "oi mundo");
    assert_eq!(app.edit("campo").selection, None);

    // Por palavra: Ctrl+Shift+Direita seleciona até o começo da próxima.
    app.key(Key::Home);
    app.key_with(Key::Right, WORD | Modifiers::SHIFT);
    assert_eq!(app.edit("campo").selection, Some((0, "oi ".len())));
    app.key(Key::End);
    app.key_with(Key::Backspace, WORD);
    assert_eq!(app.text("campo"), "oi ");

    // Esquerda com seleção vai para o começo dela.
    app.shortcut('a');
    assert_eq!(app.edit("campo").selection, Some((0, 3)));
    app.key(Key::Left);
    assert_eq!(app.edit("campo").caret, Some(0));
    assert_eq!(app.edit("campo").selection, None);
}

#[test]
fn clipboard() {
    let mut app = TestApp::new(campo(Edit::bind));
    app.type_into("campo", "copia");
    app.shortcut('a');
    app.shortcut('c');
    assert_eq!(app.clipboard().read().as_deref(), Some("copia"));

    app.key(Key::End);
    app.shortcut('v');
    assert_eq!(app.text("campo"), "copiacopia");

    app.shortcut('a');
    app.shortcut('x');
    assert_eq!(app.text("campo"), "");
    assert_eq!(app.clipboard().read().as_deref(), Some("copiacopia"));

    // Uma linha só: quebras de linha e tabs coladas viram espaço.
    app.clipboard().write("rua A,\r\nnº 10\tfundos".into());
    app.shortcut('v');
    assert_eq!(app.text("campo"), "rua A, nº 10 fundos");
}

#[test]
fn undo_e_redo() {
    let mut app = TestApp::new(campo(Edit::bind));
    app.type_into("campo", "abc");
    app.key(Key::Backspace);
    assert_eq!(app.text("campo"), "ab");

    app.shortcut('z');
    assert_eq!(app.text("campo"), "abc");
    app.shortcut('z');
    assert_eq!(
        app.text("campo"),
        "",
        "a digitação seguida volta de uma vez"
    );
    app.shortcut('z');
    assert_eq!(app.text("campo"), "");

    app.shortcut('y');
    assert_eq!(app.text("campo"), "abc");
    app.key_with(Key::Char('z'), Modifiers::COMMAND | Modifiers::SHIFT);
    assert_eq!(app.text("campo"), "ab");
}

#[test]
fn mouse_seleciona_palavra_e_tudo() {
    let mut app = TestApp::new(campo(Edit::bind));
    app.type_into("campo", "olá mundo novo");

    // Clique simples posiciona o cursor entre "ol" e "á", perto de onde clicou.
    let ponto = no_texto(&app, "campo", 12.0);
    app.click_at(ponto, 1);
    assert_eq!(app.edit("campo").caret, Some("ol".len()));

    app.click_at(ponto, 2);
    assert_eq!(app.edit("campo").selection, Some((0, "olá".len())));
    app.click_at(ponto, 3);
    assert_eq!(
        app.edit("campo").selection,
        Some((0, "olá mundo novo".len()))
    );

    // Arrastar do começo até o fim do campo seleciona tudo.
    let fim = Point::new(app.rect("campo").x1 - 2.0, ponto.y);
    app.drag(no_texto(&app, "campo", 0.0), fim);
    assert_eq!(
        app.edit("campo").selection,
        Some((0, "olá mundo novo".len()))
    );
}

#[test]
fn rolagem_mantem_o_cursor_visivel() {
    let mut app = TestApp::new(campo(Edit::bind)).size(160, 48);
    let rect = app.rect("campo");
    app.type_into("campo", "um texto bem maior que o campo");
    let scroll = app.edit("campo").scroll;
    assert!(scroll > 0.0, "rolou para mostrar o fim");

    let area = app.text_input().expect("campo com foco").area;
    assert!(
        area.x1 <= rect.x1 && area.x0 >= rect.x0,
        "{area:?} fora de {rect:?}"
    );

    app.key(Key::Home);
    assert_eq!(app.edit("campo").scroll, 0.0);
    app.key(Key::End);
    assert_eq!(app.edit("campo").scroll, scroll);
}

#[test]
fn senha() {
    let mut app = TestApp::new(campo(|s| Edit::bind(s).password()));
    app.type_into("campo", "s3nh@ 1");
    assert_eq!(app.text("campo"), "s3nh@ 1");
    let mascara: String = std::iter::repeat_n(PASSWORD_CHAR, 7).collect();
    assert_eq!(app.edit("campo").text, mascara);

    // Nada sai da senha: copiar e recortar não fazem nada.
    app.shortcut('a');
    app.shortcut('c');
    app.shortcut('x');
    assert_eq!(app.clipboard().read(), None);
    assert_eq!(app.text("campo"), "s3nh@ 1");

    // Não dá para descobrir onde estão os espaços: o duplo clique seleciona tudo.
    app.key(Key::End);
    app.click_at(no_texto(&app, "campo", 3.0), 2);
    assert_eq!(app.edit("campo").selection, Some((0, mascara.len())));

    let input = app.text_input().expect("campo com foco");
    assert_eq!(input.purpose, InputPurpose::Password);
}

#[test]
fn acento_pelo_ime() {
    let mut app = TestApp::new(login);
    app.type_into("email", "caf");

    // Tecla morta: o acento fica em composição, sublinhado, sem entrar no valor.
    app.ime_preedit("´");
    let visual = app.edit("email");
    assert_eq!(visual.text, "caf´");
    assert_eq!(visual.composing, Some((3, "caf´".len())));
    assert_eq!(app.text("email"), "caf");

    app.ime_commit("é");
    assert_eq!(app.text("email"), "café");
    assert_eq!(app.edit("email").composing, None);
    assert_eq!(app.edit("email").caret, Some("café".len()));

    // Composição abandonada ao sair do campo não deixa rastro.
    app.ime_preedit("´");
    app.tab();
    assert_eq!(app.focused().as_deref(), Some("senha"));
    assert_eq!(app.text("email"), "café");
    assert_eq!(app.edit("email").text, "café");
}

#[test]
fn text_input_segue_o_foco_e_o_cursor() {
    let mut app = TestApp::new(login).size(390, 844);
    assert_eq!(app.text_input(), None, "sem foco não há teclado");

    app.click("email");
    let vazio = app.text_input().expect("e-mail com foco");
    assert_eq!(vazio.purpose, InputPurpose::Normal);
    let rect = app.rect("email");
    assert!(rect.contains(vazio.area.center()));

    app.type_text("ana@x.com");
    let cheio = app.text_input().unwrap();
    assert!(cheio.area.x0 > vazio.area.x0 + 40.0, "acompanha o cursor");

    app.tab();
    assert_eq!(app.text_input().unwrap().purpose, InputPurpose::Password);
    app.type_text("123456");
    app.tab();
    assert_eq!(app.focused().as_deref(), Some("entrar"));
    assert_eq!(app.text_input(), None, "botão não pede teclado");
}

#[test]
fn cluster_nao_se_divide() {
    let mut app = TestApp::new(campo(Edit::bind));
    // "é" decomposto: "e" mais o acento agudo combinado (U+0301).
    app.type_into("campo", "cafe\u{301}");
    app.key(Key::Left);
    assert_eq!(app.edit("campo").caret, Some("caf".len()));
    app.key(Key::Right);
    app.key(Key::Backspace);
    assert_eq!(app.text("campo"), "caf");
}

#[test]
fn desabilitado_nao_recebe_foco() {
    let mut app = TestApp::new(campo(|s| Edit::bind(s).disabled(true)));
    app.click("campo");
    assert_eq!(app.focused(), None);
    app.type_text("x");
    assert_eq!(app.text("campo"), "");
    assert_eq!(app.edit("campo").state, EditState::Disabled);
}

#[test]
fn cursor_pisca_sem_mudar_o_estado() {
    let mut app = TestApp::new(campo(Edit::bind));
    app.type_into("campo", "Ana");
    app.repaints_since_last();
    let aceso = app.snapshot();

    app.ui().set_caret_visible(false);
    assert!(app.ui().frame(), "o campo com foco é gravado de novo");
    assert_eq!(app.repaints_since_last(), ["campo"]);
    let apagado = app.snapshot();
    assert!(
        tomni_ui_test::snapshot::compare(&aceso, &apagado)
            .unwrap()
            .differing
            > 0
    );
    // O estado do `Edit` não muda: o cursor some só no desenho.
    assert_eq!(app.edit("campo").caret, Some(3));
    assert!(app.text_input().is_some());

    app.ui().set_caret_visible(true);
    app.frame();
    assert_eq!(app.repaints_since_last(), ["campo"]);
    let comparison = tomni_ui_test::snapshot::compare(&aceso, &app.snapshot()).unwrap();
    assert_eq!(comparison.differing, 0);
    assert!(!app.ui().frame(), "sem mudança, nada a pintar");
}

/// Estados do campo para os snapshots.
#[derive(Clone, Copy, Debug)]
enum Case {
    Placeholder,
    Hover,
    Focused,
    Selection,
    Password,
    Scrolled,
    Disabled,
}

impl Case {
    const ALL: [Self; 7] = [
        Self::Placeholder,
        Self::Hover,
        Self::Focused,
        Self::Selection,
        Self::Password,
        Self::Scrolled,
        Self::Disabled,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Placeholder => "placeholder",
            Self::Hover => "hover",
            Self::Focused => "focused",
            Self::Selection => "selection",
            Self::Password => "password",
            Self::Scrolled => "scrolled",
            Self::Disabled => "disabled",
        }
    }

    fn app(self, theme: Theme) -> TestApp {
        let password = matches!(self, Self::Password);
        let disabled = matches!(self, Self::Disabled);
        let view = move || {
            let texto = signal(if disabled {
                "Somente leitura".to_owned()
            } else {
                String::new()
            });
            let mut edit = Edit::bind(texto)
                .id("campo")
                .placeholder("Nome completo")
                .disabled(disabled);
            if password {
                edit = edit.password();
            }
            Form::new("Campo").child(Column::new().padding(8).child(edit))
        };
        let mut app = TestApp::new(view).size(200, 48).theme(theme);
        match self {
            Self::Placeholder | Self::Disabled => {}
            Self::Hover => {
                app.hover("campo");
            }
            Self::Focused => {
                app.type_into("campo", "Ana Souza");
            }
            Self::Selection => {
                app.type_into("campo", "Ana Souza");
                app.key_with(Key::Left, WORD | Modifiers::SHIFT);
            }
            Self::Password => {
                app.type_into("campo", "123456");
            }
            Self::Scrolled => {
                app.type_into("campo", "Ana Maria de Souza Albuquerque");
            }
        }
        app
    }
}

fn check(theme: Theme) {
    let theme_name = match theme {
        Theme::Light => "claro",
        Theme::Dark => "escuro",
    };
    for case in Case::ALL {
        let app = case.app(theme);
        app.assert_snapshot(&format!("edit_{}_{theme_name}", case.name()));
    }
}

#[test]
fn edit_claro() {
    check(Theme::Light);
}

#[test]
fn edit_escuro() {
    check(Theme::Dark);
}
