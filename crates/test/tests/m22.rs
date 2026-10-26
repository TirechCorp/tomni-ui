//! Marco M22: o `PropertyGrid` e o seletor de arquivo.

mod comum;

use std::path::PathBuf;

use tomni_ui::dialogs;
use tomni_ui::kurbo::Point;
use tomni_ui::prelude::*;
use tomni_ui::theme::Theme;
use tomni_ui::{Look, Modifiers, PropertyGridRowVisual};
use tomni_ui_test::TestApp;

/// Um botão que pede um arquivo e mostra o que veio; `chamadas` conta os callbacks.
fn app_do_seletor() -> (TestApp, Signal<Option<Option<PathBuf>>>) {
    let guardado = std::rc::Rc::new(std::cell::Cell::new(None));
    let g = guardado.clone();
    let app = TestApp::new(move || {
        let resposta = signal(None::<Option<PathBuf>>);
        g.set(Some(resposta));
        Form::new("Seletor").child(Button::new("Procurar").id("procurar").on_click(move || {
            dialogs::open_file()
                .title("Chave")
                .filter("Chaves", &["pem", "key"])
                .directory("/tmp")
                .show(move |caminho| resposta.set(Some(caminho)));
        }))
    });
    (app, guardado.get().unwrap())
}

#[test]
fn seletor_resposta_com_caminho_chega_no_callback() {
    let (mut app, resposta) = app_do_seletor();
    app.click("procurar");
    assert_eq!(
        resposta.get_untracked(),
        None,
        "o callback espera a resposta"
    );
    app.file_dialog_answer(Some("/tmp/a.pem"));
    assert_eq!(
        resposta.get_untracked(),
        Some(Some(PathBuf::from("/tmp/a.pem")))
    );
}

#[test]
fn seletor_cancelado_chega_como_none() {
    let (mut app, resposta) = app_do_seletor();
    app.click("procurar");
    app.file_dialog_cancel();
    assert_eq!(resposta.get_untracked(), Some(None));
}

#[test]
fn seletor_resposta_adiantada_vale_para_o_proximo_pedido() {
    let (mut app, resposta) = app_do_seletor();
    app.file_dialog_answer(Some("/tmp/b.key"));
    assert_eq!(resposta.get_untracked(), None);
    app.click("procurar");
    app.frame();
    assert_eq!(
        resposta.get_untracked(),
        Some(Some(PathBuf::from("/tmp/b.key")))
    );
}

#[test]
fn seletor_registra_os_pedidos() {
    let (mut app, _) = app_do_seletor();
    app.click("procurar");
    let pedidos = app.file_dialogs();
    assert_eq!(pedidos.len(), 1);
    assert_eq!(pedidos[0].title, "Chave");
    assert_eq!(pedidos[0].filters[0].name, "Chaves");
    assert_eq!(pedidos[0].filters[0].extensions, ["pem", "key"]);
    assert_eq!(pedidos[0].directory, Some(PathBuf::from("/tmp")));
}

// ---------------------------------------------------------------------------------------
// PropertyGrid
// ---------------------------------------------------------------------------------------

/// Os signals de uma conexão.
#[derive(Clone, Copy)]
struct Conexao {
    host: Signal<String>,
    porta: Signal<f64>,
    protocolo: Signal<String>,
    usuario: Signal<String>,
    senha: Signal<String>,
    chave: Signal<String>,
    bipe: Signal<bool>,
}

impl Conexao {
    fn nova(host: &str, porta: f64, protocolo: &str, usuario: &str, senha: &str) -> Self {
        Self {
            host: signal(host.to_owned()),
            porta: signal(porta),
            protocolo: signal(protocolo.to_owned()),
            usuario: signal(usuario.to_owned()),
            senha: signal(senha.to_owned()),
            chave: signal(String::new()),
            bipe: signal(true),
        }
    }
}

/// As duas conexões e qual delas a grade mostra.
#[derive(Clone, Copy)]
struct Conexoes {
    a: Conexao,
    b: Conexao,
    escolhida: Signal<usize>,
    id_fixo: Signal<String>,
    desabilitada: Signal<bool>,
}

