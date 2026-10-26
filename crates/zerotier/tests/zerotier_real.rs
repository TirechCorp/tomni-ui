//! O teste real do M61: o `tomni-ui-zerotier` contra o `zerotier-one` oficial, em Docker. Fora
//! do CI. Rode com `testes/zerotier/rodar.sh`, que gera o planet de teste, sobe a rede
//! (`testes/zerotier/docker-compose.yml`), roda estes testes (`#[ignore]`) e derruba.
//!
//! A rede oficial: um `raiz` (o único do planet de teste), um `controlador` (a API local do
//! `zerotier-one`), o `servicos` (sshd, eco na porta 7 e gateway da rota para a rede interna) e o
//! `escondido` (só alcançável pelo raiz). O nó do teste roda no computador, entra no planet de
//! teste e abre as conexões pela pilha embutida, sem placa virtual. Cada teste cria a própria
//! rede pela API do controlador.

use std::io::Read;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tomni_ui_zerotier::node::{NodeState, State, ZeroTier, ZeroTierBuilder};
use tomni_ui_zerotier::{Address, Identity, NetworkId};

const TOKEN: &str = "tokenteste";
const IP_SERVICOS: &str = "10.61.0.2";
const IP_ESCONDIDO: &str = "10.61.0.3";
const IP_FIXO_CLIENTE: &str = "10.61.0.50";
/// Quanto o nó espera pela rede antes de desistir (a máquina pode estar carregada).
const PRAZO: Duration = Duration::from_secs(120);

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/identidades")
}

fn identidade(n: u32) -> Identity {
    let texto = std::fs::read_to_string(fixtures().join(format!("id{n}.secret"))).unwrap();
    Identity::from_secret_str(texto.trim()).unwrap()
}

fn planet() -> Vec<u8> {
    let caminho =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testes/zerotier/gerado/planet");
    std::fs::read(&caminho).unwrap_or_else(|e| {
        panic!("sem o planet de teste ({caminho:?}): {e}; rode testes/zerotier/rodar.sh")
    })
}

fn pasta_compose() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testes/zerotier")
}

/// Roda um comando num contêiner do compose e devolve a saída (falha o teste se o comando falhar).
fn exec(conteiner: &str, args: &[&str]) -> String {
    let saida = Command::new("docker")
        .arg("exec")
        .arg(conteiner)
        .args(args)
        .output()
        .expect("o docker");
    assert!(
        saida.status.success(),
        "docker exec {conteiner} {args:?}: {}{}",
        String::from_utf8_lossy(&saida.stdout),
        String::from_utf8_lossy(&saida.stderr)
    );
    String::from_utf8_lossy(&saida.stdout).into_owned()
}

/// Como [`exec`], mas devolve se o comando terminou com sucesso (para o que deve falhar).
fn exec_ok(conteiner: &str, args: &[&str]) -> bool {
    Command::new("docker")
        .arg("exec")
        .arg(conteiner)
        .args(args)
        .output()
        .expect("o docker")
        .status
        .success()
}

fn compose(args: &[&str]) {
    let status = Command::new("docker")
        .args(["compose", "--project-directory"])
        .arg(pasta_compose())
        .args(args)
        .status()
        .expect("o docker compose");
    assert!(status.success(), "docker compose {args:?}");
}

/// A API local do controlador (a do `zerotier-one`), de dentro do contêiner dele.
fn api(metodo: &str, caminho: &str, corpo: Option<&Value>) -> Value {
    let url = format!("http://127.0.0.1:9993{caminho}");
    let cabecalho = format!("X-ZT1-Auth: {TOKEN}");
    let mut args = vec!["curl", "-sS", "-X", metodo, "-H", &cabecalho];
    let texto = corpo.map(Value::to_string);
    if let Some(texto) = &texto {
        args.extend(["-H", "Content-Type: application/json", "-d", texto]);
    }
    args.push(&url);
    let saida = exec("zt-controlador", &args);
    serde_json::from_str(&saida).unwrap_or_else(|e| panic!("{caminho}: {e}: {saida:?}"))
}

fn controlador() -> Address {
    let texto = std::fs::read_to_string(fixtures().join("id4.public")).unwrap();
    Address::parse(texto.split(':').next().unwrap()).unwrap()
}

