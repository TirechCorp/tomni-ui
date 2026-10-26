//! O `Rclone` contra o rcd falso (`comum/rcd_falso.rs`), sem o binário do rclone.
#![cfg(all(unix, feature = "rclone"))]

mod comum;

use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tomni_ui_files::rclone::config::ConfigStep;
use tomni_ui_files::rclone::{Rcd, Rclone, RcloneError};
use tomni_ui_files::{
    Conflict, Endpoint, EntryKind, Fs, Location, Opener, TransferState, Transfers,
};

use comum::rcd_falso::{Opcoes, RcdFalso};
use comum::{Pasta, dados};

const MIB: usize = 1 << 20;

fn ler_tudo(fs: &mut dyn Fs, caminho: &str, offset: u64) -> Vec<u8> {
    let mut arquivo = fs.open_read(caminho, offset).unwrap();
    let mut saida = Vec::new();
    arquivo.read_to_end(&mut saida).unwrap();
    arquivo.finish().unwrap();
    saida
}

fn gravar(fs: &mut dyn Fs, caminho: &str, conteudo: &[u8]) {
    let mut arquivo = fs.open_write(caminho, 0).unwrap();
    for pedaco in conteudo.chunks(100_000) {
        arquivo.write_all(pedaco).unwrap();
    }
    arquivo.finish().unwrap();
}

fn nomes(fs: &mut dyn Fs, caminho: &str) -> Vec<String> {
    let mut nomes: Vec<String> = fs
        .list(caminho)
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    nomes.sort();
    nomes
}

// ----- o Rcd -----

#[test]
fn attach_conversa_e_diz_a_versao() {
    let servidor = RcdFalso::iniciar();
    let rcd = servidor.rcd();
    assert_eq!(rcd.version().unwrap(), (1, 60, 1));
    assert_eq!(rcd.check_version().unwrap(), (1, 60, 1));
}

#[test]
fn a_versao_antiga_e_recusada() {
    let servidor = RcdFalso::com(Opcoes {
        versao: [1, 59, 2],
        ..Opcoes::default()
    });
    match servidor.rcd().check_version() {
        Err(RcloneError::TooOld(versao)) => assert_eq!(versao, (1, 59, 2)),
        outro => panic!("esperava TooOld, veio {outro:?}"),
    }
}

#[test]
fn a_senha_errada_diz_que_a_autenticacao_falhou() {
    let servidor = RcdFalso::iniciar();
    let erro = servidor.rcd_com_senha_errada().version().unwrap_err();
    assert!(
        matches!(erro, RcloneError::Api { status: 401, .. }),
        "{erro:?}"
    );
    assert!(erro.to_string().contains("autenticação"), "{erro}");
}

#[test]
fn o_erro_da_api_vira_mensagem_em_portugues_com_o_texto_entre_aspas() {
    let servidor = RcdFalso::iniciar();
    let erro = servidor
        .rcd()
        .call("metodo/que-nao-existe", serde_json::json!({}))
        .unwrap_err();
    let texto = erro.to_string();
    assert!(texto.starts_with("O rclone recusou: \""), "{texto}");
    assert!(texto.contains("metodo/que-nao-existe"), "{texto}");
}

// ----- o Fs -----

#[test]
fn lista_e_faz_stat() {
    let servidor = RcdFalso::iniciar();
    servidor.semear("m:", "docs/a.txt", b"abc");
    servidor.semear("m:", "docs/sub/b.txt", b"12345");
    servidor.semear("m:", "raiz.txt", b"x");
    let mut fs = Rclone::new(servidor.rcd(), "m:");

    assert_eq!(fs.home().unwrap(), "/");
    assert_eq!(nomes(&mut fs, "/"), ["docs", "raiz.txt"]);
    let lista = fs.list("/docs").unwrap();
    let a = lista.iter().find(|e| e.name == "a.txt").unwrap();
    assert_eq!(a.kind, EntryKind::File);
    assert_eq!(a.size, 3);
    assert!(a.modified.is_some());
    assert_eq!(
        lista.iter().find(|e| e.name == "sub").unwrap().kind,
        EntryKind::Dir
    );

    assert_eq!(fs.stat("/docs/a.txt").unwrap().size, 3);
    assert_eq!(fs.stat("/docs").unwrap().kind, EntryKind::Dir);
    let erro = fs.stat("/nao-existe").unwrap_err();
    assert_eq!(erro.kind(), io::ErrorKind::NotFound);
}

