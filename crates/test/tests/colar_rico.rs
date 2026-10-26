//! M45: colar imagem e arquivos no `Terminal`. O handler `on_rich_paste`, o Ctrl+V sem Shift, os
//! arquivos soltos, o envio para vários terminais e o `Session::paste`.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use tomni_ui::prelude::*;
use tomni_ui::terminal::{
    Broadcast, LoopbackHandle, PaneLayout, RichPaste, RichPasteContent, RichPasteSource, Session,
    TerminalPanes,
};
use tomni_ui::{ClipboardImage, Key, Modifiers};
use tomni_ui_test::TestApp;

type Colagens = Rc<RefCell<Vec<RichPaste>>>;

const COLAR: Modifiers = Modifiers::CTRL.with(Modifiers::SHIFT);

/// Um terminal `console` sobre um `Loopback`, com o handler guardando as colagens ricas
/// (`com_handler`) e o Ctrl+V rico ligado ou não.
fn tela(com_handler: bool, ctrl_v: bool) -> (TestApp, LoopbackHandle, Colagens) {
    let conexao = Loopback::new();
    let handle = conexao.handle();
    let colagens = Colagens::default();
    let guardadas = Rc::clone(&colagens);
    let mut app = TestApp::new(move || {
        let mut terminal = Terminal::new(conexao).id("console").cols_rows(80, 24);
        if com_handler {
            let guardadas = Rc::clone(&guardadas);
            terminal = terminal
                .on_rich_paste(move |colagem| guardadas.borrow_mut().push(colagem))
                .rich_ctrl_v(ctrl_v);
        }
        Form::new("Console").child(Column::new().padding(16).child(terminal))
    })
    .size(780, 560);
    app.click("console");
    (app, handle, colagens)
}

fn escrito(handle: &LoopbackHandle) -> String {
    String::from_utf8(handle.take_written()).unwrap()
}

fn imagem() -> ClipboardImage {
    ClipboardImage {
        width: 2,
        height: 2,
        rgba: vec![255; 16],
    }
}

fn caminhos() -> Vec<PathBuf> {
    vec![PathBuf::from("/tmp/a b.txt"), PathBuf::from("/tmp/c")]
}

#[test]
fn arquivos_chegam_ao_handler_e_nada_vai_ao_programa() {
    let (mut app, handle, colagens) = tela(true, false);
    app.clipboard().write_files(caminhos());
    app.key_with(Key::Char('v'), COLAR);

    let colagens = colagens.borrow();
    assert_eq!(colagens.len(), 1);
    assert_eq!(colagens[0].content, RichPasteContent::Files(caminhos()));
    assert_eq!(colagens[0].source, RichPasteSource::Paste);
    assert_eq!(escrito(&handle), "");
}

#[test]
fn arquivos_e_texto_juntos_ainda_sao_arquivos() {
    let (mut app, handle, colagens) = tela(true, false);
    // Os gerenciadores de arquivos põem também o caminho como texto.
    app.clipboard().write("/tmp/a b.txt".into());
    app.clipboard().write_files(caminhos());
    app.key_with(Key::Char('v'), COLAR);
    assert_eq!(colagens.borrow().len(), 1);
    assert_eq!(escrito(&handle), "");
}

#[test]
fn so_texto_segue_o_fluxo_de_sempre() {
    let (mut app, handle, colagens) = tela(true, false);
    app.clipboard().write("ls -la".into());
    app.key_with(Key::Char('v'), COLAR);
    assert_eq!(escrito(&handle), "ls -la");
    assert!(colagens.borrow().is_empty());
}

#[test]
fn texto_e_imagem_colam_o_texto() {
    let (mut app, handle, colagens) = tela(true, false);
    // Uma planilha põe os dois.
    app.clipboard().write_image(imagem());
    app.clipboard().write("a\tb".into());
    app.key_with(Key::Char('v'), COLAR);
    assert_eq!(escrito(&handle), "a\tb");
    assert!(colagens.borrow().is_empty());
}

