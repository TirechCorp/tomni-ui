//! M35: o `SettingsCard` e o `SettingsExpander` (as linhas de uma página de configurações), o
//! tema trocado com o app aberto e o resto da lib que a página de Configurações do Tirech Omni
//! usa.

#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/settings.rs"]
mod settings;

use tomni_ui::theme::Theme;
use tomni_ui::{Key, Node};
use tomni_ui_core::{Look, SettingsCardPart};
use tomni_ui_test::TestApp;

fn app(largura: u32, tema: Theme) -> TestApp {
    TestApp::new(settings::tela).size(largura, 700).theme(tema)
}

fn nos_dois_temas(largura: u32, check: impl Fn(&mut TestApp)) {
    for tema in [Theme::Light, Theme::Dark] {
        check(&mut app(largura, tema));
    }
}

/// Quantos nós de `kind` aparecem dentro de `node`, ele incluído.
fn contar(node: &Node, kind: &str) -> usize {
    if !node.visible {
        return 0;
    }
    usize::from(node.kind == kind) + node.children.iter().map(|c| contar(c, kind)).sum::<usize>()
}

#[test]
fn controle_a_direita_em_card_largo() {
    nos_dois_temas(600, |app| {
        let card = app.rect("idioma");
        let lista = app.rect("idioma-lista");
        assert!(card.width() >= 476.0, "o card tem {}", card.width());
        assert!(
            lista.x0 > card.x0 + card.width() / 2.0,
            "o controle fica na metade da direita"
        );
        let meio_card = (card.y0 + card.y1) / 2.0;
        let meio_lista = (lista.y0 + lista.y1) / 2.0;
        assert!(
            (meio_card - meio_lista).abs() <= 1.0,
            "o controle é centrado na vertical"
        );
        assert!(
            (card.height() - 78.0).abs() < 1.0,
            "34 de padding e borda mais o controle de 44"
        );
    });
}

#[test]
fn controle_desce_em_card_estreito() {
    nos_dois_temas(400, |app| {
        let card = app.rect("idioma");
        let lista = app.rect("idioma-lista");
        assert!(card.width() < 476.0, "o card tem {}", card.width());
        assert!(
            lista.x0 < card.x0 + 100.0,
            "o controle fica à esquerda, embaixo do texto"
        );
        assert!(
            lista.y0 > card.y0 + 16.0 + 20.0 + 8.0,
            "embaixo do cabeçalho e da descrição: {lista:?} em {card:?}"
        );
        assert!(lista.y1 <= card.y1 - 16.0, "dentro do padding de baixo");
        assert!(card.height() > 100.0, "o card cresceu: {}", card.height());
    });
}

#[test]
fn icone_some_em_card_muito_estreito() {
    nos_dois_temas(600, |app| {
        assert_eq!(contar(app.tree().find("idioma").unwrap(), "icon"), 1);
    });
    nos_dois_temas(330, |app| {
        let card = app.rect("idioma");
        assert!(card.width() < 286.0, "o card tem {}", card.width());
        assert_eq!(
            contar(app.tree().find("idioma").unwrap(), "icon"),
            0,
            "sem o ícone"
        );
    });
}

/// Tab até `id` ter o foco.
fn focar(app: &mut TestApp, id: &str) {
    for _ in 0..40 {
        if app.focused().as_deref() == Some(id) {
            return;
        }
        app.tab();
    }
    panic!("o foco não chegou em {id}");
}

#[test]
fn card_clicavel_responde_a_clique_enter_e_espaco() {
    nos_dois_temas(600, |app| {
        assert_eq!(app.text("ultimo"), "Nada aberto");
        app.click("sobre");
        assert_eq!(app.text("ultimo"), "Aberto: Sobre");
        app.click("pasta");
        assert_eq!(app.text("ultimo"), "Aberto: Pasta");
        focar(app, "sobre");
        app.key(Key::Enter);
        assert_eq!(app.text("ultimo"), "Aberto: Sobre");
        focar(app, "pasta");
        app.key(Key::Space);
        assert_eq!(app.text("ultimo"), "Aberto: Pasta");
    });
}

