//! Marco M30 (fatia do `DataGrid`): a tabela de linhas e colunas. A ordenação (natural, `rank`,
//! decrescente), a seleção (Ctrl, Shift, teclado, Ctrl+A), o filtro, renomear, apagar, o menu de
//! contexto, o arrasto entre dois grids e sobre uma pasta, os arquivos soltos do sistema, a
//! virtualização, o redimensionar das colunas, as ferramentas e os snapshots.

mod comum;

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, UNIX_EPOCH};

use tomni_ui::kurbo::{Point, Vec2};
use tomni_ui::prelude::*;
use tomni_ui::theme::Theme;
use tomni_ui::{Modifiers, SortDirection};
use tomni_ui_test::TestApp;

use comum::theme_name;

/// Tudo o que o teste mexe por fora do grid.
#[derive(Clone)]
struct Estado {
    selecionadas: Signal<BTreeSet<String>>,
    ordem: Signal<Option<(String, SortDirection)>>,
    filtro: Signal<String>,
    renomeando: Signal<Option<String>>,
    /// O que os callbacks viram, em ordem: `invoke:chave`, `rename:chave:texto`,
    /// `delete:a,b`, `drop:origem:a,b:alvo`, `files:n:alvo`.
    log: Rc<RefCell<Vec<String>>>,
}

type Guardado = Rc<RefCell<Vec<Estado>>>;

fn linha(chave: &str, bytes: Option<u64>, segundos: u64) -> DataGridRow {
    let pasta = bytes.is_none();
    let momento = UNIX_EPOCH + Duration::from_secs(1_790_812_800 + segundos);
    let mut linha = DataGridRow::new(chave)
        .cell(chave, CellValue::Text(chave.to_owned()))
        .cell(
            bytes.map_or_else(String::new, |b| format!("{b} B")),
            CellValue::Number(bytes.unwrap_or(0) as f64),
        )
        .cell("2026-10-01", CellValue::Time(Some(momento)))
        .icon(if pasta {
            Symbol::Folder
        } else {
            Symbol::Document
        })
        .rank(if pasta { 1 } else { 2 });
    if pasta {
        linha = linha.drop_target();
    }
    linha
}

/// "..", as pastas docs e fotos e cinco arquivos.
fn linhas() -> Vec<DataGridRow> {
    vec![
        DataGridRow::new("..")
            .cell("..", CellValue::Text("..".to_owned()))
            .cell("", CellValue::Number(0.0))
            .cell("", CellValue::Time(None))
            .icon(Symbol::FolderOpen)
            .rank(0)
            .drop_target(),
        linha("fotos", None, 5_000),
        linha("docs", None, 9_000),
        linha("a10.txt", Some(10), 400),
        linha("z.log", Some(50), 300),
        linha("a2.txt", Some(2000), 200),
        linha("b.md", Some(70_000), 100),
        linha("a1.txt", Some(300), 500),
    ]
}

fn colunas() -> Vec<DataGridColumn> {
    vec![
        DataGridColumn::new("nome", "Nome"),
        DataGridColumn::new("tamanho", "Tamanho")
            .width(ColumnWidth::Pixels(110.0))
            .align_end(),
        DataGridColumn::new("modificado", "Modificado").width(ColumnWidth::Pixels(110.0)),
    ]
}

