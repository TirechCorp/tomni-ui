//! Marco M9: camadas e popups (`ContentDialog`, `Flyout`, `MenuFlyout`, `ComboBox` e a dica
//! de `.tooltip()`), pelo `TestApp`.

use std::cell::RefCell;
use std::rc::Rc;

use tomni_ui::kurbo::Point;
use tomni_ui::prelude::*;
use tomni_ui::theme::Theme;
use tomni_ui::{Key, Modifiers, PointerKind, TOOLTIP_DELAY};
use tomni_ui_test::TestApp;

/// Largura e altura da janela da tela de conta.
const TAMANHO: (u32, u32) = (360, 560);

type Registro = Rc<RefCell<Vec<&'static str>>>;

/// Tela com um de cada popup. As ações vão para `registro`.
fn conta(registro: Registro) -> impl FnOnce() -> Form {
    move || {
        let nome = signal("Ana Souza".to_owned());
        let uf = signal(String::new());
        let excluir = signal(false);
        let ajuda = signal(false);
        let (r1, r2) = (Rc::clone(&registro), Rc::clone(&registro));
        Form::new("Minha conta").child(
            Column::new()
                .padding(16)
                .gap(12)
                .child(Edit::bind(nome).id("nome").height(44))
                .child(
                    ComboBox::bind(uf, ["MG", "RJ", "SP"])
                        .id("uf")
                        .placeholder("UF")
                        .height(44),
                )
                .child(
                    Row::new()
                        .gap(8)
                        .child(
                            Button::new("Mais opções").id("mais").height(44).flyout(
                                MenuFlyout::new()
                                    .id("menu")
                                    .item("sair", "Sair", move || r1.borrow_mut().push("sair"))
                                    .separator()
                                    .item("excluir-conta", "Excluir conta", move || {
                                        excluir.set(true);
                                    }),
                            ),
                        )
                        .child(
                            Button::new("Ajuda")
                                .id("ajuda")
                                .height(44)
                                .width(88)
                                .tooltip("Como mudar os dados")
                                .flyout(
                                    Flyout::new(
                                        Column::new()
                                            .gap(12)
                                            .child(
                                                Text::new("Toque num campo para mudá-lo.").wrap(),
                                            )
                                            .child(
                                                Button::new("OK")
                                                    .id("ajuda-ok")
                                                    .height(44)
                                                    .on_click(move || ajuda.set(false)),
                                            ),
                                    )
                                    .id("ajuda-popup")
                                    .open(ajuda),
                                ),
                        ),
                )
                .child(
                    ContentDialog::new("Excluir conta?", excluir)
                        .id("dialogo")
                        .content(
                            Text::new("Seus dados serão apagados e não dá para desfazer.").wrap(),
                        )
                        .primary("Excluir", move || r2.borrow_mut().push("excluída"))
                        .close("Cancelar"),
                ),
        )
    }
}

fn app() -> (TestApp, Registro) {
    let registro = Registro::default();
    let app = TestApp::new(conta(Rc::clone(&registro))).size(TAMANHO.0, TAMANHO.1);
    (app, registro)
}

/// Abre o dialog pelo menu, com o mouse.
fn abre_dialogo(app: &mut TestApp) {
    app.click("mais").click("excluir-conta");
    assert!(app.is_open("dialogo"));
}

#[test]
fn esc_fecha_cada_popup() {
    let (mut app, registro) = app();
    for (origem, popup) in [("mais", "menu"), ("uf", "uf"), ("ajuda", "ajuda-popup")] {
        app.click(origem);
        assert!(app.is_open(popup), "{popup} abre com o clique em {origem}");
        app.key(Key::Escape);
        assert!(!app.is_open(popup), "Esc fecha {popup}");
        assert_eq!(
            app.focused().as_deref(),
            Some(origem),
            "o foco volta a {origem}"
        );
    }
    abre_dialogo(&mut app);
    app.key(Key::Escape);
    assert!(!app.is_open("dialogo"));
    assert_eq!(app.text("uf"), "", "Esc não escolhe nada");
    assert!(
        registro.borrow().is_empty(),
        "Esc não roda ação: {registro:?}"
    );
}