#[test]
fn card_clicavel_tem_hover_e_o_sem_clique_nao() {
    nos_dois_temas(600, |app| {
        let estado = |app: &TestApp, id: &str| app.tree().find(id).unwrap().state;
        assert_eq!(estado(app, "sobre"), Some("rest"));
        app.hover("sobre");
        assert_eq!(estado(app, "sobre"), Some("hover"));
        app.hover("notificacoes");
        assert_eq!(
            estado(app, "notificacoes"),
            None,
            "só o clicável tem estado"
        );
        assert_eq!(estado(app, "sobre"), Some("rest"));
        let Look::SettingsCard(visual) = app.look("notificacoes") else {
            panic!("não é um SettingsCard");
        };
        assert!(!visual.clickable);
        assert_eq!(visual.part, SettingsCardPart::Alone);
    });
}

#[test]
fn card_desabilitado_nao_responde_nem_recebe_foco() {
    nos_dois_temas(600, |app| {
        assert!(!app.enabled("conta"));
        app.click("conta");
        assert_eq!(app.text("ultimo"), "Nada aberto");
        for _ in 0..20 {
            app.tab();
            assert_ne!(app.focused().as_deref(), Some("conta"));
        }
        let Look::SettingsCard(visual) = app.look("conta") else {
            panic!("não é um SettingsCard");
        };
        assert_eq!(visual.state, tomni_ui_core::ButtonState::Disabled);
    });
}

#[test]
fn clicar_no_cabecalho_do_expander_abre_e_fecha() {
    nos_dois_temas(600, |app| {
        assert!(!app.visible("aparencia-conteudo"));
        app.click("aparencia-cabecalho");
        assert!(app.visible("aparencia-conteudo"));
        app.click("aparencia-cabecalho");
        assert!(!app.visible("aparencia-conteudo"));
        // O teclado também.
        focar(app, "aparencia-cabecalho");
        app.key(Key::Enter);
        assert!(app.visible("aparencia-conteudo"));
    });
}

#[test]
fn o_controle_do_cabecalho_nao_abre_o_expander() {
    nos_dois_temas(600, |app| {
        assert!(app.checked("aparencia-chave"));
        app.click("aparencia-chave");
        assert!(!app.checked("aparencia-chave"), "a chave virou");
        assert!(!app.visible("aparencia-conteudo"), "e o expander não abriu");
    });
}

#[test]
fn filhos_do_expander_sao_filhos_sem_raio_e_com_separador() {
    nos_dois_temas(600, |app| {
        app.click("aparencia-cabecalho");
        let Look::SettingsCard(cabecalho) = app.look("aparencia-cabecalho") else {
            panic!("não é um SettingsCard");
        };
        assert_eq!(
            cabecalho.part,
            SettingsCardPart::ExpanderHeader { open: true }
        );
        let Look::SettingsCard(primeiro) = app.look("fonte") else {
            panic!("não é um SettingsCard");
        };
        assert_eq!(
            primeiro.part,
            SettingsCardPart::ExpanderItem { last: false }
        );
        let Look::SettingsCard(ultimo) = app.look("cursor") else {
            panic!("não é um SettingsCard");
        };
        assert_eq!(ultimo.part, SettingsCardPart::ExpanderItem { last: true });
        // Os filhos encostam uns nos outros e no cabeçalho, sem os 4 px dos cards soltos.
        let (a, b) = (app.rect("aparencia-cabecalho"), app.rect("fonte"));
        assert!((b.y0 - a.y1).abs() < 1.0, "{a:?} {b:?}");
        let (b, c) = (app.rect("fonte"), app.rect("esquema"));
        assert!((c.y0 - b.y1).abs() < 1.0, "{b:?} {c:?}");
        assert!(
            b.height() >= 52.0 - 0.5,
            "altura mínima do filho: {}",
            b.height()
        );
        // O texto do filho alinha com o do pai: 58 px da borda do filho.
        assert!((b.x0 - a.x0).abs() < 0.5, "mesma largura que o cabeçalho");
    });
}