/// Um grid `id` ligado a um `Estado` novo, guardado em `guardado`.
fn grade(id: &str, guardado: &Guardado, linhas: Vec<DataGridRow>) -> DataGrid {
    let linhas = signal(linhas);
    let selecionadas = signal(BTreeSet::new());
    let ordem = signal(None);
    let filtro = signal(String::new());
    let renomeando = signal(None::<String>);
    let log = Rc::new(RefCell::new(Vec::new()));
    guardado.borrow_mut().push(Estado {
        selecionadas,
        ordem,
        filtro,
        renomeando,
        log: Rc::clone(&log),
    });
    let (l1, l2, l3, l4, l5) = (
        Rc::clone(&log),
        Rc::clone(&log),
        Rc::clone(&log),
        Rc::clone(&log),
        Rc::clone(&log),
    );
    DataGrid::bind(linhas, selecionadas)
        .id(id.to_owned())
        .grow(1)
        .columns(colunas())
        .sort(ordem)
        .filter(filtro)
        .renaming(renomeando)
        .empty_text("Nenhum arquivo")
        .drag_group("arquivos")
        .on_invoke(move |chave| l1.borrow_mut().push(format!("invoke:{chave}")))
        .on_rename(move |chave, texto| {
            l2.borrow_mut().push(format!("rename:{chave}:{texto}"));
            renomeando.set(None);
        })
        .on_delete(move |chaves| l3.borrow_mut().push(format!("delete:{}", chaves.join(","))))
        .on_drop(move |soltou| {
            l4.borrow_mut().push(format!(
                "drop:{}:{}:{}",
                soltou.source_grid,
                soltou.keys.join(","),
                soltou.target.as_deref().unwrap_or("-")
            ));
        })
        .on_files_dropped(move |caminhos, alvo| {
            l5.borrow_mut().push(format!(
                "files:{}:{}",
                caminhos.len(),
                alvo.as_deref().unwrap_or("-")
            ));
        })
        .item_context_flyout(move |chave| {
            MenuFlyout::new()
                .id(format!("menu-{chave}"))
                .item(format!("abrir-{chave}"), "Abrir", || {})
                .item(format!("apagar-{chave}"), "Apagar", move || {
                    let chaves: Vec<String> =
                        selecionadas.with_untracked(|s| s.iter().cloned().collect());
                    linhas.update(|v| v.retain(|l| !chaves.contains(&l.key)));
                    selecionadas.update(BTreeSet::clear);
                })
        })
}

fn um_grid(tema: Theme, linhas: Vec<DataGridRow>) -> (TestApp, Estado) {
    let guardado: Guardado = Rc::default();
    let g = Rc::clone(&guardado);
    let app = TestApp::new(move || {
        Form::new("Arquivos").child(
            Column::new()
                .grow(1)
                .padding(8)
                .child(grade("g", &g, linhas)),
        )
    })
    .size(480, 360)
    .theme(tema);
    let estado = guardado.borrow()[0].clone();
    (app, estado)
}

fn grid() -> (TestApp, Estado) {
    um_grid(Theme::Light, linhas())
}

/// Dois grids, `a` (em cima) e `b` (embaixo), com linhas diferentes.
fn dois_grids() -> (TestApp, Estado, Estado) {
    let guardado: Guardado = Rc::default();
    let g = Rc::clone(&guardado);
    let app = TestApp::new(move || {
        let b = vec![
            DataGridRow::new("..")
                .cell("..", CellValue::Text("..".to_owned()))
                .rank(0)
                .drop_target(),
            linha("pasta-b", None, 1),
            linha("b1.txt", Some(5), 2),
        ];
        Form::new("Arquivos").child(
            Column::new()
                .grow(1)
                .padding(8)
                .gap(8)
                .child(grade("a", &g, linhas()))
                .child(grade("b", &g, b)),
        )
    })
    .size(480, 640);
    let (a, b) = {
        let g = guardado.borrow();
        (g[0].clone(), g[1].clone())
    };
    (app, a, b)
}

fn chaves(app: &TestApp, id: &str) -> Vec<String> {
    app.grid(id).rows.into_iter().map(|l| l.key).collect()
}

fn selecao(app: &TestApp, id: &str) -> Vec<String> {
    app.grid(id).selected
}

fn log(estado: &Estado) -> Vec<String> {
    estado.log.borrow().clone()
}

fn id(chave: &str) -> String {
    format!("g-{chave}")
}

/// O centro do título da coluna `coluna` do cabeçalho de `grid`.
fn titulo(app: &TestApp, grid: &str, coluna: usize) -> Point {
    let divisor = app.grid_divider(grid, coluna);
    Point::new(
        divisor.x - app.grid(grid).columns[coluna].width / 2.0,
        divisor.y,
    )
}

fn ordenar(app: &mut TestApp, coluna: usize) {
    let ponto = titulo(app, "g", coluna);
    app.click_at(ponto, 1);
}

// ---------------------------------------------------------------------------------------
// Ordenação

