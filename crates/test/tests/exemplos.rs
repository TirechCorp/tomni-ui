//! Os exemplos de `crates/tomni-ui/examples`: cada tela passa no lint, nos temas claro e
//! escuro, e se comporta como o comentário do exemplo diz.
//!
//! Os arquivos entram como módulos (`#[path]`): o `main` deles não roda aqui.

use std::time::Duration;
use tomni_ui::prelude::*;
use tomni_ui::terminal::Session;

use tomni_ui::theme::Theme;
use tomni_ui::{Key, Look, Modifiers, PASSWORD_CHAR, SPIN_REPEAT, TOOLTIP_DELAY, WindowRequest};
use tomni_ui_test::TestApp;

#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/auto_suggest_box.rs"]
mod auto_suggest_box;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/bandeja.rs"]
mod bandeja;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/border.rs"]
mod border;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/button.rs"]
mod button;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/card.rs"]
mod card;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/checkbox.rs"]
mod checkbox;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/combo_box.rs"]
mod combo_box;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/content_dialog.rs"]
mod content_dialog;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/data_grid.rs"]
mod data_grid;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/dock.rs"]
mod dock;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/edit.rs"]
mod edit;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/expander.rs"]
mod expander;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/flyout.rs"]
mod flyout;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/form.rs"]
mod form;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/ia.rs"]
mod ia;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/icon.rs"]
mod icon;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/image.rs"]
mod image;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/info_bar.rs"]
mod info_bar;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/janelas.rs"]
mod janelas;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/layout.rs"]
mod layout;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/lista.rs"]
mod lista;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/login.rs"]
mod login;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/markdown.rs"]
mod markdown;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/menu_bar.rs"]
mod menu_bar;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/menu_flyout.rs"]
mod menu_flyout;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/navigation_view.rs"]
mod navigation_view;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/number_box.rs"]
mod number_box;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/password_box.rs"]
mod password_box;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/progress.rs"]
mod progress;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/property_grid.rs"]
mod property_grid;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/radio_buttons.rs"]
mod radio_buttons;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/rich_text.rs"]
mod rich_text;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/scroll_viewer.rs"]
mod scroll_viewer;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/settings.rs"]
mod settings;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/show.rs"]
mod show;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/slider.rs"]
mod slider;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/split.rs"]
mod split;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/strings.rs"]
mod strings;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/tab_view.rs"]
mod tab_view;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/terminal.rs"]
mod terminal;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/terminal_panes.rs"]
mod terminal_panes;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/text.rs"]
mod text;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/text_editor.rs"]
mod text_editor;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/title_bar.rs"]
mod title_bar;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/toggle_button.rs"]
mod toggle_button;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/toggle_switch.rs"]
mod toggle_switch;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/tooltip.rs"]
mod tooltip;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/tree_view.rs"]
mod tree_view;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/uniform_grid.rs"]
mod uniform_grid;
#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/webview.rs"]
mod webview;

/// Os exemplos testados abaixo, cada um com um `#[test]` de mesmo nome.
const EXEMPLOS: [&str; 50] = [
    "auto_suggest_box",
    "bandeja",
    "border",
    "button",
    "card",
    "checkbox",
    "combo_box",
    "content_dialog",
    "data_grid",
    "dock",
    "edit",
    "expander",
    "flyout",
    "form",
    "ia",
    "icon",
    "image",
    "info_bar",
    "janelas",
    "layout",
    "lista",
    "login",
    "markdown",
    "menu_bar",
    "menu_flyout",
    "navigation_view",
    "number_box",
    "password_box",
    "progress",
    "property_grid",
    "radio_buttons",
    "rich_text",
    "scroll_viewer",
    "settings",
    "show",
    "slider",
    "split",
    "strings",
    "tab_view",
    "terminal",
    "terminal_panes",
    "text",
    "text_editor",
    "title_bar",
    "toggle_button",
    "toggle_switch",
    "tooltip",
    "tree_view",
    "uniform_grid",
    "webview",
];

fn app<V: View>(view: impl FnOnce() -> V, theme: Theme) -> TestApp {
    TestApp::new(view).size(390, 844).theme(theme)
}

/// Roda `check` na tela, nos dois temas.
fn nos_dois_temas<V: View>(view: impl Fn() -> V, check: impl Fn(&mut TestApp)) {
    for theme in [Theme::Light, Theme::Dark] {
        let mut app = app(&view, theme);
        check(&mut app);
    }
}

#[test]
fn todo_exemplo_esta_aqui() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../tomni-ui/examples");
    let mut arquivos: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| {
            let name = e.unwrap().file_name().into_string().unwrap();
            name.strip_suffix(".rs").map(str::to_owned)
        })
        .collect();
    arquivos.sort();
    assert_eq!(
        arquivos, EXEMPLOS,
        "exemplo novo: inclua o módulo e as telas dele neste arquivo"
    );
}

#[test]
fn button() {
    nos_dois_temas(button::botoes, |app| {
        app.assert_lint();
        assert!(!app.enabled("zerar"));
        assert!(!app.enabled("desabilitado"));
        assert_eq!(app.rect("menos").width(), app.rect("mais").width());
        app.click("contar");
        app.click("mais");
        assert_eq!(app.label("contar"), "Cliques: 2");
        app.click("menos");
        assert_eq!(app.label("contar"), "Cliques: 1");
        app.click("zerar");
        assert_eq!(app.label("contar"), "Cliques: 0");
        app.click("desabilitado");
        app.assert_lint();
        // O link fica da largura do texto, mas alto o bastante para o toque.
        assert!(app.rect("dez").width() < 200.0);
        assert_eq!(app.rect("dez").height(), 44.0);
        app.click("dez");
        assert_eq!(app.label("contar"), "Cliques: 10");
    });
}

#[test]
fn edit() {
    nos_dois_temas(edit::campos, |app| {
        app.assert_lint();
        assert!(!app.enabled("limpar"));
        assert!(!app.enabled("travado"));
        app.type_into("nome", "Ana").type_into("senha", "segredo");
        assert_eq!(app.label("limpar"), "Limpar (3 letras)");
        app.assert_lint();
        app.click("limpar");
        assert_eq!(app.text("nome"), "");
        assert_eq!(app.text("senha"), "segredo");
        assert_eq!(app.text("travado"), "Não dá para editar");
    });
}

#[test]
fn form() {
    nos_dois_temas(form::cadastro, |app| {
        app.assert_lint();
        app.type_into("nome", "Ana")
            .type_into("email", "ana@x.com")
            .type_into("senha", "123456")
            .type_into("confirmar", "12345");
        assert!(!app.enabled("criar"), "senhas diferentes");
        app.type_into("confirmar", "6");
        assert!(app.enabled("criar"));
        app.click("criar");
        assert_eq!(app.label("criar"), "Conta criada");
        app.assert_lint();
        app.click("limpar");
        assert_eq!(app.text("email"), "");
        assert_eq!(app.label("criar"), "Criar conta");
        assert!(!app.enabled("criar"));
    });
}

#[test]
fn layout() {
    nos_dois_temas(layout::coluna, |app| {
        app.assert_lint();
        // Centralizados nos dois eixos, cada um da largura do próprio texto.
        let (primeiro, segundo) = (app.rect("primeiro"), app.rect("segundo"));
        assert!(primeiro.width() < segundo.width());
        for rect in [primeiro, segundo, app.rect("terceiro")] {
            assert!((rect.center().x - 195.0).abs() < 1.0, "{rect:?}");
        }
        assert!(primeiro.y0 > 300.0, "{primeiro:?}");
    });
    nos_dois_temas(layout::linha, |app| {
        app.assert_lint();
        assert_eq!(app.rect("voltar").x0, 24.0);
        assert_eq!(app.rect("ajuda").x1, 366.0);
        assert_eq!(app.rect("ok").x1, 366.0);
        assert_eq!(app.rect("ok").width(), 88.0);
        assert!(app.rect("cancelar").x1 <= app.rect("ok").x0 - 8.0);
    });
}

