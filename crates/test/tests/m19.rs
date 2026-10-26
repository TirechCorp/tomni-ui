//! Marco M19: a densidade compacta e as abas completas. O formulário compacto e o padrão, o
//! `TabView` compacto (escolhida, hover, fechar e "+", rolado com as setas) e o padrão com o
//! botão de fechar, nos 2 temas; a herança da densidade, o alvo de toque do lint nas duas, as
//! abas de uma lista, o fechar (botão, clique do meio e Ctrl+F4), as setas e a largura.

mod comum;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use comum::theme_name;
use tomni_ui::lint::Rule;
use tomni_ui::prelude::*;
use tomni_ui::theme::Theme;
use tomni_ui::{
    Key, Look, Modifiers, TAB_BUTTON_SIZE_COMPACT, TAB_HEIGHT, TAB_HEIGHT_COMPACT, TAB_MAX_WIDTH,
    TAB_MIN_WIDTH, TAB_SCROLL_STEP, TabWidth,
};
use tomni_ui_test::TestApp;

const TEMAS: [Theme; 2] = [Theme::Light, Theme::Dark];

/// Uma nota aberta: o título e o texto.
#[derive(Clone)]
struct Nota {
    id: u32,
    titulo: Signal<String>,
    texto: Signal<String>,
}

fn nota(id: u32, titulo: &str) -> Nota {
    Nota {
        id,
        titulo: signal(titulo.to_owned()),
        texto: signal(String::new()),
    }
}

/// O `TabView` das notas, com fechar e "+", e o que o teste quer ver de fora.
struct Notas {
    lista: Signal<Vec<Nota>>,
    aberta: Signal<String>,
    fechadas: Rc<RefCell<Vec<String>>>,
}

/// Notas com os títulos `titulos` (chaves 1, 2, ...), num `TabView` de densidade `density`.
fn notas(
    titulos: &[&str],
    density: Density,
) -> (impl FnOnce() -> Form, Rc<RefCell<Option<Notas>>>) {
    let titulos: Vec<String> = titulos.iter().map(|t| (*t).to_owned()).collect();
    let fora = Rc::new(RefCell::new(None));
    let guarda = Rc::clone(&fora);
    let view = move || {
        let lista = signal(
            titulos
                .iter()
                .zip(1..)
                .map(|(t, id)| nota(id, t))
                .collect::<Vec<_>>(),
        );
        let aberta = signal("1".to_owned());
        let fechadas = Rc::new(RefCell::new(Vec::new()));
        let registro = Rc::clone(&fechadas);
        *guarda.borrow_mut() = Some(Notas {
            lista,
            aberta,
            fechadas: Rc::clone(&fechadas),
        });
        Form::new("Notas").density(density).child(
            TabView::bind(aberta)
                .id("notas")
                .grow(1)
                .items(
                    lista,
                    |n| n.id,
                    |n| n.titulo,
                    move |n| {
                        let texto = Edit::bind(n.texto).id(format!("texto-{}", n.id));
                        if density == Density::Standard {
                            texto.height(44)
                        } else {
                            texto
                        }
                    },
                )
                .on_close(move |chave| {
                    registro.borrow_mut().push(chave.clone());
                    lista.update(|v| v.retain(|n| n.id.to_string() != chave));
                })
                .on_add(move || {
                    let id =
                        lista.with_untracked(|v| v.iter().map(|n| n.id).max().unwrap_or(0)) + 1;
                    lista.update(|v| v.push(nota(id, &format!("Nota {id}"))));
                    aberta.set(id.to_string());
                }),
        )
    };
    (view, fora)
}

fn com<R>(fora: &Rc<RefCell<Option<Notas>>>, f: impl FnOnce(&Notas) -> R) -> R {
    f(fora.borrow().as_ref().expect("a tela foi montada"))
}

