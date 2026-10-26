//! Marco M13: `Border`, `Card`, `Row::wrap`, `UniformGrid`, a rampa completa e o texto cortado
//! com `max_lines`, nos 2 temas, e o comportamento pelo `TestApp` e pelo lint.

mod comum;

use comum::{State, theme_name};
use tomni_ui::lint::Rule;
use tomni_ui::prelude::*;
use tomni_ui::theme::Theme;
use tomni_ui::{Key, Look, ThemeColor};
use tomni_ui_test::TestApp;

const TEMAS: [Theme; 2] = [Theme::Light, Theme::Dark];

fn card(id: &str, titulo: &'static str, descricao: &'static str) -> Card {
    Card::new().id(id).child(
        Column::new()
            .gap(4)
            .child(Text::new(titulo).body_strong().id(format!("{id}-titulo")))
            .child(
                Text::new(descricao)
                    .secondary()
                    .wrap()
                    .max_lines(2)
                    .id(format!("{id}-descricao")),
            ),
    )
}

fn card_app(theme: Theme, state: State) -> TestApp {
    let view = move || {
        Form::new("Card").child(
            Column::new().padding(12).child(
                card(
                    "tema",
                    "Tema escuro",
                    "Menos luz à noite, com as mesmas cores de destaque.",
                )
                .label("Tema escuro")
                .on_click(|| {})
                .disabled(state == State::Disabled),
            ),
        )
    };
    let mut app = TestApp::new(view).size(300, 122).theme(theme);
    state.apply(&mut app, "tema");
    app
}

#[test]
fn card_nos_cinco_estados() {
    for theme in TEMAS {
        for state in State::ALL {
            card_app(theme, state).assert_snapshot(&format!(
                "card_{}_{}",
                state.name(),
                theme_name(theme)
            ));
        }
    }
}

#[test]
fn texto_do_card_herda_a_cor_do_estado() {
    let cor = |app: &TestApp, id: &str| match app.look(id) {
        Look::Text(visual) => visual.color,
        look => panic!("{id:?} não é um texto: {look:?}"),
    };
    let app = card_app(Theme::Light, State::Rest);
    assert_eq!(cor(&app, "tema-titulo"), None, "em repouso, a cor primária");
    let app = card_app(Theme::Light, State::Pressed);
    assert_eq!(
        cor(&app, "tema-titulo"),
        Some(ThemeColor::Token(Token::TextFillColorSecondary)),
    );
    let app = card_app(Theme::Light, State::Disabled);
    assert_eq!(
        cor(&app, "tema-titulo"),
        Some(ThemeColor::Token(Token::TextFillColorDisabled)),
    );
    assert_eq!(
        cor(&app, "tema-descricao"),
        Some(ThemeColor::Token(Token::TextFillColorDisabled)),
        "desabilitado, a cor secundária também apaga, como no SettingsCard",
    );
    let app = card_app(Theme::Light, State::Pressed);
    assert_eq!(
        cor(&app, "tema-descricao"),
        Some(ThemeColor::Token(Token::TextFillColorSecondary)),
        "pressionado, a cor própria do texto vale sobre a herdada",
    );
}

fn cards_clicaveis() -> TestApp {
    TestApp::new(|| {
        let cliques = signal(0);
        Form::new("Cards").child(
            Column::new()
                .padding(24)
                .gap(12)
                .child(
                    card("a", "Primeiro", "Abre a primeira página.")
                        .label("Primeiro")
                        .on_click(move || cliques.update(|n| *n += 1)),
                )
                .child(
                    card("b", "Segundo", "Abre a segunda página.")
                        .label("Segundo")
                        .on_click(move || cliques.update(|n| *n += 10)),
                )
                .child(Text::new(move || format!("{} cliques", cliques.get())).id("total")),
        )
    })
}

fn foco(app: &TestApp, id: &str) -> bool {
    match app.look(id) {
        Look::Card(visual) => visual.focus_ring,
        look => panic!("{id:?} não é um card: {look:?}"),
    }
}

#[test]
fn card_responde_ao_clique_ao_enter_e_ao_espaco() {
    let mut app = cards_clicaveis();
    app.click("a");
    assert_eq!(app.text("total"), "1 cliques");
    assert_eq!(app.focused().as_deref(), Some("a"), "o clique dá o foco");
    assert!(!foco(&app, "a"), "sem anel de foco pelo ponteiro");
    app.key(Key::Enter);
    assert_eq!(app.text("total"), "2 cliques");
    app.key(Key::Space);
    assert_eq!(app.text("total"), "3 cliques");
    app.tab();
    assert_eq!(app.focused().as_deref(), Some("b"));
    assert!(foco(&app, "b"), "anel de foco pelo teclado");
    app.key(Key::Space);
    assert_eq!(app.text("total"), "13 cliques");
    assert_eq!(app.label("b"), "Segundo");
}

