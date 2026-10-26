//! Marco M41 (fatia da lib): o tom de fundo da linha do `DataGrid` (`RowTone`), a `.folder` da
//! `PropertyGrid` e o item de rádio do `MenuFlyout` (`.radio`).

mod comum;

use std::cell::Cell;
use std::collections::BTreeSet;
use std::rc::Rc;

use tomni_ui::Key;
use tomni_ui::dialogs::FileDialogKind;
use tomni_ui::kurbo::Point;
use tomni_ui::prelude::*;
use tomni_ui::theme::Theme;
use tomni_ui_test::TestApp;

const TEMAS: [Theme; 2] = [Theme::Light, Theme::Dark];

// ---------------------------------------------------------------------------------------
// RowTone
// ---------------------------------------------------------------------------------------

fn linha(chave: &str, tom: Option<RowTone>) -> DataGridRow {
    let linha = DataGridRow::new(chave).cell(chave, CellValue::Text(chave.to_owned()));
    match tom {
        Some(tom) => linha.tone(tom),
        None => linha,
    }
}

/// Quatro linhas: sem tom, sucesso, atenção e crítico. A seleção fica no `Signal` devolvido.
fn grade_de_tons(tema: Theme) -> (TestApp, Signal<BTreeSet<String>>) {
    let guardado = Rc::new(Cell::new(None));
    let g = Rc::clone(&guardado);
    let app = TestApp::new(move || {
        let linhas = signal(vec![
            linha("igual", None),
            linha("novo", Some(RowTone::Success)),
            linha("so-aqui", Some(RowTone::Caution)),
            linha("diferente", Some(RowTone::Critical)),
        ]);
        let selecionadas = signal(BTreeSet::new());
        g.set(Some(selecionadas));
        Form::new("Tons").child(
            Column::new().padding(12).child(
                DataGrid::bind(linhas, selecionadas)
                    .id("g")
                    .height(260)
                    .columns([DataGridColumn::new("nome", "Nome").width(ColumnWidth::Star(1.0))]),
            ),
        )
    })
    .size(360, 320)
    .theme(tema);
    (app, guardado.get().unwrap())
}

/// A cor (RGB) do pixel perto da borda direita da linha `id`, longe do texto.
fn cor_da_linha(app: &TestApp, id: &str) -> [u8; 3] {
    let r = app.rect(id);
    let quadro = app.snapshot();
    let (x, y) = ((r.x1 - 20.0) as usize, r.center().y as usize);
    let pixel = quadro.data()[y * usize::from(quadro.width()) + x];
    [pixel.r, pixel.g, pixel.b]
}

#[test]
fn a_inspecao_da_grade_traz_o_tom_de_cada_linha() {
    let (app, _) = grade_de_tons(Theme::Light);
    let tons: Vec<Option<String>> = app.grid("g").rows.into_iter().map(|l| l.tone).collect();
    assert_eq!(
        tons,
        [
            None,
            Some("success".to_owned()),
            Some("caution".to_owned()),
            Some("critical".to_owned())
        ]
    );
}

#[test]
fn o_no_da_linha_descreve_o_tom_e_so_ele() {
    let (app, _) = grade_de_tons(Theme::Light);
    let arvore = app.tree();
    assert_eq!(arvore.find("g-igual").unwrap().description, None);
    assert_eq!(
        arvore.find("g-novo").unwrap().description.as_deref(),
        Some("destaque: sucesso")
    );
    assert_eq!(
        arvore.find("g-so-aqui").unwrap().description.as_deref(),
        Some("destaque: atenção")
    );
    assert_eq!(
        arvore.find("g-diferente").unwrap().description.as_deref(),
        Some("destaque: crítico")
    );
}

#[test]
fn cada_tom_pinta_um_fundo_proprio_e_a_selecao_continua_visivel() {
    for tema in TEMAS {
        let (mut app, selecionadas) = grade_de_tons(tema);
        let sem_tom = cor_da_linha(&app, "g-igual");
        let sucesso = cor_da_linha(&app, "g-novo");
        let atencao = cor_da_linha(&app, "g-so-aqui");
        let critico = cor_da_linha(&app, "g-diferente");
        for (nome, cor) in [
            ("sucesso", sucesso),
            ("atenção", atencao),
            ("crítico", critico),
        ] {
            assert_ne!(cor, sem_tom, "o tom {nome} muda o fundo ({tema:?})");
        }
        assert_ne!(sucesso, atencao);
        assert_ne!(atencao, critico);
        assert_ne!(sucesso, critico);
        selecionadas.set(["novo".to_owned()].into());
        app.hover("g-igual");
        let selecionada = cor_da_linha(&app, "g-novo");
        assert_ne!(
            selecionada, sucesso,
            "a seleção aparece sobre o tom ({tema:?})"
        );
        assert_ne!(selecionada, sem_tom);
        app.assert_lint();
    }
}

