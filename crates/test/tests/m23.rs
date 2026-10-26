//! Marco M23: terminais em painéis e o SSH completo. O divisor por fração, as abas que trocam
//! de `TabView`, a sessão separada do widget, o layout dos painéis, os atalhos, o envio para
//! vários e as ferramentas.

mod comum;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use tomni_ui::kurbo::Point;
use tomni_ui::prelude::*;
use tomni_ui::terminal::{
    Loopback, LoopbackHandle, PaneLayout, PaneShortcuts, Session, Terminal, TerminalPanes,
};
use tomni_ui::theme::Theme;
use tomni_ui::{Key, Modifiers, SPLIT_KEY_STEP, SPLIT_SIZER, Shortcut};
use tomni_ui_test::TestApp;

/// Um `Split` com a fração em `fracao`, os mínimos de 100 e 200 px.
fn divisor_fracao(fracao: Signal<f64>) -> Form {
    Form::new("Divisor").child(
        Split::row(
            Text::new("Arquivos").id("esq"),
            Text::new("Conteúdo").id("dir"),
        )
        .id("div")
        .ratio(fracao)
        .min_first(100.0)
        .min_second(200.0),
    )
}

fn fracao_do_app(inicial: f64) -> (TestApp, Signal<f64>) {
    let guarda = Rc::new(Cell::new(None));
    let g = Rc::clone(&guarda);
    let app = TestApp::new(move || {
        let s = signal(inicial);
        g.set(Some(s));
        divisor_fracao(s)
    })
    .size(800, 400);
    (app, guarda.get().unwrap())
}

#[test]
fn fracao_se_mantem_quando_a_janela_muda() {
    let (mut app, fracao) = fracao_do_app(0.5);
    let todo = app.rect("div").width();
    let esperado = ((todo - SPLIT_SIZER) * 0.5).round();
    assert!((app.rect("div-primeiro").width() - esperado).abs() <= 1.0);
    assert_eq!(app.text("div"), "0.50");

    app.resize(1200, 400);
    let todo = app.rect("div").width();
    let esperado = (todo - SPLIT_SIZER) * 0.5;
    assert!((app.rect("div-primeiro").width() - esperado).abs() <= 1.0);
    assert_eq!(fracao.get_untracked(), 0.5);

    // O app muda o signal e o divisor anda.
    fracao.set(0.25);
    app.frame();
    let esperado = (todo - SPLIT_SIZER) * 0.25;
    assert!((app.rect("div-primeiro").width() - esperado).abs() <= 1.0);
}

#[test]
fn arrasto_e_setas_mudam_a_fracao_e_os_minimos_valem() {
    let (mut app, fracao) = fracao_do_app(0.5);
    let sala = app.rect("div").width() - SPLIT_SIZER;
    let sizer = app.rect("div-divisor").center();
    app.drag(sizer, sizer + (80.0, 0.0));
    assert!((fracao.get_untracked() - (0.5 + 80.0 / sala)).abs() < 1e-9);

    app.tab();
    assert_eq!(app.focused().as_deref(), Some("div-divisor"));
    let antes = fracao.get_untracked();
    app.key(Key::Left);
    assert!((fracao.get_untracked() - (antes - SPLIT_KEY_STEP / sala)).abs() < 1e-9);

    // Mínimos: 100 px para o primeiro, 200 px para o segundo.
    let sizer = app.rect("div-divisor").center();
    app.drag(sizer, Point::new(0.0, sizer.y));
    assert!((fracao.get_untracked() - 100.0 / sala).abs() < 1e-9);
    let sizer = app.rect("div-divisor").center();
    app.drag(sizer, Point::new(799.0, sizer.y));
    assert!((fracao.get_untracked() - (1.0 - 200.0 / sala)).abs() < 1e-9);
    assert_eq!(app.rect("div-segundo").width(), 200.0);
}

// ---- Fatia 2: o TabView troca abas com outro do mesmo grupo e tem ícone ----

use tomni_ui::{Look, Symbol};

type Lista = Signal<Vec<String>>;
type Soltas = Rc<RefCell<Vec<(String, usize)>>>;

