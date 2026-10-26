//! Marco M8: ids estáveis e reconciliação (`Show`, `For`) e o `ScrollViewer`, pelo `TestApp`.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use tomni_ui::kurbo::{Point, Vec2};
use tomni_ui::lint::{Rule, Severity};
use tomni_ui::prelude::*;
use tomni_ui::theme::Theme;
use tomni_ui::{Key, PointerKind, ScrollAxis, ScrollBarParts};
use tomni_ui_test::TestApp;

/// Contador que pode ir para um `on_cleanup` (que pede `Send + Sync`).
#[derive(Clone, Default)]
struct Contador(Arc<AtomicU32>);

impl Contador {
    fn mais(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }

    fn get(&self) -> u32 {
        self.0.load(Ordering::Relaxed)
    }
}

/// Itens da lista e o "relógio" que os efeitos dos itens leem.
type Sinais = (Signal<Vec<u32>>, Signal<u32>);

/// O que o teste da lista acompanha de fora.
#[derive(Clone, Default)]
struct Sondas {
    /// Views de item criadas.
    criadas: Contador,
    /// Views de item descartadas (`on_cleanup`).
    descartadas: Contador,
    /// Rodadas dos efeitos dos itens.
    efeitos: Contador,
    /// Os signals da tela, para mexer de fora: itens e o "relógio" que os efeitos leem.
    signals: Rc<Cell<Option<Sinais>>>,
}

impl Sondas {
    fn itens(&self) -> Signal<Vec<u32>> {
        self.signals.get().expect("tela montada").0
    }

    fn relogio(&self) -> Signal<u32> {
        self.signals.get().expect("tela montada").1
    }
}

/// Lista de `n` itens num `ScrollViewer`, cada um com um texto e um botão de remover. Cada
/// item tem um efeito que lê o relógio e um `on_cleanup`.
fn lista(n: u32, sondas: Sondas) -> impl FnOnce() -> Form {
    move || {
        let itens = signal((1..=n).collect::<Vec<u32>>());
        let relogio = signal(0_u32);
        sondas.signals.set(Some((itens, relogio)));
        let Sondas {
            criadas,
            descartadas,
            efeitos,
            ..
        } = sondas;
        Form::new("Itens").child(
            Column::new().padding(16).gap(8).child(
                ScrollViewer::new().id("lista").grow(1).child(
                    Column::new()
                        .gap(4)
                        .padding(Insets::new(0.0, 12.0, 0.0, 0.0))
                        .child(For::each(
                            itens,
                            |&i| i,
                            move |i| {
                                criadas.mais();
                                let efeitos = efeitos.clone();
                                effect(move || {
                                    relogio.get();
                                    efeitos.mais();
                                });
                                let descartadas = descartadas.clone();
                                on_cleanup(move || descartadas.mais());
                                Row::new()
                                    .gap(8)
                                    .align(Align::Center)
                                    .child(
                                        Text::new(format!("Item {i}"))
                                            .id(format!("texto-{i}"))
                                            .grow(1),
                                    )
                                    .child(
                                        Button::new("Remover")
                                            .id(format!("remover-{i}"))
                                            .height(44)
                                            .on_click(move || {
                                                itens.update(|v| v.retain(|&x| x != i))
                                            }),
                                    )
                            },
                        )),
                ),
            ),
        )
    }
}

/// Itens cujo texto ou botão está em `repaints`.
fn itens_em(repaints: &[String]) -> Vec<u32> {
    let mut itens: Vec<u32> = repaints
        .iter()
        .filter_map(|id| {
            id.strip_prefix("texto-")
                .or_else(|| id.strip_prefix("remover-"))
        })
        .map(|n| n.parse().unwrap())
        .collect();
    itens.dedup();
    itens
}