#[test]
fn clique_fora_fecha_e_nao_chega_atras() {
    let (mut app, registro) = app();
    for (origem, popup) in [("mais", "menu"), ("uf", "uf"), ("ajuda", "ajuda-popup")] {
        app.click(origem);
        assert!(app.is_open(popup));
        // Em cima do campo de nome, que está atrás do popup ou ao lado.
        app.click("nome");
        assert!(!app.is_open(popup), "o clique fora fecha {popup}");
        assert_ne!(
            app.focused().as_deref(),
            Some("nome"),
            "o clique que fecha {popup} não chega ao campo"
        );
    }
    // O dialog é modal: o clique fora não fecha nem chega atrás.
    abre_dialogo(&mut app);
    app.click_at(Point::new(10.0, 10.0), 1);
    assert!(app.is_open("dialogo"), "clique fora não fecha o dialog");
    app.click("mais");
    assert!(
        !app.is_open("menu"),
        "o botão atrás do dialog não recebe o clique"
    );
    app.click("dialogo-close");
    assert!(!app.is_open("dialogo"));
    assert!(registro.borrow().is_empty(), "{registro:?}");
}

#[test]
fn tab_nao_sai_do_dialogo() {
    let (mut app, _) = app();
    abre_dialogo(&mut app);
    assert_eq!(app.focused().as_deref(), Some("dialogo-primary"));
    let mut vistos = Vec::new();
    for _ in 0..5 {
        app.tab();
        vistos.push(app.focused().unwrap_or_default());
    }
    for _ in 0..5 {
        app.shift_tab();
        vistos.push(app.focused().unwrap_or_default());
    }
    assert!(
        vistos.iter().all(|id| id.starts_with("dialogo-")),
        "o Tab saiu do dialog: {vistos:?}"
    );
    assert!(vistos.contains(&"dialogo-close".to_owned()), "{vistos:?}");
}

#[test]
fn foco_volta_a_origem_pelo_teclado() {
    let (mut app, registro) = app();
    // nome, uf, mais.
    app.tab().tab().tab();
    assert_eq!(app.focused().as_deref(), Some("mais"));
    app.key(Key::Enter);
    assert!(app.is_open("menu"));
    assert_eq!(
        app.focused().as_deref(),
        Some("sair"),
        "o foco vai para o primeiro item"
    );
    // O separador não recebe foco; ↓ no último fica nele.
    app.key(Key::Down).key(Key::Down);
    assert_eq!(app.focused().as_deref(), Some("excluir-conta"));
    app.key(Key::Up);
    assert_eq!(app.focused().as_deref(), Some("sair"));
    app.key(Key::End).key(Key::Enter);
    assert!(!app.is_open("menu"), "escolher um item fecha o menu");
    assert!(app.is_open("dialogo"), "o item abriu o dialog");
    assert_eq!(app.focused().as_deref(), Some("dialogo-primary"));
    app.key(Key::Escape);
    assert!(!app.is_open("dialogo"));
    assert_eq!(
        app.focused().as_deref(),
        Some("mais"),
        "o foco volta ao botão do menu"
    );
    assert!(registro.borrow().is_empty(), "{registro:?}");
}

#[test]
fn enter_no_dialogo_aciona_o_primario() {
    let (mut app, registro) = app();
    abre_dialogo(&mut app);
    // O foco abre no primário: Enter o aciona e fecha.
    app.key(Key::Enter);
    assert!(!app.is_open("dialogo"));
    assert_eq!(*registro.borrow(), ["excluída"]);
}

#[test]
fn menu_roda_a_acao_do_item() {
    let (mut app, registro) = app();
    app.click("mais").click("sair");
    assert!(!app.is_open("menu"));
    assert_eq!(*registro.borrow(), ["sair"]);
    assert_eq!(app.label("sair"), "Sair");
}

#[test]
fn flyout_fecha_pelo_signal() {
    let (mut app, _) = app();
    app.click("ajuda");
    assert!(app.is_open("ajuda-popup"));
    assert_eq!(app.focused().as_deref(), Some("ajuda-ok"));
    app.key(Key::Enter);
    assert!(!app.is_open("ajuda-popup"));
    assert_eq!(app.focused().as_deref(), Some("ajuda"));
}

