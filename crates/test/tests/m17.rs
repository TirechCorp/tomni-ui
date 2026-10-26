//! Marco M17: o `Terminal`. Snapshots nos 2 temas (cores, atributos, cursores, seleção, `htop`
//! e `vim` gravados, CJK), a emulação com saídas gravadas de programas de verdade, as teclas,
//! copiar e colar, o histórico, o redimensionamento e 10 MB de saída sem travar a interface.
//!
//! As gravações em `gravacoes/` são a saída real dos programas num pseudoterminal de 80 × 24,
//! feitas com `gravacoes/gravar.py` (o `htop` e o `top` só com os processos do `nobody`).

mod comum;

use std::time::{Duration, Instant};

use comum::theme_name;
use tomni_ui::kurbo::{Point, Vec2};
use tomni_ui::lint::Severity;
use tomni_ui::prelude::*;
use tomni_ui::terminal::cell::{Color as Cor, Flags};
use tomni_ui::terminal::{
    BELL_FLASH, CursorShape, LoopbackHandle, Palette, Session, TerminalVisual,
};
use tomni_ui::theme::Theme;
use tomni_ui::{Key, Look, Modifiers, ScrollAxis};
use tomni_ui_test::TestApp;

const TEMAS: [Theme; 2] = [Theme::Light, Theme::Dark];

const LS: &[u8] = include_bytes!("gravacoes/ls.bin");
const VIM: &[u8] = include_bytes!("gravacoes/vim.bin");
const HTOP: &[u8] = include_bytes!("gravacoes/htop.bin");
const TOP: &[u8] = include_bytes!("gravacoes/top.bin");

/// Um terminal de 80 × 24 com `Loopback`, e um botão depois dele (para o F6).
fn tela(
    configurar: impl FnOnce(Terminal) -> Terminal + 'static,
) -> (impl FnOnce() -> Form, LoopbackHandle) {
    let conexao = Loopback::new();
    let handle = conexao.handle();
    let view = move || {
        Form::new("Console").child(
            Column::new()
                .padding(16)
                .gap(8)
                .child(configurar(
                    Terminal::new(conexao).id("console").cols_rows(80, 24),
                ))
                .child(Button::new("Depois").id("depois").height(44)),
        )
    };
    (view, handle)
}

fn app(
    configurar: impl FnOnce(Terminal) -> Terminal + 'static,
    theme: Theme,
) -> (TestApp, LoopbackHandle) {
    let (view, handle) = tela(configurar);
    let app = TestApp::new(view).size(780, 560).theme(theme);
    (app, handle)
}

/// O visual do terminal, para achar as células na tela.
fn visual(app: &TestApp) -> TerminalVisual {
    match app.look("console") {
        Look::Custom(custom) => custom
            .downcast::<TerminalVisual>()
            .expect("o visual do terminal")
            .clone(),
        outro => panic!("não é o terminal: {outro:?}"),
    }
}

/// Centro da célula (`col`, `linha`) na janela.
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

// --- Snapshots ---------------------------------------------------------------------------

/// As 16 cores (texto e fundo), a paleta de 256 e um degradê *truecolor*.
fn cores() -> Vec<u8> {
    let mut s = String::from("\x1b[1mAs 16 cores\x1b[0m\r\n");
    for base in [30, 90] {
        for i in 0..8 {
            s.push_str(&format!(
                "\x1b[{}m cor{:<2}",
                base + i,
                if base == 30 { i } else { i + 8 }
            ));
        }
        s.push_str("\x1b[0m\r\n");
    }
    for base in [40, 100] {
        for i in 0..8 {
            s.push_str(&format!("\x1b[{}m      ", base + i));
        }
        s.push_str("\x1b[0m\r\n");
    }
    s.push_str("\x1b[1mPaleta de 256\x1b[0m\r\n");
    for linha in 0..8 {
        for i in 0..32 {
            s.push_str(&format!("\x1b[48;5;{}m  ", linha * 32 + i));
        }
        s.push_str("\x1b[0m\r\n");
    }
    s.push_str("\x1b[1mTruecolor\x1b[0m\r\n");
    for i in 0..80 {
        let r = 255 - i * 255 / 79;
        let g = (i * 510 / 79).min(510 - i * 510 / 79);
        let b = i * 255 / 79;
        s.push_str(&format!("\x1b[48;2;{r};{g};{b}m "));
    }
    s.push_str("\x1b[0m\r\n");
    s.into_bytes()
}

/// Os atributos do SGR e um pouco de texto largo e acentuado.
fn atributos() -> Vec<u8> {
    let mut s = String::new();
    for (sgr, nome) in [
        ("0", "normal"),
        ("1", "negrito"),
        ("2", "esmaecido"),
        ("3", "itálico"),
        ("4", "sublinhado"),
        ("4:2", "sublinhado duplo"),
        ("9", "tachado"),
        ("7", "inverso"),
        ("8", "oculto"),
        ("1;3;4;31", "negrito itálico sublinhado vermelho"),
        ("1;34", "azul em negrito"),
    ] {
        s.push_str(&format!("\x1b[{sgr}m{nome}\x1b[0m  ({sgr})\r\n"));
    }
    s.push_str("Ação, coração, pão — ✓ ±½ «aspas» │┌─┐│ ▒▓█\r\n");
    s.into_bytes()
}

#[test]
fn snapshots_de_cores_e_atributos() {
    for theme in TEMAS {
        let (mut app, _) = app(|t| t, theme);
        app.feed("console", cores());
        app.assert_snapshot(&format!("m17_cores_{}", theme_name(theme)));
        app.feed("console", "\x1b[2J\x1b[H");
        app.feed("console", atributos());
        app.assert_snapshot(&format!("m17_atributos_{}", theme_name(theme)));
    }
}

