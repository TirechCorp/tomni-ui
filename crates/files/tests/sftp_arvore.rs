//! Enviar uma pasta pelo SFTP com vários arquivos e várias escritas em voo (`Fs::upload_tree`),
//! e o uso dele na fila: uma pasta nova vira um item só.
#![cfg(all(unix, feature = "sftp"))]

mod comum;

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use tomni_ui_files::sftp::Sftp;
use tomni_ui_files::{
    ARQUIVOS_EM_VOO, Conflict, Endpoint, Fs, MemFs, Opener, TransferState, Transfers, TreeOptions,
};

use comum::servidor::Servidor;
use comum::{Pasta, dados};

const ESPERA: Duration = Duration::from_secs(60);

fn conectar(servidor: &Servidor) -> Sftp {
    Sftp::connect(&servidor.builder()).expect("conectar ao servidor de teste")
}

fn opener(servidor: &Servidor) -> Opener {
    let builder = servidor.builder();
    Arc::new(move || Ok(Box::new(Sftp::connect(&builder)?) as Box<dyn Fs>))
}

/// Uma pasta com `quantos` arquivos pequenos em 3 níveis, com nomes com espaço e acento, uma
/// subpasta vazia e um arquivo de 3 MiB. Devolve o total de bytes.
fn montar(pasta: &Pasta, raiz: &str, quantos: usize) -> u64 {
    let mut total = 0;
    for i in 0..quantos {
        let nivel = match i % 3 {
            0 => format!("{raiz}/a"),
            1 => format!("{raiz}/a/b c"),
            _ => format!("{raiz}/a/b c/ção"),
        };
        let conteudo = dados(10 + i % 200);
        total += conteudo.len() as u64;
        pasta.escrever(&format!("{nivel}/arquivo {i:04}.txt"), &conteudo);
    }
    let grande = dados(3 * 1024 * 1024);
    total += grande.len() as u64;
    pasta.escrever(&format!("{raiz}/a/grande.bin"), &grande);
    fs::create_dir_all(pasta.caminho().join(raiz).join("vazia")).unwrap();
    total
}

/// Tudo o que há em `raiz`, por caminho relativo: o conteúdo, ou `None` numa pasta.
fn arvore(raiz: &Path) -> BTreeMap<String, Option<Vec<u8>>> {
    fn andar(raiz: &Path, pasta: &Path, saida: &mut BTreeMap<String, Option<Vec<u8>>>) {
        for item in fs::read_dir(pasta).unwrap() {
            let caminho = item.unwrap().path();
            let relativo = caminho
                .strip_prefix(raiz)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            if caminho.is_dir() {
                saida.insert(relativo, None);
                andar(raiz, &caminho, saida);
            } else {
                saida.insert(relativo, Some(fs::read(&caminho).unwrap()));
            }
        }
    }
    let mut saida = BTreeMap::new();
    andar(raiz, raiz, &mut saida);
    saida
}

fn nunca() -> AtomicBool {
    AtomicBool::new(false)
}

#[test]
fn envia_300_arquivos_em_3_niveis_e_o_progresso_termina_no_total() {
    let pasta = Pasta::nova("arvore-300");
    let total = montar(&pasta, "origem", 300);
    let servidor = Servidor::novo(pasta.caminho());
    let mut sftp = conectar(&servidor);

    let chamadas = RefCell::new(Vec::new());
    let resultado = sftp.upload_tree(
        &pasta.caminho().join("origem"),
        "/home/destino",
        &TreeOptions::default(),
        &|feito, tudo| chamadas.borrow_mut().push((feito, tudo)),
        &nunca(),
    );
    resultado.expect("o SFTP sabe").unwrap();

    assert_eq!(
        arvore(&pasta.caminho().join("origem")),
        arvore(&servidor.real("/home/destino"))
    );
    let chamadas = chamadas.borrow();
    assert_eq!(chamadas.first(), Some(&(0, total)));
    assert_eq!(chamadas.last(), Some(&(total, total)));
    assert!(chamadas.windows(2).all(|par| par[0].0 <= par[1].0));
    assert!(chamadas.iter().all(|&(_, tudo)| tudo == total));
}

