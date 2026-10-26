//! Marco M14: `Icon`, os botões com ícone, `Image` e o botão de busca do `AutoSuggestBox`, nos
//! 2 temas, e o comportamento pelo `TestApp` e pelo lint.

mod comum;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use comum::{State, theme_name};
use tomni_ui::lint::{Issue, Rule, Severity};
use tomni_ui::prelude::*;
use tomni_ui::theme::Theme;
use tomni_ui::{Look, TOOLTIP_DELAY, ThemeColor};
use tomni_ui_test::TestApp;

const TEMAS: [Theme; 2] = [Theme::Light, Theme::Dark];

static PAISAGEM: &[u8] = include_bytes!("../../tomni-ui/examples/imagens/paisagem.png");
static LOGO: &[u8] = include_bytes!("../../tomni-ui/examples/imagens/logo.svg");

// Ícones

#[test]
fn grade_de_icones() {
    for theme in TEMAS {
        let mut app = TestApp::new(|| {
            let mut grade = Row::new().id("grade").wrap().gap(8).padding(12);
            for symbol in Symbol::ALL {
                grade = grade.child(Icon::new(symbol).id(symbol.name()).decorative());
            }
            Form::new("Ícones").child(grade)
        })
        .size(436, 408)
        .theme(theme);
        assert_eq!(
            app.rect("Home").size(),
            (20.0, 20.0).into(),
            "o padrão é 20 px"
        );
        app.assert_lint();
        app.assert_snapshot(&format!("icones_{}", theme_name(theme)));
    }
}

#[test]
fn icone_nos_quatro_tamanhos_e_com_cor() {
    for theme in TEMAS {
        let mut app = TestApp::new(|| {
            let mut linha = Row::new().gap(12).padding(12).align(Align::Center);
            for size in [12, 16, 20, 24] {
                linha = linha.child(
                    Icon::new(Symbol::Search)
                        .id(format!("busca-{size}"))
                        .size(size),
                );
            }
            Form::new("Tamanhos").child(
                linha.child(
                    Icon::new(Symbol::Heart)
                        .id("coracao")
                        .size(24)
                        .color(Token::AccentTextFillColorPrimary),
                ),
            )
        })
        .size(180, 48)
        .theme(theme);
        for size in [12.0, 16.0, 20.0, 24.0] {
            assert_eq!(
                app.rect(&format!("busca-{size}")).size(),
                (size, size).into()
            );
        }
        app.assert_lint();
        app.assert_snapshot(&format!("icone_tamanhos_{}", theme_name(theme)));
    }
}

#[test]
fn outro_tamanho_escala_o_desenho() {
    let app = TestApp::new(|| {
        Form::new("Tamanho").child(
            Column::new()
                .align(Align::Start)
                .child(Icon::new(Symbol::Add).id("grande").size(32)),
        )
    })
    .size(100, 100);
    assert_eq!(app.rect("grande").size(), (32.0, 32.0).into());
}

#[test]
fn arvore_mostra_o_icone() {
    let app = TestApp::new(|| {
        Form::new("Árvore").child(
            Row::new()
                .child(Icon::new(Symbol::Mail).id("correio"))
                .child(Icon::new(Symbol::Star).id("enfeite").decorative()),
        )
    })
    .size(100, 40);
    let arvore = app.tree();
    let correio = arvore.find("correio").unwrap();
    assert_eq!(correio.kind, "icon");
    assert_eq!(correio.icon, Some("Mail"));
    assert!(!correio.decorative);
    assert!(arvore.find("enfeite").unwrap().decorative);
}

fn icone_sobre(cor: Color, decorativo: bool) -> TestApp {
    TestApp::new(move || {
        let icone = Icon::new(Symbol::Alert).id("alerta").color(cor);
        let icone = if decorativo {
            icone.decorative()
        } else {
            icone
        };
        Form::new("Contraste").child(Column::new().padding(12).child(icone))
    })
    .size(100, 50)
}

