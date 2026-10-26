//! Marco M62 (pedido pelo Tirech Omni), fatia da lib: o selo de estado nas linhas do `TreeView`
//! (`.item_status`), a dica da linha (`.item_tooltip`) e o item desabilitado do `MenuFlyout`
//! (`.item_enabled`). Também, a aba que conecta (`Session::connecting`, o anel e o nome
//! "conectando") e a aba que caiu (o ícone vermelho).

mod comum;

use std::cell::Cell;
use std::cell::RefCell;
use std::io;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use comum::theme_name;
use tomni_ui::prelude::*;
use tomni_ui::terminal::{
    Loopback, LoopbackHandle, PaneLayout, Session, Status, TerminalPanes, Transport,
};
use tomni_ui::theme::Theme;
use tomni_ui::{Key, Look, ProgressStatus, TOOLTIP_DELAY, TabProgress};
use tomni_ui_test::TestApp;

const TEMAS: [Theme; 2] = [Theme::Light, Theme::Dark];

fn servidores() -> Vec<TreeItem> {
    vec![
        TreeItem::folder("producao", "Produção")
            .child(TreeItem::leaf("web-01", "web-01").icon(Symbol::Server))
            .child(TreeItem::leaf("web-02", "web-02").icon(Symbol::Server))
            .child(TreeItem::leaf("db-01", "db-01").icon(Symbol::Database))
            .child(TreeItem::leaf("cache", "cache").icon(Symbol::Server)),
        TreeItem::leaf("bastion", "bastion").icon(Symbol::Desktop),
    ]
}

type Estados = Signal<Vec<(&'static str, ItemStatus)>>;

/// Uma árvore com o estado de cada linha vindo de um signal e a dica só na "web-02".
fn app_arvore(estados: Estados) -> TestApp {
    TestApp::new(move || {
        let itens = signal(servidores());
        let selecionado = signal(None::<String>);
        let aberto = signal(std::collections::BTreeSet::from(["producao".to_owned()]));
        Form::new("Servidores").child(
            Column::new().padding(12).grow(1).child(
                TreeView::bind(itens, selecionado)
                    .id("arvore")
                    .grow(1)
                    .expanded(aberto)
                    .item_status(move |chave| {
                        estados.with(|v| v.iter().find(|(k, _)| *k == chave).map(|(_, s)| *s))
                    })
                    .item_tooltip(|chave| {
                        (chave == "web-02").then(|| "A conexão caiu: tempo esgotado".to_owned())
                    }),
            ),
        )
    })
    .size(360, 320)
}

fn todos_os_estados() -> Estados {
    signal(vec![
        ("web-01", ItemStatus::Success),
        ("web-02", ItemStatus::Critical),
        ("db-01", ItemStatus::Caution),
        ("cache", ItemStatus::Neutral),
    ])
}

#[test]
fn o_selo_aparece_muda_e_some_com_o_signal() {
    let estados = signal(vec![("web-01", ItemStatus::Caution)]);
    let mut app = app_arvore(estados);
    let status = |app: &TestApp, id: &str| app.tree().find(id).and_then(|n| n.status);
    assert_eq!(status(&app, "arvore-web-01"), Some("caution"));
    assert_eq!(status(&app, "arvore-web-02"), None);

    estados.set(vec![
        ("web-01", ItemStatus::Success),
        ("web-02", ItemStatus::Critical),
    ]);
    app.frame();
    assert_eq!(status(&app, "arvore-web-01"), Some("success"));
    assert_eq!(status(&app, "arvore-web-02"), Some("critical"));

    estados.set(Vec::new());
    app.frame();
    assert_eq!(status(&app, "arvore-web-01"), None);
    assert_eq!(status(&app, "arvore-web-02"), None);
}

#[test]
fn o_nome_acessivel_leva_o_estado() {
    let app = app_arvore(todos_os_estados());
    assert_eq!(app.text("arvore-web-01"), "web-01, conectado");
    assert_eq!(app.text("arvore-web-02"), "web-02, com erro");
    assert_eq!(app.text("arvore-db-01"), "db-01, conectando");
    assert_eq!(app.text("arvore-cache"), "cache, encerrado");
    assert_eq!(app.text("arvore-bastion"), "bastion");
}

#[test]
fn a_dica_abre_com_o_ponteiro_parado_e_texto_vazio_nao_abre() {
    let mut app = app_arvore(todos_os_estados());
    app.hover("arvore-web-02");
    assert!(!app.is_open("arvore-web-02-tooltip"), "a dica espera");
    app.wait(TOOLTIP_DELAY + Duration::from_millis(10));
    assert!(app.is_open("arvore-web-02-tooltip"));

    app.hover("arvore-web-01");
    app.wait(TOOLTIP_DELAY + Duration::from_millis(10));
    assert!(!app.is_open("arvore-web-02-tooltip"), "o ponteiro saiu");
    assert!(
        !app.is_open("arvore-web-01-tooltip"),
        "sem texto, a dica não abre"
    );
}

#[test]
fn snapshot_dos_estados_na_arvore() {
    for theme in TEMAS {
        let mut app = app_arvore(todos_os_estados()).theme(theme);
        app.assert_lint();
        app.assert_snapshot(&format!("m62-arvore-estados-{}", theme_name(theme)));
    }
}

/// Um menu com um item desabilitado entre dois habilitados; `log` anota o que rodou.
fn app_menu(log: Rc<RefCell<Vec<String>>>, habilitado: Signal<bool>) -> TestApp {
    let anota = |o: &'static str| {
        let log = Rc::clone(&log);
        move || log.borrow_mut().push(o.to_owned())
    };
    TestApp::new(move || {
        Form::new("Menu").child(
            Button::new("Menu").id("botao").flyout(
                MenuFlyout::new()
                    .id("menu")
                    .item("um", "Um", anota("um"))
                    .access_key('1')
                    .item("dois", "Dois", anota("dois"))
                    .access_key('2')
                    .shortcut(Shortcut::ctrl('D'))
                    .item_enabled(habilitado)
                    .item("tres", "Três", anota("tres"))
                    .access_key('3'),
            ),
        )
    })
    .size(360, 240)
}

