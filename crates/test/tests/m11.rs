//! Marco M11: o `NumberBox` e o botão de revelar a senha, nas cores, medidas e comportamento do
//! WinUI 3 (com o `fluent_ui`, o pacote Flutter do Fluent, onde o WinUI não serve). Os snapshots
//! nos dois temas, a roda do mouse, a repetição dos botões de incremento e os modos de revelar.

mod comum;

use comum::theme_name;
use tomni_ui::kurbo::Vec2;
use tomni_ui::prelude::*;
use tomni_ui::theme::Theme;
use tomni_ui::{Key, PASSWORD_CHAR, SPIN_REPEAT};
use tomni_ui_test::TestApp;

const TEMAS: [Theme; 2] = [Theme::Light, Theme::Dark];

fn mascara(n: usize) -> String {
    PASSWORD_CHAR.to_string().repeat(n)
}

/// Um `NumberBox` com o valor `inicial`, ajustado por `ajuste`, no meio de uma tela pequena.
fn numero(
    inicial: f64,
    ajuste: impl Fn(NumberBox) -> NumberBox + 'static,
) -> impl Fn() -> Form + 'static {
    move || {
        let valor = signal(inicial);
        Form::new("Número").child(
            Column::new()
                .padding(Insets::new(24.0, 40.0, 24.0, 24.0))
                .child(ajuste(NumberBox::bind(valor).id("numero").height(44))),
        )
    }
}

// --- Snapshots -------------------------------------------------------------------------------

#[test]
fn snapshots_do_number_box() {
    for tema in TEMAS {
        // Compacto, com o foco: o X no campo e os botões grandes sobre o fim dele.
        let mut app = TestApp::new(numero(42.0, |n| n.spin_buttons(SpinButtons::Compact)))
            .size(300, 150)
            .theme(tema);
        app.click("numero").hover("numero-aumentar");
        assert!(app.is_open("numero"));
        app.assert_lint();
        app.assert_snapshot(&format!("number_box_compacto_{}", theme_name(tema)));

        // Em linha, sem foco: as setas no fim do campo.
        let mut app = TestApp::new(numero(1.5, |n| n.spin_buttons(SpinButtons::Inline)))
            .size(300, 150)
            .theme(tema);
        app.hover("numero-diminuir");
        app.assert_lint();
        app.assert_snapshot(&format!("number_box_em_linha_{}", theme_name(tema)));

        // Desabilitado: as setas apagadas.
        let mut app = TestApp::new(numero(7.0, |n| {
            n.spin_buttons(SpinButtons::Inline).disabled(true)
        }))
        .size(300, 150)
        .theme(tema);
        assert!(!app.enabled("numero-aumentar"));
        app.assert_lint();
        app.assert_snapshot(&format!("number_box_desabilitado_{}", theme_name(tema)));
    }
}

#[test]
fn snapshots_da_senha() {
    for tema in TEMAS {
        let mut app = TestApp::new(|| {
            let senha = signal(String::new());
            Form::new("Senha").child(
                Column::new().padding(24).child(
                    Edit::bind(senha)
                        .id("senha")
                        .placeholder("Senha")
                        .password()
                        .height(44),
                ),
            )
        })
        .size(300, 100)
        .theme(tema);
        app.type_into("senha", "segredo").hover("senha-revelar");
        app.assert_lint();
        app.assert_snapshot(&format!("senha_com_olho_{}", theme_name(tema)));
        app.press("senha-revelar");
        assert_eq!(app.edit("senha").text, "segredo");
        app.assert_lint();
        app.assert_snapshot(&format!("senha_revelada_{}", theme_name(tema)));
        app.release();
    }
}

// --- NumberBox -------------------------------------------------------------------------------

#[test]
fn roda_muda_o_valor_so_com_o_foco() {
    let mut app = TestApp::new(|| {
        let valor = signal(5.0);
        Form::new("Rolagem").child(
            ScrollViewer::new().id("lista").child(
                Column::new()
                    .gap(12)
                    .padding(24)
                    .child(NumberBox::bind(valor).id("numero").height(44))
                    .child(Button::new("Outro").id("outro").height(44))
                    .child(Column::new().height(2000)),
            ),
        )
    })
    .size(300, 400);
    let sobre = app.rect("numero").center();

    // Sem foco, a roda rola a lista.
    app.wheel(sobre, Vec2::new(0.0, 40.0));
    assert_eq!(app.value("numero"), 5.0);
    assert_eq!(app.scroll_offset("lista").y, 40.0);
    app.scroll("lista", -40.0);

    // Com foco, para cima aumenta e para baixo diminui, e a lista fica parada.
    app.click("numero");
    app.wheel(sobre, Vec2::new(0.0, -40.0));
    assert_eq!(app.value("numero"), 6.0);
    app.wheel(sobre, Vec2::new(0.0, 40.0))
        .wheel(sobre, Vec2::new(0.0, 40.0));
    assert_eq!(app.value("numero"), 4.0);
    assert_eq!(app.scroll_offset("lista").y, 0.0);

    // Fora do campo, mesmo com o foco nele, a roda rola.
    let fora = app.rect("outro").center();
    app.wheel(fora, Vec2::new(0.0, 40.0));
    assert_eq!(app.value("numero"), 4.0);
    assert_eq!(app.scroll_offset("lista").y, 40.0);
}

