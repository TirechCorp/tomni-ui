//! O mouse do terminal no relato do xterm (pedido pelo Tirech Omni): o programa que liga o
//! `ESC[?1000h` (e o 9, o 1002 e o 1003) recebe a roda e os botões, na forma SGR, padrão,
//! UTF-8 ou urxvt; o Shift devolve o mouse ao terminal; sem o relato, a roda continua rolando o
//! histórico ou mandando setas na tela alternativa; o 1004 avisa o foco.

use std::cell::RefCell;
use std::rc::Rc;

use tomni_ui::kurbo::{Point, Vec2};
use tomni_ui::prelude::*;
use tomni_ui::terminal::{LinkHit, LoopbackHandle, TerminalVisual};
use tomni_ui::{Key, Look, Modifiers, PointerKind, SCROLL_LINE};
use tomni_ui_test::TestApp;

/// Um terminal de `cols` × `rows` com `Loopback` e um menu de contexto, e um botão depois dele.
fn app_com(
    cols: u16,
    rows: u16,
    configurar: impl FnOnce(Terminal) -> Terminal + 'static,
) -> (TestApp, LoopbackHandle) {
    let conexao = Loopback::new();
    let handle = conexao.handle();
    let app = TestApp::new(move || {
        let menu = MenuFlyout::new()
            .id("console-menu")
            .item("limpar", "Limpar", || {});
        Form::new("Console").child(
            Column::new()
                .padding(16)
                .gap(8)
                .child(configurar(
                    Terminal::new(conexao)
                        .id("console")
                        .cols_rows(cols, rows)
                        .paste_warning(false)
                        .context_flyout(menu),
                ))
                .child(Button::new("Depois").id("depois").height(44)),
        )
    })
    .size(1400, 560);
    (app, handle)
}