/// Dois `TabView` (`a` e `b`) no grupo `grupo_a` e `grupo_b`; o `Drop` move a aba de uma
/// lista para a outra. Devolve as listas.
fn dois_tabviews(
    grupo_a: Option<&'static str>,
    grupo_b: Option<&'static str>,
) -> (TestApp, Lista, Lista, Soltas) {
    let guarda: Rc<RefCell<Option<(Lista, Lista)>>> = Rc::new(RefCell::new(None));
    let soltas = Rc::new(RefCell::new(Vec::new()));
    let (g, s) = (Rc::clone(&guarda), Rc::clone(&soltas));
    let app = TestApp::new(move || {
        let a = signal(vec!["a1".to_owned(), "a2".to_owned()]);
        let b = signal(vec!["b1".to_owned()]);
        *g.borrow_mut() = Some((a, b));
        let aba_a = signal("a1".to_owned());
        let aba_b = signal("b1".to_owned());
        let monta = |id: &'static str,
                     lista: Lista,
                     outra: Lista,
                     aba: Signal<String>,
                     grupo: Option<&'static str>,
                     s: Rc<RefCell<Vec<(String, usize)>>>| {
            let mut tv = TabView::bind(aba)
                .id(id)
                .density(Density::Compact)
                .items(lista, |k| k.clone(), |k| k.clone(), Text::new)
                .tab_icon(|k| (k == "a2").then_some(Symbol::Broadcast))
                .on_tab_drop(move |chave, indice| {
                    s.borrow_mut().push((chave.clone(), indice));
                    outra.update(|v| v.retain(|k| *k != chave));
                    lista.update(|v| v.insert(indice.min(v.len()), chave));
                });
            if let Some(grupo) = grupo {
                tv = tv.drag_group(grupo);
            }
            tv.height(120)
        };
        Form::new("Abas").child(
            Column::new()
                .child(monta("ta", a, b, aba_a, grupo_a, Rc::clone(&s)))
                .child(monta("tb", b, a, aba_b, grupo_b, Rc::clone(&s))),
        )
    })
    .size(600, 400);
    let (a, b) = (*guarda.borrow()).unwrap();
    (app, a, b, soltas)
}

#[test]
fn abas_do_mesmo_grupo_trocam_de_tabview_pelo_arrasto() {
    let (mut app, a, b, soltas) = dois_tabviews(Some("g"), Some("g"));
    let b1 = app.rect("tb-b1");
    let destino = Point::new(b1.x0 + b1.width() * 0.25, b1.center().y);
    app.drag("ta-a2", destino);
    assert_eq!(*soltas.borrow(), [("a2".to_owned(), 0)]);
    assert_eq!(a.get_untracked(), ["a1"]);
    assert_eq!(b.get_untracked(), ["a2", "b1"]);
    assert_eq!(app.tab_titles("tb"), ["a2", "b1"]);

    // De volta, para depois da última aba de `a`.
    let a1 = app.rect("ta-a1");
    let destino = Point::new(a1.x0 + a1.width() * 0.75, a1.center().y);
    app.drag("tb-a2", destino);
    assert_eq!(soltas.borrow()[1], ("a2".to_owned(), 1));
    assert_eq!(app.tab_titles("ta"), ["a1", "a2"]);
}

#[test]
fn grupos_diferentes_ou_sem_grupo_nao_trocam_abas() {
    for (ga, gb) in [(Some("x"), Some("y")), (None, None), (Some("x"), None)] {
        let (mut app, a, b, soltas) = dois_tabviews(ga, gb);
        let b1 = app.rect("tb-b1");
        app.drag("ta-a2", b1.center());
        assert!(soltas.borrow().is_empty(), "{ga:?} {gb:?}");
        assert_eq!(a.get_untracked(), ["a1", "a2"]);
        assert_eq!(b.get_untracked(), ["b1"]);
    }
}

#[test]
fn icone_da_aba_aparece_no_visual() {
    let (app, ..) = dois_tabviews(None, None);
    let icone = |id: &str| match app.look(id) {
        Look::TabHeader { visual, .. } => visual.icon,
        outro => panic!("{outro:?}"),
    };
    assert_eq!(icone("ta-a2"), Some(Symbol::Broadcast));
    assert_eq!(icone("ta-a1"), None);
    assert!(app.rect("ta-a2").width() >= 100.0);
}

// ---------------------------------------------------------------------------------------------
// Sessão separada do widget
// ---------------------------------------------------------------------------------------------

/// Uma tela que mostra a sessão `sessao` em `Terminal::session` enquanto `visivel` for `true`,
/// com o título da sessão numa linha de texto.
fn tela_da_sessao(sessao: Session, visivel: Signal<bool>) -> Form {
    Form::new("Sessão").child(
        Column::new()
            .child(Text::new(move || sessao.title()).id("titulo"))
            .child(Show::when(visivel, move || {
                Terminal::session(sessao).id("console").grow(1)
            })),
    )
}

#[test]
fn sessao_sobrevive_ao_terminal_que_a_mostrava() {
    let conexao = Loopback::new().echo();
    let handle = conexao.handle();
    let sessao = Session::new(conexao).key("web-01").name("Servidor web");
    let visivel_guardado = Rc::new(Cell::new(None));
    let guarda = Rc::clone(&visivel_guardado);
    let mut app = TestApp::new(move || {
        let visivel = signal(true);
        guarda.set(Some(visivel));
        tela_da_sessao(sessao, visivel)
    })
    .size(800, 400);
    let visivel = visivel_guardado.get().unwrap();

    app.feed("console", "um\r\ndois\r\n");
    assert_eq!(app.text("titulo"), "Servidor web");
    assert_eq!(app.terminal_text("console"), "um\ndois");

    // Sem o widget, a sessão continua viva, lendo a conexão e guardando o histórico.
    visivel.set(false);
    app.frame();
    assert!(sessao.is_alive());
    handle.feed("tres\r\n");
    Session::poll_all();
    sessao.feed(b"\x1b]0;root@web-01\x07");
    app.frame();
    assert_eq!(app.text("titulo"), "root@web-01");

    // Outro terminal mostra a sessão com o histórico inteiro e o tamanho de antes.
    visivel.set(true);
    app.frame();
    assert_eq!(app.terminal_text("console"), "um\ndois\ntres");
    assert_eq!(sessao.history(), "um\ndois\ntres");
}

