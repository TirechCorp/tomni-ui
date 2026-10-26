//! Marco M18: o `WebView`. Com o motor falso dos testes: o quadro carregando, pronto e com
//! erro nos 2 temas, a captura no lugar da página sob um `ContentDialog`, onde a janela do
//! sistema fica (e some numa aba escondida ou com `.visible(false)`), o histórico e o `WebViewController`, as mensagens entre a página e o app, a
//! política de navegação, o foco (F6 para sair da página) e a regra `webview-in-scroll`.

mod comum;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use comum::theme_name;
use tomni_ui::kurbo::Rect;
use tomni_ui::lint::Rule;
use tomni_ui::prelude::*;
use tomni_ui::theme::Theme;
use tomni_ui::webview::{Loaded, WEBVIEW_MIN_SIZE};
use tomni_ui_test::TestApp;

const TEMAS: [Theme; 2] = [Theme::Light, Theme::Dark];

const INICIO: &str = "<title>Início</title><p>Olá</p>";
const SOBRE: &str = "<title>Sobre</title><p>Sobre</p>";

/// A barra com voltar e avançar, a página e um botão depois dela (para o foco).
fn tela(web: WebViewController, aberto: Signal<bool>) -> impl View {
    Form::new("Ajuda").child(
        Column::new()
            .padding(16)
            .gap(8)
            .child(
                Row::new()
                    .gap(4)
                    .align(Align::Center)
                    .child(
                        Button::icon_only(Symbol::ArrowLeft)
                            .id("voltar")
                            .label("Voltar")
                            .disabled(move || !web.can_go_back())
                            .on_click(move || web.go_back()),
                    )
                    .child(
                        Button::icon_only(Symbol::ArrowRight)
                            .id("avancar")
                            .label("Avançar")
                            .disabled(move || !web.can_go_forward())
                            .on_click(move || web.go_forward()),
                    )
                    .child(Text::new(move || web.title()).id("titulo").max_lines(1)),
            )
            .child(
                WebView::html(INICIO)
                    .asset("sobre.html", SOBRE)
                    .id("pagina")
                    .grow(1)
                    .controller(web),
            )
            .child(
                Button::new("Abrir o diálogo")
                    .id("depois")
                    .height(44)
                    .on_click(move || aberto.set(true)),
            )
            .child(
                ContentDialog::new("Sair da ajuda?", aberto)
                    .id("dialogo")
                    .content(Text::new("A página fica onde está.").wrap())
                    .close("Cancelar"),
            ),
    )
}

/// O app de [`tela`] em 480 × 400, com a página inicial já carregada.
fn pronto(theme: Theme) -> (TestApp, WebViewController) {
    let web = Rc::new(RefCell::new(None));
    let guardado = Rc::clone(&web);
    let mut app = TestApp::new(move || {
        let controller = WebViewController::new();
        *guardado.borrow_mut() = Some(controller);
        tela(controller, signal(false))
    })
    .size(480, 400)
    .theme(theme);
    let web = web.borrow().expect("a tela criou o controller");
    assert_eq!(app.tree().find("pagina").unwrap().state, Some("ready"));
    app.repaints_since_last();
    (app, web)
}

/// A área de conteúdo do widget: a janela do sistema ocupa ela inteira.
fn area(app: &TestApp) -> Rect {
    app.rect("pagina")
}

#[test]
fn a_carga_termina_no_passo_seguinte() {
    let mut app =
        TestApp::new(|| Form::new("Página").child(WebView::html(INICIO).id("pagina").grow(1)));
    app.frame();
    let no = app.tree().find("pagina").cloned().unwrap();
    assert_eq!(no.kind, "webview");
    assert_eq!(no.state, Some("loading"));
    assert_eq!(no.url.as_deref(), Some("fluent://app/index.html"));
    assert!(app.visible("pagina-carregando"));
    assert_eq!(
        app.webview_shown("pagina"),
        None,
        "a página só aparece pronta"
    );
    app.frame();
    let no = app.tree().find("pagina").cloned().unwrap();
    assert_eq!(no.state, Some("ready"));
    assert_eq!(no.title.as_deref(), Some("Início"));
    assert_eq!(no.can_go_back, Some(false));
    assert!(!app.visible("pagina-carregando"));
    assert!(app.webview_shown("pagina").is_some());
}