#[test]
fn card_sem_clique_nao_recebe_foco_nem_estado() {
    let mut app = TestApp::new(|| {
        Form::new("Cards").child(Column::new().padding(24).child(card(
            "info",
            "Informação",
            "Só leitura.",
        )))
    });
    app.tab();
    assert_eq!(app.focused(), None);
    let tree = app.tree();
    let node = tree.find("info").unwrap();
    assert_eq!(node.kind, "card");
    assert_eq!(node.state, None);
    assert_eq!(
        node.background.as_deref(),
        Some("CardBackgroundFillColorDefault")
    );
    assert_eq!(node.corner_radius, Some(8.0));
    assert!(app.lint().iter().all(|i| i.rule != Rule::MissingLabel));
}

#[test]
fn arvore_mostra_o_card_e_o_lint_passa() {
    for theme in TEMAS {
        let mut app = cards_clicaveis().theme(theme);
        app.hover("a");
        let tree = app.tree();
        let node = tree.find("a").unwrap();
        assert_eq!(node.label.as_deref(), Some("Primeiro"));
        assert_eq!(node.state, Some("hover"));
        assert_eq!(
            node.background.as_deref(),
            Some("ControlFillColorSecondary")
        );
        app.assert_lint();
    }
}

#[test]
fn card_clicavel_sem_rotulo_tem_aviso() {
    let mut app = TestApp::new(|| {
        Form::new("Cards").child(
            Column::new()
                .padding(24)
                .child(card("sem-rotulo", "Sem rótulo", "Falta o .label.").on_click(|| {})),
        )
    });
    let issues = app.lint();
    let issue = issues
        .iter()
        .find(|i| i.rule == Rule::MissingLabel)
        .expect("aviso missing-label");
    assert_eq!(issue.widget, "sem-rotulo");
    assert!(issue.message.contains(".label("), "{}", issue.message);
}

fn gradiente() -> Brush {
    Brush::linear(
        (0.0, 0.0),
        (1.0, 1.0),
        [(0.0, Color::hex(0x003E92)), (1.0, Color::hex(0x6B2FA0))],
    )
}

fn border_app(theme: Theme) -> TestApp {
    TestApp::new(|| {
        Form::new("Border").child(
            Column::new().padding(12).child(
                Border::new()
                    .id("destaque")
                    .background(gradiente())
                    .border(Token::CardStrokeColorDefault, 1)
                    .corner_radius(8)
                    .padding(24)
                    .child(
                        Column::new()
                            .gap(4)
                            .child(Text::new("Tomni UI").title().color(Color::WHITE))
                            .child(
                                Text::new("Interfaces nativas em Rust.")
                                    .color(Color::hex(0xE8E8F8)),
                            ),
                    ),
            ),
        )
    })
    .size(320, 140)
    .theme(theme)
}

#[test]
fn border_com_gradiente() {
    for theme in TEMAS {
        let mut app = border_app(theme);
        app.assert_snapshot(&format!("border_gradiente_{}", theme_name(theme)));
        app.assert_lint();
        let tree = app.tree();
        let node = tree.find("destaque").unwrap();
        assert_eq!(node.kind, "border");
        assert_eq!(
            node.background.as_deref(),
            Some("linear-gradient(#003E92 0%, #6B2FA0 100%)")
        );
        assert_eq!(node.corner_radius, Some(8.0));
    }
}