type Grade = (TestApp, Conexoes);

fn grade_com(densidade: Option<Density>) -> Grade {
    let guardado = std::rc::Rc::new(std::cell::Cell::new(None));
    let g = guardado.clone();
    let app = TestApp::new(move || {
        let conexoes = Conexoes {
            a: Conexao::nova("exemplo.com", 2222.0, "SSH", "ana", "segredo"),
            b: Conexao::nova("outro.com", 22.0, "Telnet", "bia", ""),
            escolhida: signal(0),
            id_fixo: signal("abc-123".to_owned()),
            desabilitada: signal(false),
        };
        g.set(Some(conexoes));
        let Conexoes {
            a,
            b,
            escolhida,
            id_fixo,
            desabilitada,
        } = conexoes;
        let atual = move || if escolhida.get() == 0 { a } else { b };
        let mut props = PropertyGrid::new()
            .id("props")
            .category("Conexão")
            .text("Host", move || atual().host)
            .description("Nome ou endereço do servidor.")
            .default("")
            .number("Porta", move || atual().porta, 1.0..=65535.0)
            .default(22)
            .choice(
                "Protocolo",
                move || atual().protocolo,
                ["SSH", "Telnet", "Serial"],
            )
            .default("SSH")
            .category("Autenticação")
            .text("Usuário", move || atual().usuario)
            .password("Senha", move || atual().senha)
            .file("Chave privada", move || atual().chave)
            .filter("Chaves", &["pem", "key"])
            .text("Opcional", move || atual().usuario)
            .disabled(desabilitada)
            .category("Terminal")
            .collapsed()
            .check("Bipe", move || atual().bipe)
            .default(true)
            .text("Id do sistema", id_fixo)
            .read_only()
            .grow(1);
        if let Some(densidade) = densidade {
            props = props.density(densidade);
        }
        Form::new("Conexão").child(
            Column::new()
                .child(Button::new("antes").id("antes").height(44))
                .child(props)
                .child(Button::new("depois").id("depois").height(44)),
        )
    })
    .size(480, 520);
    (app, guardado.get().unwrap())
}

fn grade() -> Grade {
    grade_com(None)
}

/// O visual da linha de propriedade `nome`.
fn linha(app: &TestApp, nome: &str) -> (String, String, PropertyGridRowVisual) {
    match app.look(&format!("props-{nome}")) {
        Look::PropertyGridRow {
            name,
            value,
            visual,
        } => (name, value, visual),
        outro => panic!("`props-{nome}` não é uma linha: {outro:?}"),
    }
}

fn existe(app: &TestApp, id: &str) -> bool {
    app.tree().find(id).is_some()
}

#[test]
fn grade_comeca_com_a_primeira_propriedade_escolhida_e_a_terminal_fechada() {
    let (app, _) = grade();
    assert!(linha(&app, "Host").2.selected);
    assert!(!linha(&app, "Porta").2.selected);
    assert!(existe(&app, "props-Usuário"));
    assert!(
        !existe(&app, "props-Bipe"),
        "a categoria Terminal começa fechada"
    );
    assert!(existe(&app, "props-categoria-Terminal"));
}

#[test]
fn clicar_numa_linha_escolhe_a_linha() {
    let (mut app, _) = grade();
    // O centro da linha é o divisor das colunas (50%): o clique vai no nome.
    clicar_no_nome(&mut app, "Porta");
    assert!(linha(&app, "Porta").2.selected);
    assert!(!linha(&app, "Host").2.selected);
}

#[test]
fn linhas_mostram_nome_e_valor_e_mascaram_a_senha() {
    let (app, _) = grade();
    let (nome, valor, _) = linha(&app, "Host");
    assert_eq!((nome.as_str(), valor.as_str()), ("Host", "exemplo.com"));
    assert_eq!(linha(&app, "Porta").1, "2222");
    assert_eq!(linha(&app, "Senha").1, "•••••••");
}

