//! Marco M47 (pedido pelo Tirech SSH): `RichText`, `TextEditor`, `MarkdownView`, clipboard de
//! imagem e de arquivos, imagens GIF e WebP.

mod comum;

use comum::theme_name;
use tomni_ui::kurbo::{Point, Vec2};
use tomni_ui::prelude::*;
use tomni_ui::theme::Theme;
use tomni_ui::{Cursor, Key, Modifiers};
use tomni_ui_test::TestApp;

#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/rich_text.rs"]
mod rich_text;

fn app<V: View>(view: impl FnOnce() -> V, theme: Theme) -> TestApp {
    TestApp::new(view).size(390, 844).theme(theme)
}

#[test]
fn rich_text_snapshots() {
    for theme in [Theme::Light, Theme::Dark] {
        app(rich_text::texto_rico, theme)
            .assert_snapshot(&format!("rich_text_{}", theme_name(theme)));
    }
}

#[test]
fn rich_text_tab_percorre_os_links_e_enter_abre() {
    let mut app = app(rich_text::texto_rico, Theme::Light);
    app.key(Key::Tab);
    assert_eq!(app.focused().as_deref(), Some("paragrafo"));
    app.key(Key::Enter);
    assert_eq!(app.text("aberto"), "https://exemplo.com/manual");
    app.key(Key::Tab);
    assert_eq!(
        app.focused().as_deref(),
        Some("paragrafo"),
        "o segundo link"
    );
    app.key(Key::Enter);
    assert_eq!(app.text("aberto"), "https://exemplo.com/lista");
    app.key(Key::Tab);
    assert_eq!(app.focused().as_deref(), Some("limpar"), "saiu do texto");
    app.key_with(Key::Tab, Modifiers::SHIFT);
    assert_eq!(
        app.focused().as_deref(),
        Some("paragrafo"),
        "volta pelo último link"
    );
    app.key(Key::Enter);
    assert_eq!(app.text("aberto"), "https://exemplo.com/lista");
}

#[test]
fn rich_text_hover_mostra_a_mao_e_marca_o_link() {
    let mut app = app(rich_text::texto_rico, Theme::Light);
    app.hover("paragrafo-link-0");
    let centro = app.tree().find("paragrafo-link-0").map(|n| {
        let [x, y, w, h] = n.rect;
        Point::new(x + w / 2.0, y + h / 2.0)
    });
    let centro = centro.unwrap();
    assert_eq!(app.ui().cursor(centro), Cursor::Hand);
    let tree = app.tree();
    assert_eq!(tree.find("paragrafo-link-0").unwrap().state, Some("hover"));
    assert_eq!(tree.find("paragrafo-link-1").unwrap().state, Some("rest"));
    // Fora do link, no mesmo texto, a seta.
    let paragrafo = app.rect("paragrafo");
    app.ui().pointer_move(
        Point::new(paragrafo.x0 + 2.0, paragrafo.y0 + 4.0),
        tomni_ui::PointerKind::Mouse,
    );
    assert_eq!(
        app.ui()
            .cursor(Point::new(paragrafo.x0 + 2.0, paragrafo.y0 + 4.0)),
        Cursor::Default
    );
}

// ---------------------------------------------------------------------------------------------
// TextEditor
// ---------------------------------------------------------------------------------------------

#[allow(dead_code, reason = "o `main` do exemplo não é usado no teste")]
#[path = "../../tomni-ui/examples/text_editor.rs"]
mod text_editor;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use tomni_ui::{ClipboardImage, TextEditor, TextEditorController};

/// Um editor de 200 de altura, ligado a um signal com o texto `inicial`, e um botão para o
/// foco ir para fora.
fn editor_app(inicial: &str) -> (TestApp, Signal<String>) {
    let inicial = inicial.to_owned();
    let texto = Rc::new(RefCell::new(None));
    let guarda = Rc::clone(&texto);
    let app = TestApp::new(move || {
        let texto = signal(inicial.clone());
        *guarda.borrow_mut() = Some(texto);
        Form::new("Editor").child(
            Column::new()
                .padding(16)
                .gap(8)
                .child(TextEditor::bind(texto).id("editor").min_height(200.0))
                .child(Button::new("Fora").id("fora")),
        )
    })
    .size(390, 844)
    .theme(Theme::Light);
    let texto = texto.borrow().expect("a view foi montada");
    (app, texto)
}

#[test]
fn editor_digita_quebra_linha_e_desfaz() {
    let (mut app, texto) = editor_app("");
    app.type_into("editor", "olá\nmundo");
    assert_eq!(texto.get_untracked(), "olá\nmundo");
    assert_eq!(app.text_editor("editor").caret, Some("olá\nmundo".len()));
    app.shortcut('z');
    assert_eq!(
        texto.get_untracked(),
        "olá\n",
        "a digitação seguida é um passo só"
    );
    app.shortcut('z');
    assert_eq!(texto.get_untracked(), "olá");
    app.shortcut(Shortcut::ctrl_shift('Z'));
    assert_eq!(texto.get_untracked(), "olá\n");
    app.shortcut('y');
    assert_eq!(texto.get_untracked(), "olá\nmundo");
}