#[test]
fn pronto_com_erro_e_carregando_de_novo() {
    for theme in TEMAS {
        let (mut app, web) = pronto(theme);
        assert_eq!(app.text("titulo"), "Início");
        assert!(!app.visible("pagina-carregando"));
        assert!(!app.visible("pagina-erro"));
        assert_eq!(app.webview_shown("pagina"), Some(area(&app)));
        app.assert_lint();
        app.assert_snapshot(&format!("m18_pronto_{}", theme_name(theme)));

        app.webview_fail("pagina", "Não foi possível resolver o nome do servidor");
        let no = app.tree().find("pagina").cloned().unwrap();
        assert_eq!(no.state, Some("failed"));
        assert_eq!(
            no.error.as_deref(),
            Some("Não foi possível resolver o nome do servidor")
        );
        assert_eq!(web.error().as_deref(), no.error.as_deref());
        assert!(app.visible("pagina-erro"));
        assert!(!app.visible("pagina-carregando"));
        assert_eq!(app.webview_shown("pagina"), None);
        app.assert_lint();
        app.assert_snapshot(&format!("m18_erro_{}", theme_name(theme)));

        // Uma rede lenta: o "Tentar de novo" deixa o anel até a página responder.
        app.webview_hold("pagina", true);
        app.click("pagina-tentar");
        assert_eq!(app.tree().find("pagina").unwrap().state, Some("loading"));
        assert!(app.visible("pagina-carregando"));
        assert!(!app.visible("pagina-erro"));
        assert_eq!(app.webview_shown("pagina"), None);
        app.assert_lint();
        app.assert_snapshot(&format!("m18_carregando_{}", theme_name(theme)));

        app.webview_hold("pagina", false);
        app.frame();
        assert_eq!(app.tree().find("pagina").unwrap().state, Some("ready"));
        assert!(!app.visible("pagina-carregando"));
        assert_eq!(
            app.webview_navigations("pagina"),
            ["fluent://app/index.html", "fluent://app/index.html"],
            "depois de uma falha, recarregar carrega o endereço de novo"
        );
    }
}

#[test]
fn sob_o_dialogo_a_pagina_vira_a_captura() {
    for theme in TEMAS {
        let (mut app, _) = pronto(theme);
        let antes = app.webview_shown("pagina");
        assert!(antes.is_some());
        app.click("depois");
        // A captura chega no passo seguinte; até lá a página continua no lugar.
        app.wait(Duration::from_millis(400));
        assert!(app.is_open("dialogo"));
        assert_eq!(
            app.webview_shown("pagina"),
            None,
            "a janela do sistema ficaria por cima do diálogo"
        );
        app.assert_snapshot(&format!("m18_dialogo_{}", theme_name(theme)));

        app.key(tomni_ui::Key::Escape);
        app.wait(Duration::from_millis(400));
        assert!(!app.is_open("dialogo"));
        assert_eq!(app.webview_shown("pagina"), antes);
    }
}

#[test]
fn a_pagina_acompanha_o_widget() {
    let (mut app, _) = pronto(Theme::Light);
    assert_eq!(app.webview_shown("pagina"), Some(area(&app)));
    app.resize(640, 480);
    let novo = area(&app);
    assert!(
        novo.width() > 448.0,
        "o WebView cresce com a janela: {novo:?}"
    );
    assert_eq!(app.webview_shown("pagina"), Some(novo));
    assert!(!app.webview_dark("pagina"));
    let (app, _) = pronto(Theme::Dark);
    assert!(app.webview_dark("pagina"), "a página segue o tema do app");
}

#[test]
fn tamanho_minimo() {
    let mut app = TestApp::new(|| {
        Form::new("Ajuda").child(
            Row::new()
                .align(Align::Start)
                .child(WebView::new("https://exemplo.com").id("pagina")),
        )
    })
    .size(480, 400);
    app.frame();
    let rect = app.rect("pagina");
    assert_eq!(
        (rect.width(), rect.height()),
        (WEBVIEW_MIN_SIZE.width, WEBVIEW_MIN_SIZE.height)
    );
}

