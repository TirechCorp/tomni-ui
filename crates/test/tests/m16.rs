//! Marco M16: a `TitleBar` alta e a compacta, ativa e inativa, nos 2 temas, e o que ela pede à
//! janela, pelo runner falso do `TestApp`.

mod comum;

use comum::theme_name;
use tomni_ui::kurbo::Point;
use tomni_ui::prelude::*;
use tomni_ui::theme::Theme;
use tomni_ui::{
    CAPTION_BUTTON_WIDTH, PointerKind, ResizeEdge, TITLE_BAR_COMPACT_HEIGHT, TITLE_BAR_HEIGHT,
    WindowChrome, WindowRequest,
};
use tomni_ui_test::TestApp;

const TEMAS: [Theme; 2] = [Theme::Light, Theme::Dark];

/// Uma janela de notas com a barra compacta: só o ícone, o título e os botões da janela.
fn notas() -> impl View {
    let nota = signal(String::new());
    Form::new("Notas")
        .title_bar(TitleBar::new("Notas").icon(Symbol::Edit))
        .child(
            Column::new().padding(Insets::all(16.0)).child(
                Edit::bind(nota)
                    .id("nota")
                    .height(44)
                    .placeholder("Escreva aqui"),
            ),
        )
}

/// A barra alta: o voltar, o subtítulo, uma busca no meio e um botão à direita. O voltar
/// conta os cliques.
fn leitor() -> impl View {
    let busca = signal(String::new());
    let voltas = signal(0);
    Form::new("Leitor")
        .title_bar(
            TitleBar::new("Leitor")
                .subtitle("Prévia")
                .icon(Symbol::Book)
                .back(true)
                .on_back(move || voltas.update(|n| *n += 1))
                .content(
                    AutoSuggestBox::bind(busca, |_| Vec::new())
                        .id("busca")
                        .width(280)
                        .placeholder("Buscar"),
                )
                .right_header(Button::new("Ana").id("conta")),
        )
        .child(
            Column::new()
                .padding(Insets::all(16.0))
                .child(Text::new(move || format!("Voltas: {}", voltas.get())).id("voltas")),
        )
}

/// A barra com o `NavigationView`: o ☰ vai para a barra, e a busca fica no meio dela.
fn biblioteca() -> impl View {
    let atual = signal("inicio".to_owned());
    let busca = signal(String::new());
    Form::new("Biblioteca")
        .title_bar(
            TitleBar::new("Biblioteca")
                .icon(Symbol::Library)
                .back(true)
                .back_enabled(false)
                .content(
                    AutoSuggestBox::bind(busca, |_| Vec::new())
                        .id("busca")
                        .width(320)
                        .placeholder("Buscar na biblioteca"),
                ),
        )
        .child(
            NavigationView::bind(atual)
                .id("nav")
                .item(
                    "inicio",
                    Symbol::Home,
                    "Início",
                    Column::new()
                        .padding(Insets::all(24.0))
                        .child(Text::new("Início").title()),
                )
                .item("livros", Symbol::Book, "Livros", Text::new("Livros"))
                .settings(Text::new("Configurações")),
        )
}

/// Um ponto livre da barra (sem botão nem conteúdo), à esquerda dos botões da janela.
fn livre(app: &TestApp) -> Point {
    let barra = app.rect("barra-de-titulo");
    Point::new(
        barra.x1 - 3.0 * CAPTION_BUTTON_WIDTH - 20.0,
        barra.center().y,
    )
}

#[test]
fn snapshots_barra_compacta() {
    for theme in TEMAS {
        let mut app = TestApp::new(notas).size(480, 120).theme(theme);
        app.assert_lint();
        app.hover("barra-de-titulo-fechar");
        app.assert_snapshot(&format!("m16_compacta_fechar_hover_{}", theme_name(theme)));
        // Maximizada e inativa: o glifo de restaurar, apagado.
        app.hover("nota")
            .set_maximized(true)
            .set_window_active(false);
        app.assert_snapshot(&format!("m16_compacta_inativa_{}", theme_name(theme)));
    }
}