#[test]
fn contraste_usa_o_fundo_do_border() {
    let tela = |claro: bool| {
        move || {
            let texto = Text::new("Texto sobre o fundo escuro").id("texto");
            let texto = if claro {
                texto.color(Color::WHITE)
            } else {
                texto
            };
            Form::new("Contraste").child(
                Column::new().padding(12).child(
                    Border::new()
                        .background(Color::hex(0x202020))
                        .padding(12)
                        .child(texto),
                ),
            )
        }
    };
    let mut app = TestApp::new(tela(false)).size(320, 100);
    let issues = app.lint();
    let issue = issues
        .iter()
        .find(|i| i.rule == Rule::Contrast)
        .expect("texto escuro sobre o Border escuro");
    assert_eq!(issue.widget, "texto");
    assert!(issue.message.contains("#202020"), "{}", issue.message);
    TestApp::new(tela(true)).size(320, 100).assert_lint();

    // Num gradiente, vale a ponta mais perto do texto.
    let mut app = TestApp::new(|| {
        Form::new("Contraste").child(
            Column::new().padding(12).child(
                Border::new()
                    .background(Brush::linear(
                        (0.0, 0.0),
                        (0.0, 1.0),
                        [(0.0, Color::hex(0x202020)), (1.0, Color::WHITE)],
                    ))
                    .child(
                        Column::new()
                            .gap(40)
                            .padding(8)
                            .child(Text::new("Em cima").id("cima").color(Color::WHITE))
                            .child(Text::new("Embaixo").id("baixo").color(Color::WHITE)),
                    ),
            ),
        )
    })
    .size(320, 140);
    let issues = app.lint();
    let ruins: Vec<&str> = issues
        .iter()
        .filter(|i| i.rule == Rule::Contrast)
        .map(|i| i.widget.as_str())
        .collect();
    assert_eq!(ruins, ["baixo"]);
}

#[test]
fn contraste_ignora_o_que_esta_num_card_desabilitado() {
    let mut app = card_app(Theme::Light, State::Disabled);
    assert!(
        app.lint().iter().all(|i| i.rule != Rule::Contrast),
        "{:?}",
        app.lint()
    );
}

fn botoes() -> Row {
    let mut row = Row::new().id("linha").wrap().gap(8);
    for (i, nome) in [
        "Copiar",
        "Recortar",
        "Colar",
        "Desfazer",
        "Refazer",
        "Selecionar tudo",
    ]
    .into_iter()
    .enumerate()
    {
        row = row.child(Button::new(nome).id(format!("b{i}")).height(44));
    }
    row
}

fn linha_app(theme: Theme, largura: u32) -> TestApp {
    TestApp::new(|| Form::new("Linha").child(Column::new().padding(12).child(botoes())))
        .size(largura, 180)
        .theme(theme)
}

#[test]
fn linha_que_quebra() {
    for theme in TEMAS {
        for largura in [420, 240] {
            linha_app(theme, largura)
                .assert_snapshot(&format!("row_wrap_{largura}_{}", theme_name(theme)));
        }
    }
}

#[test]
fn linha_com_wrap_nunca_corta_um_filho() {
    for largura in [600, 420, 300, 240] {
        let mut app = linha_app(Theme::Light, largura);
        let linha = app.rect("linha");
        let rects: Vec<_> = (0..6).map(|i| app.rect(&format!("b{i}"))).collect();
        for (i, rect) in rects.iter().enumerate() {
            assert!(
                linha.inflate(0.5, 0.5).contains_rect(*rect),
                "b{i} {rect:?} sai da linha {linha:?} com {largura} px"
            );
            for (j, outro) in rects.iter().enumerate().skip(i + 1) {
                assert!(
                    rect.intersect(*outro).area() == 0.0,
                    "b{i} e b{j} se sobrepõem com {largura} px"
                );
            }
        }
        let linhas = {
            let mut ys: Vec<i64> = rects.iter().map(|r| r.y0 as i64).collect();
            ys.dedup();
            ys.len()
        };
        if largura == 600 {
            assert_eq!(linhas, 1);
        } else {
            assert!(linhas > 1, "{largura} px numa linha só");
        }
        // O gap vale entre as linhas também.
        if linhas > 1 {
            let segunda = rects.iter().find(|r| r.y0 > rects[0].y0).unwrap();
            assert_eq!(segunda.y0 - rects[0].y1, 8.0);
        }
        app.assert_lint();
    }
}

fn grade_app(theme: Theme, largura: u32) -> TestApp {
    TestApp::new(|| {
        let mut grade = UniformGrid::new().id("grade").min_column_width(160).gap(8);
        for (i, (titulo, descricao)) in [
            ("Botões", "Button, ToggleButton e HyperlinkButton."),
            ("Texto", "Rampa de tipos, cor e texto cortado."),
            ("Cards", "Border e Card, clicáveis ou não."),
            ("Listas", "ComboBox, RadioButtons e AutoSuggestBox."),
            ("Progresso", "ProgressBar e ProgressRing."),
            ("Navegação", "TabView e Expander."),
        ]
        .into_iter()
        .enumerate()
        {
            grade = grade.child(card(&format!("c{i}"), titulo, descricao));
        }
        Form::new("Grade").child(ScrollViewer::new().child(Column::new().padding(12).child(grade)))
    })
    .size(largura, 480)
    .theme(theme)
}