#[test]
fn tab_percorre_os_controles_na_ordem() {
    nos_dois_temas(600, |app| {
        app.click("aparencia-cabecalho");
        let mut ordem = Vec::new();
        for _ in 0..12 {
            app.tab();
            if let Some(id) = app.focused()
                && ordem.last() != Some(&id)
            {
                ordem.push(id);
            }
        }
        let posicao = |id: &str| ordem.iter().position(|x| x == id).expect(id);
        assert!(posicao("notificacoes-chave") < posicao("idioma-lista"));
        assert!(posicao("idioma-lista") < posicao("sobre"));
        assert!(posicao("pasta") < posicao("aparencia-cabecalho"));
        assert!(posicao("aparencia-chave") < posicao("cursor"));
    });
}

#[test]
fn snapshots_da_pagina() {
    for (largura, nome) in [(800, "800"), (400, "400")] {
        for (tema, tema_nome) in [(Theme::Light, "claro"), (Theme::Dark, "escuro")] {
            let app = app(largura, tema);
            app.assert_snapshot(&format!("settings_{nome}_{tema_nome}"));
        }
    }
    for (tema, tema_nome) in [(Theme::Light, "claro"), (Theme::Dark, "escuro")] {
        let mut app = app(800, tema);
        app.click("aparencia-cabecalho");
        app.assert_snapshot(&format!("settings_aberto_{tema_nome}"));
    }
}

// ---------------------------------------------------------------------------------------------
// `set_theme`
// ---------------------------------------------------------------------------------------------

/// O pixel do canto de cima e à esquerda: o fundo da janela, que muda com o tema.
fn fundo(app: &TestApp) -> [u8; 4] {
    let pixel = app.snapshot().sample(0, 0);
    [pixel.r, pixel.g, pixel.b, pixel.a]
}

fn tela_do_tema() -> impl tomni_ui::View {
    use tomni_ui::prelude::*;
    Column::new()
        .child(
            Button::new("Escuro")
                .id("escuro")
                .on_click(|| tomni_ui::set_theme(Some(Theme::Dark))),
        )
        .child(
            Button::new("Sistema")
                .id("sistema")
                .on_click(|| tomni_ui::set_theme(None)),
        )
}

#[test]
fn set_theme_troca_o_tema_na_hora_e_none_volta_ao_do_sistema() {
    let mut app = TestApp::new(tela_do_tema).size(300, 200);
    assert_eq!(app.current_theme(), Theme::Light);
    let claro = fundo(&app);
    app.click("escuro");
    assert_eq!(app.current_theme(), Theme::Dark);
    assert_ne!(fundo(&app), claro, "o fundo mudou para o escuro");
    app.click("sistema");
    assert_eq!(
        app.current_theme(),
        Theme::Light,
        "no teste o sistema é claro"
    );
    assert_eq!(fundo(&app), claro);
}

#[test]
fn set_theme_do_test_app_vale_como_o_do_app() {
    let mut app = TestApp::new(tela_do_tema).size(300, 200);
    app.set_theme(Some(Theme::Dark));
    assert_eq!(app.current_theme(), Theme::Dark);
    app.set_theme(Some(Theme::Light));
    assert_eq!(app.current_theme(), Theme::Light);
}

// ---------------------------------------------------------------------------------------------
// `Terminal` com propriedades reativas, `right_click` e `bell`
// ---------------------------------------------------------------------------------------------

mod terminal {
    use tomni_ui::prelude::*;
    use tomni_ui::terminal::{
        BELL_FLASH, BellStyle, CursorShape, LoopbackHandle, RightClick, Scheme, TerminalVisual,
    };
    use tomni_ui::theme::Theme;
    use tomni_ui_core::{Key, Look, Modifiers};
    use tomni_ui_test::TestApp;