#[test]
fn le_a_partir_do_meio_e_alem_do_fim() {
    let servidor = RcdFalso::iniciar();
    let conteudo = dados(10_000);
    servidor.semear("m:", "grande.bin", &conteudo);
    let mut fs = Rclone::new(servidor.rcd(), "m:");

    assert_eq!(ler_tudo(&mut fs, "/grande.bin", 0), conteudo);
    assert_eq!(ler_tudo(&mut fs, "/grande.bin", 4_000), conteudo[4_000..]);
    assert!(ler_tudo(&mut fs, "/grande.bin", 10_000).is_empty());
    let Err(erro) = fs.open_read("/falta.bin", 0) else {
        panic!("abriu um arquivo que não existe");
    };
    assert_eq!(erro.kind(), io::ErrorKind::NotFound);
}

#[test]
fn envia_tres_mib_e_le_igual() {
    let servidor = RcdFalso::iniciar();
    let conteudo = dados(3 * MIB);
    let mut fs = Rclone::new(servidor.rcd(), "m:");
    fs.mkdir("/pasta").unwrap();

    gravar(&mut fs, "/pasta/tres.bin", &conteudo);

    assert_eq!(servidor.arquivo("m:", "pasta/tres.bin").unwrap(), conteudo);
    assert_eq!(ler_tudo(&mut fs, "/pasta/tres.bin", 0), conteudo);
}

#[test]
fn soltar_o_envio_sem_terminar_nao_deixa_o_arquivo() {
    let servidor = RcdFalso::iniciar();
    let mut fs = Rclone::new(servidor.rcd(), "m:");
    {
        let mut envio = fs.open_write("/pela-metade.bin", 0).unwrap();
        envio.write_all(&dados(1000)).unwrap();
    }
    std::thread::sleep(Duration::from_millis(200));
    assert!(servidor.arquivo("m:", "pela-metade.bin").is_none());
}

#[test]
fn o_envio_nao_retoma() {
    let servidor = RcdFalso::iniciar();
    let mut fs = Rclone::new(servidor.rcd(), "m:");
    let Err(erro) = fs.open_write("/x.bin", 10) else {
        panic!("aceitou retomar");
    };
    assert_eq!(erro.kind(), io::ErrorKind::Unsupported);
}

#[test]
fn cria_pasta_renomeia_arquivo_e_pasta_e_apaga() {
    let servidor = RcdFalso::iniciar();
    servidor.semear("m:", "dir/x.txt", b"x");
    servidor.semear("m:", "dir/fundo/y.txt", b"y");
    servidor.semear("m:", "solto.txt", b"s");
    let mut fs = Rclone::new(servidor.rcd(), "m:");

    fs.mkdir("/nova").unwrap();
    assert_eq!(fs.stat("/nova").unwrap().kind, EntryKind::Dir);

    fs.rename("/solto.txt", "/nova/movido.txt", false).unwrap();
    assert_eq!(servidor.arquivo("m:", "nova/movido.txt").unwrap(), b"s");
    assert!(servidor.arquivo("m:", "solto.txt").is_none());

    fs.rename("/dir", "/outra", false).unwrap();
    assert_eq!(servidor.arquivo("m:", "outra/x.txt").unwrap(), b"x");
    assert_eq!(servidor.arquivo("m:", "outra/fundo/y.txt").unwrap(), b"y");
    assert!(
        servidor
            .caminhos("m:")
            .iter()
            .all(|c| !c.starts_with("dir")),
        "{:?}",
        servidor.caminhos("m:")
    );

    fs.remove_file("/nova/movido.txt").unwrap();
    assert!(servidor.arquivo("m:", "nova/movido.txt").is_none());
    fs.remove_dir("/nova").unwrap();
    assert!(fs.stat("/nova").is_err());
}

#[test]
fn o_que_o_rclone_nao_faz_diz_que_nao_faz() {
    let servidor = RcdFalso::iniciar();
    servidor.semear("m:", "a.txt", b"a");
    let mut fs = Rclone::new(servidor.rcd(), "m:");
    for erro in [
        fs.set_permissions("/a.txt", 0o644).unwrap_err(),
        fs.set_modified("/a.txt", std::time::UNIX_EPOCH)
            .unwrap_err(),
        fs.symlink("/a.txt", "/b").unwrap_err(),
        fs.read_link("/a.txt").map(drop).unwrap_err(),
    ] {
        assert_eq!(erro.kind(), io::ErrorKind::Unsupported);
    }
}