/// Campos, `ComboBox`, botões e `CheckBox` na densidade `density`.
fn formulario(density: Density) -> Form {
    let nome = signal("Servidor de produção".to_owned());
    let porta = signal(22.0);
    let protocolo = signal("SSH".to_owned());
    let lembrar = signal(true);
    // No padrão, os campos e os botões sobem para os 44 px do alvo de toque; no compacto, os
    // campos ficam com os 24 e os botões com os 32 do WinUI.
    let padrao = density == Density::Standard;
    let campo = Edit::bind(nome).id("nome");
    let campo = if padrao { campo.height(44) } else { campo };
    let numero = NumberBox::bind(porta).id("porta");
    let numero = if padrao { numero.height(44) } else { numero };
    let lista = ComboBox::bind(protocolo, ["SSH", "Telnet", "Serial"]).id("protocolo");
    let lista = if padrao { lista.height(44) } else { lista };
    let caixa = CheckBox::bind(lembrar, "Lembrar a senha").id("lembrar");
    let caixa = if padrao { caixa.height(44) } else { caixa };
    let conectar = Button::new("Conectar").id("conectar").accent();
    let conectar = if padrao {
        conectar.height(44)
    } else {
        conectar
    };
    let cancelar = Button::new("Cancelar").id("cancelar");
    let cancelar = if padrao {
        cancelar.height(44)
    } else {
        cancelar
    };
    Form::new("Conexão").density(density).child(
        Column::new()
            .padding(16)
            .gap(8)
            .child(Text::new("Nome"))
            .child(campo)
            .child(Text::new("Porta"))
            .child(numero)
            .child(Text::new("Protocolo"))
            .child(lista)
            .child(caixa)
            .child(
                Row::new()
                    .gap(8)
                    .align(Align::Center)
                    .child(conectar)
                    .child(cancelar)
                    .child(
                        Button::icon_only(Symbol::Settings)
                            .id("opcoes")
                            .label("Opções"),
                    ),
            ),
    )
}

#[test]
fn snapshots_do_formulario_nas_duas_densidades() {
    for tema in TEMAS {
        for (density, nome) in [
            (Density::Compact, "compacto"),
            (Density::Standard, "padrao"),
        ] {
            let mut app = TestApp::new(move || formulario(density))
                .size(360, 380)
                .theme(tema);
            app.assert_lint();
            // Os campos têm 24 px no compacto; o botão só de ícone, 32 × 24 px.
            let (campo, botao, icone) = match density {
                Density::Compact => (24.0, 32.0, (32.0, 24.0)),
                Density::Standard => (44.0, 44.0, (44.0, 44.0)),
            };
            for id in ["nome", "porta", "protocolo"] {
                assert_eq!(app.rect(id).height(), campo, "{id} no {nome}");
            }
            assert_eq!(app.rect("conectar").height(), botao);
            let opcoes = app.rect("opcoes");
            assert_eq!((opcoes.width(), opcoes.height()), icone);
            app.assert_snapshot(&format!("m19_formulario_{nome}_{}", theme_name(tema)));
        }
    }
}

#[test]
fn densidade_passa_para_os_filhos_e_um_filho_volta_ao_padrao() {
    let mut app = TestApp::new(|| {
        let a = signal(String::new());
        let b = signal(String::new());
        Form::new("Densidade").density(Density::Compact).child(
            Column::new()
                .padding(16)
                .gap(8)
                .child(Edit::bind(a).id("compacto"))
                .child(
                    Column::new()
                        .id("padrao")
                        .density(Density::Standard)
                        .child(Edit::bind(b).id("normal").height(44)),
                ),
        )
    });
    assert_eq!(app.rect("compacto").height(), 24.0);
    assert!(app.rect("normal").height() > 24.0);
    let tree = app.tree();
    assert_eq!(tree.density, Some("compact"));
    assert_eq!(tree.find("padrao").unwrap().density, Some("standard"));
    assert_eq!(tree.find("compacto").unwrap().density, None);
    assert_eq!(tree.find("normal").unwrap().density, None);
    app.assert_lint();
}

/// Problemas de alvo de toque num botão de `height` px na densidade `density`.
fn alvos(density: Density, height: f64) -> Vec<String> {
    let mut app = TestApp::new(move || {
        Form::new("Alvo").density(density).child(
            Column::new()
                .padding(16)
                .child(Button::new("Salvar").id("salvar").width(80).height(height)),
        )
    });
    app.lint()
        .into_iter()
        .filter(|i| i.rule == Rule::TouchTarget)
        .map(|i| i.message)
        .collect()
}