    /// Um terminal que acompanha o tamanho do widget, com `configurar` aplicado a ele.
    fn app(configurar: impl FnOnce(Terminal) -> Terminal + 'static) -> (TestApp, LoopbackHandle) {
        let conexao = Loopback::new();
        let handle = conexao.handle();
        let app = TestApp::new(move || {
            Form::new("Console").child(configurar(Terminal::new(conexao).id("console").grow(1)))
        })
        .size(780, 560)
        .theme(Theme::Light);
        (app, handle)
    }

    fn visual(app: &TestApp) -> TerminalVisual {
        match app.look("console") {
            Look::Custom(custom) => custom
                .downcast::<TerminalVisual>()
                .expect("o visual do terminal")
                .clone(),
            outro => panic!("não é o terminal: {outro:?}"),
        }
    }

    fn escrito(handle: &LoopbackHandle) -> String {
        String::from_utf8(handle.take_written()).unwrap()
    }

    #[test]
    fn font_size_com_signal_remede_a_grade_e_avisa_a_sessao() {
        let tamanho = signal(16.0);
        let (mut app, handle) = app(move |t| t.font_size(tamanho));
        let antes = handle.size().expect("a sessão recebeu o tamanho");
        tamanho.set(32.0);
        app.frame();
        let depois = handle.size().expect("a sessão recebeu o tamanho novo");
        assert!(
            depois.0 < antes.0 && depois.1 < antes.1,
            "a fonte maior cabe em menos colunas e linhas: {antes:?} → {depois:?}"
        );
        tamanho.set(16.0);
        app.frame();
        assert_eq!(handle.size(), Some(antes), "voltar à fonte volta à grade");
    }

    #[test]
    fn font_size_de_valor_fixo_continua_valendo() {
        let (app, handle) = app(|t| t.font_size(24.0));
        let (cols, _) = handle.size().unwrap();
        drop(app);
        assert!(
            cols < 80,
            "a fonte de 24 px deixa menos de 80 colunas: {cols}"
        );
    }

    #[test]
    fn scheme_com_signal_troca_as_cores() {
        let esquema = signal(Scheme::Campbell);
        let (mut app, _) = app(move |t| t.scheme(esquema));
        assert_eq!(visual(&app).scheme(), Scheme::Campbell);
        esquema.set(Scheme::Theme);
        app.frame();
        assert_eq!(visual(&app).scheme(), Scheme::Theme);
    }

    #[test]
    fn cursor_shape_com_signal_muda_o_cursor_mas_nao_depois_de_um_decscusr() {
        let forma = signal(CursorShape::Bar);
        let (mut app, _) = app(move |t| t.cursor_shape(forma));
        let atual = |app: &TestApp| visual(app).cursor().map(|(_, _, forma)| forma);
        assert_eq!(atual(&app), Some(CursorShape::Bar));
        forma.set(CursorShape::Block);
        app.frame();
        assert_eq!(atual(&app), Some(CursorShape::Block));
        // O programa pede o sublinhado (`ESC[4 q`): a configuração não passa por cima.
        app.feed("console", "\x1b[4 q");
        assert_eq!(atual(&app), Some(CursorShape::Underline));
        forma.set(CursorShape::Bar);
        app.frame();
        assert_eq!(atual(&app), Some(CursorShape::Underline));
        // O `ESC[0 q` volta ao padrão de agora.
        app.feed("console", "\x1b[0 q");
        assert_eq!(atual(&app), Some(CursorShape::Bar));
    }

