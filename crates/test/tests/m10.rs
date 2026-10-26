//! Marco M10: `RadioButtons`, `Slider`, `ProgressBar`/`ProgressRing`, o botão de link, o
//! `Expander`, o `TabView` e o `AutoSuggestBox`. Os snapshots nos dois temas e os testes do
//! aceite (critérios 1 e 2 da spec).

mod comum;

use std::time::Duration;

use comum::{State, theme_name};
use tomni_ui::kurbo::Point;
use tomni_ui::prelude::*;
use tomni_ui::theme::Theme;
use tomni_ui::{ANIMATION_FRAME, Key, Modifiers};
use tomni_ui_test::TestApp;

const TEMAS: [Theme; 2] = [Theme::Light, Theme::Dark];

// --- Snapshots -------------------------------------------------------------------------------

#[test]
fn snapshots_do_radio_buttons() {
    for tema in TEMAS {
        // Os cinco estados na opção marcada; hover e pressionado também na desmarcada.
        let casos = State::ALL
            .iter()
            .map(|&e| (e, "cor-claro", "marcada"))
            .chain([
                (State::Hover, "cor-escuro", "desmarcada"),
                (State::Pressed, "cor-escuro", "desmarcada"),
            ]);
        for (estado, alvo, marca) in casos {
            let mut app = TestApp::new(move || {
                let cor = signal("Claro".to_owned());
                Form::new("Tema").child(
                    Column::new().padding(12).child(
                        RadioButtons::bind(cor, ["Claro", "Escuro"])
                            .id("cor")
                            .header("Tema")
                            .disabled(estado == State::Disabled),
                    ),
                )
            })
            .size(200, 140)
            .theme(tema);
            estado.apply(&mut app, alvo);
            assert_eq!(app.text("cor"), "Claro");
            app.assert_snapshot(&format!(
                "radio_buttons_{}_{marca}_{}",
                estado.name(),
                theme_name(tema)
            ));
        }
    }
}

#[test]
fn snapshots_do_slider() {
    for tema in TEMAS {
        for estado in State::ALL {
            let mut app = TestApp::new(move || {
                let volume = signal(50.0);
                Form::new("Som").child(
                    Column::new().padding(12).child(
                        Slider::bind(volume, 0..=100)
                            .id("volume")
                            .header("Volume")
                            .disabled(estado == State::Disabled),
                    ),
                )
            })
            .size(240, 84)
            .theme(tema);
            estado.apply(&mut app, "volume");
            assert_eq!(app.value("volume"), 50.0);
            app.assert_snapshot(&format!("slider_{}_{}", estado.name(), theme_name(tema)));
        }
    }
}

#[test]
fn snapshots_do_progresso() {
    for tema in TEMAS {
        let app = TestApp::new(|| {
            Form::new("Progresso").child(
                Column::new()
                    .padding(12)
                    .gap(16)
                    .child(ProgressBar::new().value(0.25))
                    .child(ProgressBar::new())
                    .child(
                        Row::new()
                            .gap(16)
                            .child(ProgressRing::new().value(0.6))
                            .child(ProgressRing::new()),
                    ),
            )
        })
        .size(200, 100)
        .theme(tema);
        app.assert_snapshot(&format!("progress_{}", theme_name(tema)));
    }
}

#[test]
fn snapshots_do_progresso_pausado_e_com_erro() {
    for tema in TEMAS {
        let app = TestApp::new(|| {
            Form::new("Progresso").child(
                Column::new()
                    .padding(12)
                    .gap(16)
                    .child(ProgressBar::new().value(0.6).paused(true))
                    .child(ProgressBar::new().value(0.3).error(true))
                    .child(ProgressBar::new().paused(true))
                    .child(ProgressBar::new().error(true)),
            )
        })
        .size(200, 100)
        .theme(tema);
        app.assert_snapshot(&format!("progress_status_{}", theme_name(tema)));
    }
}

