//! O teste real do M48: o `tomni-ui-nebula` contra o `nebula` oficial, em Docker. Fora do CI.
//! Rode com `testes/nebula/rodar.sh`, que sobe a rede (`testes/nebula/docker-compose.yml`), roda
//! estes testes (`#[ignore]`) e derruba.
//!
//! A rede oficial: um lighthouse publicado em `127.0.0.1` e um nó `direto`, com um sshd (porta
//! 22, usuário e senha `teste`) e um eco (porta 7). O nó do teste entra pelo lighthouse e abre as
//! conexões pela pilha embutida, sem placa virtual.

use std::io::Read;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tomni_ui_nebula::{Config, Nebula, NebulaBuilder};

fn pasta() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Os dados de uma das três redes do `docker-compose.yml`.
struct Rede {
    /// O certificado do nó do teste (`app`, `app-chacha`, `app-v1`).
    no: &'static str,
    /// A porta UDP do lighthouse em `127.0.0.1`.
    porta: u16,
    /// O endereço Nebula do lighthouse.
    lighthouse: &'static str,
    /// O endereço Nebula do nó `direto`.
    direto: &'static str,
    /// A cifra do config.
    cifra: &'static str,
    /// O contêiner do `direto`.
    conteiner: &'static str,
}

const AES: Rede = Rede {
    no: "app",
    porta: 14242,
    lighthouse: "10.42.0.1",
    direto: "10.42.0.10",
    cifra: "aes",
    conteiner: "nebula-direto",
};
const CHACHA: Rede = Rede {
    no: "app-chacha",
    porta: 14244,
    lighthouse: "10.42.0.3",
    direto: "10.42.0.12",
    cifra: "chachapoly",
    conteiner: "nebula-direto-chacha",
};
const V1: Rede = Rede {
    no: "app-v1",
    porta: 14243,
    lighthouse: "10.42.0.4",
    direto: "10.42.0.13",
    cifra: "aes",
    conteiner: "nebula-direto-v1",
};

fn config(rede: &Rede) -> Config {
    let yaml = format!(
        "pki:\n  ca: ca-25519.crt\n  cert: {}.crt\n  key: {}.key\n\
         static_host_map:\n  \"{}\": [\"127.0.0.1:{}\"]\n\
         lighthouse:\n  interval: 5\n  hosts: [\"{}\"]\n\
         punchy:\n  punch: true\n  respond: true\n\
         listen:\n  host: 0.0.0.0\n  port: 0\n\
         cipher: {}\n",
        rede.no, rede.no, rede.lighthouse, rede.porta, rede.lighthouse, rede.cifra
    );
    Config::from_yaml(&yaml, Some(&pasta()))
        .expect("o config do teste real")
        .0
}

async fn entrar(rede: &Rede) -> Nebula {
    let nebula = NebulaBuilder::new(config(rede))
        .start()
        .await
        .expect("a rede não subiu");
    nebula
        .wait_connected(Duration::from_secs(30))
        .await
        .unwrap_or_else(|e| panic!("o lighthouse oficial não respondeu: {e}"));
    nebula
}