    #[test]
    fn right_click_copy_paste_copia_com_selecao_e_cola_sem() {
        let acao = signal(RightClick::CopyPaste);
        let (mut app, handle) = app(move |t| {
            t.right_click(acao)
                .paste_warning(false)
                .context_flyout(MenuFlyout::new().id("menu").item("colar", "Colar", || {}))
        });
        app.feed("console", "usuario@servidor:~$ ");
        let origem = visual(&app)
            .cell_rect(app.rect("console"), 0, 0, 1)
            .center();
        let fim = visual(&app)
            .cell_rect(app.rect("console"), 0, 6, 1)
            .center();
        app.drag(origem, fim);
        app.right_click("console");
        assert_eq!(app.clipboard().read().as_deref(), Some("usuario"));
        assert!(!app.is_open("menu"), "CopyPaste não abre o menu");
        assert_eq!(escrito(&handle), "", "com seleção só copia");
        app.right_click("console");
        assert_eq!(escrito(&handle), "usuario", "sem seleção cola");
        // O Ctrl+clique direito abre o menu, como no PuTTY, e não cola.
        app.right_click_with("console", Modifiers::CTRL);
        assert!(app.is_open("menu"), "Ctrl+clique direito abre o menu");
        assert_eq!(escrito(&handle), "", "o Ctrl+clique direito não cola");
        app.key(Key::Escape);
        assert!(!app.is_open("menu"));
        acao.set(RightClick::Menu);
        app.right_click("console");
        assert!(app.is_open("menu"), "Menu abre o menu de contexto");
    }

    #[test]
    fn on_ctrl_secondary_fica_com_o_ctrl_clique_direito_quando_o_app_quer() {
        let quer = signal(true);
        let chamadas = std::rc::Rc::new(std::cell::Cell::new(0));
        let contar = std::rc::Rc::clone(&chamadas);
        let (mut app, handle) = app(move |t| {
            t.right_click(RightClick::CopyPaste)
                .paste_warning(false)
                .context_flyout(MenuFlyout::new().id("menu").item("colar", "Colar", || {}))
                .on_ctrl_secondary(quer, move || contar.set(contar.get() + 1))
        });
        app.feed("console", "arquivo.txt");
        app.right_click_with("console", Modifiers::CTRL);
        assert_eq!(chamadas.get(), 1, "o app recebeu o Ctrl+clique direito");
        assert!(!app.is_open("menu"), "o menu não abre");
        assert_eq!(escrito(&handle), "", "não cola");
        // Sem Ctrl, o clique direito segue o `RightClick`.
        app.right_click("console");
        assert_eq!(chamadas.get(), 1);
        // Com `when` falso, o Ctrl+clique direito volta a abrir o menu.
        quer.set(false);
        app.right_click_with("console", Modifiers::CTRL);
        assert_eq!(chamadas.get(), 1);
        assert!(app.is_open("menu"));
    }

    #[test]
    fn bell_none_nao_pisca() {
        let estilo = signal(BellStyle::Flash);
        let (mut app, handle) = app(move |t| t.bell(estilo));
        handle.feed("\x07");
        app.frame();
        assert!(visual(&app).flash(), "o padrão pisca");
        app.wait(BELL_FLASH);
        assert!(!visual(&app).flash());
        estilo.set(BellStyle::None);
        handle.feed("\x07");
        app.frame();
        assert!(!visual(&app).flash(), "BellStyle::None não pisca");
    }
}

// ---------------------------------------------------------------------------------------------
// `Shortcut` lido de texto
// ---------------------------------------------------------------------------------------------

mod atalho {
    use tomni_ui::prelude::*;
    use tomni_ui_core::{Key, Modifiers};

    fn ler(texto: &str) -> Shortcut {
        texto.parse().unwrap_or_else(|e| panic!("{texto}: {e}"))
    }

    #[test]
    fn le_o_texto_que_o_display_escreve() {
        assert_eq!(ler("Ctrl+Shift+T"), Shortcut::ctrl_shift('t'));
        assert_eq!(
            ler("Ctrl+Alt+1"),
            Shortcut::new('1', Modifiers::CTRL | Modifiers::ALT)
        );
        assert_eq!(
            ler("Alt+Shift+-"),
            Shortcut::new('-', Modifiers::ALT | Modifiers::SHIFT)
        );
        assert_eq!(ler("F5"), Shortcut::key(Key::F(5)));
        assert_eq!(ler("Ctrl+PageUp"), Shortcut::ctrl(Key::PageUp));
        assert_eq!(ler("Ctrl++"), Shortcut::ctrl('+'));
    }