#[test]
fn snapshots_dos_tons_da_grade() {
    for tema in TEMAS {
        let (mut app, selecionadas) = grade_de_tons(tema);
        selecionadas.set(["so-aqui".to_owned()].into());
        app.hover("g-igual");
        app.assert_lint();
        app.assert_snapshot(&format!("m41_tons_{}", comum::theme_name(tema)));
    }
}

// ---------------------------------------------------------------------------------------
// PropertyGrid::folder
// ---------------------------------------------------------------------------------------

/// Uma grade com uma pasta (`.folder`) e um arquivo (`.file`), mais um texto para tirar a
/// escolha da pasta.
fn grade_de_pastas(tema: Theme) -> (TestApp, Signal<String>, Signal<String>) {
    let guardado = Rc::new(Cell::new(None));
    let g = Rc::clone(&guardado);
    let app = TestApp::new(move || {
        let pasta = signal(String::new());
        let chave = signal(String::new());
        g.set(Some((pasta, chave)));
        Form::new("Pastas").child(
            PropertyGrid::new()
                .id("props")
                .grow(1)
                .text("Nome", signal("a".to_owned()))
                .folder("Pasta local", pasta)
                .description("Onde o painel do computador abre.")
                .file("Chave privada", chave),
        )
    })
    .size(480, 320)
    .theme(tema);
    let (pasta, chave) = guardado.get().unwrap();
    (app, pasta, chave)
}

fn escolher_linha(app: &mut TestApp, nome: &str) {
    let r = app.rect(&format!("props-{nome}"));
    app.click_at(Point::new(r.x0 + 30.0, r.center().y), 1);
}

#[test]
fn folder_so_mostra_o_botao_na_linha_escolhida() {
    let (mut app, _, _) = grade_de_pastas(Theme::Light);
    assert!(app.tree().find("props-Pasta local-procurar").is_none());
    escolher_linha(&mut app, "Pasta local");
    assert!(app.tree().find("props-Pasta local-procurar").is_some());
    assert!(app.tree().find("props-Pasta local-valor").is_some());
}

#[test]
fn folder_pede_uma_pasta_ao_seletor_e_grava_a_resposta() {
    let (mut app, pasta, _) = grade_de_pastas(Theme::Light);
    escolher_linha(&mut app, "Pasta local");
    app.click("props-Pasta local-procurar");
    let pedidos = app.file_dialogs();
    assert_eq!(pedidos.len(), 1);
    assert_eq!(pedidos[0].kind, FileDialogKind::Folder);
    assert_eq!(pedidos[0].title, "Pasta local");
    app.file_dialog_answer(Some("/tmp/projetos"));
    assert_eq!(pasta.get_untracked(), "/tmp/projetos");
    let linha = app
        .properties("props")
        .into_iter()
        .find(|p| p.name == "Pasta local")
        .unwrap();
    assert_eq!(linha.value, "/tmp/projetos");
}

#[test]
fn folder_cancelado_nao_muda_o_valor() {
    let (mut app, pasta, _) = grade_de_pastas(Theme::Light);
    pasta.set("/tmp/antiga".to_owned());
    escolher_linha(&mut app, "Pasta local");
    app.click("props-Pasta local-procurar");
    app.file_dialog_answer(None::<&str>);
    assert_eq!(pasta.get_untracked(), "/tmp/antiga");
}

#[test]
fn folder_abre_o_seletor_na_pasta_do_valor_quando_ela_existe() {
    let (mut app, pasta, _) = grade_de_pastas(Theme::Light);
    let existente = std::env::temp_dir();
    pasta.set(existente.to_string_lossy().into_owned());
    escolher_linha(&mut app, "Pasta local");
    app.click("props-Pasta local-procurar");
    assert_eq!(
        app.file_dialogs()[0].directory.as_deref(),
        Some(existente.as_path())
    );
}

#[test]
fn folder_digitada_vai_para_o_signal_e_o_file_continua_pedindo_arquivo() {
    let (mut app, pasta, _) = grade_de_pastas(Theme::Light);
    escolher_linha(&mut app, "Pasta local");
    app.type_text("/var/www");
    app.key(Key::Enter);
    assert_eq!(pasta.get_untracked(), "/var/www");
    escolher_linha(&mut app, "Chave privada");
    app.click("props-Chave privada-procurar");
    assert_eq!(app.file_dialogs()[0].kind, FileDialogKind::Open);
}

#[test]
fn snapshots_da_grade_com_pasta() {
    for tema in TEMAS {
        let (mut app, pasta, _) = grade_de_pastas(tema);
        pasta.set("/home/ana/projetos".to_owned());
        escolher_linha(&mut app, "Pasta local");
        app.assert_lint();
        app.assert_snapshot(&format!("m41_pasta_{}", comum::theme_name(tema)));
    }
}