#[test]
fn item_desabilitado_nao_dispara_no_clique_no_atalho_nem_no_enter() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let habilitado = signal(false);
    let mut app = app_menu(Rc::clone(&log), habilitado);

    app.click("botao");
    assert!(app.is_open("menu"));
    assert!(!app.enabled("dois"));
    // O clique no item desabilitado não faz nada e o menu continua aberto.
    app.click("dois");
    assert!(log.borrow().is_empty());
    assert!(app.is_open("menu"));
    // A tecla de acesso também não.
    app.key(Key::Char('2'));
    assert!(log.borrow().is_empty());
    // O teclado pula o item: o foco abre em "Um", desce direto para "Três" e o Enter roda o "Três".
    assert_eq!(app.focused().as_deref(), Some("um"));
    app.key(Key::Down);
    assert_eq!(app.focused().as_deref(), Some("tres"));
    app.key(Key::Enter);
    assert_eq!(*log.borrow(), ["tres"]);
    // O atalho do item desabilitado não roda com o menu fechado.
    app.shortcut(Shortcut::ctrl('D'));
    assert_eq!(*log.borrow(), ["tres"]);

    // Habilitado pelo signal, o item responde sem montar o menu de novo.
    habilitado.set(true);
    app.frame();
    app.shortcut(Shortcut::ctrl('D'));
    assert_eq!(*log.borrow(), ["tres", "dois"]);
    app.click("botao");
    assert!(app.enabled("dois"));
    app.click("dois");
    assert_eq!(*log.borrow(), ["tres", "dois", "dois"]);
}

#[test]
fn snapshot_do_menu_com_item_desabilitado() {
    for theme in TEMAS {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut app = app_menu(log, signal(false)).theme(theme);
        app.click("botao");
        app.assert_lint();
        app.assert_snapshot(&format!("m62-menu-desabilitado-{}", theme_name(theme)));
    }
}

/// Um transporte que começa conectando: `pronto` o libera, como o SSH que abriu o shell. A
/// reabertura volta a conectar.
struct Conectando {
    interno: Loopback,
    conectando: Arc<AtomicBool>,
}

impl Conectando {
    fn novo() -> (Self, Arc<AtomicBool>, LoopbackHandle) {
        let interno = Loopback::new();
        let handle = interno.handle();
        let conectando = Arc::new(AtomicBool::new(true));
        let transporte = Self {
            interno,
            conectando: Arc::clone(&conectando),
        };
        (transporte, conectando, handle)
    }
}

impl Transport for Conectando {
    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.interno.write(bytes)
    }

    fn read(&mut self, buf: &mut Vec<u8>, max: usize) -> Status {
        self.interno.read(buf, max)
    }

    fn pending(&self) -> bool {
        self.interno.pending()
    }

    fn connecting(&self) -> bool {
        self.conectando.load(Ordering::Acquire)
    }

    fn reopen(&mut self) -> bool {
        self.conectando.store(true, Ordering::Release);
        self.interno.reopen()
    }
}

struct Aba {
    app: TestApp,
    sessao: Session,
    conectando: Arc<AtomicBool>,
    handle: LoopbackHandle,
}

const ABA: &str = "paineis-painel-0-web-01";