#[test]
fn varios_arquivos_ficam_abertos_ao_mesmo_tempo_ate_o_pedido() {
    let pasta = Pasta::nova("arvore-voo");
    montar(&pasta, "origem", 120);
    let servidor = Servidor::novo(pasta.caminho());
    let mut sftp = conectar(&servidor);

    sftp.upload_tree(
        &pasta.caminho().join("origem"),
        "/home/destino",
        &TreeOptions::default(),
        &|_, _| {},
        &nunca(),
    )
    .unwrap()
    .unwrap();
    let maximo = servidor.maximo_abertos();
    assert!(maximo > 1, "só um por vez: {maximo}");
    assert!(maximo <= ARQUIVOS_EM_VOO, "passou do pedido: {maximo}");

    let opcoes = TreeOptions {
        files_in_flight: 4,
        ..TreeOptions::default()
    };
    let pasta_4 = Pasta::nova("arvore-voo-4");
    let servidor = Servidor::novo(pasta_4.caminho());
    let mut sftp = conectar(&servidor);
    sftp.upload_tree(
        &pasta.caminho().join("origem"),
        "/home/destino",
        &opcoes,
        &|_, _| {},
        &nunca(),
    )
    .unwrap()
    .unwrap();
    let maximo = servidor.maximo_abertos();
    assert!((2..=4).contains(&maximo), "esperava de 2 a 4: {maximo}");
}

#[test]
fn servidor_que_recusa_o_quinto_arquivo_aberto_faz_o_envio_usar_menos_e_chegar_tudo() {
    let pasta = Pasta::nova("arvore-limite");
    montar(&pasta, "origem", 100);
    let servidor = Servidor::novo(pasta.caminho());
    servidor.limitar_abertos(4);
    let mut sftp = conectar(&servidor);

    sftp.upload_tree(
        &pasta.caminho().join("origem"),
        "/home/destino",
        &TreeOptions::default(),
        &|_, _| {},
        &nunca(),
    )
    .unwrap()
    .unwrap();

    assert!(servidor.recusados() > 0, "o limite nunca foi testado");
    assert!(servidor.maximo_abertos() <= 4);
    assert_eq!(
        arvore(&pasta.caminho().join("origem")),
        arvore(&servidor.real("/home/destino"))
    );
}

#[test]
fn cancelar_para_de_escrever_e_devolve_interrupted() {
    let pasta = Pasta::nova("arvore-cancela");
    for i in 0..4 {
        pasta.escrever(&format!("origem/grande {i}.bin"), &dados(24 * 1024 * 1024));
    }
    let servidor = Servidor::novo(pasta.caminho());
    let mut sftp = conectar(&servidor);
    let cancelar = nunca();
    let visto = Cell::new(false);

    let erro = sftp
        .upload_tree(
            &pasta.caminho().join("origem"),
            "/home/destino",
            &TreeOptions::default(),
            &|feito, _| {
                if feito > 0 {
                    visto.set(true);
                    cancelar.store(true, Ordering::SeqCst);
                }
            },
            &cancelar,
        )
        .unwrap()
        .unwrap_err();

    assert!(visto.get(), "o envio acabou antes do primeiro progresso");
    assert_eq!(erro.kind(), io::ErrorKind::Interrupted, "{erro}");
    let tamanho = |servidor: &Servidor| -> u64 {
        arvore(&servidor.real("/home/destino"))
            .values()
            .flatten()
            .map(|dados| dados.len() as u64)
            .sum()
    };
    let depois = tamanho(&servidor);
    assert!(depois < 4 * 24 * 1024 * 1024, "mandou tudo mesmo assim");
    // Nada continua escrevendo.
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(tamanho(&servidor), depois);
}

#[test]
fn falha_num_arquivo_do_meio_diz_o_caminho_e_para_as_outras() {
    let pasta = Pasta::nova("arvore-falha");
    montar(&pasta, "origem", 150);
    let servidor = Servidor::novo(pasta.caminho());
    servidor.falhar_escrita_em("/home/destino/a/b c/arquivo 0100.txt");
    let mut sftp = conectar(&servidor);

    let erro = sftp
        .upload_tree(
            &pasta.caminho().join("origem"),
            "/home/destino",
            &TreeOptions::default(),
            &|_, _| {},
            &nunca(),
        )
        .unwrap()
        .unwrap_err();

    assert!(
        erro.to_string()
            .contains("/home/destino/a/b c/arquivo 0100.txt"),
        "{erro}"
    );
    assert_ne!(erro.kind(), io::ErrorKind::Interrupted, "{erro}");
    // A conexão segue boa para o resto.
    assert!(sftp.lstat("/home/destino").is_ok());
}