/// Abre uma conexão TCP pelo túnel, tentando de novo por até um minuto (o nó `direto` pode ainda
/// estar fechando o túnel com o lighthouse).
async fn conectar(nebula: &Nebula, destino: &str, porta: u16) -> tomni_ui_netstack::TcpStream {
    let addr = format!("{destino}:{porta}").parse().unwrap();
    let inicio = Instant::now();
    loop {
        match nebula
            .stack()
            .connect_timeout(addr, Duration::from_secs(10))
            .await
        {
            Ok(conexao) => return conexao,
            Err(erro) if inicio.elapsed() < Duration::from_secs(60) => {
                eprintln!("ainda sem {destino}:{porta}: {erro}");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Err(erro) => panic!("sem conexão com {destino}:{porta}: {erro}"),
        }
    }
}

/// Manda `tamanho` bytes ao eco e confere o SHA-256 do que volta.
async fn eco(nebula: &Nebula, destino: &str, tamanho: usize) {
    let conexao = conectar(nebula, destino, 7).await;
    let (mut le, mut escreve) = tokio::io::split(conexao);
    let escritor = tokio::spawn(async move {
        let mut hash = Sha256::new();
        let mut falta = tamanho;
        let mut n = 0u32;
        while falta > 0 {
            let lote = falta.min(16 * 1024);
            let bloco: Vec<u8> = (0..lote)
                .map(|i| (n.wrapping_mul(2_654_435_761).wrapping_add(i as u32) >> 3) as u8)
                .collect();
            n = n.wrapping_add(1);
            hash.update(&bloco);
            escreve.write_all(&bloco).await.unwrap();
            falta -= lote;
        }
        escreve.flush().await.unwrap();
        (hash.finalize(), escreve)
    });
    let mut recebido = Sha256::new();
    let mut falta = tamanho;
    let mut buf = vec![0u8; 64 * 1024];
    while falta > 0 {
        let n = tokio::time::timeout(
            Duration::from_secs(30),
            le.read(&mut buf[..falta.min(64 * 1024)]),
        )
        .await
        .expect("o eco parou de responder")
        .unwrap();
        assert!(n > 0, "o eco fechou antes da hora");
        recebido.update(&buf[..n]);
        falta -= n;
    }
    let (enviado, _escreve) = escritor.await.unwrap();
    assert_eq!(enviado, recebido.finalize(), "o eco voltou diferente");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "precisa da rede oficial em Docker: testes/nebula/rodar.sh"]
async fn entra_na_rede_oficial_e_fala_com_o_ssh_e_o_eco() {
    let nebula = entrar(&AES).await;

    // O banner do sshd, pelo proxy SOCKS5 que os clientes do app usam.
    let proxy = nebula.proxy();
    let banner = tokio::task::spawn_blocking(move || {
        let inicio = Instant::now();
        loop {
            match proxy.open("10.42.0.10", 22, Duration::from_secs(10)) {
                Ok(mut tcp) => {
                    let mut buf = [0u8; 64];
                    let n = tcp.read(&mut buf).unwrap();
                    return String::from_utf8_lossy(&buf[..n]).into_owned();
                }
                Err(erro) if inicio.elapsed() < Duration::from_secs(60) => {
                    eprintln!("ainda sem o sshd: {erro}");
                    std::thread::sleep(Duration::from_secs(1));
                }
                Err(erro) => panic!("sem o sshd: {erro}"),
            }
        }
    })
    .await
    .unwrap();
    assert!(banner.starts_with("SSH-2.0-"), "{banner:?}");

    // 10 MiB de ida e volta, com o hash conferido.
    eco(&nebula, AES.direto, 10 << 20).await;

    let tuneis = nebula.tunnels().await;
    assert!(tuneis.iter().any(|t| t.lighthouse), "{tuneis:?}");
    let direto = tuneis.iter().find(|t| t.vpn_addr.to_string() == AES.direto);
    assert!(direto.is_some(), "{tuneis:?}");
    nebula.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "precisa da rede oficial em Docker: testes/nebula/rodar.sh"]
async fn cifra_chachapoly_com_a_rede_oficial() {
    let nebula = entrar(&CHACHA).await;
    eco(&nebula, CHACHA.direto, 2 << 20).await;
    nebula.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "precisa da rede oficial em Docker: testes/nebula/rodar.sh"]
async fn certificados_v1_com_a_rede_oficial() {
    let nebula = entrar(&V1).await;
    eco(&nebula, V1.direto, 2 << 20).await;
    nebula.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "precisa da rede oficial em Docker: testes/nebula/rodar.sh"]
async fn parar_avisa_o_nebula_oficial_com_close_tunnel() {
    let nebula = entrar(&AES).await;
    eco(&nebula, AES.direto, 64 * 1024).await;
    let desde = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .saturating_sub(1);
    nebula.stop().await;
    // O nó oficial registra "Close tunnel received" ao receber o CloseTunnel.
    let inicio = Instant::now();
    loop {
        let saida = Command::new("docker")
            .args(["logs", "--since", &desde.to_string(), AES.conteiner])
            .output()
            .expect("o docker");
        let log = format!(
            "{}{}",
            String::from_utf8_lossy(&saida.stdout),
            String::from_utf8_lossy(&saida.stderr)
        );
        if log.contains("Close tunnel received") {
            return;
        }
        assert!(
            inicio.elapsed() < Duration::from_secs(10),
            "o nebula oficial não registrou o CloseTunnel:\n{log}"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "precisa da rede oficial em Docker: testes/nebula/rodar.sh"]
async fn ssh_de_verdade_pela_ponte_da_rede() {
    use tomni_ui_terminal::ssh::SshBuilder;

    let nebula = entrar(&AES).await;
    let proxy = nebula.proxy();
    let saida = tokio::task::spawn_blocking(move || {
        let inicio = Instant::now();
        loop {
            let resultado = SshBuilder::new(AES.direto, 22, "teste")
                .password("teste")
                .proxy(proxy.clone())
                .check_host_key(|_, _| true)
                .exec("echo ok", b"");
            match resultado {
                Ok(saida) => return saida.stdout_text(),
                Err(erro) if inicio.elapsed() < Duration::from_secs(60) => {
                    eprintln!("ainda sem o ssh: {erro}");
                    std::thread::sleep(Duration::from_secs(1));
                }
                Err(erro) => panic!("o ssh pela rede falhou: {erro}"),
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(saida.trim(), "ok");
    nebula.stop().await;
}