/// O endereço ZeroTier de um nó oficial.
fn endereco_de(conteiner: &str) -> Address {
    let texto = exec(conteiner, &["cat", "/var/lib/zerotier-one/identity.public"]);
    Address::parse(texto.split(':').next().unwrap().trim()).unwrap()
}

/// A rede do teste: criada no controlador, com o `servicos` e o `escondido` dentro.
struct Ambiente {
    id: NetworkId,
    servicos: Address,
    escondido: Address,
}

impl Ambiente {
    /// Cria a rede `sufixo` (6 dígitos hex) com as `regras`, entra com os nós oficiais e espera
    /// os dois terem o IP.
    fn nova(sufixo: &str, regras: &Value) -> Self {
        sair_de_todas();
        let texto = format!("{}{sufixo}", controlador());
        let id = NetworkId::parse(&texto).unwrap();
        api(
            "POST",
            &format!("/controller/network/{texto}"),
            Some(&json!({
                "name": format!("teste-{sufixo}"),
                "private": true,
                "enableBroadcast": true,
                "mtu": 2800,
                "v4AssignMode": {"zt": true},
                "ipAssignmentPools": [{"ipRangeStart": "10.61.0.100", "ipRangeEnd": "10.61.0.200"}],
                "routes": [
                    {"target": "10.61.0.0/24"},
                    {"target": "172.31.62.0/24", "via": IP_SERVICOS},
                ],
                "rules": regras,
            })),
        );
        let servicos = endereco_de("zt-servicos");
        let escondido = endereco_de("zt-escondido");
        let amb = Self {
            id,
            servicos,
            escondido,
        };
        amb.autorizar(servicos, Some(IP_SERVICOS));
        amb.autorizar(escondido, Some(IP_ESCONDIDO));
        exec("zt-servicos", &["zerotier-cli", "join", &texto]);
        exec("zt-escondido", &["zerotier-cli", "join", &texto]);
        for (conteiner, ip) in [("zt-servicos", IP_SERVICOS), ("zt-escondido", IP_ESCONDIDO)] {
            let inicio = Instant::now();
            loop {
                let redes = exec(conteiner, &["zerotier-cli", "listnetworks"]);
                if redes
                    .lines()
                    .any(|l| l.contains(&texto) && l.contains(" OK ") && l.contains(ip))
                {
                    break;
                }
                assert!(
                    inicio.elapsed() < Duration::from_secs(150),
                    "{conteiner} não entrou na rede {texto}: {redes}"
                );
                std::thread::sleep(Duration::from_secs(1));
            }
        }
        amb
    }

    fn texto(&self) -> String {
        self.id.to_string()
    }

    fn autorizar(&self, membro: Address, ip: Option<&str>) {
        let mut corpo = json!({"authorized": true});
        if let Some(ip) = ip {
            corpo["ipAssignments"] = json!([ip]);
        }
        api(
            "POST",
            &format!("/controller/network/{}/member/{membro}", self.texto()),
            Some(&corpo),
        );
    }

    fn mudar_regras(&self, regras: &Value) {
        api(
            "POST",
            &format!("/controller/network/{}", self.texto()),
            Some(&json!({"rules": regras})),
        );
    }
}

/// Congela o contêiner do controlador (`docker pause`) e o descongela ao sair, mesmo se o teste
/// falhar. Parar de vez (`stop`) mudaria as portas UDP dele e os nós oficiais levariam minutos
/// para achá-lo de novo, o que atrapalharia os testes seguintes.
struct ControladorParado;

impl ControladorParado {
    fn novo() -> Self {
        compose(&["pause", "controlador"]);
        Self
    }
}

impl Drop for ControladorParado {
    fn drop(&mut self) {
        compose(&["unpause", "controlador"]);
    }
}

impl Drop for Ambiente {
    /// Sai da rede nos dois nós oficiais, mesmo se o teste falhar: duas redes com a mesma faixa
    /// no `servicos` fariam as respostas sair pela placa errada.
    fn drop(&mut self) {
        sair_de_todas();
    }
}