#[test]
fn snapshots_barra_alta() {
    for theme in TEMAS {
        let mut app = TestApp::new(leitor).size(720, 120).theme(theme);
        app.assert_lint();
        app.hover("barra-de-titulo-maximizar");
        app.assert_snapshot(&format!("m16_alta_maximizar_hover_{}", theme_name(theme)));
        app.hover("voltas").set_window_active(false);
        app.assert_snapshot(&format!("m16_alta_inativa_{}", theme_name(theme)));
    }
}

#[test]
fn snapshots_navegacao_e_busca() {
    for theme in TEMAS {
        let mut app = TestApp::new(biblioteca).size(1008, 360).theme(theme);
        app.assert_lint();
        app.hover("barra-de-titulo-alternar");
        app.assert_snapshot(&format!("m16_navegacao_{}", theme_name(theme)));
    }
}

#[test]
fn medidas_do_winui() {
    let mut app = TestApp::new(notas).size(480, 200);
    assert_eq!(
        app.rect("barra-de-titulo").height(),
        TITLE_BAR_COMPACT_HEIGHT
    );
    let fechar = app.rect("barra-de-titulo-fechar");
    assert_eq!(fechar.size(), (CAPTION_BUTTON_WIDTH, 32.0).into());
    assert_eq!(fechar.x1, 480.0, "o fechar encosta na direita");
    assert!(
        app.rect("nota").y0 >= TITLE_BAR_COMPACT_HEIGHT,
        "o conteúdo vem embaixo"
    );
    app.assert_lint();

    let app = TestApp::new(leitor).size(720, 200);
    assert_eq!(app.rect("barra-de-titulo").height(), TITLE_BAR_HEIGHT);
    assert_eq!(
        app.rect("barra-de-titulo-voltar").size(),
        (44.0, 48.0).into()
    );
    let busca = app.rect("busca");
    assert!(
        busca.y0 > 0.0 && busca.y1 < TITLE_BAR_HEIGHT,
        "a busca cabe na barra"
    );
}

#[test]
fn botoes_da_janela_mandam_os_pedidos() {
    let mut app = TestApp::new(notas).size(480, 200);
    assert_eq!(app.label("barra-de-titulo-maximizar"), "Maximizar");
    app.click("barra-de-titulo-minimizar");
    assert_eq!(app.window_requests(), [WindowRequest::Minimize]);
    app.click("barra-de-titulo-maximizar");
    assert_eq!(app.window_requests(), [WindowRequest::ToggleMaximize]);
    assert!(app.is_maximized());
    assert_eq!(app.label("barra-de-titulo-maximizar"), "Restaurar");
    app.click("barra-de-titulo-maximizar");
    assert!(!app.is_maximized());
    app.click("barra-de-titulo-fechar");
    assert_eq!(
        app.window_requests(),
        [WindowRequest::ToggleMaximize, WindowRequest::Close]
    );
    // Pressionar e soltar fora não clica.
    let fechar = app.rect("barra-de-titulo-fechar").center();
    let fora = Point::new(240.0, 100.0);
    app.ui().pointer_move(fechar, PointerKind::Mouse);
    app.ui().pointer_down(fechar, PointerKind::Mouse, 1);
    app.ui().pointer_move(fora, PointerKind::Mouse);
    app.ui().pointer_up(fora, PointerKind::Mouse);
    app.frame();
    assert!(app.window_requests().is_empty());
}

#[test]
fn duplo_clique_maximiza_e_restaura() {
    let mut app = TestApp::new(notas).size(480, 200);
    let ponto = livre(&app);
    app.click_at(ponto, 1).click_at(ponto, 2);
    assert!(app.is_maximized());
    app.click_at(ponto, 1).click_at(ponto, 2);
    assert!(!app.is_maximized());
    assert_eq!(
        app.window_requests(),
        [WindowRequest::ToggleMaximize, WindowRequest::ToggleMaximize]
    );
    // No título também: ele não é clicável.
    app.double_click("barra-de-titulo");
    assert!(app.is_maximized());
}