#[test]
fn login() {
    nos_dois_temas(login::login, |app| {
        app.assert_lint();
        app.type_into("email", "ana@x.com")
            .type_into("senha", "123456");
        app.click("entrar");
        assert_eq!(app.label("entrar"), "Bem-vindo!");
        app.assert_lint();
    });
}

#[test]
fn checkbox() {
    nos_dois_temas(checkbox::caixas, |app| {
        app.assert_lint();
        assert!(!app.enabled("continuar"));
        assert!(app.checked("lembrar"));
        app.click("termos");
        assert!(app.checked("termos"));
        assert!(app.enabled("continuar"));
        app.click("bloqueada");
        assert!(app.checked("bloqueada"));
        app.tab();
        app.key(tomni_ui::Key::Space);
        assert!(!app.checked("lembrar"));
        app.assert_lint();
    });
}

#[test]
fn info_bar() {
    nos_dois_temas(info_bar::avisos, |app| {
        app.assert_lint();
        assert!(!app.visible("reabrir"));
        app.click("erro");
        assert!(!app.visible("erro"));
        assert!(app.visible("reabrir"));
        app.assert_lint();
        app.click("reabrir");
        assert!(app.visible("erro"));
        assert_eq!(app.text("erro"), "Este e-mail já está cadastrado.");
    });
}

#[test]
fn text() {
    nos_dois_temas(text::textos, |app| {
        app.assert_lint();
        let paragrafo = app.rect("paragrafo");
        assert!(
            paragrafo.height() > 20.0,
            "o parágrafo quebra: {paragrafo:?}"
        );
        assert!(paragrafo.width() <= 342.0);
        let tree = app.tree();
        let resumo = tree.find("resumo").unwrap();
        assert!(resumo.trimmed, "o resumo é cortado em duas linhas");
        assert_eq!(app.rect("resumo").height(), 40.0);
        app.click("mais");
        assert_eq!(app.text("contagem"), "1 de 8 caracteres");
    });
}

#[test]
fn rich_text() {
    for theme in [Theme::Light, Theme::Dark] {
        let mut app = app(rich_text::texto_rico, theme);
        app.assert_lint();
        let paragrafo = app.rect("paragrafo");
        assert!(
            paragrafo.height() > 40.0,
            "o parágrafo quebra: {paragrafo:?}"
        );
        assert!(paragrafo.width() <= 342.0);
        let tree = app.tree();
        let resumo = tree.find("resumo").unwrap();
        assert!(resumo.trimmed, "o resumo é cortado em duas linhas");
        assert_eq!(app.rect("resumo").height(), 40.0);
        let texto = tree.find("paragrafo").unwrap();
        assert_eq!(texto.kind, "rich_text");
        assert!(
            texto
                .text
                .as_deref()
                .unwrap()
                .starts_with("Para entrar, rode ssh")
        );
        let manual = tree.find("paragrafo-link-0").unwrap();
        assert_eq!(manual.kind, "link");
        assert_eq!(manual.text.as_deref(), Some("manual"));
        assert_eq!(manual.url.as_deref(), Some("https://exemplo.com/manual"));
        assert!(tree.find("paragrafo-link-1").is_some());
        assert!(tree.find("paragrafo-link-2").is_none());
        app.click("paragrafo-link-1");
        assert_eq!(app.text("aberto"), "https://exemplo.com/lista");
        app.click("paragrafo-link-0");
        assert_eq!(app.text("aberto"), "https://exemplo.com/manual");
    }
}

#[test]
fn text_editor() {
    nos_dois_temas(text_editor::editor_de_texto, |app| {
        app.assert_lint();
        let editor = app.rect("editor");
        assert!(editor.height() >= 96.0, "o editor tem altura: {editor:?}");
        let tree = app.tree();
        let no = tree.find("editor").unwrap();
        assert_eq!(no.kind, "text_editor");
        assert!(
            no.text
                .as_deref()
                .unwrap()
                .starts_with("# Notas do servidor")
        );
        assert_eq!(app.text("contagem"), "5 linhas · 257 caracteres");
        app.type_into("editor", "\nfim");
        assert_eq!(app.text("contagem"), "6 linhas · 261 caracteres");
        app.click("achar");
        let visual = app.text_editor("editor");
        assert_eq!(visual.marks.len(), 1, "um nginx no texto");
        assert!(visual.marks[0].current);
    });
}

#[test]
fn ia() {
    for theme in [Theme::Light, Theme::Dark] {
        let mut app = TestApp::new(ia::perguntar).size(720, 520).theme(theme);
        app.assert_lint();
        assert!(app.enabled("perguntar"));
        assert!(!app.enabled("parar"));
        app.click("perguntar");
        assert!(
            !app.enabled("perguntar"),
            "enquanto roda, não pergunta de novo"
        );
        assert!(app.enabled("parar"));

        // A resposta chega por outra thread: espera o fim, quadro a quadro.
        for _ in 0..300 {
            if app.enabled("perguntar") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
            app.frame();
        }
        assert!(app.enabled("perguntar"), "a resposta não terminou");
        let arvore = app.tree();
        assert!(
            arvore.find("resposta-bloco-0").is_some(),
            "o bloco de código da resposta"
        );
        assert!(arvore.find("resposta-copiar-0").is_some());
        app.click("resposta-copiar-0");
        assert_eq!(app.clipboard().read().as_deref(), Some("ss -tlnp"));
    }
}

#[test]
fn markdown() {
    for theme in [Theme::Light, Theme::Dark] {
        let mut app = TestApp::new(markdown::notas).size(1000, 720).theme(theme);
        app.assert_lint();
        let tree = app.tree();
        assert_eq!(tree.find("previa").unwrap().kind, "markdown");
        for id in [
            "previa-tarefa-0",
            "previa-tarefa-2",
            "previa-texto-0",
            "previa-link-0",
            "previa-bloco-0",
            "previa-copiar-0",
            "previa-tabela-0",
            "previa-imagem-0",
        ] {
            assert!(tree.find(id).is_some(), "falta {id}");
        }
        assert!(app.checked("previa-tarefa-0"));
        assert!(!app.checked("previa-tarefa-1"));

        // Marcar a tarefa na prévia troca o `[ ]` no texto do editor.
        app.click("previa-tarefa-1");
        let texto = app.text_editor("editor").text.to_string();
        assert!(texto.contains("- [x] Conferir o backup"), "{texto}");
        assert!(texto.contains("- [ ] Revisar os logs"));
        assert!(app.checked("previa-tarefa-1"));

        // O link chama o `on_link`.
        app.click("previa-link-0");
        assert_eq!(app.text("estado"), "Link: https://tirech.com.br");
        app.click("previa-link-1");
        assert_eq!(app.text("estado"), "Link: https://exemplo.com/diagrama.png");

        // O botão copia o bloco de código para a área de transferência.
        app.click("previa-copiar-0");
        assert_eq!(
            app.clipboard().read().as_deref(),
            Some("sudo systemctl restart nginx")
        );

        // A prévia acompanha o que se digita.
        app.type_into("editor", "\n## Fim");
        assert!(app.tree().find("previa").is_some());
        assert!(app.text("previa-texto-0").contains("Um parágrafo"));
    }
}