#[test]
fn terminal_new_descarta_a_sessao_ao_sair_da_arvore() {
    let achada = Rc::new(Cell::new(None));
    let guarda = Rc::clone(&achada);
    let visivel_guardado = Rc::new(Cell::new(None));
    let v = Rc::clone(&visivel_guardado);
    let mut app = TestApp::new(move || {
        let visivel = signal(true);
        v.set(Some(visivel));
        let guarda = Rc::clone(&guarda);
        Form::new("Console").child(Show::when(visivel, move || {
            let terminal = Terminal::new(Loopback::new()).id("console").grow(1);
            guarda.set(Some(terminal.session_ref()));
            terminal
        }))
    })
    .size(800, 400);
    let sessao = achada.get().unwrap();
    assert!(sessao.is_alive());
    visivel_guardado.get().unwrap().set(false);
    app.frame();
    assert!(!sessao.is_alive());
}

#[test]
fn tamanho_zero_nao_muda_a_grade_da_sessao() {
    let conexao = Loopback::new();
    let handle = conexao.handle();
    let sessao = Session::new(conexao);
    let mut app = TestApp::new(move || {
        Form::new("Console").child(Terminal::session(sessao).id("console").grow(1))
    })
    .size(800, 400);
    let antes = sessao.emulator(|e| (e.cols(), e.rows())).unwrap();
    assert_eq!(handle.size(), Some((antes.0 as u16, antes.1 as u16)));
    app.resize(0, 0);
    app.frame();
    assert_eq!(sessao.emulator(|e| (e.cols(), e.rows())), Some(antes));
    assert_eq!(handle.size(), Some((antes.0 as u16, antes.1 as u16)));
}

// ---------------------------------------------------------------------------------------------
// O widget TerminalPanes
// ---------------------------------------------------------------------------------------------

const TEMAS: [Theme; 2] = [Theme::Light, Theme::Dark];

fn nome_do_tema(tema: Theme) -> &'static str {
    match tema {
        Theme::Light => "claro",
        Theme::Dark => "escuro",
    }
}

/// Um app com `TerminalPanes` de id `paineis` e `n` sessões `web-01`, `web-02`… (cada uma com
/// eco), todas como abas de um painel só.
struct Paineis {
    app: TestApp,
    layout: Signal<PaneLayout>,
    sessoes: Vec<Session>,
    /// A outra ponta de cada sessão: o que ela escreveu e o que o servidor manda.
    handles: Vec<LoopbackHandle>,
    /// As chaves das sessões que o `on_session_closed` recebeu.
    fechadas: Rc<RefCell<Vec<String>>>,
}

fn paineis(n: usize, tema: Theme) -> Paineis {
    paineis_com(n, tema, |paineis| paineis)
}

/// Como [`paineis`], com `ajusta` mexendo no `TerminalPanes` (atalhos, `new_session`).
fn paineis_com(
    n: usize,
    tema: Theme,
    ajusta: impl Fn(TerminalPanes) -> TerminalPanes + 'static,
) -> Paineis {
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
        g.set(Some(layout));
        let f = Rc::clone(&f);
        Form::new("Painéis").child(ajusta(
            TerminalPanes::bind(layout)
                .id("paineis")
                .on_session_closed(move |sessao| {
                    f.borrow_mut().push(sessao.key_str());
                    sessao.dispose();
                })
                .grow(1),
        ))
    })
    .theme(tema)
    .size(900, 600);
    Paineis {
        app,
        layout: guarda.get().unwrap(),
        sessoes,
        handles,
        fechadas,
    }
}

impl Paineis {
    fn forma(&self) -> String {
        self.layout.get_untracked().to_string()
    }

    /// As chaves das abas do painel `n`, da esquerda para a direita.
    fn abas(&self, n: usize) -> Vec<String> {
        self.layout
            .with_untracked(|l| l.pane(n).tabs.iter().map(|s| s.key_str()).collect())
    }

    /// O id do terminal da aba escolhida do painel `n`.
    fn terminal_do_painel(&self, n: usize) -> String {
        let chave = self
            .layout
            .with_untracked(|l| l.pane(n).selected.unwrap().key_str());
        format!("paineis-terminal-{chave}")
    }

    /// O foco está no terminal do painel ativo.
    fn foco_no_ativo(&self) -> bool {
        let ativo = self.layout.get_untracked().active();
        self.app.focused() == Some(self.terminal_do_painel(ativo))
    }

    /// Muda o layout e passa um frame.
    fn muda(&mut self, f: impl FnOnce(&mut PaneLayout)) {
        self.layout.update(f);
        self.app.frame();
    }
}