#[test]
fn combo_box_pelo_teclado() {
    let (mut app, _) = app();
    app.tab().tab();
    assert_eq!(app.focused().as_deref(), Some("uf"));
    // Fechado, as setas trocam o valor direto.
    app.key(Key::Down);
    assert_eq!(app.text("uf"), "MG");
    app.key(Key::Down);
    assert_eq!(app.text("uf"), "RJ");
    app.key(Key::Up);
    assert_eq!(app.text("uf"), "MG");
    app.key(Key::End);
    assert_eq!(app.text("uf"), "SP");
    app.key(Key::Home);
    assert_eq!(app.text("uf"), "MG");
    assert!(!app.is_open("uf"));

    // Enter abre com o foco no valor escolhido; Enter no item escolhe e fecha.
    app.key(Key::Enter);
    assert!(app.is_open("uf"));
    assert_eq!(app.focused().as_deref(), Some("uf-mg"));
    app.key(Key::Down);
    assert_eq!(app.focused().as_deref(), Some("uf-rj"));
    assert_eq!(app.text("uf"), "MG", "andar na lista não escolhe");
    app.key(Key::Enter);
    assert!(!app.is_open("uf"));
    assert_eq!(app.text("uf"), "RJ");
    assert_eq!(app.focused().as_deref(), Some("uf"));

    // Espaço abre; Esc fecha sem mudar.
    app.key(Key::Space);
    assert!(app.is_open("uf"));
    app.key(Key::Down).key(Key::Escape);
    assert!(!app.is_open("uf"));
    assert_eq!(app.text("uf"), "RJ");

    // Alt+↓ abre.
    app.key_with(Key::Down, Modifiers::ALT);
    assert!(app.is_open("uf"));
    assert_eq!(app.focused().as_deref(), Some("uf-rj"));
    app.key(Key::Escape);
}

#[test]
fn combo_box_com_o_mouse() {
    let (mut app, _) = app();
    app.select("uf", "SP");
    assert_eq!(app.text("uf"), "SP");
    assert!(!app.is_open("uf"));
    let arvore = app.tree();
    let caixa = arvore.find("uf").unwrap();
    assert_eq!(caixa.text.as_deref(), Some("SP"));
    assert_eq!(arvore.find("uf-lista").unwrap().open, Some(false));
    app.click("uf");
    let arvore = app.tree();
    assert_eq!(arvore.find("uf-lista").unwrap().open, Some(true));
    assert_eq!(arvore.find("uf-sp").unwrap().selected, Some(true));
    assert_eq!(arvore.find("uf-mg").unwrap().selected, Some(false));
    // A lista tem a largura da caixa e abre logo abaixo dela.
    let (caixa, lista) = (app.rect("uf"), app.rect("uf-lista"));
    assert_eq!(lista.width(), caixa.width());
    assert_eq!(lista.x0, caixa.x0);
    assert!(lista.y0 >= caixa.y1, "{lista:?} abaixo de {caixa:?}");
}

/// `ComboBox` e um botão com menu no pé da tela.
fn rodape() -> Form {
    let uf = signal(String::new());
    Form::new("Rodapé").child(
        Column::new()
            .padding(16)
            .gap(12)
            .child(Column::new().grow(1))
            .child(
                ComboBox::bind(uf, ["MG", "RJ", "SP"])
                    .id("uf")
                    .placeholder("UF")
                    .height(44),
            )
            .child(
                Button::new("Mais opções").id("mais").height(44).flyout(
                    MenuFlyout::new()
                        .id("menu")
                        .item("sair", "Sair", || {})
                        .item("ajuda", "Ajuda", || {}),
                ),
            ),
    )
}

#[test]
fn perto_da_borda_de_baixo_abre_para_cima() {
    let mut app = TestApp::new(rodape).size(360, 400);
    app.click("uf");
    let (caixa, lista) = (app.rect("uf"), app.rect("uf-lista"));
    assert!(
        lista.y1 <= caixa.y0,
        "a lista {lista:?} abre acima de {caixa:?}"
    );
    assert!(lista.y0 >= 0.0);
    app.key(Key::Escape).click("mais");
    let (botao, menu) = (app.rect("mais"), app.rect("menu"));
    assert!(
        menu.y1 <= botao.y0,
        "o menu {menu:?} abre acima de {botao:?}"
    );
    app.assert_lint();
}

#[test]
fn dialogo_no_centro_da_janela() {
    let (mut app, _) = app();
    abre_dialogo(&mut app);
    let caixa = app.rect("dialogo");
    let (w, h) = (f64::from(TAMANHO.0), f64::from(TAMANHO.1));
    assert!((caixa.center().x - w / 2.0).abs() <= 1.0, "{caixa:?}");
    assert!((caixa.center().y - h / 2.0).abs() <= 1.0, "{caixa:?}");
    assert!(
        caixa.width() >= 320.0 && caixa.width() <= 548.0,
        "{caixa:?}"
    );
    assert!(caixa.height() >= 184.0, "{caixa:?}");
}

