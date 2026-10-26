//! Marco M15: `NavigationView` nos três modos e nos 2 temas, e o comportamento pelo `TestApp`,
//! pela árvore e pelo lint.

mod comum;

use comum::theme_name;
use tomni_ui::kurbo::Point;
use tomni_ui::prelude::*;
use tomni_ui::theme::Theme;
use tomni_ui::{Key, NAV_COMPACT_WIDTH, NAV_ITEM_HEIGHT, NAV_PANE_WIDTH};
use tomni_ui_test::TestApp;

const TEMAS: [Theme; 2] = [Theme::Light, Theme::Dark];

/// Altura das janelas dos snapshots.
const ALTURA: u32 = 520;

fn pagina(titulo: &str, texto: &str) -> Column {
    Column::new()
        .padding(Insets::new(28.0, 40.0, 24.0, 40.0))
        .gap(12)
        .child(Text::new(titulo).title())
        .child(Text::new(texto).wrap())
}

/// Uma biblioteca com busca, um título de seção, um grupo, um item no rodapé e as
/// configurações. A página inicial tem um campo, para ver o estado preservado.
fn biblioteca(escolhido: &str) -> impl View + use<> {
    let atual = signal(escolhido.to_owned());
    let busca = signal(String::new());
    let nota = signal(String::new());
    Form::new("Biblioteca").child(
        NavigationView::bind(atual)
            .id("nav")
            .search(
                AutoSuggestBox::bind(busca, |_| Vec::new())
                    .id("busca")
                    .height(44)
                    .placeholder("Buscar"),
            )
            .item(
                "inicio",
                Symbol::Home,
                "Início",
                pagina("Início", "Os livros, fotos e vídeos guardados aqui.").child(
                    Edit::bind(nota)
                        .id("nota")
                        .height(44)
                        .placeholder("Uma nota"),
                ),
            )
            .header("Coleção")
            .item(
                "livros",
                Symbol::Book,
                "Livros",
                pagina("Livros", "Quatro livros."),
            )
            .group("midia", Symbol::ImageMultiple, "Mídia", |g| {
                g.item(
                    "fotos",
                    Symbol::Camera,
                    "Fotos",
                    pagina("Fotos", "As fotos."),
                )
                .item(
                    "videos",
                    Symbol::Video,
                    "Vídeos",
                    pagina("Vídeos", "Os vídeos."),
                )
            })
            .footer_item(
                "conta",
                Symbol::Person,
                "Conta",
                pagina("Conta", "Ana Souza."),
            )
            .settings(pagina("Configurações", "Tema e idioma.")),
    )
}

fn app(largura: u32, theme: Theme) -> TestApp {
    TestApp::new(|| biblioteca("inicio"))
        .size(largura, ALTURA)
        .theme(theme)
}

// Snapshots

#[test]
fn tres_modos() {
    for theme in TEMAS {
        for (largura, nome, modo) in [
            (1280, "expandido", NavigationMode::Expanded),
            (800, "compacto", NavigationMode::Compact),
            (400, "minimo", NavigationMode::Minimal),
        ] {
            let mut app = app(largura, theme);
            let node = app.tree();
            let nav = node.find("nav").unwrap();
            assert_eq!(nav.variant, Some(modo.name()), "{largura} px");
            app.assert_lint();
            app.assert_snapshot(&format!("modo_{nome}_{}", theme_name(theme)));
        }
    }
}

#[test]
fn medidas_do_painel_e_dos_itens() {
    let mut app = app(1280, Theme::Light);
    assert_eq!(app.rect("nav-painel").width(), NAV_PANE_WIDTH);
    assert_eq!(app.rect("nav-inicio").height(), NAV_ITEM_HEIGHT);
    assert_eq!(app.rect("nav-alternar").height(), NAV_ITEM_HEIGHT);
    app.resize(800, ALTURA);
    assert_eq!(app.rect("nav-painel").width(), NAV_COMPACT_WIDTH);
    assert_eq!(app.rect("nav-inicio").width(), NAV_COMPACT_WIDTH - 4.0);
    // O conteúdo começa depois do painel compacto.
    assert_eq!(app.rect("nav-conteudo").x0, NAV_COMPACT_WIDTH);
}

#[test]
fn escolhido_em_hover_e_pressionado() {
    for theme in TEMAS {
        let mut app = app(1080, theme);
        app.hover("nav-inicio");
        app.assert_snapshot(&format!("escolhido_hover_{}", theme_name(theme)));
        app.press("nav-inicio");
        app.assert_snapshot(&format!("escolhido_pressionado_{}", theme_name(theme)));
        app.release();
    }
}

#[test]
fn grupo_fechado_e_aberto() {
    for theme in TEMAS {
        let mut app = app(1080, theme);
        assert!(!app.visible("nav-fotos"));
        assert_eq!(app.tree().find("nav-midia").unwrap().open, Some(false));
        app.click("nav-midia");
        assert!(app.visible("nav-fotos") && app.visible("nav-videos"));
        assert_eq!(app.tree().find("nav-midia").unwrap().open, Some(true));
        app.hover("nav-fotos");
        app.assert_lint();
        app.assert_snapshot(&format!("grupo_aberto_{}", theme_name(theme)));
        // Abrir o grupo não escolhe nada.
        assert_eq!(app.text("nav"), "inicio");
        app.click("nav-midia");
        assert!(!app.visible("nav-fotos"));
    }
}

