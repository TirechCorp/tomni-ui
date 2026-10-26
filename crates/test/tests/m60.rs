//! Marco M60: `PropertyGrid::hidden` (linha e categoria) e `PropertyGrid::default_with`.

use tomni_ui::kurbo::Point;
use tomni_ui::prelude::*;
use tomni_ui::theme::Theme;
use tomni_ui_test::TestApp;

/// Os signals que a grade de teste lê.
#[derive(Clone, Copy)]
struct Estado {
    tipo: Signal<String>,
    porta: Signal<f64>,
    proxy: Signal<bool>,
    endereco: Signal<String>,
    esconde_id: Signal<bool>,
    esconde_extra: Signal<bool>,
    id: Signal<String>,
    extra: Signal<String>,
}

fn grade(tema: Theme) -> (TestApp, Estado) {
    let guardado = std::rc::Rc::new(std::cell::Cell::new(None));
    let g = guardado.clone();
    let app = TestApp::new(move || {
        let e = Estado {
            tipo: signal("SSH".to_owned()),
            porta: signal(22.0),
            proxy: signal(false),
            endereco: signal("proxy.local".to_owned()),
            esconde_id: signal(false),
            esconde_extra: signal(false),
            id: signal("abc".to_owned()),
            extra: signal("xyz".to_owned()),
        };
        g.set(Some(e));
        let host = signal("exemplo.com".to_owned());
        let final_ = signal("fim".to_owned());
        Form::new("Conexão").child(
            Column::new()
                .child(Button::new("antes").id("antes").height(44))
                .child(
                    PropertyGrid::new()
                        .id("props")
                        .category("Conexão")
                        .text("Host", host)
                        .number("Porta", e.porta, 1.0..=65535.0)
                        .default_with(move || if e.tipo.get() == "Telnet" { 23 } else { 22 })
                        .category("Proxy")
                        .hidden(move || !e.proxy.get())
                        .text("Endereço do proxy", e.endereco)
                        .category("Avançado")
                        .text("Id", e.id)
                        .hidden(e.esconde_id)
                        .text("Extra", e.extra)
                        .hidden(e.esconde_extra)
                        .category("Final")
                        .text("Fim", final_)
                        .grow(1),
                ),
        )
    })
    .size(480, 420)
    .theme(tema);
    (app, guardado.get().unwrap())
}

fn existe(app: &TestApp, id: &str) -> bool {
    app.tree().find(id).is_some()
}

fn clicar_no_nome(app: &mut TestApp, nome: &str) {
    let r = app.rect(&format!("props-{nome}"));
    app.click_at(Point::new(r.x0 + 30.0, r.center().y), 1);
}

fn propriedade(app: &TestApp, nome: &str) -> tomni_ui_test::PropertyInfo {
    app.properties("props")
        .into_iter()
        .find(|p| p.name == nome)
        .unwrap_or_else(|| panic!("sem a propriedade {nome}"))
}

#[test]
fn esconder_tira_a_linha_e_a_categoria_vazia_e_mostrar_devolve_o_valor() {
    let (mut app, e) = grade(Theme::Light);
    assert!(!existe(&app, "props-categoria-Proxy"));
    assert!(!existe(&app, "props-Endereço do proxy"));
    assert!(propriedade(&app, "Endereço do proxy").hidden);
    assert!(!propriedade(&app, "Host").hidden);
    assert!(existe(&app, "props-Id") && existe(&app, "props-Extra"));

    // A categoria só perde o cabeçalho quando todas as linhas somem.
    e.esconde_id.set(true);
    app.frame();
    assert!(!existe(&app, "props-Id"));
    assert!(propriedade(&app, "Id").hidden);
    assert!(existe(&app, "props-categoria-Avançado"));
    e.esconde_extra.set(true);
    app.frame();
    assert!(!existe(&app, "props-categoria-Avançado"));
    let no = app.tree();
    let categorias = no.find("props").unwrap().categories.clone().unwrap();
    let hidden: Vec<_> = categorias
        .iter()
        .map(|c| (c.name.as_str(), c.hidden))
        .collect();
    assert_eq!(
        hidden,
        [
            ("Conexão", false),
            ("Proxy", true),
            ("Avançado", true),
            ("Final", false)
        ]
    );

    // A categoria com `.hidden` volta com o valor que tinha.
    e.proxy.set(true);
    e.esconde_id.set(false);
    app.frame();
    assert!(existe(&app, "props-categoria-Proxy"));
    assert_eq!(app.text("props-Endereço do proxy"), "proxy.local");
    assert_eq!(propriedade(&app, "Endereço do proxy").value, "proxy.local");
    assert!(!propriedade(&app, "Endereço do proxy").hidden);
    assert_eq!(propriedade(&app, "Id").value, "abc");
    assert!(existe(&app, "props-categoria-Avançado"));
}

