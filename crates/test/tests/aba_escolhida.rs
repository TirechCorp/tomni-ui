//! A aba escolhida dos `TerminalPanes` (pedido pelo Tirech Omni): com `.tab_scheme`, ela tem o
//! fundo e o texto do esquema dos terminais, como no Windows Terminal; com
//! `.active_tab_indicator`, a do painel ativo ganha a barra de destaque.

use tomni_ui::prelude::*;
use tomni_ui::terminal::{Loopback, PaneLayout, Scheme, Session, TerminalPanes};
use tomni_ui::{Look, TabHeaderVisual};
use tomni_ui_test::TestApp;

fn sessao(chave: &str) -> Session {
    Session::new(Loopback::new()).key(chave).name(chave)
}

/// Dois painéis: `a` e `b` no 0 (com `a` escolhida), `c` no 1; o painel 0 ativo.
fn app(
    configurar: impl FnOnce(TerminalPanes) -> TerminalPanes + 'static,
) -> (TestApp, Signal<PaneLayout>) {
    let (a, b, c) = (sessao("a"), sessao("b"), sessao("c"));
    let guarda = std::rc::Rc::new(std::cell::Cell::new(None));
    let g = std::rc::Rc::clone(&guarda);
    let app = TestApp::new(move || {
        let mut inicial = PaneLayout::with(vec![a, b, c]);
        inicial.grid(2, 1);
        inicial.move_tab(b, 0);
        inicial.select(a);
        inicial.set_active(0);
        let layout = signal(inicial);
        g.set(Some(layout));
        Form::new("Painéis").child(configurar(
            TerminalPanes::bind(layout).id("paineis").grow(1),
        ))
    })
    .size(900, 500);
    let layout = guarda.get().expect("o layout");
    (app, layout)
}

fn visual(app: &TestApp, id: &str) -> TabHeaderVisual {
    match app.look(id) {
        Look::TabHeader { visual, .. } => visual,
        outro => panic!("{outro:?}"),
    }
}

fn cor_do_x(app: &TestApp, id: &str) -> Option<ThemeColor> {
    match app.look(id) {
        Look::TabStripButton(visual) => visual.color,
        outro => panic!("{outro:?}"),
    }
}

#[test]
fn sem_os_ganchos_a_aba_escolhida_fica_como_era() {
    let (app, _) = app(|p| p);
    let a = visual(&app, "paineis-painel-0-a");
    assert!(a.selected);
    assert_eq!(a.selected_colors, None);
    assert!(!a.indicator);
}

#[test]
fn a_escolhida_tem_as_cores_do_esquema_e_so_ela() {
    let esquema = signal(Some(Scheme::Campbell));
    let (mut app, _) = app(move |p| p.tab_scheme(esquema));
    let campbell = Some(Scheme::Campbell.tab_colors());
    assert_eq!(visual(&app, "paineis-painel-0-a").selected_colors, campbell);
    assert_eq!(visual(&app, "paineis-painel-1-c").selected_colors, campbell);
    assert_eq!(visual(&app, "paineis-painel-0-b").selected_colors, None);
    // O X da escolhida acompanha o título, para não sumir no fundo do esquema.
    assert_eq!(
        cor_do_x(&app, "paineis-painel-0-a-fechar"),
        campbell.map(|(_, texto)| texto)
    );
    assert_eq!(cor_do_x(&app, "paineis-painel-0-b-fechar"), None);

    // O esquema do tema usa os tokens, e acompanha o signal.
    esquema.set(Some(Scheme::Theme));
    app.frame();
    assert_eq!(
        visual(&app, "paineis-painel-0-a").selected_colors,
        Some((
            ThemeColor::Token(Token::SolidBackgroundFillColorBase),
            ThemeColor::Token(Token::TextFillColorPrimary),
        ))
    );
    esquema.set(None);
    app.frame();
    assert_eq!(visual(&app, "paineis-painel-0-a").selected_colors, None);
    app.assert_lint();
}

#[test]
fn a_barra_fica_so_na_escolhida_do_painel_ativo() {
    let (mut app, layout) = app(|p| p.active_tab_indicator(true));
    assert!(visual(&app, "paineis-painel-0-a").indicator);
    assert!(!visual(&app, "paineis-painel-0-b").indicator);
    assert!(!visual(&app, "paineis-painel-1-c").indicator);

    layout.update(|l| l.set_active(1));
    app.frame();
    assert!(!visual(&app, "paineis-painel-0-a").indicator);
    assert!(visual(&app, "paineis-painel-1-c").indicator);
}