#[test]
fn o_remoto_conta_o_espaco_e_o_link_publico() {
    let servidor = RcdFalso::iniciar();
    servidor.semear("m:", "a.txt", b"abcd");
    let fs = Rclone::new(servidor.rcd(), "m:");

    let about = fs.about().unwrap().expect("diz o espaço");
    assert_eq!(about.used, Some(4));
    assert_eq!(about.total, Some(10_000_000));
    assert!(fs.features().unwrap().public_link);
    assert!(
        fs.public_link("/a.txt", None)
            .unwrap()
            .starts_with("https://")
    );

    let sem = RcdFalso::com(Opcoes {
        sem_about: true,
        sem_link_publico: true,
        ..Opcoes::default()
    });
    let fs = Rclone::new(sem.rcd(), "m:");
    assert!(fs.about().unwrap().is_none());
    assert!(!fs.features().unwrap().public_link);
}

// ----- a cópia delegada -----

#[test]
fn a_copia_delegada_sobe_e_desce_com_progresso_crescente() {
    let servidor = RcdFalso::iniciar();
    let local = Pasta::nova("rclone-delegada");
    let conteudo = dados(300_000);
    local.escrever("sobe.bin", &conteudo);
    let mut fs = Rclone::new(servidor.rcd(), "m:");
    assert!(fs.delegation().is_some());

    let vistos = std::sync::Mutex::new(Vec::new());
    let sem_cancelar = AtomicBool::new(false);
    let sobe = fs.delegated_copy(
        &Location::Local(local.caminho().join("sobe.bin")),
        &Location::Here("/nuvem.bin".to_owned()),
        &|feito, _| vistos.lock().unwrap().push(feito),
        &sem_cancelar,
    );
    sobe.expect("o rclone sabe copiar").unwrap();
    assert_eq!(servidor.arquivo("m:", "nuvem.bin").unwrap(), conteudo);
    let vistos = vistos.into_inner().unwrap();
    assert!(vistos.windows(2).all(|j| j[0] <= j[1]), "{vistos:?}");
    assert_eq!(*vistos.last().unwrap(), 300_000);

    let desce = fs.delegated_copy(
        &Location::Here("/nuvem.bin".to_owned()),
        &Location::Local(local.caminho().join("desce.bin")),
        &|_, _| {},
        &sem_cancelar,
    );
    desce.expect("o rclone sabe copiar").unwrap();
    assert_eq!(
        std::fs::read(local.caminho().join("desce.bin")).unwrap(),
        conteudo
    );
}

#[test]
fn a_copia_delegada_entre_dois_remotos_do_mesmo_rcd() {
    let servidor = RcdFalso::iniciar();
    servidor.semear("a:", "x.txt", b"conteudo");
    let rcd = servidor.rcd();
    let mut origem = Rclone::new(Arc::clone(&rcd), "a:");
    let destino = Rclone::new(Arc::clone(&rcd), "b:pasta");

    let par = destino.delegation().unwrap();
    assert_eq!(par.group, origem.delegation().unwrap().group);
    origem
        .delegated_copy(
            &Location::Here("/x.txt".to_owned()),
            &Location::Peer(par, "/y.txt".to_owned()),
            &|_, _| {},
            &AtomicBool::new(false),
        )
        .unwrap()
        .unwrap();
    assert_eq!(servidor.arquivo("b:", "pasta/y.txt").unwrap(), b"conteudo");
}

#[test]
fn outro_rcd_nao_e_delegavel() {
    let um = RcdFalso::iniciar();
    let outro = RcdFalso::iniciar();
    let mut origem = Rclone::new(um.rcd(), "a:");
    let destino = Rclone::new(outro.rcd(), "b:");
    let par = destino.delegation().unwrap();
    assert!(
        origem
            .delegated_copy(
                &Location::Here("/x".to_owned()),
                &Location::Peer(par, "/y".to_owned()),
                &|_, _| {},
                &AtomicBool::new(false),
            )
            .is_none()
    );
}

#[test]
fn cancelar_a_copia_delegada_chama_job_stop() {
    let servidor = RcdFalso::com(Opcoes {
        copia_trava: true,
        ..Opcoes::default()
    });
    servidor.semear("m:", "grande.bin", &dados(1000));
    let local = Pasta::nova("rclone-cancela");
    let mut fs = Rclone::new(servidor.rcd(), "m:");
    let cancelar = Arc::new(AtomicBool::new(false));
    {
        let cancelar = Arc::clone(&cancelar);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(500));
            cancelar.store(true, Ordering::SeqCst);
        });
    }

    let resultado = fs
        .delegated_copy(
            &Location::Here("/grande.bin".to_owned()),
            &Location::Local(local.caminho().join("x.bin")),
            &|_, _| {},
            &cancelar,
        )
        .expect("o rclone sabe copiar");

    assert_eq!(resultado.unwrap_err().kind(), io::ErrorKind::Interrupted);
    assert_eq!(servidor.pedidos_de("job/stop").len(), 1);
    assert!(!local.caminho().join("x.bin").exists());
}