#[test]
fn snapshots_dos_tres_cursores() {
    for theme in TEMAS {
        for (forma, nome) in [
            (CursorShape::Bar, "barra"),
            (CursorShape::Block, "bloco"),
            (CursorShape::Underline, "sublinhado"),
        ] {
            let conexao = Loopback::new();
            let handle = conexao.handle();
            let mut app = TestApp::new(move || {
                Form::new("Cursor").child(
                    Column::new().padding(8).child(
                        Terminal::new(conexao)
                            .id("console")
                            .cols_rows(24, 3)
                            .cursor_shape(forma),
                    ),
                )
            })
            .size(280, 100)
            .theme(theme);
            handle.feed("$ echo olá");
            app.click("console");
            // O cursor fica sobre a letra "c" do "echo".
            app.feed("console", "\x1b[4G");
            app.assert_snapshot(&format!("m17_cursor_{nome}_{}", theme_name(theme)));
        }
    }
}

#[test]
fn snapshot_de_uma_selecao() {
    for theme in TEMAS {
        let (mut app, _) = app(|t| t, theme);
        app.feed("console", LS);
        app.feed(
            "console",
            "$ cargo build\r\n   Compiling tomni-ui v0.1.0\r\n    Finished `dev` profile\r\n$ ",
        );
        let (de, ate) = (celula(&app, 3, 2), celula(&app, 20, 3));
        app.drag(de, ate);
        app.assert_snapshot(&format!("m17_selecao_{}", theme_name(theme)));
    }
}

#[test]
fn snapshots_do_htop_e_do_vim() {
    for theme in TEMAS {
        let (mut app, _) = app(|t| t, theme);
        app.feed("console", HTOP);
        app.assert_snapshot(&format!("m17_htop_{}", theme_name(theme)));

        // O `vim` com as cores do tema, no lugar do Campbell.
        let (mut app, _) = app_com(|t| t.scheme(Scheme::Theme), theme);
        app.click("console");
        app.feed("console", VIM);
        app.assert_snapshot(&format!("m17_vim_{}", theme_name(theme)));
    }
}

fn app_com(
    configurar: impl FnOnce(Terminal) -> Terminal + 'static,
    theme: Theme,
) -> (TestApp, LoopbackHandle) {
    app(configurar, theme)
}

#[cfg(feature = "cjk")]
#[test]
fn snapshot_de_texto_cjk_em_duas_celulas() {
    for theme in TEMAS {
        let (mut app, _) = app(|t| t, theme);
        app.feed(
            "console",
            "日本語のテキスト|\r\n漢字とかな|ABC\r\nひらがなカタカナ|\r\n中文文本|0123456789\r\n",
        );
        assert_eq!(app.terminal_cell("console", 0, 0).c, '日');
        assert!(
            app.terminal_cell("console", 0, 0)
                .flags
                .contains(Flags::WIDE)
        );
        assert_eq!(
            app.terminal_cell("console", 16, 0).c,
            '|',
            "8 largos = 16 células"
        );
        app.assert_snapshot(&format!("m17_cjk_{}", theme_name(theme)));
    }
}

#[test]
fn snapshot_do_aviso_de_colar() {
    for theme in TEMAS {
        let (mut app, _) = app(|t| t, theme);
        app.click("console");
        app.clipboard().write("rm -rf build\nmake".into());
        app.key_with(Key::Char('v'), Modifiers::CTRL.with(Modifiers::SHIFT));
        assert!(app.is_open("console-paste"));
        app.assert_snapshot(&format!("m17_aviso_colar_{}", theme_name(theme)));
    }
}

// --- Gravações de programas de verdade ---------------------------------------------------

#[test]
fn ls_com_cores() {
    let (mut app, _) = app(|t| t, Theme::Light);
    app.feed("console", LS);
    assert_eq!(
        app.terminal_text("console"),
        "Cargo.toml  README.md  atalho.rs@  build.sh*  docs/  src/"
    );
    let docs = app.terminal_cell("console", 46, 0);
    assert_eq!((docs.c, docs.fg), ('d', Cor::Indexed(4)));
    assert!(docs.flags.contains(Flags::BOLD));
    let atalho = app.terminal_cell("console", 23, 0);
    assert_eq!((atalho.c, atalho.fg), ('a', Cor::Indexed(6)));
    let barra = app.terminal_cell("console", 50, 0);
    assert_eq!(
        (barra.c, barra.fg),
        ('/', Cor::Default),
        "o `/` vem depois do reset"
    );
}

#[test]
fn vim_na_tela_alternativa() {
    let (mut app, handle) = app(|t| t, Theme::Light);
    app.feed("console", "$ vim src/main.rs\r\n");
    app.feed("console", VIM);
    let tela = app.terminal_text("console");
    let linhas: Vec<&str> = tela.lines().collect();
    assert_eq!(linhas[0], "//! Um app de exemplo.");
    assert_eq!(linhas[4], "    let saudação = \"Olá, mundo!\";");
    assert_eq!(linhas[7], "    // fim do arquivo");
    assert!(linhas[8..23].iter().all(|l| *l == "~"), "{linhas:?}");
    assert!(
        linhas[23].ends_with("8,21          All"),
        "{:?}",
        linhas[23]
    );
    let no = app.tree().find("console").cloned().unwrap();
    assert_eq!(no.cursor, Some([20, 7]));
    assert_eq!(
        no.history,
        Some([0, 0]),
        "a tela alternativa não tem histórico"
    );
    // O `vim` perguntou a posição do cursor e o tipo de terminal: o terminal respondeu.
    let respostas = escrito(&handle);
    assert_eq!(respostas, "\x1b[2;2R\x1b[3;1R\x1b[>0;10;1c");
    // Sair da tela alternativa devolve a principal como estava.
    app.feed("console", "\x1b[?1049l");
    assert_eq!(app.terminal_text("console"), "$ vim src/main.rs");
}