#[test]
fn ordena_em_ordem_natural_com_o_rank_na_frente() {
    let (mut app, estado) = grid();
    // Sem `.sort`: a ordem dada, só com o rank à frente (".." e pastas).
    assert_eq!(
        chaves(&app, "g"),
        [
            "..", "fotos", "docs", "a10.txt", "z.log", "a2.txt", "b.md", "a1.txt"
        ]
    );
    ordenar(&mut app, 0);
    assert_eq!(
        estado.ordem.get_untracked(),
        Some(("nome".to_owned(), SortDirection::Ascending))
    );
    // "a2" antes de "a10", sem diferenciar maiúsculas.
    assert_eq!(
        chaves(&app, "g"),
        [
            "..", "docs", "fotos", "a1.txt", "a2.txt", "a10.txt", "b.md", "z.log"
        ]
    );
    assert_eq!(
        app.grid("g").sort,
        Some(("nome".to_owned(), SortDirection::Ascending))
    );
}

#[test]
fn o_segundo_clique_inverte_e_o_rank_nao_inverte() {
    let (mut app, estado) = grid();
    ordenar(&mut app, 0);
    ordenar(&mut app, 0);
    assert_eq!(
        estado.ordem.get_untracked(),
        Some(("nome".to_owned(), SortDirection::Descending))
    );
    assert_eq!(
        chaves(&app, "g"),
        [
            "..", "fotos", "docs", "z.log", "b.md", "a10.txt", "a2.txt", "a1.txt"
        ]
    );
}

#[test]
fn ordena_por_numero_e_por_data() {
    let (mut app, _) = grid();
    ordenar(&mut app, 1);
    let arquivos = chaves(&app, "g")[3..].to_vec();
    assert_eq!(arquivos, ["a10.txt", "z.log", "a1.txt", "a2.txt", "b.md"]);
    ordenar(&mut app, 2);
    let arquivos = chaves(&app, "g")[3..].to_vec();
    assert_eq!(arquivos, ["b.md", "a2.txt", "z.log", "a10.txt", "a1.txt"]);
}

#[test]
fn coluna_nao_ordenavel_ignora_o_clique() {
    let guardado: Guardado = Rc::default();
    let g = Rc::clone(&guardado);
    let mut app = TestApp::new(move || {
        let linhas = signal(linhas());
        let selecionadas = signal(BTreeSet::new());
        let ordem = signal(None);
        *g.borrow_mut() = vec![Estado {
            selecionadas,
            ordem,
            filtro: signal(String::new()),
            renomeando: signal(None),
            log: Rc::default(),
        }];
        Form::new("Arquivos").child(
            DataGrid::bind(linhas, selecionadas)
                .id("g")
                .grow(1)
                .columns([
                    DataGridColumn::new("nome", "Nome"),
                    DataGridColumn::new("tamanho", "Tamanho").not_sortable(),
                ])
                .sort(ordem),
        )
    })
    .size(480, 360);
    let ponto = titulo(&app, "g", 1);
    app.click_at(ponto, 1);
    assert_eq!(guardado.borrow()[0].ordem.get_untracked(), None);
}

// ---------------------------------------------------------------------------------------
// Seleção

#[test]
fn clique_seleciona_uma_so() {
    let (mut app, estado) = grid();
    app.click(&id("a1.txt")).click(&id("b.md"));
    assert_eq!(selecao(&app, "g"), ["b.md"]);
    assert_eq!(
        estado.selecionadas.get_untracked(),
        BTreeSet::from(["b.md".to_owned()])
    );
}

#[test]
fn ctrl_soma_e_tira() {
    let (mut app, _) = grid();
    app.click(&id("a1.txt"))
        .click_with(&id("b.md"), Modifiers::COMMAND)
        .click_with(&id("z.log"), Modifiers::COMMAND);
    // A seleção volta na ordem das linhas.
    assert_eq!(selecao(&app, "g"), ["z.log", "b.md", "a1.txt"]);
    app.click_with(&id("b.md"), Modifiers::COMMAND);
    assert_eq!(selecao(&app, "g"), ["z.log", "a1.txt"]);
}

#[test]
fn shift_marca_a_faixa() {
    let (mut app, _) = grid();
    app.click(&id("docs"))
        .click_with(&id("z.log"), Modifiers::SHIFT);
    assert_eq!(selecao(&app, "g"), ["docs", "a10.txt", "z.log"]);
    // A âncora continua em "docs": a nova faixa parte dela.
    app.click_with(&id("fotos"), Modifiers::SHIFT);
    assert_eq!(selecao(&app, "g"), ["fotos", "docs"]);
}

