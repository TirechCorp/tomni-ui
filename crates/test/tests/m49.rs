//! Marco M49: o ícone e a atividade nas abas. O ícone da janela e o pedido de atenção, o selo da
//! barra de título, o progresso e os avisos do emulador, o anel e o ponto das abas e os links.

mod comum;

use comum::theme_name;
use std::cell::Cell;
use std::rc::Rc;

use tomni_ui::prelude::*;
use tomni_ui::terminal::{Loopback, LoopbackHandle, PaneLayout, Session, TerminalPanes};
use tomni_ui::theme::Theme;
use tomni_ui::{Look, ProgressStatus, TabProgress, TitleBadge, WindowHandle};
use tomni_ui_test::TestApp;

const TEMAS: [Theme; 2] = [Theme::Light, Theme::Dark];

/// O ícone pequeno do Tirech SSH, para a barra de título de 16 px.
const ICONE: &[u8] = include_bytes!("dados/icone-pequeno.svg");

#[test]
fn o_pedido_de_atencao_chega_a_plataforma() {
    let mut app = TestApp::new(|| Form::new("Aviso").child(Text::new("oi")));
    assert!(app.attention_requests().is_empty());
    WindowHandle::main().request_attention();
    // Dois pedidos antes de a plataforma atender valem um.
    WindowHandle::main().request_attention();
    app.frame();
    assert_eq!(app.attention_requests(), [WindowHandle::main()]);
    assert!(app.attention_requests().is_empty());
}

fn selo() -> TitleBadge {
    TitleBadge::new(Color::hex(0x063A66))
        .part("Tirech", Color::WHITE)
        .bold_part(" SSH", Color::hex(0x4CC2FF))
}

/// A barra de 48 px com o ícone por imagem, o selo, um subtítulo e um conteúdo à direita.
fn barra_com_selo() -> impl View {
    Form::new("Tirech SSH")
        .title_bar(
            TitleBar::new("Tirech SSH")
                .icon_image(ICONE)
                .title_badge(selo())
                .subtitle("Prévia")
                .right_header(Button::new("Ana").id("conta")),
        )
        .child(
            Column::new()
                .padding(Insets::all(16.0))
                .child(Text::new("Conteúdo")),
        )
}

#[test]
fn snapshots_titulo_selo() {
    for theme in TEMAS {
        let mut app = TestApp::new(barra_com_selo).size(520, 120).theme(theme);
        app.assert_lint();
        app.assert_snapshot(&format!("titulo-selo_{}", theme_name(theme)));
        // Com a janela inativa o texto do título se apagaria; o selo é a marca e não muda.
        app.set_window_active(false);
        app.assert_snapshot(&format!("titulo-selo-inativa_{}", theme_name(theme)));
    }
}

#[test]
fn o_selo_nao_muda_o_nome_da_barra() {
    let app = TestApp::new(barra_com_selo).size(520, 120);
    assert_eq!(app.text("barra-de-titulo"), "Tirech SSH");
    assert_eq!(selo().text(), "Tirech SSH");
    let barra = app.rect("barra-de-titulo");
    assert_eq!(barra.height(), 48.0);
}

#[test]
fn o_icone_por_imagem_ocupa_16_px() {
    let mut app = TestApp::new(|| {
        Form::new("Aba")
            .title_bar(TitleBar::new("Aba").icon_image(ICONE))
            .child(Text::new("oi"))
    })
    .size(300, 100);
    app.assert_lint();
}

// ---------------------------------------------------------------------------------------
// O anel e o ponto nas abas
// ---------------------------------------------------------------------------------------

struct Abas {
    app: TestApp,
    layout: Signal<PaneLayout>,
    sessoes: Vec<Session>,
    handles: Vec<LoopbackHandle>,
}

/// `n` sessões `web-01`… num painel só; a primeira é a escolhida.
fn abas(n: usize, tema: Theme) -> Abas {
    let (sessoes, handles): (Vec<Session>, Vec<LoopbackHandle>) = (1..=n)
        .map(|i| {
            let nome = format!("web-{i:02}");
            let loopback = Loopback::new();
            let handle = loopback.handle();
            (Session::new(loopback).key(nome.clone()).name(nome), handle)
        })
        .unzip();
    let guarda = Rc::new(Cell::new(None));
    let (g, s) = (Rc::clone(&guarda), sessoes.clone());
    let app = TestApp::new(move || {
        let layout = signal(PaneLayout::with(s.clone()));
        g.set(Some(layout));
        Form::new("Painéis").child(TerminalPanes::bind(layout).id("paineis").grow(1))
    })
    .theme(tema)
    .size(900, 600);
    Abas {
        app,
        layout: guarda.get().unwrap(),
        sessoes,
        handles,
    }
}