#[test]
fn historico_pelo_controller_e_pelos_botoes() {
    let (mut app, web) = pronto(Theme::Light);
    assert!(!app.enabled("voltar"));
    app.webview_hold("pagina", true);
    assert!(app.webview_follow("pagina", "fluent://app/sobre.html"));
    assert_eq!(app.tree().find("pagina").unwrap().state, Some("loading"));
    // A página antiga fica à mostra enquanto a nova carrega, como num navegador.
    assert!(!app.visible("pagina-carregando"));
    assert!(app.webview_shown("pagina").is_some());
    app.webview_hold("pagina", false);
    app.frame();
    assert_eq!(app.text("titulo"), "Sobre");
    assert_eq!(web.url(), "fluent://app/sobre.html");
    assert!(app.enabled("voltar"));
    assert!(!app.enabled("avancar"));

    app.click("voltar");
    app.frame();
    assert_eq!(app.webview_url("pagina"), "fluent://app/index.html");
    assert_eq!(app.text("titulo"), "Início");
    let no = app.tree().find("pagina").cloned().unwrap();
    assert_eq!(
        (no.can_go_back, no.can_go_forward),
        (Some(false), Some(true))
    );

    app.click("avancar");
    app.frame();
    assert_eq!(web.url(), "fluent://app/sobre.html");

    web.navigate("https://exemplo.com/ajuda");
    app.frame();
    app.frame();
    assert_eq!(web.url(), "https://exemplo.com/ajuda");
    assert_eq!(web.title(), "exemplo.com");
    // A origem pedida pelo app passa a valer para os links da página.
    assert!(app.webview_follow("pagina", "https://exemplo.com/outra"));
    assert!(!app.webview_follow("pagina", "https://outro.com/"));
    assert_eq!(app.webview_opened("pagina"), ["https://outro.com/"]);
}

#[test]
fn links_de_fora_abrem_no_navegador() {
    let (mut app, _) = pronto(Theme::Light);
    assert!(!app.webview_follow("pagina", "https://www.rust-lang.org/"));
    assert!(!app.webview_follow("pagina", "mailto:ajuda@exemplo.com"));
    assert!(!app.webview_follow("pagina", "javascript:alert(1)"));
    assert!(!app.webview_follow("pagina", "file:///etc/passwd"));
    assert_eq!(
        app.webview_opened("pagina"),
        ["https://www.rust-lang.org/", "mailto:ajuda@exemplo.com"]
    );
    assert_eq!(app.webview_url("pagina"), "fluent://app/index.html");
}

#[test]
fn mensagens_entre_a_pagina_e_o_app() {
    let recebidas = Rc::new(RefCell::new(Vec::new()));
    let cargas = Rc::new(RefCell::new(Vec::new()));
    let titulos = Rc::new(RefCell::new(Vec::new()));
    let (r, c, t) = (
        Rc::clone(&recebidas),
        Rc::clone(&cargas),
        Rc::clone(&titulos),
    );
    let web = Rc::new(RefCell::new(None));
    let guardado = Rc::clone(&web);
    let mut app = TestApp::new(move || {
        let controller = WebViewController::new();
        *guardado.borrow_mut() = Some(controller);
        Form::new("Página").child(
            WebView::html(INICIO)
                .id("pagina")
                .grow(1)
                .controller(controller)
                .on_message(move |texto| r.borrow_mut().push(texto))
                .on_load(move |carga: &Loaded| c.borrow_mut().push(carga.clone()))
                .on_title(move |titulo| t.borrow_mut().push(titulo)),
        )
    })
    .size(480, 400);
    app.frame();
    let web = web.borrow().unwrap();
    assert_eq!(
        *cargas.borrow(),
        [Loaded {
            url: "fluent://app/index.html".into(),
            error: None
        }]
    );
    assert_eq!(*titulos.borrow(), ["Início"]);

    app.webview_message("pagina", "olá, app");
    assert_eq!(*recebidas.borrow(), ["olá, app"]);

    web.post_message("olá, página");
    web.eval("document.body.style.zoom = 2");
    app.frame();
    assert_eq!(app.webview_posted("pagina"), ["olá, página"]);
    assert_eq!(
        app.webview_scripts("pagina"),
        ["document.body.style.zoom = 2"]
    );

    app.webview_fail("pagina", "sem rede");
    web.reload();
    app.frame();
    app.frame();
    let cargas = cargas.borrow();
    assert_eq!(cargas.len(), 3, "{cargas:?}");
    assert_eq!(cargas[1].error.as_deref(), Some("sem rede"));
    assert_eq!(cargas[2].error, None);
}

