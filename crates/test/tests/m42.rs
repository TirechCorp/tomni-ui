//! Marco M42 (fatia da lib): a dica de cada aba (`TabView::tab_tooltip` e
//! `TerminalPanes::on_tab_tooltip`), o ícone de aba caída, o botão "Fechar o painel" do painel
//! vazio, a opção `close_emptied` e o cursor (`Terminal::cursor_blink` e o cursor vazado da
//! sessão encerrada).

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use tomni_ui::prelude::*;
use tomni_ui::terminal::{
    Broadcast, Loopback, LoopbackHandle, PaneLayout, Session, Terminal, TerminalPanes,
    TerminalVisual,
};
use tomni_ui::theme::Theme;
use tomni_ui::{Look, TOOLTIP_DELAY};
use tomni_ui_test::TestApp;

const TEMAS: [Theme; 2] = [Theme::Light, Theme::Dark];

/// O texto da dica `id` (`{id do widget}-tooltip`): o `Text` filho dela.
fn texto_da_dica(app: &TestApp, id: &str) -> String {
    let arvore = app.tree();
    let no = arvore.find(id).unwrap_or_else(|| panic!("sem {id:?}"));
    no.children
        .first()
        .and_then(|texto| texto.text.clone())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------------------
// TabView::tab_tooltip
// ---------------------------------------------------------------------------------------

/// Um `TabView` `abas` com `a1` e `a2`. A dica de `a1` vem do signal `dica` (vazio: o padrão,
/// o título), e a de `a2` nunca é dada.
fn abas_com_dica() -> (TestApp, Signal<Option<String>>) {
    let guardado = Rc::new(Cell::new(None));
    let g = Rc::clone(&guardado);
    let app = TestApp::new(move || {
        let dica = signal(Some("Servidor web (conectada)".to_owned()));
        g.set(Some(dica));
        let aba = signal("a1".to_owned());
        Form::new("Abas").child(
            TabView::bind(aba)
                .id("abas")
                .items(
                    || vec!["a1".to_owned(), "a2".to_owned()],
                    |k| k.clone(),
                    |k| k.clone(),
                    Text::new,
                )
                .tab_tooltip(move |chave| if chave == "a1" { dica.get() } else { None })
                .height(120),
        )
    })
    .size(600, 300);
    (app, guardado.get().unwrap())
}

#[test]
fn a_dica_da_aba_vem_do_app_e_o_padrao_e_o_titulo() {
    let (mut app, dica) = abas_com_dica();
    app.hover("abas-a1");
    assert!(!app.is_open("abas-a1-tooltip"), "a dica espera");
    app.wait(TOOLTIP_DELAY + Duration::from_millis(10));
    assert!(app.is_open("abas-a1-tooltip"));
    assert_eq!(
        texto_da_dica(&app, "abas-a1-tooltip"),
        "Servidor web (conectada)"
    );

    // A dica acompanha o signal que a closure lê.
    dica.set(Some("Servidor web (caiu)".to_owned()));
    app.frame();
    assert_eq!(
        texto_da_dica(&app, "abas-a1-tooltip"),
        "Servidor web (caiu)"
    );

    // Sem texto do app, vale o título da aba.
    dica.set(None);
    app.frame();
    assert_eq!(texto_da_dica(&app, "abas-a1-tooltip"), "a1");

    app.hover("abas-a2");
    app.wait(TOOLTIP_DELAY + Duration::from_millis(10));
    assert!(app.is_open("abas-a2-tooltip"));
    assert_eq!(texto_da_dica(&app, "abas-a2-tooltip"), "a2");
}

#[test]
fn tabview_sem_tab_tooltip_mostra_o_titulo() {
    let mut app = TestApp::new(|| {
        let aba = signal("x".to_owned());
        Form::new("Abas").child(
            TabView::bind(aba)
                .id("abas")
                .tab("x", "Primeira", Text::new("um"))
                .tab("y", "Segunda", Text::new("dois"))
                .height(120),
        )
    })
    .size(600, 300);
    app.hover("abas-y");
    app.wait(TOOLTIP_DELAY + Duration::from_millis(10));
    assert_eq!(texto_da_dica(&app, "abas-y-tooltip"), "Segunda");
}

// ---------------------------------------------------------------------------------------
// TerminalPanes: ícone, dica e painel vazio
// ---------------------------------------------------------------------------------------

struct Paineis {
    app: TestApp,
    layout: Signal<PaneLayout>,
    envio: Signal<Broadcast>,
    sessoes: Vec<Session>,
    handles: Vec<LoopbackHandle>,
    fechadas: Rc<RefCell<Vec<String>>>,
}

/// `n` sessões `web-01`… num painel só, com a dica `dica: <título> / <aberta ou caída>`.
fn paineis(n: usize, tema: Theme) -> Paineis {
    let (sessoes, handles): (Vec<Session>, Vec<LoopbackHandle>) = (1..=n)
        .map(|i| {
            let nome = format!("web-{i:02}");
            let loopback = Loopback::new().echo();
            let handle = loopback.handle();
            (Session::new(loopback).key(nome.clone()).name(nome), handle)
        })
        .unzip();
    let guarda = Rc::new(Cell::new(None));
    let fechadas = Rc::new(RefCell::new(Vec::new()));
    let (g, f, s) = (Rc::clone(&guarda), Rc::clone(&fechadas), sessoes.clone());
    let app = TestApp::new(move || {
        let layout = signal(PaneLayout::with(s.clone()));
        let envio = signal(Broadcast::None);
        g.set(Some((layout, envio)));
        let f = Rc::clone(&f);
        Form::new("Painéis").child(
            TerminalPanes::bind(layout)
                .id("paineis")
                .broadcast(envio)
                .on_session_closed(move |sessao| {
                    f.borrow_mut().push(sessao.key_str());
                    sessao.dispose();
                })
                .on_tab_tooltip(|sessao| {
                    let estado = if sessao.closed() { "caída" } else { "aberta" };
                    Some(format!("{} / {estado}", sessao.title()))
                })
                .grow(1),
        )
    })
    .theme(tema)
    .size(900, 600);
    let (layout, envio) = guarda.get().unwrap();
    Paineis {
        app,
        layout,
        envio,
        sessoes,
        handles,
        fechadas,
    }
}

fn icone(app: &TestApp, id: &str) -> Option<Symbol> {
    match app.look(id) {
        Look::TabHeader { visual, .. } => visual.icon,
        outro => panic!("{outro:?}"),
    }
}

#[test]
fn a_aba_da_sessao_encerrada_mostra_o_aviso() {
    let mut p = paineis(2, Theme::Light);
    assert_eq!(icone(&p.app, "paineis-painel-0-web-01"), None);

    p.handles[0].close(Some("caiu".to_owned()));
    p.app.frame();
    assert_eq!(
        icone(&p.app, "paineis-painel-0-web-01"),
        Some(Symbol::Warning)
    );
    assert_eq!(icone(&p.app, "paineis-painel-0-web-02"), None);

    // Com o envio para todas ligado, a caída continua com o aviso, e a viva ganha o ícone de
    // envio.
    p.envio.set(Broadcast::All);
    p.app.frame();
    assert_eq!(
        icone(&p.app, "paineis-painel-0-web-01"),
        Some(Symbol::Warning)
    );
    assert_eq!(
        icone(&p.app, "paineis-painel-0-web-02"),
        Some(Symbol::Broadcast)
    );

    // Reabrir tira o aviso: volta o ícone do envio.
    assert!(p.sessoes[0].restart());
    p.app.frame();
    assert_eq!(
        icone(&p.app, "paineis-painel-0-web-01"),
        Some(Symbol::Broadcast)
    );
    p.envio.set(Broadcast::None);
    p.app.frame();
    assert_eq!(icone(&p.app, "paineis-painel-0-web-01"), None);
}

#[test]
fn a_marca_do_app_troca_o_icone_da_aba_e_perde_para_a_queda() {
    let loopback = Loopback::new().echo();
    let handle = loopback.handle();
    let sessao = Session::new(loopback).key("web-01").name("web-01");
    let marcada = Rc::new(Cell::new(None));
    let m = Rc::clone(&marcada);
    let mut app = TestApp::new(move || {
        let layout = signal(PaneLayout::with(vec![sessao]));
        let marca = signal(false);
        m.set(Some(marca));
        Form::new("Painéis").child(
            TerminalPanes::bind(layout)
                .id("paineis")
                .on_tab_mark(move |_| marca.get().then_some(Symbol::Sparkle))
                .grow(1),
        )
    })
    .theme(Theme::Light)
    .size(900, 600);
    let marca = marcada.get().unwrap();
    assert_eq!(icone(&app, "paineis-painel-0-web-01"), None);

    marca.set(true);
    app.frame();
    assert_eq!(
        icone(&app, "paineis-painel-0-web-01"),
        Some(Symbol::Sparkle)
    );

    // A sessão encerrada vale mais que a marca.
    handle.close(Some("caiu".to_owned()));
    app.frame();
    assert_eq!(
        icone(&app, "paineis-painel-0-web-01"),
        Some(Symbol::Warning)
    );
}

fn cor_do_icone(app: &TestApp, id: &str) -> Option<ThemeColor> {
    match app.look(id) {
        Look::TabHeader { visual, .. } => visual.icon_tint,
        outro => panic!("{outro:?}"),
    }
}

#[test]
fn a_cor_da_marca_do_app_pinta_o_icone_e_some_com_a_queda() {
    let loopback = Loopback::new().echo();
    let handle = loopback.handle();
    let sessao = Session::new(loopback).key("web-01").name("web-01");
    let cor = ThemeColor::themed(Color::hex(0x0F6CBD), Color::hex(0x4CC2FF));
    let marcada = Rc::new(Cell::new(None));
    let m = Rc::clone(&marcada);
    let mut app = TestApp::new(move || {
        let layout = signal(PaneLayout::with(vec![sessao]));
        let marca = signal(false);
        m.set(Some(marca));
        Form::new("Painéis").child(
            TerminalPanes::bind(layout)
                .id("paineis")
                .on_tab_mark(move |_| marca.get().then_some(Symbol::Sparkle))
                .on_tab_mark_tint(move |_| Some(cor))
                .grow(1),
        )
    })
    .theme(Theme::Light)
    .size(900, 600);
    let marca = marcada.get().unwrap();
    // Sem marca, a cor não aparece sozinha.
    assert_eq!(cor_do_icone(&app, "paineis-painel-0-web-01"), None);

    marca.set(true);
    app.frame();
    assert_eq!(cor_do_icone(&app, "paineis-painel-0-web-01"), Some(cor));

    // A queda com motivo troca a cor da marca pelo vermelho (M62).
    handle.close(Some("caiu".to_owned()));
    app.frame();
    assert_eq!(
        cor_do_icone(&app, "paineis-painel-0-web-01"),
        Some(ThemeColor::Token(Token::SystemFillColorCritical))
    );
}

#[test]
fn a_dica_da_aba_do_terminal_vem_do_app_e_acompanha_a_queda() {
    let mut p = paineis(2, Theme::Light);
    p.app.hover("paineis-painel-0-web-01");
    p.app.wait(TOOLTIP_DELAY + Duration::from_millis(10));
    assert_eq!(
        texto_da_dica(&p.app, "paineis-painel-0-web-01-tooltip"),
        "web-01 / aberta"
    );
    p.handles[0].close(Some("caiu".to_owned()));
    p.app.frame();
    assert_eq!(
        texto_da_dica(&p.app, "paineis-painel-0-web-01-tooltip"),
        "web-01 / caída"
    );
}

#[test]
fn o_painel_vazio_ganha_o_botao_de_fechar() {
    let mut p = paineis(2, Theme::Light);
    // Com um painel só, não há o que fechar.
    assert!(!p.app.visible("paineis-painel-0-fechar"));

    p.layout.update(|l| l.grid(2, 1));
    p.app.frame();
    assert_eq!(p.layout.get_untracked().to_string(), "row[0,1]");
    // Esvazia o painel 1 pelo ✕ da aba: a sessão sai do layout e o app a descarta.
    p.app.click("paineis-painel-1-web-02-fechar");
    assert_eq!(*p.fechadas.borrow(), ["web-02"]);
    assert!(p.app.visible("paineis-painel-1-vazio"));
    assert!(p.app.visible("paineis-painel-1-fechar"));
    assert!(!p.app.visible("paineis-painel-0-fechar"));

    p.app.click("paineis-painel-1-fechar");
    assert_eq!(p.layout.get_untracked().to_string(), "[0]");
    assert_eq!(p.layout.get_untracked().sessions(), [p.sessoes[0]]);
    assert!(!p.sessoes[0].closed(), "a sessão do outro painel segue");
}

#[test]
fn o_painel_vazio_fecha_pelo_menu_de_contexto() {
    let mut p = paineis(2, Theme::Light);
    p.layout.update(|l| {
        l.grid(2, 1);
        l.remove(p.sessoes[1]);
    });
    p.app.frame();
    p.app.context_menu("paineis-painel-1-vazio");
    p.app.click("paineis-painel-1-fechar-menu");
    assert_eq!(p.layout.get_untracked().pane_count(), 1);
}

#[test]
fn com_close_emptied_o_painel_some_sozinho() {
    let mut p = paineis(3, Theme::Light);
    p.layout.update(|l| {
        l.close_emptied(true);
        l.grid(2, 1);
    });
    p.app.frame();
    assert_eq!(p.layout.get_untracked().pane_count(), 2);
    // Fechar a última aba do painel 0 fecha o painel.
    p.app.click("paineis-painel-0-web-01-fechar");
    assert_eq!(p.layout.get_untracked().pane_count(), 1);
    assert_eq!(p.layout.get_untracked().to_string(), "[0]");
}

// ---------------------------------------------------------------------------------------
// PaneLayout::close_emptied
// ---------------------------------------------------------------------------------------

fn sessao() -> Session {
    Session::new(Loopback::new())
}

#[test]
fn close_emptied_fecha_o_painel_que_perdeu_a_ultima_aba() {
    let (a, b, c) = (sessao(), sessao(), sessao());
    let mut layout = PaneLayout::with([a, b, c]);
    layout.grid(3, 1);
    assert_eq!(layout.to_string(), "row[0,1,2]");

    // Desligada, o painel fica vazio.
    assert!(layout.remove(b));
    assert_eq!(layout.to_string(), "row[0,1,2]");
    layout.move_tab(b, 1);

    layout.close_emptied(true);
    assert!(layout.remove(b));
    assert_eq!(layout.to_string(), "row[0,1]");
    assert_eq!(layout.sessions(), [a, c]);
    assert!(!layout.remove(b), "quem não está lá não fecha nada");

    // O último painel nunca fecha.
    assert!(layout.remove(a));
    assert!(layout.remove(c));
    assert_eq!(layout.pane_count(), 1);
    assert!(layout.sessions().is_empty());
    for s in [a, b, c] {
        s.dispose();
    }
}

#[test]
fn close_emptied_nao_fecha_o_painel_que_nasce_vazio() {
    let a = sessao();
    let mut layout = PaneLayout::with([a]);
    layout.close_emptied(true);
    let novo = layout.split(0, tomni_ui::terminal::PaneDirection::Right);
    assert_eq!(layout.pane_count(), 2);
    assert!(layout.pane(novo).tabs.is_empty());
    // Mover a aba para o painel novo esvazia o de origem, que fecha: sobra o painel novo.
    layout.move_tab(a, novo);
    assert_eq!(layout.pane_count(), 1);
    assert_eq!(layout.sessions(), [a]);
    assert_eq!(layout.active(), 0);
    a.dispose();
}

#[test]
fn close_emptied_ao_mover_a_ultima_aba_fecha_a_origem_e_ajusta_o_destino() {
    let (a, b, c) = (sessao(), sessao(), sessao());
    let mut layout = PaneLayout::with([a, b, c]);
    layout.grid(3, 1);
    layout.close_emptied(true);
    // `a` está no painel 0; mover para o painel 2 fecha o 0, e o destino vira o 1.
    layout.move_tab(a, 2);
    assert_eq!(layout.to_string(), "row[0,1]");
    assert_eq!(layout.pane_of(a), Some(1));
    assert_eq!(layout.active(), 1);
    assert_eq!(layout.pane(1).tabs, [c, a]);
    for s in [a, b, c] {
        s.dispose();
    }
}

#[test]
fn close_emptied_mantem_a_opcao_no_single_e_no_grid() {
    let (a, b) = (sessao(), sessao());
    let mut layout = PaneLayout::with([a, b]);
    layout.close_emptied(true);
    layout.grid(2, 1);
    layout.single();
    layout.grid(2, 1);
    assert!(layout.closes_emptied());
    layout.remove(a);
    assert_eq!(layout.pane_count(), 1);
    for s in [a, b] {
        s.dispose();
    }
}

// ---------------------------------------------------------------------------------------
// Terminal::cursor_blink e o cursor da sessão encerrada
// ---------------------------------------------------------------------------------------

fn terminal(piscar: Signal<bool>) -> (TestApp, LoopbackHandle) {
    let conexao = Loopback::new();
    let handle = conexao.handle();
    let app = TestApp::new(move || {
        Form::new("Console").child(
            Terminal::new(conexao)
                .id("console")
                .cursor_blink(piscar)
                .grow(1),
        )
    })
    .size(780, 400)
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

/// Se o visual do terminal tem a fase apagada do pisca, o que só acontece com foco e piscando.
fn pisca(app: &TestApp) -> bool {
    match app.look("console") {
        Look::Custom(custom) => custom.0.hide_caret().is_some(),
        outro => panic!("não é o terminal: {outro:?}"),
    }
}

#[test]
fn o_cursor_pisca_com_foco_e_a_sessao_encerrada_o_deixa_fixo() {
    let piscar = signal(true);
    let (mut app, handle) = terminal(piscar);
    app.click("console");
    assert!(pisca(&app));
    assert!(!visual(&app).closed());

    handle.close(Some("caiu".to_owned()));
    app.frame();
    assert!(visual(&app).closed());
    assert!(!pisca(&app), "a sessão encerrada tem o cursor fixo");
}

#[test]
fn cursor_blink_falso_vale_mais_que_o_pedido_do_programa() {
    let piscar = signal(true);
    let (mut app, handle) = terminal(piscar);
    app.click("console");
    assert!(pisca(&app));

    // Sem recriar o widget.
    piscar.set(false);
    app.frame();
    assert!(!pisca(&app));
    // O programa pede para piscar (`ESC[?12h`): a escolha da pessoa vale mais.
    handle.feed(b"\x1b[?12h");
    app.frame();
    assert!(!pisca(&app));

    piscar.set(true);
    app.frame();
    assert!(pisca(&app));
}

#[test]
fn snapshot_do_cursor_da_sessao_encerrada() {
    for tema in TEMAS {
        let (mut app, handle) = terminal(signal(true));
        app = app.theme(tema);
        handle.feed(b"user@web-01:~$ uptime\r\n 10:42 up 3 days\r\nuser@web-01:~$ ");
        app.click("console");
        handle.close(Some("o servidor caiu".to_owned()));
        app.frame();
        app.assert_snapshot(&format!(
            "m42_cursor_sessao_encerrada_{}",
            match tema {
                Theme::Light => "claro",
                Theme::Dark => "escuro",
            }
        ));
    }
}

#[test]
fn atalhos_nenhum_nao_contam_como_repetidos_no_lint() {
    let mut app = TestApp::new(|| {
        Form::new("Atalhos").child(
            Column::new()
                .child(
                    Button::new("Um")
                        .id("um")
                        .accelerator(Shortcut::NONE, || {}),
                )
                .child(
                    Button::new("Dois")
                        .id("dois")
                        .accelerator(Shortcut::NONE, || {}),
                ),
        )
    });
    let problemas = app.lint();
    assert!(
        problemas
            .iter()
            .all(|p| p.rule.name() != "duplicate-accelerator"),
        "{problemas:?}"
    );
}
