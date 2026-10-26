//! Marco M27: várias janelas no mesmo processo. Signals compartilhados, a sessão que passa de
//! uma janela para outra (arrastando e por `move_session`), o tear-off, e o fechar.

mod comum;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use tomni_ui::prelude::*;
use tomni_ui::terminal::{Loopback, PaneLayout, Session, TerminalPanes, move_session};
use tomni_ui::{WindowHandle, active_window, on_close_requested, open_window};
use tomni_ui_test::TestApp;

#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/janelas.rs"]
mod janelas;

fn sessao(chave: &str) -> Session {
    let sessao = Session::new(Loopback::new().echo()).key(chave).name(chave);
    sessao.feed(format!("Conectado a {chave}.\r\n$ ").as_bytes());
    sessao
}

/// Os títulos das abas do painel 0 de `paineis`.
fn abas(app: &TestApp) -> Vec<String> {
    app.panes("paineis").panes[0]
        .tabs
        .iter()
        .map(|(chave, _)| chave.clone())
        .collect()
}

#[test]
fn duas_janelas_compartilham_signals() {
    let contador = Rc::new(Cell::new(None));
    let guarda = Rc::clone(&contador);
    let mut app = TestApp::new(move || {
        let n = signal(0);
        guarda.set(Some(n));
        Column::new()
            .child(Text::new(move || format!("n = {}", n.get())).id("n"))
            .child(Button::new("Abrir").id("abrir").on_click(move || {
                open_window(move || {
                    Column::new()
                        .child(Text::new(move || format!("n = {}", n.get())).id("n"))
                        .child(
                            Button::new("Mais")
                                .id("mais")
                                .on_click(move || n.update(|v| *v += 1)),
                        )
                })
                .title("Segunda")
                .size(300, 200)
                .open();
            }))
    })
    .size(400, 300);
    app.click("abrir");
    app.frame();
    let janelas = app.windows();
    assert_eq!(janelas.len(), 2);
    assert_eq!(janelas[0], WindowHandle::main());
    assert_eq!(janelas[1].title(), "Segunda");

    app.switch_to(janelas[1]);
    app.click("mais");
    assert_eq!(app.text("n"), "n = 1");
    app.switch_to(janelas[0]);
    app.frame();
    assert_eq!(app.text("n"), "n = 1");
    assert_eq!(contador.get().unwrap().get_untracked(), 1);
}

#[test]
fn a_janela_ativa_e_reativa() {
    let ativa = Rc::new(RefCell::new(None));
    let guarda = Rc::clone(&ativa);
    let mut app = TestApp::new(move || {
        let a = active_window();
        *guarda.borrow_mut() = Some(a);
        Button::new("Abrir").id("abrir").on_click(|| {
            open_window(|| Text::new("outra")).open();
        })
    });
    app.click("abrir");
    app.frame();
    let outra = app.windows()[1];
    app.set_active_window(outra);
    assert_eq!(ativa.borrow().unwrap().get_untracked(), outra);
    app.set_active_window(WindowHandle::main());
    assert_eq!(
        ativa.borrow().unwrap().get_untracked(),
        WindowHandle::main()
    );
}

#[test]
fn a_sessao_passa_de_janela_arrastando_sem_reconectar() {
    let mut app = TestApp::new(janelas::principal).size(900, 600);
    app.click("nova-janela");
    app.frame();
    let [principal, outra] = app.windows()[..] else {
        panic!("esperava duas janelas");
    };
    let sessao = Session::find("web3").unwrap();
    sessao.feed(b"uptime\r\n");
    app.switch_to(outra);
    assert_eq!(abas(&app), ["web3"]);
    app.switch_to(principal);
    assert_eq!(abas(&app), ["web1", "web2"]);

    app.switch_to(outra);
    app.drag_tab_to_window("paineis-painel-0-web3", principal, "paineis-painel-0-web1");
    app.frame();

    // A aba chegou, na posição onde caiu, e a janela que ficou sem abas fechou.
    assert_eq!(app.current_window(), principal);
    assert_eq!(abas(&app), ["web3", "web1", "web2"]);
    assert!(!outra.is_open());
    // A mesma sessão, com o histórico: ninguém reconectou.
    assert_eq!(Session::find("web3"), Some(sessao));
    assert!(app.session_text("web3").contains("uptime"));
    assert!(app.session_history("web3").contains("Conectado a web3"));
    for chave in ["web1", "web2", "web3"] {
        Session::find(chave).unwrap().dispose();
    }
}