fn visual(app: &TestApp, id: &str) -> tomni_ui::TabHeaderVisual {
    match app.look(id) {
        Look::TabHeader { visual, .. } => visual,
        outro => panic!("{outro:?}"),
    }
}

fn anel(app: &TestApp, id: &str) -> Option<TabProgress> {
    visual(app, id).progress
}

const ABA1: &str = "paineis-painel-0-web-01";
const ABA2: &str = "paineis-painel-0-web-02";

#[test]
fn o_osc_9_4_acende_o_anel_da_aba_de_tras_e_o_zero_apaga() {
    let mut p = abas(2, Theme::Light);
    assert_eq!(anel(&p.app, ABA2), None);

    p.sessoes[1].feed(b"\x1b]9;4;3\x07");
    p.app.frame();
    assert_eq!(
        anel(&p.app, ABA2),
        Some(TabProgress {
            value: None,
            status: ProgressStatus::Normal
        })
    );
    assert_eq!(p.app.text(ABA2), "web-02, trabalhando");
    assert_eq!(anel(&p.app, ABA1), None, "a outra aba não muda");
    assert_eq!(p.app.text(ABA1), "web-01");

    p.sessoes[1].feed(b"\x1b]9;4;1;45\x07");
    p.app.frame();
    assert_eq!(
        anel(&p.app, ABA2),
        Some(TabProgress {
            value: Some(0.45),
            status: ProgressStatus::Normal
        })
    );
    assert_eq!(p.app.text(ABA2), "web-02, 45%");

    p.sessoes[1].feed(b"\x1b]9;4;4;60\x07");
    p.app.frame();
    assert_eq!(
        anel(&p.app, ABA2),
        Some(TabProgress {
            value: Some(0.6),
            status: ProgressStatus::Paused
        })
    );
    assert_eq!(p.app.text(ABA2), "web-02, pausado, 60%");

    p.sessoes[1].feed(b"\x1b]9;4;2\x07");
    p.app.frame();
    assert_eq!(p.app.text(ABA2), "web-02, erro, 60%");

    p.sessoes[1].feed(b"\x1b]9;4;0\x07");
    p.app.frame();
    assert_eq!(anel(&p.app, ABA2), None);
    assert_eq!(p.app.text(ABA2), "web-02");
}

#[test]
fn a_sessao_fechada_apaga_o_anel() {
    let mut p = abas(2, Theme::Light);
    p.sessoes[1].feed(b"\x1b]9;4;1;30\x07");
    p.app.frame();
    assert!(anel(&p.app, ABA2).is_some());
    p.handles[1].close(None);
    p.app.frame();
    assert_eq!(anel(&p.app, ABA2), None);
    // A aba caída mostra o aviso, e o anel não volta por cima.
    assert_eq!(visual(&p.app, ABA2).icon, Some(Symbol::Warning));
}

#[test]
fn o_bel_na_aba_de_tras_acende_o_ponto_e_clicar_apaga() {
    let mut p = abas(2, Theme::Light);
    assert!(!visual(&p.app, ABA2).attention);

    p.sessoes[1].feed(b"\x07");
    p.app.frame();
    assert!(visual(&p.app, ABA2).attention);
    assert_eq!(p.app.text(ABA2), "web-02, pede atenção");
    assert_eq!(p.app.attention_requests(), [WindowHandle::main()]);

    p.app.click(ABA2);
    assert!(!visual(&p.app, ABA2).attention);
    assert_eq!(p.app.text(ABA2), "web-02");
}

#[test]
fn o_aviso_na_aba_escolhida_ja_nasce_visto() {
    let mut p = abas(2, Theme::Light);
    p.sessoes[0].feed("\x1b]9;Terminou\x07".as_bytes());
    p.app.frame();
    assert!(!visual(&p.app, ABA1).attention);
    // O pedido de atenção da janela vale também aqui: a plataforma só o atende sem o foco.
    assert_eq!(p.app.attention_requests(), [WindowHandle::main()]);

    // Ao trocar de aba, a de antes passa a ser "de trás" e o próximo aviso acende.
    p.app.click(ABA2);
    p.sessoes[0].feed(b"\x1b]777;notify;Build;Pronto\x07");
    p.app.frame();
    assert!(visual(&p.app, ABA1).attention);
}