#[test]
fn lint_do_alvo_de_toque_nas_duas_densidades() {
    assert_eq!(alvos(Density::Compact, 24.0), Vec::<String>::new());
    let compacto = alvos(Density::Compact, 23.0);
    assert_eq!(compacto.len(), 1, "{compacto:?}");
    assert!(
        compacto[0].contains("24×24 na densidade compacta"),
        "{compacto:?}"
    );
    assert_eq!(alvos(Density::Standard, 44.0), Vec::<String>::new());
    let padrao = alvos(Density::Standard, 32.0);
    assert_eq!(padrao.len(), 1, "{padrao:?}");
    assert!(
        padrao[0].contains("44×44 na densidade padrão"),
        "{padrao:?}"
    );
}

#[test]
fn lista_cria_descarta_e_preserva_as_abas() {
    let (view, fora) = notas(&["Compras", "Ideias", "Viagem"], Density::Compact);
    let mut app = TestApp::new(view).size(600, 200);
    assert_eq!(app.tab_titles("notas"), ["Compras", "Ideias", "Viagem"]);
    assert!(app.visible("notas-1-conteudo"));
    assert!(!app.visible("notas-2-conteudo"));

    // O texto digitado na aba 2 continua lá depois de remover a 1 e de mover as outras.
    app.click("notas-2");
    app.type_into("texto-2", "passagens");
    com(&fora, |n| n.lista.update(|v| v.retain(|n| n.id != 1)));
    app.frame();
    assert_eq!(app.tab_titles("notas"), ["Ideias", "Viagem"]);
    assert!(app.ui().find("notas-1").is_none());
    assert!(app.ui().find("texto-1").is_none());
    assert_eq!(app.text("texto-2"), "passagens");

    com(&fora, |n| n.lista.update(|v| v.reverse()));
    app.frame();
    assert_eq!(app.tab_titles("notas"), ["Viagem", "Ideias"]);
    assert_eq!(app.text("texto-2"), "passagens");
    assert!(app.rect("notas-3").x0 < app.rect("notas-2").x0);

    // Um item novo ganha aba, e o título acompanha o signal.
    com(&fora, |n| n.lista.update(|v| v.push(nota(7, "Livros"))));
    app.frame();
    assert_eq!(app.tab_titles("notas"), ["Viagem", "Ideias", "Livros"]);
    com(&fora, |n| {
        n.lista
            .with_untracked(|v| v[2].titulo.set("Leituras".to_owned()));
    });
    app.frame();
    assert_eq!(app.label("notas-7"), "Leituras");
    assert_eq!(app.tab_titles("notas"), ["Viagem", "Ideias", "Leituras"]);
    assert_eq!(app.text("texto-2"), "passagens");
    app.assert_lint();
}

#[test]
fn fechar_pelo_botao_pelo_meio_e_pelo_ctrl_f4() {
    let (view, fora) = notas(&["Um", "Dois", "Três", "Quatro"], Density::Compact);
    let mut app = TestApp::new(view).size(600, 200);
    let fechadas = com(&fora, |n| Rc::clone(&n.fechadas));

    // O X só aparece na escolhida e na aba sob o ponteiro.
    assert!(app.visible("notas-1-fechar"));
    assert!(!app.visible("notas-3-fechar"));
    app.hover("notas-3");
    assert!(app.visible("notas-3-fechar"));

    // Fechar uma aba que não é a escolhida não muda a escolha.
    app.close_tab("notas", "3");
    assert_eq!(*fechadas.borrow(), ["3"]);
    assert_eq!(app.tab_titles("notas"), ["Um", "Dois", "Quatro"]);
    assert_eq!(app.text("notas"), "1");

    // Fechar a escolhida passa a escolha para a da direita.
    app.middle_click("notas-1");
    assert_eq!(*fechadas.borrow(), ["3", "1"]);
    assert_eq!(app.tab_titles("notas"), ["Dois", "Quatro"]);
    assert_eq!(app.text("notas"), "2");
    assert_eq!(com(&fora, |n| n.aberta.get_untracked()), "2");

    // Da última, para a da esquerda.
    app.click("notas-4");
    app.click("texto-4");
    app.key_with(Key::F(4), Modifiers::CTRL);
    assert_eq!(*fechadas.borrow(), ["3", "1", "4"]);
    assert_eq!(app.tab_titles("notas"), ["Dois"]);
    assert_eq!(app.text("notas"), "2");
    assert!(app.visible("notas-2-conteudo"));
}