#[test]
fn erro_da_copia_delegada_vem_em_portugues() {
    let servidor = RcdFalso::iniciar();
    let local = Pasta::nova("rclone-erro-copia");
    let mut fs = Rclone::new(servidor.rcd(), "m:");
    let erro = fs
        .delegated_copy(
            &Location::Here("/falta.bin".to_owned()),
            &Location::Local(local.caminho().join("x.bin")),
            &|_, _| {},
            &AtomicBool::new(false),
        )
        .unwrap()
        .unwrap_err();
    assert_eq!(erro.kind(), io::ErrorKind::NotFound);
}

// ----- a fila usando a delegação -----

fn opener(rcd: &Arc<Rcd>, spec: &'static str) -> Opener {
    let rcd = Arc::clone(rcd);
    Arc::new(move || Ok(Box::new(Rclone::new(Arc::clone(&rcd), spec)) as Box<dyn Fs>))
}

#[test]
fn a_fila_delega_ao_rclone_o_envio_e_o_download() {
    let servidor = RcdFalso::iniciar();
    let rcd = servidor.rcd();
    let local = Pasta::nova("rclone-fila");
    let conteudo = dados(200_000);
    local.escrever("sobe.bin", &conteudo);
    let nuvem = opener(&rcd, "m:");
    let fila = Transfers::new(2);

    fila.add(
        (Endpoint::Local, local.texto("sobe.bin")),
        (Endpoint::Remote(nuvem.clone()), "/sobe.bin".to_owned()),
        Conflict::Overwrite,
    );
    assert!(fila.wait_idle(Duration::from_secs(30)));
    assert_eq!(servidor.arquivo("m:", "sobe.bin").unwrap(), conteudo);
    assert_eq!(servidor.pedidos_de("operations/copyfile").len(), 1);
    assert!(servidor.pedidos_de("operations/uploadfile").is_empty());

    fila.add(
        (Endpoint::Remote(nuvem), "/sobe.bin".to_owned()),
        (Endpoint::Local, local.texto("desce.bin")),
        Conflict::Overwrite,
    );
    assert!(fila.wait_idle(Duration::from_secs(30)));
    assert_eq!(
        std::fs::read(local.caminho().join("desce.bin")).unwrap(),
        conteudo
    );
    assert!(
        fila.items()
            .get_untracked()
            .iter()
            .all(|item| item.state == TransferState::Done)
    );
}

#[test]
fn a_fila_copia_entre_dois_remotos_do_mesmo_rcd_sem_passar_pelo_app() {
    let servidor = RcdFalso::iniciar();
    servidor.semear("a:", "x.bin", &dados(5000));
    let rcd = servidor.rcd();
    let fila = Transfers::new(1);

    fila.add(
        (Endpoint::Remote(opener(&rcd, "a:")), "/x.bin".to_owned()),
        (Endpoint::Remote(opener(&rcd, "b:")), "/y.bin".to_owned()),
        Conflict::Overwrite,
    );
    assert!(fila.wait_idle(Duration::from_secs(30)));

    assert_eq!(servidor.arquivo("b:", "y.bin").unwrap(), dados(5000));
    assert!(servidor.pedidos_de("operations/uploadfile").is_empty());
}

#[test]
fn a_fila_cancela_a_copia_delegada() {
    let servidor = RcdFalso::com(Opcoes {
        copia_trava: true,
        ..Opcoes::default()
    });
    servidor.semear("m:", "x.bin", &dados(5000));
    let rcd = servidor.rcd();
    let local = Pasta::nova("rclone-fila-cancela");
    let fila = Transfers::new(1);

    let id = fila.add(
        (Endpoint::Remote(opener(&rcd, "m:")), "/x.bin".to_owned()),
        (Endpoint::Local, local.texto("x.bin")),
        Conflict::Overwrite,
    );
    std::thread::sleep(Duration::from_millis(500));
    fila.cancel(id);
    assert!(fila.wait_idle(Duration::from_secs(30)));

    assert!(!servidor.pedidos_de("job/stop").is_empty());
    assert!(
        fila.items()
            .get_untracked()
            .iter()
            .all(|item| item.state == TransferState::Cancelled)
    );
}