#[test]
fn data_e_permissoes_so_quando_pedidas() {
    let pasta = Pasta::nova("arvore-atributos");
    let cheio = pasta.escrever("origem/cheio.sh", b"#!/bin/sh\n");
    let leitura = pasta.escrever("origem/sub/leitura.txt", b"dados");
    fs::set_permissions(&cheio, fs::Permissions::from_mode(0o755)).unwrap();
    fs::set_permissions(&leitura, fs::Permissions::from_mode(0o640)).unwrap();
    let data = SystemTime::UNIX_EPOCH + Duration::from_secs(1_500_000_000);
    fs::File::open(&cheio).unwrap().set_modified(data).unwrap();
    fs::File::open(&leitura)
        .unwrap()
        .set_modified(data)
        .unwrap();
    let servidor = Servidor::novo(pasta.caminho());
    let mut sftp = conectar(&servidor);

    let tudo = TreeOptions {
        preserve_date: true,
        preserve_permissions: true,
        ..TreeOptions::default()
    };
    sftp.upload_tree(
        &pasta.caminho().join("origem"),
        "/home/com",
        &tudo,
        &|_, _| {},
        &nunca(),
    )
    .unwrap()
    .unwrap();
    let com = fs::metadata(servidor.real("/home/com/sub/leitura.txt")).unwrap();
    assert_eq!(com.permissions().mode() & 0o7777, 0o640);
    assert_eq!(com.modified().unwrap(), data);
    let com = fs::metadata(servidor.real("/home/com/cheio.sh")).unwrap();
    assert_eq!(com.permissions().mode() & 0o7777, 0o755);

    let nada = TreeOptions {
        preserve_date: false,
        preserve_permissions: false,
        ..TreeOptions::default()
    };
    sftp.upload_tree(
        &pasta.caminho().join("origem"),
        "/home/sem",
        &nada,
        &|_, _| {},
        &nunca(),
    )
    .unwrap()
    .unwrap();
    let sem = fs::metadata(servidor.real("/home/sem/sub/leitura.txt")).unwrap();
    assert_ne!(sem.modified().unwrap(), data);
}

#[test]
fn link_para_arquivo_vira_arquivo_e_link_para_pasta_fica_de_fora() {
    let pasta = Pasta::nova("arvore-links");
    pasta.escrever("origem/real.txt", b"conteudo");
    pasta.escrever("origem/outra/x.txt", b"x");
    std::os::unix::fs::symlink(
        pasta.caminho().join("origem/real.txt"),
        pasta.caminho().join("origem/atalho.txt"),
    )
    .unwrap();
    std::os::unix::fs::symlink(
        pasta.caminho().join("origem/outra"),
        pasta.caminho().join("origem/atalho-pasta"),
    )
    .unwrap();
    let servidor = Servidor::novo(pasta.caminho());
    let mut sftp = conectar(&servidor);

    sftp.upload_tree(
        &pasta.caminho().join("origem"),
        "/home/destino",
        &TreeOptions::default(),
        &|_, _| {},
        &nunca(),
    )
    .unwrap()
    .unwrap();

    let chegou = arvore(&servidor.real("/home/destino"));
    assert_eq!(
        chegou.keys().map(String::as_str).collect::<Vec<_>>(),
        ["atalho.txt", "outra", "outra/x.txt", "real.txt"]
    );
    assert_eq!(chegou["atalho.txt"].as_deref(), Some(&b"conteudo"[..]));
    assert!(
        !fs::symlink_metadata(servidor.real("/home/destino/atalho.txt"))
            .unwrap()
            .is_symlink()
    );
}