/// Duas janelas, cada uma com os painéis de um layout do mesmo grupo.
fn dois_layouts() -> (TestApp, Signal<PaneLayout>, Signal<PaneLayout>) {
    let guarda = Rc::new(Cell::new(None));
    let g = Rc::clone(&guarda);
    let app = TestApp::new(move || {
        let a = signal(PaneLayout::with([sessao("ms1"), sessao("ms2")]));
        let b = signal(PaneLayout::with([sessao("ms3")]));
        g.set(Some((a, b)));
        Column::new()
            .child(Button::new("Abrir").id("abrir").on_click(move || {
                open_window(move || {
                    Column::new()
                        .child(TerminalPanes::bind(b).id("paineis").drag_group("g").grow(1))
                })
                .open();
            }))
            .child(TerminalPanes::bind(a).id("paineis").drag_group("g").grow(1))
    })
    .size(800, 500);
    let (a, b) = guarda.get().unwrap();
    (app, a, b)
}

#[test]
fn move_session_leva_a_sessao_viva_para_outro_layout() {
    let (mut app, a, b) = dois_layouts();
    app.click("abrir");
    app.frame();
    let [principal, outra] = app.windows()[..] else {
        panic!("esperava duas janelas");
    };
    let sessao = Session::find("ms1").unwrap();
    sessao.feed(b"historico\r\n");

    move_session(sessao, b, 0);
    app.frame();
    assert_eq!(abas(&app), ["ms2"]);
    app.switch_to(outra);
    assert_eq!(abas(&app), ["ms3", "ms1"]);
    assert_eq!(Session::find("ms1"), Some(sessao));
    assert!(app.session_text("ms1").contains("historico"));
    assert_eq!(a.with_untracked(|l| l.sessions().len()), 1);
    assert_eq!(b.with_untracked(|l| l.sessions().len()), 2);
    app.switch_to(principal);
    for chave in ["ms1", "ms2", "ms3"] {
        Session::find(chave).unwrap().dispose();
    }
}

/// Os pontos de tear-off registrados.
type Soltos = Rc<RefCell<Vec<Option<(i32, i32)>>>>;

/// Uma janela com os painéis de `layout`, que registram em `solto` o ponto do tear-off.
fn com_tear_off(solto: &Soltos) -> impl FnOnce() -> Column {
    let solto = Rc::clone(solto);
    move || {
        let layout = signal(PaneLayout::with([sessao("t1"), sessao("t2")]));
        Column::new().child(
            TerminalPanes::bind(layout)
                .id("paineis")
                .drag_group("g")
                .on_tear_off(move |_sessao, ponto| solto.borrow_mut().push(ponto))
                .grow(1),
        )
    }
}

#[test]
fn soltar_fora_chama_o_tear_off_com_ou_sem_posicao() {
    for (posicoes, esperado) in [(true, Some((10_000, 200))), (false, None)] {
        let solto = Rc::new(RefCell::new(Vec::new()));
        let mut app = TestApp::new(com_tear_off(&solto)).size(600, 400);
        if !posicoes {
            app = app.without_window_positions();
        }
        app.drag_tab_outside("paineis-painel-0-t1");
        assert_eq!(*solto.borrow(), [esperado]);
        // O tear-off não tira a aba: quem decide é o app.
        assert_eq!(abas(&app), ["t1", "t2"]);
        for chave in ["t1", "t2"] {
            Session::find(chave).unwrap().dispose();
        }
    }
}

#[test]
fn o_exemplo_abre_uma_janela_nova_com_a_aba_solta_fora() {
    for posicoes in [true, false] {
        let mut app = TestApp::new(janelas::principal).size(900, 600);
        if !posicoes {
            app = app.without_window_positions();
        }
        let sessao = Session::find("web1").unwrap();
        sessao.feed(b"ls\r\n");
        app.drag_tab_outside("paineis-painel-0-web1");
        app.frame();
        let [principal, nova] = app.windows()[..] else {
            panic!("esperava duas janelas");
        };
        assert_eq!(abas(&app), ["web2"]);
        app.switch_to(nova);
        assert_eq!(abas(&app), ["web1"]);
        assert_eq!(Session::find("web1"), Some(sessao));
        assert!(app.session_text("web1").contains("ls"));
        app.assert_lint();
        app.switch_to(principal);
        app.close_window(principal);
        assert!(app.exited());
        for chave in ["web1", "web2"] {
            Session::find(chave).unwrap().dispose();
        }
    }
}

#[test]
fn o_menu_da_aba_move_para_uma_janela_nova() {
    let mut app = TestApp::new(janelas::principal).size(900, 600);
    app.context_menu("paineis-painel-0-web2");
    app.click("janela-nova-web2");
    app.frame();
    assert_eq!(app.windows().len(), 2);
    assert_eq!(abas(&app), ["web1"]);
    app.switch_to(app.windows()[1]);
    assert_eq!(abas(&app), ["web2"]);
    for chave in ["web1", "web2"] {
        Session::find(chave).unwrap().dispose();
    }
}