#[test]
fn monta_com_um_dois_por_dois_e_tres_por_tres() {
    let mut p = paineis(9, Theme::Light);
    assert_eq!(p.forma(), "[0]");
    assert!(p.app.visible("paineis-painel-0"));
    assert!(p.app.visible("paineis-terminal-web-01"));
    assert!(!p.app.visible("paineis-painel-0-vazio"));

    p.muda(|l| l.grid(2, 2));
    assert_eq!(p.forma(), "col[row[0,1],row[2,3]]");
    for n in 0..4 {
        assert!(p.app.rect(&format!("paineis-painel-{n}-quadro")).width() > 100.0);
    }
    assert!(p.app.visible("paineis-terminal-web-03"));

    p.muda(|l| l.grid(3, 3));
    assert_eq!(p.forma(), "col[row[0,1,2],row[3,4,5],row[6,7,8]]");
    for n in 0..9 {
        assert!(p.app.visible(&format!("paineis-painel-{n}")), "painel {n}");
    }
    p.app.assert_lint();
}

#[test]
fn trocar_o_layout_nao_fecha_sessoes_e_mantem_o_historico() {
    let mut p = paineis(4, Theme::Light);
    p.muda(|l| l.grid(2, 2));
    p.app.feed("paineis-terminal-web-02", "um\r\ndois\r\n");
    p.muda(|l| l.single());
    p.muda(|l| l.grid(2, 1));
    for sessao in &p.sessoes {
        assert!(sessao.is_alive());
    }
    assert!(p.fechadas.borrow().is_empty());
    assert_eq!(p.sessoes[1].history(), "um\ndois");
}

#[test]
fn clicar_num_painel_o_faz_ativo() {
    let mut p = paineis(4, Theme::Light);
    p.muda(|l| l.grid(2, 2));
    assert_eq!(p.layout.get_untracked().active(), 0);
    p.app.click("paineis-terminal-web-02");
    assert_eq!(p.layout.get_untracked().active(), 1);
    assert_eq!(p.app.focused().as_deref(), Some("paineis-terminal-web-02"));
}

#[test]
fn o_x_da_aba_tira_a_sessao_do_layout_e_avisa_o_app() {
    let mut p = paineis(2, Theme::Light);
    p.muda(|l| l.grid(2, 1));
    p.app.close_tab("paineis-painel-0", "web-01");
    assert_eq!(*p.fechadas.borrow(), ["web-01"]);
    assert!(!p.sessoes[0].is_alive());
    assert!(p.sessoes[1].is_alive());
    assert_eq!(p.layout.get_untracked().pane(0).tabs.len(), 0);
    assert!(p.app.visible("paineis-painel-0-vazio"));
    assert!(!p.app.visible("paineis-painel-0"));
}

#[test]
fn snapshots_dos_paineis() {
    for tema in TEMAS {
        let nome = |caso: &str| format!("m23_{caso}_{}", nome_do_tema(tema));

        let mut p = paineis(9, tema);
        p.app
            .feed("paineis-terminal-web-01", "$ ls\r\napp  logs\r\n$ ");
        p.app.assert_snapshot(&nome("um_painel"));

        p.muda(|l| l.grid(2, 2));
        p.app.click("paineis-terminal-web-02");
        p.app.assert_snapshot(&nome("grade_2x2_ativo"));

        p.muda(|l| l.grid(3, 3));
        p.app.assert_snapshot(&nome("grade_3x3"));

        p.muda(|l| l.zoom(4));
        p.app.assert_snapshot(&nome("zoom"));
    }
}

const ALT_SHIFT: Modifiers = Modifiers {
    alt: true,
    shift: true,
    ..Modifiers::NONE
};

#[test]
fn atalhos_dividem_pelas_duas_teclas_logicas() {
    for (tecla, forma) in [
        ('-', "col[0,1]"),
        ('_', "col[0,1]"),
        ('+', "row[0,1]"),
        ('=', "row[0,1]"),
    ] {
        let mut p = paineis(1, Theme::Light);
        p.app.click("paineis-terminal-web-01");
        p.app.key_with(Key::Char(tecla), ALT_SHIFT);
        assert_eq!(p.forma(), forma, "Alt+Shift+{tecla}");
        assert_eq!(p.layout.get_untracked().active(), 1, "Alt+Shift+{tecla}");
        assert!(
            p.foco_no_ativo(),
            "Alt+Shift+{tecla}: o foco vai ao painel novo"
        );
    }
}

#[test]
fn atalho_de_duplicar_divide_pelo_lado_maior_e_copia_o_eco() {
    let mut p = paineis(1, Theme::Light);
    p.app.click("paineis-terminal-web-01");
    // 900 × 600: o painel é mais largo que alto, o novo vai à direita.
    p.app.key_with(Key::Char('d'), ALT_SHIFT);
    assert_eq!(p.forma(), "row[0,1]");
    assert!(p.foco_no_ativo());
    // Cada metade é mais alta que larga, o novo vai embaixo.
    p.app.key_with(Key::Char('d'), ALT_SHIFT);
    assert_eq!(p.forma(), "row[0,col[1,2]]");
    assert_eq!(p.layout.get_untracked().active(), 2);
    assert!(p.foco_no_ativo());

    let nova = p.terminal_do_painel(2);
    // O texto logo depois de um atalho é engolido (a plataforma manda a letra do atalho): o "x"
    // fica para isso.
    p.app.type_text("xoi");
    p.app.frame();
    assert!(
        p.app.terminal_text(&nova).contains("oi"),
        "a cópia tem o mesmo eco"
    );
    assert_eq!(p.layout.get_untracked().sessions().len(), 3);
}

