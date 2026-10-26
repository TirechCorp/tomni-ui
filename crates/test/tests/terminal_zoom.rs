//! O zoom da fonte do terminal pela roda (pedido pelo Tirech Omni): com `on_font_zoom`, o
//! Ctrl+roda chama o app com os passos e não rola o histórico, não manda setas nem vai ao
//! programa (mesmo com o mouse pedido); o touchpad soma os pixels; o app devolve o tamanho por
//! `font_size` e a grade (colunas e linhas) muda e a sessão é avisada do tamanho novo.

use std::cell::RefCell;
use std::rc::Rc;

use tomni_ui::kurbo::{Point, Vec2};
use tomni_ui::prelude::*;
use tomni_ui::terminal::{
    FONT_ZOOM_WHEEL, LoopbackHandle, TERMINAL_FONT_MAX, TERMINAL_FONT_MIN, TerminalVisual,
};
use tomni_ui::{Look, Modifiers, SCROLL_LINE};
use tomni_ui_test::TestApp;

/// Um terminal que acompanha o tamanho do widget, com `configurar` aplicado a ele.
fn app_com(configurar: impl FnOnce(Terminal) -> Terminal + 'static) -> (TestApp, LoopbackHandle) {
    let conexao = Loopback::new();
    let handle = conexao.handle();
    let app = TestApp::new(move || {
        Form::new("Console").child(configurar(Terminal::new(conexao).id("console").grow(1)))
    })
    .size(780, 560);
    (app, handle)
}