#[test]
fn snapshots_do_expander() {
    for tema in TEMAS {
        for (nome, aberto, estado) in [
            ("fechado", false, State::Rest),
            ("aberto", true, State::Rest),
            ("hover", false, State::Hover),
            ("pressed", false, State::Pressed),
            ("focused", true, State::Focused),
        ] {
            let mut app = TestApp::new(move || {
                let aberto = signal(aberto);
                let vibrar = signal(true);
                Form::new("Sons").child(
                    Column::new().padding(12).child(
                        Expander::new("Sons", aberto).id("sons").content(
                            CheckBox::bind(vibrar, "Vibrar ao tocar")
                                .id("vibrar")
                                .height(44),
                        ),
                    ),
                )
            })
            .size(300, 160)
            .theme(tema);
            estado.apply(&mut app, "sons");
            assert_eq!(app.is_open("sons"), aberto);
            app.assert_snapshot(&format!("expander_{nome}_{}", theme_name(tema)));
        }
    }
}

/// Com `.description`, o cabeçalho tem 64 px e a segunda linha na cor secundária.
#[test]
fn snapshots_do_expander_com_descricao() {
    for tema in TEMAS {
        for (nome, aberto) in [("fechado", false), ("aberto", true)] {
            let app = TestApp::new(move || {
                let aberto = signal(aberto);
                let vibrar = signal(true);
                Form::new("Sons").child(
                    Column::new().padding(12).child(
                        Expander::new("Sons", aberto)
                            .id("sons")
                            .description("Toque, volume e vibração")
                            .content(
                                CheckBox::bind(vibrar, "Vibrar ao tocar")
                                    .id("vibrar")
                                    .height(44),
                            ),
                    ),
                )
            })
            .size(300, 180)
            .theme(tema);
            assert_eq!(app.is_open("sons"), aberto);
            assert_eq!(app.rect("sons").height() > 64.0, aberto);
            app.assert_snapshot(&format!("expander_descricao_{nome}_{}", theme_name(tema)));
        }
    }
}

#[test]
fn snapshots_do_tab_view() {
    for tema in TEMAS {
        for estado in [State::Rest, State::Hover, State::Pressed, State::Focused] {
            let mut app = TestApp::new(|| {
                let aba = signal("geral".to_owned());
                Form::new("Configurações").child(
                    TabView::bind(aba)
                        .id("config")
                        .grow(1)
                        .tab("geral", "Geral", Text::new("Opções do dia a dia."))
                        .tab("avancado", "Avançado", Text::new("Opções raras."))
                        .tab("sobre", "Sobre", Text::new("Versão 1.0.")),
                )
            })
            .size(390, 140)
            .theme(tema);
            // Hover e pressionado numa aba que não é a escolhida; o foco chega na escolhida.
            estado.apply(&mut app, "config-avancado");
            assert_eq!(app.text("config"), "geral");
            app.assert_snapshot(&format!("tab_view_{}_{}", estado.name(), theme_name(tema)));
        }
    }
}

#[test]
fn snapshot_do_auto_suggest_box_aberto() {
    for tema in TEMAS {
        let mut app = TestApp::new(|| {
            let cidade = signal(String::new());
            Form::new("Cidade").child(
                Column::new().padding(12).child(
                    AutoSuggestBox::bind(cidade, |t: &str| {
                        ["Belo Horizonte", "Belém", "Brasília"]
                            .into_iter()
                            .filter(|c| !t.is_empty() && c.starts_with(t))
                            .map(str::to_owned)
                            .collect()
                    })
                    .id("cidade")
                    .placeholder("Cidade")
                    .height(44),
                ),
            )
        })
        .size(300, 230)
        .theme(tema);
        app.type_into("cidade", "Be").key(Key::Down);
        assert!(app.is_open("cidade"));
        app.hover("cidade-belém");
        app.assert_snapshot(&format!("auto_suggest_box_aberto_{}", theme_name(tema)));
    }
}

// --- Comportamento ---------------------------------------------------------------------------