#[test]
fn contraste_do_icone_e_3_para_1() {
    // #A0A0A0 sobre o fundo claro (#F3F3F3) dá 2,3:1; #808080, 3,5:1.
    let mut app = icone_sobre(Color::hex(0xA0A0A0), false);
    let issues = app.lint();
    let issue = issues
        .iter()
        .find(|i| i.rule == Rule::Contrast)
        .unwrap_or_else(|| panic!("ícone claro sobre fundo claro: {issues:?}"));
    assert_eq!(issue.widget, "alerta");
    assert_eq!(issue.severity, Severity::Error);
    assert!(issue.message.contains("Alert"), "{}", issue.message);
    assert!(issue.message.contains("3,00:1"), "{}", issue.message);
    icone_sobre(Color::hex(0x808080), false).assert_lint();
    icone_sobre(Color::hex(0xA0A0A0), true).assert_lint();
}

fn card_com_icone(state: State) -> TestApp {
    let view = move || {
        Form::new("Card").child(
            Column::new().padding(12).child(
                Card::new()
                    .id("card")
                    .label("Correio")
                    .on_click(|| {})
                    .disabled(state == State::Disabled)
                    .child(
                        Row::new()
                            .gap(8)
                            .child(Icon::new(Symbol::Mail).id("icone"))
                            .child(
                                Icon::new(Symbol::Star)
                                    .id("destaque")
                                    .color(Token::AccentTextFillColorPrimary),
                            ),
                    ),
            ),
        )
    };
    let mut app = TestApp::new(view).size(200, 80);
    state.apply(&mut app, "card");
    app
}

#[test]
fn icone_herda_a_cor_do_card() {
    let cor = |app: &TestApp, id: &str| match app.look(id) {
        Look::Icon(visual) => visual.color,
        look => panic!("{id:?} não é um ícone: {look:?}"),
    };
    let app = card_com_icone(State::Rest);
    assert_eq!(cor(&app, "icone"), None, "em repouso, a cor do texto");
    let app = card_com_icone(State::Pressed);
    assert_eq!(
        cor(&app, "icone"),
        Some(ThemeColor::Token(Token::TextFillColorSecondary))
    );
    assert_eq!(
        cor(&app, "destaque"),
        Some(ThemeColor::Token(Token::AccentTextFillColorPrimary)),
        "uma cor própria fica"
    );
    let app = card_com_icone(State::Disabled);
    assert_eq!(
        cor(&app, "icone"),
        Some(ThemeColor::Token(Token::TextFillColorDisabled))
    );
}

// Botões com ícone

fn botao_app(theme: Theme, state: State) -> TestApp {
    let view = move || {
        Form::new("Botão").child(
            Row::new().gap(8).padding(12).child(
                Button::new("Salvar")
                    .icon(Symbol::Save)
                    .id("salvar")
                    .on_click(|| {})
                    .disabled(state == State::Disabled),
            ),
        )
    };
    let mut app = TestApp::new(view).size(140, 56).theme(theme);
    state.apply(&mut app, "salvar");
    app
}

#[test]
fn botao_com_icone_nos_cinco_estados() {
    for theme in TEMAS {
        for state in State::ALL {
            botao_app(theme, state).assert_snapshot(&format!(
                "botao_icone_{}_{}",
                state.name(),
                theme_name(theme)
            ));
        }
    }
}

#[test]
fn icone_fica_8_px_antes_do_rotulo() {
    let app = botao_app(Theme::Light, State::Rest);
    let Look::Button { label, visual } = app.look("salvar") else {
        panic!("não é botão");
    };
    assert_eq!(label, "Salvar");
    assert_eq!(visual.icon, Some(Symbol::Save));
    assert!(!visual.icon_only);
    assert_eq!(app.tree().find("salvar").unwrap().icon, Some("Save"));
    // O botão sem ícone é 24 px (16 do ícone e 8 de espaço) mais estreito.
    let sem_icone = TestApp::new(|| {
        Form::new("Botão").child(
            Row::new()
                .padding(12)
                .child(Button::new("Salvar").id("salvar")),
        )
    })
    .size(140, 56);
    assert_eq!(
        app.rect("salvar").width() - sem_icone.rect("salvar").width(),
        24.0
    );
}

fn so_icone_app(theme: Theme) -> TestApp {
    TestApp::new(|| {
        Form::new("Só ícone").child(
            Row::new()
                .gap(4)
                .padding(12)
                .child(
                    Button::icon_only(Symbol::Settings)
                        .id("ajustes")
                        .label("Ajustes"),
                )
                .child(
                    Button::icon_only(Symbol::Add)
                        .id("novo")
                        .label("Novo")
                        .accent(),
                )
                .child(
                    ToggleButton::icon_only(signal(false), Symbol::Star)
                        .id("favorito")
                        .label("Favorito"),
                )
                .child(
                    ToggleButton::icon_only(signal(true), Symbol::Pin)
                        .id("fixar")
                        .label("Fixar"),
                )
                .child(
                    Button::icon_only(Symbol::Delete)
                        .id("apagar")
                        .label("Apagar")
                        .disabled(true),
                ),
        )
    })
    .size(260, 68)
    .theme(theme)
}