/// Uma janela principal e uma secundária, cada uma com um `on_close_requested` que obedece a
/// `permitir`.
fn com_ganchos(permitir: &Rc<Cell<bool>>, chamados: &Rc<Cell<u32>>) -> TestApp {
    let (principal, outra) = (Rc::clone(permitir), Rc::clone(permitir));
    let (conta_principal, conta_outra) = (Rc::clone(chamados), Rc::clone(chamados));
    TestApp::new(move || {
        on_close_requested(move || {
            conta_principal.set(conta_principal.get() + 1);
            principal.get()
        });
        let (outra, conta_outra) = (Rc::clone(&outra), Rc::clone(&conta_outra));
        Button::new("Abrir").id("abrir").on_click(move || {
            let (outra, conta_outra) = (Rc::clone(&outra), Rc::clone(&conta_outra));
            open_window(move || {
                on_close_requested(move || {
                    conta_outra.set(conta_outra.get() + 1);
                    outra.get()
                });
                Text::new("outra")
            })
            .open();
        })
    })
}

#[test]
fn on_close_requested_pode_impedir_o_fechamento() {
    let permitir = Rc::new(Cell::new(false));
    let chamados = Rc::new(Cell::new(0));
    let mut app = com_ganchos(&permitir, &chamados);
    app.click("abrir");
    app.frame();
    let [principal, outra] = app.windows()[..] else {
        panic!("esperava duas janelas");
    };

    // Recusado: a janela fica.
    assert!(!app.close_window(outra));
    assert!(outra.is_open());
    assert_eq!(chamados.get(), 1);

    // Fechar a principal pergunta a todas; uma recusa basta.
    assert!(!app.close_window(principal));
    assert!(!app.exited());
    assert_eq!(app.windows().len(), 2);

    // Liberado, fechar a principal fecha todas e encerra o app.
    permitir.set(true);
    assert!(app.close_window(principal));
    assert!(app.exited());
    assert!(!outra.is_open());
    assert!(!principal.is_open());
}

#[test]
fn fechar_a_ultima_janela_encerra_o_app() {
    let permitir = Rc::new(Cell::new(true));
    let chamados = Rc::new(Cell::new(0));
    let mut app = com_ganchos(&permitir, &chamados);
    app.click("abrir");
    app.frame();
    let outra = app.windows()[1];
    // Fechar só a secundária não encerra.
    assert!(app.close_window(outra));
    assert!(!app.exited());
    assert_eq!(app.windows(), [WindowHandle::main()]);
    assert!(app.close_window(WindowHandle::main()));
    assert!(app.exited());
}

#[test]
fn item_visible_mostra_so_os_itens_que_existem() {
    let quantos = Rc::new(Cell::new(None));
    let guarda = Rc::clone(&quantos);
    let mut app = TestApp::new(move || {
        let n = signal(1usize);
        guarda.set(Some(n));
        let mut menu = MenuFlyout::new().item("fixo", "Fixo", || {}).separator();
        for i in 0..3 {
            menu = menu
                .item(format!("janela-{i}"), format!("Janela {i}"), || {})
                .item_visible(move || i < n.get());
        }
        Column::new().child(Button::new("Menu").id("menu").flyout(menu))
    })
    .size(400, 300);
    let n = quantos.get().unwrap();

    app.click("menu");
    assert!(app.visible("fixo"));
    assert!(app.visible("janela-0"));
    assert!(!app.visible("janela-1"));
    app.key(Key::Escape);

    n.set(3);
    app.frame();
    app.click("menu");
    assert!(app.visible("janela-1") && app.visible("janela-2"));
}

#[test]
fn a_geometria_da_janela_acompanha_a_posicao_e_o_tamanho() {
    let mut app = TestApp::new(|| {
        Button::new("Abrir").id("abrir").on_click(|| {
            open_window(|| Text::new("outra"))
                .size(300, 200)
                .position(40, 60)
                .open();
        })
    })
    .size(400, 300);
    let principal = WindowHandle::main().geometry().unwrap();
    assert_eq!(principal.size, (400, 300));
    app.click("abrir");
    app.frame();
    let outra = app.windows()[1];
    let geometria = outra.geometry().unwrap();
    assert_eq!(geometria.size, (300, 200));
    assert_eq!(geometria.position, Some((40, 60)));
    app.close_window(outra);
    assert_eq!(outra.geometry(), None);
}