#[test]
fn grupo_no_compacto_abre_o_flyout() {
    for theme in TEMAS {
        let mut app = app(800, theme);
        app.click("nav-midia");
        assert!(app.is_open("nav-midia-filhos"));
        assert!(app.visible("nav-fotos-flyout"));
        // O painel continua fechado: o flyout sai ao lado do ícone.
        assert!(app.rect("nav-midia-filhos").x0 > app.rect("nav-midia").x1);
        assert!(!app.tree().find("nav").unwrap().open.unwrap());
        app.assert_lint();
        app.assert_snapshot(&format!("grupo_flyout_{}", theme_name(theme)));
        app.click("nav-fotos-flyout");
        assert!(!app.is_open("nav-midia-filhos"));
        assert_eq!(app.text("nav"), "fotos");
        assert!(app.visible("nav-fotos-conteudo"));
        // O grupo representa o filho escolhido no compacto.
        assert_eq!(app.tree().find("nav-midia").unwrap().selected, Some(true));
    }
}

#[test]
fn flyout_fecha_com_esc() {
    let mut app = app(800, Theme::Light);
    app.click("nav-midia");
    assert!(app.is_open("nav-midia-filhos"));
    assert_eq!(app.focused().as_deref(), Some("nav-fotos-flyout"));
    app.key(Key::Down);
    assert_eq!(app.focused().as_deref(), Some("nav-videos-flyout"));
    app.key(Key::Escape);
    assert!(!app.is_open("nav-midia-filhos"));
    assert_eq!(app.text("nav"), "inicio");
}

#[test]
fn painel_aberto_por_cima_no_compacto_e_no_minimo() {
    for theme in TEMAS {
        for (largura, nome) in [(800, "compacto"), (400, "minimo")] {
            let mut app = app(largura, theme);
            assert!(!app.visible("nav-fora"));
            if largura == 400 {
                assert!(!app.visible("nav-inicio"), "o mínimo esconde os itens");
            }
            app.click("nav-alternar");
            let node = app.tree();
            assert_eq!(node.find("nav").unwrap().open, Some(true));
            assert!(app.visible("nav-fora") && app.visible("nav-inicio"));
            assert_eq!(app.rect("nav-painel").width(), NAV_PANE_WIDTH);
            app.assert_lint();
            app.assert_snapshot(&format!("{nome}_aberto_{}", theme_name(theme)));
        }
    }
}

#[test]
fn minimo_fecha_com_esc_e_com_clique_fora() {
    let mut app = app(400, Theme::Light);
    app.click("nav-alternar");
    assert!(app.visible("nav-inicio"));
    app.key(Key::Escape);
    assert!(!app.visible("nav-inicio"));
    assert!(!app.visible("nav-fora"));

    app.click("nav-alternar");
    assert!(app.visible("nav-inicio"));
    let fora = Point::new(380.0, 300.0);
    app.click_at(fora, 1);
    assert!(!app.visible("nav-inicio"));
    assert_eq!(app.text("nav"), "inicio", "o clique fora não escolhe nada");

    // Escolher um item também fecha o painel.
    app.click("nav-alternar").click("nav-livros");
    assert_eq!(app.text("nav"), "livros");
    assert!(!app.visible("nav-livros"));
}

// Comportamento

#[test]
fn escolher_troca_a_pagina_e_preserva_o_estado() {
    let mut app = app(1280, Theme::Light);
    assert!(app.visible("nav-inicio-conteudo"));
    assert!(!app.visible("nav-livros-conteudo"));
    app.type_into("nota", "ler Iracema");
    app.click("nav-livros");
    assert_eq!(app.text("nav"), "livros");
    assert!(app.visible("nav-livros-conteudo"));
    assert!(!app.visible("nav-inicio-conteudo"));
    app.click("nav-configuracoes");
    assert_eq!(app.text("nav"), "configuracoes");
    app.click("nav-inicio");
    assert_eq!(
        app.text("nota"),
        "ler Iracema",
        "a página escondida continua montada"
    );
}

#[test]
fn limites_de_largura_trocam_o_modo() {
    let mut app = app(1008, Theme::Light);
    let modo = |app: &TestApp| app.tree().find("nav").unwrap().variant;
    assert_eq!(modo(&app), Some("expanded"));
    assert_eq!(app.tree().find("nav").unwrap().open, Some(true));
    for (largura, esperado) in [
        (1007, "compact"),
        (641, "compact"),
        (640, "minimal"),
        (641, "compact"),
        (1008, "expanded"),
    ] {
        app.resize(largura, ALTURA);
        assert_eq!(modo(&app), Some(esperado), "{largura} px");
    }
    // Voltar ao expandido abre o painel de novo.
    assert_eq!(app.tree().find("nav").unwrap().open, Some(true));
    assert_eq!(NavigationMode::for_width(1007.5), NavigationMode::Compact);
}