#[test]
fn endereco_dinamico() {
    let endereco = signal(String::from("https://exemplo.com/a"));
    let mut app = TestApp::new(move || {
        Form::new("Página").child(WebView::new(endereco).id("pagina").grow(1))
    })
    .size(480, 400);
    app.frame();
    assert_eq!(app.webview_url("pagina"), "https://exemplo.com/a");
    endereco.set("https://exemplo.com/b".into());
    app.frame();
    app.frame();
    assert_eq!(app.webview_url("pagina"), "https://exemplo.com/b");
    assert_eq!(
        app.tree().find("pagina").unwrap().url.as_deref(),
        Some("https://exemplo.com/b")
    );
}

#[test]
fn foco_entra_e_o_f6_sai() {
    let (mut app, _) = pronto(Theme::Light);
    app.click("voltar"); // desabilitado: não pega o foco
    app.tab();
    while app.focused().as_deref() != Some("pagina") {
        app.tab();
    }
    assert!(
        app.webview_focused("pagina"),
        "o foco do app vai para a janela do sistema"
    );
    app.webview_f6("pagina", false);
    assert_eq!(app.focused().as_deref(), Some("depois"));
    app.webview_page_focus("pagina");
    assert_eq!(
        app.focused().as_deref(),
        Some("pagina"),
        "um clique na página"
    );
    app.webview_f6("pagina", true);
    assert_ne!(app.focused().as_deref(), Some("pagina"));
}

#[test]
fn webview_num_scrollviewer_e_erro_do_lint() {
    let mut app = TestApp::new(|| {
        Form::new("Página").child(
            ScrollViewer::new()
                .id("rolagem")
                .child(Column::new().child(WebView::new("https://exemplo.com").id("pagina"))),
        )
    })
    .size(480, 400);
    app.frame();
    let problemas: Vec<_> = app
        .lint()
        .into_iter()
        .filter(|p| p.rule == Rule::WebViewInScroll)
        .collect();
    assert_eq!(problemas.len(), 1, "{problemas:?}");
    assert_eq!(problemas[0].widget, "pagina");
    assert!(problemas[0].message.contains("rolagem"));
}

#[test]
fn a_pagina_some_com_o_widget_escondido_e_volta() {
    let aba = signal(String::from("ajuda"));
    let mostrar = signal(true);
    let mut app = TestApp::new(move || {
        Form::new("Abas").child(
            TabView::bind(aba)
                .id("abas")
                .grow(1)
                .tab(
                    "ajuda",
                    "Ajuda",
                    Column::new()
                        .grow(1)
                        .child(WebView::html(INICIO).id("pagina").grow(1).visible(mostrar)),
                )
                .tab("outra", "Outra", Text::new("Outra aba").id("outra-texto")),
        )
    })
    .size(480, 400);
    app.frame();
    app.frame();
    let antes = app.webview_shown("pagina");
    assert!(antes.is_some(), "a página aparece na aba escolhida");

    aba.set("outra".into());
    app.frame();
    assert_eq!(
        app.webview_shown("pagina"),
        None,
        "a aba da página ficou escondida"
    );
    aba.set("ajuda".into());
    app.frame();
    assert_eq!(app.webview_shown("pagina"), antes);

    mostrar.set(false);
    app.frame();
    assert_eq!(app.webview_shown("pagina"), None, "o widget ficou oculto");
    mostrar.set(true);
    app.frame();
    assert_eq!(app.webview_shown("pagina"), antes);
}