#[test]
fn on_duplicate_troca_o_atalho_e_a_copia_entra_quando_o_app_entrega() {
    let chamadas = Rc::new(RefCell::new(Vec::<String>::new()));
    let guardado = Rc::new(RefCell::new(None));
    let (c, g) = (Rc::clone(&chamadas), Rc::clone(&guardado));
    let mut p = paineis_com(1, Theme::Light, move |paineis| {
        let (c, g) = (Rc::clone(&c), Rc::clone(&g));
        paineis.on_duplicate(move |origem, lugar| {
            c.borrow_mut().push(origem.key_str());
            // O app pode entregar depois: guarda o lugar para um momento posterior.
            *g.borrow_mut() = Some(lugar);
        })
    });
    p.app.click("paineis-terminal-web-01");
    p.app.key_with(Key::Char('d'), ALT_SHIFT);
    assert_eq!(*chamadas.borrow(), vec!["web-01".to_owned()]);
    assert_eq!(p.forma(), "[0]", "o gancho decide: nada se divide sozinho");

    let lugar = guardado
        .borrow_mut()
        .take()
        .expect("o lugar chegou ao gancho");
    lugar.place(
        Session::new(Loopback::new().echo())
            .key("copia")
            .name("copia"),
    );
    p.app.frame();
    assert_eq!(p.forma(), "row[0,1]");
    assert_eq!(p.layout.get_untracked().sessions().len(), 2);
}

#[test]
fn atalhos_de_foco_andam_na_grade_e_nao_saem_da_borda() {
    let mut p = paineis(4, Theme::Light);
    p.muda(|l| l.grid(2, 2));
    p.app.click("paineis-terminal-web-01");
    let ativo = |p: &Paineis| p.layout.get_untracked().active();
    for (tecla, esperado) in [
        (Key::Right, 1),
        (Key::Down, 3),
        (Key::Left, 2),
        (Key::Up, 0),
        (Key::Left, 0),
        (Key::Up, 0),
    ] {
        p.app.key_with(tecla, Modifiers::ALT);
        assert_eq!(ativo(&p), esperado, "Alt+{tecla:?}");
        assert!(p.foco_no_ativo(), "Alt+{tecla:?}: o foco acompanha");
    }
}

#[test]
fn atalho_de_redimensionar_muda_a_fracao() {
    let mut p = paineis(4, Theme::Light);
    p.muda(|l| l.grid(2, 2));
    p.app.click("paineis-terminal-web-01");
    let antes = p.app.rect("paineis-painel-0-quadro").width();
    p.app.key_with(Key::Right, ALT_SHIFT);
    let depois = p.app.rect("paineis-painel-0-quadro").width();
    assert!(depois > antes + 10.0, "{antes} -> {depois}");
    p.app.key_with(Key::Left, ALT_SHIFT);
    p.app.key_with(Key::Left, ALT_SHIFT);
    assert!(p.app.rect("paineis-painel-0-quadro").width() < antes - 10.0);
    assert!(p.foco_no_ativo());
}

#[test]
fn atalho_de_fechar_o_painel_avisa_o_app() {
    let mut p = paineis(4, Theme::Light);
    p.muda(|l| l.grid(2, 2));
    p.app.click("paineis-terminal-web-01");
    p.app
        .key_with(Key::Char('w'), Modifiers::CTRL | Modifiers::SHIFT);
    assert_eq!(p.forma(), "col[0,row[1,2]]");
    assert_eq!(*p.fechadas.borrow(), ["web-01"]);
    assert!(p.foco_no_ativo());
}

#[test]
fn atalhos_podem_ser_desligados_e_trocados() {
    let mut p = paineis_com(1, Theme::Light, |paineis| {
        paineis.shortcuts(PaneShortcuts::none())
    });
    p.app.click("paineis-terminal-web-01");
    p.app.key_with(Key::Char('-'), ALT_SHIFT);
    assert_eq!(p.forma(), "[0]", "sem atalhos");

    let mut p = paineis_com(1, Theme::Light, |paineis| {
        paineis.shortcuts(PaneShortcuts {
            zoom: vec![Shortcut::ctrl_shift('z')],
            ..PaneShortcuts::none()
        })
    });
    p.app.click("paineis-terminal-web-01");
    p.app
        .key_with(Key::Char('z'), Modifiers::CTRL | Modifiers::SHIFT);
    assert_eq!(p.layout.get_untracked().zoomed(), Some(0));
    p.app
        .key_with(Key::Char('z'), Modifiers::CTRL | Modifiers::SHIFT);
    assert_eq!(p.layout.get_untracked().zoomed(), None);
}

#[test]
fn dividir_usa_o_new_session_do_app() {
    let criadas = Rc::new(Cell::new(0));
    let c = Rc::clone(&criadas);
    let mut p = paineis_com(1, Theme::Light, move |paineis| {
        let c = Rc::clone(&c);
        paineis.new_session(move || {
            c.set(c.get() + 1);
            let nome = format!("novo-{}", c.get());
            Session::new(Loopback::new().echo())
                .key(nome.clone())
                .name(nome)
        })
    });
    p.app.click("paineis-terminal-web-01");
    p.app.key_with(Key::Char('+'), ALT_SHIFT);
    assert_eq!(criadas.get(), 1);
    assert_eq!(p.forma(), "row[0,1]");
    assert!(p.app.visible("paineis-terminal-novo-1"));
    assert!(p.foco_no_ativo());
}