#[test]
fn editor_seleciona_com_teclado_e_apaga() {
    let (mut app, texto) = editor_app("um dois\ntrês");
    app.click("editor");
    app.key_with(Key::Home, Modifiers::COMMAND);
    app.key_with(Key::Right, Modifiers::SHIFT);
    app.key_with(Key::Right, Modifiers::SHIFT);
    assert_eq!(app.text_editor("editor").selection, Some(0..2));
    app.key(Key::Delete);
    assert_eq!(texto.get_untracked(), " dois\ntrês");
    app.shortcut('a');
    assert_eq!(
        app.text_editor("editor").selection,
        Some(0..texto.get_untracked().len())
    );
    app.type_text("x");
    assert_eq!(texto.get_untracked(), "x");
}

#[test]
fn editor_duplo_e_triplo_clique_e_shift_clique() {
    let (mut app, _texto) = editor_app("alfa beta\ngama delta");
    let r = app.rect("editor");
    let ponto = Point::new(r.x0 + 14.0, r.y0 + 12.0);
    app.click_at(ponto, 1);
    app.click_at(ponto, 2);
    let sel = app.text_editor("editor").selection.unwrap();
    assert_eq!(&"alfa beta\ngama delta"[sel], "alfa");
    app.click_at(ponto, 3);
    let sel = app.text_editor("editor").selection.unwrap();
    assert_eq!(
        &"alfa beta\ngama delta"[sel], "alfa beta",
        "a linha, sem o \\n"
    );
    // Shift+clique estende a partir do cursor.
    app.key_with(Key::Home, Modifiers::COMMAND);
    app.ui().set_modifiers(Modifiers::SHIFT);
    app.click_at(Point::new(r.x0 + 60.0, r.y0 + 12.0 + 20.0), 1);
    app.ui().set_modifiers(Modifiers::NONE);
    let sel = app.text_editor("editor").selection.unwrap();
    assert!(
        sel.start == 0 && sel.end > 10,
        "pegou a segunda linha: {sel:?}"
    );
}

#[test]
fn editor_cola_texto_e_o_menu_de_contexto_age() {
    let (mut app, texto) = editor_app("abc");
    app.clipboard().write("XYZ".into());
    app.click("editor");
    app.key_with(Key::End, Modifiers::COMMAND);
    app.shortcut('v');
    assert_eq!(texto.get_untracked(), "abcXYZ");
    app.shortcut('a');
    app.shortcut('c');
    assert_eq!(app.clipboard().read().as_deref(), Some("abcXYZ"));
    app.right_click("editor");
    app.click("editor-selecionar-tudo");
    app.right_click("editor");
    app.click("editor-recortar");
    assert_eq!(texto.get_untracked(), "");
    assert_eq!(app.clipboard().read().as_deref(), Some("abcXYZ"));
    app.right_click("editor");
    app.click("editor-colar");
    assert_eq!(texto.get_untracked(), "abcXYZ");
    app.right_click("editor");
    app.click("editor-desfazer");
    assert_eq!(texto.get_untracked(), "");
}

/// Como `editor_app`, mas deixa o teste configurar o editor (`.on_key`, `.read_only`...).
fn editor_com(
    inicial: &str,
    configura: impl FnOnce(TextEditor) -> TextEditor + 'static,
) -> (TestApp, Signal<String>) {
    let inicial = inicial.to_owned();
    let texto = Rc::new(RefCell::new(None));
    let guarda = Rc::clone(&texto);
    let app = TestApp::new(move || {
        let texto = signal(inicial.clone());
        *guarda.borrow_mut() = Some(texto);
        Form::new("Editor").child(
            Column::new()
                .padding(16)
                .gap(8)
                .child(configura(
                    TextEditor::bind(texto).id("editor").min_height(120.0),
                ))
                .child(Button::new("Fora").id("fora")),
        )
    })
    .size(390, 844)
    .theme(Theme::Light);
    let texto = texto.borrow().expect("a view foi montada");
    (app, texto)
}

#[test]
fn editor_on_key_consome_tab_e_enter() {
    let (mut app, texto) = editor_com("- item", |e| {
        e.on_key(|tecla, controle| match tecla.key {
            Key::Tab => {
                controle.replace_selection("    ");
                true
            }
            Key::Enter if tecla.modifiers == Modifiers::NONE => {
                controle.replace_selection("\n- ");
                true
            }
            _ => false,
        })
    });
    app.click("editor");
    app.key_with(Key::End, Modifiers::COMMAND);
    app.key(Key::Tab);
    assert_eq!(
        app.focused().as_deref(),
        Some("editor"),
        "o Tab não saiu do campo"
    );
    assert_eq!(texto.get_untracked(), "- item    ");
    app.key(Key::Enter);
    assert_eq!(texto.get_untracked(), "- item    \n- ");
    // Sem consumir, o Shift+Tab ainda sai do campo.
    app.shift_tab();
    assert_ne!(app.focused().as_deref(), Some("editor"));
}