#[test]
fn for_com_mil_itens_cria_e_descarta_so_o_item() {
    let sondas = Sondas::default();
    let mut app = TestApp::new(lista(1000, sondas.clone())).size(400, 400);
    assert_eq!(sondas.criadas.get(), 1000);
    assert_eq!(
        sondas.efeitos.get(),
        1000,
        "cada efeito roda uma vez ao montar"
    );
    let nos = app.ui().tree().len();
    let item_500 = app.ui().find("texto-500").unwrap();
    let primeiro = app.ui().find("remover-1").unwrap();
    assert!(app.in_view("texto-1"));
    assert!(!app.in_view("texto-500"));

    // Inserir no começo: só a view do item novo é criada, e os outros nós continuam os mesmos.
    sondas.itens().update(|v| v.insert(0, 0));
    app.frame();
    assert_eq!(sondas.criadas.get(), 1001);
    assert_eq!(
        app.ui().tree().len(),
        nos + 3,
        "uma Row, um Text e um Button"
    );
    assert_eq!(app.ui().find("texto-500"), Some(item_500));
    assert_eq!(app.ui().find("remover-1"), Some(primeiro));

    // Repinta o item novo e os que desceram na área visível; os de baixo, fora da tela, não.
    let repaints = app.repaints_since_last();
    let repintados = itens_em(&repaints);
    let visiveis: Vec<u32> = (0..=1000)
        .filter(|i| app.in_view(&format!("texto-{i}")))
        .collect();
    assert!(repintados.len() < 15, "{repaints:?}");
    for i in &visiveis {
        assert!(
            repintados.contains(i),
            "item {i} visível e não repintado: {repaints:?}"
        );
    }
    for i in &repintados {
        // Uma folga: o último pode ter acabado de sair por baixo.
        assert!(
            *i <= visiveis.last().unwrap() + 1,
            "item {i} fora da tela repintado"
        );
    }
    assert!(
        repaints.contains(&"lista".to_owned()),
        "a barra encolheu: {repaints:?}"
    );

    // Remover pelo botão descarta a view, os nós e o efeito do item.
    app.click("remover-0");
    assert_eq!(sondas.descartadas.get(), 1);
    assert_eq!(app.ui().tree().len(), nos);
    assert_eq!(app.ui().find("texto-0"), None);
    assert_eq!(app.ui().find("texto-500"), Some(item_500));
    sondas.relogio().set(1);
    app.frame();
    // 1001 ao nascer (os 1000 e o item 0); agora, só os 1000 vivos rodam de novo.
    assert_eq!(sondas.efeitos.get(), 1001 + 1000);

    // Remover um item lá de baixo não mexe em nada visível: só a barra e a coluna, que
    // encolheu (e não desenha nada).
    app.repaints_since_last();
    sondas.itens().update(|v| v.retain(|&i| i != 900));
    app.frame();
    assert_eq!(sondas.descartadas.get(), 2);
    assert_eq!(app.repaints_since_last(), ["lista", "column"]);

    // Trocar a ordem mantém as views (nada é criado nem descartado).
    sondas.itens().update(|v| v.reverse());
    app.frame();
    assert_eq!(sondas.criadas.get(), 1001);
    assert_eq!(sondas.descartadas.get(), 2);
    assert_eq!(app.ui().find("texto-500"), Some(item_500));
    assert!(app.in_view("texto-1000"));
    app.assert_lint();
}

#[test]
fn ao_descartar_a_tela_os_efeitos_dos_itens_somem() {
    let sondas = Sondas::default();
    let app = TestApp::new(lista(10, sondas.clone()));
    drop(app);
    assert_eq!(sondas.descartadas.get(), 10);
}

/// Cem textos num `ScrollViewer` de 400 × 300.
fn cem() -> Form {
    Form::new("Cem").child(
        Column::new()
            .padding(16)
            .child(
                ScrollViewer::new()
                    .id("lista")
                    .grow(1)
                    .child(Column::new().gap(4).child(For::each(
                        (1..=100).collect::<Vec<u32>>(),
                        |&i| i,
                        |i| Text::new(format!("Linha {i}")).id(format!("linha-{i}")),
                    ))),
            ),
    )
}