/// 3 sessões em duas colunas: o painel 0 com a web-01 e o 1 com a web-02 e a web-03. A web-02
/// tem 50 linhas de histórico.
fn duas_colunas(tema: Theme) -> Paineis {
    let mut p = paineis(3, tema);
    for linha in 0..50 {
        p.sessoes[1].feed(format!("linha {linha}\r\n").as_bytes());
    }
    p.muda(|l| l.grid(2, 1));
    assert_eq!(p.abas(0), ["web-01"]);
    assert_eq!(p.abas(1), ["web-02", "web-03"]);
    p
}

#[test]
fn mover_aba_mantem_sessao() {
    let mut p = duas_colunas(Theme::Light);
    let historico = p.sessoes[1].history();
    let web02 = p.sessoes[1];
    let intacta = |p: &Paineis| {
        assert!(!web02.closed());
        assert!(web02.is_alive());
        assert_eq!(web02.history(), historico);
        assert!(p.fechadas.borrow().is_empty());
    };

    // Por código.
    p.muda(|l| l.move_tab(web02, 0));
    assert_eq!(p.abas(0), ["web-01", "web-02"]);
    assert_eq!(p.abas(1), ["web-03"]);
    intacta(&p);

    // Pelo menu da aba: "Mover para o painel" ▸ "Painel 2".
    p.app.context_menu("paineis-painel-0-web-02");
    assert!(p.app.is_open("paineis-painel-0-web-02-menu"));
    p.app.click("paineis-painel-0-web-02-menu-mover");
    assert!(p.app.is_open("paineis-painel-0-web-02-menu-mover-lista"));
    p.app.click("paineis-painel-0-web-02-menu-mover-1");
    assert_eq!(p.abas(0), ["web-01"]);
    assert_eq!(p.abas(1), ["web-03", "web-02"]);
    intacta(&p);
    assert!(p.foco_no_ativo(), "o foco vai ao terminal do painel novo");

    // Pelo arrasto para a faixa do outro painel, antes da primeira aba.
    let primeira = p.app.rect("paineis-painel-0-web-01");
    let destino = Point::new(primeira.x0 + primeira.width() * 0.25, primeira.center().y);
    p.app.drag("paineis-painel-1-web-02", destino);
    assert_eq!(p.abas(0), ["web-02", "web-01"]);
    assert_eq!(p.abas(1), ["web-03"]);
    intacta(&p);

    // Pelo arrasto para a área do terminal do outro painel: vai para o fim dele.
    let terminal = p.app.rect(&p.terminal_do_painel(1));
    p.app.drag("paineis-painel-0-web-02", terminal.center());
    assert_eq!(p.abas(0), ["web-01"]);
    assert_eq!(p.abas(1), ["web-03", "web-02"]);
    assert_eq!(p.layout.get_untracked().active(), 1);
    intacta(&p);
}

#[test]
fn menu_da_aba_abre_painel_novo_reinicia_e_fecha() {
    let mut p = duas_colunas(Theme::Light);
    // "Novo painel abaixo": a web-03 vai para um painel embaixo do 1.
    p.app.context_menu("paineis-painel-1-web-03");
    p.app.click("paineis-painel-1-web-03-menu-abaixo");
    assert_eq!(p.forma(), "row[0,col[1,2]]");
    assert_eq!(p.abas(1), ["web-02"]);
    assert_eq!(p.abas(2), ["web-03"]);
    assert!(p.foco_no_ativo());

    // "Novo painel à direita".
    p.app.context_menu("paineis-painel-0-web-01");
    p.app.click("paineis-painel-0-web-01-menu-direita");
    assert_eq!(p.forma(), "row[0,1,col[2,3]]");
    assert_eq!(p.abas(0), Vec::<String>::new());
    assert_eq!(p.abas(1), ["web-01"]);
    assert!(p.app.visible("paineis-painel-0-vazio"));

    // "Fechar aba" avisa o app e tira a sessão do layout.
    p.app.context_menu("paineis-painel-3-web-03");
    p.app.click("paineis-painel-3-web-03-menu-fechar");
    assert_eq!(*p.fechadas.borrow(), ["web-03"]);
    assert_eq!(p.abas(3), Vec::<String>::new());
}

#[test]
fn restart_pelo_menu() {
    let mut p = duas_colunas(Theme::Light);
    p.handles[0].close(Some("o servidor caiu".to_owned()));
    p.app.frame();
    assert!(p.sessoes[0].closed());
    assert!(
        p.app
            .terminal_text("paineis-terminal-web-01")
            .contains("o servidor caiu")
    );

    p.app.context_menu("paineis-painel-0-web-01");
    p.app.click("paineis-painel-0-web-01-menu-reiniciar");
    assert!(!p.sessoes[0].closed());
    assert_eq!(p.handles[0].reopened(), 1);
    assert!(p.sessoes[0].history().contains("o servidor caiu"));

    p.app.click("paineis-terminal-web-01");
    p.app.type_text("ok");
    p.app.frame();
    assert!(
        p.app
            .terminal_text("paineis-terminal-web-01")
            .contains("ok")
    );
}

