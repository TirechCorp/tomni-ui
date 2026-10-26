//! O teste real do M57: o `tomni-ui-wireguard` contra o WireGuard do kernel do Linux, em Docker.
//! Fora do CI. Rode com `testes/wireguard/rodar.sh`, que sobe o servidor
//! (`testes/wireguard/docker-compose.yml`), roda estes testes (`#[ignore]`) e derruba.
//!
//! O servidor: `wg0` com `10.66.0.1`, um sshd (porta 22, usuário e senha `teste`) e um eco (porta
//! 7). Os `.conf` dos clientes, com chaves geradas pelo `wg genkey` do contêiner, ficam na pasta
//! de `TOMNI_UI_WIREGUARD_TESTE`. O cliente do teste entra pela porta UDP publicada em
//! `127.0.0.1` e abre as conexões pela pilha embutida, sem placa virtual.

use std::io::Read;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tomni_ui_wireguard::{State, WgConfig, WireGuard, WireGuardBuilder};

const SERVIDOR: &str = "10.66.0.1";
const CONTEINER: &str = "wireguard-servidor";

fn conf(nome: &str) -> WgConfig {
    let pasta = PathBuf::from(std::env::var("TOMNI_UI_WIREGUARD_TESTE").expect(
        "TOMNI_UI_WIREGUARD_TESTE aponta para a pasta dos .conf (testes/wireguard/rodar.sh)",
    ));
    let texto = std::fs::read_to_string(pasta.join(nome)).expect("o .conf do cliente");
    WgConfig::from_conf(&texto).expect("o .conf é válido").0
}

async fn entrar(nome: &str) -> WireGuard {
    let wg = WireGuardBuilder::new(conf(nome))
        .start()
        .await
        .expect("o túnel não subiu");
    wg.wait_connected(Duration::from_secs(30))
        .await
        .unwrap_or_else(|e| panic!("o servidor WireGuard não respondeu: {e}"));
    wg
}

/// Abre uma conexão TCP pelo túnel, tentando de novo por até um minuto.
async fn conectar(wg: &WireGuard, porta: u16, limite: Duration) -> tomni_ui_netstack::TcpStream {
    let addr = format!("{SERVIDOR}:{porta}").parse().unwrap();
    let inicio = Instant::now();
    loop {
        match wg
            .stack()
            .connect_timeout(addr, Duration::from_secs(10))
            .await
        {
            Ok(conexao) => return conexao,
            Err(erro) if inicio.elapsed() < limite => {
                eprintln!("ainda sem {SERVIDOR}:{porta}: {erro}");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Err(erro) => panic!("sem conexão com {SERVIDOR}:{porta}: {erro}"),
        }
    }
}

/// Manda `tamanho` bytes ao eco e confere o SHA-256 do que volta.
async fn eco(wg: &WireGuard, tamanho: usize) {
    let conexao = conectar(wg, 7, Duration::from_secs(60)).await;
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
#[ignore = "precisa do servidor WireGuard em Docker: testes/wireguard/rodar.sh"]
async fn importa_o_conf_conecta_le_o_banner_ssh_e_faz_eco_de_10_mib() {
    let wg = entrar("cliente.conf").await;
    assert!(
        matches!(wg.state(), State::Connected { .. }),
        "{:?}",
        wg.state()
    );

    // O banner do sshd, pelo proxy SOCKS5 que os clientes do app usam.
    let proxy = wg.proxy();
    let banner = tokio::task::spawn_blocking(move || {
        let mut tcp = proxy
            .open(SERVIDOR, 22, Duration::from_secs(10))
            .expect("o sshd pelo túnel");
        let mut buf = [0u8; 64];
        let n = tcp.read(&mut buf).unwrap();
        String::from_utf8_lossy(&buf[..n]).into_owned()
    })
    .await
    .unwrap();
    assert!(banner.starts_with("SSH-2.0-"), "{banner:?}");

    eco(&wg, 10 << 20).await;

    let pares = wg.peers().await;
    assert_eq!(pares.len(), 1);
    assert!(
        pares[0].connected && pares[0].rx_bytes > 10 << 20,
        "{pares:?}"
    );
    wg.stop(Duration::from_secs(2)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "precisa do servidor WireGuard em Docker: testes/wireguard/rodar.sh"]
async fn com_chave_pre_compartilhada() {
    let wg = entrar("cliente-psk.conf").await;
    eco(&wg, 2 << 20).await;
    wg.stop(Duration::from_secs(2)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "precisa do servidor WireGuard em Docker: testes/wireguard/rodar.sh"]
async fn ssh_de_verdade_pelo_proxy_do_tunel() {
    use tomni_ui_terminal::ssh::SshBuilder;

    let wg = entrar("cliente.conf").await;
    let proxy = wg.proxy();
    let saida = tokio::task::spawn_blocking(move || {
        SshBuilder::new(SERVIDOR, 22, "teste")
            .password("teste")
            .proxy(proxy)
            .check_host_key(|_, _| true)
            .exec("echo ok", b"")
            .map(|s| s.stdout_text())
    })
    .await
    .unwrap()
    .expect("o ssh pelo túnel falhou");
    assert_eq!(saida.trim(), "ok");
    wg.stop(Duration::from_secs(2)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "precisa do servidor WireGuard em Docker: testes/wireguard/rodar.sh"]
async fn o_servidor_reinicia_o_wireguard_e_o_motor_refaz_o_handshake() {
    let wg = entrar("cliente.conf").await;
    eco(&wg, 64 * 1024).await;

    // Recria a interface do servidor: ele perde todas as sessões.
    let status = Command::new("docker")
        .args(["exec", CONTEINER, "/subir-wg.sh"])
        .status()
        .expect("o docker");
    assert!(status.success(), "o servidor não reiniciou o wireguard");

    // Sem sessão no servidor, o cliente só se recupera quando o relógio manda o handshake de
    // novo (o keepalive persistente de 25 s ou o rekey de quem envia sem receber): espera.
    let conexao = conectar(&wg, 7, Duration::from_secs(90)).await;
    drop(conexao);
    eco(&wg, 256 * 1024).await;
    // `last_handshake` é o tempo desde o último handshake: refeito, é recente.
    let depois = wg.peers().await[0].last_handshake;
    assert!(
        depois.is_some_and(|d| d < Duration::from_secs(60)),
        "o handshake não foi refeito: {depois:?}"
    );
    wg.stop(Duration::from_secs(2)).await;
}