#[test]
fn border() {
    nos_dois_temas(border::destaque, |app| {
        app.assert_lint();
        let tree = app.tree();
        let faixa = tree.find("faixa").unwrap();
        assert_eq!(faixa.kind, "border");
        assert_eq!(
            faixa.background.as_deref(),
            Some("linear-gradient(#003E92 0%, #6B2FA0 100%)")
        );
        assert_eq!(
            tree.find("quadro").unwrap().background.as_deref(),
            Some("LayerFillColorDefault")
        );
    });
}

#[test]
fn settings() {
    nos_dois_temas(settings::tela, |app| {
        app.assert_lint();
        // A 390 px de janela o card tem menos de 476: o controle desce para baixo do texto.
        let tree = app.tree();
        assert_eq!(tree.find("notificacoes").unwrap().state, None);
        assert_eq!(tree.find("sobre").unwrap().state, Some("rest"));
        assert_eq!(app.text("ultimo"), "Nada aberto");
        app.click("sobre");
        assert_eq!(app.text("ultimo"), "Aberto: Sobre");
        app.click("conta");
        assert_eq!(
            app.text("ultimo"),
            "Aberto: Sobre",
            "o desabilitado não abre"
        );
        assert!(!app.enabled("conta"));
    });
}

#[test]
fn card() {
    nos_dois_temas(card::configuracoes, |app| {
        app.assert_lint();
        assert_eq!(app.text("aberto"), "Escolha uma opção");
        app.click("notificacoes");
        assert_eq!(app.text("aberto"), "Aberto: Notificações");
        app.tab().key(Key::Enter);
        assert_eq!(app.text("aberto"), "Aberto: Privacidade");
        assert!(!app.enabled("conta"));
        app.tab();
        assert_ne!(
            app.focused().as_deref(),
            Some("conta"),
            "o card desabilitado não recebe foco"
        );
        let tree = app.tree();
        assert_eq!(
            tree.find("versao").unwrap().state,
            None,
            "o card sem clique não tem estado"
        );
        assert_eq!(app.label("tema"), "Tema");
        app.assert_lint();
    });
}

#[test]
fn toggle_button() {
    nos_dois_temas(toggle_button::formatacao, |app| {
        app.assert_lint();
        assert_eq!(app.text("resumo"), "Ligado: negrito");
        app.click("italico");
        assert!(app.checked("italico"));
        assert_eq!(app.text("resumo"), "Ligado: negrito, itálico");
        app.key(Key::Enter);
        assert_eq!(
            app.text("resumo"),
            "Ligado: negrito",
            "Enter alterna o botão com foco"
        );
        app.click("negrito");
        assert_eq!(app.text("resumo"), "Sem formatação");
        app.assert_lint();
    });
}

#[test]
fn uniform_grid() {
    nos_dois_temas(uniform_grid::galeria, |app| {
        app.assert_lint();
        // 342 px de largura útil: duas colunas de pelo menos 160.
        let x: Vec<f64> = ["botoes", "texto", "cards"]
            .iter()
            .map(|id| app.rect(id).x0)
            .collect();
        assert_eq!(x[0], x[2]);
        assert!(x[1] > x[0]);
        app.resize(700, 844);
        let y: Vec<f64> = ["botoes", "texto", "cards"]
            .iter()
            .map(|id| app.rect(id).y0)
            .collect();
        assert!(y.iter().all(|&v| v == y[0]), "três colunas: {y:?}");
        app.click("listas");
        assert_eq!(app.text("aberto"), "Aberto: Listas");
        app.assert_lint();
    });
}

#[test]
fn toggle_switch() {
    nos_dois_temas(toggle_switch::chaves, |app| {
        app.assert_lint();
        assert_eq!(app.text("resumo"), "Avisos em silêncio");
        app.click("som");
        assert_eq!(app.text("resumo"), "Avisos com som");
        app.click("notificacoes");
        assert!(!app.enabled("som"));
        assert_eq!(app.text("resumo"), "Sem avisos");
        app.assert_lint();
    });
}

#[test]
fn show() {
    nos_dois_temas(show::conta, |app| {
        app.assert_lint();
        assert!(app.ui().find("apelido").is_none());
        app.click("entrar");
        assert!(app.ui().find("aviso").is_none());
        app.type_into("apelido", "Ana");
        app.type_into("rascunho", "comprar pão");
        app.assert_lint();

        // `Show` descarta o campo; `.visible()` só esconde.
        app.click("entrar").click("notas");
        assert!(app.ui().find("apelido").is_none());
        assert!(!app.visible("rascunho"));
        app.click("entrar").click("notas");
        assert_eq!(app.text("apelido"), "");
        assert_eq!(app.text("rascunho"), "comprar pão");
        app.assert_lint();
    });
}

#[test]
fn lista() {
    nos_dois_temas(lista::playlist, |app| {
        app.assert_lint();
        app.click("favorita-3");
        assert_eq!(
            app.tree().find("total").unwrap().text.as_deref(),
            Some("Favoritas: 1")
        );

        // Subir troca de posição sem recriar: a marca continua.
        let y = app.rect("favorita-3").y0;
        app.click("subir-3");
        assert!(app.rect("favorita-3").y0 < y);
        assert!(app.checked("favorita-3"));

        app.click("remover-3");
        assert!(app.ui().find("favorita-3").is_none());
        assert_eq!(
            app.tree().find("total").unwrap().text.as_deref(),
            Some("Favoritas: 0")
        );
        assert!(app.ui().find("vazia").is_none());
        for id in [1, 2, 4, 5] {
            app.click(&format!("remover-{id}"));
        }
        assert!(app.ui().find("vazia").is_some());
        app.assert_lint();
        app.click("restaurar");
        assert!(app.ui().find("vazia").is_none());
        assert!(!app.checked("favorita-3"));
        app.assert_lint();
    });
}

#[test]
fn scroll_viewer() {
    nos_dois_temas(scroll_viewer::rolagem, |app| {
        app.assert_lint();
        assert!(app.in_view("aviso-1"));
        assert!(!app.in_view("aviso-40"));
        assert!(!app.in_view("cat-compras"));

        app.scroll("avisos", 1e6);
        assert!(app.in_view("aviso-40"));
        assert_eq!(app.repaints_since_last(), ["avisos"]);
        app.scroll("avisos", -1e6);

        // A faixa horizontal rola com a roda comum, e clicar rola até o botão.
        let faixa = app.rect("categorias").center();
        app.wheel(faixa, tomni_ui::kurbo::Vec2::new(0.0, 100.0));
        assert!(app.scroll_offset("categorias").x > 0.0);
        app.click("cat-compras");
        assert!(app.in_view("cat-compras"));
        assert_eq!(
            app.tree().find("categoria").unwrap().text.as_deref(),
            Some("Categoria: Compras")
        );
        app.assert_lint();
    });
}

#[test]
fn combo_box() {
    nos_dois_temas(combo_box::endereco, |app| {
        app.assert_lint();
        assert_eq!(app.text("uf"), "");
        assert_eq!(app.text("resumo"), "Nenhuma UF escolhida");
        app.click("uf");
        assert!(app.is_open("uf"));
        app.assert_lint();
        app.click("uf-sp");
        assert!(!app.is_open("uf"));
        assert_eq!(app.text("uf"), "SP");
        app.select("cidade", "São Paulo");
        assert_eq!(app.text("resumo"), "São Paulo, SP");
        assert!(!app.enabled("pais"));
        app.assert_lint();
    });
}