#[test]
fn so_imagem_chega_ao_handler_com_o_tamanho_certo() {
    let (mut app, handle, colagens) = tela(true, false);
    app.clipboard().write_image(imagem());
    app.key_with(Key::Char('v'), COLAR);

    let colagens = colagens.borrow();
    assert_eq!(colagens.len(), 1);
    assert_eq!(colagens[0].content, RichPasteContent::Image(imagem()));
    assert_eq!(colagens[0].source, RichPasteSource::Paste);
    assert_eq!(escrito(&handle), "");
}

#[test]
fn shift_insert_tambem_cola_a_imagem() {
    let (mut app, _handle, colagens) = tela(true, false);
    app.clipboard().write_image(imagem());
    app.key_with(Key::Insert, Modifiers::SHIFT);
    assert_eq!(colagens.borrow().len(), 1);
}

#[test]
fn ctrl_v_com_imagem_e_rich_ctrl_v_chega_ao_handler() {
    let (mut app, handle, colagens) = tela(true, true);
    app.clipboard().write_image(imagem());
    app.key_with(Key::Char('v'), Modifiers::CTRL);
    {
        let colagens = colagens.borrow();
        assert_eq!(colagens.len(), 1);
        assert_eq!(colagens[0].source, RichPasteSource::CtrlV);
        assert_eq!(colagens[0].content, RichPasteContent::Image(imagem()));
    }
    assert_eq!(escrito(&handle), "", "o ^V não vai junto");
}

#[test]
fn ctrl_v_com_texto_continua_indo_ao_programa() {
    let (mut app, handle, colagens) = tela(true, true);
    app.clipboard().write_image(imagem());
    app.clipboard().write("texto".into());
    app.key_with(Key::Char('v'), Modifiers::CTRL);
    assert_eq!(escrito(&handle), "\x16");
    assert!(colagens.borrow().is_empty());
}

#[test]
fn ctrl_v_com_rich_ctrl_v_desligado_vai_ao_programa_mesmo_com_imagem() {
    let (mut app, handle, colagens) = tela(true, false);
    app.clipboard().write_image(imagem());
    app.key_with(Key::Char('v'), Modifiers::CTRL);
    assert_eq!(escrito(&handle), "\x16");
    assert!(colagens.borrow().is_empty());
}

#[test]
fn ctrl_v_com_a_area_vazia_vai_ao_programa() {
    let (mut app, handle, colagens) = tela(true, true);
    app.key_with(Key::Char('v'), Modifiers::CTRL);
    assert_eq!(escrito(&handle), "\x16");
    assert!(colagens.borrow().is_empty());
}

#[test]
fn sem_handler_imagem_nao_faz_nada() {
    let (mut app, handle, _) = tela(false, false);
    app.clipboard().write_image(imagem());
    app.key_with(Key::Char('v'), COLAR);
    assert_eq!(escrito(&handle), "");
}

#[test]
fn sem_handler_arquivos_soltos_colam_os_caminhos_entre_aspas() {
    let (mut app, handle, _) = tela(false, false);
    app.drop_files("console", &caminhos());
    assert_eq!(escrito(&handle), "'/tmp/a b.txt' /tmp/c ");
}

#[test]
fn com_handler_arquivos_soltos_chegam_com_origem_drop() {
    let (mut app, handle, colagens) = tela(true, false);
    app.drop_files("console", &caminhos());
    let colagens = colagens.borrow();
    assert_eq!(colagens.len(), 1);
    assert_eq!(colagens[0].content, RichPasteContent::Files(caminhos()));
    assert_eq!(colagens[0].source, RichPasteSource::Drop);
    assert_eq!(escrito(&handle), "");
}