// ---------------------------------------------------------------------------------------
// MenuFlyout::radio
// ---------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ordem {
    Nome,
    Tamanho,
    Data,
}

/// Um botão "Ordenar" com três rádios, um separador e um item de marcar, no mesmo menu.
fn menu_de_radios(tema: Theme) -> (TestApp, Signal<Ordem>, Signal<bool>) {
    let guardado = Rc::new(Cell::new(None));
    let g = Rc::clone(&guardado);
    let app = TestApp::new(move || {
        let ordem = signal(Ordem::Nome);
        let inverter = signal(false);
        g.set(Some((ordem, inverter)));
        Form::new("Ordenar").child(
            Column::new().padding(24).child(
                Button::new("Ordenar").id("ordenar").height(44).flyout(
                    MenuFlyout::new()
                        .id("menu")
                        .radio("por-nome", "Por nome", ordem, Ordem::Nome)
                        .radio("por-tamanho", "Por tamanho", ordem, Ordem::Tamanho)
                        .radio("por-data", "Por data", ordem, Ordem::Data)
                        .separator()
                        .toggle("inverter", "Inverter", inverter),
                ),
            ),
        )
    })
    .size(320, 320)
    .theme(tema);
    let (ordem, inverter) = guardado.get().unwrap();
    (app, ordem, inverter)
}

fn marcados(app: &TestApp) -> Vec<bool> {
    let arvore = app.tree();
    ["por-nome", "por-tamanho", "por-data"]
        .iter()
        .map(|id| arvore.find(id).unwrap().checked.unwrap())
        .collect()
}

#[test]
fn radio_marca_so_o_do_valor_do_grupo() {
    let (mut app, ordem, _) = menu_de_radios(Theme::Light);
    app.click("ordenar");
    let arvore = app.tree();
    assert_eq!(arvore.find("por-nome").unwrap().kind, "radio_menu_item");
    assert_eq!(arvore.find("inverter").unwrap().kind, "toggle_menu_item");
    assert_eq!(marcados(&app), [true, false, false]);
    ordem.set(Ordem::Data);
    assert_eq!(marcados(&app), [false, false, true]);
}

#[test]
fn clicar_no_segundo_radio_marca_so_ele_muda_o_signal_e_fecha_o_menu() {
    let (mut app, ordem, _) = menu_de_radios(Theme::Light);
    app.click("ordenar");
    assert!(app.is_open("menu"));
    app.click("por-tamanho");
    assert_eq!(ordem.get_untracked(), Ordem::Tamanho);
    assert!(!app.is_open("menu"));
    app.click("ordenar");
    assert_eq!(marcados(&app), [false, true, false]);
}

#[test]
fn clicar_no_radio_ja_marcado_nao_muda_o_grupo() {
    let (mut app, ordem, _) = menu_de_radios(Theme::Light);
    let mudancas = Rc::new(Cell::new(0));
    let contador = Rc::clone(&mudancas);
    // O efeito só roda de novo se o signal for escrito.
    let _efeito = effect(move || {
        ordem.get();
        contador.set(contador.get() + 1);
    });
    app.click("ordenar");
    let antes = mudancas.get();
    app.click("por-nome");
    assert_eq!(ordem.get_untracked(), Ordem::Nome);
    assert_eq!(
        mudancas.get(),
        antes,
        "escolher o marcado não escreve no grupo"
    );
    assert_eq!(marcados(&app), [true, false, false]);
}

#[test]
fn enter_e_espaco_escolhem_o_radio_focado() {
    for tecla in [Key::Enter, Key::Space] {
        let (mut app, ordem, _) = menu_de_radios(Theme::Light);
        app.click("ordenar");
        for _ in 0..4 {
            if app.focused().as_deref() == Some("por-tamanho") {
                break;
            }
            app.key(Key::Tab);
        }
        assert_eq!(app.focused().as_deref(), Some("por-tamanho"));
        app.key(tecla);
        assert_eq!(ordem.get_untracked(), Ordem::Tamanho, "{tecla:?}");
        assert!(!app.is_open("menu"));
    }
}

#[test]
fn radio_e_toggle_dividem_a_coluna_da_marca() {
    let (mut app, _, _) = menu_de_radios(Theme::Light);
    app.click("ordenar");
    let radio = app.rect("por-nome");
    let toggle = app.rect("inverter");
    assert_eq!(radio.x0, toggle.x0);
    assert_eq!(radio.width(), toggle.width());
}

#[test]
fn snapshots_do_menu_com_radios() {
    for tema in TEMAS {
        let (mut app, _, inverter) = menu_de_radios(tema);
        inverter.set(true);
        app.click("ordenar");
        app.hover("por-tamanho");
        app.assert_lint();
        app.assert_snapshot(&format!("m41_radio_{}", comum::theme_name(tema)));
    }
}