#[test]
fn negrito_acompanha_o_default() {
    let (mut app, c) = grade();
    assert!(linha(&app, "Host").2.changed, "exemplo.com difere de \"\"");
    assert!(linha(&app, "Porta").2.changed, "2222 difere de 22");
    assert!(!linha(&app, "Protocolo").2.changed);
    assert!(!linha(&app, "Usuário").2.changed, "sem .default nunca muda");
    c.a.porta.set(22.0);
    c.a.host.set(String::new());
    c.a.bipe.set(false);
    app.frame();
    assert!(!linha(&app, "Porta").2.changed);
    assert!(!linha(&app, "Host").2.changed);
}

#[test]
fn cabecalho_abre_e_fecha_com_clique_duplo_e_com_o_chevron() {
    let (mut app, _) = grade();
    app.double_click("props-categoria-Terminal");
    assert!(existe(&app, "props-Bipe"));
    let r = app.rect("props-categoria-Terminal");
    app.click_at(Point::new(r.x1 - 12.0, r.center().y), 1);
    assert!(!existe(&app, "props-Bipe"));
}

#[test]
fn alfabetica_ignora_acento_e_maiuscula_e_sem_cabecalhos() {
    let (mut app, _) = grade();
    app.click("props-alfabetica");
    let tree = app.tree();
    let grade = tree.find("props").expect("a grade");
    assert!(!existe(&app, "props-categoria-Conexão"));
    let nomes: Vec<&str> = ["Host", "Opcional", "Porta", "Protocolo", "Senha", "Usuário"]
        .into_iter()
        .collect();
    let ys: Vec<f64> = nomes
        .iter()
        .map(|n| app.rect(&format!("props-{n}")).y0)
        .collect();
    assert!(ys.windows(2).all(|w| w[0] < w[1]), "fora de ordem: {ys:?}");
    let _ = grade;
    app.click("props-alfabetica");
    assert!(existe(&app, "props-categoria-Conexão"));
    assert!(!existe(&app, "props-Bipe"), "a Terminal continua fechada");
}

#[test]
fn lint_passa_nos_dois_temas_e_nas_duas_densidades() {
    for tema in [Theme::Light, Theme::Dark] {
        for densidade in [None, Some(Density::Standard)] {
            let (mut app, _) = grade_com(densidade);
            app = app.theme(tema);
            app.assert_lint();
        }
    }
}

/// Clica no nome da propriedade (a coluna do nome não leva o foco ao editor).
fn clicar_no_nome(app: &mut TestApp, nome: &str) {
    let r = app.rect(&format!("props-{nome}"));
    app.click_at(Point::new(r.x0 + 30.0, r.center().y), 1);
}

#[test]
fn so_a_linha_escolhida_tem_editor() {
    let (mut app, _) = grade();
    assert!(existe(&app, "props-Host-valor"));
    assert!(!existe(&app, "props-Porta-valor"));
    assert!(linha(&app, "Host").2.editing);
    assert!(!linha(&app, "Porta").2.editing);
    clicar_no_nome(&mut app, "Porta");
    assert!(!existe(&app, "props-Host-valor"));
    assert!(existe(&app, "props-Porta-valor"));
    assert!(linha(&app, "Porta").2.editing);
}

#[test]
fn clicar_no_valor_de_outra_linha_escolhe_e_leva_o_foco_ao_editor() {
    let (mut app, _) = grade();
    let r = app.rect("props-Usuário");
    app.click_at(Point::new(r.x1 - 40.0, r.center().y), 1);
    assert!(linha(&app, "Usuário").2.selected);
    assert_eq!(app.focused().as_deref(), Some("props-Usuário-valor"));
}

#[test]
fn enter_confirma_o_texto_e_devolve_o_foco_a_linha() {
    let (mut app, c) = grade();
    app.type_into("props-Host-valor", "x");
    assert_eq!(c.a.host.get_untracked(), "exemplo.com", "ainda é rascunho");
    app.key(Key::Enter);
    assert_eq!(c.a.host.get_untracked(), "exemplo.comx");
    assert_eq!(app.focused().as_deref(), Some("props-Host"));
}

