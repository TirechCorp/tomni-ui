//! O teste real do M57: o `tomni-ui-tailscale` contra o headscale e o `tailscale` oficiais, em
//! Docker. Fora do CI. Rode com `testes/tailscale/rodar.sh`, que sobe o ambiente
//! (`testes/tailscale/docker-compose.yml`), roda estes testes (`#[ignore]`) e derruba.
//!
//! O ambiente: o `headscale` (TLS em `127.0.0.1:18443`, DERP embutido), o `direto` (anuncia a
//! sub-rede interna `10.99.0.0/24`), o `escondido` (só usa o DERP) e o `alvo` (`10.99.0.10`,
//! sem rota para o computador). Cada um tem sshd (usuário e senha `teste`) e eco (porta 7). A
//! pasta de `TOMNI_UI_TAILSCALE_TESTE` guarda a CA de teste e a chave de autenticação; a de
//! `TOMNI_UI_TAILSCALE_COMPOSE` é a do `docker-compose.yml`. O cliente do teste não cria placa
//! virtual: abre as conexões pela pilha embutida e pelo proxy SOCKS5.

use std::io::Read;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tomni_ui_tailscale::{NodeState, State, Tailscale, TailscaleBuilder};
use zeroize::Zeroizing;

const CONTROLE: &str = "https://127.0.0.1:18443";

fn pasta() -> PathBuf {
    PathBuf::from(std::env::var("TOMNI_UI_TAILSCALE_TESTE").expect(
        "TOMNI_UI_TAILSCALE_TESTE aponta para a pasta do ambiente (testes/tailscale/rodar.sh)",
    ))
}

fn compose() -> PathBuf {
    PathBuf::from(std::env::var("TOMNI_UI_TAILSCALE_COMPOSE").expect(
        "TOMNI_UI_TAILSCALE_COMPOSE aponta para testes/tailscale (testes/tailscale/rodar.sh)",
    ))
}

fn ca() -> String {
    std::fs::read_to_string(pasta().join("tls/ca.pem")).expect("a CA de teste")
}

fn chave() -> Zeroizing<String> {
    Zeroizing::new(
        std::fs::read_to_string(pasta().join("chave-auth"))
            .expect("a chave de autenticação")
            .trim()
            .to_string(),
    )
}

/// Um nome de máquina que não repete entre as execuções.
fn nome(base: &str) -> String {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    format!("{base}-{}", t % 1_000_000)
}

fn construtor(nome_da_maquina: &str) -> TailscaleBuilder {
    TailscaleBuilder::new(CONTROLE, NodeState::default())
        .hostname(nome(nome_da_maquina))
        .extra_roots(ca())
        .ephemeral(true)
}

async fn entrar(b: TailscaleBuilder) -> Tailscale {
    let ts = b.auth_key(chave()).start().await.expect("o nó não subiu");
    ts.wait_connected(Duration::from_secs(60))
        .await
        .unwrap_or_else(|e| panic!("não conectou: {e}"));
    ts
}

/// Roda um comando do `docker compose` do ambiente.
fn docker(args: &[&str]) -> std::process::Output {
    let saida = Command::new("docker")
        .arg("compose")
        .arg("-f")
        .arg(compose().join("docker-compose.yml"))
        .args(args)
        .env("TS_SAIDA", pasta())
        .output()
        .expect("o docker");
    assert!(
        saida.status.success(),
        "docker compose {args:?} falhou: {}",
        String::from_utf8_lossy(&saida.stderr)
    );
    saida
}

fn headscale(args: &[&str]) -> String {
    let mut tudo = vec!["exec", "-T", "headscale", "headscale"];
    tudo.extend_from_slice(args);
    String::from_utf8_lossy(&docker(&tudo).stdout).into_owned()
}

fn politica(arquivo: &str) {
    headscale(&["policy", "set", "-f", &format!("/politicas/{arquivo}")]);
}

fn endereco_do(ts_peers: &[tomni_ui_tailscale::Peer], nome: &str) -> String {
    ts_peers
        .iter()
        .find(|p| p.name == nome)
        .unwrap_or_else(|| panic!("a máquina {nome} não está no mapa: {ts_peers:?}"))
        .addresses
        .iter()
        .find(|a| a.is_ipv4())
        .expect("endereço IPv4")
        .to_string()
}