/// Um terminal que guarda os passos que o gancho recebeu.
fn app_com_passos() -> (TestApp, LoopbackHandle, Rc<RefCell<Vec<i32>>>) {
    let passos: Rc<RefCell<Vec<i32>>> = Rc::default();
    let destino = passos.clone();
    let (app, handle) = app_com(move |t| t.on_font_zoom(move |p| destino.borrow_mut().push(p)));
    (app, handle, passos)
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

fn centro(app: &TestApp) -> Point {
    app.rect("console").center()
}

fn escrito(handle: &LoopbackHandle) -> String {
    String::from_utf8(handle.take_written()).unwrap()
}

/// O quanto a vista está rolada para cima, em linhas.
fn rolagem(app: &TestApp) -> usize {
    app.tree().find("console").unwrap().history.unwrap()[1]
}

fn com(app: &mut TestApp, mods: Modifiers) {
    app.ui().set_modifiers(mods);
}

/// Uma volta do mouse com o Ctrl: `voltas` negativas sobem (aumentam a fonte).
fn ctrl_roda(app: &mut TestApp, voltas: f64) {
    let ponto = centro(app);
    com(app, Modifiers::CTRL);
    app.wheel(ponto, Vec2::new(0.0, voltas * FONT_ZOOM_WHEEL));
    com(app, Modifiers::NONE);
}

fn historico(app: &mut TestApp) {
    let linhas: String = (0..200).map(|n| format!("linha {n}\r\n")).collect();
    app.feed("console", &linhas);
}

#[test]
fn ctrl_roda_chama_o_gancho_e_nao_rola() {
    let (mut app, handle, passos) = app_com_passos();
    historico(&mut app);
    // Sobe duas linhas do histórico, sem Ctrl: a roda rola e o gancho não é chamado.
    let ponto = centro(&app);
    app.wheel(ponto, Vec2::new(0.0, -2.0 * SCROLL_LINE));
    assert_eq!(rolagem(&app), 2);
    assert!(passos.borrow().is_empty());
    let _ = escrito(&handle);

    // Ctrl + roda para cima aumenta (passo positivo); para baixo diminui.
    ctrl_roda(&mut app, -1.0);
    ctrl_roda(&mut app, 1.0);
    ctrl_roda(&mut app, -3.0);
    assert_eq!(*passos.borrow(), [1, -1, 3]);
    assert_eq!(rolagem(&app), 2, "o zoom não rola o histórico");
    assert_eq!(escrito(&handle), "", "nem manda nada ao programa");
}

#[test]
fn ctrl_roda_nao_vai_ao_programa_que_pediu_o_mouse() {
    let (mut app, handle, passos) = app_com_passos();
    app.feed("console", "\x1b[?1049h\x1b[?1000h\x1b[?1006h");
    ctrl_roda(&mut app, -1.0);
    assert_eq!(*passos.borrow(), [1]);
    assert_eq!(escrito(&handle), "", "o programa não recebe o Ctrl+roda");
    // Sem o Ctrl, a roda continua indo ao programa.
    let ponto = centro(&app);
    app.wheel(ponto, Vec2::new(0.0, -SCROLL_LINE));
    assert!(escrito(&handle).starts_with("\x1b[<64;"));
    assert_eq!(*passos.borrow(), [1]);
}

#[test]
fn na_tela_alternativa_o_ctrl_roda_nao_manda_setas() {
    let (mut app, handle, passos) = app_com_passos();
    app.feed("console", "\x1b[?1049h");
    ctrl_roda(&mut app, 2.0);
    assert_eq!(*passos.borrow(), [-2]);
    assert_eq!(escrito(&handle), "");
}

#[test]
fn o_touchpad_soma_os_pixels_ate_formar_um_passo() {
    let (mut app, _, passos) = app_com_passos();
    let ponto = centro(&app);
    com(&mut app, Modifiers::CTRL);
    // 11 pedaços de 4 px: ainda não formam um passo.
    for _ in 0..11 {
        app.wheel(ponto, Vec2::new(0.0, -4.0));
    }
    assert!(passos.borrow().is_empty());
    // O 12º completa os 48 px: um passo só, e não vinte.
    app.wheel(ponto, Vec2::new(0.0, -4.0));
    assert_eq!(*passos.borrow(), [1]);
    // 20 pedaços de 4 px (80 px) valem um passo e sobram 32 px.
    for _ in 0..20 {
        app.wheel(ponto, Vec2::new(0.0, -4.0));
    }
    assert_eq!(*passos.borrow(), [1, 1]);
    // Os 32 px que sobraram e mais 16 completam o seguinte.
    app.wheel(ponto, Vec2::new(0.0, -16.0));
    com(&mut app, Modifiers::NONE);
    assert_eq!(*passos.borrow(), [1, 1, 1]);
}

#[test]
fn so_o_ctrl_sozinho_faz_zoom() {
    let (mut app, _, passos) = app_com_passos();
    historico(&mut app);
    let ponto = centro(&app);
    for mods in [
        Modifiers::SHIFT,
        Modifiers::CTRL.with(Modifiers::SHIFT),
        Modifiers::CTRL.with(Modifiers::ALT),
    ] {
        com(&mut app, mods);
        app.wheel(ponto, Vec2::new(0.0, -FONT_ZOOM_WHEEL));
    }
    com(&mut app, Modifiers::NONE);
    assert!(passos.borrow().is_empty());
}

#[test]
fn sem_o_gancho_o_ctrl_roda_faz_o_que_fazia() {
    let (mut app, handle) = app_com(|t| t);
    historico(&mut app);
    let ponto = centro(&app);
    com(&mut app, Modifiers::CTRL);
    app.wheel(ponto, Vec2::new(0.0, -3.0 * SCROLL_LINE));
    com(&mut app, Modifiers::NONE);
    assert_eq!(rolagem(&app), 3);
    assert_eq!(escrito(&handle), "");
}

#[test]
fn o_zoom_refaz_a_grade_e_avisa_a_sessao() {
    let tamanho = signal(16.0_f64);
    let (mut app, handle) = app_com(move |t| {
        t.font_size(move || tamanho.get())
            .on_font_zoom(move |passos| {
                let novo = tamanho.get_untracked() + f64::from(passos);
                tamanho.set(novo.clamp(f64::from(TERMINAL_FONT_MIN), f64::from(TERMINAL_FONT_MAX)));
            })
    });
    let (cols, linhas) = handle.size().expect("a sessão recebeu o tamanho");
    let largura = visual(&app).metrics().width;

    // Aumentar a fonte: menos colunas e linhas, células maiores, SIGWINCH com o tamanho novo.
    ctrl_roda(&mut app, -8.0);
    assert_eq!(tamanho.get_untracked(), 24.0);
    let (cols2, linhas2) = handle.size().expect("a sessão recebeu o tamanho novo");
    assert!(
        cols2 < cols && linhas2 < linhas,
        "{cols}x{linhas} → {cols2}x{linhas2}"
    );
    assert!(visual(&app).metrics().width > largura);

    // Diminuir: mais colunas e linhas que no tamanho original.
    ctrl_roda(&mut app, 12.0);
    assert_eq!(tamanho.get_untracked(), 12.0);
    let (cols3, linhas3) = handle.size().unwrap();
    assert!(cols3 > cols && linhas3 > linhas);

    // Os limites de 6 e 72 valem.
    ctrl_roda(&mut app, 50.0);
    assert_eq!(tamanho.get_untracked(), f64::from(TERMINAL_FONT_MIN));
    ctrl_roda(&mut app, -100.0);
    assert_eq!(tamanho.get_untracked(), f64::from(TERMINAL_FONT_MAX));
}