#[test]
fn arrastar_a_barra_move_a_janela() {
    let mut app = TestApp::new(notas).size(480, 200);
    let ponto = livre(&app);
    app.drag(ponto, ponto + (2.0, 1.0));
    assert!(app.window_requests().is_empty(), "menos de 4 px é clique");
    app.drag(ponto, ponto + (30.0, 12.0));
    assert_eq!(app.window_requests(), [WindowRequest::Drag]);
}

#[test]
fn controles_da_barra_clicam_sem_arrastar() {
    let mut app = TestApp::new(leitor).size(720, 200);
    app.click("barra-de-titulo-voltar");
    assert_eq!(app.text("voltas"), "Voltas: 1");
    app.type_into("busca", "Iracema");
    assert_eq!(app.text("busca"), "Iracema");
    // Arrastar dentro da busca seleciona o texto, não move a janela.
    let busca = app.rect("busca");
    app.drag(
        Point::new(busca.x0 + 12.0, busca.center().y),
        Point::new(busca.x0 + 60.0, busca.center().y),
    );
    app.click("conta");
    app.double_click("busca");
    assert!(app.window_requests().is_empty());
    assert!(!app.is_maximized());
}

#[test]
fn nav_poe_o_alternar_na_barra() {
    let mut app = TestApp::new(biblioteca).size(800, 400);
    assert!(app.visible("barra-de-titulo-alternar"));
    assert!(!app.enabled("barra-de-titulo-voltar"));
    // Em 800 px, o painel compacto: o ☰ da barra abre por cima.
    assert_eq!(app.label("barra-de-titulo-alternar"), "Abrir navegação");
    app.click("barra-de-titulo-alternar");
    assert_eq!(app.label("barra-de-titulo-alternar"), "Fechar navegação");
    app.click("nav-livros");
    assert_eq!(app.text("nav"), "livros");
    assert!(app.window_requests().is_empty());
    app.assert_lint();
}

#[test]
fn bordas_redimensionam() {
    let mut app = TestApp::new(notas).size(480, 200);
    for (ponto, borda) in [
        (Point::new(2.0, 100.0), ResizeEdge::West),
        (Point::new(478.0, 100.0), ResizeEdge::East),
        (Point::new(240.0, 198.0), ResizeEdge::South),
        (Point::new(240.0, 1.0), ResizeEdge::North),
        (Point::new(1.0, 1.0), ResizeEdge::NorthWest),
        (Point::new(479.0, 199.0), ResizeEdge::SouthEast),
    ] {
        app.click_at(ponto, 1);
        assert_eq!(
            app.window_requests(),
            [WindowRequest::Resize(borda)],
            "{ponto:?}"
        );
    }
    // Maximizada, não redimensiona: o canto de cima à direita é o fechar.
    app.set_maximized(true);
    app.click_at(Point::new(479.0, 1.0), 1);
    assert_eq!(app.window_requests(), [WindowRequest::Close]);
}

#[test]
fn janela_inativa() {
    let mut app = TestApp::new(leitor).size(720, 200);
    let estado = |app: &TestApp| app.tree().find("barra-de-titulo").unwrap().state;
    assert_eq!(estado(&app), Some("active"));
    app.set_window_active(false);
    assert_eq!(estado(&app), Some("inactive"));
    // Inativa, a barra continua funcionando.
    app.click("barra-de-titulo-voltar");
    assert_eq!(app.text("voltas"), "Voltas: 1");
    app.set_window_active(true);
    assert_eq!(estado(&app), Some("active"));
}

#[test]
fn arvore_mostra_a_barra() {
    let app = TestApp::new(notas).size(480, 200);
    let arvore = app.tree();
    let barra = arvore.find("barra-de-titulo").unwrap();
    assert_eq!(barra.kind, "title_bar");
    assert_eq!(barra.text.as_deref(), Some("Notas"));
    for (id, kind, rotulo) in [
        ("barra-de-titulo-minimizar", "window_minimize", "Minimizar"),
        ("barra-de-titulo-maximizar", "window_maximize", "Maximizar"),
        ("barra-de-titulo-fechar", "window_close", "Fechar"),
    ] {
        let node = arvore.find(id).unwrap();
        assert_eq!(node.kind, kind);
        assert_eq!(node.label.as_deref(), Some(rotulo));
    }
}