#[test]
fn editor_on_copy_substitui_a_area_de_transferencia_e_recortar_apaga() {
    let copiado = Rc::new(RefCell::new(Vec::<String>::new()));
    let guarda = Rc::clone(&copiado);
    let (mut app, texto) = editor_com("segredo público", move |e| {
        e.on_copy(move |t| guarda.borrow_mut().push(t))
    });
    app.clipboard().write("antes".into());
    app.click("editor");
    app.key_with(Key::Home, Modifiers::COMMAND);
    for _ in 0..7 {
        app.key_with(Key::Right, Modifiers::SHIFT);
    }
    app.shortcut('c');
    assert_eq!(*copiado.borrow(), vec!["segredo".to_owned()]);
    assert_eq!(
        app.clipboard().read().as_deref(),
        Some("antes"),
        "o sistema não foi tocado"
    );
    app.shortcut('x');
    assert_eq!(copiado.borrow().len(), 2);
    assert_eq!(
        texto.get_untracked(),
        " público",
        "recortar apaga a seleção"
    );
}

#[test]
fn editor_somente_leitura_e_desabilitado_nao_editam() {
    let (mut app, texto) = editor_com("fixo", |e| e.read_only(true));
    app.click("editor");
    app.type_text("x");
    app.key(Key::Backspace);
    app.shortcut('v');
    assert_eq!(texto.get_untracked(), "fixo");
    // Seleciona e copia mesmo assim.
    app.shortcut('a');
    app.shortcut('c');
    assert_eq!(app.clipboard().read().as_deref(), Some("fixo"));
    assert_eq!(
        app.text_editor("editor").caret,
        None,
        "sem cursor no só leitura"
    );

    let (mut app, texto) = editor_com("preso", |e| e.disabled(true));
    app.click("editor");
    assert_ne!(
        app.focused().as_deref(),
        Some("editor"),
        "desabilitado não ganha foco"
    );
    app.type_text("x");
    assert_eq!(texto.get_untracked(), "preso");
}

#[test]
fn editor_cola_arquivos_e_imagem_pelos_callbacks() {
    let arquivos = Rc::new(RefCell::new(Vec::<PathBuf>::new()));
    let imagens = Rc::new(RefCell::new(Vec::<(u32, u32)>::new()));
    let (a, i) = (Rc::clone(&arquivos), Rc::clone(&imagens));
    let (mut app, texto) = editor_com("nota ", move |e| {
        e.on_paste_files(move |caminhos, controle| {
            a.borrow_mut().extend(caminhos);
            controle.replace_selection("[arquivo]");
        })
        .on_paste_image(move |imagem, controle| {
            i.borrow_mut().push((imagem.width, imagem.height));
            controle.replace_selection("[imagem]");
        })
    });
    app.click("editor");
    app.key_with(Key::End, Modifiers::COMMAND);
    // Com texto na área de transferência, é o texto que entra.
    app.clipboard().write("só texto".into());
    app.shortcut('v');
    assert_eq!(texto.get_untracked(), "nota só texto");
    // Arquivos têm preferência sobre o texto.
    app.clipboard().write_files(vec![
        PathBuf::from("/tmp/a.txt"),
        PathBuf::from("/tmp/b.png"),
    ]);
    app.shortcut('v');
    assert_eq!(texto.get_untracked(), "nota só texto[arquivo]");
    assert_eq!(arquivos.borrow().len(), 2);
    // Sem texto nem arquivos, a imagem.
    app.clipboard().write_image(ClipboardImage {
        width: 2,
        height: 1,
        rgba: vec![255; 8],
    });
    app.shortcut('v');
    assert_eq!(texto.get_untracked(), "nota só texto[arquivo][imagem]");
    assert_eq!(*imagens.borrow(), vec![(2, 1)]);
}

#[test]
fn editor_recebe_arquivos_soltos_e_o_cursor_vai_ao_ponto() {
    let soltos = Rc::new(RefCell::new(Vec::<PathBuf>::new()));
    let guarda = Rc::clone(&soltos);
    let (mut app, texto) = editor_com("linha um\nlinha dois", move |e| {
        e.on_files_dropped(move |caminhos, controle| {
            guarda.borrow_mut().extend(caminhos);
            controle.replace_selection("<anexo>");
        })
    });
    let r = app.rect("editor");
    let ponto = Point::new(r.x0 + 200.0, r.y0 + 12.0);
    app.ui()
        .files_dropped(vec![PathBuf::from("/tmp/x.pdf")], ponto);
    assert_eq!(*soltos.borrow(), vec![PathBuf::from("/tmp/x.pdf")]);
    assert_eq!(
        texto.get_untracked(),
        "linha um<anexo>\nlinha dois",
        "no fim da primeira linha"
    );
}