#[test]
fn htop_e_top() {
    let (mut app, _) = app(|t| t, Theme::Light);
    app.feed("console", HTOP);
    let tela = app.terminal_text("console");
    let linhas: Vec<&str> = tela.lines().collect();
    assert!(
        linhas[9].trim_start().starts_with("PID USER"),
        "{:?}",
        linhas[9]
    );
    assert!(linhas[10].contains("nobody"), "{:?}", linhas[10]);
    assert!(
        linhas[23].starts_with("F1Help  F2Setup F3SearchF4Filter"),
        "{:?}",
        linhas[23]
    );
    let cabecalho = app.terminal_cell("console", 4, 9);
    assert_eq!(
        (cabecalho.fg, cabecalho.bg),
        (Cor::Indexed(0), Cor::Indexed(2))
    );
    // O `htop` limita a rolagem às 24 linhas e liga o modo aplicação das setas.
    app.click("console");
    app.ui();
    let (mut app, handle) = self::app(|t| t, Theme::Light);
    app.feed("console", TOP);
    let tela = app.terminal_text("console");
    assert!(tela.starts_with("top - "), "{tela:?}");
    assert!(tela.contains("load average:"));
    let linha = tela.lines().position(|l| l.contains("PID USER")).unwrap();
    assert!(
        app.terminal_cell("console", 4, linha)
            .flags
            .contains(Flags::INVERSE)
    );
    app.click("console");
    handle.take_written();
    app.key(Key::Up);
    assert_eq!(escrito(&handle), "\x1bOA", "o `top` ligou o DECCKM");
}

/// O primeiro teste do menu 1 do `vttest` ("Test of cursor movements"), refeito com as mesmas
/// sequências: uma moldura de `*` e `+` na borda, desenhada com CUP, HVP, IND, RI, NEL, CUF,
/// CUB, CUU e CUD, e uma moldura de `E` (do DECALN) no meio, recortada com ED e EL.
#[test]
fn vttest_movimentos_do_cursor() {
    let mut s = String::new();
    let cup =
        |s: &mut String, linha: usize, col: usize| s.push_str(&format!("\x1b[{linha};{col}H"));
    s.push_str("\x1b[2J\x1b#8");
    cup(&mut s, 9, 10);
    s.push_str("\x1b[1J");
    cup(&mut s, 18, 60);
    s.push_str("\x1b[0J\x1b[1K");
    cup(&mut s, 9, 71);
    s.push_str("\x1b[0K");
    for linha in 10..=16 {
        cup(&mut s, linha, 10);
        s.push_str("\x1b[1K");
        cup(&mut s, linha, 71);
        s.push_str("\x1b[0K");
    }
    cup(&mut s, 17, 30);
    s.push_str("\x1b[2K");
    for col in 1..=80 {
        s.push_str(&format!("\x1b[24;{col}f*\x1b[1;{col}f*"));
    }
    cup(&mut s, 2, 2);
    for _ in 2..=23 {
        s.push_str("+\x1b[1D\x1bD");
    }
    cup(&mut s, 23, 79);
    for _ in 2..=23 {
        s.push_str("+\x1b[1D\x1bM");
    }
    cup(&mut s, 2, 1);
    for linha in 2..=23 {
        s.push('*');
        cup(&mut s, linha, 80);
        s.push('*');
        s.push_str("\x1b[10D");
        s.push_str(if linha < 10 { "\x1bE" } else { "\r\n" });
    }
    cup(&mut s, 2, 10);
    s.push_str("\x1b[42D\x1b[2C");
    for _ in 3..=78 {
        s.push_str("+\x1b[0C\x1b[2D\x1b[1C");
    }
    cup(&mut s, 23, 70);
    s.push_str("\x1b[42C\x1b[2D");
    for _ in 3..=78 {
        s.push_str("+\x1b[1D\x1b[1C\x1b[0D\x08");
    }
    cup(&mut s, 1, 1);
    s.push_str("\x1b[10A\x1b[1A\x1b[0A");
    cup(&mut s, 24, 80);
    s.push_str("\x1b[10B\x1b[1B\x1b[0B");
    cup(&mut s, 10, 12);
    for _ in 10..=15 {
        s.push_str(&" ".repeat(58));
        s.push_str("\x1b[1B\x1b[58D");
    }
    s.push_str("\x1b[5A\x1b[1C");
    s.push_str("The screen should be cleared,  and have an unbroken bor-");
    cup(&mut s, 12, 13);
    s.push_str("der of *'s and +'s around the edge,   and exactly in the");
    cup(&mut s, 13, 13);
    s.push_str("middle  there should be a frame of E's around this  text");
    cup(&mut s, 14, 13);
    s.push_str("with  one (1) free position around it.    ");

    let (mut app, _) = app(|t| t, Theme::Light);
    app.feed("console", s);
    let tela = app.terminal_text("console");
    let linhas: Vec<&str> = tela.lines().collect();
    let estrelas = "*".repeat(80);
    let mais = format!("*{}*", "+".repeat(78));
    let borda = |meio: &str| format!("*+{meio}+*");
    let vazio = " ".repeat(76);
    let es = format!("{}{}{}", " ".repeat(8), "E".repeat(60), " ".repeat(8));
    let lado = |meio: &str| format!("{}E{meio}E{}", " ".repeat(8), " ".repeat(8));
    let texto = |t: &str| lado(&format!(" {t:<56} "));
    let mut esperado = vec![estrelas.clone(), mais.clone()];
    for _ in 3..=8 {
        esperado.push(borda(&vazio));
    }
    esperado.push(borda(&es));
    esperado.push(borda(&lado(&" ".repeat(58))));
    esperado.push(borda(&texto(
        "The screen should be cleared,  and have an unbroken bor-",
    )));
    esperado.push(borda(&texto(
        "der of *'s and +'s around the edge,   and exactly in the",
    )));
    esperado.push(borda(&texto(
        "middle  there should be a frame of E's around this  text",
    )));
    esperado.push(borda(&texto("with  one (1) free position around it.")));
    esperado.push(borda(&lado(&" ".repeat(58))));
    esperado.push(borda(&es));
    for _ in 17..=22 {
        esperado.push(borda(&vazio));
    }
    esperado.push(mais);
    esperado.push(estrelas);
    for (i, (linha, esperada)) in linhas.iter().zip(&esperado).enumerate() {
        assert_eq!(linha, &esperada.trim_end(), "linha {}", i + 1);
    }
    assert_eq!(linhas.len(), 24);
}