    #[test]
    fn maiusculas_e_ordem_nao_importam_e_control_e_ctrl() {
        assert_eq!(ler("control+SHIFT+t"), ler("Shift+Ctrl+T"));
        assert_eq!(ler("ctrl+pgdn"), ler("Ctrl+PageDown"));
        assert_eq!(ler("esc"), Shortcut::key(Key::Escape));
    }

    #[test]
    fn erro_em_portugues_com_o_texto_digitado() {
        for texto in ["Ctrl+Xyz", "Foo+T", "F99", "", "Ctrl+"] {
            let erro = texto.parse::<Shortcut>().unwrap_err().to_string();
            assert_eq!(erro, format!("tecla desconhecida: {texto}"));
        }
    }

    #[test]
    fn ida_e_volta_para_todas_as_teclas() {
        let mut teclas = vec![
            Key::Enter,
            Key::Space,
            Key::Tab,
            Key::Escape,
            Key::Backspace,
            Key::Delete,
            Key::Left,
            Key::Right,
            Key::Up,
            Key::Down,
            Key::Home,
            Key::End,
            Key::PageUp,
            Key::PageDown,
            Key::Insert,
            Key::ContextMenu,
        ];
        teclas.extend((1..=24).map(Key::F));
        teclas.extend(
            ('a'..='z')
                .chain('0'..='9')
                .chain("-=[];',./`\\+".chars())
                .map(Key::Char),
        );
        for tecla in teclas {
            for mods in [
                Modifiers::NONE,
                Modifiers::CTRL,
                Modifiers::CTRL | Modifiers::SHIFT,
                Modifiers::CTRL | Modifiers::ALT | Modifiers::SHIFT,
                Modifiers {
                    meta: true,
                    ..Modifiers::ALT
                },
            ] {
                let atalho = Shortcut::new(tecla, mods);
                assert_eq!(ler(&atalho.to_string()), atalho, "{atalho}");
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Atalhos que mudam sem montar a tela de novo
// ---------------------------------------------------------------------------------------------

mod atalhos_reativos {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use tomni_ui::prelude::*;
    use tomni_ui::terminal::{Loopback, PaneLayout, PaneShortcuts, Session, TerminalPanes};
    use tomni_ui::theme::Theme;
    use tomni_ui_core::{Key, Look, Modifiers};
    use tomni_ui_test::TestApp;

    #[test]
    fn item_de_menu_com_signal_troca_a_tecla_e_o_texto() {
        let guarda = Rc::new(Cell::new(None));
        let log = Rc::new(RefCell::new(Vec::new()));
        let (g, l) = (Rc::clone(&guarda), Rc::clone(&log));
        let mut app = TestApp::new(move || {
            let atalho = signal(Shortcut::ctrl('N'));
            g.set(Some(atalho));
            let l = Rc::clone(&l);
            Form::new("Menu").child(
                MenuBar::new().id("barra").menu(
                    "Arquivo",
                    MenuFlyout::new()
                        .id("arquivo")
                        .item("novo", "Novo", move || l.borrow_mut().push("novo"))
                        .shortcut(atalho),
                ),
            )
        })
        .theme(Theme::Light)
        .size(600, 300);
        let atalho: Signal<Shortcut> = guarda.get().unwrap();
        app.shortcut(Shortcut::ctrl('N'));
        assert_eq!(*log.borrow(), ["novo"]);
        assert_eq!(app.tree().find("novo").unwrap().accelerators, ["Ctrl+N"]);

        atalho.set(Shortcut::ctrl_shift('L'));
        app.frame();
        app.shortcut(Shortcut::ctrl('N'));
        assert_eq!(*log.borrow(), ["novo"], "a tecla antiga não dispara mais");
        app.shortcut(Shortcut::ctrl_shift('L'));
        assert_eq!(*log.borrow(), ["novo", "novo"], "a nova dispara");
        assert_eq!(
            app.tree().find("novo").unwrap().accelerators,
            ["Ctrl+Shift+L"]
        );
        // O texto à direita do item acompanha, com o menu aberto.
        app.click("barra-arquivo");
        match app.look("novo") {
            Look::MenuItem { visual, .. } => {
                assert_eq!(visual.shortcut, Some(atalho.get_untracked()))
            }
            outro => panic!("não é um item de menu: {outro:?}"),
        }
    }

    #[test]
    fn paineis_com_signal_trocam_os_atalhos_na_hora() {
        let guarda = Rc::new(Cell::new(None));
        let layout_guarda = Rc::new(Cell::new(None));
        let sessao = Session::new(Loopback::new().echo())
            .key("web-01")
            .name("web-01");
        let (g, lg) = (Rc::clone(&guarda), Rc::clone(&layout_guarda));
        let mut app = TestApp::new(move || {
            let atalhos = signal(PaneShortcuts::none());
            let layout = signal(PaneLayout::with(vec![sessao]));
            g.set(Some(atalhos));
            lg.set(Some(layout));
            Form::new("Painéis").child(
                TerminalPanes::bind(layout)
                    .id("paineis")
                    .shortcuts(atalhos)
                    .grow(1),
            )
        })
        .theme(Theme::Light)
        .size(900, 600);
        let atalhos: Signal<PaneShortcuts> = guarda.get().unwrap();
        let layout: Signal<PaneLayout> = layout_guarda.get().unwrap();
        let ctrl_shift = Modifiers::CTRL | Modifiers::SHIFT;
        app.click("paineis-terminal-web-01");
        app.key_with(Key::Char('z'), ctrl_shift);
        assert_eq!(layout.get_untracked().zoomed(), None, "sem atalhos");

        atalhos.set(PaneShortcuts {
            zoom: vec![Shortcut::ctrl_shift('z')],
            ..PaneShortcuts::none()
        });
        app.key_with(Key::Char('z'), ctrl_shift);
        assert_eq!(layout.get_untracked().zoomed(), Some(0));

        atalhos.set(PaneShortcuts::none());
        app.key_with(Key::Char('z'), ctrl_shift);
        assert_eq!(
            layout.get_untracked().zoomed(),
            Some(0),
            "desligado de novo"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Área de transferência e seletor de pasta
// ---------------------------------------------------------------------------------------------

mod ambiente {
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::rc::Rc;

    use tomni_ui::prelude::*;
    use tomni_ui::{clipboard, dialogs};
    use tomni_ui_test::TestApp;

    fn vazio() -> TestApp {
        TestApp::new(|| Form::new("Vazio").child(Text::new("x"))).size(300, 200)
    }

    #[test]
    fn clear_if_apaga_so_o_que_o_app_copiou() {
        let mut app = vazio();
        clipboard::write("senha");
        app.frame();
        clipboard::clear_if("senha");
        app.frame();
        assert_eq!(app.clipboard().read().as_deref(), Some(""));

        clipboard::write("senha");
        app.frame();
        app.clipboard().write("outra coisa".into());
        clipboard::clear_if("senha");
        app.frame();
        assert_eq!(app.clipboard().read().as_deref(), Some("outra coisa"));
    }

    #[test]
    fn read_devolve_o_que_esta_na_area_de_transferencia() {
        let mut app = vazio();
        app.clipboard().write("texto".into());
        let lido = Rc::new(RefCell::new(None));
        let l = Rc::clone(&lido);
        clipboard::read(move |t| *l.borrow_mut() = Some(t));
        assert!(lido.borrow().is_none(), "nunca na hora");
        app.frame();
        assert_eq!(*lido.borrow(), Some(Some("texto".to_string())));
    }

    #[test]
    fn pick_folder_entrega_a_pasta_escolhida() {
        let mut app = vazio();
        let escolhida = Rc::new(RefCell::new(None));
        let e = Rc::clone(&escolhida);
        dialogs::pick_folder()
            .title("Pasta de dados")
            .show(move |p| *e.borrow_mut() = Some(p));
        app.file_dialog_answer(Some("/tmp/dados"));
        app.frame();
        assert_eq!(*escolhida.borrow(), Some(Some(PathBuf::from("/tmp/dados"))));
    }
}
