//! O `Rclone` contra o `rclone` de verdade. Fora do `cargo test --workspace`: os testes são
//! `#[ignore]` e só rodam com `TOMNI_UI_RCLONE` apontando para o binário (ou `rclone`, para o
//! do `PATH`):
//!
//! ```sh
//! TOMNI_UI_RCLONE=rclone cargo test -p tomni-ui-files --features rclone -- --ignored rclone_real
//! ```
#![cfg(all(unix, feature = "rclone"))]

mod comum;

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use tomni_ui_files::rclone::config::ConfigStep;
use tomni_ui_files::rclone::{Rcd, RcdConfig, Rclone, find_rclone};
use tomni_ui_files::{EntryKind, Fs, Location};

use comum::{Pasta, dados};

const MIB: usize = 1 << 20;

fn binario() -> PathBuf {
    let valor = std::env::var("TOMNI_UI_RCLONE")
        .expect("TOMNI_UI_RCLONE com o caminho do rclone (ou `rclone`)");
    if valor == "rclone" {
        find_rclone(None).expect("rclone no PATH")
    } else {
        PathBuf::from(valor)
    }
}

/// Um `rcd` de verdade com a configuração numa pasta temporária.
fn subir(pasta: &Pasta) -> Arc<Rcd> {
    Rcd::spawn(RcdConfig {
        config_file: Some(pasta.caminho().join("rclone.conf")),
        ..RcdConfig::new(binario())
    })
    .expect("subir o rclone rcd")
}

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

#[test]
#[ignore = "precisa do rclone (TOMNI_UI_RCLONE)"]
fn rclone_real_o_fs_contra_o_remoto_memory() {
    let pasta = Pasta::nova("rclone-real");
    let rcd = subir(&pasta);
    assert!(rcd.version().unwrap() >= (1, 60, 0));
    assert!(matches!(
        rcd.config_create("m", "memory", &[]).unwrap(),
        ConfigStep::Done
    ));
    assert!(
        rcd.config_remotes()
            .unwrap()
            .iter()
            .any(|r| r.name == "m" && r.kind == "memory")
    );

    let mut fs = Rclone::new(Arc::clone(&rcd), "m:");
    assert_eq!(fs.home().unwrap(), "/");
    assert!(fs.list("/").unwrap().is_empty());

    // Enviar 3 MiB em fluxo e ler de volta, inteiro e do meio.
    let conteudo = dados(3 * MIB + 17);
    fs.mkdir("/dir x").unwrap();
    gravar(&mut fs, "/dir x/ação.bin", &conteudo);
    let entrada = fs.stat("/dir x/ação.bin").unwrap();
    assert_eq!(entrada.kind, EntryKind::File);
    assert_eq!(entrada.size, conteudo.len() as u64);
    assert!(entrada.modified.is_some());
    assert_eq!(ler_tudo(&mut fs, "/dir x/ação.bin", 0), conteudo);
    assert_eq!(
        ler_tudo(&mut fs, "/dir x/ação.bin", 1_000_000),
        conteudo[1_000_000..]
    );
    assert_eq!(fs.stat("/dir x").unwrap().kind, EntryKind::Dir,);
    assert_eq!(
        fs.stat("/nada").unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );
    assert_eq!(
        fs.list("/nada").unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );

    // Renomear arquivo e pasta, apagar.
    fs.rename("/dir x/ação.bin", "/dir x/b.bin", false).unwrap();
    assert_eq!(ler_tudo(&mut fs, "/dir x/b.bin", 0), conteudo);
    fs.rename("/dir x", "/dir y", false).unwrap();
    let nomes: Vec<String> = fs.list("/").unwrap().into_iter().map(|e| e.name).collect();
    assert_eq!(nomes, vec!["dir y".to_owned()]);
    fs.remove_file("/dir y/b.bin").unwrap();
    fs.remove_dir("/dir y").unwrap();
    assert!(fs.list("/").unwrap().is_empty());

    // Features e espaço.
    let features = fs.features().unwrap();
    assert!(features.copy);

    // Cópia delegada: uma pasta local para `m:` e de volta.
    let local = Pasta::nova("rclone-real-local");
    local.escrever("origem/a.bin", &dados(300_000));
    let nao_cancelar = AtomicBool::new(false);
    let maior = std::cell::Cell::new(0u64);
    let progresso = |feito: u64, _total: u64| maior.set(maior.get().max(feito));
    fs.delegated_copy(
        &Location::Local(local.caminho().join("origem/a.bin")),
        &Location::Here("/copiado/a.bin".into()),
        &progresso,
        &nao_cancelar,
    )
    .expect("o rclone copia")
    .unwrap();
    assert_eq!(ler_tudo(&mut fs, "/copiado/a.bin", 0), dados(300_000));
    fs.delegated_copy(
        &Location::Here("/copiado/a.bin".into()),
        &Location::Local(local.caminho().join("volta.bin")),
        &progresso,
        &nao_cancelar,
    )
    .expect("o rclone copia")
    .unwrap();
    assert_eq!(
        std::fs::read(local.caminho().join("volta.bin")).unwrap(),
        dados(300_000)
    );

    // Apagar o remoto da configuração.
    rcd.config_delete("m").unwrap();
    assert!(rcd.config_remotes().unwrap().is_empty());

    // Encerrar.
    rcd.shutdown();
    assert!(rcd.version().is_err());
}