#[test]
fn editor_controlador_troca_trechos_seleciona_busca_e_rola() {
    let controle = TextEditorController::new();
    let c = controle.clone();
    let mudou = Rc::new(RefCell::new(0));
    let m = Rc::clone(&mudou);
    let (mut app, texto) = editor_com(
        "um\ndois\ntrês\nquatro\ncinco\nseis\nsete\noito\nnove\ndez",
        move |e| {
            e.controller(c)
                .max_height(100.0)
                .on_change(move || *m.borrow_mut() += 1)
        },
    );
    assert_eq!(controle.value(), texto.get_untracked());
    controle.replace(0..2, "UM");
    assert!(texto.get_untracked().starts_with("UM\ndois"));
    app.frame();
    assert_eq!(*mudou.borrow(), 1);
    controle.set_selection(3..7);
    assert_eq!(controle.selection(), 3..7);
    assert_eq!(
        app.text_editor("editor").selection,
        None,
        "sem foco não há seleção visível"
    );
    controle.focus();
    app.frame();
    assert_eq!(app.focused().as_deref(), Some("editor"));
    assert_eq!(app.text_editor("editor").selection, Some(3..7));
    assert_eq!(controle.find("TRÊS", true).len(), 1);
    assert_eq!(controle.find("TRÊS", false).len(), 0);
    assert_eq!(
        controle.find("tres", true).len(),
        1,
        "a busca ignora o acento"
    );
    // Rola até o fim.
    let fim = texto.get_untracked().len();
    controle.scroll_to(fim);
    app.frame();
    assert!(app.text_editor("editor").scroller.offset().y > 0.0);
}

#[test]
fn editor_quebra_linhas_longas_rola_com_a_roda_e_cresce_ate_o_maximo() {
    let longo = "palavra ".repeat(60);
    let (mut app, _texto) = editor_com(&longo, |e| e.wrap(true).max_height(150.0));
    let visual = app.text_editor("editor");
    assert!(
        visual.scroller.content().height > visual.scroller.viewport().height,
        "quebrou em várias linhas"
    );
    assert_eq!(
        visual.scroller.max_offset().x,
        0.0,
        "com quebra não rola na horizontal"
    );
    let r = app.rect("editor");
    assert_eq!(r.height(), 150.0, "parou no máximo");
    app.wheel(Point::new(r.x0 + 40.0, r.y0 + 40.0), Vec2::new(0.0, 80.0));
    assert!(app.text_editor("editor").scroller.offset().y > 0.0);

    // Sem quebra, a linha comprida rola na horizontal.
    let (app, _texto) = editor_com(&longo, |e| e.wrap(false).max_height(150.0));
    assert!(app.text_editor("editor").scroller.max_offset().x > 0.0);

    // Com pouco texto o campo fica no mínimo; com muito, cresce até o máximo.
    let (mut app, _texto) = editor_com("a", |e| e.min_height(60.0).max_height(150.0));
    assert_eq!(app.rect("editor").height(), 60.0);
    app.type_into("editor", "\n\n\n\n\n\n\n\n\n\n");
    assert_eq!(app.rect("editor").height(), 150.0);
}

#[test]
fn editor_aceita_o_ime_e_aparece_na_arvore_como_text_editor() {
    let (mut app, texto) = editor_app("ola ");
    app.click("editor");
    app.key_with(Key::End, Modifiers::COMMAND);
    app.ime_preedit("かん");
    assert_eq!(texto.get_untracked(), "ola ", "a composição não grava");
    assert!(app.text_editor("editor").composing.is_some());
    app.ime_commit("漢");
    assert_eq!(texto.get_untracked(), "ola 漢");
    assert!(app.text_editor("editor").composing.is_none());
    let arvore = app.tree();
    let no = arvore.find("editor").unwrap();
    assert_eq!(no.kind, "text_editor");
    assert_eq!(no.text.as_deref(), Some("ola 漢"));
    assert!(app.text_input().is_some(), "liga o IME na plataforma");
}

#[test]
fn text_editor_snapshots() {
    for theme in [Theme::Light, Theme::Dark] {
        app(text_editor::editor_de_texto, theme)
            .assert_snapshot(&format!("text_editor_{}", theme_name(theme)));
    }
}

#[test]
fn text_editor_snapshots_com_busca_e_selecao() {
    for theme in [Theme::Light, Theme::Dark] {
        let mut app = app(text_editor::editor_de_texto, theme);
        app.click("achar");
        app.assert_snapshot(&format!("text_editor_busca_{}", theme_name(theme)));
    }
}