#[test]
fn fallback_mantem_a_barra_do_sistema() {
    let mut app = TestApp::new(notas)
        .size(480, 200)
        .chrome(WindowChrome::System);
    // A barra do app fica, sem os botões da janela, e não arrasta nem redimensiona.
    assert!(app.visible("barra-de-titulo"));
    for id in ["minimizar", "maximizar", "fechar"] {
        assert!(!app.visible(&format!("barra-de-titulo-{id}")));
    }
    let ponto = livre(&app);
    app.click_at(ponto, 1).click_at(ponto, 2);
    app.drag(ponto, ponto + (40.0, 0.0));
    app.click_at(Point::new(1.0, 100.0), 1);
    assert!(app.window_requests().is_empty());
    assert!(!app.is_maximized());
    assert_eq!(app.ui().resize_edge(Point::new(1.0, 1.0)), None);
    app.type_into("nota", "ok");
    assert_eq!(app.text("nota"), "ok");
    app.assert_lint();
}

#[test]
fn macos_deixa_o_lugar_dos_botoes_do_sistema() {
    let normal = TestApp::new(notas).size(480, 200);
    let mac = TestApp::new(notas)
        .size(480, 200)
        .chrome(WindowChrome::MacOs);
    for id in ["minimizar", "maximizar", "fechar"] {
        assert!(!mac.visible(&format!("barra-de-titulo-{id}")));
    }
    let titulo = |app: &TestApp| {
        app.tree()
            .find("barra-de-titulo")
            .unwrap()
            .children
            .iter()
            .flat_map(|filho| filho.children.iter())
            .find(|n| n.kind == "text")
            .map(|n| n.rect[0])
            .unwrap()
    };
    assert_eq!(
        titulo(&mac) - titulo(&normal),
        78.0,
        "os três botões do macOS"
    );
}

#[test]
fn sem_title_bar_nada_muda() {
    let mut app = TestApp::new(|| {
        let nota = signal(String::new());
        Form::new("Notas").child(Edit::bind(nota).id("nota").height(44))
    })
    .size(480, 200);
    assert!(!app.ui().has_title_bar());
    assert_eq!(app.rect("nota").y0, 0.0);
    app.click_at(Point::new(1.0, 100.0), 1);
    app.click_at(Point::new(1.0, 1.0), 1)
        .click_at(Point::new(1.0, 1.0), 2);
    assert!(app.window_requests().is_empty());
    assert_eq!(app.ui().resize_edge(Point::new(1.0, 1.0)), None);
    app.set_maximized(true);
    assert!(
        !app.is_maximized(),
        "sem barra, o estado da janela não é guardado"
    );
}

/// A barra com um subtítulo `sub` e a busca no meio.
fn com_subtitulo(sub: Option<&'static str>) -> impl View {
    let busca = signal(String::new());
    let mut barra = TitleBar::new("Leitor").icon(Symbol::Book).content(
        AutoSuggestBox::bind(busca, |_| Vec::new())
            .id("busca")
            .width(280)
            .placeholder("Buscar"),
    );
    if let Some(sub) = sub {
        barra = barra.subtitle(sub);
    }
    Form::new("Leitor").title_bar(barra).child(Column::new())
}

#[test]
fn subtitulo_vazio_some_e_nao_desloca_o_conteudo() {
    let busca = |sub: Option<&'static str>| {
        let mut app = TestApp::new(move || com_subtitulo(sub)).size(720, 120);
        app.frame();
        app.rect("busca")
    };
    assert_eq!(busca(Some("")), busca(None));
    assert_ne!(busca(Some("Uma versão bem comprida")), busca(None));
}