#[test]
fn o_item_colar_do_menu_tambem_entrega_a_imagem() {
    let conexao = Loopback::new();
    let handle = conexao.handle();
    let colagens = Colagens::default();
    let guardadas = Rc::clone(&colagens);
    let mut app = TestApp::new(move || {
        let guardadas = Rc::clone(&guardadas);
        Form::new("Console").child(
            Terminal::new(conexao)
                .id("console")
                .cols_rows(80, 24)
                .context_flyout(MenuFlyout::new().id("console-menu"))
                .context_copy_paste()
                .on_rich_paste(move |colagem| guardadas.borrow_mut().push(colagem)),
        )
    })
    .size(780, 560);
    app.click("console");
    app.clipboard().write_image(imagem());
    app.right_click("console");
    app.click("console-menu-colar");
    app.wait(std::time::Duration::from_millis(50));
    assert_eq!(colagens.borrow().len(), 1);
    assert_eq!(escrito(&handle), "");
}

#[test]
fn o_botao_direito_cola_a_imagem_quando_o_menu_nao_existe() {
    let conexao = Loopback::new();
    let handle = conexao.handle();
    let colagens = Colagens::default();
    let guardadas = Rc::clone(&colagens);
    let mut app = TestApp::new(move || {
        let guardadas = Rc::clone(&guardadas);
        Form::new("Console").child(
            Terminal::new(conexao)
                .id("console")
                .cols_rows(80, 24)
                .right_click(tomni_ui::terminal::RightClick::CopyPaste)
                .on_rich_paste(move |colagem| guardadas.borrow_mut().push(colagem)),
        )
    })
    .size(780, 560);
    app.click("console");
    app.clipboard().write_image(imagem());
    app.right_click("console");
    assert_eq!(colagens.borrow().len(), 1);
    assert_eq!(escrito(&handle), "");
}

#[test]
fn session_paste_respeita_o_bracketed_paste() {
    let conexao = Loopback::new();
    let handle = conexao.handle();
    let sessao = Session::new(conexao);
    let mut app = TestApp::new(move || {
        Form::new("Console").child(Terminal::session(sessao).id("console").cols_rows(80, 24))
    })
    .size(780, 560);

    sessao.paste("texto");
    assert_eq!(escrito(&handle), "texto");

    app.feed("console", "\x1b[?2004h");
    sessao.paste("texto");
    assert_eq!(escrito(&handle), "\x1b[200~texto\x1b[201~");
}

#[test]
fn com_envio_para_varios_o_handler_recebe_as_sessoes_do_router_sem_perguntar() {
    let (sessoes, handles): (Vec<Session>, Vec<LoopbackHandle>) = (1..=2)
        .map(|i| {
            let nome = format!("web-{i:02}");
            let loopback = Loopback::new();
            let handle = loopback.handle();
            (Session::new(loopback).key(nome.clone()).name(nome), handle)
        })
        .unzip();
    let colagens = Colagens::default();
    let (guardadas, dadas) = (Rc::clone(&colagens), sessoes.clone());
    let mut app = TestApp::new(move || {
        let layout = signal(PaneLayout::with(dadas.clone()));
        let envio = signal(Broadcast::All);
        let guardadas = Rc::clone(&guardadas);
        Form::new("Painéis").child(
            TerminalPanes::bind(layout)
                .id("paineis")
                .broadcast(envio)
                .terminal(move |terminal| {
                    let guardadas = Rc::clone(&guardadas);
                    terminal.on_rich_paste(move |colagem| guardadas.borrow_mut().push(colagem))
                })
                .grow(1),
        )
    })
    .size(900, 600);
    app.click("paineis-terminal-web-01");

    app.clipboard().write_image(imagem());
    app.key_with(Key::Char('v'), COLAR);

    let colagens = colagens.borrow();
    assert_eq!(colagens.len(), 1);
    assert_eq!(colagens[0].sessions.len(), 2);
    assert!(colagens[0].sessions.contains(&sessoes[0]));
    assert!(colagens[0].sessions.contains(&sessoes[1]));
    // Não abriu o diálogo de vários terminais, e nada foi escrito.
    assert!(!app.is_open("paineis-terminal-web-01-paste"));
    for handle in &handles {
        assert_eq!(escrito(handle), "");
    }
}