// ---------------------------------------------------------------------------------------------
// Imagens: GIF, WebP, `probe` e `natural_size`
// ---------------------------------------------------------------------------------------------

const GIF: &[u8] = include_bytes!("../../render/tests/data/dois.gif");
const WEBP_SEM_PERDA: &[u8] = include_bytes!("../../render/tests/data/lossless.webp");
const WEBP_COM_PERDA: &[u8] = include_bytes!("../../render/tests/data/lossy.webp");
const SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="30"/>"#;

#[test]
fn probe_de_cada_formato_e_de_arquivo_cortado() {
    use tomni_ui::image::probe;
    assert_eq!(probe(GIF), Ok((3, 2)));
    assert_eq!(probe(WEBP_SEM_PERDA), Ok((8, 4)));
    assert_eq!(probe(WEBP_COM_PERDA), Ok((8, 4)));
    assert_eq!(probe(SVG), Ok((40, 30)));
    let png = ClipboardImage {
        width: 5,
        height: 7,
        rgba: vec![255; 5 * 7 * 4],
    }
    .to_png()
    .unwrap();
    assert_eq!(probe(&png), Ok((5, 7)));
    assert!(probe(&png[..20]).unwrap_err().starts_with("PNG inválido"));
    assert!(
        probe(&WEBP_COM_PERDA[..20])
            .unwrap_err()
            .starts_with("WebP inválido")
    );
    assert!(probe(b"").is_err());
    assert_eq!(
        probe(b"BMxxxxxxxx").unwrap_err(),
        "formato de imagem desconhecido: só PNG, JPEG, GIF, WebP e SVG"
    );
}

/// Uma imagem de 200 × 100 informada (os bytes são um SVG de 40 × 30: a medida não os decodifica)
/// numa coluna de `largura`.
fn coluna_com_imagem(largura: u32) -> TestApp {
    TestApp::new(move || {
        Form::new("Imagem").child(
            Column::new().padding(0).child(
                Column::new().width(largura).align(Align::Start).child(
                    Image::bytes(SVG)
                        .id("figura")
                        .natural_size(200, 100)
                        .alt("figura"),
                ),
            ),
        )
    })
    .size(500, 400)
}

#[test]
fn natural_size_ajusta_a_imagem_a_largura_da_coluna() {
    let larga = coluna_com_imagem(300);
    let r = larga.rect("figura");
    assert_eq!(
        (r.width(), r.height()),
        (200.0, 100.0),
        "cabe: tamanho natural"
    );
    let estreita = coluna_com_imagem(120);
    let r = estreita.rect("figura");
    assert_eq!(
        (r.width(), r.height()),
        (120.0, 60.0),
        "encolhe na proporção"
    );
    assert_eq!(
        estreita.tree().find("figura").unwrap().natural_size,
        Some([200, 100])
    );
}

#[test]
fn imagem_gif_e_webp_aparecem_na_arvore_sem_erro() {
    for (id, bytes) in [
        ("gif", GIF),
        ("webp", WEBP_COM_PERDA),
        ("webp2", WEBP_SEM_PERDA),
    ] {
        let app = TestApp::new(move || {
            Form::new("Imagem").child(Image::bytes(bytes).id(id).alt("figura"))
        });
        let no = app.tree();
        let no = no.find(id).unwrap();
        assert_eq!(no.error, None, "{id}");
        let r = app.rect(id);
        assert!(r.width() > 0.0 && r.height() > 0.0, "{id}: {r:?}");
    }
    let app = TestApp::new(|| {
        Form::new("Imagem").child(Image::bytes(&b"BMxxxxxxxx"[..]).id("ruim").alt("ruim"))
    });
    assert_eq!(
        app.tree().find("ruim").unwrap().error.as_deref(),
        Some("formato de imagem desconhecido: só PNG, JPEG, GIF, WebP e SVG")
    );
}

// ---- MarkdownView ----

use tomni_ui::markdown::MarkdownView;

/// Um documento com um exemplo de cada elemento.
const DOCUMENTO: &str = "\
# Título um

## Título dois

### Título três