#[test]
fn rolar_nao_regrava_os_filhos() {
    let mut app = TestApp::new(cem);
    let topo = app.rect("linha-1");
    assert_eq!(app.scroll_offset("lista"), Vec2::ZERO);
    assert!(app.scroll_content("lista").height > app.rect("lista").height());

    app.scroll("lista", 100.0);
    assert_eq!(app.scroll_offset("lista"), Vec2::new(0.0, 100.0));
    assert_eq!(
        app.rect("linha-1").y0,
        topo.y0 - 100.0,
        "o rect já vem rolado"
    );
    assert_eq!(app.repaints_since_last(), ["lista"]);

    // A roda sobre a lista, o arrasto com o dedo e a barra também só repintam o SV.
    let meio = app.rect("lista").center();
    app.wheel(meio, Vec2::new(0.0, 48.0));
    assert_eq!(app.scroll_offset("lista").y, 148.0);
    assert_eq!(app.repaints_since_last(), ["lista"]);

    let ui = app.ui();
    ui.pointer_down(meio, PointerKind::Touch, 1);
    ui.pointer_move(meio - Vec2::new(0.0, 40.0), PointerKind::Touch);
    ui.pointer_move(meio - Vec2::new(0.0, 60.0), PointerKind::Touch);
    ui.pointer_up(meio - Vec2::new(0.0, 60.0), PointerKind::Touch);
    app.frame();
    assert_eq!(
        app.scroll_offset("lista").y,
        208.0,
        "o conteúdo segue o dedo"
    );
    assert_eq!(app.repaints_since_last(), ["lista"]);

    // Para no fim e no começo.
    app.scroll("lista", 1e6);
    let fim = app.scroll_offset("lista").y;
    assert_eq!(
        fim,
        app.scroll_content("lista").height - app.rect("lista").height()
    );
    assert!(app.in_view("linha-100"));
    assert!(!app.in_view("linha-1"));
    app.scroll("lista", -1e6);
    assert_eq!(app.scroll_offset("lista"), Vec2::ZERO);
    app.assert_lint();
}

/// Partes da barra vertical do `ScrollViewer` `lista`, na janela.
fn barra_vertical(app: &mut TestApp) -> ScrollBarParts {
    let rect = app.rect("lista");
    let ui = app.ui();
    let lista = ui.find("lista").unwrap();
    let scroller = ui.tree().scroller(lista).unwrap();
    scroller.bar(rect, ScrollAxis::Y).unwrap()
}

#[test]
fn barra_arrastavel_e_cliques_no_trilho() {
    let mut app = TestApp::new(cem);
    let rect = app.rect("lista");
    let partes = barra_vertical(&mut app);
    let max = app.scroll_content("lista").height - rect.height();

    // Arrastar o polegar até o fim do trilho rola até o fim.
    let polegar = partes.thumb.center();
    app.drag(polegar, polegar + Vec2::new(0.0, partes.travel));
    assert_eq!(app.scroll_offset("lista").y, max);
    assert_eq!(app.tree().find("lista").unwrap().state, Some("hover"));

    // Clique no trilho acima do polegar: uma página para cima.
    let partes = barra_vertical(&mut app);
    let trilho = Point::new(partes.thumb.center().x, partes.thumb.y0 - 4.0);
    app.click_at(trilho, 1);
    assert_eq!(app.scroll_offset("lista").y, max - rect.height());

    // Seta de cima: uma linha.
    let antes = app.scroll_offset("lista").y;
    app.click_at(partes.decrease.center(), 1);
    assert_eq!(app.scroll_offset("lista").y, antes - tomni_ui::SCROLL_LINE);
}

