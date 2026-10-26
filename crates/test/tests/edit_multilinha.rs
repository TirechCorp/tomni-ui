//! `Edit` de várias linhas e somente leitura (pedido pelo M44, para colar e mostrar chaves).
//!
//! Passam pelo `Ui` com o `FluentDrawer`, como o M3: o cursor e o clique usam as medidas da
//! Selawik.

use tomni_ui::kurbo::{Point, Vec2};
use tomni_ui::prelude::*;
use tomni_ui::{Key, Modifiers};
use tomni_ui_test::TestApp;

fn campo(altura: u32, inicial: &'static str, somente_leitura: bool) -> impl Fn() -> Form {
    move || {
        let texto = signal(inicial.to_owned());
        let mut edit = Edit::bind(texto)
            .multiline()
            .id("campo")
            .width(240)
            .height(altura);
        if somente_leitura {
            edit = edit.read_only();
        }
        Form::new("Campo").child(Column::new().padding(8).child(edit))
    }
}

#[test]
fn enter_quebra_a_linha_e_o_texto_digitado_mantem_as_quebras() {
    let mut app = TestApp::new(campo(96, "", false));
    app.type_into("campo", "um");
    app.key(Key::Enter);
    app.type_text("dois\ntrês");
    assert_eq!(app.text("campo"), "um\ndois\ntrês");
    assert_eq!(app.edit("campo").caret, Some("um\ndois\ntrês".len()));
}

#[test]
fn colar_mantem_as_quebras_e_troca_tab_por_espaco() {
    let mut app = TestApp::new(campo(96, "", false));
    app.click("campo");
    app.clipboard().write("a\r\nb\rc\td\u{7}".into());
    app.shortcut('v');
    assert_eq!(app.text("campo"), "a\nb\nc d");
}

#[test]
fn cima_e_baixo_andam_por_linha_e_mantem_a_coluna() {
    let mut app = TestApp::new(campo(96, "abcdef\nab\nabcdef", false));
    app.click("campo");
    app.shortcut(Shortcut::ctrl(Key::Home));
    assert_eq!(app.edit("campo").caret, Some(0));
    for _ in 0..4 {
        app.key(Key::Right);
    }
    app.key(Key::Down);
    // A segunda linha só tem 2 letras: o cursor vai ao fim dela.
    assert_eq!(app.edit("campo").caret, Some("abcdef\nab".len()));
    app.key(Key::Down);
    let na_terceira = app.edit("campo").caret.unwrap();
    assert!(na_terceira > "abcdef\nab\n".len());
    app.key(Key::Down);
    assert_eq!(app.edit("campo").caret, Some("abcdef\nab\nabcdef".len()));
    app.key(Key::Up);
    app.key(Key::Up);
    app.key(Key::Up);
    assert_eq!(app.edit("campo").caret, Some(0));
}

#[test]
fn home_e_end_valem_para_a_linha_e_com_ctrl_para_o_texto() {
    let mut app = TestApp::new(campo(96, "um\ndois\ntrês", false));
    app.click("campo");
    app.shortcut(Shortcut::ctrl(Key::Home));
    app.key(Key::Down);
    app.key(Key::End);
    assert_eq!(app.edit("campo").caret, Some("um\ndois".len()));
    app.key(Key::Home);
    assert_eq!(app.edit("campo").caret, Some("um\n".len()));
    app.key_with(Key::End, Modifiers::CTRL);
    assert_eq!(app.edit("campo").caret, Some("um\ndois\ntrês".len()));
}

#[test]
fn clicar_leva_o_cursor_para_a_linha_certa() {
    let mut app = TestApp::new(campo(96, "um\ndois\ntrês", false));
    let rect = app.rect("campo");
    // Padding de 1 + 5 px em cima; cada linha tem 20 px.
    let na_segunda = Point::new(rect.x0 + 200.0, rect.y0 + 6.0 + 20.0 + 10.0);
    app.click_at(na_segunda, 1);
    assert_eq!(app.edit("campo").caret, Some("um\ndois".len()));
    let na_primeira = Point::new(rect.x0 + 11.0, rect.y0 + 10.0);
    app.click_at(na_primeira, 1);
    assert_eq!(app.edit("campo").caret, Some(0));
}

#[test]
fn campo_baixo_rola_para_mostrar_o_cursor_e_com_a_roda() {
    let linhas = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10";
    let mut app = TestApp::new(campo(64, linhas, false));
    app.click("campo");
    app.shortcut(Shortcut::ctrl(Key::End));
    assert!(app.edit("campo").scroll_y > 0.0);
    let rect = app.rect("campo");
    let meio = Point::new(rect.center().x, rect.center().y);
    app.wheel(meio, Vec2::new(0.0, -2000.0));
    assert_eq!(app.edit("campo").scroll_y, 0.0);
    app.wheel(meio, Vec2::new(0.0, 30.0));
    assert!(app.edit("campo").scroll_y > 0.0);
}

#[test]
fn somente_leitura_deixa_selecionar_e_copiar_mas_nao_mudar() {
    let mut app = TestApp::new(campo(96, "segredo\nda chave", true));
    app.click("campo");
    app.type_text("x");
    app.key(Key::Enter);
    app.key(Key::Backspace);
    app.key(Key::Delete);
    app.shortcut('a');
    app.shortcut('x');
    assert_eq!(app.text("campo"), "segredo\nda chave");
    app.shortcut('c');
    assert_eq!(app.clipboard().read().as_deref(), Some("segredo\nda chave"));
    app.clipboard().write("outra".into());
    app.shortcut('v');
    assert_eq!(app.text("campo"), "segredo\nda chave");
    assert_eq!(app.edit("campo").caret, None);
}

#[test]
fn passa_no_lint_nos_dois_temas() {
    let mut app = TestApp::new(campo(96, "linha um\nlinha dois", false));
    app.assert_lint();
}

#[test]
fn snapshot_do_campo_de_varias_linhas() {
    for (nome, tema) in [
        ("claro", tomni_ui::theme::Theme::Light),
        ("escuro", tomni_ui::theme::Theme::Dark),
    ] {
        let mut app = TestApp::new(campo(
            96,
            "-----BEGIN KEY-----\nAAAAB3NzaC1\n-----END KEY-----",
            false,
        ))
        .size(260, 120)
        .theme(tema);
        app.click("campo");
        app.shortcut('a');
        app.assert_snapshot(&format!("edit_multilinha_{nome}"));
    }
}