/// O segundo teste do `vttest` de apagamento: inserir e apagar caracteres e linhas (ICH, DCH,
/// IL, DL) e o apagar com a cor de fundo (o "Test of screen features" do menu 2).
#[test]
fn vttest_inserir_e_apagar() {
    let (mut app, _) = app(|t| t, Theme::Light);
    // "Insert Mode": com o IRM, o texto empurra o resto da linha.
    app.feed("console", "\x1b[2J\x1b[HABCDEFGH\x1b[1;3H\x1b[4hxy\x1b[4l");
    assert_eq!(app.terminal_text("console"), "ABxyCDEFGH");
    // DCH e ICH.
    app.feed("console", "\x1b[1;3H\x1b[2P\x1b[1;1H\x1b[3@");
    assert_eq!(app.terminal_text("console"), "   ABCDEFGH");
    // IL e DL dentro da região de rolagem.
    app.feed(
        "console",
        "\x1b[2J\x1b[H1\r\n2\r\n3\r\n4\r\n5\x1b[2;4r\x1b[2;1H\x1b[L",
    );
    assert_eq!(
        app.terminal_text("console"),
        "1\n\n2\n3\n5",
        "o 4 saiu pela margem de baixo"
    );
    app.feed("console", "\x1b[2;1H\x1b[2M\x1b[r");
    assert_eq!(
        app.terminal_text("console"),
        "1\n3\n\n\n5",
        "o 5 fica fora da região"
    );
    // Apagar usa a cor de fundo da caneta (o "BCE" do xterm).
    app.feed("console", "\x1b[2J\x1b[44m\x1b[3;1H\x1b[K\x1b[0m");
    assert_eq!(app.terminal_cell("console", 79, 2).bg, Cor::Indexed(4));
    assert_eq!(app.terminal_cell("console", 79, 3).bg, Cor::Default);
}

// --- Teclado -----------------------------------------------------------------------------

#[test]
fn teclas_no_formato_do_xterm() {
    let (mut app, handle) = app(|t| t, Theme::Light);
    app.click("console");
    assert_eq!(app.focused().as_deref(), Some("console"));
    let ctrl = Modifiers::CTRL;
    for (tecla, mods, bytes) in [
        (Key::Up, Modifiers::NONE, "\x1b[A"),
        (Key::Left, ctrl, "\x1b[1;5D"),
        (Key::Home, Modifiers::NONE, "\x1b[H"),
        (Key::End, Modifiers::SHIFT, "\x1b[1;2F"),
        (Key::PageUp, Modifiers::NONE, "\x1b[5~"),
        (Key::Delete, Modifiers::NONE, "\x1b[3~"),
        (Key::Insert, Modifiers::NONE, "\x1b[2~"),
        (Key::F(1), Modifiers::NONE, "\x1bOP"),
        (Key::F(5), Modifiers::NONE, "\x1b[15~"),
        (Key::F(12), Modifiers::SHIFT, "\x1b[24;2~"),
        (Key::Enter, Modifiers::NONE, "\r"),
        (Key::Backspace, Modifiers::NONE, "\x7f"),
        (Key::Escape, Modifiers::NONE, "\x1b"),
        (Key::Tab, Modifiers::NONE, "\t"),
        (Key::Tab, Modifiers::SHIFT, "\x1b[Z"),
        (Key::Char('a'), ctrl, "\x01"),
        (Key::Char('c'), ctrl, "\x03"),
        (Key::Char('z'), ctrl, "\x1a"),
    ] {
        app.key_with(tecla, mods);
        assert_eq!(escrito(&handle), bytes, "{tecla:?} com {mods:?}");
    }
    assert_eq!(
        app.focused().as_deref(),
        Some("console"),
        "o Tab fica no terminal"
    );

    // O modo aplicação (DECCKM) muda as setas e o Home/End.
    app.feed("console", "\x1b[?1h");
    app.key(Key::Up).key(Key::Home);
    assert_eq!(escrito(&handle), "\x1bOA\x1bOH");
    app.feed("console", "\x1b[?1l");
    app.key(Key::Up);
    assert_eq!(escrito(&handle), "\x1b[A");

    // Texto comum e acentos vão em UTF-8; com Alt, com um ESC antes.
    app.type_text("ação");
    assert_eq!(escrito(&handle), "ação");
    app.ui().set_modifiers(Modifiers::ALT);
    app.type_text("x");
    app.ui().set_modifiers(Modifiers::NONE);
    assert_eq!(escrito(&handle), "\x1bx");

    // O F6 sai do terminal, para o próximo widget, e o Shift+Tab volta; o Shift+F6 sai para
    // o anterior.
    app.key(Key::F(6));
    assert_eq!(app.focused().as_deref(), Some("depois"));
    app.shift_tab();
    assert_eq!(app.focused().as_deref(), Some("console"));
    assert_eq!(escrito(&handle), "");
}