// ----- a configuração -----

#[test]
fn lista_provedores_com_as_opcoes() {
    let servidor = RcdFalso::iniciar();
    let provedores = servidor.rcd().config_providers().unwrap();
    let falso = provedores.iter().find(|p| p.name == "falso").unwrap();
    let opcao = |nome: &str| falso.options.iter().find(|o| o.name == nome).unwrap();

    assert!(opcao("url").required);
    assert_eq!(opcao("usar_tls").kind, "bool");
    assert!(opcao("modo").exclusive);
    assert_eq!(opcao("modo").examples.len(), 2);
    assert_eq!(opcao("modo").default, "a");
    assert!(opcao("senha").is_password);
    assert!(opcao("avancada").advanced);
    assert!(opcao("escondida").hidden());
    assert!(opcao("so_aws").applies_to("AWS"));
    assert!(!opcao("so_aws").applies_to("Outro"));
    assert!(opcao("menos_aws").applies_to("Outro"));
    assert!(!opcao("menos_aws").applies_to("AWS"));
}

#[test]
fn criar_com_pergunta_responder_e_achar_o_remoto() {
    let servidor = RcdFalso::iniciar();
    let rcd = servidor.rcd();
    let parametros = vec![
        ("url".to_owned(), "https://exemplo.test".to_owned()),
        ("senha".to_owned(), "segredo".to_owned()),
    ];

    let ConfigStep::Question { state, option, .. } =
        rcd.config_create("meu", "falso", &parametros).unwrap()
    else {
        panic!("devia perguntar");
    };
    assert_eq!(state, "passo1");
    assert_eq!(option.name, "pergunta1");
    let ConfigStep::Question { state, option, .. } =
        rcd.config_answer("meu", &state, "resposta um").unwrap()
    else {
        panic!("devia perguntar de novo");
    };
    assert_eq!(state, "passo2");
    assert_eq!(option.name, "pergunta2");
    assert!(matches!(
        rcd.config_answer("meu", &state, "resposta dois").unwrap(),
        ConfigStep::Done
    ));

    let remotos = rcd.config_remotes().unwrap();
    assert!(remotos.iter().any(|r| r.name == "meu" && r.kind == "falso"));
    // A senha foi pedida ofuscada ao rclone; nunca em claro nos valores guardados.
    let pedidos = servidor.pedidos_de("config/create");
    assert!(pedidos[0].contains("\"obscure\":true"), "{}", pedidos[0]);
    let valores = rcd.config_get("meu").unwrap();
    assert_eq!(valores["url"], "https://exemplo.test");
    assert_ne!(valores["senha"], "segredo");
}

#[test]
fn atualizar_e_apagar_um_remoto() {
    let servidor = RcdFalso::iniciar();
    let rcd = servidor.rcd();
    servidor.configurar("meu", "falso");

    rcd.config_update(
        "meu",
        &[("url".to_owned(), "https://outro.test".to_owned())],
    )
    .unwrap();
    assert_eq!(rcd.config_get("meu").unwrap()["url"], "https://outro.test");
    let pedido = &servidor.pedidos_de("config/update")[0];
    assert!(pedido.contains("\"obscure\":true"), "{pedido}");

    rcd.config_delete("meu").unwrap();
    assert!(rcd.config_remotes().unwrap().is_empty());
    assert!(rcd.config_get("meu").is_err());
}

#[test]
fn o_login_pergunta_se_e_local_e_a_resposta_false_pede_o_token() {
    let servidor = RcdFalso::iniciar();
    let rcd = servidor.rcd();

    let ConfigStep::Question { state, option, .. } =
        rcd.config_create("drive", "nuvem", &[]).unwrap()
    else {
        panic!("devia perguntar");
    };
    assert_eq!(option.name, "config_is_local");
    assert_eq!(option.kind, "bool");
    let ConfigStep::Question { option, .. } = rcd.config_answer("drive", &state, "false").unwrap()
    else {
        panic!("devia pedir o token");
    };
    assert_eq!(option.name, "config_token");

    // A resposta `true` só volta depois do login.
    let _ = rcd.config_delete("drive");
    let ConfigStep::Question { state, .. } = rcd.config_create("drive", "nuvem", &[]).unwrap()
    else {
        panic!("devia perguntar");
    };
    assert!(matches!(
        rcd.config_answer("drive", &state, "true").unwrap(),
        ConfigStep::Done
    ));
}