#[test]
fn content_dialog() {
    nos_dois_temas(content_dialog::sair, |app| {
        app.assert_lint();
        assert!(!app.is_open("dialogo"));
        app.click("sair");
        assert!(app.is_open("dialogo"));
        app.assert_lint();
        app.key(Key::Escape);
        assert!(!app.is_open("dialogo"));
        assert_eq!(app.text("resultado"), "Há alterações não salvas.");
        app.click("sair");
        app.click("dialogo-secondary");
        assert_eq!(app.text("resultado"), "Alterações descartadas.");
        app.click("sair");
        app.key(Key::Enter);
        assert!(!app.is_open("dialogo"));
        assert_eq!(app.text("resultado"), "Salvo.");
    });
}

#[test]
fn flyout() {
    nos_dois_temas(flyout::ajuda, |app| {
        app.assert_lint();
        app.click("ajuda");
        assert!(app.is_open("ajuda-popup"));
        app.assert_lint();
        app.click("entendi");
        assert!(!app.is_open("ajuda-popup"));
        app.click("ajuda");
        app.click("senha");
        assert!(!app.is_open("ajuda-popup"), "o clique fora fecha");
    });
}

#[test]
fn menu_flyout() {
    nos_dois_temas(menu_flyout::menu, |app| {
        app.assert_lint();
        app.click("mais");
        assert!(app.is_open("menu"));
        assert_eq!(app.label("apagar"), "Apagar");
        app.assert_lint();
        app.click("copiar");
        assert!(!app.is_open("menu"));
        assert_eq!(app.text("ultima"), "Link copiado");
        app.click("mais");
        app.key(Key::Escape);
        assert!(!app.is_open("menu"));
        assert_eq!(app.text("ultima"), "Link copiado");
        // O grupo de rádios: um marcado, o clique troca e fecha.
        assert_eq!(app.text("ordem-atual"), "Ordem: nome");
        app.click("ordenar");
        assert!(app.checked("ordem-nome"));
        app.assert_lint();
        assert!(!app.checked("ordem-data"));
        app.click("ordem-data");
        assert!(!app.is_open("ordem"));
        assert_eq!(app.text("ordem-atual"), "Ordem: data");
    });
}

#[test]
fn tooltip() {
    nos_dois_temas(tooltip::barra, |app| {
        app.assert_lint();
        app.hover("mover");
        assert!(!app.is_open("mover-tooltip"), "a dica espera");
        app.wait(TOOLTIP_DELAY + Duration::from_millis(10));
        assert!(app.is_open("mover-tooltip"));
        app.assert_lint();
        app.click("copiar");
        assert!(!app.is_open("mover-tooltip"));
        assert_eq!(app.text("copias"), "Links copiados: 1");
    });
}

#[test]
fn radio_buttons() {
    nos_dois_temas(radio_buttons::aparencia, |app| {
        app.assert_lint();
        assert_eq!(app.text("tema"), "Do sistema");
        assert!(app.checked("tema-do-sistema"));
        assert_eq!(app.label("tema-claro"), "Claro");
        app.click("tema-escuro");
        assert!(app.checked("tema-escuro"));
        assert!(!app.checked("tema-do-sistema"));
        assert_eq!(app.text("resumo"), "Tema: Escuro");
        // ↑ volta uma opção, e o grupo é uma parada só do Tab.
        app.key(Key::Up);
        assert_eq!(app.text("tema"), "Claro");
        assert_eq!(app.focused().as_deref(), Some("tema-claro"));
        app.select("tema", "Do sistema");
        assert_eq!(app.text("resumo"), "Tema: Do sistema");
        assert!(!app.enabled("tamanho"));
        app.click("tamanho-grande");
        assert_eq!(app.text("tamanho"), "Médio");
        app.assert_lint();
    });
}

#[test]
fn slider() {
    nos_dois_temas(slider::som, |app| {
        app.assert_lint();
        assert_eq!(app.value("volume"), 40.0);
        assert_eq!(app.label("volume"), "Volume");
        assert_eq!(app.text("resumo"), "Volume: 40%");
        app.set_value("volume", 72.0);
        assert_eq!(app.value("volume"), 70.0, "arredonda para o passo de 5");
        assert_eq!(app.text("resumo"), "Volume: 70%");
        app.key(Key::Right);
        assert_eq!(app.value("volume"), 75.0);
        app.key(Key::End);
        assert_eq!(app.text("resumo"), "Volume: 100%");
        assert!(!app.enabled("brilho"));
        assert_eq!(app.value("brilho"), 0.8);
        app.assert_lint();
    });
}

#[test]
fn progress() {
    nos_dois_temas(progress::envio, |app| {
        app.assert_lint();
        assert_eq!(app.value("barra"), 0.25);
        assert_eq!(app.value("anel"), 0.25);
        assert_eq!(app.text("resumo"), "25% enviado");
        assert_eq!(app.tree().find("procurando").unwrap().value, None);
        // Os indeterminados se repintam enquanto o tempo passa.
        app.repaints_since_last();
        app.wait(Duration::from_millis(100));
        let repintados = app.repaints_since_last();
        for id in ["procurando", "carregando"] {
            assert!(repintados.iter().any(|r| r == id), "{repintados:?}");
        }
        app.click("avancar").click("avancar").click("avancar");
        assert_eq!(app.value("barra"), 1.0);
        assert_eq!(app.text("resumo"), "100% enviado");
        assert!(!app.enabled("avancar"));
        app.assert_lint();
    });
}

#[test]
fn expander() {
    nos_dois_temas(expander::notificacoes, |app| {
        app.assert_lint();
        assert!(app.is_open("sons"));
        assert!(!app.is_open("avancado"));
        assert_eq!(app.label("avancado"), "Avançado");
        assert!(app.visible("vibrar"));
        assert!(!app.visible("resumo-diario"));
        let y = app.rect("estado").y0;
        app.click("avancado");
        assert!(app.is_open("avancado"));
        assert!(app.visible("resumo-diario"));
        assert_eq!(app.text("estado"), "Opções avançadas à mostra");
        assert!(app.rect("estado").y0 > y, "o texto de baixo desce");
        app.assert_lint();
        app.click("resumo-diario");
        assert!(app.checked("resumo-diario"));
        app.click("sons");
        assert!(!app.is_open("sons"));
        assert!(!app.visible("vibrar"));
        app.key(Key::Enter);
        assert!(app.is_open("sons"));
        assert!(app.checked("vibrar"), "fechar não descarta o conteúdo");
    });
}

#[test]
fn navigation_view() {
    // Na largura do celular, o painel mínimo: só o ☰.
    nos_dois_temas(navigation_view::biblioteca, |app| {
        app.assert_lint();
        assert_eq!(app.text("biblioteca"), "inicio");
        assert!(!app.visible("biblioteca-livros"));
        app.type_into("nota", "devolver Iracema");
        app.select("biblioteca", "Fotos");
        assert_eq!(app.text("biblioteca"), "fotos");
        assert!(app.visible("biblioteca-fotos-conteudo"));
        assert!(!app.visible("biblioteca-fotos"), "escolher fecha o painel");
        app.select("biblioteca", "Início");
        assert_eq!(app.text("nota"), "devolver Iracema");
    });
    // Na largura do `main`, o painel aberto.
    for theme in [Theme::Light, Theme::Dark] {
        let mut app = TestApp::new(navigation_view::biblioteca)
            .size(1280, 800)
            .theme(theme);
        app.assert_lint();
        assert!(app.visible("biblioteca-livros"));
        app.click("biblioteca-midia");
        app.click("biblioteca-videos");
        assert_eq!(app.text("biblioteca"), "videos");
        app.resize(800, 800);
        app.assert_lint();
    }
}