#[test]
fn dica_com_atraso_no_hover() {
    let (mut app, _) = app();
    app.hover("ajuda");
    assert!(!app.is_open("ajuda-tooltip"));
    app.wait(TOOLTIP_DELAY / 2);
    assert!(!app.is_open("ajuda-tooltip"), "ainda não deu o tempo");
    app.wait(TOOLTIP_DELAY / 2);
    assert!(app.is_open("ajuda-tooltip"));
    let (botao, dica) = (app.rect("ajuda"), app.rect("ajuda-tooltip"));
    assert!(
        dica.y1 <= botao.y0,
        "a dica {dica:?} fica acima de {botao:?}"
    );
    // O ponteiro sai: a dica fecha.
    app.ui()
        .pointer_move(Point::new(5.0, 540.0), PointerKind::Mouse);
    app.frame();
    assert!(!app.is_open("ajuda-tooltip"));
}

#[test]
fn dica_com_foco_do_teclado() {
    let (mut app, _) = app();
    app.tab().tab().tab().tab();
    assert_eq!(app.focused().as_deref(), Some("ajuda"));
    app.wait(TOOLTIP_DELAY);
    assert!(app.is_open("ajuda-tooltip"));
    app.tab();
    assert!(!app.is_open("ajuda-tooltip"), "o foco saiu: a dica fecha");
}

#[test]
fn dica_nao_abre_com_foco_do_mouse_e_fecha_com_clique() {
    let (mut app, _) = app();
    app.hover("ajuda").wait(TOOLTIP_DELAY);
    assert!(app.is_open("ajuda-tooltip"));
    app.click("ajuda");
    assert!(!app.is_open("ajuda-tooltip"), "o clique fecha a dica");
    assert!(app.is_open("ajuda-popup"));
}

#[test]
fn popups_abertos_na_arvore_e_no_lint() {
    for tema in [Theme::Light, Theme::Dark] {
        let registro = Registro::default();
        let mut app = TestApp::new(conta(registro))
            .size(TAMANHO.0, TAMANHO.1)
            .theme(tema);
        app.assert_lint();
        assert_eq!(app.tree().find("menu").unwrap().open, Some(false));
        for (origem, popup) in [
            ("mais", "menu"),
            ("uf", "uf-lista"),
            ("ajuda", "ajuda-popup"),
        ] {
            app.click(origem);
            assert_eq!(app.tree().find(popup).unwrap().open, Some(true), "{popup}");
            app.assert_lint();
            app.key(Key::Escape);
        }
        app.hover("ajuda").wait(TOOLTIP_DELAY);
        app.assert_lint();
        abre_dialogo(&mut app);
        let arvore = app.tree();
        let dialogo = arvore.find("dialogo").unwrap();
        assert_eq!(dialogo.open, Some(true));
        assert_eq!(dialogo.title.as_deref(), Some("Excluir conta?"));
        app.assert_lint();
    }
}

fn nome_do_tema(tema: Theme) -> &'static str {
    match tema {
        Theme::Light => "claro",
        Theme::Dark => "escuro",
    }
}

#[test]
fn snapshots_dos_popups_abertos() {
    for tema in [Theme::Light, Theme::Dark] {
        let t = nome_do_tema(tema);
        let novo = || {
            TestApp::new(conta(Registro::default()))
                .size(TAMANHO.0, TAMANHO.1)
                .theme(tema)
        };
        let mut app = novo();
        app.click("mais");
        app.ui()
            .pointer_move(Point::new(5.0, 540.0), PointerKind::Mouse);
        app.frame();
        app.assert_snapshot(&format!("m9_menu_{t}"));

        let mut app = novo();
        app.select("uf", "RJ").click("uf");
        app.hover("uf-sp");
        app.assert_snapshot(&format!("m9_combo_box_aberto_{t}"));

        let mut app = novo();
        app.click("ajuda");
        app.assert_snapshot(&format!("m9_flyout_{t}"));

        let mut app = novo();
        app.hover("ajuda").wait(TOOLTIP_DELAY);
        app.assert_snapshot(&format!("m9_dica_{t}"));

        let mut app = novo();
        abre_dialogo(&mut app);
        app.assert_snapshot(&format!("m9_dialogo_{t}"));
    }
}

/// Estados do `ComboBox` fechado.
#[derive(Clone, Copy, Debug)]
enum Estado {
    Rest,
    Hover,
    Pressed,
    Focused,
    Disabled,
}