#[test]
fn tab_e_clicar_fora_confirmam() {
    let (mut app, c) = grade();
    app.type_into("props-Host-valor", "1");
    app.key(Key::Tab);
    assert_eq!(c.a.host.get_untracked(), "exemplo.com1");
    app.type_into("props-Host-valor", "2");
    app.click("depois");
    assert_eq!(c.a.host.get_untracked(), "exemplo.com12");
}

#[test]
fn esc_desfaz_o_rascunho_e_devolve_o_foco_a_linha() {
    let (mut app, c) = grade();
    app.type_into("props-Host-valor", "x");
    app.key(Key::Escape);
    assert_eq!(c.a.host.get_untracked(), "exemplo.com");
    assert_eq!(app.text("props-Host-valor"), "exemplo.com");
    assert_eq!(app.focused().as_deref(), Some("props-Host"));
}

#[test]
fn numero_confirma_com_limites() {
    let (mut app, c) = grade();
    clicar_no_nome(&mut app, "Porta");
    app.key(Key::F(2));
    assert_eq!(app.focused().as_deref(), Some("props-Porta-valor"));
    app.type_text("99999").key(Key::Enter);
    assert_eq!(c.a.porta.get_untracked(), 65535.0);
    assert_eq!(app.focused().as_deref(), Some("props-Porta"));
}

#[test]
fn enter_e_f2_entram_no_editor() {
    let (mut app, _) = grade();
    clicar_no_nome(&mut app, "Host");
    app.key(Key::Enter);
    assert_eq!(app.focused().as_deref(), Some("props-Host-valor"));
    app.key(Key::Escape).key(Key::F(2));
    assert_eq!(app.focused().as_deref(), Some("props-Host-valor"));
}

#[test]
fn digitar_na_linha_abre_o_editor_com_o_texto() {
    let (mut app, c) = grade();
    clicar_no_nome(&mut app, "Usuário");
    app.type_text("zé");
    assert_eq!(app.focused().as_deref(), Some("props-Usuário-valor"));
    assert_eq!(app.text("props-Usuário-valor"), "zé");
    app.key(Key::Enter);
    assert_eq!(c.a.usuario.get_untracked(), "zé");
}

#[test]
fn escolha_pela_lista_com_f4_e_alt_seta() {
    let (mut app, c) = grade();
    clicar_no_nome(&mut app, "Protocolo");
    app.key(Key::F(4));
    assert!(app.is_open("props-Protocolo-valor-lista"));
    app.click("props-Protocolo-valor-telnet");
    assert_eq!(c.a.protocolo.get_untracked(), "Telnet");
    assert!(linha(&app, "Protocolo").2.changed);
    assert!(!app.is_open("props-Protocolo-valor-lista"));
    clicar_no_nome(&mut app, "Protocolo");
    app.key_with(Key::Down, Modifiers::ALT);
    assert!(app.is_open("props-Protocolo-valor-lista"));
    app.key(Key::Escape);
    assert!(!app.is_open("props-Protocolo-valor-lista"));
    app.key(Key::Enter);
    assert!(app.is_open("props-Protocolo-valor-lista"));
}

#[test]
fn caixa_de_marcacao_fica_a_vista_e_o_espaco_alterna() {
    let (mut app, c) = grade();
    app.double_click("props-categoria-Terminal");
    assert!(
        existe(&app, "props-Bipe-valor"),
        "mesmo sem ser a escolhida"
    );
    app.click("props-Bipe-valor");
    assert!(!c.a.bipe.get_untracked());
    clicar_no_nome(&mut app, "Bipe");
    app.key(Key::Space);
    assert!(c.a.bipe.get_untracked());
    app.key(Key::Enter);
    assert!(!c.a.bipe.get_untracked());
}

#[test]
fn so_leitura_nao_tem_editor_mas_a_linha_se_escolhe() {
    let (mut app, _) = grade();
    app.double_click("props-categoria-Terminal");
    clicar_no_nome(&mut app, "Id do sistema");
    assert!(linha(&app, "Id do sistema").2.selected);
    assert!(!existe(&app, "props-Id do sistema-valor"));
    assert!(!linha(&app, "Id do sistema").2.editing);
    app.key(Key::Enter);
    assert_eq!(app.focused().as_deref(), Some("props-Id do sistema"));
}

