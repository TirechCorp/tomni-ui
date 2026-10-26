//! Marco M37: a linha `.button` do `PropertyGrid`.

use std::cell::Cell;
use std::rc::Rc;

use tomni_ui::kurbo::Point;
use tomni_ui::prelude::*;
use tomni_ui::theme::Theme;
use tomni_ui_test::TestApp;

/// Uma grade com uma linha de botão: `chamadas` conta os cliques e `resumo` é o texto da linha.
struct Grade {
    app: TestApp,
    chamadas: Rc<Cell<u32>>,
    resumo: Signal<String>,
    desabilitada: Signal<bool>,
}

fn grade() -> Grade {
    let chamadas = Rc::new(Cell::new(0));
    let guardado = Rc::new(Cell::new(None));
    let (c, g) = (chamadas.clone(), guardado.clone());
    let app = TestApp::new(move || {
        let resumo = signal("Nenhum".to_owned());
        let desabilitada = signal(false);
        g.set(Some((resumo, desabilitada)));
        let (c, outro) = (c.clone(), c.clone());
        Form::new("Botão").child(
            PropertyGrid::new()
                .id("props")
                .grow(1)
                .text("Host", signal("exemplo.com".to_owned()))
                .button("Túneis", resumo, move || c.set(c.get() + 1))
                .description("Os túneis da conexão.")
                .text("Usuário", signal("ana".to_owned()))
                .button("Outro", signal("Nenhum".to_owned()), move || {
                    outro.set(outro.get() + 100);
                })
                .disabled(desabilitada),
        )
    })
    .size(480, 360);
    let (resumo, desabilitada) = guardado.get().unwrap();
    Grade {
        app,
        chamadas,
        resumo,
        desabilitada,
    }
}

fn escolher(app: &mut TestApp, nome: &str) {
    let r = app.rect(&format!("props-{nome}"));
    app.click_at(Point::new(r.x0 + 20.0, r.center().y), 1);
}

#[test]
fn o_botao_so_existe_na_linha_escolhida_e_clicar_nele_chama() {
    let mut g = grade();
    assert!(g.app.tree().find("props-Túneis-botao").is_none());
    escolher(&mut g.app, "Túneis");
    g.app.click("props-Túneis-botao");
    assert_eq!(g.chamadas.get(), 1);
    g.app.click("props-Túneis-botao");
    assert_eq!(g.chamadas.get(), 2);
}

#[test]
fn enter_f2_e_espaco_chamam_na_linha_e_no_texto() {
    let mut g = grade();
    escolher(&mut g.app, "Túneis");
    g.app.key(Key::Enter);
    g.app.key(Key::F(2));
    g.app.key(Key::Space);
    assert_eq!(g.chamadas.get(), 3, "com o foco na linha");
    g.app.click("props-Túneis-valor");
    assert_eq!(g.app.focused().as_deref(), Some("props-Túneis-valor"));
    g.app.key(Key::Enter);
    g.app.key(Key::Space);
    assert_eq!(g.chamadas.get(), 5, "com o foco no texto");
}

#[test]
fn o_texto_e_so_de_leitura() {
    let mut g = grade();
    escolher(&mut g.app, "Túneis");
    g.app.click("props-Túneis-valor");
    g.app.type_text("abc");
    g.app.key(Key::Backspace);
    g.app.key(Key::Delete);
    assert_eq!(g.resumo.get_untracked(), "Nenhum");
    assert_eq!(g.app.text("props-Túneis-valor"), "Nenhum");
    assert_eq!(g.chamadas.get(), 0);
    // O Esc devolve o foco à linha.
    g.app.key(Key::Escape);
    assert_eq!(g.app.focused().as_deref(), Some("props-Túneis"));
}

#[test]
fn digitar_na_linha_nao_abre_edicao() {
    let mut g = grade();
    escolher(&mut g.app, "Túneis");
    g.app.type_text("x");
    assert_eq!(g.app.focused().as_deref(), Some("props-Túneis"));
    assert_eq!(g.chamadas.get(), 0);
}

#[test]
fn o_resumo_reativo_muda_o_texto() {
    let mut g = grade();
    let valor = |app: &TestApp| {
        app.properties("props")
            .into_iter()
            .find(|p| p.name == "Túneis")
            .unwrap()
            .value
    };
    assert_eq!(valor(&g.app), "Nenhum");
    g.resumo.set("2 túneis".to_owned());
    assert_eq!(valor(&g.app), "2 túneis");
    escolher(&mut g.app, "Túneis");
    assert_eq!(g.app.text("props-Túneis-valor"), "2 túneis");
    g.resumo.set("1 túnel".to_owned());
    assert_eq!(g.app.text("props-Túneis-valor"), "1 túnel");
}

#[test]
fn desabilitada_nao_chama() {
    let mut g = grade();
    g.desabilitada.set(true);
    escolher(&mut g.app, "Outro");
    assert!(g.app.tree().find("props-Outro-botao").is_none());
    g.app.key(Key::Enter);
    assert_ne!(g.app.focused().as_deref(), Some("props-Outro"));
    assert_eq!(g.chamadas.get(), 0);
}

#[test]
fn lint_passa_nos_dois_temas() {
    for tema in [Theme::Light, Theme::Dark] {
        let mut g = grade();
        g.app = g.app.theme(tema);
        escolher(&mut g.app, "Túneis");
        g.app.assert_lint();
    }
}