#[test]
fn snapshot_da_aba_arrastada() {
    for tema in TEMAS {
        let mut p = paineis(4, tema);
        p.muda(|l| l.grid(2, 1));
        let web02 = p.sessoes[1];
        p.muda(|l| l.move_tab(web02, 0));
        assert_eq!(p.abas(0), ["web-01", "web-02"]);
        let destino = p.app.rect(&p.terminal_do_painel(1)).center();
        p.app.press("paineis-painel-0-web-02").move_to(destino);
        assert_eq!(p.app.drag_label().as_deref(), Some("web-02"));
        p.app
            .assert_snapshot(&format!("m23_aba_arrastada_{}", nome_do_tema(tema)));
        p.app.release();
        assert_eq!(p.abas(1), ["web-03", "web-04", "web-02"]);
    }
}

// ---- Fatia 9: o envio para vários e o colar ----

use tomni_ui::terminal::Broadcast;

/// `n` sessões em grade de 2 × 2 (a quinta, se houver, é a segunda aba do último painel) e o
/// signal do envio, que o `TerminalPanes` lê.
fn com_envio(n: usize, tema: Theme) -> (Paineis, Signal<Broadcast>) {
    let guarda = Rc::new(Cell::new(None));
    let g = Rc::clone(&guarda);
    let mut p = paineis_com(n, tema, move |paineis| {
        let envio = signal(Broadcast::None);
        g.set(Some(envio));
        paineis.broadcast(envio)
    });
    p.muda(|l| l.grid(2, 2));
    let envio = guarda.get().unwrap();
    (p, envio)
}

impl Paineis {
    /// O que cada sessão escreveu desde a última chamada, como texto.
    fn escritos(&self) -> Vec<String> {
        self.handles
            .iter()
            .map(|h| String::from_utf8_lossy(&h.take_written()).into_owned())
            .collect()
    }
}

#[test]
fn envio_nos_quatro_modos() {
    let (mut p, envio) = com_envio(5, Theme::Light);
    assert_eq!(p.abas(3), ["web-04", "web-05"]);
    p.app.click("paineis-terminal-web-01");
    p.escritos();

    // Sem envio: só a sessão com o foco.
    p.app.type_text("a");
    assert_eq!(p.escritos(), ["a", "", "", "", ""]);

    // Visíveis: a aba escolhida de cada painel (a web-05 está escondida).
    envio.set(Broadcast::Visible);
    p.app.type_text("b");
    assert_eq!(p.escritos(), ["b", "b", "b", "b", ""]);

    // Todas.
    envio.set(Broadcast::All);
    p.app.type_text("c");
    assert_eq!(p.escritos(), ["c", "c", "c", "c", "c"]);

    // Um grupo: com o foco dentro dele, as duas; com o foco fora, só a sessão.
    envio.set(Broadcast::Group(vec!["web-01".into(), "web-03".into()]));
    p.app.type_text("d");
    assert_eq!(p.escritos(), ["d", "", "d", "", ""]);
    p.app.click("paineis-terminal-web-02");
    p.app.type_text("e");
    assert_eq!(p.escritos(), ["", "e", "", "", ""]);

    // Uma sessão encerrada não recebe.
    envio.set(Broadcast::All);
    p.handles[2].close(None);
    p.app.frame();
    p.app.type_text("f");
    assert_eq!(p.escritos(), ["f", "f", "", "f", "f"]);
}

#[test]
fn envio_respeita_o_modo_de_cada_sessao() {
    let (mut p, envio) = com_envio(4, Theme::Light);
    envio.set(Broadcast::All);
    p.app.click("paineis-terminal-web-01");
    p.escritos();

    // Uma consulta de posição do cursor (DSR) só é respondida pela sessão que a recebeu.
    p.sessoes[0].feed(b"\x1b[6n");
    p.app.frame();
    let respostas = p.escritos();
    assert!(
        respostas[0].starts_with("\x1b[") && respostas[0].ends_with('R'),
        "{respostas:?}"
    );
    assert_eq!(&respostas[1..], ["", "", ""]);

    // O DECCKM de uma sessão não vale para as outras: cada uma codifica a seta do seu jeito.
    p.sessoes[1].feed(b"\x1b[?1h");
    p.app.frame();
    p.app.key(Key::Up);
    assert_eq!(p.escritos(), ["\x1b[A", "\x1bOA", "\x1b[A", "\x1b[A"]);
}