#[test]
fn categoria_escondida_leva_as_linhas_na_ordem_alfabetica_tambem() {
    let (mut app, e) = grade(Theme::Light);
    app.click("props-alfabetica");
    assert!(!existe(&app, "props-Endereço do proxy"));
    e.proxy.set(true);
    app.frame();
    assert!(existe(&app, "props-Endereço do proxy"));
    e.proxy.set(false);
    e.esconde_id.set(true);
    app.frame();
    assert!(!existe(&app, "props-Endereço do proxy"));
    assert!(!existe(&app, "props-Id"));
    assert!(existe(&app, "props-Extra"));
}

#[test]
fn tab_e_setas_pulam_a_linha_escondida() {
    let (mut app, e) = grade(Theme::Light);
    e.esconde_id.set(true);
    app.frame();
    clicar_no_nome(&mut app, "Porta");
    app.key(Key::Down);
    assert_eq!(app.focused().as_deref(), Some("props-categoria-Avançado"));
    app.key(Key::Down);
    assert_eq!(app.focused().as_deref(), Some("props-Extra"));
    app.key(Key::Down);
    assert_eq!(app.focused().as_deref(), Some("props-categoria-Final"));
    app.key(Key::Up).key(Key::Up).key(Key::Up);
    assert_eq!(app.focused().as_deref(), Some("props-Porta"));

    // O Tab de fora para na linha escolhida; se ela some, na primeira que aparece.
    clicar_no_nome(&mut app, "Extra");
    e.esconde_extra.set(true);
    app.frame();
    app.click("antes");
    app.tab().tab();
    assert_eq!(app.focused().as_deref(), Some("props-categoria-Conexão"));
}

#[test]
fn linha_escolhida_que_some_deixa_a_descricao_vazia_e_volta_inteira() {
    let (mut app, e) = grade(Theme::Light);
    clicar_no_nome(&mut app, "Id");
    assert_eq!(app.text("props-descricao-nome"), "Id");
    e.esconde_id.set(true);
    app.frame();
    assert_eq!(app.text("props-descricao-nome"), "");
    // Nada quebra: a grade continua respondendo.
    clicar_no_nome(&mut app, "Host");
    assert_eq!(app.text("props-descricao-nome"), "Host");
    e.esconde_id.set(false);
    app.frame();
    clicar_no_nome(&mut app, "Id");
    assert_eq!(app.text("props-descricao-nome"), "Id");
    assert_eq!(app.text("props-Id-valor"), "abc");
}

#[test]
fn esconder_no_meio_da_edicao_fecha_o_editor_e_o_valor_continua() {
    let (mut app, e) = grade(Theme::Light);
    clicar_no_nome(&mut app, "Id");
    app.type_into("props-Id-valor", "1").key(Key::Enter);
    assert_eq!(e.id.get_untracked(), "abc1");
    clicar_no_nome(&mut app, "Id");
    assert!(existe(&app, "props-Id-valor"));
    e.esconde_id.set(true);
    app.frame();
    assert!(!existe(&app, "props-Id-valor"));
    assert_eq!(e.id.get_untracked(), "abc1");
    e.esconde_id.set(false);
    app.frame();
    assert_eq!(propriedade(&app, "Id").value, "abc1");
}

#[test]
fn default_with_acompanha_o_outro_campo() {
    let (mut app, e) = grade(Theme::Light);
    assert!(!propriedade(&app, "Porta").changed, "22 é o padrão do SSH");
    e.tipo.set("Telnet".to_owned());
    app.frame();
    assert!(propriedade(&app, "Porta").changed, "o padrão agora é 23");
    e.porta.set(23.0);
    app.frame();
    assert!(!propriedade(&app, "Porta").changed);
    e.porta.set(2222.0);
    app.frame();
    assert!(propriedade(&app, "Porta").changed);
}

#[test]
fn a_ferramenta_mostra_o_hidden_nas_linhas_e_nas_categorias() {
    let (mut app, e) = grade(Theme::Light);
    e.esconde_id.set(true);
    app.frame();
    let no = app.tree();
    let grade = no.find("props").unwrap();
    let propriedades = grade.properties.as_ref().unwrap();
    let hidden = |nome: &str| propriedades.iter().find(|p| p.name == nome).unwrap().hidden;
    assert!(hidden("Id") && hidden("Endereço do proxy"));
    assert!(!hidden("Extra") && !hidden("Host"));
    let _ = &mut app;
}

/// A categoria Proxy escondida e a linha Id escondida no meio da categoria Avançado.
fn snapshot_das_escondidas(tema: Theme, nome: &str) {
    let (mut app, e) = grade(tema);
    e.esconde_id.set(true);
    app.frame();
    clicar_no_nome(&mut app, "Host");
    app.assert_snapshot(nome);
}

#[test]
fn snapshot_claro() {
    snapshot_das_escondidas(Theme::Light, "m60-escondidas-claro");
}

#[test]
fn snapshot_escuro() {
    snapshot_das_escondidas(Theme::Dark, "m60-escondidas-escuro");
}