#[test]
fn setas_andam_e_enter_escolhe() {
    let mut app = app(1280, Theme::Light);
    // O painel é uma parada só de Tab: o ☰, a busca e o item escolhido.
    app.tab();
    assert_eq!(app.focused().as_deref(), Some("nav-alternar"));
    app.tab();
    assert_eq!(app.focused().as_deref(), Some("busca"));
    app.tab();
    assert_eq!(app.focused().as_deref(), Some("nav-inicio"));
    app.key(Key::Down);
    assert_eq!(app.focused().as_deref(), Some("nav-livros"));
    assert_eq!(app.text("nav"), "inicio", "andar não escolhe");
    app.key(Key::Down);
    assert_eq!(app.focused().as_deref(), Some("nav-midia"));
    // → abre o grupo e ← fecha.
    app.key(Key::Right);
    assert!(app.visible("nav-fotos"));
    app.key(Key::Down);
    assert_eq!(app.focused().as_deref(), Some("nav-fotos"));
    app.key(Key::Enter);
    assert_eq!(app.text("nav"), "fotos");
    app.key(Key::Left);
    assert_eq!(
        app.focused().as_deref(),
        Some("nav-midia"),
        "← no filho sobe ao grupo"
    );
    app.key(Key::Left);
    assert!(!app.visible("nav-fotos"));
    app.key(Key::Home);
    assert_eq!(app.focused().as_deref(), Some("nav-inicio"));
    // End vai ao último item (as configurações, no rodapé) e Espaço escolhe.
    app.key(Key::End);
    assert_eq!(app.focused().as_deref(), Some("nav-configuracoes"));
    app.key(Key::Space);
    assert_eq!(app.text("nav"), "configuracoes");
    // Tab sai do painel para a página.
    app.key(Key::Home).key(Key::Enter);
    assert_eq!(app.text("nav"), "inicio");
    app.tab();
    assert_eq!(app.focused().as_deref(), Some("nota"));
}

#[test]
fn select_pelo_rotulo() {
    for largura in [1280, 800, 400] {
        let mut app = app(largura, Theme::Light);
        app.select("nav", "Vídeos");
        assert_eq!(app.text("nav"), "videos", "{largura} px");
        assert!(app.visible("nav-videos-conteudo"));
        app.select("nav", "Configurações");
        assert_eq!(app.text("nav"), "configuracoes", "{largura} px");
        app.select("nav", "Início");
        assert_eq!(app.text("nav"), "inicio", "{largura} px");
    }
}

#[test]
#[should_panic(expected = "Livros")]
fn select_com_rotulo_errado_lista_os_rotulos() {
    let mut app = app(1280, Theme::Light);
    app.select("nav", "Revistas");
}

#[test]
fn arvore_mostra_modo_painel_pagina_e_grupos() {
    let mut app = app(1280, Theme::Light);
    app.click("nav-midia");
    let node = app.tree();
    let nav = node.find("nav").unwrap();
    assert_eq!(nav.kind, "navigation_view");
    assert_eq!(nav.variant, Some("expanded"));
    assert_eq!(nav.open, Some(true));
    assert_eq!(nav.text.as_deref(), Some("inicio"));
    let inicio = node.find("nav-inicio").unwrap();
    assert_eq!(inicio.kind, "navigation_item");
    assert_eq!(inicio.label.as_deref(), Some("Início"));
    assert_eq!(inicio.selected, Some(true));
    assert_eq!(inicio.icon, Some("Home"));
    assert_eq!(node.find("nav-livros").unwrap().selected, Some(false));
    assert_eq!(node.find("nav-midia").unwrap().open, Some(true));
    assert_eq!(node.find("nav-livros").unwrap().open, None);
}

#[test]
fn grupo_fechado_representa_o_filho_escolhido() {
    let mut app = TestApp::new(|| biblioteca("videos")).size(1280, ALTURA);
    assert!(app.visible("nav-videos-conteudo"));
    assert!(!app.visible("nav-videos"));
    assert_eq!(app.tree().find("nav-midia").unwrap().selected, Some(true));
    app.click("nav-midia");
    let node = app.tree();
    assert_eq!(node.find("nav-midia").unwrap().selected, Some(false));
    assert_eq!(node.find("nav-videos").unwrap().selected, Some(true));
    app.assert_lint();
}

#[test]
fn lupa_do_compacto_abre_o_painel_na_busca() {
    let mut app = app(800, Theme::Light);
    assert!(app.visible("nav-pesquisar"));
    assert!(!app.visible("busca"));
    app.click("nav-pesquisar");
    assert_eq!(app.tree().find("nav").unwrap().open, Some(true));
    assert!(app.visible("busca"));
    assert_eq!(app.focused().as_deref(), Some("busca"));
    // Esc com o foco na busca vazia fecha o painel.
    app.key(Key::Escape);
    assert_eq!(app.tree().find("nav").unwrap().open, Some(false));
}