#[test]
fn setas_movem_a_escolha_e_o_tab_atravessa_o_grupo() {
    let mut app = TestApp::new(|| {
        let tema = signal("Claro".to_owned());
        Form::new("Aparência").child(
            Column::new()
                .child(Button::new("Antes").id("antes").height(44))
                .child(
                    RadioButtons::bind(tema, ["Claro", "Escuro", "Do sistema"])
                        .header("Tema")
                        .id("tema"),
                )
                .child(Button::new("Depois").id("depois").height(44)),
        )
    })
    .size(390, 400);
    assert_eq!(app.label("tema"), "Tema", "o rótulo do grupo é o cabeçalho");
    assert_eq!(app.label("tema-escuro"), "Escuro");
    app.tab().tab();
    assert_eq!(app.focused().as_deref(), Some("tema-claro"));
    app.key(Key::Down).key(Key::Down);
    assert_eq!(app.text("tema"), "Do sistema");
    assert_eq!(app.focused().as_deref(), Some("tema-do-sistema"));
    app.key(Key::Up);
    assert_eq!(app.text("tema"), "Escuro");
    app.tab();
    assert_eq!(app.focused().as_deref(), Some("depois"), "uma parada só");
    app.shift_tab();
    assert_eq!(
        app.focused().as_deref(),
        Some("tema-escuro"),
        "volta na marcada"
    );
    app.shift_tab();
    assert_eq!(app.focused().as_deref(), Some("antes"));
}

#[test]
fn slider_responde_ao_clique_ao_arrasto_e_as_teclas_sem_passar_das_pontas() {
    let mut app = TestApp::new(|| {
        let volume = signal(50.0);
        Form::new("Som").child(
            Column::new()
                .padding(24)
                .child(Slider::bind(volume, 0..=100).id("volume").step(10.0)),
        )
    })
    .size(390, 200);
    app.set_value("volume", 23.0);
    assert_eq!(app.value("volume"), 20.0);

    let rect = app.rect("volume");
    let y = rect.center().y;
    app.drag(
        Point::new(rect.center().x, y),
        Point::new(rect.x1 + 200.0, y),
    );
    assert_eq!(app.value("volume"), 100.0, "o arrasto para na ponta");
    app.drag(Point::new(rect.x1 - 4.0, y), Point::new(rect.x0 - 200.0, y));
    assert_eq!(app.value("volume"), 0.0);

    app.key(Key::Left);
    assert_eq!(app.value("volume"), 0.0, "← na ponta não passa");
    app.key(Key::Right).key(Key::Right);
    assert_eq!(app.value("volume"), 20.0);
    app.key(Key::PageUp);
    assert_eq!(app.value("volume"), 30.0);
    app.key(Key::End).key(Key::Right);
    assert_eq!(app.value("volume"), 100.0);
    app.key(Key::Home);
    assert_eq!(app.value("volume"), 0.0);
}

#[test]
fn trocar_de_aba_preserva_o_texto_das_outras() {
    let mut app = TestApp::new(|| {
        let aba = signal("geral".to_owned());
        let nome = signal(String::new());
        let chave = signal(String::new());
        Form::new("Configurações").child(
            TabView::bind(aba)
                .id("config")
                .grow(1)
                .tab("geral", "Geral", Edit::bind(nome).id("nome").height(44))
                .tab(
                    "avancado",
                    "Avançado",
                    Edit::bind(chave).id("chave").height(44),
                ),
        )
    })
    .size(390, 400);
    app.type_into("nome", "Ana");
    app.select("config", "Avançado");
    app.type_into("chave", "abc-123");
    app.click("config-geral");
    assert_eq!(app.text("nome"), "Ana");
    app.click("nome").key_with(Key::Tab, Modifiers::CTRL);
    assert_eq!(app.text("config"), "avancado");
    assert_eq!(app.text("chave"), "abc-123");
    app.assert_lint();
}

fn cidades() -> TestApp {
    TestApp::new(|| {
        let cidade = signal(String::new());
        let escolhida = signal(String::new());
        Form::new("Cidade").child(
            Column::new()
                .padding(24)
                .gap(12)
                .child(
                    AutoSuggestBox::bind(cidade, |t: &str| {
                        ["Belo Horizonte", "Belém", "Brasília"]
                            .into_iter()
                            .filter(|c| !t.is_empty() && c.starts_with(t))
                            .map(str::to_owned)
                            .collect()
                    })
                    .id("cidade")
                    .height(44)
                    .on_submit(move |c| escolhida.set(c)),
                )
                .child(Text::new(move || escolhida.get()).id("escolhida")),
        )
    })
    .size(390, 400)
}