#[test]
fn setas_pulam_a_desabilitada_e_esquerda_sobe_ao_cabecalho() {
    let (mut app, c) = grade();
    c.desabilitada.set(true);
    app.frame();
    clicar_no_nome(&mut app, "Chave privada");
    app.key(Key::Down);
    assert_eq!(app.focused().as_deref(), Some("props-categoria-Terminal"));
    app.key(Key::Up).key(Key::Up).key(Key::Up);
    assert_eq!(app.focused().as_deref(), Some("props-Usuário"));
    app.key(Key::Left);
    assert_eq!(
        app.focused().as_deref(),
        Some("props-categoria-Autenticação")
    );
    app.key(Key::Home);
    assert_eq!(app.focused().as_deref(), Some("props-categoria-Conexão"));
    app.key(Key::End);
    assert_eq!(app.focused().as_deref(), Some("props-categoria-Terminal"));
    assert!(!existe(&app, "props-Opcional-valor"));
}

#[test]
fn trocar_o_signal_ligado_mostra_o_valor_do_outro() {
    let (mut app, c) = grade();
    assert_eq!(app.text("props-Host-valor"), "exemplo.com");
    c.escolhida.set(1);
    app.frame();
    assert_eq!(linha(&app, "Host").1, "outro.com");
    assert_eq!(app.text("props-Host-valor"), "outro.com");
    app.type_into("props-Host-valor", "!").key(Key::Enter);
    assert_eq!(c.b.host.get_untracked(), "outro.com!");
    assert_eq!(c.a.host.get_untracked(), "exemplo.com");
}

#[test]
fn descricao_mostra_o_nome_e_o_texto_da_linha_escolhida() {
    let (mut app, _) = grade();
    assert_eq!(app.text("props-descricao-nome"), "Host");
    assert_eq!(
        app.text("props-descricao-texto"),
        "Nome ou endereço do servidor."
    );
    clicar_no_nome(&mut app, "Porta");
    assert_eq!(app.text("props-descricao-nome"), "Porta");
    assert_eq!(app.text("props-descricao-texto"), "");
    clicar_no_nome(&mut app, "Usuário");
    app.key(Key::Left);
    assert_eq!(app.text("props-descricao-nome"), "Autenticação");
    assert_eq!(app.text("props-descricao-texto"), "");
}

#[test]
fn arrastar_a_linha_das_colunas_muda_a_largura_do_nome() {
    let (mut app, _) = grade();
    let antes = app.rect("props-Host-valor").x0;
    let r = app.rect("props-Porta");
    let y = r.center().y;
    let x = antes - 1.0;
    app.drag(Point::new(x, y), Point::new(x + 40.0, y));
    assert_eq!(app.rect("props-Host-valor").x0, antes + 40.0);
    assert!(
        linha(&app, "Host").2.selected,
        "arrastar não escolhe outra linha"
    );
    assert!(!linha(&app, "Porta").2.selected);
    // O mínimo é de 40 px de cada lado.
    let x = app.rect("props-Host-valor").x0 - 1.0;
    app.drag(Point::new(x, y), Point::new(-500.0, y));
    assert_eq!(app.rect("props-Host-valor").x0 - app.rect("props").x0, 41.0);
}

#[test]
fn arrastar_o_divisor_muda_a_altura_da_descricao() {
    let (mut app, _) = grade();
    let antes = app.rect("props-descricao").height();
    let d = app.rect("props-divisor");
    assert_eq!(d.height(), 8.0);
    app.drag(d.center(), Point::new(d.center().x, d.center().y - 30.0));
    assert_eq!(app.rect("props-descricao").height(), antes + 30.0);
    let d = app.rect("props-divisor");
    app.drag(d.center(), Point::new(d.center().x, d.center().y + 500.0));
    assert_eq!(app.rect("props-descricao").height(), 32.0);
}