#[test]
fn o_ponto_aparece_por_cima_do_anel_e_fechar_a_sessao_limpa() {
    let mut p = abas(3, Theme::Light);
    let terceira = "paineis-painel-0-web-03";
    p.sessoes[2].feed(b"\x1b]9;4;3\x07\x07");
    p.app.frame();
    let v = visual(&p.app, terceira);
    assert!(v.progress.is_some() && v.attention);
    assert_eq!(p.app.text(terceira), "web-03, trabalhando, pede atenção");

    p.layout.update(|l| {
        l.remove(p.sessoes[2]);
    });
    p.app.frame();
    assert!(p.app.tree().find(terceira).is_none());
}

#[test]
fn o_anel_indeterminado_gira_e_o_com_valor_nao() {
    let mut p = abas(2, Theme::Light);
    p.sessoes[1].feed(b"\x1b]9;4;3\x07");
    p.app.frame();
    let antes = visual(&p.app, ABA2).phase;
    p.app.wait(std::time::Duration::from_millis(500));
    assert!(visual(&p.app, ABA2).phase > antes, "o anel gira");

    p.sessoes[1].feed(b"\x1b]9;4;1;50\x07");
    p.app.frame();
    assert_eq!(visual(&p.app, ABA2).phase, 0.0);
}

// ---------------------------------------------------------------------------------------
// Snapshots das abas
// ---------------------------------------------------------------------------------------

/// Um `TabView` compacto (a densidade das abas do terminal) com as abas `abas`.
fn faixa(
    abas: &'static [(&'static str, &'static str)],
    progresso: impl Fn(String) -> Option<TabProgress> + 'static,
    atencao: impl Fn(String) -> bool + 'static,
) -> impl View {
    let escolhida = signal(abas[0].0.to_owned());
    Form::new("Abas").child(
        TabView::bind(escolhida)
            .id("abas")
            .density(Density::Compact)
            .items(
                move || abas.iter().map(|(chave, _)| (*chave).to_owned()).collect(),
                |chave: &String| chave.clone(),
                move |chave: &String| {
                    let titulo = abas
                        .iter()
                        .find(|(c, _)| c == chave)
                        .map_or("", |(_, titulo)| *titulo);
                    titulo.to_owned()
                },
                Text::new,
            )
            .tab_progress(progresso)
            .tab_attention(atencao)
            .height(80),
    )
}

#[test]
fn snapshots_aba_progresso() {
    const ABAS: &[(&str, &str)] = &[("a", "Compilar"), ("b", "Testes"), ("c", "Deploy")];
    for theme in TEMAS {
        let mut app = TestApp::new(|| {
            faixa(
                ABAS,
                |chave| match chave.as_str() {
                    "a" => Some(TabProgress {
                        value: None,
                        status: ProgressStatus::Normal,
                    }),
                    "b" => Some(TabProgress {
                        value: Some(0.45),
                        status: ProgressStatus::Normal,
                    }),
                    _ => Some(TabProgress {
                        value: Some(0.6),
                        status: ProgressStatus::Paused,
                    }),
                },
                |_| false,
            )
        })
        .size(520, 120)
        .theme(theme);
        app.assert_lint();
        app.assert_snapshot(&format!("aba-progresso_{}", theme_name(theme)));
    }
}

#[test]
fn snapshots_aba_atencao() {
    const ABAS: &[(&str, &str)] = &[
        ("a", "Sem aviso"),
        ("b", "Com ponto"),
        ("c", "Ponto e anel"),
    ];
    for theme in TEMAS {
        let mut app = TestApp::new(|| {
            faixa(
                ABAS,
                |chave| {
                    (chave == "c").then_some(TabProgress {
                        value: Some(0.7),
                        status: ProgressStatus::Error,
                    })
                },
                |chave| chave != "a",
            )
        })
        .size(520, 120)
        .theme(theme);
        app.assert_lint();
        app.assert_snapshot(&format!("aba-atencao_{}", theme_name(theme)));
    }
}

// Não há snapshot `aba-titulo-atividade`: as fontes embutidas não têm `✳ ◐ ◑ ◒ ◓` nem o braile
// (as abas saem com um vazio no lugar, e o lint `missing-glyph` recusa a tela). O que falta
// está registrado na "Entrega" do M49; o teste abaixo confere só o caminho do título até a aba.

#[test]
fn o_titulo_com_caractere_de_atividade_chega_a_aba() {
    let mut p = abas(2, Theme::Light);
    p.sessoes[1].feed("\x1b]2;◐ Tarefa\x07".as_bytes());
    p.app.frame();
    assert_eq!(p.app.text(ABA2), "◐ Tarefa");
    p.sessoes[1].feed("\x1b]2;✳ Tarefa\x07".as_bytes());
    p.app.frame();
    assert_eq!(p.app.text(ABA2), "✳ Tarefa");
}