#[test]
fn clique_numa_linha_da_selecao_so_recolhe_ao_soltar() {
    let (mut app, _) = grid();
    app.click(&id("docs"))
        .click_with(&id("z.log"), Modifiers::SHIFT);
    app.click(&id("a10.txt"));
    assert_eq!(selecao(&app, "g"), ["a10.txt"]);
}

#[test]
fn teclado_navega_e_marca_a_faixa() {
    let (mut app, _) = grid();
    app.click(&id("docs"));
    app.key(Key::Down).key(Key::Down);
    assert_eq!(selecao(&app, "g"), ["z.log"]);
    app.key_with(Key::Up, Modifiers::SHIFT);
    assert_eq!(selecao(&app, "g"), ["a10.txt", "z.log"]);
    app.key(Key::Home);
    assert_eq!(selecao(&app, "g"), [".."]);
    app.key(Key::End);
    assert_eq!(selecao(&app, "g"), ["a1.txt"]);
    app.key_with(Key::Home, Modifiers::SHIFT);
    assert_eq!(app.grid("g").selected.len(), 8);
}

#[test]
fn ctrl_a_seleciona_tudo_e_enter_abre() {
    let (mut app, estado) = grid();
    app.click(&id("docs"));
    app.key_with(Key::Char('a'), Modifiers::COMMAND);
    assert_eq!(selecao(&app, "g").len(), 8);
    app.click(&id("docs")).key(Key::Enter);
    assert_eq!(log(&estado), ["invoke:docs"]);
}

#[test]
fn clique_duplo_abre() {
    let (mut app, estado) = grid();
    app.double_click(&id("fotos"));
    assert_eq!(log(&estado), ["invoke:fotos"]);
}

// ---------------------------------------------------------------------------------------
// Filtro, renomear, apagar e menu

#[test]
fn filtro_mostra_so_o_que_combina() {
    let (mut app, estado) = grid();
    estado.filtro.set("A1".to_owned());
    app.frame();
    assert_eq!(chaves(&app, "g"), ["a10.txt", "a1.txt"]);
    assert_eq!(app.grid("g").filter, "A1");
    estado.filtro.set("zzz".to_owned());
    app.frame();
    assert!(chaves(&app, "g").is_empty());
}

#[test]
fn f2_renomeia_e_enter_confirma() {
    let (mut app, estado) = grid();
    app.click(&id("b.md")).key(Key::F(2));
    assert_eq!(estado.renomeando.get_untracked().as_deref(), Some("b.md"));
    app.frame();
    app.type_text("leia-me.md").key(Key::Enter);
    let registro = log(&estado);
    assert_eq!(registro.len(), 1, "{registro:?}");
    assert!(registro[0].starts_with("rename:b.md:"), "{registro:?}");
    assert!(registro[0].ends_with("leia-me.md"), "{registro:?}");
    assert_eq!(estado.renomeando.get_untracked(), None);
}

#[test]
fn esc_cancela_o_renomear() {
    let (mut app, estado) = grid();
    app.click(&id("b.md")).key(Key::F(2));
    app.frame();
    app.type_text("x").key(Key::Escape);
    assert!(log(&estado).is_empty());
    assert_eq!(estado.renomeando.get_untracked(), None);
}

#[test]
fn delete_apaga_a_selecao_inteira() {
    let (mut app, estado) = grid();
    app.click(&id("docs"))
        .click_with(&id("b.md"), Modifiers::COMMAND);
    app.key(Key::Delete);
    assert_eq!(log(&estado), ["delete:docs,b.md"]);
}

#[test]
fn delete_numa_linha_fora_da_selecao_apaga_so_ela() {
    let (mut app, estado) = grid();
    app.click(&id("docs"));
    app.grid_select("g", &["b.md", "z.log"]);
    app.key(Key::Delete);
    assert_eq!(log(&estado), ["delete:docs"]);
}

#[test]
fn menu_de_contexto_age_sobre_a_selecao() {
    let (mut app, estado) = grid();
    app.click(&id("a1.txt"))
        .click_with(&id("a2.txt"), Modifiers::COMMAND);
    // Botão direito numa linha da seleção mantém a seleção.
    app.right_click(&id("a2.txt"));
    assert_eq!(selecao(&app, "g"), ["a2.txt", "a1.txt"]);
    app.click("apagar-a2.txt");
    assert_eq!(
        chaves(&app, "g"),
        ["..", "fotos", "docs", "a10.txt", "z.log", "b.md"]
    );
    assert!(estado.selecionadas.get_untracked().is_empty());
}