#[test]
fn adicionar_cria_a_aba_e_a_escolhe() {
    let (view, _fora) = notas(&["Um"], Density::Compact);
    let mut app = TestApp::new(view).size(600, 200);
    let um = app.rect("notas-1");
    let mais = app.rect("notas-adicionar");
    assert_eq!(mais.size(), TAB_BUTTON_SIZE_COMPACT);
    assert_eq!(mais.x0, um.x1 + 3.0);
    app.click("notas-adicionar");
    assert_eq!(app.tab_titles("notas"), ["Um", "Nota 2"]);
    assert_eq!(app.text("notas"), "2");
    assert!(app.visible("texto-2"));
    // O "+" é parada de Tab, entre as abas e o conteúdo.
    app.click("notas-2");
    app.tab();
    assert_eq!(app.focused().as_deref(), Some("notas-adicionar"));
    app.tab();
    assert_eq!(app.focused().as_deref(), Some("texto-2"));
}

#[test]
fn setas_aparecem_rolam_e_repetem() {
    let titulos: Vec<String> = (1..=8).map(|i| format!("Nota {i}")).collect();
    let titulos: Vec<&str> = titulos.iter().map(String::as_str).collect();
    let (view, fora) = notas(&titulos[..2], Density::Compact);
    let mut app = TestApp::new(view).size(500, 200);
    assert!(!app.visible("notas-rolar-esquerda"));
    assert!(!app.visible("notas-rolar-direita"));
    assert_eq!(app.scroll_offset("notas-abas").x, 0.0);

    // Com mais abas do que cabem, as setas aparecem; a da esquerda começa desabilitada.
    com(&fora, |n| {
        n.lista
            .set(titulos.iter().zip(1..).map(|(t, id)| nota(id, t)).collect());
    });
    app.frame();
    assert!(app.visible("notas-rolar-esquerda"));
    assert!(app.visible("notas-rolar-direita"));
    assert!(!app.enabled("notas-rolar-esquerda"));
    assert!(app.enabled("notas-rolar-direita"));
    assert_eq!(app.rect("notas-1").width(), TAB_MIN_WIDTH);

    app.click("notas-rolar-direita");
    assert_eq!(app.scroll_offset("notas-abas").x, TAB_SCROLL_STEP);
    assert!(app.enabled("notas-rolar-esquerda"));

    // Pressionada, repete depois de 50 ms e a cada 100 ms.
    app.press("notas-rolar-direita");
    assert_eq!(app.scroll_offset("notas-abas").x, 2.0 * TAB_SCROLL_STEP);
    app.wait(Duration::from_millis(40));
    assert_eq!(app.scroll_offset("notas-abas").x, 2.0 * TAB_SCROLL_STEP);
    app.wait(Duration::from_millis(10));
    assert_eq!(app.scroll_offset("notas-abas").x, 3.0 * TAB_SCROLL_STEP);
    app.wait(Duration::from_millis(100));
    assert_eq!(app.scroll_offset("notas-abas").x, 4.0 * TAB_SCROLL_STEP);
    app.release();
    app.wait(Duration::from_millis(300));
    assert_eq!(app.scroll_offset("notas-abas").x, 4.0 * TAB_SCROLL_STEP);

    // Até o fim: a da direita desabilita.
    for _ in 0..20 {
        app.click("notas-rolar-direita");
    }
    let content = app.scroll_content("notas-abas").width;
    let viewport = app.rect("notas-abas").width();
    assert_eq!(app.scroll_offset("notas-abas").x, content - viewport);
    assert!(!app.enabled("notas-rolar-direita"));

    // A faixa não mostra barra, e as setas não são parada de Tab.
    let tree = app.tree();
    let abas = tree.find("notas-abas").unwrap();
    assert!(abas.children.iter().all(|c| c.kind != "scroll_bar"));
    app.click("texto-1");
    app.shift_tab();
    assert_eq!(app.focused().as_deref(), Some("notas-adicionar"));
    app.shift_tab();
    assert_eq!(app.focused().as_deref(), Some("notas-1"));

    // Voltando a caber, as setas somem.
    com(&fora, |n| n.lista.update(|v| v.truncate(2)));
    app.frame();
    assert!(!app.visible("notas-rolar-esquerda"));
    assert!(!app.visible("notas-rolar-direita"));
}

