//! Marco M2: `Column`, `Row` e `Form` com layout do taffy, foco por Tab e repaint seletivo.

use tomni_ui::kurbo::Rect;
use tomni_ui::lint::Rule;
use tomni_ui::prelude::*;
use tomni_ui::theme::Theme;
use tomni_ui::{Key, Look};
use tomni_ui_test::{TestApp, assert_snapshot};

/// Contador: um rótulo reativo, "+1" e "Zerar", que só habilita com o contador acima de zero.
fn contador() -> impl View {
    let n = signal(0);
    Form::new("Contador").child(
        Column::new()
            .gap(8)
            .padding(16)
            .child(Button::new(move || format!("Cliques: {}", n.get())).id("rotulo"))
            .child(
                Row::new()
                    .gap(8)
                    .child(
                        Button::new("+1")
                            .id("mais")
                            .accent()
                            .on_click(move || n.update(|n| *n += 1)),
                    )
                    .child(
                        Button::new("Zerar")
                            .id("zerar")
                            .disabled(move || n.get() == 0)
                            .on_click(move || n.set(0)),
                    ),
            ),
    )
}

#[test]
fn repaint_seletivo() {
    let mut app = TestApp::new(contador).size(320, 200);
    assert!(
        app.repaints_since_last().is_empty(),
        "o TestApp descarta os repaints da montagem"
    );

    app.click("mais");
    assert_eq!(app.label("rotulo"), "Cliques: 1");
    assert!(app.enabled("zerar"));
    assert_eq!(app.repaints_since_last(), ["rotulo", "mais", "zerar"]);

    // O "zerar" depende do contador, mas continua habilitado: o efeito roda e nada muda.
    app.click("mais");
    assert_eq!(app.label("rotulo"), "Cliques: 2");
    assert_eq!(app.repaints_since_last(), ["rotulo", "mais"]);

    app.frame();
    assert!(app.repaints_since_last().is_empty());

    // Sair do "mais" tira o hover dele; o "zerar" desabilita depois do clique.
    app.click("zerar");
    assert_eq!(app.label("rotulo"), "Cliques: 0");
    assert!(!app.enabled("zerar"));
    assert_eq!(app.repaints_since_last(), ["rotulo", "mais", "zerar"]);
}

#[test]
fn layout_da_coluna() {
    let app = TestApp::new(contador).size(320, 200);
    assert_eq!(app.rect("rotulo"), Rect::new(16.0, 16.0, 304.0, 48.0));
    let row = Rect::new(16.0, 56.0, 304.0, 88.0);
    let mais = app.rect("mais");
    let zerar = app.rect("zerar");
    assert_eq!((mais.x0, mais.y0, mais.y1), (16.0, 56.0, 88.0));
    assert_eq!(zerar.x0, mais.x1 + 8.0, "gap da linha");
    assert_eq!((zerar.y0, zerar.y1), (row.y0, row.y1));
    assert!(
        zerar.x1 < row.x1,
        "na linha, os botões não esticam na horizontal"
    );
}

#[test]
fn alinhamento_grow_e_tamanho_fixo() {
    let app = TestApp::new(|| {
        Form::new("Layout").child(
            Column::new()
                .padding(Insets::new(10.0, 20.0, 10.0, 20.0))
                .gap(4)
                .align(Align::Center)
                .child(Button::new("Centro").id("centro"))
                .child(Button::new("Fixo").id("fixo").width(100).height(40))
                .child(
                    Row::new()
                        .id("linha")
                        .width(200)
                        .child(Button::new("Cresce").id("cresce").grow(1))
                        .child(Button::new("Fim").id("fim")),
                )
                .child(
                    Row::new()
                        .id("fim-da-linha")
                        .width(200)
                        .justify(Justify::End)
                        .child(Button::new("Ok").id("ok")),
                ),
        )
    })
    .size(300, 240);

    let centro = app.rect("centro");
    assert_eq!(centro.y0, 10.0);
    assert!(
        (centro.center().x - 150.0).abs() <= 0.5,
        "centralizado: {centro:?}"
    );

    assert_eq!(app.rect("fixo"), Rect::new(100.0, 46.0, 200.0, 86.0));

    let linha = app.rect("linha");
    assert_eq!(linha, Rect::new(50.0, 90.0, 250.0, 122.0));
    let cresce = app.rect("cresce");
    let fim = app.rect("fim");
    assert_eq!(cresce.x0, linha.x0);
    assert_eq!(cresce.x1, fim.x0, "o grow ocupa o espaço que sobra");
    assert_eq!(fim.x1, linha.x1);

    assert_eq!(
        app.rect("ok").x1,
        app.rect("fim-da-linha").x1,
        "justify End"
    );
}