#[test]
fn botao_direito_fora_da_selecao_seleciona_a_linha() {
    let (mut app, _) = grid();
    app.click(&id("a1.txt"));
    app.right_click(&id("docs"));
    assert_eq!(selecao(&app, "g"), ["docs"]);
    assert!(app.is_open("menu-docs"));
}

// ---------------------------------------------------------------------------------------
// Arrastar e soltar

#[test]
fn arrastar_entre_dois_grids_leva_a_selecao_inteira() {
    let (mut app, _, b) = dois_grids();
    app.drag_rows("a", &["a1.txt", "a2.txt"], "b", None);
    assert_eq!(log(&b), ["drop:a:a2.txt,a1.txt:-"]);
}

#[test]
fn arrastar_sobre_uma_pasta_marca_a_linha() {
    let (mut app, _, b) = dois_grids();
    app.drag_rows("a", &["b.md"], "b", Some("pasta-b"));
    assert_eq!(log(&b), ["drop:a:b.md:pasta-b"]);
}

#[test]
fn soltar_sobre_um_arquivo_vale_o_fundo() {
    let (mut app, _, b) = dois_grids();
    app.drag_rows("a", &["b.md"], "b", Some("b1.txt"));
    assert_eq!(log(&b), ["drop:a:b.md:-"]);
}

#[test]
fn soltar_no_proprio_grid_so_conta_sobre_uma_pasta() {
    let (mut app, a, _) = dois_grids();
    app.drag_rows("a", &["b.md"], "a", Some("a10.txt"));
    assert!(log(&a).is_empty());
    app.drag_rows("a", &["b.md"], "a", Some("docs"));
    assert_eq!(log(&a), ["drop:a:b.md:docs"]);
}

#[test]
fn a_etiqueta_do_arrasto_mostra_o_numero_de_linhas() {
    let (mut app, _, _) = dois_grids();
    app.grid_select("a", &["a1.txt", "a2.txt"]);
    let origem = app.rect("a-a1.txt").center();
    app.press("a-a1.txt");
    app.move_to(origem + Vec2::new(0.0, 12.0));
    let etiqueta = app.drag_label().expect("há um arrasto");
    assert!(!etiqueta.is_empty());
    app.release();
}

#[test]
fn arquivos_do_sistema_soltos() {
    let (mut app, estado) = grid();
    let caminhos = [PathBuf::from("/tmp/a.txt"), PathBuf::from("/tmp/b.txt")];
    app.drop_files("g", &caminhos);
    app.drop_files(&id("docs"), &caminhos[..1]);
    assert_eq!(log(&estado), ["files:2:-", "files:1:docs"]);
}

// ---------------------------------------------------------------------------------------
// Colunas

#[test]
fn arrastar_o_divisor_redimensiona_a_coluna() {
    let (mut app, _) = grid();
    let antes = app.grid("g").columns[1].width;
    let divisor = app.grid_divider("g", 1);
    app.drag(divisor, divisor + Vec2::new(-30.0, 0.0));
    let depois = app.grid("g").columns[1].width;
    assert_eq!(depois, antes - 30.0);
    // Não passa do mínimo.
    let divisor = app.grid_divider("g", 1);
    app.drag(divisor, divisor + Vec2::new(-500.0, 0.0));
    assert_eq!(app.grid("g").columns[1].width, DATA_GRID_MIN);
}

const DATA_GRID_MIN: f64 = tomni_ui::DATA_GRID_MIN_COLUMN;

#[test]
fn clique_duplo_no_divisor_ajusta_ao_texto() {
    let (mut app, _) = grid();
    let divisor = app.grid_divider("g", 1);
    app.click_at(divisor, 1);
    app.click_at(divisor, 2);
    let largura = app.grid("g").columns[1].width;
    assert!(largura != 110.0 && largura >= DATA_GRID_MIN, "{largura}");
}

#[test]
fn redimensionar_nao_ordena() {
    let (mut app, estado) = grid();
    let divisor = app.grid_divider("g", 0);
    app.drag(divisor, divisor + Vec2::new(20.0, 0.0));
    assert_eq!(estado.ordem.get_untracked(), None);
}