#[test]
fn teclado_rola_e_tab_traz_o_focado() {
    let mut app = TestApp::new(|| {
        Form::new("Botões").child(
            Column::new().padding(16).child(
                ScrollViewer::new().id("lista").grow(1).child(
                    Column::new()
                        .gap(8)
                        .padding(Insets::new(0.0, 12.0, 0.0, 0.0))
                        .child(For::each(
                            (1..=20).collect::<Vec<u32>>(),
                            |&i| i,
                            |i| {
                                Button::new(format!("Botão {i}"))
                                    .id(format!("botao-{i}"))
                                    .height(44)
                            },
                        )),
                ),
            ),
        )
    });
    for i in 1..=20 {
        app.tab();
        let id = format!("botao-{i}");
        assert_eq!(app.focused(), Some(id.clone()));
        assert!(app.in_view(&id), "{id} focado fora da área visível");
        let (botao, lista) = (app.rect(&id), app.rect("lista"));
        assert!(
            botao.y0 >= lista.y0 && botao.y1 <= lista.y1,
            "{id} aparece inteiro"
        );
    }
    let fim = app.scroll_offset("lista").y;
    assert!(fim > 0.0);

    // Com o foco num botão, PageUp/PageDown e Home/End rolam a lista em volta.
    app.key(Key::Home);
    assert_eq!(app.scroll_offset("lista").y, 0.0);
    app.key(Key::PageDown);
    let pagina = app.scroll_offset("lista").y;
    assert!(pagina > 0.0 && pagina < fim);
    app.key(Key::End);
    assert_eq!(app.scroll_offset("lista").y, fim);
    app.key(Key::PageUp);
    assert_eq!(app.scroll_offset("lista").y, fim - pagina);

    // Clicar num botão fora da área visível rola até ele antes.
    app.key(Key::Home);
    assert!(!app.in_view("botao-20"));
    app.click("botao-20");
    assert!(app.in_view("botao-20"));
    app.assert_lint();
}

#[test]
fn show_troca_as_views_e_descarta_o_estado() {
    let descartadas = Contador::default();
    let saida = descartadas.clone();
    let mut app = TestApp::new(move || {
        let logado = signal(false);
        Form::new("Conta").child(
            Column::new()
                .padding(16)
                .gap(8)
                .child(
                    Button::new(move || if logado.get() { "Sair" } else { "Entrar" }.to_owned())
                        .id("alternar")
                        .height(44)
                        .on_click(move || logado.update(|l| *l = !*l)),
                )
                .child(
                    Show::when(logado, move || {
                        let saida = saida.clone();
                        on_cleanup(move || saida.mais());
                        Edit::bind(signal(String::new())).id("apelido").height(44)
                    })
                    .otherwise(|| Text::new("Entre para continuar").id("aviso")),
                ),
        )
    });
    assert!(app.ui().find("apelido").is_none());
    assert_eq!(
        app.tree().find("aviso").unwrap().text.as_deref(),
        Some("Entre para continuar")
    );

    app.click("alternar");
    assert!(app.ui().find("aviso").is_none());
    app.type_into("apelido", "Ana");
    assert_eq!(app.text("apelido"), "Ana");

    // Sair descarta o campo (com o texto e o efeito); entrar de novo cria um vazio.
    app.click("alternar");
    assert!(app.ui().find("apelido").is_none());
    assert_eq!(descartadas.get(), 1);
    app.click("alternar");
    assert_eq!(app.text("apelido"), "");
    app.assert_lint();
}

#[test]
fn lint_considera_a_area_rolavel() {
    // Rolado para fora não é erro: dá para alcançar rolando.
    let mut app = TestApp::new(cem);
    assert!(!app.in_view("linha-100"));
    app.assert_lint();

    // Mais largo que um `ScrollViewer` vertical: não dá para alcançar.
    let mut app = TestApp::new(|| {
        Form::new("Largo").child(
            Column::new().padding(16).child(
                ScrollViewer::new().id("lista").grow(1).child(
                    Column::new().align(Align::Start).child(
                        Button::new("Largo demais")
                            .id("largo")
                            .width(600)
                            .height(44),
                    ),
                ),
            ),
        )
    });
    let issues = app.lint();
    let largo: Vec<_> = issues
        .iter()
        .filter(|issue| issue.widget == "largo")
        .collect();
    assert_eq!(largo.len(), 1, "{issues:?}");
    assert_eq!(largo[0].rule, Rule::Offscreen);
    assert_eq!(largo[0].severity, Severity::Error);
}

