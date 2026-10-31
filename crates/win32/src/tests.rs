//! Testes do crate: só rodam no Windows (o crate inteiro é `cfg(windows)`).

use std::time::{SystemTime, UNIX_EPOCH};

use tokio::io::AsyncWriteExt;
use tokio::net::windows::named_pipe::ClientOptions;

use super::*;

/// Um nome que nenhum outro teste (nem processo) usa.
fn nome_unico(prefixo: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{prefixo}-{}-{nanos}", std::process::id())
}

#[test]
fn protecao_e_igual_em_duas_chamadas() {
    let mut a = [0u8; 16];
    a[..8].copy_from_slice(b"Pageant\0");
    let mut b = a;
    protect_memory_cross_process(&mut a).unwrap();
    protect_memory_cross_process(&mut b).unwrap();
    assert_eq!(a, b, "é o que torna o nome do pipe estável");
    assert_ne!(&a[..8], b"Pageant\0", "o buffer foi cifrado");
}

#[test]
fn tamanho_que_nao_e_multiplo_de_16_e_recusado() {
    for tamanho in [0, 1, 15, 17, 24] {
        let erro = protect_memory_cross_process(&mut vec![0u8; tamanho]).unwrap_err();
        assert_eq!(erro.kind(), std::io::ErrorKind::InvalidInput, "{tamanho}");
    }
}

#[test]
fn o_nome_do_usuario_nao_e_vazio_nem_tem_arroba() {
    let nome = pageant_user_name().unwrap();
    assert!(!nome.is_empty());
    assert!(!nome.contains('@'));
}

#[tokio::test]
async fn pid_do_cliente_e_o_proprio() {
    let nome = format!(r"\\.\pipe\{}", nome_unico("tomni-ui-win32-pid"));
    let servidor = private_pipe(&nome, true).unwrap();
    let mut cliente = ClientOptions::new().open(&nome).unwrap();
    servidor.connect().await.unwrap();
    assert_eq!(
        named_pipe_client_pid(&servidor).unwrap(),
        std::process::id()
    );
    cliente.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_primeira_instancia_nao_deixa_outro_tomar_o_nome() {
    let nome = format!(r"\\.\pipe\{}", nome_unico("tomni-ui-win32-primeiro"));
    let _primeiro = private_pipe(&nome, true).unwrap();
    let erro = private_pipe(&nome, true).unwrap_err();
    assert_eq!(erro.kind(), std::io::ErrorKind::PermissionDenied);
}

#[test]
fn tabela_tem_o_proprio_processo() {
    let tabela = process_table().unwrap();
    let eu = tabela
        .iter()
        .find(|p| p.pid == std::process::id())
        .expect("o processo do teste está na tabela");
    assert!(eu.parent_pid != 0);
    assert!(eu.name.to_lowercase().ends_with(".exe"), "{}", eu.name);
    assert!(eu.threads >= 1);
}

#[test]
fn o_caminho_e_o_usuario_do_proprio_processo() {
    let caminho = process_path(std::process::id()).unwrap();
    let exe = std::env::current_exe().unwrap();
    assert_eq!(
        caminho.to_lowercase(),
        exe.to_string_lossy().to_lowercase(),
        "o caminho do executável"
    );
    let usuario = process_user(std::process::id()).unwrap();
    assert!(!usuario.is_empty());
    assert!(!usuario.contains('\\'), "sem o domínio: {usuario}");
    assert!(process_path(u32::MAX).is_err());
    assert!(process_user(u32::MAX).is_err());
}

#[test]
fn a_escrita_em_disco_aparece_nos_contadores() {
    let antes = process_usage(std::process::id()).unwrap();
    let arquivo = std::env::temp_dir().join(nome_unico("tomni-ui-win32-es"));
    std::fs::write(&arquivo, vec![7u8; 256 * 1024]).unwrap();
    let lido = std::fs::read(&arquivo).unwrap();
    std::fs::remove_file(&arquivo).unwrap();
    let depois = process_usage(std::process::id()).unwrap();
    assert_eq!(lido.len(), 256 * 1024);
    assert!(depois.written_bytes >= antes.written_bytes + 256 * 1024);
    assert!(depois.read_bytes >= antes.read_bytes + 256 * 1024);
    assert_eq!(depois.start_time, antes.start_time, "o mesmo processo");
    assert!(antes.start_time > 0);
}

#[test]
fn o_uso_do_proprio_processo_cresce() {
    let antes = process_usage(std::process::id()).unwrap();
    assert!(antes.memory > 0);
    let fim = std::time::Instant::now() + std::time::Duration::from_millis(200);
    let mut n = 0_u64;
    while std::time::Instant::now() < fim {
        n = std::hint::black_box(n.wrapping_add(1));
    }
    let depois = process_usage(std::process::id()).unwrap();
    assert!(depois.cpu > antes.cpu, "{antes:?} {depois:?}");
    assert!(process_usage(u32::MAX).is_err());
}

#[test]
fn janela_inexistente() {
    assert!(!window_exists(&nome_unico("ClasseQueNaoExiste"), "nada"));
}

#[test]
fn a_janela_aparece_enquanto_vive_e_some_ao_soltar() {
    let classe = nome_unico("TomniUiTeste");
    assert!(!window_exists(&classe, "Teste"));
    let janela = CopyDataWindow::start(&classe, "Teste", 0x1234, |_| true).unwrap();
    assert!(window_exists(&classe, "Teste"));
    drop(janela);
    assert!(!window_exists(&classe, "Teste"));
}