#[test]
fn botoes_so_de_icone() {
    for theme in TEMAS {
        let mut app = so_icone_app(theme);
        for id in ["ajustes", "novo", "favorito", "fixar", "apagar"] {
            assert_eq!(app.rect(id).size(), (44.0, 44.0).into(), "{id}");
        }
        assert_eq!(app.label("ajustes"), "Ajustes");
        assert_eq!(app.label("favorito"), "Favorito");
        assert!(app.checked("fixar"));
        app.assert_lint();
        app.assert_snapshot(&format!("botoes_so_icone_{}", theme_name(theme)));
    }
}

#[test]
fn rotulo_do_botao_so_de_icone_vira_a_dica() {
    let mut app = so_icone_app(Theme::Light);
    let arvore = app.tree();
    let ajustes = arvore.find("ajustes").unwrap();
    assert_eq!(ajustes.label.as_deref(), Some("Ajustes"));
    assert_eq!(ajustes.icon, Some("Settings"));
    app.hover("favorito");
    app.wait(TOOLTIP_DELAY + Duration::from_millis(10));
    assert!(app.is_open("favorito-tooltip"));
    let arvore = app.tree();
    let dica = arvore.find("favorito-tooltip").unwrap();
    assert_eq!(dica.children[0].text.as_deref(), Some("Favorito"));
}

#[test]
fn botao_so_de_icone_responde_ao_clique() {
    let cliques = signal(0);
    let favorito = signal(false);
    let mut app = TestApp::new(move || {
        Form::new("Cliques").child(
            Row::new()
                .padding(12)
                .child(
                    Button::icon_only(Symbol::Add)
                        .id("mais")
                        .label("Mais um")
                        .on_click(move || cliques.update(|c| *c += 1)),
                )
                .child(
                    ToggleButton::icon_only(favorito, Symbol::Star)
                        .id("favorito")
                        .label("Favorito"),
                ),
        )
    })
    .size(200, 68);
    app.click("mais");
    app.click("mais");
    assert_eq!(cliques.get_untracked(), 2);
    app.click("favorito");
    assert!(favorito.get_untracked());
}

#[test]
fn botao_so_de_icone_sem_rotulo_tem_aviso() {
    let mut app = TestApp::new(|| {
        Form::new("Sem rótulo").child(
            Row::new()
                .padding(12)
                .child(Button::icon_only(Symbol::Settings).id("ajustes"))
                .child(ToggleButton::icon_only(signal(false), Symbol::Star).id("favorito")),
        )
    })
    .size(200, 68);
    let issues = app.lint();
    let avisos: Vec<&str> = issues
        .iter()
        .filter(|i| i.rule == Rule::MissingLabel)
        .map(|i| i.widget.as_str())
        .collect();
    assert_eq!(avisos, ["ajustes", "favorito"], "{issues:?}");
    assert!(
        issues
            .iter()
            .filter(|i| i.rule == Rule::MissingLabel)
            .all(|i| i.severity == Severity::Warning)
    );
    app.assert_lint();
    assert_eq!(app.tree().find("ajustes").unwrap().label, None);
}

// Imagens

/// A paisagem (96×64) numa caixa de 120×48 com fundo cinza, para ver o que cada `Stretch` faz.
fn caixa(stretch: Stretch) -> Border {
    Border::new()
        .background(Token::TextFillColorTertiary)
        .child(
            Image::bytes(PAISAGEM)
                .id(stretch_id(stretch))
                .width(120)
                .height(48)
                .stretch(stretch)
                .alt("Colinas verdes sob o sol"),
        )
}

fn stretch_id(stretch: Stretch) -> &'static str {
    match stretch {
        Stretch::None => "none",
        Stretch::Fill => "fill",
        Stretch::Uniform => "uniform",
        Stretch::UniformToFill => "uniform-to-fill",
    }
}