#[test]
fn title_bar() {
    for theme in [Theme::Light, Theme::Dark] {
        let mut app = TestApp::new(title_bar::biblioteca)
            .size(1280, 800)
            .theme(theme);
        app.assert_lint();
        assert!(!app.enabled("barra-de-titulo-voltar"), "no início");
        assert!(
            app.visible("barra-de-titulo-alternar"),
            "o ☰ foi para a barra"
        );
        app.click("navegacao-livros");
        app.click("barra-de-titulo-voltar");
        assert_eq!(app.text("navegacao"), "inicio");
        app.type_into("busca", "ira");
        assert!(app.is_open("busca"));
        assert!(app.window_requests().is_empty(), "a busca não arrasta");
        app.key(Key::Escape);
        // O meio da barra é a busca: o duplo clique vai no espaço livre à direita dela.
        let barra = app.rect("barra-de-titulo");
        let livre = tomni_ui::kurbo::Point::new(barra.x1 - 200.0, barra.center().y);
        app.click_at(livre, 1).click_at(livre, 2);
        assert!(app.is_maximized());
        assert_eq!(app.window_requests(), [WindowRequest::ToggleMaximize]);
        app.set_window_active(false);
        app.set_window_active(true);
        app.click("barra-de-titulo-minimizar");
        assert_eq!(app.window_requests(), [WindowRequest::Minimize]);

        let mut app = TestApp::new(title_bar::notas).size(480, 320).theme(theme);
        app.assert_lint();
        assert_eq!(app.rect("barra-de-titulo").height(), 32.0);
        app.type_into("nota", "comprar pão");
        app.click("barra-de-titulo-fechar");
        assert_eq!(app.window_requests(), [WindowRequest::Close]);
    }
}

#[test]
fn strings() {
    nos_dois_temas(strings::ingles, |app| {
        app.assert_lint();
        assert_eq!(app.label("barra-de-titulo-voltar"), "Back");
        assert_eq!(app.text("abas-adicionar"), "New tab");
        assert_eq!(app.text("quantidade-aumentar"), "Increase");
    });
}

#[test]
fn tab_view() {
    nos_dois_temas(tab_view::perfil, |app| {
        app.assert_lint();
        assert_eq!(app.text("perfil"), "dados");
        assert!(app.checked("perfil-dados"));
        assert!(app.visible("nome"));
        assert!(!app.visible("publico"));
        app.type_into("nome", " Lima");
        app.select("perfil", "Privacidade");
        assert_eq!(app.text("perfil"), "privacidade");
        assert!(!app.checked("perfil-dados"));
        assert!(app.visible("publico"));
        assert!(!app.visible("nome"));
        app.assert_lint();
        app.click("publico");
        assert!(!app.checked("publico"));
        // →, com o foco numa aba, vai à seguinte.
        app.click("perfil-privacidade").key(Key::Right);
        assert_eq!(app.text("perfil"), "sobre");
        app.click("perfil-dados");
        assert_eq!(
            app.text("nome"),
            "Ana Souza Lima",
            "a aba escondida guarda o texto"
        );
        // Ctrl+Tab de dentro do conteúdo.
        app.click("nome").key_with(Key::Tab, Modifiers::CTRL);
        assert_eq!(app.text("perfil"), "privacidade");
        assert!(!app.checked("publico"));
    });
}

#[test]
fn tab_view_atividade() {
    nos_dois_temas(tab_view::atividade, |app| {
        app.assert_lint();
        // Só as abas de trás pedem atenção: a escolhida já foi vista.
        assert_eq!(app.text("atividade-compilar"), "Compilar, trabalhando");
        assert_eq!(app.text("atividade-testes"), "Testes, 45%, pede atenção");
        assert_eq!(app.text("atividade-backup"), "Backup, pausado, 60%");
        assert_eq!(
            app.text("atividade-deploy"),
            "Deploy, erro, 90%, pede atenção"
        );
        app.click("atividade-testes");
        assert_eq!(app.text("atividade-testes"), "Testes, 45%");
    });
}

#[test]
fn tab_view_notas() {
    nos_dois_temas(tab_view::notas, |app| {
        app.assert_lint();
        assert_eq!(app.tab_titles("notas"), ["Compras", "Ideias", "Reunião"]);
        assert_eq!(app.rect("notas-1").height(), 24.0);
        assert!(!app.visible("notas-rolar-direita"), "três abas cabem");
        // Arrastar a primeira para o meio da terceira a põe antes dela.
        app.drag("notas-1", "notas-3");
        assert_eq!(app.tab_titles("notas"), ["Ideias", "Compras", "Reunião"]);
        app.drag("notas-1", "notas-2");
        assert_eq!(app.tab_titles("notas"), ["Compras", "Ideias", "Reunião"]);
        // O X da aba escolhida fecha a nota e escolhe a vizinha.
        app.close_tab("notas", "1");
        assert_eq!(app.tab_titles("notas"), ["Ideias", "Reunião"]);
        assert_eq!(app.text("notas"), "2");
        // O "+" abre uma nota e a escolhe; o clique do meio fecha.
        app.click("notas-adicionar");
        assert_eq!(app.text("notas"), "4");
        app.type_into("texto-4", "Ligar para o suporte");
        app.middle_click("notas-3");
        assert_eq!(app.tab_titles("notas"), ["Ideias", "Nota 4"]);
        assert_eq!(app.text("texto-4"), "Ligar para o suporte");
        // Com mais abas do que cabem, aparecem as setas.
        for _ in 0..4 {
            app.click("notas-adicionar");
        }
        assert!(app.visible("notas-rolar-direita"));
        app.assert_lint();
    });
}

#[test]
fn auto_suggest_box() {
    assert_eq!(auto_suggest_box::sugerir("  SÃO"), ["São Paulo"]);
    assert!(auto_suggest_box::sugerir("").is_empty());
    nos_dois_temas(auto_suggest_box::busca, |app| {
        app.assert_lint();
        assert_eq!(app.text("resultado"), "Nenhuma cidade escolhida");
        assert!(!app.is_open("cidade"));
        app.type_into("cidade", "be");
        assert!(app.is_open("cidade"));
        assert_eq!(app.label("cidade-belém"), "Belém");
        app.assert_lint();
        app.click("cidade-belém");
        assert!(!app.is_open("cidade"));
        assert_eq!(app.text("cidade"), "Belém");
        assert_eq!(app.text("resultado"), "Previsão para Belém");

        // Pelo teclado: ↓ passa pelas sugestões, escrevendo no campo, e Enter escolhe.
        app.shortcut('a').type_text("r");
        assert!(app.is_open("cidade"));
        app.key(Key::Down).key(Key::Down);
        assert_eq!(app.text("cidade"), "Rio de Janeiro");
        app.key(Key::Enter);
        assert!(!app.is_open("cidade"));
        assert_eq!(app.text("resultado"), "Previsão para Rio de Janeiro");
        app.type_text("x");
        assert!(!app.is_open("cidade"), "sem sugestão, a lista fecha");

        // A lupa busca o que está escrito, mesmo sem sugestão.
        assert_eq!(app.label("cidade-buscar"), "Buscar");
        app.click("cidade-buscar");
        assert_eq!(app.text("resultado"), "Previsão para Rio de Janeirox");
        app.assert_lint();
    });
}