#[test]
fn segurar_o_botao_repete_a_cada_250_ms() {
    let mut app = TestApp::new(numero(0.0, |n| {
        n.max(10.0).spin_buttons(SpinButtons::Compact)
    }))
    .size(300, 200);
    app.click("numero");
    assert!(app.is_open("numero"));
    app.press("numero-aumentar");
    assert_eq!(app.value("numero"), 1.0, "age ao pressionar");
    app.wait(SPIN_REPEAT / 2);
    assert_eq!(app.value("numero"), 1.0);
    app.wait(SPIN_REPEAT / 2);
    assert_eq!(app.value("numero"), 2.0);
    app.wait(SPIN_REPEAT * 20);
    assert_eq!(app.value("numero"), 10.0, "para no máximo");
    assert!(
        !app.enabled("numero-aumentar"),
        "no máximo, o botão desabilita"
    );
    app.release();
    assert_eq!(app.focused().as_deref(), Some("numero"));

    app.press("numero-diminuir").wait(SPIN_REPEAT * 2).release();
    assert_eq!(app.value("numero"), 7.0);
    app.wait(SPIN_REPEAT * 5);
    assert_eq!(app.value("numero"), 7.0, "solto, para");
}

#[test]
fn teclado_passos_e_casas_decimais() {
    let mut app = TestApp::new(numero(0.0, |n| {
        n.min(-1.0)
            .max(5.0)
            .small_change(0.25)
            .large_change(2.0)
            .precision(1)
    }))
    .size(300, 200);
    app.click("numero").key(Key::Up);
    // 0,25 arredondado em uma casa.
    assert_eq!(app.text("numero"), "0,3");
    app.key(Key::PageUp).key(Key::PageUp).key(Key::PageUp);
    assert_eq!(app.value("numero"), 5.0);
    app.key(Key::PageDown);
    assert_eq!(app.value("numero"), 3.0);

    // Ponto e vírgula valem; o ponto antes de uma vírgula é de milhar.
    app.set_value("numero", -0.04);
    assert_eq!(app.text("numero"), "0", "-0 vira 0");
    app.shortcut('a').type_text("1.5").key(Key::Enter);
    assert_eq!(app.text("numero"), "1,5");
    app.shortcut('a').type_text("-7").key(Key::Enter);
    assert_eq!(app.value("numero"), -1.0, "preso no mínimo");
}

#[test]
fn opcional_fica_vazio_e_obrigatorio_volta() {
    let mut app = TestApp::new(|| {
        let opcional = signal(Some(3.0));
        let obrigatorio = signal(8.0);
        Form::new("Valores").child(
            Column::new()
                .gap(12)
                .padding(24)
                .child(NumberBox::bind_optional(opcional).id("opcional").height(44))
                .child(NumberBox::bind(obrigatorio).id("obrigatorio").height(44))
                .child(
                    Text::new(move || format!("{:?} {}", opcional.get(), obrigatorio.get()))
                        .id("valores"),
                ),
        )
    })
    .size(300, 300);
    app.click("opcional").click("opcional-limpar");
    assert_eq!(app.text("opcional"), "");
    app.key(Key::Enter);
    assert_eq!(app.text("valores"), "None 8");

    // Vazio, a seta não faz nada, como no WinUI (o valor é `NaN` lá).
    app.key(Key::Up);
    assert_eq!(app.text("valores"), "None 8");
    app.type_text("2").key(Key::Up);
    assert_eq!(app.text("valores"), "Some(3.0) 8");

    app.click("obrigatorio")
        .click("obrigatorio-limpar")
        .key(Key::Enter);
    assert_eq!(
        app.text("obrigatorio"),
        "8",
        "o obrigatório volta ao valor anterior"
    );
    app.shortcut('a').type_text("abc").key(Key::Enter);
    assert_eq!(app.text("obrigatorio"), "8");
}