#[test]
fn imagem_nos_quatro_stretch() {
    for theme in TEMAS {
        let mut app = TestApp::new(|| {
            Form::new("Stretch").child(
                UniformGrid::new()
                    .padding(12)
                    .gap(12)
                    .min_column_width(120)
                    .child(caixa(Stretch::Uniform))
                    .child(caixa(Stretch::UniformToFill))
                    .child(caixa(Stretch::Fill))
                    .child(caixa(Stretch::None)),
            )
        })
        .size(276, 132)
        .theme(theme);
        for stretch in [
            Stretch::Uniform,
            Stretch::UniformToFill,
            Stretch::Fill,
            Stretch::None,
        ] {
            let id = stretch_id(stretch);
            assert_eq!(app.rect(id).size(), (120.0, 48.0).into(), "{id}");
            assert_eq!(app.tree().find(id).unwrap().variant, Some(stretch.name()));
        }
        app.assert_lint();
        app.assert_snapshot(&format!("imagem_stretch_{}", theme_name(theme)));
    }
}

#[test]
fn imagem_com_cantos_e_tamanho_natural() {
    for theme in TEMAS {
        let mut app = TestApp::new(|| {
            Form::new("Cantos").child(
                Row::new()
                    .gap(12)
                    .padding(12)
                    .child(Image::bytes(PAISAGEM).id("natural").alt("Paisagem"))
                    .child(
                        Image::bytes(PAISAGEM)
                            .id("redonda")
                            .corner_radius(16)
                            .alt("Paisagem com cantos arredondados"),
                    ),
            )
        })
        .size(228, 88)
        .theme(theme);
        assert_eq!(app.rect("natural").size(), (96.0, 64.0).into());
        app.assert_lint();
        app.assert_snapshot(&format!("imagem_cantos_{}", theme_name(theme)));
    }
}

#[test]
fn imagem_encolhe_na_proporcao() {
    let app = TestApp::new(|| {
        Form::new("Estreita").child(
            Column::new()
                .padding(12)
                .child(Image::bytes(PAISAGEM).id("paisagem").alt("Paisagem")),
        )
    })
    .size(72, 100);
    // 48 px de largura útil: metade da largura, metade da altura.
    assert_eq!(app.rect("paisagem").size(), (48.0, 32.0).into());
}

#[test]
fn svg_no_tamanho_natural_e_ampliado() {
    for theme in TEMAS {
        let mut app = TestApp::new(|| {
            Form::new("SVG").child(
                Row::new()
                    .gap(12)
                    .padding(12)
                    .align(Align::End)
                    .child(Image::bytes(LOGO).id("logo").alt("Logotipo"))
                    .child(
                        Image::bytes(LOGO)
                            .id("grande")
                            .width(96)
                            .height(96)
                            .alt("Logotipo ampliado"),
                    ),
            )
        })
        .size(180, 120)
        .theme(theme);
        assert_eq!(app.rect("logo").size(), (48.0, 48.0).into());
        assert_eq!(app.rect("grande").size(), (96.0, 96.0).into());
        assert_eq!(app.tree().find("logo").unwrap().error, None);
        app.assert_lint();
        app.assert_snapshot(&format!("svg_{}", theme_name(theme)));
    }
}

#[test]
fn imagem_que_nao_abre_vira_quadro_vazio() {
    let mut app = TestApp::new(|| {
        Form::new("Erros").child(
            Column::new()
                .gap(8)
                .padding(12)
                .child(
                    Image::bytes(b"isto nao e uma imagem".to_vec())
                        .id("invalida")
                        .width(100)
                        .height(50)
                        .alt("Foto"),
                )
                .child(
                    Image::file("/caminho/que/nao/existe.png")
                        .id("sumida")
                        .alt("Foto"),
                )
                .child(
                    Image::bytes(&b"\x89PNG\r\n\x1a\n quebrado"[..])
                        .id("truncada")
                        .alt("Foto"),
                ),
        )
    })
    .size(200, 120);
    assert_eq!(
        app.rect("invalida").size(),
        (100.0, 50.0).into(),
        "o tamanho pedido fica"
    );
    assert_eq!(
        app.rect("sumida").height(),
        0.0,
        "a que não abre não ocupa altura"
    );
    let arvore = app.tree();
    for id in ["invalida", "sumida", "truncada"] {
        let erro = arvore.find(id).unwrap().error.clone();
        assert!(erro.is_some(), "{id} sem erro na árvore");
    }
    let sumida = arvore.find("sumida").unwrap().error.clone().unwrap();
    assert!(sumida.contains("/caminho/que/nao/existe.png"), "{sumida}");
    // Pinta sem pânico: só o fundo.
    let vazio = TestApp::new(|| Form::new("Erros").child(Column::new()))
        .size(200, 120)
        .snapshot();
    assert_eq!(app.snapshot().data(), vazio.data());
    app.assert_lint();
}