#[test]
fn pasta_vazia_e_destino_dentro_de_pasta_que_falta() {
    let pasta = Pasta::nova("arvore-vazia");
    fs::create_dir_all(pasta.caminho().join("origem")).unwrap();
    let servidor = Servidor::novo(pasta.caminho());
    let mut sftp = conectar(&servidor);

    sftp.upload_tree(
        &pasta.caminho().join("origem"),
        "/home/nova",
        &TreeOptions::default(),
        &|_, _| {},
        &nunca(),
    )
    .unwrap()
    .unwrap();
    assert!(servidor.real("/home/nova").is_dir());

    let erro = sftp
        .upload_tree(
            &pasta.caminho().join("origem"),
            "/home/falta/nova",
            &TreeOptions::default(),
            &|_, _| {},
            &nunca(),
        )
        .unwrap()
        .unwrap_err();
    assert_eq!(erro.kind(), io::ErrorKind::NotFound, "{erro}");
}

#[test]
fn quem_nao_sabe_devolve_none() {
    let mut memoria = MemFs::new();
    let resposta = memoria.upload_tree(
        Path::new("/tmp"),
        "/destino",
        &TreeOptions::default(),
        &|_, _| {},
        &nunca(),
    );
    assert!(resposta.is_none());
}

#[test]
fn a_fila_manda_uma_pasta_nova_como_um_item_so() {
    let pasta = Pasta::nova("arvore-fila");
    let total = montar(&pasta, "origem", 60);
    let servidor = Servidor::novo(pasta.caminho());
    let fila = Transfers::new(2);
    let remoto = Endpoint::Remote(opener(&servidor));

    fila.add(
        (Endpoint::Local, pasta.texto("origem")),
        (remoto.clone(), "/home/destino".to_owned()),
        Conflict::Overwrite,
    );
    assert!(fila.wait_idle(ESPERA));

    let itens = fila.items().get_untracked();
    assert_eq!(itens.len(), 1, "{itens:?}");
    assert_eq!(itens[0].state, TransferState::Done);
    assert!(itens[0].is_dir);
    assert_eq!(itens[0].size, Some(total));
    assert_eq!(itens[0].done, total);
    assert_eq!(
        arvore(&pasta.caminho().join("origem")),
        arvore(&servidor.real("/home/destino"))
    );
}

#[test]
fn a_fila_continua_item_a_item_quando_a_pasta_ja_existe_e_so_a_nova_de_dentro_vai_de_uma_vez() {
    let pasta = Pasta::nova("arvore-fila-existe");
    pasta.escrever("origem/um.txt", b"um");
    pasta.escrever("origem/sub/dois.txt", b"dois");
    let servidor = Servidor::novo(pasta.caminho());
    servidor.escrever("/home/destino/velho.txt", b"velho");
    let fila = Transfers::new(2);

    fila.add(
        (Endpoint::Local, pasta.texto("origem")),
        (
            Endpoint::Remote(opener(&servidor)),
            "/home/destino".to_owned(),
        ),
        Conflict::Overwrite,
    );
    assert!(fila.wait_idle(ESPERA));

    let itens = fila.items().get_untracked();
    // A pasta (que já existe), `um.txt` e `sub`, que é nova e vai inteira num item só.
    assert_eq!(itens.len(), 3, "{itens:?}");
    assert_eq!(itens[2].size, Some(4));
    assert!(itens.iter().all(|i| i.state == TransferState::Done));
    assert_eq!(
        fs::read(servidor.real("/home/destino/um.txt")).unwrap(),
        b"um"
    );
    assert!(servidor.real("/home/destino/velho.txt").exists());
}

#[test]
fn cancelar_na_fila_marca_o_item_como_cancelado() {
    let pasta = Pasta::nova("arvore-fila-cancela");
    for i in 0..4 {
        pasta.escrever(&format!("origem/grande {i}.bin"), &dados(24 * 1024 * 1024));
    }
    let servidor = Servidor::novo(pasta.caminho());
    let fila = Transfers::new(1);
    let id = fila.add(
        (Endpoint::Local, pasta.texto("origem")),
        (
            Endpoint::Remote(opener(&servidor)),
            "/home/destino".to_owned(),
        ),
        Conflict::Overwrite,
    );
    let limite = std::time::Instant::now() + ESPERA;
    while fila
        .items()
        .get_untracked()
        .iter()
        .all(|item| item.done == 0)
    {
        assert!(std::time::Instant::now() < limite, "não começou");
        std::thread::sleep(Duration::from_millis(10));
    }
    fila.cancel(id);
    assert!(fila.wait_idle(ESPERA));
    let itens = fila.items().get_untracked();
    assert_eq!(itens[0].state, TransferState::Cancelled, "{itens:?}");
}