#[test]
fn modos_dos_botoes() {
    // Sem botões, o padrão do WinUI (`SpinButtonPlacementMode.Hidden`).
    let mut app = TestApp::new(numero(1.0, |n| n)).size(300, 200);
    assert!(app.ui().find("numero-aumentar").is_none());
    app.click("numero");
    assert!(!app.is_open("numero"));
    app.key(Key::Up);
    assert_eq!(app.value("numero"), 2.0, "sem botões, o teclado continua");

    // Em linha, as setas ficam no campo, mesmo sem foco, e o texto não passa por baixo delas.
    let app = TestApp::new(numero(1.0, |n| n.spin_buttons(SpinButtons::Inline))).size(300, 200);
    assert!(app.visible("numero-aumentar"));
    assert!(app.rect("numero-aumentar").x1 <= app.rect("numero").x1);
    // 32 px de cada botão, com as margens do WinUI: 4 em volta do de aumentar e 4 fora do outro.
    assert_eq!(app.edit("numero").trailing, 40.0 + 36.0);
    assert!(!app.is_open("numero"));
}

#[test]
fn mudar_o_valor_por_fora_reformata() {
    let valor = std::rc::Rc::new(std::cell::Cell::new(None));
    let guarda = std::rc::Rc::clone(&valor);
    let mut app = TestApp::new(move || {
        let v = signal(2.0);
        guarda.set(Some(v));
        Form::new("Número").child(
            Column::new()
                .padding(24)
                .child(NumberBox::bind(v).id("numero").height(44)),
        )
    });
    let v = valor.get().unwrap();
    v.set(7.256);
    app.frame();
    assert_eq!(app.text("numero"), "7,256");
    assert_eq!(app.tree().find("numero").unwrap().value, Some(7.256));
}

// --- Senha -----------------------------------------------------------------------------------

/// Um campo de senha com o modo `modo` e um botão para tirar o foco dele.
fn senha(modo: PasswordReveal) -> impl Fn() -> Form + 'static {
    move || {
        let texto = signal(String::new());
        Form::new("Senha").child(
            Column::new()
                .gap(12)
                .padding(24)
                .child(
                    Edit::bind(texto)
                        .id("senha")
                        .password()
                        .password_reveal(modo)
                        .height(44),
                )
                .child(Button::new("Outro").id("outro").height(44)),
        )
    }
}

#[test]
fn revelar_segue_o_modo() {
    // Peek: o olho some depois de sair do campo e volta quando a senha é apagada.
    let mut app = TestApp::new(senha(PasswordReveal::Peek));
    app.type_into("senha", "abc");
    assert!(app.visible("senha-revelar"));
    app.click("outro").click("senha");
    assert!(!app.visible("senha-revelar"));
    app.shortcut('a').key(Key::Backspace).type_text("x");
    assert!(app.visible("senha-revelar"));

    // Pressionado, mostra; solto, esconde.
    app.press("senha-revelar");
    assert_eq!(app.edit("senha").text, "x");
    app.release();
    assert_eq!(app.edit("senha").text, mascara(1));

    // Hidden: nunca há olho.
    let mut app = TestApp::new(senha(PasswordReveal::Hidden));
    app.type_into("senha", "abc");
    assert!(!app.visible("senha-revelar"));

    // Visible: a senha à vista, e o teclado da plataforma deixa de ser o de senha.
    let mut app = TestApp::new(senha(PasswordReveal::Visible));
    app.type_into("senha", "abc");
    assert_eq!(app.edit("senha").text, "abc");
    assert!(!app.visible("senha-revelar"));
    assert!(!app.tree().find("senha").unwrap().password);
}

#[test]
fn revelada_a_senha_continua_sem_copiar() {
    let mut app = TestApp::new(senha(PasswordReveal::Peek));
    app.type_into("senha", "segredo");
    app.clipboard().write("antes".into());
    app.press("senha-revelar");
    app.shortcut('a').shortcut('c');
    app.release();
    assert_eq!(app.clipboard().read().as_deref(), Some("antes"));
}

#[test]
fn soltar_fora_do_olho_tambem_esconde() {
    let mut app = TestApp::new(senha(PasswordReveal::Peek));
    app.type_into("senha", "segredo");
    app.press("senha-revelar");
    let fora = app.rect("outro").center();
    app.ui().pointer_move(fora, tomni_ui::PointerKind::Mouse);
    app.frame();
    assert_eq!(
        app.edit("senha").text,
        "segredo",
        "segue revelada até soltar"
    );
    app.release();
    assert_eq!(app.edit("senha").text, mascara(7));
}