#[test]
fn ctrl_tab_nao_fica_no_terminal() {
    let conexao = Loopback::new();
    let handle = conexao.handle();
    let aba = signal(String::from("um"));
    let mut app = TestApp::new(move || {
        Form::new("Abas").child(
            TabView::bind(aba)
                .id("abas")
                .tab("um", "Um", Terminal::new(conexao).id("console").grow(1))
                .tab("dois", "Dois", Text::new("Segunda aba").id("dois")),
        )
    })
    .size(780, 560);
    app.click("console");
    app.key_with(Key::Tab, Modifiers::CTRL);
    assert_eq!(escrito(&handle), "", "o Ctrl+Tab não vai ao programa");
    assert!(app.visible("dois"), "o Ctrl+Tab trocou de aba");
}

#[test]
fn ime_compoe_no_cursor_e_escreve_o_texto() {
    let (mut app, handle) = app(|t| t, Theme::Light);
    app.click("console");
    app.feed("console", "$ ");
    app.ime_preedit("にほん");
    assert_eq!(visual(&app).preedit(), Some("にほん"));
    assert_eq!(escrito(&handle), "", "a composição não vai ao programa");
    app.ime_commit("日本");
    assert_eq!(visual(&app).preedit(), None);
    assert_eq!(escrito(&handle), "日本");
    let caret = app.text_input().expect("o IME fica ligado no terminal");
    let cursor = celula(&app, 2, 0);
    assert!(
        caret.area.contains(cursor) || (caret.area.x0 - cursor.x).abs() < 10.0,
        "{caret:?}"
    );
}

// --- Copiar e colar ----------------------------------------------------------------------

#[test]
fn selecao_por_arrasto_palavra_linha_e_bloco() {
    let (mut app, handle) = app(|t| t, Theme::Light);
    app.feed(
        "console",
        "primeira linha de texto\r\nsegunda linha aqui\r\nterceira",
    );
    app.click("console");
    // Arrastar: da coluna 9 da linha 0 até a coluna 6 da linha 1.
    let (de, ate) = (celula(&app, 9, 0), celula(&app, 6, 1));
    app.drag(de, ate);
    app.key_with(Key::Char('c'), Modifiers::CTRL.with(Modifiers::SHIFT));
    assert_eq!(
        app.clipboard().read().as_deref(),
        Some("linha de texto\nsegunda")
    );
    assert_eq!(escrito(&handle), "", "copiar não manda nada ao programa");

    // Duplo clique: a palavra. O Ctrl+C com seleção copia, e não manda ^C.
    let ponto = celula(&app, 10, 1);
    app.click_at(ponto, 1).click_at(ponto, 2);
    app.key_with(Key::Char('c'), Modifiers::CTRL);
    assert_eq!(app.clipboard().read().as_deref(), Some("linha"));
    assert_eq!(escrito(&handle), "");
    // Copiar apaga a seleção: o próximo Ctrl+C é o ^C.
    app.key_with(Key::Char('c'), Modifiers::CTRL);
    assert_eq!(escrito(&handle), "\x03");

    // Triplo clique: a linha.
    let ponto = celula(&app, 3, 2);
    app.click_at(ponto, 1).click_at(ponto, 2).click_at(ponto, 3);
    app.key_with(Key::Insert, Modifiers::CTRL);
    assert_eq!(app.clipboard().read().as_deref(), Some("terceira"));

    // Com Alt, um bloco: as colunas 0 a 4 das linhas 0 e 1.
    let (de, ate) = (celula(&app, 0, 0), celula(&app, 4, 1));
    app.ui().set_modifiers(Modifiers::ALT);
    app.drag(de, ate);
    app.ui().set_modifiers(Modifiers::NONE);
    app.key_with(Key::Char('c'), Modifiers::CTRL.with(Modifiers::SHIFT));
    assert_eq!(app.clipboard().read().as_deref(), Some("prime\nsegun"));
}

#[test]
fn selection_text_segue_o_widget() {
    let conexao = Loopback::new();
    let sessao = Session::new(conexao);
    let mut app = TestApp::new(move || {
        Form::new("Console").child(Terminal::session(sessao).id("console").cols_rows(80, 24))
    })
    .size(780, 560);
    app.feed(
        "console",
        "primeira linha de texto\r\nsegunda linha aqui\r\nterceira",
    );
    assert_eq!(sessao.selection_text(), None);
    assert!(!sessao.has_selection());

    // Arrastar seleciona; a sessão enxerga o mesmo texto, sem tocar na área de transferência.
    app.click("console");
    let (de, ate) = (celula(&app, 9, 0), celula(&app, 6, 1));
    app.drag(de, ate);
    assert_eq!(
        sessao.selection_text().as_deref(),
        Some("linha de texto\nsegunda")
    );
    assert!(sessao.has_selection());
    assert_eq!(app.clipboard().read(), None);

    // Copiar apaga a seleção, e a sessão acompanha.
    app.key_with(Key::Char('c'), Modifiers::CTRL.with(Modifiers::SHIFT));
    assert_eq!(sessao.selection_text(), None);
    assert!(!sessao.has_selection());

    // Duplo clique: a palavra.
    let ponto = celula(&app, 10, 1);
    app.click_at(ponto, 1).click_at(ponto, 2);
    assert_eq!(sessao.selection_text().as_deref(), Some("linha"));

    // Digitar tira a seleção.
    app.type_text("x");
    assert_eq!(sessao.selection_text(), None);
    assert!(!sessao.has_selection());
}

#[test]
fn item_do_menu_aparece_so_com_selecao() {
    let sessao = Session::new(Loopback::new());
    let mut app = TestApp::new(move || {
        let menu = MenuFlyout::new()
            .id("console-menu")
            .item("explicar-selecao", "Explicar a seleção", || {})
            .item_visible(move || sessao.has_selection())
            .item("explicar-erro", "Explicar o erro", || {})
            .item_visible(move || !sessao.has_selection());
        Form::new("Console").child(
            Terminal::session(sessao)
                .id("console")
                .cols_rows(80, 24)
                .context_flyout(menu),
        )
    })
    .size(780, 560);
    app.feed("console", "primeira linha de texto\r\nsegunda");

    app.right_click("console");
    assert!(app.is_open("console-menu"));
    assert!(!app.visible("explicar-selecao"));
    assert!(app.visible("explicar-erro"));
    app.key(Key::Escape);

    app.click("console");
    let (de, ate) = (celula(&app, 0, 0), celula(&app, 5, 0));
    app.drag(de, ate);
    app.right_click("console");
    assert!(app.is_open("console-menu"));
    assert!(app.visible("explicar-selecao"));
    assert!(!app.visible("explicar-erro"));
}