#[test]
fn grow_divide_na_proporcao_sem_cortar_texto() {
    let app = TestApp::new(|| {
        Form::new("Grow").child(
            Column::new()
                .gap(4)
                .child(
                    Row::new()
                        .gap(10)
                        .child(Button::new("-1").id("curto").grow(1))
                        .child(Button::new("Criar conta").id("longo").grow(1)),
                )
                .child(
                    Row::new()
                        .child(Button::new("Um").id("um").grow(1))
                        .child(Button::new("Dois").id("dois").grow(3)),
                )
                .child(
                    Row::new()
                        .child(Button::new("x").id("pouco").grow(1))
                        .child(
                            Button::new("Um texto que não cabe na metade")
                                .id("muito")
                                .grow(1),
                        ),
                ),
        )
    })
    .size(300, 200);

    // Larguras iguais, qualquer que seja o texto (o `flex: 1` do CSS).
    assert_eq!(app.rect("curto").width(), 145.0);
    assert_eq!(app.rect("longo").width(), 145.0);
    assert_eq!(app.rect("um").width(), 75.0);
    assert_eq!(app.rect("dois").width(), 225.0);
    // Sem espaço para a metade, o texto longo fica com a largura dele e o outro, com o resto.
    let (pouco, muito) = (app.rect("pouco"), app.rect("muito"));
    assert!(muito.width() > 150.0, "{muito:?}");
    assert_eq!(pouco.width() + muito.width(), 300.0);
    let mut app = app;
    let cortados: Vec<_> = app
        .lint()
        .into_iter()
        .filter(|i| matches!(i.rule, Rule::ClippedText | Rule::Offscreen))
        .collect();
    assert!(cortados.is_empty(), "{cortados:?}");
}

#[test]
fn safe_area_e_resize_movem_e_repintam() {
    let mut app = TestApp::new(contador).size(320, 200);
    app.safe_area(Insets::new(24.0, 0.0, 0.0, 0.0));
    assert_eq!(app.rect("rotulo"), Rect::new(16.0, 40.0, 304.0, 72.0));
    assert_eq!(
        app.repaints_since_last(),
        ["column", "rotulo", "row", "mais", "zerar"],
        "o form não mudou de lugar nem de tamanho"
    );

    app.resize(400, 200);
    assert_eq!(app.rect("rotulo").x1, 384.0);
    assert_eq!(
        app.repaints_since_last(),
        ["form", "column", "rotulo", "row"],
        "os botões da linha não mudaram"
    );
}

#[test]
fn tab_e_teclado() {
    let mut app = TestApp::new(contador);
    app.tab();
    assert_eq!(app.focused().as_deref(), Some("rotulo"));
    app.tab();
    assert_eq!(app.focused().as_deref(), Some("mais"));
    app.tab();
    assert_eq!(
        app.focused().as_deref(),
        Some("rotulo"),
        "o zerar está desabilitado"
    );
    app.shift_tab();
    assert_eq!(app.focused().as_deref(), Some("mais"));
    let Look::Button { visual, .. } = app.look("mais") else {
        unreachable!()
    };
    assert!(visual.focus_ring);

    app.key(Key::Enter);
    app.key(Key::Space);
    assert_eq!(app.label("rotulo"), "Cliques: 2");

    app.tab();
    assert_eq!(app.focused().as_deref(), Some("zerar"));
    app.key(Key::Space);
    assert_eq!(app.label("rotulo"), "Cliques: 0");
    assert_eq!(app.focused(), None, "o zerar desabilitou e perdeu o foco");
}

#[test]
fn titulo_reativo() {
    let mut app = TestApp::new(|| {
        let n = signal(0);
        Form::new(move || format!("Contador ({})", n.get())).child(
            Button::new("+1")
                .id("mais")
                .on_click(move || n.update(|n| *n += 1)),
        )
    });
    assert_eq!(app.ui().title().as_deref(), Some("Contador (0)"));
    app.click("mais");
    assert_eq!(app.ui().title().as_deref(), Some("Contador (1)"));
}

fn snapshot(theme: Theme, name: &str) {
    let mut app = TestApp::new(contador).size(320, 112).theme(theme);
    app.click("mais");
    app.tab();
    assert_eq!(app.focused().as_deref(), Some("zerar"));
    assert_snapshot!(name, app.snapshot());
}

#[test]
fn snapshot_claro() {
    snapshot(Theme::Light, "m2_contador_claro");
}

#[test]
fn snapshot_escuro() {
    snapshot(Theme::Dark, "m2_contador_escuro");
}