/// Sai de todas as redes dos dois nós oficiais.
fn sair_de_todas() {
    for conteiner in ["zt-servicos", "zt-escondido"] {
        let redes = Command::new("docker")
            .args(["exec", conteiner, "zerotier-cli", "listnetworks"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        for linha in redes.lines().skip(1) {
            if let Some(id) = linha.split_whitespace().nth(2) {
                exec_ok(conteiner, &["zerotier-cli", "leave", id]);
            }
        }
    }
}

fn aceitar_tudo() -> Value {
    json!([{"type": "ACTION_ACCEPT"}])
}

/// "Descarta o TCP para a porta 7", e aceita o resto.
fn sem_porta_7() -> Value {
    json!([
        {"type": "MATCH_IP_PROTOCOL", "ipProtocol": 6},
        {"type": "MATCH_IP_DEST_PORT_RANGE", "start": 7, "end": 7},
        {"type": "ACTION_DROP"},
        {"type": "ACTION_ACCEPT"},
    ])
}

/// O nó do teste (a identidade `n` das fixtures), com o estado que o app guardaria.
fn cliente(amb: &Ambiente, n: u32, estado: Arc<Mutex<NodeState>>) -> ZeroTierBuilder {
    let mut inicial = estado.lock().unwrap().clone();
    if inicial.identity.is_none() {
        inicial.identity = Some(identidade(n));
    }
    ZeroTierBuilder::new(amb.id, inicial)
        .planet(planet())
        .on_node_state(move |novo| {
            let mut guardado = estado.lock().unwrap();
            let identidade = guardado.identity.clone();
            *guardado = novo.clone();
            if guardado.identity.is_none() {
                guardado.identity = identidade;
            }
        })
}

async fn conectar(zt: &ZeroTier, ip: &str, porta: u16) -> tomni_ui_netstack::TcpStream {
    let addr = format!("{ip}:{porta}").parse().unwrap();
    let inicio = Instant::now();
    loop {
        let pilha = zt.stack().expect("a pilha existe depois do Connected");
        match pilha.connect_timeout(addr, Duration::from_secs(10)).await {
            Ok(conexao) => return conexao,
            Err(erro) if inicio.elapsed() < Duration::from_secs(60) => {
                eprintln!("ainda sem {ip}:{porta}: {erro}");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Err(erro) => panic!("sem conexão com {ip}:{porta}: {erro}"),
        }
    }
}

/// Manda `tamanho` bytes ao eco e confere o SHA-256 do que volta.
async fn eco(zt: &ZeroTier, ip: &str, tamanho: usize) {
    let conexao = conectar(zt, ip, 7).await;
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

/// O banner do sshd, pelo proxy SOCKS5 que os clientes do app usam.
async fn banner(zt: &ZeroTier, ip: &str) -> String {
    let proxy = zt.proxy();
    let ip = ip.to_string();
    tokio::task::spawn_blocking(move || {
        let inicio = Instant::now();
        loop {
            match proxy.open(&ip, 22, Duration::from_secs(10)) {
                Ok(mut tcp) => {
                    let mut buf = [0u8; 64];
                    let n = tcp.read(&mut buf).unwrap();
                    return String::from_utf8_lossy(&buf[..n]).into_owned();
                }
                Err(erro) if inicio.elapsed() < Duration::from_secs(60) => {
                    eprintln!("ainda sem o sshd de {ip}: {erro}");
                    std::thread::sleep(Duration::from_secs(1));
                }
                Err(erro) => panic!("sem o sshd de {ip}: {erro}"),
            }
        }
    })
    .await
    .unwrap()
}

/// Conecta o nó do teste, autorizado antes, e espera o IP.
async fn entrar(amb: &Ambiente, n: u32, estado: Arc<Mutex<NodeState>>) -> (ZeroTier, Ipv4Addr) {
    amb.autorizar(identidade(n).address(), Some(IP_FIXO_CLIENTE));
    entrar_sem_autorizar(amb, n, estado).await
}

async fn entrar_sem_autorizar(
    amb: &Ambiente,
    n: u32,
    estado: Arc<Mutex<NodeState>>,
) -> (ZeroTier, Ipv4Addr) {
    let zt = cliente(amb, n, estado)
        .start()
        .await
        .expect("o nó não subiu");
    let ip = zt
        .wait_connected(PRAZO)
        .await
        .unwrap_or_else(|e| panic!("a rede oficial não conectou: {e}"));
    (zt, ip)
}

fn estado_novo() -> Arc<Mutex<NodeState>> {
    Arc::new(Mutex::new(NodeState::default()))
}

async fn caminho_de(zt: &ZeroTier, par: Address) -> Option<String> {
    zt.peers()
        .await
        .into_iter()
        .find(|p| p.address == par)
        .map(|p| p.path)
}

// 1. Entrar.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "precisa da rede oficial em Docker: testes/zerotier/rodar.sh"]
async fn entra_na_rede_oficial_com_o_membro_autorizado_antes() {
    let amb = Ambiente::nova("000001", &aceitar_tudo());
    let (zt, ip) = entrar(&amb, 2, estado_novo()).await;
    assert_eq!(ip.to_string(), IP_FIXO_CLIENTE);
    assert!(
        matches!(zt.state(), State::Connected { .. }),
        "{:?}",
        zt.state()
    );
    assert!(!zt.roots().await.is_empty());
    zt.stop(Duration::from_secs(5)).await;
}

// 2. Autorização depois.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "precisa da rede oficial em Docker: testes/zerotier/rodar.sh"]
async fn sem_autorizar_espera_e_ao_autorizar_conecta_sem_reiniciar() {
    let amb = Ambiente::nova("000002", &aceitar_tudo());
    let zt = cliente(&amb, 2, estado_novo())
        .start()
        .await
        .expect("o nó não subiu");
    let inicio = Instant::now();
    loop {
        match zt.state() {
            State::NotAuthorized { address } => {
                assert_eq!(address, identidade(2).address());
                break;
            }
            outro => assert!(
                inicio.elapsed() < PRAZO,
                "esperava NotAuthorized, está em {outro:?}"
            ),
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    amb.autorizar(identidade(2).address(), None);
    let ip = zt
        .wait_connected(PRAZO)
        .await
        .unwrap_or_else(|e| panic!("não conectou depois de autorizar: {e}"));
    assert_eq!(ip.octets()[..3], [10, 61, 0]);
    zt.stop(Duration::from_secs(5)).await;
}

// 3. O banner, o eco de 10 MiB e o caminho.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "precisa da rede oficial em Docker: testes/zerotier/rodar.sh"]
async fn banner_e_eco_de_10_mib_com_o_servicos() {
    let amb = Ambiente::nova("000003", &aceitar_tudo());
    let (zt, _ip) = entrar(&amb, 2, estado_novo()).await;
    let banner = banner(&zt, IP_SERVICOS).await;
    assert!(banner.starts_with("SSH-2.0-"), "{banner:?}");
    eco(&zt, IP_SERVICOS, 10 << 20).await;
    let caminho = caminho_de(&zt, amb.servicos)
        .await
        .expect("o servicos é um par");
    eprintln!("caminho até o servicos: {caminho}");
    // O servicos fala com o raiz oficial e com a gente na mesma rede Docker: o caminho vira direto.
    assert!(caminho.starts_with("direto"), "{caminho}");
    zt.stop(Duration::from_secs(5)).await;
}

// 4. Só pelo raiz.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "precisa da rede oficial em Docker: testes/zerotier/rodar.sh"]
async fn so_pelo_raiz_o_banner_e_o_eco_funcionam() {
    let amb = Ambiente::nova("000004", &aceitar_tudo());
    amb.autorizar(identidade(2).address(), Some(IP_FIXO_CLIENTE));
    let zt = cliente(&amb, 2, estado_novo())
        .direct_paths(false)
        .start()
        .await
        .expect("o nó não subiu");
    zt.wait_connected(PRAZO).await.unwrap();
    let banner = banner(&zt, IP_SERVICOS).await;
    assert!(banner.starts_with("SSH-2.0-"), "{banner:?}");
    eco(&zt, IP_SERVICOS, 2 << 20).await;
    zt.stop(Duration::from_secs(5)).await;
}

// 5. O escondido, só alcançável pelo raiz.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "precisa da rede oficial em Docker: testes/zerotier/rodar.sh"]
async fn o_escondido_so_responde_pelo_raiz() {
    // O computador alcança a rede `interna` pela ponte do Docker (o `internal: true` só impede
    // a saída dos contêineres), então o direto até o escondido existiria. `direct_paths(false)`
    // garante que ele só é alcançado pelo raiz, como numa rede de verdade com NAT.
    let amb = Ambiente::nova("000005", &aceitar_tudo());
    amb.autorizar(identidade(2).address(), Some(IP_FIXO_CLIENTE));
    let zt = cliente(&amb, 2, estado_novo())
        .direct_paths(false)
        .start()
        .await
        .expect("o nó não subiu");
    zt.wait_connected(PRAZO).await.unwrap();
    let banner = banner(&zt, IP_ESCONDIDO).await;
    assert!(banner.starts_with("SSH-2.0-"), "{banner:?}");
    eco(&zt, IP_ESCONDIDO, 1 << 20).await;
    assert_eq!(
        caminho_de(&zt, amb.escondido).await.as_deref(),
        Some("pelo raiz"),
        "o escondido não é alcançável fora do raiz"
    );
    zt.stop(Duration::from_secs(5)).await;
}

// 6. As regras da rede.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "precisa da rede oficial em Docker: testes/zerotier/rodar.sh"]
async fn as_regras_da_rede_calam_o_eco_e_deixam_o_ssh() {
    let amb = Ambiente::nova("000006", &aceitar_tudo());
    let (zt, _ip) = entrar(&amb, 2, estado_novo()).await;
    eco(&zt, IP_SERVICOS, 64 * 1024).await;

    amb.mudar_regras(&sem_porta_7());
    let addr = format!("{IP_SERVICOS}:7").parse().unwrap();
    let inicio = Instant::now();
    loop {
        let pilha = zt.stack().unwrap();
        if pilha
            .connect_timeout(addr, Duration::from_secs(3))
            .await
            .is_err()
        {
            break;
        }
        assert!(
            inicio.elapsed() < Duration::from_secs(180),
            "o eco continuou respondendo depois de a rede descartar a porta 7"
        );
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    let banner = banner(&zt, IP_SERVICOS).await;
    assert!(banner.starts_with("SSH-2.0-"), "{banner:?}");
    zt.stop(Duration::from_secs(5)).await;
}

// 7. A rota com gateway.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "precisa da rede oficial em Docker: testes/zerotier/rodar.sh"]
async fn a_rota_com_gateway_leva_ao_sshd_da_rede_interna() {
    let amb = Ambiente::nova("000007", &aceitar_tudo());
    let (zt, _ip) = entrar(&amb, 2, estado_novo()).await;
    // O escondido tem também o IP da rede interna (172.31.62.20): só o gateway chega lá.
    let mut conexao = conectar(&zt, "172.31.62.20", 22).await;
    let mut buf = [0u8; 64];
    let n = tokio::time::timeout(Duration::from_secs(20), conexao.read(&mut buf))
        .await
        .expect("o banner não veio")
        .unwrap();
    assert!(String::from_utf8_lossy(&buf[..n]).starts_with("SSH-2.0-"));
    zt.stop(Duration::from_secs(5)).await;
}

// 8. Reiniciar com o mesmo estado, com o controlador parado.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "precisa da rede oficial em Docker: testes/zerotier/rodar.sh"]
async fn reinicia_com_o_mesmo_estado_e_sobe_sem_o_controlador() {
    let amb = Ambiente::nova("000008", &aceitar_tudo());
    let estado = estado_novo();
    let (zt, ip) = entrar(&amb, 2, estado.clone()).await;
    let endereco = zt.address();
    eco(&zt, IP_SERVICOS, 64 * 1024).await;
    zt.stop(Duration::from_secs(5)).await;
    {
        let guardado = estado.lock().unwrap();
        assert!(
            !guardado.network_config.is_empty(),
            "a configuração não foi guardada"
        );
        assert!(guardado.identity.is_some());
    }

    // O mesmo estado: o mesmo endereço e o mesmo IP.
    let (zt, ip2) = entrar_sem_autorizar(&amb, 2, estado.clone()).await;
    assert_eq!(zt.address(), endereco);
    assert_eq!(ip2, ip);
    zt.stop(Duration::from_secs(5)).await;

    // Sem o controlador, sobe com a configuração guardada.
    let _parado = ControladorParado::novo();
    let (zt, ip3) = entrar_sem_autorizar(&amb, 2, estado.clone()).await;
    assert_eq!(ip3, ip);
    eco(&zt, IP_SERVICOS, 64 * 1024).await;
    zt.stop(Duration::from_secs(5)).await;
}

// 9. Uma sessão SSH de verdade.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "precisa da rede oficial em Docker: testes/zerotier/rodar.sh"]
async fn ssh_de_verdade_pela_ponte_da_rede() {
    use tomni_ui_terminal::ssh::SshBuilder;

    let amb = Ambiente::nova("000009", &aceitar_tudo());
    let (zt, _ip) = entrar(&amb, 2, estado_novo()).await;
    let proxy = zt.proxy();
    let saida = tokio::task::spawn_blocking(move || {
        let inicio = Instant::now();
        loop {
            let resultado = SshBuilder::new(IP_SERVICOS, 22, "teste")
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
    zt.stop(Duration::from_secs(5)).await;
}

// 10. Invisível: ninguém da rede alcança o nosso IP.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "precisa da rede oficial em Docker: testes/zerotier/rodar.sh"]
async fn invisivel_o_servicos_nao_alcanca_o_ip_do_cliente() {
    let amb = Ambiente::nova("00000a", &aceitar_tudo());
    let (zt, ip) = entrar(&amb, 2, estado_novo()).await;
    // O tráfego do cliente para o servicos já passou: o servicos conhece o nosso MAC.
    eco(&zt, IP_SERVICOS, 16 * 1024).await;
    let ip = ip.to_string();
    assert!(
        !exec_ok("zt-servicos", &["nc", "-z", "-w", "3", &ip, "22"]),
        "o servicos conectou na porta 22 do cliente"
    );
    assert!(
        !exec_ok("zt-servicos", &["ping", "-c", "1", "-W", "2", &ip]),
        "o servicos recebeu resposta de ping do cliente"
    );
    zt.stop(Duration::from_secs(5)).await;
}

// 11. Moons: orbitar a do `lua` põe o `lua` entre os raiz.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "precisa da rede oficial em Docker: testes/zerotier/rodar.sh"]
async fn orbitar_a_moon_do_lua_poe_o_lua_entre_os_raiz() {
    use tomni_ui_zerotier::peer::PeerRole;

    let amb = Ambiente::nova("00000b", &aceitar_tudo());
    amb.autorizar(identidade(2).address(), Some(IP_FIXO_CLIENTE));
    // O `lua` gera a moon dele ao subir: o id é o endereço dele.
    let lua = endereco_de("zt-lua");
    let estado = estado_novo();
    let zt = cliente(&amb, 2, estado.clone())
        .orbit(lua.as_u64(), lua)
        .start()
        .await
        .expect("o nó não subiu");
    zt.wait_connected(PRAZO).await.unwrap();
    let inicio = Instant::now();
    loop {
        let raizes = zt.roots().await;
        if raizes
            .iter()
            .any(|r| r.address == lua && r.role == PeerRole::Moon)
        {
            break;
        }
        assert!(
            inicio.elapsed() < Duration::from_secs(90),
            "o lua não virou raiz de moon: {raizes:?}"
        );
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    zt.stop(Duration::from_secs(5)).await;
    assert_eq!(
        estado.lock().unwrap().moons.len(),
        1,
        "a moon não foi guardada"
    );
}

// 12. As duas cifras: o protocolo anunciado 11 (Salsa20/12) e o padrão (AES-GMAC-SIV).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "precisa da rede oficial em Docker: testes/zerotier/rodar.sh"]
async fn eco_de_1_mib_com_salsa_e_com_aes() {
    let amb = Ambiente::nova("00000c", &aceitar_tudo());
    amb.autorizar(identidade(2).address(), Some(IP_FIXO_CLIENTE));
    for protocolo in [Some(11u8), None] {
        let mut builder = cliente(&amb, 2, estado_novo());
        if let Some(p) = protocolo {
            builder = builder.protocol_version(p);
        }
        let zt = builder.start().await.expect("o nó não subiu");
        zt.wait_connected(PRAZO).await.unwrap();
        eco(&zt, IP_SERVICOS, 1 << 20).await;
        eprintln!("protocolo {protocolo:?}: eco de 1 MiB confere");
        zt.stop(Duration::from_secs(5)).await;
    }
}