#[test]
fn menu_do_app_com_copiar_e_colar_do_terminal() {
    let (mut app, handle) = app(
        |t| {
            t.context_flyout(
                MenuFlyout::new()
                    .id("console-menu")
                    .item("do-app", "Do app", || {}),
            )
            .context_copy_paste()
        },
        Theme::Light,
    );
    app.feed("console", "primeira linha de texto\r\nsegunda");

    // Sem seleção: "Colar" e o item do app; "Copiar" some.
    app.right_click("console");
    assert!(app.is_open("console-menu"));
    assert!(!app.visible("console-menu-copiar"));
    assert!(app.visible("console-menu-colar"));
    assert!(app.visible("do-app"));
    app.clipboard().write("ls -la".into());
    app.click("console-menu-colar");
    app.wait(std::time::Duration::from_millis(50));
    assert_eq!(escrito(&handle), "ls -la");

    // Várias linhas: o "Colar" do menu passa pelo mesmo aviso do teclado.
    app.clipboard().write("cd /tmp\nrm -rf *\n".into());
    app.right_click("console");
    app.click("console-menu-colar");
    app.wait(std::time::Duration::from_millis(50));
    assert!(app.is_open("console-paste"));
    assert_eq!(escrito(&handle), "");
    app.click("console-paste-close");

    // Com seleção: "Copiar" aparece, copia e limpa a seleção.
    app.click("console");
    let (de, ate) = (celula(&app, 0, 0), celula(&app, 5, 0));
    app.drag(de, ate);
    app.right_click("console");
    assert!(app.visible("console-menu-copiar"));
    app.click("console-menu-copiar");
    app.wait(std::time::Duration::from_millis(50));
    assert_eq!(app.clipboard().read().as_deref(), Some("primei"));
}

#[test]
fn colar_uma_linha_e_varias_com_confirmacao() {
    let (mut app, handle) = app(|t| t, Theme::Light);
    app.click("console");
    let colar = Modifiers::CTRL.with(Modifiers::SHIFT);

    app.clipboard().write("ls -la".into());
    app.key_with(Key::Char('v'), colar);
    assert_eq!(escrito(&handle), "ls -la");
    assert!(!app.is_open("console-paste"));

    // Várias linhas: o aviso abre, e nada vai antes de confirmar.
    app.clipboard().write("cd /tmp\nrm -rf *\n".into());
    app.key_with(Key::Char('v'), colar);
    assert!(app.is_open("console-paste"));
    assert_eq!(escrito(&handle), "");
    app.click("console-paste-close");
    assert!(!app.is_open("console-paste"));
    assert_eq!(escrito(&handle), "", "cancelar não cola");
    assert_eq!(
        app.focused().as_deref(),
        Some("console"),
        "o foco volta ao terminal"
    );

    app.key_with(Key::Char('v'), colar);
    app.click("console-paste-primary");
    assert_eq!(
        escrito(&handle),
        "cd /tmp\rrm -rf *\r",
        "as quebras viram Enter"
    );

    // Com o "bracketed paste" (2004), o programa sabe que é colagem: sem aviso, entre marcas.
    app.feed("console", "\x1b[?2004h");
    app.key_with(Key::Char('v'), colar);
    assert!(!app.is_open("console-paste"));
    assert_eq!(escrito(&handle), "\x1b[200~cd /tmp\rrm -rf *\r\x1b[201~");

    // Shift+Insert também cola.
    app.clipboard().write("pwd".into());
    app.key_with(Key::Insert, Modifiers::SHIFT);
    assert_eq!(escrito(&handle), "\x1b[200~pwd\x1b[201~");
}

#[test]
fn copiar_ao_selecionar_e_clique_direito_cola() {
    let (mut app, handle) = app(
        |t| t.copy_on_select(true).paste_warning(false),
        Theme::Light,
    );
    app.feed("console", "usuario@servidor:~$ ");
    let (de, ate) = (celula(&app, 0, 0), celula(&app, 6, 0));
    app.drag(de, ate);
    assert_eq!(app.clipboard().read().as_deref(), Some("usuario"));
    app.right_click("console");
    assert_eq!(escrito(&handle), "usuario");
    // Sem o aviso, várias linhas colam direto.
    app.clipboard().write("a\nb".into());
    app.right_click("console");
    assert!(!app.is_open("console-paste"));
    assert_eq!(escrito(&handle), "a\rb");
}

// --- Histórico ---------------------------------------------------------------------------