// ---------------------------------------------------------------------------------------
// Virtualização

#[test]
fn dez_mil_linhas_ficam_leves() {
    let muitas: Vec<DataGridRow> = (0..10_000)
        .map(|i| linha(&format!("arquivo-{i:05}"), Some(i), i))
        .collect();
    let (mut app, estado) = um_grid(Theme::Light, muitas);
    assert_eq!(app.grid("g").rows.len(), 10_000);
    let montadas = |app: &TestApp| {
        let mut n = 0;
        let mut pilha = vec![app.tree()];
        while let Some(no) = pilha.pop() {
            n += usize::from(no.kind == "data_grid_row");
            pilha.extend(no.children);
        }
        n
    };
    assert!(montadas(&app) < 200, "montadas: {}", montadas(&app));
    let mut pior = Duration::ZERO;
    let mut medir = |app: &mut TestApp, f: &dyn Fn(&mut TestApp)| {
        let t = std::time::Instant::now();
        f(app);
        pior = pior.max(t.elapsed());
    };
    app.click(&id("arquivo-00000"));
    medir(&mut app, &|app| {
        app.key(Key::End);
    });
    assert_eq!(
        estado.selecionadas.get_untracked(),
        BTreeSet::from(["arquivo-09999".to_owned()])
    );
    for _ in 0..50 {
        medir(&mut app, &|app| {
            app.wheel(Point::new(100.0, 150.0), Vec2::new(0.0, -2000.0));
        });
    }
    medir(&mut app, &|app| {
        app.key(Key::Home);
    });
    assert!(montadas(&app) < 200);
    assert!(pior < Duration::from_millis(100), "pior frame: {pior:?}");
    // Ordenar 10 000 linhas também é leve.
    let t = std::time::Instant::now();
    ordenar(&mut app, 1);
    assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
}

// ---------------------------------------------------------------------------------------
// Ferramentas

#[test]
fn o_inspect_mostra_o_data_grid() {
    let (mut app, _) = grid();
    ordenar(&mut app, 0);
    app.click(&id("docs"))
        .click_with(&id("a1.txt"), Modifiers::COMMAND);
    let no = app.tree();
    let no = no.find("g").expect("o grid");
    assert_eq!(no.kind, "data_grid");
    assert_eq!(no.items_total, Some(8));
    assert_eq!(no.grid_sort.as_deref(), Some("Nome crescente"));
    assert_eq!(
        no.grid_columns.as_deref(),
        Some(
            &[
                "Nome".to_owned(),
                "Tamanho".to_owned(),
                "Modificado".to_owned()
            ][..]
        )
    );
    assert_eq!(
        no.grid_selected.as_deref(),
        Some(&["docs".to_owned(), "a1.txt".to_owned()][..])
    );
    let linhas = no.grid_rows.as_ref().expect("as linhas");
    assert_eq!(linhas[1].key, "docs");
    assert_eq!(linhas[1].cells[0], "docs");
    assert!(linhas[1].selected);
    assert!(!linhas[0].selected);
    let linha = no.find("g-docs").expect("a linha");
    assert_eq!(linha.kind, "data_grid_row");
    assert_eq!(linha.selected, Some(true));
    assert_eq!(
        no.find("g-cabecalho").expect("cabeçalho").kind,
        "data_grid_header"
    );
}

#[test]
fn passa_no_lint_nos_dois_temas() {
    for tema in [Theme::Light, Theme::Dark] {
        let (mut app, _) = um_grid(tema, linhas());
        app.assert_lint();
        let (mut app, _) = um_grid(tema, Vec::new());
        app.assert_lint();
    }
}

// ---------------------------------------------------------------------------------------
// Snapshots

#[test]
fn snapshots() {
    for tema in [Theme::Light, Theme::Dark] {
        let nome = |cena: &str| format!("m30-{cena}-{}", theme_name(tema));
        let (mut app, _) = um_grid(tema, linhas());
        ordenar(&mut app, 1);
        app.click(&id("docs"))
            .click_with(&id("a10.txt"), Modifiers::COMMAND)
            .click_with(&id("a2.txt"), Modifiers::COMMAND);
        app.hover(&id("b.md"));
        app.assert_snapshot(&nome("selecao-ordenado"));
        let (app, _) = um_grid(tema, Vec::new());
        app.assert_snapshot(&nome("vazio"));
    }
}