/// O banner do sshd (`SSH-2.0-…`) de `destino:22`, pelo proxy SOCKS5 que os clientes do app usam.
async fn banner(ts: &Tailscale, destino: &str) -> std::io::Result<String> {
    let proxy = ts.proxy();
    let destino = destino.to_string();
    tokio::task::spawn_blocking(move || {
        let mut tcp = proxy
            .open(&destino, 22, Duration::from_secs(20))
            .map_err(std::io::Error::other)?;
        let mut buf = [0u8; 64];
        let n = tcp.read(&mut buf)?;
        Ok(String::from_utf8_lossy(&buf[..n]).into_owned())
    })
    .await
    .unwrap()
}

/// Tenta o banner por até `limite` (o caminho e as rotas levam um instante).
async fn banner_com_espera(ts: &Tailscale, destino: &str, limite: Duration) -> String {
    let inicio = Instant::now();
    loop {
        match banner(ts, destino).await {
            Ok(b) if b.starts_with("SSH-2.0-") => return b,
            outro if inicio.elapsed() >= limite => {
                panic!("sem banner SSH de {destino}: {outro:?}")
            }
            _ => tokio::time::sleep(Duration::from_secs(1)).await,
        }
    }
}

/// Manda `tamanho` bytes ao eco de `destino:7` pelo proxy e confere o SHA-256 do que volta.
async fn eco(ts: &Tailscale, destino: &str, tamanho: usize) {
    let addr = ts.socks().addr;
    let (usuario, senha) = (ts.socks().user.clone(), ts.socks().password.clone());
    let mut tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    socks5(&mut tcp, &usuario, &senha, destino, 7).await;
    let (mut le, mut escreve) = tcp.into_split();
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
            Duration::from_secs(60),
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

async fn socks5(
    tcp: &mut tokio::net::TcpStream,
    usuario: &str,
    senha: &str,
    host: &str,
    porta: u16,
) {
    tcp.write_all(&[5, 1, 2]).await.unwrap();
    let mut r = [0u8; 2];
    tcp.read_exact(&mut r).await.unwrap();
    let mut auth = vec![1, usuario.len() as u8];
    auth.extend_from_slice(usuario.as_bytes());
    auth.push(senha.len() as u8);
    auth.extend_from_slice(senha.as_bytes());
    tcp.write_all(&auth).await.unwrap();
    tcp.read_exact(&mut r).await.unwrap();
    assert_eq!(r, [1, 0], "a ponte recusou a credencial");
    let mut pedido = vec![5, 1, 0];
    match host.parse::<std::net::Ipv4Addr>() {
        Ok(ip) => {
            pedido.push(1);
            pedido.extend_from_slice(&ip.octets());
        }
        Err(_) => {
            pedido.push(3);
            pedido.push(host.len() as u8);
            pedido.extend_from_slice(host.as_bytes());
        }
    }
    pedido.extend_from_slice(&porta.to_be_bytes());
    tcp.write_all(&pedido).await.unwrap();
    let mut cab = [0u8; 4];
    tcp.read_exact(&mut cab).await.unwrap();
    assert_eq!(cab[1], 0, "o SOCKS5 recusou {host}:{porta}");
    let resto = match cab[3] {
        1 => 6,
        4 => 18,
        _ => {
            let mut n = [0u8; 1];
            tcp.read_exact(&mut n).await.unwrap();
            usize::from(n[0]) + 2
        }
    };
    let mut lixo = vec![0u8; resto];
    tcp.read_exact(&mut lixo).await.unwrap();
}

/// Espera o caminho até a máquina `nome` começar com `prefixo`, usando o eco para manter o
/// tráfego.
async fn esperar_caminho(ts: &Tailscale, nome: &str, prefixo: &str, limite: Duration) -> String {
    let inicio = Instant::now();
    loop {
        let atual = ts
            .peers()
            .await
            .into_iter()
            .find(|p| p.name == nome)
            .map(|p| p.path)
            .unwrap_or_default();
        if atual.starts_with(prefixo) {
            return atual;
        }
        assert!(
            inicio.elapsed() < limite,
            "o caminho até {nome} devia ser \"{prefixo}\", mas é \"{atual}\""
        );
        let _ = banner(ts, nome).await;
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "precisa do ambiente em Docker: testes/tailscale/rodar.sh"]
async fn registra_com_chave_de_autenticacao_e_conecta() {
    let ts = entrar(construtor("t1")).await;
    match ts.state() {
        State::Connected {
            address,
            tailnet,
            peers_online,
            ..
        } => {
            assert!(address.to_string().starts_with("100.64."), "{address}");
            // O headscale manda como domínio o nome do servidor; o MagicDNS usa o `base_domain`.
            assert!(!tailnet.is_empty());
            assert!(peers_online >= 2, "{peers_online}");
        }
        outro => panic!("{outro:?}"),
    }
    let maquinas = ts.peers().await;
    assert!(maquinas.iter().any(|p| p.name == "direto"), "{maquinas:?}");
    assert!(
        maquinas.iter().any(|p| p.name == "escondido"),
        "{maquinas:?}"
    );
    ts.stop(Duration::from_secs(3)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "precisa do ambiente em Docker: testes/tailscale/rodar.sh"]
async fn so_derp_banner_ssh_por_ip_e_por_nome_e_eco_de_10_mib() {
    let ts = entrar(construtor("t2").udp(false)).await;
    let ip = endereco_do(&ts.peers().await, "direto");
    assert!(
        banner_com_espera(&ts, &ip, Duration::from_secs(60))
            .await
            .starts_with("SSH-2.0-")
    );
    banner_com_espera(&ts, "direto", Duration::from_secs(20)).await;
    banner_com_espera(&ts, "direto.tailnet.teste", Duration::from_secs(20)).await;
    eco(&ts, "direto", 10 << 20).await;
    let caminho = ts
        .peers()
        .await
        .into_iter()
        .find(|p| p.name == "direto")
        .map(|p| p.path)
        .unwrap_or_default();
    assert!(caminho.starts_with("DERP"), "caminho: {caminho}");
    ts.stop(Duration::from_secs(3)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "precisa do ambiente em Docker: testes/tailscale/rodar.sh"]
async fn com_udp_o_caminho_passa_a_direto() {
    let ts = entrar(construtor("t3")).await;
    banner_com_espera(&ts, "direto", Duration::from_secs(60)).await;
    let caminho = esperar_caminho(&ts, "direto", "direto", Duration::from_secs(60)).await;
    eprintln!("caminho até o direto: {caminho}");
    eco(&ts, "direto", 4 << 20).await;
    ts.stop(Duration::from_secs(3)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "precisa do ambiente em Docker: testes/tailscale/rodar.sh"]
async fn o_escondido_so_responde_pelo_derp() {
    let ts = entrar(construtor("t4")).await;
    banner_com_espera(&ts, "escondido", Duration::from_secs(60)).await;
    eco(&ts, "escondido", 2 << 20).await;
    let caminho = ts
        .peers()
        .await
        .into_iter()
        .find(|p| p.name == "escondido")
        .map(|p| p.path)
        .unwrap_or_default();
    assert!(caminho.starts_with("DERP"), "caminho: {caminho}");
    ts.stop(Duration::from_secs(3)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "precisa do ambiente em Docker: testes/tailscale/rodar.sh"]
async fn sub_rede_anunciada_so_com_accept_routes() {
    politica("politica-livre.json");
    let com = entrar(construtor("t5a").accept_routes(true)).await;
    banner_com_espera(&com, "10.99.0.10", Duration::from_secs(60)).await;
    com.stop(Duration::from_secs(3)).await;

    let sem = entrar(construtor("t5b").accept_routes(false)).await;
    let erro = banner(&sem, "10.99.0.10").await.expect_err("sem a rota");
    assert!(
        erro.to_string()
            .contains("não pertence a nenhuma máquina nem sub-rede")
            || erro.to_string().contains("rede inalcançável"),
        "{erro}"
    );
    sem.stop(Duration::from_secs(3)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "precisa do ambiente em Docker: testes/tailscale/rodar.sh"]
async fn login_pelo_navegador_e_registro_no_headscale() {
    let ts = construtor("t6").udp(false).start().await.unwrap();
    let inicio = Instant::now();
    let url = loop {
        if let State::NeedsLogin { url } = ts.state() {
            break url;
        }
        assert!(
            inicio.elapsed() < Duration::from_secs(30),
            "{:?}",
            ts.state()
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    let id = url.rsplit('/').next().expect("o id na URL").to_string();
    // O headscale novo registra por `auth register`; o antigo, por `nodes register`.
    let novo = Command::new("docker")
        .args(["compose", "-f"])
        .arg(compose().join("docker-compose.yml"))
        .args([
            "exec",
            "-T",
            "headscale",
            "headscale",
            "auth",
            "register",
            "--auth-id",
            &id,
            "-u",
            "teste",
        ])
        .env("TS_SAIDA", pasta())
        .output()
        .unwrap();
    if !novo.status.success() {
        headscale(&["nodes", "register", "--key", &id, "--user", "teste"]);
    }
    let ip = ts
        .wait_connected(Duration::from_secs(60))
        .await
        .expect("conecta depois do registro");
    assert!(ip.to_string().starts_with("100.64."), "{ip}");
    ts.stop(Duration::from_secs(3)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "precisa do ambiente em Docker: testes/tailscale/rodar.sh"]
async fn reinicio_com_o_mesmo_estado_e_a_mesma_maquina() {
    let guardado: Arc<Mutex<Option<NodeState>>> = Arc::default();
    let g = guardado.clone();
    let nome_fixo = nome("t7");
    let primeiro = TailscaleBuilder::new(CONTROLE, NodeState::default())
        .hostname(nome_fixo.clone())
        .extra_roots(ca())
        .auth_key(chave())
        .on_node_state(move |n| *g.lock().unwrap() = Some(n.clone()))
        .start()
        .await
        .unwrap();
    let ip = primeiro
        .wait_connected(Duration::from_secs(60))
        .await
        .unwrap();
    primeiro.stop(Duration::from_secs(3)).await;
    let estado = guardado
        .lock()
        .unwrap()
        .clone()
        .expect("o NodeState foi avisado");

    // Sem chave de autenticação: só o estado guardado identifica a máquina.
    let segundo = TailscaleBuilder::new(CONTROLE, estado)
        .hostname(nome_fixo)
        .extra_roots(ca())
        .start()
        .await
        .unwrap();
    let ip2 = segundo
        .wait_connected(Duration::from_secs(60))
        .await
        .unwrap();
    assert_eq!(ip, ip2, "devia ser a mesma máquina");
    segundo.stop(Duration::from_secs(3)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "precisa do ambiente em Docker: testes/tailscale/rodar.sh"]
async fn ssh_de_verdade_pelo_nome_magicdns() {
    use tomni_ui_terminal::ssh::SshBuilder;

    let ts = entrar(construtor("t8")).await;
    banner_com_espera(&ts, "direto", Duration::from_secs(60)).await;
    let proxy = ts.proxy();
    let saida = tokio::task::spawn_blocking(move || {
        SshBuilder::new("direto", 22, "teste")
            .password("teste")
            .proxy(proxy)
            .check_host_key(|_, _| true)
            .exec("echo ok", b"")
            .map(|s| s.stdout_text())
    })
    .await
    .unwrap()
    .expect("o ssh pela tailnet falhou");
    assert_eq!(saida.trim(), "ok");
    ts.stop(Duration::from_secs(3)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "precisa do ambiente em Docker: testes/tailscale/rodar.sh"]
async fn acl_que_proibe_a_porta_7_nao_responde() {
    politica("politica-sem-7.json");
    let ts = entrar(construtor("t9")).await;
    let resultado = async {
        // A porta 22 segue liberada.
        banner_com_espera(&ts, "direto", Duration::from_secs(60)).await;
        // A 7 não: o pacote é descartado pelo destino e a conexão não responde.
        let ip = endereco_do(&ts.peers().await, "direto");
        let addr: SocketAddr = format!("{ip}:7").parse().unwrap();
        let stack = ts.stack().expect("a pilha");
        stack
            .connect_timeout(addr, Duration::from_secs(8))
            .await
            .expect_err("a ACL proíbe a porta 7")
    }
    .await;
    politica("politica-livre.json");
    eprintln!("com a ACL: {resultado}");
    assert!(!resultado.to_string().is_empty());
    ts.stop(Duration::from_secs(3)).await;
}