#[test]
fn historico_com_roda_teclas_e_saida_nova() {
    let (mut app, handle) = app(|t| t, Theme::Light);
    let mut saida = String::new();
    for i in 1..=10_000 {
        saida.push_str(&format!("linha {i}\r\n"));
    }
    app.feed("console", saida);
    let [historico, subiu] = rolagem(&app);
    assert_eq!((historico, subiu), (9001, 0), "o teto padrão");
    assert_eq!(
        app.terminal_text("console").lines().last(),
        Some("linha 10000")
    );

    // A roda sobe três linhas por vez (o `SCROLL_LINE` de cada passo).
    app.click("console");
    let meio = app.rect("console").center();
    app.wheel(meio, Vec2::new(0.0, -2.0 * tomni_ui::SCROLL_LINE));
    assert_eq!(rolagem(&app)[1], 2);
    // Shift+PageUp sobe uma tela.
    app.key_with(Key::PageUp, Modifiers::SHIFT);
    assert_eq!(rolagem(&app)[1], 26);
    let visivel = visual(&app);
    let topo = visivel.emulator().view_row(0).text(0, 80);
    drop(visivel);

    // Saída nova não puxa a tela de quem subiu: o mesmo texto continua no topo.
    app.feed("console", "linha nova 1\r\nlinha nova 2\r\n");
    assert_eq!(rolagem(&app)[1], 28);
    assert_eq!(visual(&app).emulator().view_row(0).text(0, 80), topo);

    // Shift+PageDown desce, e digitar volta ao fim.
    app.key_with(Key::PageDown, Modifiers::SHIFT);
    assert_eq!(rolagem(&app)[1], 4);
    app.type_text("x");
    assert_eq!(rolagem(&app)[1], 0);
    assert_eq!(escrito(&handle), "x");

    // Na tela alternativa não há histórico: a roda vira setas.
    app.feed("console", "\x1b[?1049h");
    app.wheel(meio, Vec2::new(0.0, -tomni_ui::SCROLL_LINE));
    assert_eq!(escrito(&handle), "\x1b[A");
    assert_eq!(rolagem(&app), [0, 0]);
}

#[test]
fn barra_de_rolagem_arrasta_o_historico() {
    let (mut app, _) = app(|t| t, Theme::Light);
    let mut saida = String::new();
    for i in 1..=200 {
        saida.push_str(&format!("linha {i}\r\n"));
    }
    app.feed("console", saida);
    let rect = app.rect("console");
    let barra = visual(&app)
        .scroller(rect)
        .bar(rect, ScrollAxis::Y)
        .expect("a barra de rolagem");
    // Arrastar o polegar do fim até o topo da barra leva ao começo do histórico.
    let de = barra.thumb.center();
    app.drag(de, Point::new(de.x, de.y - barra.travel - 20.0));
    assert_eq!(rolagem(&app), [177, 177]);
    assert_eq!(visual(&app).emulator().view_row(0).text(0, 80), "linha 1");
    // Um clique no trilho, abaixo do polegar, desce uma tela.
    let barra = visual(&app)
        .scroller(rect)
        .bar(rect, ScrollAxis::Y)
        .unwrap();
    app.click_at(Point::new(barra.thumb.center().x, barra.thumb.y1 + 20.0), 1);
    assert_eq!(rolagem(&app), [177, 177 - 24]);
}

// --- Redimensionar -----------------------------------------------------------------------

#[test]
fn redimensionar_reflui_e_avisa_o_transporte() {
    let conexao = Loopback::new();
    let handle = conexao.handle();
    let mut app = TestApp::new(move || {
        Form::new("Console").child(Terminal::new(conexao).id("console").grow(1))
    })
    .size(780, 560);
    let no = app.tree().find("console").cloned().unwrap();
    let (cols, rows) = (no.cols.unwrap(), no.rows.unwrap());
    assert_eq!(handle.size(), Some((cols as u16, rows as u16)));
    let longa: String = (0..60).map(|i| char::from(b'a' + (i % 26) as u8)).collect();
    app.feed("console", format!("{longa}\r\n$ "));

    // Mais estreito que a linha: ela quebra em duas, e o transporte fica sabendo.
    app.resize(400, 560);
    let no = app.tree().find("console").cloned().unwrap();
    let estreito = no.cols.unwrap();
    assert!(estreito < 60, "{estreito} colunas");
    assert_eq!(
        handle.size(),
        Some((estreito as u16, no.rows.unwrap() as u16))
    );
    let tela = app.terminal_text("console");
    let linhas: Vec<&str> = tela.lines().collect();
    assert_eq!(linhas[0], &longa[..estreito]);
    assert_eq!(linhas[1], &longa[estreito..]);
    assert_eq!(linhas[2], "$");
    assert_eq!(no.cursor, Some([2, 2]), "o cursor vai junto");

    // De volta: a linha junta de novo.
    app.resize(780, 560);
    let tela = app.terminal_text("console");
    assert_eq!(tela, format!("{longa}\n$"));
    assert_eq!(app.terminal_history("console"), format!("{longa}\n$"));

    // Mais baixo que o conteúdo: as linhas de cima vão para o histórico.
    app.feed(
        "console",
        "\r\n1\r\n2\r\n3\r\n4\r\n5\r\n6\r\n7\r\n8\r\n9\r\n10\r\n11\r\n12\r\n$ ",
    );
    app.resize(780, 200);
    let no = app.tree().find("console").cloned().unwrap();
    assert!(no.history.unwrap()[0] > 0);
    assert_eq!(app.terminal_text("console").lines().last(), Some("$"));
}

// --- Desempenho --------------------------------------------------------------------------