// ---------------------------------------------------------------------------------------
// .file
// ---------------------------------------------------------------------------------------

#[test]
fn o_botao_procurar_so_existe_na_linha_escolhida() {
    let (mut app, _) = grade();
    assert!(!existe(&app, "props-Chave privada-procurar"));
    clicar_no_nome(&mut app, "Chave privada");
    assert!(existe(&app, "props-Chave privada-procurar"));
    assert!(existe(&app, "props-Chave privada-valor"));
    clicar_no_nome(&mut app, "Host");
    assert!(!existe(&app, "props-Chave privada-procurar"));
}

#[test]
fn procurar_pede_o_arquivo_com_titulo_filtro_e_pasta() {
    let (mut app, c) = grade();
    c.a.chave.set("/tmp/antiga.pem".to_owned());
    clicar_no_nome(&mut app, "Chave privada");
    app.click("props-Chave privada-procurar");
    let pedidos = app.file_dialogs();
    assert_eq!(pedidos.len(), 1);
    let pedido = &pedidos[0];
    assert_eq!(pedido.title, "Chave privada");
    assert_eq!(pedido.filters.len(), 1);
    assert_eq!(pedido.filters[0].name, "Chaves");
    assert_eq!(pedido.filters[0].extensions, ["pem", "key"]);
    assert_eq!(
        pedido.directory.as_deref(),
        Some(std::path::Path::new("/tmp"))
    );
}

#[test]
fn resposta_do_seletor_grava_no_signal() {
    let (mut app, c) = grade();
    clicar_no_nome(&mut app, "Chave privada");
    app.click("props-Chave privada-procurar");
    app.file_dialog_answer(Some("/tmp/nova.pem"));
    assert_eq!(c.a.chave.get_untracked(), "/tmp/nova.pem");
    assert_eq!(linha(&app, "Chave privada").1, "/tmp/nova.pem");
}

#[test]
fn seletor_cancelado_nao_muda_o_valor() {
    let (mut app, c) = grade();
    c.a.chave.set("/tmp/antiga.pem".to_owned());
    clicar_no_nome(&mut app, "Chave privada");
    app.click("props-Chave privada-procurar");
    app.file_dialog_answer(None::<&str>);
    assert_eq!(c.a.chave.get_untracked(), "/tmp/antiga.pem");
}

#[test]
fn sair_do_editor_para_o_botao_nao_fecha_o_editor() {
    let (mut app, c) = grade();
    clicar_no_nome(&mut app, "Chave privada");
    app.type_text("/tmp/digitada.pem");
    app.click("props-Chave privada-procurar");
    assert_eq!(c.a.chave.get_untracked(), "/tmp/digitada.pem");
    assert!(existe(&app, "props-Chave privada-procurar"));
    assert!(existe(&app, "props-Chave privada-valor"));
}

// ---------------------------------------------------------------------------------------
// Ferramentas
// ---------------------------------------------------------------------------------------

#[test]
fn a_ferramenta_mostra_propriedades_categorias_e_ordem() {
    let (mut app, _) = grade();
    clicar_no_nome(&mut app, "Porta");
    let no = app.tree();
    let grade = no.find("props").expect("a PropertyGrid");
    assert_eq!(grade.kind, "property_grid");
    assert_eq!(grade.alphabetical, Some(false));
    let categorias = grade.categories.as_ref().unwrap();
    let nomes: Vec<_> = categorias.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(nomes, ["Conexão", "Autenticação", "Terminal"]);
    assert_eq!(
        categorias.iter().map(|c| c.expanded).collect::<Vec<_>>(),
        [true, true, false]
    );
    let propriedades = grade.properties.as_ref().unwrap();
    let porta = propriedades.iter().find(|p| p.name == "Porta").unwrap();
    assert_eq!(porta.category.as_deref(), Some("Conexão"));
    assert_eq!(porta.value, "2222");
    assert!(porta.changed && porta.selected && !porta.read_only && porta.enabled);
    let senha = propriedades.iter().find(|p| p.name == "Senha").unwrap();
    assert_eq!(senha.value, "•••••••", "a senha sai mascarada");
    let bipe = propriedades.iter().find(|p| p.name == "Bipe").unwrap();
    assert_eq!(bipe.value, "true");
    assert!(!bipe.changed, "o padrão é true");
    let id = propriedades
        .iter()
        .find(|p| p.name == "Id do sistema")
        .unwrap();
    assert!(id.read_only);
    assert_eq!(app.properties("props").len(), propriedades.len());
}