#[test]
fn password_box() {
    let mascara = |n: usize| PASSWORD_CHAR.to_string().repeat(n);
    nos_dois_temas(password_box::senhas, |app| {
        app.assert_lint();
        assert!(!app.visible("senha-revelar"), "sem texto, sem olho");
        app.type_into("senha", "segredo");
        assert!(app.visible("senha-revelar"));
        assert_eq!(app.edit("senha").text, mascara(7));
        app.assert_lint();

        // Mostra só enquanto está pressionado, e o foco continua no campo.
        app.press("senha-revelar");
        assert_eq!(app.edit("senha").text, "segredo");
        assert_eq!(app.focused().as_deref(), Some("senha"));
        app.assert_lint();
        app.release();
        assert_eq!(app.edit("senha").text, mascara(7));

        // Depois de sair do campo, o olho só volta quando a senha é apagada.
        app.type_into("pin", "1234");
        assert!(!app.visible("senha-revelar"));
        assert!(!app.visible("pin-revelar"), "o PIN não tem olho");
        assert_eq!(app.edit("pin").text, mascara(4));
        app.click("mostrar-pin");
        assert_eq!(app.edit("pin").text, "1234");
        assert_eq!(app.text("tamanho"), "A senha tem 7 caracteres");
        app.assert_lint();
    });
}

#[test]
fn number_box() {
    nos_dois_temas(number_box::pedido, |app| {
        app.assert_lint();
        assert_eq!(app.value("quantidade"), 1.0);
        assert_eq!(app.text("preco"), "12,5");
        assert_eq!(app.text("desconto"), "");
        assert_eq!(app.text("total"), "Total: R$ 12,5");
        assert!(!app.is_open("quantidade"));

        app.set_value("quantidade", 3.0);
        assert_eq!(app.text("total"), "Total: R$ 37,5");
        // Com o foco, os botões de incremento ficam sobre o fim do campo.
        assert!(app.is_open("quantidade"));
        app.assert_lint();
        app.click("quantidade-aumentar");
        assert_eq!(app.value("quantidade"), 4.0);
        assert_eq!(app.focused().as_deref(), Some("quantidade"));
        app.key(Key::Down);
        assert_eq!(app.value("quantidade"), 3.0);
        app.set_value("quantidade", 500.0);
        assert_eq!(app.value("quantidade"), 99.0, "preso no máximo");

        // No preço, os botões ficam dentro do campo e repetem enquanto pressionados.
        assert!(app.visible("preco-aumentar"));
        app.press("preco-aumentar");
        assert_eq!(app.text("preco"), "13");
        app.wait(SPIN_REPEAT * 2).release();
        assert_eq!(app.text("preco"), "14");
        app.wait(SPIN_REPEAT * 3);
        assert_eq!(app.text("preco"), "14", "solto, para");

        // Um texto que não é número volta ao valor anterior.
        app.click("preco")
            .shortcut('a')
            .type_text("abc")
            .key(Key::Enter);
        assert_eq!(app.text("preco"), "14");
        app.set_value("preco", 9.99);
        assert_eq!(app.text("preco"), "9,99");

        // O desconto é opcional: vazio é nenhum.
        app.set_value("desconto", 10.0);
        assert_eq!(app.text("total"), "Total: R$ 890,11");
        assert!(app.visible("desconto-limpar"));
        app.assert_lint();
        app.click("desconto-limpar").key(Key::Enter);
        assert_eq!(app.text("desconto"), "");
        assert_eq!(app.text("total"), "Total: R$ 989,01");
    });
}

#[test]
fn icon() {
    nos_dois_temas(icon::editor, |app| {
        app.assert_lint();
        assert_eq!(app.label("desfazer"), "Desfazer");
        let arvore = app.tree();
        assert_eq!(arvore.find("desfazer").unwrap().icon, Some("ArrowUndo"));
        assert_eq!(arvore.find("salvar").unwrap().icon, Some("Save"));
        assert_eq!(arvore.find("status-icone").unwrap().icon, Some("Info"));
        assert_eq!(app.rect("desfazer").size(), (44.0, 44.0).into());
        app.click("desfazer");
        assert_eq!(app.text("status"), "Desfeito");
        app.click("compartilhar");
        assert_eq!(app.text("status"), "Link copiado");
        app.click("favorito");
        assert!(app.checked("favorito"));
        let Look::Icon(visual) = app.look("status-icone") else {
            panic!("não é ícone");
        };
        assert_eq!(visual.symbol, Symbol::StarFilled);
        app.assert_lint();
    });
}

#[test]
fn image() {
    nos_dois_temas(image::galeria, |app| {
        app.assert_lint();
        let arvore = app.tree();
        let logo = arvore.find("logo").unwrap();
        assert_eq!(logo.alt.as_deref(), Some("Logotipo do Tomni UI"));
        assert_eq!(logo.error, None);
        assert_eq!(
            app.rect("logo").size(),
            (48.0, 48.0).into(),
            "tamanho do SVG"
        );
        for (id, stretch) in [
            ("uniform", "uniform"),
            ("uniform-to-fill", "uniform_to_fill"),
            ("fill", "fill"),
            ("none", "none"),
        ] {
            let node = arvore.find(id).unwrap();
            assert_eq!(node.variant, Some(stretch));
            assert_eq!(node.alt.as_deref(), Some("Colinas verdes sob o sol"));
            assert_eq!(app.rect(id).size(), (150.0, 90.0).into());
        }
        assert_eq!(app.rect("foto").height(), 160.0);
    });
}

#[test]
fn terminal() {
    nos_dois_temas(terminal::console, |app| {
        app.assert_lint();
        let tela = app.terminal_text("console");
        assert!(tela.ends_with("demo:~$"), "prompt: {tela:?}");
        app.type_into("console", "eco olá\n");
        let tela = app.terminal_text("console");
        assert!(tela.contains("$ eco olá\nolá\n"), "{tela:?}");
        assert_eq!(app.focused().as_deref(), Some("console"));
        app.type_text("titulo Servidor\n");
        assert_eq!(app.text("titulo"), "Título: Servidor");
        let no = app.tree().find("console").cloned().unwrap();
        assert_eq!(no.kind, "terminal");
        assert_eq!(no.title.as_deref(), Some("Servidor"));
        app.type_text("cores\n");
        let linha = app
            .terminal_text("console")
            .lines()
            .position(|l| l.ends_with("$ cores"))
            .unwrap()
            + 1;
        assert_ne!(
            app.terminal_cell("console", 0, linha).bg,
            app.terminal_cell("console", 4, linha).bg,
            "as faixas de cor"
        );
        // Os dois links do `links`: o do texto abre com o Ctrl+clique; o `OSC 8` também.
        app.type_text("links\n");
        assert_eq!(app.text("link"), "Link aberto: nenhum");
        let tela = app.terminal_text("console");
        let linha = tela
            .lines()
            .position(|l| l.starts_with("Site: https://"))
            .unwrap();
        let ponto = match app.look("console") {
            Look::Custom(custom) => custom
                .downcast::<tomni_ui::terminal::TerminalVisual>()
                .unwrap()
                .cell_rect(app.rect("console"), linha, 10, 1)
                .center(),
            outro => panic!("não é o terminal: {outro:?}"),
        };
        app.ui().set_modifiers(Modifiers::CTRL);
        app.click_at(ponto, 1);
        app.ui().set_modifiers(Modifiers::NONE);
        assert_eq!(
            app.text("link"),
            "Link aberto: https://example.com/tomni-ui"
        );
    });
}

