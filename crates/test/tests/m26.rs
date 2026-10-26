//! Marco M26: o modo do painel do `NavigationView` vindo de um signal (`pane_mode_with`), que o
//! Tirech Omni usa para deixar o painel compacto na página dos terminais.

use tomni_ui::prelude::*;
use tomni_ui::{NAV_COMPACT_WIDTH, NAV_PANE_WIDTH};
use tomni_ui_test::TestApp;

/// Uma navegação com duas páginas; a primeira tem um campo, para ver que o estado sobrevive.
/// Os botões trocam o modo e a página.
fn tela() -> impl View + use<> {
    let pagina = signal("a".to_owned());
    let modo: Signal<Option<NavigationMode>> = signal(None);
    let nota = signal(String::new());
    Form::new("Modo").child(
        Column::new()
            .child(
                Row::new()
                    .child(
                        Button::new("Compacto")
                            .id("compacto")
                            .on_click(move || modo.set(Some(NavigationMode::Compact))),
                    )
                    .child(
                        Button::new("Expandido")
                            .id("expandido")
                            .on_click(move || modo.set(Some(NavigationMode::Expanded))),
                    )
                    .child(
                        Button::new("Pela largura")
                            .id("largura")
                            .on_click(move || modo.set(None)),
                    ),
            )
            .child(
                NavigationView::bind(pagina)
                    .id("nav")
                    .pane_mode_with(modo)
                    .item(
                        "a",
                        Symbol::Home,
                        "Página A",
                        Edit::bind(nota).id("nota").height(44),
                    )
                    .item("b", Symbol::Book, "Página B", Text::new("B")),
            ),
    )
}

fn modo(app: &mut TestApp) -> Option<&'static str> {
    app.tree().find("nav").unwrap().variant
}

#[test]
fn trocar_o_signal_muda_o_modo_sem_recriar_o_widget() {
    let mut app = TestApp::new(tela).size(1280, 600);
    assert_eq!(modo(&mut app), Some(NavigationMode::Expanded.name()));
    app.type_into("nota", "rascunho");

    app.click("compacto");
    assert_eq!(modo(&mut app), Some(NavigationMode::Compact.name()));
    assert_eq!(app.rect("nav-painel").width(), NAV_COMPACT_WIDTH);
    assert_eq!(app.text("nota"), "rascunho", "a página não é recriada");

    app.click("expandido");
    assert_eq!(modo(&mut app), Some(NavigationMode::Expanded.name()));
    assert_eq!(app.rect("nav-painel").width(), NAV_PANE_WIDTH);
    assert_eq!(app.text("nota"), "rascunho");
}

#[test]
fn none_volta_ao_modo_pela_largura() {
    let mut app = TestApp::new(tela).size(1280, 600);
    app.click("compacto");
    app.click("largura");
    assert_eq!(modo(&mut app), Some(NavigationMode::Expanded.name()));

    app.resize(800, 600);
    assert_eq!(modo(&mut app), Some(NavigationMode::Compact.name()));
    app.click("expandido");
    assert_eq!(modo(&mut app), Some(NavigationMode::Expanded.name()));
    app.click("largura");
    assert_eq!(modo(&mut app), Some(NavigationMode::Compact.name()));
}

#[test]
fn o_painel_aberto_a_mao_no_compacto_fecha_ao_trocar_de_pagina() {
    let mut app = TestApp::new(tela).size(1280, 600);
    app.click("compacto");
    app.click("nav-alternar");
    assert_eq!(
        app.rect("nav-painel").width(),
        NAV_PANE_WIDTH,
        "☰ abre o painel"
    );
    app.click("nav-b");
    assert_eq!(app.text("nav"), "b");
    assert_eq!(
        app.rect("nav-painel").width(),
        NAV_COMPACT_WIDTH,
        "escolher fecha"
    );
}

#[test]
fn a_cor_com_par_claro_escuro_segue_o_tema() {
    use tomni_ui::theme::Tokens;

    let cor = ThemeColor::themed(Color::hex(0xD4F1F9), Color::hex(0x0E3B4C));
    assert_eq!(cor.describe(), "#D4F1F9/#0E3B4C");
    let claro = Tokens::LIGHT.resolve(cor);
    let escuro = Tokens::DARK.resolve(cor);
    assert_eq!(
        (claro.to_rgba8().r, claro.to_rgba8().g, claro.to_rgba8().b),
        (0xD4, 0xF1, 0xF9)
    );
    assert_eq!(
        (
            escuro.to_rgba8().r,
            escuro.to_rgba8().g,
            escuro.to_rgba8().b
        ),
        (0x0E, 0x3B, 0x4C)
    );
}

#[test]
fn o_atalho_da_busca_leva_o_foco_para_o_campo() {
    let mut app = TestApp::new(|| {
        let texto = signal(String::new());
        let outro = signal(String::new());
        Form::new("Busca").child(
            Column::new().child(Edit::bind(outro).id("outro")).child(
                AutoSuggestBox::bind(texto, |_| Vec::new())
                    .id("busca")
                    .focus_shortcut(Shortcut::ctrl('E')),
            ),
        )
    });
    app.click("outro");
    assert_eq!(app.focused().as_deref(), Some("outro"));
    app.shortcut('e');
    assert_eq!(app.focused().as_deref(), Some("busca"));
}

#[test]
fn a_largura_maxima_limita_e_a_coluna_fica_centrada() {
    let tela = || {
        Form::new("Leitura").child(
            Row::new()
                .child(Column::new().grow(1))
                .child(
                    Column::new()
                        .id("leitura")
                        .grow(1000)
                        .max_width(400)
                        .child(Text::new("Texto de leitura")),
                )
                .child(Column::new().grow(1)),
        )
    };
    let app = TestApp::new(tela).size(1000, 300);
    let larga = app.rect("leitura");
    assert_eq!(larga.width(), 400.0);
    assert_eq!(larga.x0 + larga.x1, 1000.0, "centrada na janela");

    let app = TestApp::new(tela).size(300, 300);
    assert!(app.rect("leitura").width() <= 300.0);
    assert!(app.rect("leitura").width() > 290.0, "ocupa quase tudo");
}
