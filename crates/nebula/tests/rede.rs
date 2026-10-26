//! A rede inteira em memória: um lighthouse e nós atrás de NAT, sobre o UDP simulado. Os pacotes
//! passam pelo aperto de mão, pelo lighthouse, pelo punch e pelo túnel de verdade; só o UDP é
//! de mentira.

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tomni_ui_nebula::{
    Config, NatKind, Nebula, NebulaBuilder, SimEndpoint, SimNet, State, TunnelInfo,
};

const LIGHTHOUSE_IP: &str = "10.42.0.1";
const LIGHTHOUSE_PUBLICO: &str = "203.0.113.1:4242";

/// Prazo de cada teste: uma falha precisa aparecer como falha, e não como um teste que não acaba.
const PRAZO_DO_TESTE: Duration = Duration::from_secs(120);

/// Roda o corpo de um teste e falha, dizendo isso, se passar de [`PRAZO_DO_TESTE`].
async fn prazo(corpo: impl std::future::Future<Output = ()>) {
    if tokio::time::timeout(PRAZO_DO_TESTE, corpo).await.is_err() {
        panic!("o teste passou de {PRAZO_DO_TESTE:?} sem terminar");
    }
}

fn pasta() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// O config de um nó com as fixtures de `no` (`<no>.crt`/`.key`), na CA `ca`.
fn config_com(no: &str, ca: &str, extra: &str) -> Config {
    let yaml = format!(
        "pki:\n  ca: {ca}.crt\n  cert: {no}.crt\n  key: {no}.key\n\
         static_host_map:\n  \"{LIGHTHOUSE_IP}\": [\"{LIGHTHOUSE_PUBLICO}\"]\n\
         lighthouse:\n  interval: 5\n  hosts: [\"{LIGHTHOUSE_IP}\"]\n\
         punchy:\n  punch: true\n  respond: true\n  delay: 100ms\n  respond_delay: 100ms\n\
         handshakes:\n  try_interval: 100ms\n  retries: 10\n{extra}"
    );
    Config::from_yaml(&yaml, Some(&pasta()))
        .expect("o config de teste")
        .0
}

fn config(no: &str) -> Config {
    config_com(no, "ca-25519", "")
}

async fn lighthouse(rede: &SimNet) -> Nebula {
    let (ip, porta) = LIGHTHOUSE_PUBLICO.rsplit_once(':').unwrap();
    let ponto = rede.host(ip, porta.parse().unwrap());
    let yaml = "pki:\n  ca: ca-25519.crt\n  cert: lighthouse.crt\n  key: lighthouse.key\n\
                punchy:\n  punch: true\n";
    let (config, _) = Config::from_yaml(yaml, Some(&pasta())).unwrap();
    NebulaBuilder::new(config)
        .as_lighthouse()
        .start_on(ponto)
        .await
        .unwrap()
}

async fn no(config: Config, ponto: SimEndpoint) -> Nebula {
    NebulaBuilder::new(config).start_on(ponto).await.unwrap()
}

fn ip(texto: &str) -> IpAddr {
    texto.parse().unwrap()
}

async fn conectado(nebula: &Nebula) {
    nebula
        .wait_connected(Duration::from_secs(10))
        .await
        .unwrap_or_else(|e| panic!("não conectou: {e}"));
}

/// Espera até `condicao` valer, olhando os túneis do nó a cada 50 ms.
async fn esperar_tunel(
    nebula: &Nebula,
    condicao: impl Fn(&[TunnelInfo]) -> bool,
) -> Vec<TunnelInfo> {
    for _ in 0..400 {
        let tuneis = nebula.tunnels().await;
        if condicao(&tuneis) {
            return tuneis;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!(
        "os túneis não chegaram ao esperado: {:?}",
        nebula.tunnels().await
    );
}

/// Sobe um servidor de eco na porta 7 do nó.
fn eco(nebula: &Nebula) {
    let stack = nebula.stack().clone();
    tokio::spawn(async move {
        let mut escuta = stack.listen(7).await.unwrap();
        loop {
            let Ok(mut conexao) = escuta.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buf = vec![0u8; 16 * 1024];
                loop {
                    match conexao.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => {
                            if conexao.write_all(&buf[..n]).await.is_err() {
                                return;
                            }
                        }
                    }
                }
            });
        }
    });
}