#[test]
fn colar_com_envio_pede_confirmacao() {
    let (mut p, envio) = com_envio(4, Theme::Light);
    let colar = Modifiers::CTRL.with(Modifiers::SHIFT);
    p.app.click("paineis-terminal-web-01");
    p.escritos();

    // Sem envio, uma linha cola direto.
    p.app.clipboard().write("ls".into());
    p.app.key_with(Key::Char('v'), colar);
    assert!(!p.app.is_open("paineis-terminal-web-01-paste"));
    assert_eq!(p.escritos(), ["ls", "", "", ""]);

    // Com o envio para as quatro, até uma linha pergunta.
    envio.set(Broadcast::Visible);
    p.app.key_with(Key::Char('v'), colar);
    assert!(p.app.is_open("paineis-terminal-web-01-paste"));
    assert!(
        p.app
            .text("paineis-terminal-web-01-paste-mensagem")
            .starts_with("Colar em 4 terminais?")
    );
    assert_eq!(p.escritos(), ["", "", "", ""]);

    // Cancelar não escreve nada.
    p.app.click("paineis-terminal-web-01-paste-close");
    assert!(!p.app.is_open("paineis-terminal-web-01-paste"));
    assert_eq!(p.escritos(), ["", "", "", ""]);

    // Confirmar escreve nas quatro.
    p.app.key_with(Key::Char('v'), colar);
    p.app.click("paineis-terminal-web-01-paste-primary");
    assert_eq!(p.escritos(), ["ls", "ls", "ls", "ls"]);

    // Voltando ao normal, várias linhas mostram o aviso de sempre.
    envio.set(Broadcast::None);
    p.app.clipboard().write("cd /tmp\nrm -rf *\n".into());
    p.app.key_with(Key::Char('v'), colar);
    assert!(
        p.app
            .text("paineis-terminal-web-01-paste-mensagem")
            .starts_with("Você está prestes a colar")
    );
    p.app.click("paineis-terminal-web-01-paste-close");
}

#[test]
fn snapshot_do_envio_e_da_sessao_encerrada() {
    for tema in TEMAS {
        let (mut p, envio) = com_envio(4, tema);
        for (i, sessao) in p.sessoes.iter().enumerate() {
            sessao.feed(
                format!("user@web-{:02}:~$ uptime\r\n 10:42 up 3 days\r\n", i + 1).as_bytes(),
            );
        }
        p.app.click("paineis-terminal-web-01");
        envio.set(Broadcast::Visible);
        p.app.frame();
        p.app
            .assert_snapshot(&format!("m23_envio_visiveis_{}", nome_do_tema(tema)));

        envio.set(Broadcast::None);
        p.handles[3].close(Some("o servidor caiu".to_owned()));
        p.app.frame();
        assert!(
            p.app
                .terminal_text("paineis-terminal-web-04")
                .contains("[conexão encerrada: o servidor caiu]")
        );
        p.app
            .assert_snapshot(&format!("m23_sessao_encerrada_{}", nome_do_tema(tema)));
    }
}

#[test]
fn ferramentas() {
    let (mut p, envio) = com_envio(5, Theme::Light);
    p.app.click("paineis-terminal-web-02");

    // O `inspect`: layout, abas de cada painel, painel ativo, zoom e envio.
    let arvore = p.app.tree();
    let no = arvore
        .find("paineis")
        .expect("o TerminalPanes está na árvore");
    assert_eq!(no.kind, "terminal_panes");
    assert_eq!(no.layout.as_deref(), Some("col[row[0,1],row[2,3]]"));
    assert_eq!(no.active_pane, Some(1));
    assert_eq!(no.zoomed, None);
    assert_eq!(no.broadcast.as_deref(), Some("none"));
    let paineis = no.panes.as_ref().unwrap();
    assert_eq!(paineis.len(), 4);
    assert_eq!(
        paineis[3].tabs,
        [
            ["web-04".to_owned(), "web-04".to_owned()],
            ["web-05".to_owned(), "web-05".to_owned()]
        ]
    );
    assert_eq!(paineis[3].selected.as_deref(), Some("web-04"));

    envio.set(Broadcast::Group(vec!["web-01".into(), "web-03".into()]));
    p.muda(|l| l.zoom(2));
    let info = p.app.panes("paineis");
    assert_eq!(info.broadcast, "group:web-01,web-03");
    assert_eq!(info.zoomed, Some(2));
    assert_eq!(info.active, 2);
    assert_eq!(info.layout, "col[row[0,1],row[2,3]]");
    assert_eq!(info.panes[0].tabs, [("web-01".into(), "web-01".into())]);
    assert_eq!(info.panes[1].selected.as_deref(), Some("web-02"));
    let arvore = p.app.tree();
    let no = arvore.find("paineis").unwrap();
    assert_eq!(no.zoomed, Some(2));
    assert_eq!(no.broadcast.as_deref(), Some("group:web-01,web-03"));

    // O texto de uma sessão, à mostra ou não.
    p.sessoes[4].feed(b"escondida $ ");
    p.app.frame();
    assert_eq!(p.app.session_text("web-05"), "escondida $");
    assert!(p.app.session_history("web-05").contains("escondida $"));
    assert_eq!(p.app.session_text("web-01"), "");
}

#[test]
#[should_panic(expected = "nenhuma sessão com a chave \"nao-existe\"")]
fn session_text_com_chave_que_nao_existe() {
    let p = paineis(1, Theme::Light);
    p.app.session_text("nao-existe");
}

#[test]
#[should_panic(expected = "\"div\" não é um TerminalPanes")]
fn panes_de_outro_widget() {
    let (app, _) = fracao_do_app(0.5);
    app.panes("div");
}