#[test]
fn terminal_panes() {
    nos_dois_temas(terminal_panes::console, |app| {
        app.assert_lint();
        let no = app.tree().find("paineis").cloned().unwrap();
        assert_eq!(no.kind, "terminal_panes");
        assert_eq!(no.layout.as_deref(), Some("[0]"));
        assert_eq!(app.text("modo-envio"), "Envio: none");
        for chave in ["web1", "web2", "web3"] {
            let tela = app.session_text(chave);
            assert!(tela.contains("Conectado"), "{chave}: {tela:?}");
        }

        // Sem envio, o texto vai só ao painel ativo.
        app.type_into("paineis", "uptime");
        assert!(app.session_text("web1").ends_with("$ uptime"));
        assert!(app.session_text("web2").ends_with("$"));

        // Com o envio para todos, as três sessões recebem.
        app.click("envio-todos");
        assert_eq!(app.text("modo-envio"), "Envio: all");
        app.type_into("paineis", "!");
        for chave in ["web1", "web2", "web3"] {
            assert!(app.session_text(chave).ends_with('!'), "{chave}");
        }

        app.click("layout-2x2");
        assert_eq!(
            app.tree().find("paineis").unwrap().layout.as_deref(),
            Some("col[row[0,1],row[2,3]]")
        );
        app.click("layout-3x3");
        assert_eq!(
            app.tree()
                .find("paineis")
                .unwrap()
                .panes
                .as_ref()
                .unwrap()
                .len(),
            9
        );
        app.click("layout-1");
        app.frame();
        app.frame();
        assert_eq!(
            app.tree().find("paineis").unwrap().layout.as_deref(),
            Some("[0]")
        );
        app.assert_lint();

        // "Identificar" liga o crachá com o número de cada painel.
        assert!(!app.panes("paineis").identifying);
        app.click("layout-2x2").click("identificar");
        app.frame();
        assert!(app.panes("paineis").identifying);
        assert!(app.is_open("paineis-painel-3-numero"));
        assert_eq!(app.text("paineis-painel-3-numero-texto"), "4");
        app.assert_lint();
        app.click("identificar");
        app.frame();
        assert!(!app.is_open("paineis-painel-3-numero"));

        // As sessões moram num registro da thread: sem isto, o segundo tema acharia as do primeiro.
        for chave in ["web1", "web2", "web3"] {
            Session::find(chave).unwrap().dispose();
        }
    });
}

#[test]
fn terminal_panes_identificando() {
    nos_dois_temas(terminal_panes::identificando, |app| {
        app.assert_lint();
        let info = app.panes("paineis");
        assert!(info.identifying);
        assert_eq!(info.highlighted, Some(2));
        assert_eq!(info.panes.len(), 4);
        for n in 0..4 {
            assert_eq!(
                app.text(&format!("paineis-painel-{n}-numero-texto")),
                (n + 1).to_string()
            );
        }
        for chave in ["web1", "web2", "web3", "web4"] {
            Session::find(chave).unwrap().dispose();
        }
    });
}

#[test]
fn bandeja() {
    nos_dois_temas(bandeja::principal, |app| {
        app.assert_lint();
        let estado = app.tray().expect("o exemplo mostra a bandeja");
        assert_eq!(estado.tooltip, "Cliques: 0");
        let ids: Vec<_> = estado.items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["somar", "", "sair"]);
        assert!(estado.items[1].separator);

        app.tray_click("somar");
        assert_eq!(app.tray().unwrap().tooltip, "Cliques: 1");
        assert_eq!(app.text("cliques"), "Cliques: 1");
        app.click("somar");
        assert_eq!(app.tray().unwrap().tooltip, "Cliques: 2");

        // Fechar a janela esconde; o clique no ícone traz de volta.
        let principal = tomni_ui::WindowHandle::main();
        assert!(!app.close_window(principal));
        assert!(app.window_hidden(principal));
        assert!(!app.exited());
        app.tray_activate();
        assert!(!app.window_hidden(principal));

        // Sair encerra de verdade.
        app.tray_click("sair");
        assert!(app.exited());
    });
}

#[test]
fn bandeja_sem_bandeja_no_sistema_fecha_de_verdade() {
    let mut app = app(bandeja::principal, Theme::Light);
    app.set_tray_available(false);
    assert!(app.tray().is_none());
    assert!(app.close_window(tomni_ui::WindowHandle::main()));
    assert!(app.exited());
}

#[test]
fn janelas() {
    nos_dois_temas(janelas::principal, |app| {
        app.assert_lint();
        assert_eq!(app.windows().len(), 1);
        assert_eq!(app.text("contador"), "Contador: 0");

        // O botão abre outra janela, com a tela secundária e o mesmo contador.
        app.click("nova-janela");
        app.frame();
        let janelas = app.windows();
        assert_eq!(janelas.len(), 2);
        let principal = app.current_window();
        app.switch_to(janelas[1]);
        app.assert_lint();
        assert_eq!(app.text("contador"), "Contador: 0");
        app.click("contar");
        assert_eq!(app.text("contador"), "Contador: 1");
        app.switch_to(principal);
        assert_eq!(app.text("contador"), "Contador: 1");

        // Fechar as janelas descarta as sessões que elas tinham.
        assert!(app.close_window(janelas[1]));
        assert_eq!(app.windows().len(), 1);
        for chave in ["web1", "web2", "web3"] {
            if let Some(sessao) = Session::find(chave) {
                sessao.dispose();
            }
        }
    });
    nos_dois_temas(janelas::janela_secundaria, |app| {
        app.assert_lint();
        assert!(app.session_text("extra").contains("Conectado"));
        Session::find("extra").unwrap().dispose();
    });
}

#[test]
fn webview() {
    nos_dois_temas(webview::navegador, |app| {
        app.assert_lint();
        let no = app.tree().find("pagina").cloned().unwrap();
        assert_eq!(no.kind, "webview");
        assert_eq!(no.state, Some("ready"));
        assert_eq!(app.text("titulo"), "Início");
        assert!(!app.enabled("voltar"));
        app.webview_message("pagina", "olá, app");
        assert_eq!(app.text("recebida"), "A página disse: olá, app");
        app.click("mandar");
        app.frame();
        let enviadas = app.webview_posted("pagina");
        assert_eq!(enviadas.len(), 1);
        assert!(enviadas[0].ends_with(" UTC"), "{enviadas:?}");
        assert!(!app.webview_follow("pagina", "https://www.rust-lang.org/"));
        assert_eq!(app.webview_opened("pagina"), ["https://www.rust-lang.org/"]);
        assert!(app.webview_follow("pagina", "fluent://app/sobre.html"));
        app.frame();
        assert_eq!(app.text("titulo"), "Sobre");
        assert!(app.enabled("voltar"));
    });
}

#[test]
fn split() {
    for theme in [Theme::Light, Theme::Dark] {
        let mut app = TestApp::new(split::servidores).size(900, 600).theme(theme);
        app.assert_lint();
        assert_eq!(app.value("lados"), 220.0);
        assert_eq!(app.rect("lados-primeiro").width(), 220.0);
        assert_eq!(app.rect("sessao-primeiro").height(), 300.0);
        // As setas, com o foco no divisor, movem de 8 em 8 px.
        app.click("lados-divisor").key(Key::Right).key(Key::Right);
        assert_eq!(app.value("lados"), 236.0);
        // O arrasto respeita o mínimo do primeiro painel.
        let divisor = app.rect("lados-divisor").center();
        app.drag(divisor, divisor - (200.0, 0.0));
        assert_eq!(app.value("lados"), 160.0);
        app.click("sessao-divisor").key(Key::Up);
        assert_eq!(app.value("sessao"), 292.0);
        app.assert_lint();
    }
}