/// Manda `tamanho` bytes e confere que voltam iguais.
async fn conferir_eco(nebula: &Nebula, destino: &str, tamanho: usize) {
    let addr: SocketAddr = format!("{destino}:7").parse().unwrap();
    let conexao = nebula
        .stack()
        .connect_timeout(addr, Duration::from_secs(20))
        .await
        .unwrap_or_else(|e| panic!("sem conexão com {destino}: {e}"));
    let (mut le, mut escreve) = tokio::io::split(conexao);
    let dados: Vec<u8> = (0..tamanho)
        .map(|i| (i.wrapping_mul(31) % 251) as u8)
        .collect();
    let envio = dados.clone();
    let escritor = tokio::spawn(async move {
        escreve.write_all(&envio).await.unwrap();
        escreve
    });
    let mut volta = vec![0u8; tamanho];
    tokio::time::timeout(Duration::from_secs(60), le.read_exact(&mut volta))
        .await
        .expect("o eco demorou demais")
        .unwrap();
    assert!(volta == dados, "o eco voltou diferente");
    let _ = escritor.await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dois_nos_atras_de_nat_falam_direto_pelo_punch() {
    prazo(async {
        let rede = SimNet::new();
        let lh = lighthouse(&rede).await;
        let nat_a = rede.nat("198.51.100.1", NatKind::FullCone);
        let nat_b = rede.nat("198.51.100.2", NatKind::FullCone);
        let a = no(config("no-v2"), nat_a.endpoint("192.168.1.10", 4242)).await;
        let b = no(config("no-v2-b"), nat_b.endpoint("192.168.2.10", 4242)).await;
        conectado(&a).await;
        conectado(&b).await;
        eco(&a);

        conferir_eco(&b, "10.42.0.20", 1 << 20).await;

        let tuneis = esperar_tunel(&b, |t| t.iter().any(|x| x.vpn_addr == ip("10.42.0.20"))).await;
        let ate_a = tuneis
            .iter()
            .find(|t| t.vpn_addr == ip("10.42.0.20"))
            .unwrap();
        assert_eq!(ate_a.name, "no-v2");
        assert!(!ate_a.lighthouse);
        let remoto = ate_a.remote.expect("o túnel tem endereço");
        assert_eq!(
            remoto.ip(),
            ip("198.51.100.1"),
            "direto pelo NAT do A, não pelo lighthouse"
        );
        assert!(matches!(b.state(), State::Connected { lighthouses: 1, .. }));

        b.stop().await;
        a.stop().await;
        lh.stop().await;
    })
    .await;
}

/// Sobe o lighthouse e dois nós (A em 10.42.0.20 com eco, B em 10.42.0.42), cada um atrás de um
/// NAT `tipo`. Os certificados e extras vêm de fora.
struct Par {
    rede: SimNet,
    lh: Nebula,
    a: Nebula,
    b: Nebula,
}

async fn par(no_a: &str, no_b: &str, extra: &str, tipo: NatKind) -> Par {
    let rede = SimNet::new();
    let lh = lighthouse_com(&rede, extra).await;
    let nat_a = rede.nat("198.51.100.1", tipo);
    let nat_b = rede.nat("198.51.100.2", tipo);
    let a = no(
        config_com(no_a, "ca-25519", extra),
        nat_a.endpoint("192.168.1.10", 4242),
    )
    .await;
    let b = no(
        config_com(no_b, "ca-25519", extra),
        nat_b.endpoint("192.168.2.10", 4242),
    )
    .await;
    conectado(&a).await;
    conectado(&b).await;
    eco(&a);
    Par { rede, lh, a, b }
}

async fn lighthouse_com(rede: &SimNet, extra: &str) -> Nebula {
    let (ip, porta) = LIGHTHOUSE_PUBLICO.rsplit_once(':').unwrap();
    let ponto = rede.host(ip, porta.parse().unwrap());
    let yaml = format!(
        "pki:\n  ca: ca-25519.crt\n  cert: lighthouse.crt\n  key: lighthouse.key\n\
         punchy:\n  punch: true\n{extra}"
    );
    let (config, _) = Config::from_yaml(&yaml, Some(&pasta())).unwrap();
    NebulaBuilder::new(config)
        .as_lighthouse()
        .start_on(ponto)
        .await
        .unwrap()
}

impl Par {
    async fn parar(self) {
        self.b.stop().await;
        self.a.stop().await;
        self.lh.stop().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn certificados_v1_e_v2_se_entendem() {
    prazo(async {
        for (a, b) in [
            ("no-v1", "no-v2-b"),
            ("no-v2", "no-v1-b"),
            ("no-v1", "no-v1-b"),
            ("no-v1v2", "no-v1v2-b"),
            ("no-v1v2", "no-v1-b"),
        ] {
            let p = par(a, b, "", NatKind::FullCone).await;
            let alvo = match a {
                "no-v1" => "10.42.0.21",
                "no-v2" => "10.42.0.20",
                _ => "10.42.0.22",
            };
            conferir_eco(&p.b, alvo, 64 * 1024).await;
            p.parar().await;
        }
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chachapoly_na_rede_inteira() {
    prazo(async {
        let p = par(
            "no-v2",
            "no-v2-b",
            "cipher: chachapoly\n",
            NatKind::FullCone,
        )
        .await;
        conferir_eco(&p.b, "10.42.0.20", 256 * 1024).await;
        p.parar().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nat_restrito_tambem_abre_pelo_punch() {
    prazo(async {
        let p = par("no-v2", "no-v2-b", "", NatKind::Restricted).await;
        conferir_eco(&p.b, "10.42.0.20", 256 * 1024).await;
        p.parar().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cifras_diferentes_falham_com_mensagem_clara() {
    prazo(async {
        // Sem lighthouse: o B conhece o endereço do A e tenta direto, mas as cifras não batem.
        let rede = SimNet::new();
        let ponto_a = rede.host("203.0.113.10", 4242);
        let ponto_b = rede.host("203.0.113.20", 4242);
        let yaml_a = "pki:\n  ca: ca-25519.crt\n  cert: no-v2.crt\n  key: no-v2.key\n\
                      static_host_map:\n  \"10.42.0.42\": [\"203.0.113.20:4242\"]\ncipher: chachapoly\n";
        let yaml_b = "pki:\n  ca: ca-25519.crt\n  cert: no-v2-b.crt\n  key: no-v2-b.key\n\
                      static_host_map:\n  \"10.42.0.20\": [\"203.0.113.10:4242\"]\n\
                      handshakes:\n  try_interval: 50ms\n  retries: 4\n";
        let a = no(
            Config::from_yaml(yaml_a, Some(&pasta())).unwrap().0,
            ponto_a,
        )
        .await;
        let b = no(
            Config::from_yaml(yaml_b, Some(&pasta())).unwrap().0,
            ponto_b,
        )
        .await;
        let erro = b
            .stack()
            .connect_timeout("10.42.0.20:7".parse().unwrap(), Duration::from_secs(30))
            .await
            .expect_err("cifras diferentes não conectam");
        let texto = erro.to_string();
        assert!(texto.contains("10.42.0.20"), "{texto}");
        assert!(texto.contains("cipher"), "{texto}");
        b.stop().await;
        a.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn certificado_de_outra_ca_e_recusado_na_partida() {
    prazo(async {
        let rede = SimNet::new();
        let yaml = "pki:\n  ca: ca-25519.crt\n  cert: no-outra-ca.crt\n  key: no-outra-ca.key\n\
                    static_host_map:\n  \"10.42.0.1\": [\"203.0.113.1:4242\"]\n\
                    lighthouse:\n  hosts: [\"10.42.0.1\"]\n";
        let (cfg, _) = Config::from_yaml(yaml, Some(&pasta())).unwrap();
        let nat = rede.nat("198.51.100.9", NatKind::FullCone);
        let erro = NebulaBuilder::new(cfg)
            .start_on(nat.endpoint("192.168.9.10", 4242))
            .await
            .expect_err("a CA não confere")
            .to_string();
        assert!(erro.contains("CA"), "{erro}");
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn quem_para_avisa_o_par_com_close_tunnel() {
    prazo(async {
        let p = par("no-v2", "no-v2-b", "", NatKind::FullCone).await;
        conferir_eco(&p.b, "10.42.0.20", 4096).await;
        esperar_tunel(&p.b, |t| t.iter().any(|x| x.vpn_addr == ip("10.42.0.20"))).await;
        esperar_tunel(&p.a, |t| t.iter().any(|x| x.vpn_addr == ip("10.42.0.42"))).await;
        let Par { rede, lh, a, b } = p;
        a.stop().await;
        esperar_tunel(&b, |t| !t.iter().any(|x| x.vpn_addr == ip("10.42.0.20"))).await;
        let _ = rede;
        b.stop().await;
        lh.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn o_nat_trocou_de_porta_e_o_tunel_continua() {
    prazo(async {
        let rede = SimNet::new();
        let lh = lighthouse(&rede).await;
        let nat_a = rede.nat("198.51.100.1", NatKind::FullCone);
        let nat_b = rede.nat("198.51.100.2", NatKind::FullCone);
        let a = no(config("no-v2"), nat_a.endpoint("192.168.1.10", 4242)).await;
        let b = no(config("no-v2-b"), nat_b.endpoint("192.168.2.10", 4242)).await;
        conectado(&a).await;
        conectado(&b).await;
        eco(&a);
        conferir_eco(&b, "10.42.0.20", 64 * 1024).await;
        // O roteador do A reiniciou e o mapeamento mudou de porta.
        nat_a.rebind();
        conferir_eco(&b, "10.42.0.20", 64 * 1024).await;
        b.stop().await;
        a.stop().await;
        lh.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn um_no_que_reinicia_refaz_o_tunel_sozinho() {
    prazo(async {
        let rede = SimNet::new();
        let lh = lighthouse(&rede).await;
        let nat_a = rede.nat("198.51.100.1", NatKind::FullCone);
        let nat_b = rede.nat("198.51.100.2", NatKind::FullCone);
        let ponto_a = nat_a.endpoint("192.168.1.10", 4242);
        let b = no(config("no-v2-b"), nat_b.endpoint("192.168.2.10", 4242)).await;
        let a = no(config("no-v2"), ponto_a).await;
        conectado(&a).await;
        conectado(&b).await;
        eco(&a);
        conferir_eco(&b, "10.42.0.20", 64 * 1024).await;
        // O A "cai" sem avisar (Drop do SimEndpoint) e volta com a memória zerada.
        let _ = a.udp_addr();
        drop(a);
        let nat_a2 = nat_a;
        let a2 = no(config("no-v2"), nat_a2.endpoint("192.168.1.10", 4242)).await;
        conectado(&a2).await;
        eco(&a2);
        conferir_eco(&b, "10.42.0.20", 64 * 1024).await;
        b.stop().await;
        a2.stop().await;
        lh.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn os_dois_lados_abrem_ao_mesmo_tempo() {
    prazo(async {
        let p = par("no-v2", "no-v2-b", "", NatKind::FullCone).await;
        eco(&p.b);
        let (x, y) = tokio::join!(
            conferir_eco(&p.a, "10.42.0.42", 64 * 1024),
            conferir_eco(&p.b, "10.42.0.20", 64 * 1024)
        );
        let _ = (x, y);
        p.parar().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sem_lighthouse_o_estado_diz_o_que_acontece() {
    prazo(async {
        let rede = SimNet::new();
        let nat = rede.nat("198.51.100.1", NatKind::FullCone);
        let yaml = format!(
            "pki:\n  ca: ca-25519.crt\n  cert: no-v2.crt\n  key: no-v2.key\n\
             static_host_map:\n  \"{LIGHTHOUSE_IP}\": [\"{LIGHTHOUSE_PUBLICO}\"]\n\
             lighthouse:\n  hosts: [\"{LIGHTHOUSE_IP}\"]\n\
             handshakes:\n  try_interval: 50ms\n  retries: 3\n"
        );
        let (cfg, _) = Config::from_yaml(&yaml, Some(&pasta())).unwrap();
        let a = no(cfg, nat.endpoint("192.168.1.10", 4242)).await;
        let erro = a
            .wait_connected(Duration::from_secs(3))
            .await
            .expect_err("ninguém responde");
        let texto = erro.to_string();
        assert!(texto.contains("10.42.0.1"), "{texto}");
        assert!(matches!(a.state(), State::Retrying(_)), "{:?}", a.state());
        a.stop().await;
    })
    .await;
}