/// Lista curta numa janela pequena, para as fotos da barra.
fn barra(theme: Theme) -> TestApp {
    TestApp::new(|| {
        Form::new("Barra").child(
            Column::new().padding(8).child(
                ScrollViewer::new().id("lista").grow(1).child(
                    Column::new()
                        .gap(4)
                        .padding(Insets::new(0.0, 12.0, 0.0, 0.0))
                        .child(For::each(
                            (1..=12).collect::<Vec<u32>>(),
                            |&i| i,
                            |i| Text::new(format!("Linha {i}")).id(format!("linha-{i}")),
                        )),
                ),
            ),
        )
    })
    .size(220, 200)
    .theme(theme)
}

#[test]
fn snapshots_da_barra() {
    for (theme, nome) in [(Theme::Light, "claro"), (Theme::Dark, "escuro")] {
        let mut app = barra(theme);
        app.scroll("lista", 40.0);
        app.assert_snapshot(&format!("scroll_viewer_rest_{nome}"));

        let partes = barra_vertical(&mut app);
        app.hover("lista");
        assert_eq!(app.tree().find("lista").unwrap().state, Some("hover"));
        app.assert_snapshot(&format!("scroll_viewer_hover_{nome}"));

        let polegar = partes.thumb.center();
        let ui = app.ui();
        ui.pointer_move(polegar, PointerKind::Mouse);
        ui.pointer_down(polegar, PointerKind::Mouse, 1);
        ui.pointer_move(polegar + Vec2::new(0.0, 10.0), PointerKind::Mouse);
        app.frame();
        assert_eq!(app.tree().find("lista").unwrap().state, Some("pressed"));
        app.assert_snapshot(&format!("scroll_viewer_pressed_{nome}"));
    }
}

#[test]
fn for_preserva_texto_e_foco_dos_outros_itens() {
    let saida = Rc::new(Cell::new(None));
    let sinal = Rc::clone(&saida);
    let mut app = TestApp::new(move || {
        let itens = signal(vec![1_u32, 2, 3]);
        sinal.set(Some(itens));
        Form::new("Campos").child(Column::new().padding(16).gap(8).child(For::each(
            itens,
            |&i| i,
            |i| {
                // Estado interno do item: o signal nasce com a view.
                Edit::bind(signal(String::new()))
                    .id(format!("campo-{i}"))
                    .height(44)
            },
        )))
    });
    let itens = saida.get().unwrap();
    app.type_into("campo-1", "um").type_into("campo-3", "três");
    assert_eq!(app.focused().as_deref(), Some("campo-3"));

    // Mover: o 3 vai para o começo, com o texto e o foco.
    let y = app.rect("campo-3").y0;
    itens.set(vec![3, 1, 2]);
    app.frame();
    assert!(app.rect("campo-3").y0 < y);
    assert_eq!(app.text("campo-3"), "três");
    assert_eq!(app.text("campo-1"), "um");
    assert_eq!(app.focused().as_deref(), Some("campo-3"));
    app.type_text("!");
    assert_eq!(app.text("campo-3"), "três!");

    // Remover outro item não mexe no texto nem no foco; remover o focado tira o foco.
    itens.set(vec![3, 2]);
    app.frame();
    assert!(app.ui().find("campo-1").is_none());
    assert_eq!(app.focused().as_deref(), Some("campo-3"));
    itens.set(vec![2]);
    app.frame();
    assert_eq!(app.focused(), None);
    assert_eq!(app.text("campo-2"), "");
    app.tab();
    assert_eq!(app.focused().as_deref(), Some("campo-2"));
}