Um parágrafo com **negrito**, *itálico*, ~~riscado~~, `código` e um [link](https://tirech.com.br).

- [x] Tarefa feita
- [ ] Tarefa aberta

Itens e passos:

- Item
  - Subitem
  - Outro
    - Fundo

1. Primeiro
2. Segundo

> Uma citação
> em duas linhas

---

```sh
sudo systemctl restart nginx
```

| Servidor | Porta | Estado |
|:---------|:-----:|-------:|
| web-01 | 22 | ativo |
| db-01 | 2222 | parado |

<div>HTML cru</div>
";

fn markdown_app(fonte: &'static str, config: impl FnOnce(MarkdownView) -> MarkdownView) -> TestApp {
    TestApp::new(move || {
        Form::new("Markdown").child(
            Column::new()
                .padding(16)
                .child(config(MarkdownView::new(fonte).id("md"))),
        )
    })
    .size(520, 900)
}

#[test]
fn markdown_tarefa_chama_on_task_com_o_byte_do_colchete() {
    let fonte = "- [ ] um\n- [x] dois\n";
    let chamadas = Rc::new(RefCell::new(Vec::<(usize, bool)>::new()));
    let guarda = Rc::clone(&chamadas);
    let mut app = markdown_app(fonte, move |m| {
        m.on_task(move |byte, marcada| guarda.borrow_mut().push((byte, marcada)))
    });
    assert!(app.enabled("md-tarefa-0"));
    assert!(!app.checked("md-tarefa-0"));
    assert!(app.checked("md-tarefa-1"));
    app.click("md-tarefa-1");
    let segundo = fonte.find("[x]").unwrap();
    assert_eq!(*chamadas.borrow(), vec![(segundo, false)]);
    app.click("md-tarefa-0");
    assert_eq!(chamadas.borrow()[1], (fonte.find("[ ]").unwrap(), true));
}

#[test]
fn markdown_sem_on_task_deixa_as_tarefas_desabilitadas() {
    let app = markdown_app("- [ ] um\n", |m| m);
    assert!(!app.enabled("md-tarefa-0"));
}

#[test]
fn markdown_tarefa_alterando_a_fonte_remonta_sem_perder_o_estado() {
    let fonte = signal(String::from("- [ ] um\n- [ ] dois\n"));
    let mut app = TestApp::new(move || {
        Form::new("Markdown").child(Column::new().padding(16).child(
            MarkdownView::new(fonte).id("md").on_task(move |byte, _| {
                let atual = fonte.get_untracked();
                if let Some(troca) = tomni_ui::markdown::edit::toggle_task(&atual, byte) {
                    fonte.set(troca.apply(&atual));
                }
            }),
        ))
    })
    .size(520, 400);
    app.click("md-tarefa-1");
    assert_eq!(fonte.get_untracked(), "- [ ] um\n- [x] dois\n");
    assert!(app.checked("md-tarefa-1"));
    assert!(!app.checked("md-tarefa-0"));
    app.click("md-tarefa-0");
    assert_eq!(fonte.get_untracked(), "- [x] um\n- [x] dois\n");
    app.click("md-tarefa-1");
    assert_eq!(fonte.get_untracked(), "- [x] um\n- [ ] dois\n");
}

#[test]
fn markdown_copiar_chama_on_copy_com_o_texto_do_bloco() {
    let copiados = Rc::new(RefCell::new(Vec::<String>::new()));
    let guarda = Rc::clone(&copiados);
    let mut app = markdown_app(
        "```sh\nls -la\necho fim\n```\n\n```\nsegundo\n```\n",
        move |m| m.on_copy(move |texto| guarda.borrow_mut().push(texto)),
    );
    assert_eq!(app.label("md-copiar-0"), "Copiar");
    app.click("md-copiar-0");
    app.click("md-copiar-1");
    assert_eq!(*copiados.borrow(), ["ls -la\necho fim", "segundo"]);
}

#[test]
fn markdown_copiar_sem_on_copy_grava_na_area_de_transferencia() {
    let mut app = markdown_app("```\nsegredo\n```\n", |m| m);
    app.click("md-copiar-0");
    assert_eq!(app.clipboard().read().as_deref(), Some("segredo"));
}

#[test]
fn markdown_fence_troca_o_bloco_pela_linguagem() {
    let mut app = markdown_app("```aviso\nCuidado\n```\n\n```sh\nls\n```\n", |m| {
        m.fence("aviso", |cx| {
            assert_eq!(cx.lang, "aviso");
            assert_eq!(cx.text, "Cuidado");
            assert_eq!(cx.index, 0);
            Box::new(Text::new(format!("AVISO: {}", cx.text)).id(cx.id))
        })
    });
    assert_eq!(app.text("md-bloco-0"), "AVISO: Cuidado");
    assert_eq!(app.tree().find("md-bloco-0").unwrap().kind, "text");
    assert!(
        app.tree().find("md-copiar-0").is_none(),
        "o bloco trocado não tem botão"
    );
    assert_eq!(app.tree().find("md-bloco-1").unwrap().kind, "rich_text");
    app.click("md-copiar-1");
}

#[test]
fn code_action_chama_com_o_bloco() {
    let chamadas = Rc::new(RefCell::new(Vec::<(usize, String, String)>::new()));
    let guarda = Rc::clone(&chamadas);
    let outra = Rc::clone(&chamadas);
    let mut app = markdown_app(
        "```sh\nls -la\n```\n\n```aviso\nCuidado\n```\n\n```\nsegundo\n```\n",
        move |m| {
            m.fence("aviso", |cx| Box::new(Text::new(cx.text).id(cx.id)))
                .code_action("Inserir no terminal", Symbol::Send, move |cx| {
                    guarda.borrow_mut().push((cx.index, cx.lang, cx.text))
                })
                .code_action("Outra", Symbol::Edit, move |cx| {
                    outra.borrow_mut().push((cx.index + 100, cx.lang, cx.text))
                })
        },
    );
    assert_eq!(app.label("md-acao-0-0"), "Inserir no terminal");
    assert_eq!(app.label("md-acao-1-0"), "Outra");
    assert!(
        app.tree().find("md-acao-0-1").is_none(),
        "o bloco que a fence desenha fica sem ações"
    );
    app.click("md-acao-0-0");
    app.click("md-acao-0-2");
    app.click("md-acao-1-2");
    assert_eq!(
        *chamadas.borrow(),
        [
            (0, "sh".to_owned(), "ls -la".to_owned()),
            (2, String::new(), "segundo".to_owned()),
            (102, String::new(), "segundo".to_owned()),
        ]
    );
    // O "Copiar" continua lá.
    app.click("md-copiar-0");
    assert_eq!(app.clipboard().read().as_deref(), Some("ls -la"));
}

#[test]
fn markdown_bloco_com_acao_snapshots() {
    for theme in [Theme::Light, Theme::Dark] {
        markdown_app("```sh\nls -la /var/log\n```\n", |m| {
            m.code_action("Inserir no terminal", Symbol::Send, |_| {})
        })
        .theme(theme)
        .size(420, 140)
        .assert_snapshot(&format!("markdown_bloco_com_acao_{}", theme_name(theme)));
    }
}

#[test]
fn markdown_html_aparece_como_texto() {
    let app = markdown_app("<div>cru</div>\n", |m| m);
    let arvore = format!("{:?}", app.tree());
    assert!(
        arvore.contains("<div>cru</div>"),
        "o HTML é texto: {arvore}"
    );
}

#[test]
fn markdown_editar_um_bloco_mantem_os_widgets_dos_outros() {
    // Digitar num documento grande não pode montar o documento de novo: só o bloco que mudou.
    let fonte = signal(String::from(
        "# Título\n\nParágrafo um.\n\n```\nprimeiro\n```\n\nParágrafo dois.\n\n```\nsegundo\n```\n",
    ));
    let mut app = TestApp::new(move || {
        Form::new("Markdown").child(
            Column::new()
                .padding(16)
                .child(MarkdownView::new(fonte).id("md")),
        )
    })
    .size(520, 600);
    let achar = |app: &mut TestApp, id: &str| app.ui().tree().find(id).expect(id);
    let primeiro = achar(&mut app, "md-bloco-0");
    let segundo = achar(&mut app, "md-bloco-1");

    fonte.update(|f| *f = f.replace("Parágrafo dois.", "Parágrafo dois, editado."));
    app.frame();
    assert_eq!(
        achar(&mut app, "md-bloco-0"),
        primeiro,
        "o bloco de antes ficou"
    );
    assert_eq!(
        achar(&mut app, "md-bloco-1"),
        segundo,
        "o bloco de depois ficou"
    );
    assert!(
        format!("{:?}", app.tree()).contains("Parágrafo dois, editado."),
        "o parágrafo editado mudou"
    );

    // Um bloco de código novo no começo muda os números dos de depois: eles são montados de
    // novo, com os ids certos.
    fonte.update(|f| f.insert_str(0, "```\nnovo\n```\n\n"));
    app.frame();
    app.click("md-copiar-0");
    assert_eq!(app.clipboard().read().as_deref(), Some("novo"));
    app.click("md-copiar-1");
    assert_eq!(app.clipboard().read().as_deref(), Some("primeiro"));
    app.click("md-copiar-2");
    assert_eq!(app.clipboard().read().as_deref(), Some("segundo"));
    assert!(app.ui().tree().find("md-copiar-3").is_none());
}

#[test]
fn markdown_numera_links_blocos_e_imagens_no_documento_todo() {
    let fonte =
        "[a](u1) e [b](u2)\n\n```\nx\n```\n\n[c](u3)\n\n```\ny\n```\n\n![img](p.png)\n\n- [ ] t\n";
    let abertos = Rc::new(RefCell::new(Vec::<String>::new()));
    let guarda = Rc::clone(&abertos);
    let mut app = markdown_app(fonte, move |m| {
        m.on_link(move |url| guarda.borrow_mut().push(url))
    });
    let tree = app.tree();
    for id in [
        "md-link-0",
        "md-link-1",
        "md-link-2",
        "md-link-3",
        "md-bloco-0",
        "md-bloco-1",
        "md-copiar-0",
        "md-copiar-1",
        "md-imagem-0",
        "md-tarefa-0",
    ] {
        assert!(tree.find(id).is_some(), "falta {id}");
    }
    app.click("md-link-1");
    app.click("md-link-2");
    app.click("md-link-3");
    assert_eq!(*abertos.borrow(), ["u2", "u3", "p.png"]);
}

#[test]
fn markdown_imagem_sem_image_vira_link_com_o_texto_alternativo() {
    let abertos = Rc::new(RefCell::new(Vec::<String>::new()));
    let guarda = Rc::clone(&abertos);
    let mut app = markdown_app("![uma foto](x.png)\n", move |m| {
        m.on_link(move |url| guarda.borrow_mut().push(url))
    });
    assert_eq!(app.text("md-imagem-0"), "uma foto");
    app.click("md-link-0");
    assert_eq!(*abertos.borrow(), ["x.png"]);
}

#[test]
fn markdown_image_devolvendo_widget_fica_depois_do_texto_do_paragrafo() {
    let vistas = Rc::new(RefCell::new(
        Vec::<(String, String, Option<String>, usize)>::new(),
    ));
    let guarda = Rc::clone(&vistas);
    let app = markdown_app("Antes ![foto](a.png \"titulo\") depois\n", move |m| {
        m.image(move |cx| {
            guarda
                .borrow_mut()
                .push((cx.url.clone(), cx.alt.clone(), cx.title.clone(), cx.index));
            Some(Box::new(
                Text::new(format!("IMAGEM {}", cx.url)).id(cx.id.clone()),
            ))
        })
    });
    assert_eq!(
        *vistas.borrow(),
        [("a.png".into(), "foto".into(), Some("titulo".into()), 0)]
    );
    assert_eq!(app.text("md-imagem-0"), "IMAGEM a.png");
    let imagem = app.rect("md-imagem-0");
    let inicio = app.rect("md").y0;
    assert!(
        imagem.y0 >= inicio + 20.0,
        "a imagem vem depois do texto do parágrafo: {imagem:?}"
    );
}

#[test]
fn markdown_image_devolvendo_none_cai_no_padrao() {
    let abertos = Rc::new(RefCell::new(Vec::<String>::new()));
    let guarda = Rc::clone(&abertos);
    let mut app = markdown_app("![alt](x.png)\n\nTexto ![meio](y.png) fim\n", move |m| {
        m.image(|_| None)
            .on_link(move |url| guarda.borrow_mut().push(url))
    });
    assert_eq!(app.text("md-imagem-0"), "alt");
    app.click("md-link-0");
    assert_eq!(*abertos.borrow(), ["x.png"]);
}

#[test]
fn markdown_on_files_dropped_consome_e_sem_ele_o_evento_sobe() {
    let soltos = Rc::new(RefCell::new(Vec::<PathBuf>::new()));
    let guarda = Rc::clone(&soltos);
    let mut app = markdown_app("texto\n", move |m| {
        m.on_files_dropped(move |caminhos| guarda.borrow_mut().extend(caminhos))
    });
    app.drop_files("md", &[PathBuf::from("/tmp/a.pdf")]);
    assert_eq!(*soltos.borrow(), [PathBuf::from("/tmp/a.pdf")]);

    // Sem o handler, o evento chega ao `MarkdownView` de fora.
    let pai = Rc::new(RefCell::new(Vec::<PathBuf>::new()));
    let guarda = Rc::clone(&pai);
    let mut app = markdown_app("```aviso\nx\n```\n", move |m| {
        m.fence("aviso", |_| {
            Box::new(MarkdownView::new("dentro").id("interno"))
        })
        .on_files_dropped(move |caminhos| guarda.borrow_mut().extend(caminhos))
    });
    app.drop_files("interno", &[PathBuf::from("/tmp/b.pdf")]);
    assert_eq!(*pai.borrow(), [PathBuf::from("/tmp/b.pdf")]);
}

#[test]
fn markdown_tabela_rola_na_horizontal_quando_e_larga() {
    let fonte: &'static str = "| a | b | c | d |\n|---|---|---|---|\n| muito comprido um | muito comprido dois | muito comprido três | muito comprido quatro |\n";
    let app = TestApp::new(move || {
        Form::new("Markdown").child(
            Column::new()
                .padding(16)
                .child(MarkdownView::new(fonte).id("md")),
        )
    })
    .size(300, 300);
    let area = app.rect("md-tabela-0");
    assert!(area.width() <= 300.0);
    assert!(
        app.scroll_content("md-tabela-0").width > area.width(),
        "a tabela larga rola"
    );
}

#[test]
fn markdown_snapshots() {
    for theme in [Theme::Light, Theme::Dark] {
        markdown_app(DOCUMENTO, |m| m.on_task(|_, _| {}))
            .theme(theme)
            .assert_snapshot(&format!("markdown_{}", theme_name(theme)));
    }
}