/// 10 MB de saída chegando de uma vez: a interface continua respondendo, com cada frame
/// abaixo de 50 ms, e o terminal lê em lotes até o fim. Passa o tempo direto na `Ui`, como o
/// laço da janela, sem o `TestApp` que lê tudo de uma vez a cada passo.
#[test]
fn dez_megas_sem_frame_acima_de_50_ms() {
    let (mut app, handle) = app(|t| t, Theme::Light);
    let mut saida = Vec::with_capacity(10 * 1024 * 1024 + 100);
    let mut i = 0u32;
    while saida.len() < 10 * 1024 * 1024 {
        saida.extend_from_slice(
            format!(
                "\x1b[3{}mlinha {i:>8}\x1b[0m: o rápido cachorro marrom pula a cerca 0123456789\r\n",
                i % 8
            )
            .as_bytes(),
        );
        i += 1;
    }
    let ultima = format!(
        "linha {:>8}: o rápido cachorro marrom pula a cerca 0123456789",
        i - 1
    );
    handle.feed(&saida);
    let mut frames = 0;
    // O pior frame só da leitura, da emulação e da cena, e o pior com a rasterização.
    let (mut pior, mut pior_com_pixels) = (Duration::ZERO, Duration::ZERO);
    let inicio = Instant::now();
    loop {
        let t = Instant::now();
        let ui = app.ui();
        ui.advance(Duration::from_millis(16));
        ui.frame();
        pior = pior.max(t.elapsed());
        // A janela inteira rasterizada na CPU: mais do que um frame de verdade, que repinta
        // só as linhas que mudaram.
        std::hint::black_box(app.snapshot());
        pior_com_pixels = pior_com_pixels.max(t.elapsed());
        frames += 1;
        let tela = app.terminal_text("console");
        if tela.lines().last() == Some(ultima.as_str()) {
            break;
        }
        assert!(
            inicio.elapsed() < Duration::from_secs(300),
            "não terminou: {frames} frames"
        );
    }
    eprintln!(
        "10 MB em {frames} frames, {:?}; o pior levou {pior:?}, {pior_com_pixels:?} com os pixels",
        inicio.elapsed()
    );
    assert!(frames > 1, "a saída foi lida em lotes");
    assert!(
        pior < Duration::from_millis(50),
        "o pior frame levou {pior:?} ({frames} frames)"
    );
    // A rasterização na CPU sem otimização (o `cargo test` comum) passa dos 50 ms em máquinas
    // lentas; o limite com os pixels vale no `--release`.
    if !cfg!(debug_assertions) {
        assert!(
            pior_com_pixels < Duration::from_millis(50),
            "o pior frame com os pixels levou {pior_com_pixels:?}"
        );
    }
    assert_eq!(rolagem(&app)[0], 9001);
}

// --- Campainha, título, conexão fechada e lint -------------------------------------------

#[test]
fn campainha_titulo_e_conexao_fechada() {
    let titulo = signal(String::new());
    let conexao = Loopback::new();
    let handle = conexao.handle();
    let mut app = TestApp::new(move || {
        Form::new("Console").child(
            Terminal::new(conexao)
                .id("console")
                .grow(1)
                .on_title(move |t| titulo.set(t)),
        )
    })
    .size(780, 560);
    app.feed("console", "\x1b]2;ana@servidor: ~\x07");
    assert_eq!(titulo.get_untracked(), "ana@servidor: ~");
    assert_eq!(
        app.tree().find("console").unwrap().title.as_deref(),
        Some("ana@servidor: ~")
    );

    handle.feed("\x07");
    app.frame();
    assert!(visual(&app).flash(), "a campainha pisca a tela");
    app.wait(BELL_FLASH);
    assert!(!visual(&app).flash());

    app.click("console");
    handle.close(Some("conexão recusada".into()));
    app.frame();
    assert!(
        app.terminal_text("console")
            .ends_with("[conexão encerrada: conexão recusada]")
    );
    app.type_text("ls");
    assert_eq!(escrito(&handle), "", "nada vai para uma conexão fechada");
}

#[test]
fn lint_confere_so_o_contraste_do_esquema() {
    for theme in TEMAS {
        let (mut app, _) = app(|t| t, theme);
        // Cores ruins do programa não são problema do app.
        app.feed("console", "\x1b[30;40mpreto no preto\x1b[0m 漢字 ✓");
        app.assert_lint();

        let (mut app, _) = app_com(|t| t.scheme(Scheme::Theme), theme);
        app.assert_lint();
    }
    let ruim = Palette {
        background: Color::hex(0x202020),
        foreground: Color::hex(0x303030),
        ..tomni_ui::terminal::CAMPBELL
    };
    let (mut app, _) = app(move |t| t.scheme(Scheme::Custom(ruim)), Theme::Light);
    let problemas = app.lint();
    assert_eq!(problemas.len(), 1, "{problemas:?}");
    assert_eq!(problemas[0].rule.name(), "contrast");
    assert_eq!(problemas[0].severity, Severity::Error);
    assert!(problemas[0].widget.contains("console"));
}

#[test]
fn arvore_mostra_o_terminal() {
    let (mut app, _) = app(|t| t, Theme::Dark);
    app.feed("console", "$ ls\r\nCargo.toml\r\n$ ");
    app.click("console");
    let no = app.tree().find("console").cloned().unwrap();
    assert_eq!(no.kind, "terminal");
    assert_eq!((no.cols, no.rows), (Some(80), Some(24)));
    assert_eq!(no.cursor, Some([2, 2]));
    assert_eq!(no.text.as_deref(), Some("$ ls\nCargo.toml\n$"));
    assert_eq!(no.variant, Some("campbell"));
    assert_eq!(no.state, Some("focused"));
}

/// O critério "com a feature `pty`": um `echo` num shell de verdade, digitado no terminal.
#[cfg(unix)]
#[test]
fn echo_num_shell_local() {
    use tomni_ui::terminal::Process;

    let mut app = TestApp::new(|| {
        Form::new("Shell").child(
            Column::new().padding(16).child(
                Terminal::new(Process::builder("sh").env("PS1", "$ ").spawn())
                    .id("shell")
                    .cols_rows(80, 24),
            ),
        )
    })
    .size(780, 520);
    app.click("shell");
    app.type_text("echo olá-$((1+1)); stty size");
    app.key(Key::Enter);
    let limite = Instant::now() + Duration::from_secs(10);
    let linhas = loop {
        app.wait(Duration::from_millis(20));
        let texto = app.terminal_text("shell");
        let linhas: Vec<String> = texto.lines().map(|l| l.trim_end().to_owned()).collect();
        if linhas.iter().any(|l| l == "24 80") || Instant::now() > limite {
            break linhas;
        }
    };
    assert!(linhas.iter().any(|l| l == "olá-2"), "{linhas:#?}");
    // O tamanho que o layout deu à grade chegou ao pseudoterminal.
    assert!(linhas.iter().any(|l| l == "24 80"), "{linhas:#?}");
}