#[test]
fn snapshots_do_combo_box_fechado() {
    for tema in [Theme::Light, Theme::Dark] {
        for estado in [
            Estado::Rest,
            Estado::Hover,
            Estado::Pressed,
            Estado::Focused,
            Estado::Disabled,
        ] {
            let desabilitado = matches!(estado, Estado::Disabled);
            let mut app = TestApp::new(move || {
                let uf = signal("São Paulo".to_owned());
                Form::new("UF").child(
                    Column::new().padding(16).child(
                        ComboBox::bind(uf, ["Minas Gerais", "São Paulo"])
                            .id("uf")
                            .width(200)
                            .height(44)
                            .disabled(desabilitado),
                    ),
                )
            })
            .size(232, 76)
            .theme(tema);
            match estado {
                Estado::Rest | Estado::Disabled => {}
                Estado::Hover => {
                    app.hover("uf");
                }
                Estado::Pressed => {
                    app.hover("uf");
                    let centro = app.rect("uf").center();
                    app.ui().pointer_down(centro, PointerKind::Mouse, 1);
                    app.frame();
                }
                Estado::Focused => {
                    app.tab();
                }
            }
            assert!(!app.is_open("uf"));
            let nome = format!("{estado:?}").to_lowercase();
            app.assert_snapshot(&format!("combo_box_{nome}_{}", nome_do_tema(tema)));
        }
        let mut app = TestApp::new(|| {
            let uf = signal(String::new());
            Form::new("UF").child(
                Column::new().padding(16).child(
                    ComboBox::bind(uf, ["MG", "SP"])
                        .id("uf")
                        .placeholder("Escolha a UF")
                        .width(200)
                        .height(44),
                ),
            )
        })
        .size(232, 76)
        .theme(tema);
        app.assert_lint();
        app.assert_snapshot(&format!("combo_box_placeholder_{}", nome_do_tema(tema)));
    }
}

/// Um diálogo com os três botões: o de fechar roda a ação dele e pode ficar escondido.
#[test]
fn o_botao_de_fechar_pode_ter_acao_e_ficar_escondido() {
    let registro = Registro::default();
    let (r1, r2, r3) = (
        Rc::clone(&registro),
        Rc::clone(&registro),
        Rc::clone(&registro),
    );
    let aberto = Rc::new(RefCell::new(None));
    let terceiro = Rc::new(RefCell::new(None));
    let (a, t) = (Rc::clone(&aberto), Rc::clone(&terceiro));
    let mut app = TestApp::new(move || {
        let dialogo = signal(true);
        let mostra = signal(true);
        *a.borrow_mut() = Some(dialogo);
        *t.borrow_mut() = Some(mostra);
        Form::new("Pedido").child(
            ContentDialog::new("Quer digitar?", dialogo)
                .id("dialogo")
                .content(Text::new("Numa aba de outra sessão.").wrap())
                .primary("Recusar", move || r1.borrow_mut().push("recusou"))
                .secondary("Permitir uma vez", move || r2.borrow_mut().push("permitiu"))
                .close_with("Passar a aba", move || r3.borrow_mut().push("passou"))
                .close_visible(move || mostra.get()),
        )
    })
    .size(TAMANHO.0, TAMANHO.1);
    assert!(app.visible("dialogo-close"));
    app.click("dialogo-close");
    assert_eq!(*registro.borrow(), ["passou"]);
    assert!(!app.is_open("dialogo"));

    // Com o botão escondido, sobram os dois primeiros; o Esc fecha sem ação.
    let aberto = aberto.borrow().unwrap();
    let mostra = terceiro.borrow().unwrap();
    mostra.set(false);
    aberto.set(true);
    app.frame();
    assert!(!app.visible("dialogo-close"));
    app.key(Key::Escape);
    assert_eq!(*registro.borrow(), ["passou"]);
}

#[test]
fn dialogo_esconde_o_secundario_quando_pedido() {
    let aberto = Rc::new(RefCell::new(None));
    let mostra_ref = Rc::new(RefCell::new(None));
    let (a, m) = (Rc::clone(&aberto), Rc::clone(&mostra_ref));
    let mut app = TestApp::new(move || {
        let dialogo = signal(true);
        let mostra = signal(false);
        *a.borrow_mut() = Some(dialogo);
        *m.borrow_mut() = Some(mostra);
        Form::new("Instalar").child(
            ContentDialog::new("Instalar no computador", dialogo)
                .id("dialogo")
                .content(Text::new("Escolha.").wrap())
                .primary("Aplicar", || {})
                .secondary("Desinstalar…", || {})
                .secondary_visible(move || mostra.get())
                .close("Cancelar"),
        )
    })
    .size(TAMANHO.0, TAMANHO.1);
    assert!(app.visible("dialogo-primary"));
    assert!(!app.visible("dialogo-secondary"));
    assert!(app.visible("dialogo-close"));
    mostra_ref.borrow().unwrap().set(true);
    app.frame();
    assert!(app.visible("dialogo-secondary"));
}