#[test]
fn imagem_sem_alt_e_erro() {
    let mut app = TestApp::new(|| {
        Form::new("Sem alt").child(
            Row::new()
                .padding(12)
                .child(Image::bytes(LOGO).id("sem-alt"))
                .child(Image::bytes(LOGO).id("enfeite").decorative())
                .child(Image::bytes(LOGO).id("com-alt").alt("Logotipo")),
        )
    })
    .size(200, 80);
    let issues = app.lint();
    let erros: Vec<&Issue> = issues
        .iter()
        .filter(|i| i.rule == Rule::MissingAlt)
        .collect();
    assert_eq!(erros.len(), 1, "{issues:?}");
    assert_eq!(erros[0].widget, "sem-alt");
    assert_eq!(erros[0].severity, Severity::Error);
    assert!(erros[0].message.contains(".alt("), "{}", erros[0].message);
    let arvore = app.tree();
    assert_eq!(
        arvore.find("com-alt").unwrap().alt.as_deref(),
        Some("Logotipo")
    );
    assert!(arvore.find("enfeite").unwrap().decorative);
}

// Busca do AutoSuggestBox

const CIDADES: [&str; 3] = ["Recife", "Rio Branco", "Rio de Janeiro"];

fn busca_app(theme: Theme, desabilitada: bool) -> (TestApp, Rc<RefCell<Vec<String>>>) {
    let log = Rc::new(RefCell::new(Vec::new()));
    let enviados = Rc::clone(&log);
    let app = TestApp::new(move || {
        let enviados = Rc::clone(&enviados);
        Form::new("Busca").child(
            Column::new().padding(12).child(
                AutoSuggestBox::bind(signal(String::new()), |texto: &str| {
                    let texto = texto.to_lowercase();
                    if texto.is_empty() {
                        return Vec::new();
                    }
                    CIDADES
                        .iter()
                        .filter(|c| c.to_lowercase().starts_with(&texto))
                        .map(|c| (*c).to_owned())
                        .collect()
                })
                .id("cidade")
                .placeholder("Cidade")
                .height(44)
                .query_icon(Symbol::Search)
                .disabled(desabilitada)
                .on_submit(move |c| enviados.borrow_mut().push(c)),
            ),
        )
    })
    .size(300, 200)
    .theme(theme);
    (app, log)
}

#[test]
fn busca_nos_dois_temas() {
    for theme in TEMAS {
        let (mut app, _) = busca_app(theme, false);
        app.assert_lint();
        app.assert_snapshot(&format!("busca_vazia_{}", theme_name(theme)));
        app.type_into("cidade", "Ri");
        app.hover("cidade-buscar");
        app.assert_lint();
        app.assert_snapshot(&format!("busca_com_texto_{}", theme_name(theme)));
        let (app, _) = busca_app(theme, true);
        app.assert_snapshot(&format!("busca_desabilitada_{}", theme_name(theme)));
    }
}

#[test]
fn botao_de_busca_chama_o_on_submit_com_o_texto() {
    let (mut app, log) = busca_app(Theme::Light, false);
    assert_eq!(app.label("cidade-buscar"), "Buscar");
    assert_eq!(
        app.tree().find("cidade-buscar").unwrap().icon,
        Some("Search")
    );
    app.click("cidade-buscar");
    assert_eq!(*log.borrow(), [String::new()], "vazio também busca");
    app.type_into("cidade", "Ri");
    assert!(app.is_open("cidade"));
    app.click("cidade-buscar");
    assert_eq!(*log.borrow(), ["", "Ri"]);
    assert!(!app.is_open("cidade"), "a busca fecha a lista");
    assert_eq!(app.text("cidade"), "Ri");

    let (mut app, log) = busca_app(Theme::Light, true);
    assert!(!app.enabled("cidade-buscar"));
    app.click("cidade-buscar");
    assert!(log.borrow().is_empty());
}