#[test]
fn larguras_iguais_e_pelo_titulo() {
    // Igual: a faixa dividida entre as abas, entre 100 e 240 px.
    let abas = |width: TabWidth| {
        move || {
            let aba = signal("a".to_owned());
            Form::new("Larguras").child(
                TabView::bind(aba)
                    .id("abas")
                    .tab_width(width)
                    .tab("a", "A", Text::new("A"))
                    .tab("b", "Um título bem mais comprido", Text::new("B")),
            )
        }
    };
    let app = TestApp::new(abas(TabWidth::Equal)).size(400, 200);
    assert_eq!(app.rect("abas-a").width(), 192.0);
    assert_eq!(app.rect("abas-b").width(), 192.0);
    let app = TestApp::new(abas(TabWidth::Equal)).size(800, 200);
    assert_eq!(app.rect("abas-a").width(), TAB_MAX_WIDTH);

    // Pelo título: a curta fica no mínimo, a comprida passa dele.
    let app = TestApp::new(abas(TabWidth::SizeToContent)).size(800, 200);
    assert_eq!(app.rect("abas-a").width(), TAB_MIN_WIDTH);
    let b = app.rect("abas-b").width();
    assert!(b > TAB_MIN_WIDTH && b < TAB_MAX_WIDTH, "{b}");
    assert_eq!(app.rect("abas-a").height(), TAB_HEIGHT);
}

#[test]
fn compacto_tem_abas_de_24_px_sem_a_margem_de_cima() {
    let (view, _fora) = notas(&["Um", "Dois"], Density::Compact);
    let app = TestApp::new(view).size(600, 200);
    let um = app.rect("notas-1");
    assert_eq!(um.height(), TAB_HEIGHT_COMPACT);
    assert_eq!(um.y0, app.rect("notas").y0);
    assert_eq!(app.rect("notas-1-fechar").size(), TAB_BUTTON_SIZE_COMPACT);
    assert_eq!(app.rect("notas-1-fechar").x1, um.x1 - 4.0);
    let Look::TabView(visual) = app.look("notas") else {
        panic!("não é um TabView");
    };
    assert!(visual.closable);
    assert_eq!(
        visual.tabs,
        [
            ("1".to_owned(), "Um".to_owned()),
            ("2".to_owned(), "Dois".to_owned())
        ]
    );
}

#[test]
fn snapshots_do_tab_view_compacto() {
    for tema in TEMAS {
        let nome = |s: &str| format!("m19_abas_compactas_{s}_{}", theme_name(tema));

        // Escolhida, sem fechar nem "+".
        let mut app = TestApp::new(|| {
            let aba = signal("geral".to_owned());
            Form::new("Configurações").density(Density::Compact).child(
                TabView::bind(aba)
                    .id("config")
                    .grow(1)
                    .tab("geral", "Geral", Text::new("Opções do dia a dia."))
                    .tab("avancado", "Avançado", Text::new("Opções raras."))
                    .tab("sobre", "Sobre", Text::new("Versão 1.0.")),
            )
        })
        .size(390, 100)
        .theme(tema);
        app.assert_lint();
        app.assert_snapshot(&nome("escolhida"));
        app.hover("config-avancado");
        app.assert_snapshot(&nome("hover"));

        // Com fechar e "+", o ponteiro numa aba que não é a escolhida.
        let (view, _fora) = notas(&["Compras", "Ideias", "Viagem"], Density::Compact);
        let mut app = TestApp::new(view).size(480, 100).theme(tema);
        app.assert_lint();
        app.hover("notas-2");
        app.assert_snapshot(&nome("fechar"));

        // Rolado até o meio, com as duas setas.
        let titulos: Vec<String> = (1..=8).map(|i| format!("Nota {i}")).collect();
        let titulos: Vec<&str> = titulos.iter().map(String::as_str).collect();
        let (view, _fora) = notas(&titulos, Density::Compact);
        let mut app = TestApp::new(view).size(480, 100).theme(tema);
        app.click("notas-rolar-direita")
            .click("notas-rolar-direita");
        app.hover("texto-1");
        app.assert_lint();
        app.assert_snapshot(&nome("rolado"));
    }
}

#[test]
fn snapshots_do_tab_view_padrao_com_fechar() {
    for tema in TEMAS {
        let (view, _fora) = notas(&["Compras", "Ideias", "Viagem"], Density::Standard);
        let mut app = TestApp::new(view).size(560, 140).theme(tema);
        app.assert_lint();
        app.hover("notas-2");
        app.assert_snapshot(&format!("m19_abas_padrao_fechar_{}", theme_name(tema)));
    }
}