#[test]
fn autocompletar_mantem_o_foco_no_campo_e_confirma_por_teclado_e_clique() {
    let mut app = cidades();
    app.type_into("cidade", "B");
    assert!(app.is_open("cidade"));
    assert_eq!(app.focused().as_deref(), Some("cidade"));
    app.assert_lint();
    app.key(Key::Down).key(Key::Down);
    assert_eq!(
        app.focused().as_deref(),
        Some("cidade"),
        "↓ não tira o foco"
    );
    assert_eq!(app.text("cidade"), "Belém");
    app.type_text(" do Pará");
    assert_eq!(
        app.text("cidade"),
        "Belém do Pará",
        "o cursor foi para o fim"
    );
    assert!(!app.is_open("cidade"));

    let mut app = cidades();
    app.type_into("cidade", "Br").key(Key::Down).key(Key::Enter);
    assert_eq!(app.text("escolhida"), "Brasília");
    assert!(!app.is_open("cidade"));

    let mut app = cidades();
    app.type_into("cidade", "Be").click("cidade-belo-horizonte");
    assert_eq!(app.text("cidade"), "Belo Horizonte");
    assert_eq!(app.text("escolhida"), "Belo Horizonte");
    assert_eq!(app.focused().as_deref(), Some("cidade"));
    assert!(!app.is_open("cidade"));

    let mut app = cidades();
    app.type_into("cidade", "Be").key(Key::Escape);
    assert!(!app.is_open("cidade"));
    assert_eq!(app.text("cidade"), "Be");
}

#[test]
fn indeterminado_pede_frames_enquanto_aparece() {
    let mut app = TestApp::new(|| {
        let sincronizando = signal(true);
        Form::new("Sincronização").child(
            Column::new()
                .padding(24)
                .gap(12)
                .child(ProgressBar::new().id("barra").visible(sincronizando))
                .child(
                    Button::new("Parar")
                        .id("parar")
                        .height(44)
                        .on_click(move || sincronizando.set(false)),
                ),
        )
    });
    assert_eq!(app.ui().deadline(), Some(ANIMATION_FRAME));
    app.wait(Duration::from_millis(500));
    assert_eq!(app.repaints_since_last(), ["barra"]);
    app.click("parar");
    assert_eq!(app.ui().deadline(), None);
    app.repaints_since_last();
    app.wait(Duration::from_millis(500));
    assert!(app.repaints_since_last().is_empty());
}

#[test]
fn lint_cobre_os_widgets_novos() {
    let mut app = TestApp::new(|| {
        let tema = signal("Claro".to_owned());
        let volume = signal(50.0);
        let aberto = signal(true);
        let aba = signal("a".to_owned());
        let cidade = signal(String::new());
        Form::new("Sem ids").child(
            Column::new()
                .child(RadioButtons::bind(tema, ["Claro", "Escuro"]))
                .child(Slider::bind(volume, 0..=100).height(20))
                .child(Button::new("Ajuda").hyperlink())
                .child(Expander::new("Mais", aberto).content(Text::new("Oi")))
                .child(
                    TabView::bind(aba)
                        .height(160)
                        .tab("a", "A", Button::new("Dentro").height(44))
                        .tab("b", "B", Text::new("Dois")),
                )
                .child(AutoSuggestBox::bind(cidade, |_: &str| Vec::new())),
        )
    })
    .size(390, 844);
    let achados: Vec<(&str, String)> = app
        .lint()
        .into_iter()
        .map(|i| (i.rule.name(), i.widget))
        .collect();
    let esperado = [
        ("touch-target", "slider"),
        ("touch-target", "button"),
        ("touch-target", "auto_suggest_box"),
        // As opções e as abas não aparecem uma a uma: o id falta no grupo.
        ("missing-id", "radio_buttons"),
        ("missing-id", "slider"),
        ("missing-id", "button"),
        ("missing-id", "expander"),
        ("missing-id", "tab_view"),
        // O botão dentro da aba é do app, não do grupo.
        ("missing-id", "button"),
        ("missing-id", "auto_suggest_box"),
    ];
    assert_eq!(
        achados,
        esperado.map(|(r, w)| (r, w.to_owned())),
        "{:#?}",
        app.lint()
    );
}