fn app() -> (TestApp, LoopbackHandle) {
    app_com(80, 24, |t| t)
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

/// Centro da célula (`col`, `linha`), a partir de 0, na janela.
fn celula(app: &TestApp, col: usize, linha: usize) -> Point {
    visual(app)
        .cell_rect(app.rect("console"), linha, col, 1)
        .center()
}

fn escrito(handle: &LoopbackHandle) -> String {
    String::from_utf8(handle.take_written()).unwrap()
}

fn rolagem(app: &TestApp) -> [usize; 2] {
    app.tree().find("console").unwrap().history.unwrap()
}

fn tem_selecao(app: &TestApp) -> bool {
    visual(app).selection().is_some()
}

/// Pressiona o botão principal em `ponto`, sem soltar.
fn pressiona(app: &mut TestApp, ponto: Point) {
    app.move_to(ponto);
    app.ui().pointer_down(ponto, PointerKind::Mouse, 1);
    app.frame();
}

fn solta(app: &mut TestApp, ponto: Point) {
    app.ui().pointer_up(ponto, PointerKind::Mouse);
    app.frame();
}

fn clique_do_meio(app: &mut TestApp, ponto: Point) {
    app.move_to(ponto);
    app.ui().middle_down(ponto);
    app.frame();
    app.ui().middle_up(ponto);
    app.frame();
}

fn com(app: &mut TestApp, mods: Modifiers) {
    app.ui().set_modifiers(mods);
}

#[test]
fn programa_em_tela_cheia_recebe_a_roda() {
    let (mut app, handle) = app();
    app.feed("console", "\x1b[?1049h\x1b[?1000h\x1b[?1006h");
    // A célula (5, 3), contada a partir de 1 como no relato.
    let ponto = celula(&app, 4, 2);
    app.wheel(ponto, Vec2::new(0.0, -SCROLL_LINE));
    assert_eq!(escrito(&handle), "\x1b[<64;5;3M", "a roda para cima é o 64");
    app.wheel(ponto, Vec2::new(0.0, 2.0 * SCROLL_LINE));
    assert_eq!(
        escrito(&handle),
        "\x1b[<65;5;3M\x1b[<65;5;3M",
        "uma linha da roda, um evento"
    );
    // A roda na horizontal é o 66 e o 67; com Ctrl, soma 16.
    app.wheel(ponto, Vec2::new(SCROLL_LINE, 0.0));
    assert_eq!(escrito(&handle), "\x1b[<67;5;3M");
    com(&mut app, Modifiers::CTRL);
    app.wheel(ponto, Vec2::new(-SCROLL_LINE, -SCROLL_LINE));
    com(&mut app, Modifiers::NONE);
    assert_eq!(escrito(&handle), "\x1b[<80;5;3M\x1b[<82;5;3M");
    // Sem setas: a roda não é mais o histórico de comandos do programa.
    assert!(!escrito(&handle).contains("\x1b[A"));
}

#[test]
fn botoes_vao_ao_programa_sem_selecao_nem_menu() {
    let (mut app, handle) = app();
    app.feed("console", "texto para selecionar\r\n\x1b[?1000h\x1b[?1006h");
    let ponto = celula(&app, 2, 1);
    app.click_at(ponto, 1);
    assert_eq!(escrito(&handle), "\x1b[<0;3;2M\x1b[<0;3;2m");

    // O arrastar não seleciona (e o 1000 não conta o movimento).
    app.drag(celula(&app, 0, 0), celula(&app, 6, 0));
    assert!(!tem_selecao(&app));
    assert_eq!(escrito(&handle), "\x1b[<0;1;1M\x1b[<0;7;1m");

    // O direito vai ao programa: não abre o menu nem cola.
    app.clipboard().write("colado".to_owned());
    app.right_click("console");
    assert!(!app.is_open("console-menu"));
    let escrito_direito = escrito(&handle);
    assert!(
        escrito_direito.starts_with("\x1b[<2;") && escrito_direito.ends_with('m'),
        "{escrito_direito:?}"
    );
    assert!(!escrito_direito.contains("colado"));

    // O do meio também, e o Ctrl soma 16 (o Alt 8, o Shift fica com o terminal).
    clique_do_meio(&mut app, ponto);
    assert_eq!(escrito(&handle), "\x1b[<1;3;2M\x1b[<1;3;2m");
    com(&mut app, Modifiers::CTRL.with(Modifiers::ALT));
    app.click_at(ponto, 1);
    com(&mut app, Modifiers::NONE);
    assert_eq!(escrito(&handle), "\x1b[<24;3;2M\x1b[<24;3;2m");
}

#[test]
fn shift_devolve_o_mouse_ao_terminal() {
    let (mut app, handle) = app();
    let mut saida = String::new();
    for i in 1..=60 {
        saida.push_str(&format!("linha {i}\r\n"));
    }
    app.feed("console", saida);
    app.feed("console", "\x1b[?1002h\x1b[?1006h");
    let (de, ate) = (celula(&app, 0, 0), celula(&app, 4, 0));

    // Sem o Shift, o arrastar vai ao programa.
    app.drag(de, ate);
    assert!(!tem_selecao(&app));
    assert_eq!(escrito(&handle), "\x1b[<0;1;1M\x1b[<32;5;1M\x1b[<0;5;1m");

    // Com o Shift, seleciona e copia, e nada vai ao programa.
    com(&mut app, Modifiers::SHIFT);
    app.drag(de, ate);
    com(&mut app, Modifiers::NONE);
    assert!(tem_selecao(&app));
    app.key_with(Key::Char('c'), Modifiers::CTRL.with(Modifiers::SHIFT));
    assert_eq!(app.clipboard().read().as_deref(), Some("linha"));
    assert_eq!(escrito(&handle), "");

    // A roda com Shift rola o histórico.
    com(&mut app, Modifiers::SHIFT);
    app.wheel(de, Vec2::new(0.0, -SCROLL_LINE));
    assert_eq!(rolagem(&app)[1], 1);
    assert_eq!(escrito(&handle), "");

    // O clique direito com Shift abre o menu; sem ele, vai ao programa.
    app.right_click("console");
    com(&mut app, Modifiers::NONE);
    assert!(app.is_open("console-menu"));
    assert_eq!(escrito(&handle), "");
    app.key(Key::Escape);
    app.right_click("console");
    assert!(!app.is_open("console-menu"));
    assert!(escrito(&handle).starts_with("\x1b[<2;"));
}

#[test]
fn arrastar_com_1002_e_movimento_com_1003() {
    let (mut app, handle) = app();
    app.feed("console", "\x1b[?1002h\x1b[?1006h");
    let inicio = celula(&app, 1, 1);
    pressiona(&mut app, inicio);
    // Andar dentro da mesma célula não conta.
    app.move_to(inicio + Vec2::new(1.0, 1.0));
    app.move_to(celula(&app, 4, 2));
    app.move_to(celula(&app, 4, 2) + Vec2::new(1.0, 0.0));
    // Fora da grade, vale a borda.
    app.move_to(Point::new(0.0, 0.0));
    solta(&mut app, Point::new(0.0, 0.0));
    assert_eq!(
        escrito(&handle),
        "\x1b[<0;2;2M\x1b[<32;5;3M\x1b[<32;1;1M\x1b[<0;1;1m"
    );
    // Com o 1002, o movimento sem botão não vai.
    app.move_to(celula(&app, 7, 3));
    assert_eq!(escrito(&handle), "");

    // Com o 1003, vai: o botão 3 mais 32.
    app.feed("console", "\x1b[?1003h");
    app.move_to(celula(&app, 8, 3));
    app.move_to(celula(&app, 8, 4));
    assert_eq!(escrito(&handle), "\x1b[<35;9;4M\x1b[<35;9;5M");
    // Desligar qualquer modo desliga o relato.
    app.feed("console", "\x1b[?1000l");
    app.move_to(celula(&app, 9, 4));
    app.click_at(celula(&app, 9, 4), 1);
    assert_eq!(escrito(&handle), "");
}

#[test]
fn forma_padrao_e_o_limite_de_223() {
    let (mut app, handle) = app_com(240, 4, |t| t.font_size(6.0));
    app.feed("console", "\x1b[?1000h");
    app.click_at(celula(&app, 10, 1), 1);
    // `ESC [ M`, o botão, a coluna e a linha somados a 32; o soltar é o botão 3.
    assert_eq!(
        handle.take_written(),
        b"\x1b[M\x20\x2b\x22\x1b[M\x23\x2b\x22".to_vec()
    );
    // A coluna 223 ainda cabe num byte; a 224 não, e o evento não vai.
    app.click_at(celula(&app, 222, 0), 1);
    assert_eq!(
        handle.take_written(),
        b"\x1b[M\x20\xff\x21\x1b[M\x23\xff\x21".to_vec()
    );
    app.click_at(celula(&app, 230, 0), 1);
    assert_eq!(handle.take_written(), Vec::<u8>::new());

    // O urxvt (1015) e o UTF-8 (1005) passam do limite.
    app.feed("console", "\x1b[?1015h");
    app.click_at(celula(&app, 230, 0), 1);
    assert_eq!(escrito(&handle), "\x1b[32;231;1M\x1b[35;231;1M");
    app.feed("console", "\x1b[?1005h");
    app.click_at(celula(&app, 230, 0), 1);
    assert_eq!(escrito(&handle), "\x1b[M \u{107}!\x1b[M#\u{107}!");
}

#[test]
fn x10_so_o_pressionar_sem_modificadores() {
    let (mut app, handle) = app();
    app.feed("console", "\x1b[?9h");
    let ponto = celula(&app, 0, 0);
    app.click_at(ponto, 1);
    assert_eq!(escrito(&handle), "\x1b[M !!", "sem o soltar");
    com(&mut app, Modifiers::CTRL);
    app.click_at(ponto, 1);
    com(&mut app, Modifiers::NONE);
    assert_eq!(escrito(&handle), "\x1b[M !!", "sem o Ctrl");
    app.drag(ponto, celula(&app, 5, 0));
    assert_eq!(escrito(&handle), "\x1b[M !!", "sem o arrastar");
}

#[test]
fn sem_relato_a_roda_rola_ou_manda_setas() {
    let (mut app, handle) = app();
    let mut saida = String::new();
    for i in 1..=60 {
        saida.push_str(&format!("linha {i}\r\n"));
    }
    app.feed("console", saida);
    let meio = app.rect("console").center();

    // Na principal, a roda rola o histórico.
    app.wheel(meio, Vec2::new(0.0, -SCROLL_LINE));
    assert_eq!(rolagem(&app)[1], 1);
    assert_eq!(escrito(&handle), "");

    // Na alternativa, manda setas.
    app.feed("console", "\x1b[?1049h");
    app.wheel(meio, Vec2::new(0.0, SCROLL_LINE));
    app.wheel(meio, Vec2::new(0.0, -SCROLL_LINE));
    assert_eq!(escrito(&handle), "\x1b[B\x1b[A");

    // Com o `ESC[?1007l`, a roda não manda nada na alternativa.
    app.feed("console", "\x1b[?1007l");
    app.wheel(meio, Vec2::new(0.0, -SCROLL_LINE));
    assert_eq!(escrito(&handle), "");
    app.feed("console", "\x1b[?1007h");
    app.wheel(meio, Vec2::new(0.0, -SCROLL_LINE));
    assert_eq!(escrito(&handle), "\x1b[A");
}

#[test]
fn barra_de_rolagem_fica_com_o_terminal() {
    let (mut app, handle) = app();
    let mut saida = String::new();
    for i in 1..=100 {
        saida.push_str(&format!("linha {i}\r\n"));
    }
    app.feed("console", saida);
    app.feed("console", "\x1b[?1000h\x1b[?1006h");
    let caixa = app.rect("console");
    // Acima do polegar (que está no fim): sobe uma página.
    let na_barra = Point::new(caixa.x1 - 4.0, caixa.y0 + 40.0);
    app.click_at(na_barra, 1);
    assert_eq!(escrito(&handle), "");
    assert!(rolagem(&app)[1] > 0, "a barra rolou");
    // O clique direito na barra também: abre o menu.
    app.move_to(na_barra);
    app.ui().secondary_down(na_barra);
    app.frame();
    app.ui().secondary_up(na_barra);
    app.frame();
    assert!(app.is_open("console-menu"));
    assert_eq!(escrito(&handle), "");
    // Na grade, o clique vai ao programa.
    app.key(Key::Escape);
    app.click_at(celula(&app, 0, 0), 1);
    assert_eq!(escrito(&handle), "\x1b[<0;1;1M\x1b[<0;1;1m");
}

#[test]
fn aviso_de_foco_com_1004() {
    let (mut app, handle) = app();
    app.click("console");
    app.feed("console", "\x1b[?1004h");
    assert_eq!(escrito(&handle), "");
    app.click("depois");
    assert_eq!(escrito(&handle), "\x1b[O");
    app.click("console");
    assert_eq!(escrito(&handle), "\x1b[I");
    app.feed("console", "\x1b[?1004l");
    app.click("depois");
    assert_eq!(escrito(&handle), "");
}

#[test]
fn com_relato_o_link_abre_com_ctrl_shift() {
    let aberto: Rc<RefCell<Vec<LinkHit>>> = Rc::default();
    let destino = aberto.clone();
    let (mut app, handle) = app_com(80, 24, move |t| {
        t.on_link(move |link| destino.borrow_mut().push(link.clone()))
    });
    app.feed(
        "console",
        "veja https://exemplo.com/a fim\r\n\x1b[?1000h\x1b[?1006h",
    );
    let na_url = celula(&app, 10, 0);
    // O Ctrl+clique é do programa.
    com(&mut app, Modifiers::CTRL);
    app.click_at(na_url, 1);
    com(&mut app, Modifiers::NONE);
    assert!(aberto.borrow().is_empty());
    assert_eq!(escrito(&handle), "\x1b[<16;11;1M\x1b[<16;11;1m");
    // O Ctrl+Shift+clique abre o link.
    com(&mut app, Modifiers::CTRL.with(Modifiers::SHIFT));
    app.click_at(na_url, 1);
    com(&mut app, Modifiers::NONE);
    assert_eq!(aberto.borrow().len(), 1);
    assert_eq!(aberto.borrow()[0].uri, "https://exemplo.com/a");
    assert_eq!(escrito(&handle), "");
}