#[test]
fn dock() {
    for theme in [Theme::Light, Theme::Dark] {
        let mut app = TestApp::new(dock::editor).size(960, 600).theme(theme);
        app.assert_lint();
        // A árvore e as propriedades começam fixadas, à esquerda do conteúdo.
        assert!(app.rect("dock-arvore").x1 <= app.rect("dock-conteudo").x0);
        assert!(app.visible("dock-propriedades"));
        // A ajuda e a saída começam recolhidas, com um botão na faixa.
        assert!(app.ui().find("dock-ajuda-faixa").is_some());
        assert!(app.ui().find("dock-saida-faixa").is_some());
        // Escolher um nó da árvore muda o conteúdo.
        app.click("no-db-01");
        assert_eq!(app.text("estado"), "Selecionado: db-01");
        // O alfinete solta o painel para a faixa; o botão da faixa o abre por cima.
        app.click("dock-arvore-alfinete");
        assert!(app.ui().find("dock-arvore-faixa").is_some());
        app.click("dock-arvore-faixa");
        assert!(app.visible("dock-arvore"));
        app.assert_lint();
    }
}

#[test]
fn data_grid() {
    for theme in [Theme::Light, Theme::Dark] {
        let mut app = TestApp::new(data_grid::arquivos)
            .size(620, 640)
            .theme(theme);
        app.assert_lint();
        let grade = app.grid("arquivos");
        assert_eq!(grade.rows.len(), 200);
        assert_eq!(grade.keys()[..3], ["..", "backups", "config"]);
        assert_eq!(grade.columns.len(), 4);
        // Os três primeiros arquivos têm tom; o resto, não.
        let tons: Vec<_> = grade
            .rows
            .iter()
            .filter_map(|l| l.tone.as_deref())
            .collect();
        assert_eq!(tons, ["caution", "success", "critical"]);
        assert_eq!(
            grade.sort,
            Some(("nome".to_owned(), tomni_ui::SortDirection::Ascending))
        );
        // Um clique seleciona; o duplo abre; Delete apaga.
        app.click("arquivos-docs").double_click("arquivos-docs");
        assert_eq!(app.text("status"), "Abrindo a pasta docs");
        app.click("arquivos-arquivo1.log")
            .grid_select("arquivos", &["arquivo1.log", "arquivo2.conf"]);
        app.key(Key::Delete);
        assert_eq!(app.text("status"), "2 apagado(s)");
        assert_eq!(app.grid("arquivos").rows.len(), 198);
        // O filtro e a lista vazia.
        app.type_into("filtro", "zzz");
        assert!(app.grid("arquivos").rows.is_empty());
        app.assert_lint();
    }
}

#[test]
fn tree_view() {
    for theme in [Theme::Light, Theme::Dark] {
        let mut app = TestApp::new(tree_view::servidores)
            .size(420, 640)
            .theme(theme);
        app.assert_lint();
        let linhas: Vec<_> = app
            .tree_items("servidores")
            .into_iter()
            .map(|l| l.key)
            .collect();
        assert_eq!(
            linhas,
            [
                "producao",
                "web-01",
                "web-02",
                "db-01",
                "homologacao",
                "backup",
                "bastion"
            ]
        );
        // O clique seleciona, o Enter chama `on_invoke`.
        app.click("servidores-web-02");
        assert_eq!(app.text("servidores"), "web-02");
        app.key(Key::Enter);
        assert_eq!(app.text("status"), "Abrindo web-02");
        // Um arquivo solto do grid cai numa pasta da árvore.
        app.drag_rows("soltos", &["ata.pdf"], "servidores", Some("backup"));
        assert_eq!(app.text("status"), "ata.pdf em backup");
        assert_eq!(app.grid("soltos").rows.len(), 1);
        app.assert_lint();
    }
}

#[test]
fn property_grid() {
    for theme in [Theme::Light, Theme::Dark] {
        let mut app = TestApp::new(property_grid::conexoes)
            .size(720, 560)
            .theme(theme);
        app.assert_lint();
        let propriedades = app.properties("props");
        assert_eq!(propriedades[0].name, "Host");
        assert_eq!(propriedades[0].value, "web-01.exemplo.com");
        // A lista troca os signals: a mesma grade mostra a outra conexão.
        app.select("conexoes", "Legado");
        let propriedades = app.properties("props");
        assert_eq!(propriedades[0].value, "10.0.0.7");
        assert_eq!(propriedades[1].value, "23");
        assert!(propriedades[1].changed, "23 difere do padrão 22");
        // Escolher a linha da chave e usar o "…" grava o caminho respondido.
        let r = app.rect("props-Chave privada");
        app.click_at(tomni_ui::kurbo::Point::new(r.x0 + 30.0, r.center().y), 1);
        app.click("props-Chave privada-procurar");
        app.file_dialog_answer(Some("/tmp/legado.pem"));
        let chave = app
            .properties("props")
            .into_iter()
            .find(|p| p.name == "Chave privada")
            .unwrap();
        assert_eq!(chave.value, "/tmp/legado.pem");
        // A `.folder` pede uma pasta ao seletor.
        let r = app.rect("props-Pasta de trabalho");
        app.click_at(tomni_ui::kurbo::Point::new(r.x0 + 30.0, r.center().y), 1);
        app.click("props-Pasta de trabalho-procurar");
        assert_eq!(
            app.file_dialogs().last().map(|p| p.kind),
            Some(tomni_ui::dialogs::FileDialogKind::Folder)
        );
        app.file_dialog_answer(Some("/srv/legado"));
        let pasta = app
            .properties("props")
            .into_iter()
            .find(|p| p.name == "Pasta de trabalho")
            .unwrap();
        assert_eq!(pasta.value, "/srv/legado");
        app.assert_lint();
    }
}

#[test]
fn menu_bar() {
    for theme in [Theme::Light, Theme::Dark] {
        let mut app = TestApp::new(menu_bar::logs).size(800, 500).theme(theme);
        app.assert_lint();
        assert_eq!(app.text("status"), "Pronto, com quebra");
        // A barra abre o menu e o item roda a ação.
        app.click("barra-arquivo");
        assert!(app.is_open("arquivo"));
        app.click("abrir");
        assert!(!app.is_open("arquivo"));
        assert!(app.visible("log-3-log"));
        // Os atalhos dos itens valem com o menu fechado.
        app.shortcut('O');
        assert!(app.visible("log-4-log"));
        app.shortcut(Shortcut::ctrl_shift('W'));
        assert_eq!(app.text("status"), "Pronto, sem quebra");
        // O F5 do botão.
        app.shortcut(Shortcut::key(Key::F(5)));
        assert_eq!(app.text("status"), "Recarregado, sem quebra");
        // O menu de contexto da lista abre sobre um arquivo.
        app.context_menu("erro-log");
        assert!(app.is_open("lista-menu"));
        app.click("copiar");
        assert_eq!(app.text("status"), "Caminho copiado, sem quebra");
        app.context_menu("app-log").click("limpar");
        assert!(app.tree().find("app-log").is_none(), "a lista ficou vazia");
        app.assert_lint();
    }
}