fn colunas(app: &TestApp) -> usize {
    let mut xs: Vec<i64> = (0..6)
        .map(|i| app.rect(&format!("c{i}")).x0 as i64)
        .collect();
    xs.sort_unstable();
    xs.dedup();
    xs.len()
}

#[test]
fn grade_uniforme_com_uma_duas_e_tres_colunas() {
    for theme in TEMAS {
        for (largura, n) in [(240, 1), (360, 2), (540, 3)] {
            let mut app = grade_app(theme, largura);
            assert_eq!(colunas(&app), n, "{largura} px");
            app.assert_snapshot(&format!("uniform_grid_{n}_colunas_{}", theme_name(theme)));
            app.assert_lint();
        }
    }
}

#[test]
fn grade_muda_as_colunas_com_a_largura_e_mantem_as_larguras_iguais() {
    let mut app = grade_app(Theme::Light, 240);
    let mut anterior = 0;
    for largura in [240, 340, 400, 520, 700, 900] {
        app.resize(largura, 480);
        let n = colunas(&app);
        assert!(n >= anterior, "menos colunas com mais largura");
        anterior = n;
        // Colunas de pelo menos 160 px, em quantas couberem.
        let interna = f64::from(largura) - 24.0;
        let esperado = ((interna + 8.0) / (160.0 + 8.0)).floor().max(1.0) as usize;
        assert_eq!(n, esperado.min(6), "{largura} px");
        let rects: Vec<_> = (0..6).map(|i| app.rect(&format!("c{i}"))).collect();
        // Iguais, a menos do arredondamento para o pixel inteiro do layout.
        for rect in &rects {
            assert!((rect.width() - rects[0].width()).abs() <= 1.0, "{rects:?}");
            assert!(rect.width() >= 160.0);
        }
        // Na mesma linha, a mesma altura.
        for par in rects.chunks(n) {
            assert!(
                par.iter()
                    .all(|r| (r.height() - par[0].height()).abs() < 0.5)
            );
        }
    }
}

fn rampa_app(theme: Theme) -> TestApp {
    TestApp::new(|| {
        Form::new("Rampa").child(
            Column::new()
                .padding(12)
                .gap(4)
                .child(Text::new("Caption: legenda").caption())
                .child(Text::new("Body: texto comum"))
                .child(Text::new("Body strong: destaque").body_strong())
                .child(Text::new("Body large: introdução").body_large())
                .child(Text::new("Subtitle: seção").subtitle())
                .child(Text::new("Title: página").title())
                .child(Text::new("Title large").title_large())
                .child(Text::new("Display").display()),
        )
    })
    .size(420, 380)
    .theme(theme)
}

#[test]
fn rampa_inteira() {
    for theme in TEMAS {
        let mut app = rampa_app(theme);
        app.assert_snapshot(&format!("rampa_{}", theme_name(theme)));
        app.assert_lint();
    }
}

fn cortado_app(theme: Theme) -> TestApp {
    TestApp::new(|| {
        Form::new("Texto cortado").child(
            Column::new().padding(12).width(240).child(
                Text::new(
                    "Um texto longo que não cabe em duas linhas desta coluna estreita e termina \
                     com reticências no fim da segunda.",
                )
                .id("resumo")
                .wrap()
                .max_lines(2),
            ),
        )
    })
    .size(264, 72)
    .theme(theme)
}

#[test]
fn texto_cortado_em_duas_linhas() {
    for theme in TEMAS {
        let mut app = cortado_app(theme);
        app.assert_snapshot(&format!("texto_cortado_{}", theme_name(theme)));
        assert!(
            app.lint().iter().all(|i| i.rule != Rule::ClippedText),
            "{:?}",
            app.lint()
        );
        app.assert_lint();
        let tree = app.tree();
        let node = tree.find("resumo").unwrap();
        assert!(node.trimmed);
        assert!(node.text.as_deref().unwrap().ends_with("segunda."));
        assert_eq!(app.rect("resumo").height(), 40.0, "duas linhas de 20 px");
    }
}

#[test]
fn texto_que_cabe_nao_fica_marcado() {
    let app = TestApp::new(|| {
        Form::new("Texto").child(
            Column::new()
                .padding(12)
                .child(Text::new("Curto.").id("curto").wrap().max_lines(2)),
        )
    })
    .size(264, 72);
    assert!(!app.tree().find("curto").unwrap().trimmed);
}