#[test]
#[ignore = "precisa do rclone (TOMNI_UI_RCLONE)"]
fn rclone_real_o_drive_pergunta_se_o_login_e_local() {
    let pasta = Pasta::nova("rclone-real-drive");
    let rcd = subir(&pasta);

    let provedores = rcd.config_providers().unwrap();
    assert!(provedores.len() > 40, "{} provedores", provedores.len());
    let drive = provedores.iter().find(|p| p.name == "drive").unwrap();
    assert!(drive.options.iter().any(|o| o.name == "client_id"));
    assert!(drive.options.iter().any(|o| o.advanced));

    match rcd.config_create("gd", "drive", &[]).unwrap() {
        ConfigStep::Question { option, state, .. } => {
            assert_eq!(option.name, "config_is_local");
            assert_eq!(option.kind, "bool");
            assert!(!state.is_empty());
            // Responde que não: a pergunta seguinte pede o código do `rclone authorize`.
            match rcd.config_answer("gd", &state, "false").unwrap() {
                ConfigStep::Question { option, .. } => assert_eq!(option.name, "config_token"),
                outro => panic!("esperava a pergunta do token: {outro:?}"),
            }
        }
        outro => panic!("esperava a pergunta do config_is_local: {outro:?}"),
    }
    // O remoto inacabado é apagado, sem fazer o login.
    rcd.config_delete("gd").unwrap();
    rcd.shutdown();
}

/// Cifra a configuração `arquivo` com `senha`, dirigindo o menu interativo do rclone num
/// pseudoterminal (o rclone 1.60 só pede a senha pelo terminal).
fn cifrar(arquivo: &std::path::Path, senha: &str) {
    let roteiro = format!(
        "import pty, os, select\npid, fd = pty.fork()\nif pid == 0:\n    os.execvp({binario:?}, [{binario:?}, '--config', {arquivo:?}, 'config'])\n\
def le(t):\n    while select.select([fd], [], [], t)[0]:\n        try:\n            if not os.read(fd, 4096): break\n        except OSError: break\n\
le(1.5)\nfor e in ['s\\n', 'a\\n', {senha:?} + '\\n', {senha:?} + '\\n', 'q\\n', 'q\\n']:\n    os.write(fd, e.encode()); le(0.8)\n",
        binario = binario().to_string_lossy(),
        arquivo = arquivo.to_string_lossy(),
        senha = senha,
    );
    let status = std::process::Command::new("python3")
        .args(["-I", "-c", &roteiro])
        .status()
        .expect("python3 para dirigir o rclone config");
    assert!(status.success());
    let texto = std::fs::read_to_string(arquivo).unwrap();
    assert!(
        texto.contains("RCLONE_ENCRYPT_V0"),
        "a configuração não foi cifrada"
    );
}

#[test]
#[ignore = "precisa do rclone (TOMNI_UI_RCLONE) e do python3"]
fn rclone_real_a_configuracao_cifrada_pede_a_senha() {
    use tomni_ui_files::rclone::RcloneError;

    let pasta = Pasta::nova("rclone-real-cifrada");
    let arquivo = pasta.caminho().join("rclone.conf");
    std::fs::write(&arquivo, "[m]\ntype = memory\n").unwrap();
    cifrar(&arquivo, "segredo");

    let sem_senha = Rcd::spawn(RcdConfig {
        config_file: Some(arquivo.clone()),
        ..RcdConfig::new(binario())
    });
    assert!(
        matches!(sem_senha, Err(RcloneError::ConfigPassword)),
        "{sem_senha:?}"
    );
    let errada = Rcd::spawn(RcdConfig {
        config_file: Some(arquivo.clone()),
        config_password: Some("errada".into()),
        ..RcdConfig::new(binario())
    });
    assert!(
        matches!(errada, Err(RcloneError::ConfigPassword)),
        "{errada:?}"
    );

    let certa = Rcd::spawn(RcdConfig {
        config_file: Some(arquivo),
        config_password: Some("segredo".into()),
        ..RcdConfig::new(binario())
    })
    .expect("com a senha certa sobe");
    assert_eq!(certa.config_remotes().unwrap()[0].name, "m");
}