#[test]
fn a_ferramenta_mostra_linhas_e_cabecalhos() {
    let (mut app, _) = grade();
    clicar_no_nome(&mut app, "Porta");
    let no = app.tree();
    let porta = no.find("props-Porta").expect("a linha");
    assert_eq!(porta.kind, "property_grid_row");
    assert_eq!(porta.text.as_deref(), Some("2222"));
    assert_eq!(porta.selected, Some(true));
    assert!(porta.changed);
    let host = no.find("props-Host").expect("a linha");
    assert!(host.changed, "exemplo.com difere de \"\"");
    let protocolo = no.find("props-Protocolo").expect("a linha");
    assert!(!protocolo.changed);
    let categoria = no.find("props-categoria-Conexão").expect("o cabeçalho");
    assert_eq!(categoria.kind, "property_grid_category");
    assert_eq!(categoria.open, Some(true));
    let fechada = no.find("props-categoria-Terminal").expect("o cabeçalho");
    assert_eq!(fechada.open, Some(false));
    assert_eq!(
        no.find("props-divisor").unwrap().kind,
        "property_grid_sizer"
    );
}

#[test]
fn propriedades_do_test_app_trazem_o_estado_da_grade() {
    let (mut app, c) = grade();
    clicar_no_nome(&mut app, "Usuário");
    c.a.usuario.set("ana2".to_owned());
    app.frame();
    let propriedades = app.properties("props");
    let usuario = propriedades.iter().find(|p| p.name == "Usuário").unwrap();
    assert_eq!(usuario.value, "ana2");
    assert!(usuario.selected);
    assert_eq!(usuario.category.as_deref(), Some("Autenticação"));
}

// ---------------------------------------------------------------------------------------
// Snapshots
// ---------------------------------------------------------------------------------------

fn nome_do_tema(tema: Theme) -> &'static str {
    match tema {
        Theme::Light => "claro",
        Theme::Dark => "escuro",
    }
}

fn grade_no_tema(tema: Theme, densidade: Option<Density>) -> Grade {
    let (app, c) = grade_com(densidade);
    (app.theme(tema), c)
}

#[test]
fn snapshots() {
    for tema in [Theme::Light, Theme::Dark] {
        let nome = |cena: &str| format!("m22-{cena}-{}", nome_do_tema(tema));

        // O Host em edição, com o painel de descrição; a Porta e o Host em negrito (mudaram
        // do padrão), a senha mascarada e a categoria Terminal fechada.
        let (mut app, _) = grade_no_tema(tema, None);
        clicar_no_nome(&mut app, "Host");
        app.assert_snapshot(&nome("grade"));

        // A lista da `.choice` aberta.
        let (mut app, _) = grade_no_tema(tema, None);
        clicar_no_nome(&mut app, "Protocolo");
        app.key(Key::F(4));
        assert!(app.is_open("props-Protocolo-valor-lista"));
        app.assert_snapshot(&nome("escolha-aberta"));

        // A ordem alfabética, com a Porta escolhida pelo teclado (anel de foco).
        let (mut app, _) = grade_no_tema(tema, None);
        app.click("props-alfabetica");
        clicar_no_nome(&mut app, "Porta");
        app.key(Key::Down).key(Key::Up);
        assert_eq!(app.focused().as_deref(), Some("props-Porta"));
        app.assert_snapshot(&nome("alfabetica"));

        // A densidade padrão, com as linhas de 44 px.
        let (mut app, _) = grade_no_tema(tema, Some(Density::Standard));
        clicar_no_nome(&mut app, "Host");
        app.assert_snapshot(&nome("grade-padrao"));
    }
}