fn aba_conectando(tema: Theme) -> Aba {
    let (transporte, conectando, handle) = Conectando::novo();
    let sessao = Session::new(transporte).key("web-01").name("web-01");
    let guarda = Rc::new(Cell::new(None));
    let (g, s) = (Rc::clone(&guarda), sessao);
    let app = TestApp::new(move || {
        let layout = signal(PaneLayout::with(vec![s]));
        g.set(Some(layout));
        Form::new("Painéis").child(TerminalPanes::bind(layout).id("paineis").grow(1))
    })
    .theme(tema)
    .size(900, 600);
    Aba {
        app,
        sessao,
        conectando,
        handle,
    }
}

fn visual(app: &TestApp, id: &str) -> tomni_ui::TabHeaderVisual {
    match app.look(id) {
        Look::TabHeader { visual, .. } => visual,
        outro => panic!("{outro:?}"),
    }
}

const INDETERMINADO: TabProgress = TabProgress {
    value: None,
    status: ProgressStatus::Normal,
};

#[test]
fn a_sessao_acompanha_o_transporte_ao_conectar_cair_e_reabrir() {
    let mut a = aba_conectando(Theme::Light);
    assert!(a.sessao.connecting(), "nasce conectando");

    a.conectando.store(false, Ordering::Release);
    a.app.frame();
    assert!(!a.sessao.connecting(), "sem bytes, só pelo transporte");

    a.handle.close(Some("tempo esgotado".into()));
    a.app.frame();
    assert!(a.sessao.closed());
    assert_eq!(a.sessao.close_reason().as_deref(), Some("tempo esgotado"));
    assert!(!a.sessao.connecting());

    // Reconectar: volta a conectar até o transporte liberar de novo.
    assert!(a.sessao.restart());
    assert!(a.sessao.connecting());
    assert!(a.sessao.close_reason().is_none());
    a.conectando.store(false, Ordering::Release);
    a.app.frame();
    assert!(!a.sessao.connecting());

    // Cair no meio da conexão também zera.
    assert!(!a.sessao.restart(), "aberta, não reinicia");
    a.handle.close(None);
    a.app.frame();
    assert!(a.sessao.restart());
    assert!(a.sessao.connecting());
    a.handle.close(Some("recusada".into()));
    a.app.frame();
    assert!(!a.sessao.connecting(), "fechar encerra o conectando");
}

#[test]
fn a_aba_mostra_o_anel_e_diz_conectando_ate_abrir() {
    let mut a = aba_conectando(Theme::Light);
    assert_eq!(visual(&a.app, ABA).progress, Some(INDETERMINADO));
    assert_eq!(a.app.text(ABA), "web-01, conectando");

    a.conectando.store(false, Ordering::Release);
    a.app.frame();
    assert_eq!(visual(&a.app, ABA).progress, None);
    assert_eq!(a.app.text(ABA), "web-01");
}

#[test]
fn o_progresso_do_programa_vale_mais_que_o_conectando() {
    let mut a = aba_conectando(Theme::Light);
    a.sessao.feed(b"\x1b]9;4;1;45\x07");
    a.app.frame();
    assert_eq!(
        visual(&a.app, ABA).progress,
        Some(TabProgress {
            value: Some(0.45),
            status: ProgressStatus::Normal
        })
    );
}

#[test]
fn a_aba_que_caiu_com_motivo_fica_com_o_icone_vermelho() {
    let mut a = aba_conectando(Theme::Light);
    a.conectando.store(false, Ordering::Release);
    a.app.frame();
    assert_eq!(visual(&a.app, ABA).icon_tint, None);

    a.handle.close(Some("tempo esgotado".into()));
    a.app.frame();
    assert_eq!(
        visual(&a.app, ABA).icon_tint,
        Some(ThemeColor::Token(Token::SystemFillColorCritical))
    );

    // Encerrar sem motivo (o usuário saiu) não é queda: o ícone volta à cor do título.
    assert!(a.sessao.restart());
    a.conectando.store(false, Ordering::Release);
    a.handle.close(None);
    a.app.frame();
    assert_eq!(visual(&a.app, ABA).icon_tint, None);
}

#[test]
fn snapshot_da_aba_conectando_e_da_que_caiu() {
    for theme in TEMAS {
        let mut a = aba_conectando(theme);
        a.app.assert_lint();
        a.app
            .assert_snapshot(&format!("m62-aba-conectando-{}", theme_name(theme)));

        a.conectando.store(false, Ordering::Release);
        a.handle.close(Some("tempo esgotado".into()));
        a.app.frame();
        a.app
            .assert_snapshot(&format!("m62-aba-caiu-{}", theme_name(theme)));
    }
}
