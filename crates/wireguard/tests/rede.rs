//! Dois (ou três) motores WireGuard sobre o UDP simulado do `tomni-ui-netstack`: o handshake, o
//! transporte e a pilha TCP são de verdade; só o UDP é de mentira.

use std::io::{Read, Write};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tomni_ui_netstack::{NatKind, SimEndpoint, SimNet};
use tomni_ui_wireguard::{PresharedKey, PrivateKey, State, WgConfig, WireGuard, WireGuardBuilder};

const IP_A: &str = "10.66.0.2";
const IP_B: &str = "10.66.0.1";

struct Par {
    privada: PrivateKey,
}

impl Par {
    fn novo() -> Par {
        Par {
            privada: PrivateKey::generate().unwrap(),
        }
    }

    fn publica(&self) -> String {
        self.privada.public_key().to_base64()
    }
}

/// O `.conf` de um nó: `meu_ip`, o par com `endpoint` e as faixas que ele atende.
fn conf(
    eu: &Par,
    meu_ip: &str,
    outro: &Par,
    endpoint: &str,
    faixas: &str,
    psk: Option<&PresharedKey>,
    extra: &str,
) -> WgConfig {
    let psk = psk.map_or(String::new(), |p| {
        format!("PresharedKey = {}\n", p.to_base64().as_str())
    });
    let texto = format!(
        "[Interface]\nPrivateKey = {}\nAddress = {meu_ip}/32\n{extra}\n\
         [Peer]\nPublicKey = {}\n{psk}Endpoint = {endpoint}\nAllowedIPs = {faixas}\n",
        eu.privada.to_base64().as_str(),
        outro.publica(),
    );
    WgConfig::from_conf(&texto).expect("o .conf de teste").0
}

async fn iniciar(config: WgConfig, ponto: SimEndpoint) -> WireGuard {
    WireGuardBuilder::new(config)
        .start_on(ponto)
        .await
        .expect("o túnel sobe")
}

async fn conectado(wg: &WireGuard) {
    wg.wait_connected(Duration::from_secs(15))
        .await
        .unwrap_or_else(|e| panic!("não conectou: {e}"));
}

/// Sobe um servidor de eco na porta 7 da pilha do nó.
fn eco(wg: &WireGuard) {
    let stack = wg.stack().clone();
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

fn dados(tamanho: usize) -> Vec<u8> {
    (0..tamanho)
        .map(|i| (i.wrapping_mul(31) % 251) as u8)
        .collect()
}

/// Manda `tamanho` bytes pela pilha e confere que o eco volta igual.
async fn conferir_eco(wg: &WireGuard, destino: &str, tamanho: usize) {
    let addr: SocketAddr = format!("{destino}:7").parse().unwrap();
    let conexao = wg
        .stack()
        .connect_timeout(addr, Duration::from_secs(20))
        .await
        .unwrap_or_else(|e| panic!("sem conexão com {destino}: {e}"));
    let (mut le, mut escreve) = tokio::io::split(conexao);
    let envio = dados(tamanho);
    let a_enviar = envio.clone();
    let escritor = tokio::spawn(async move {
        escreve.write_all(&a_enviar).await.unwrap();
        escreve
    });
    let mut volta = vec![0u8; tamanho];
    tokio::time::timeout(Duration::from_secs(60), le.read_exact(&mut volta))
        .await
        .expect("o eco demorou demais")
        .unwrap();
    assert!(volta == envio, "o eco voltou diferente");
    let _ = escritor.await;
}

/// A e B em hosts públicos, com o eco em B.
async fn dupla(
    rede: &SimNet,
    psk: Option<&PresharedKey>,
    extra_a: &str,
) -> (WireGuard, WireGuard, Par, Par) {
    let (a, b) = (Par::novo(), Par::novo());
    let ponto_a = rede.host("203.0.113.1", 51820);
    let ponto_b = rede.host("203.0.113.2", 51820);
    let conf_a = conf(
        &a,
        IP_A,
        &b,
        "203.0.113.2:51820",
        "10.66.0.1/32",
        psk,
        extra_a,
    );
    let conf_b = conf(&b, IP_B, &a, "203.0.113.1:51820", "10.66.0.2/32", psk, "");
    let wb = iniciar(conf_b, ponto_b).await;
    let wa = iniciar(conf_a, ponto_a).await;
    eco(&wb);
    (wa, wb, a, b)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dois_nos_conectam_e_o_estado_diz_quantos_pares() {
    let rede = SimNet::new();
    let (a, b, _, _) = dupla(&rede, None, "").await;
    conectado(&a).await;
    conectado(&b).await;
    assert_eq!(
        a.state(),
        State::Connected {
            address: IP_A.parse().unwrap(),
            peers_up: 1,
            peers: 1
        }
    );
    assert_eq!(a.state().address(), Some(IP_A.parse().unwrap()));
    assert_eq!(a.addresses().len(), 1);
    assert_eq!(a.routes().len(), 1);
    a.stop(Duration::from_secs(5)).await;
    b.stop(Duration::from_secs(5)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn eco_tcp_de_1_mib_pela_pilha() {
    let rede = SimNet::new();
    let (a, b, _, _) = dupla(&rede, None, "").await;
    conectado(&a).await;
    conferir_eco(&a, IP_B, 1024 * 1024).await;
    let pares = a.peers().await;
    assert_eq!(pares.len(), 1);
    assert!(pares[0].connected);
    assert!(pares[0].tx_bytes >= 1024 * 1024);
    assert!(pares[0].rx_bytes >= 1024 * 1024);
    assert_eq!(
        pares[0].endpoint,
        Some("203.0.113.2:51820".parse().unwrap())
    );
    a.stop(Duration::from_secs(5)).await;
    b.stop(Duration::from_secs(5)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn eco_pela_ponte_socks_com_o_cliente_da_lib() {
    let rede = SimNet::new();
    let (a, b, _, _) = dupla(&rede, None, "").await;
    conectado(&a).await;
    let proxy = a.proxy();
    let enviado = dados(300 * 1024);
    let esperado = enviado.clone();
    let volta = tokio::task::spawn_blocking(move || {
        let mut tcp = proxy
            .open(IP_B, 7, Duration::from_secs(20))
            .expect("o SOCKS5 abre a conexão");
        tcp.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
        let mut escrita = tcp.try_clone().unwrap();
        let escritor = std::thread::spawn(move || {
            escrita.write_all(&enviado).unwrap();
        });
        let mut volta = vec![0u8; esperado.len()];
        tcp.read_exact(&mut volta).unwrap();
        escritor.join().unwrap();
        (volta, esperado)
    })
    .await
    .unwrap();
    assert!(volta.0 == volta.1, "o eco voltou diferente");
    a.stop(Duration::from_secs(5)).await;
    b.stop(Duration::from_secs(5)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_ponte_so_escuta_no_loopback_com_credencial() {
    let rede = SimNet::new();
    let (a, b, _, _) = dupla(&rede, None, "").await;
    let acesso = a.socks();
    assert!(acesso.addr.ip().is_loopback());
    assert_ne!(acesso.addr.port(), 0);
    assert!(!acesso.user.is_empty() && !acesso.password.is_empty());
    a.stop(Duration::from_secs(5)).await;
    b.stop(Duration::from_secs(5)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn com_chave_pre_compartilhada() {
    let rede = SimNet::new();
    let psk = PresharedKey::generate().unwrap();
    let (a, b, _, _) = dupla(&rede, Some(&psk), "").await;
    conectado(&a).await;
    conferir_eco(&a, IP_B, 64 * 1024).await;
    a.stop(Duration::from_secs(5)).await;
    b.stop(Duration::from_secs(5)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn psk_diferente_nao_conecta() {
    let rede = SimNet::new();
    let (a, b) = (Par::novo(), Par::novo());
    let psk_a = PresharedKey::generate().unwrap();
    let psk_b = PresharedKey::generate().unwrap();
    let pa = rede.host("203.0.113.1", 51820);
    let pb = rede.host("203.0.113.2", 51820);
    let wa = iniciar(
        conf(
            &a,
            IP_A,
            &b,
            "203.0.113.2:51820",
            "10.66.0.1/32",
            Some(&psk_a),
            "",
        ),
        pa,
    )
    .await;
    let wb = iniciar(
        conf(
            &b,
            IP_B,
            &a,
            "203.0.113.1:51820",
            "10.66.0.2/32",
            Some(&psk_b),
            "",
        ),
        pb,
    )
    .await;
    assert!(
        wa.wait_connected(Duration::from_millis(1500))
            .await
            .is_err()
    );
    assert!(wb.wait_connected(Duration::from_millis(500)).await.is_err());
    wa.stop(Duration::from_secs(5)).await;
    wb.stop(Duration::from_secs(5)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn destino_fora_dos_allowed_ips_e_recusado_na_hora() {
    let rede = SimNet::new();
    let (a, b, _, _) = dupla(&rede, None, "").await;
    conectado(&a).await;
    let erro = a
        .stack()
        .connect_timeout("10.99.0.1:80".parse().unwrap(), Duration::from_secs(5))
        .await
        .unwrap_err();
    assert_eq!(erro.kind(), std::io::ErrorKind::NetworkUnreachable);
    assert!(erro.to_string().contains("fora das redes do túnel"));
    a.stop(Duration::from_secs(5)).await;
    b.stop(Duration::from_secs(5)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn iniciacao_de_quem_nao_e_par_e_descartada_em_silencio() {
    let rede = SimNet::new();
    let (a, b, _, par_b) = dupla(&rede, None, "").await;
    conectado(&a).await;
    // Um intruso conhece a chave pública de B, mas B não o tem como par.
    let intruso = Par::novo();
    let ponto_c = rede.host("203.0.113.9", 51820);
    let c = iniciar(
        conf(
            &intruso,
            "10.66.0.9",
            &par_b,
            "203.0.113.2:51820",
            "10.66.0.1/32",
            None,
            "",
        ),
        ponto_c,
    )
    .await;
    assert!(c.wait_connected(Duration::from_millis(1500)).await.is_err());
    assert!(matches!(c.state(), State::Connecting));
    // B segue só com A.
    let pares = b.peers().await;
    assert_eq!(pares.len(), 1);
    assert_eq!(
        b.state(),
        State::Connected {
            address: IP_B.parse().unwrap(),
            peers_up: 1,
            peers: 1
        }
    );
    c.stop(Duration::from_secs(5)).await;
    a.stop(Duration::from_secs(5)).await;
    b.stop(Duration::from_secs(5)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn o_par_muda_de_endereco_no_meio_roaming() {
    let rede = SimNet::new();
    let nat = rede.nat("198.51.100.1", NatKind::FullCone);
    let (a, b) = (Par::novo(), Par::novo());
    let ponto_a = nat.endpoint("192.168.1.10", 51820);
    let ponto_b = rede.host("203.0.113.2", 51820);
    // B não sabe onde A está (atrás do NAT): o endereço é só um marcador.
    let wb = iniciar(
        conf(&b, IP_B, &a, "203.0.113.99:1", "10.66.0.2/32", None, ""),
        ponto_b,
    )
    .await;
    let wa = iniciar(
        conf(&a, IP_A, &b, "203.0.113.2:51820", "10.66.0.1/32", None, ""),
        ponto_a,
    )
    .await;
    eco(&wb);
    conectado(&wa).await;
    conferir_eco(&wa, IP_B, 64 * 1024).await;
    let antes = wb.peers().await[0].endpoint.unwrap();
    assert_eq!(antes.ip().to_string(), "198.51.100.1");
    // O roteador reinicia: A passa a sair por outra porta externa.
    nat.rebind();
    conferir_eco(&wa, IP_B, 64 * 1024).await;
    let depois = wb.peers().await[0].endpoint.unwrap();
    assert_ne!(antes, depois, "B não acompanhou o novo endereço de A");
    wa.stop(Duration::from_secs(5)).await;
    wb.stop(Duration::from_secs(5)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn com_perda_de_pacotes_o_eco_ainda_fecha() {
    let rede = SimNet::new();
    rede.set_loss(10);
    let (a, b, _, _) = dupla(&rede, None, "").await;
    conectado(&a).await;
    conferir_eco(&a, IP_B, 256 * 1024).await;
    a.stop(Duration::from_secs(5)).await;
    b.stop(Duration::from_secs(5)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nome_de_endpoint_que_nao_resolve_vira_retrying_e_aviso() {
    let rede = SimNet::new();
    let (a, b) = (Par::novo(), Par::novo());
    let ponto = rede.host("203.0.113.1", 51820);
    let wa = iniciar(
        conf(
            &a,
            IP_A,
            &b,
            "servidor-que-nao-existe.invalid:51820",
            "10.66.0.1/32",
            None,
            "",
        ),
        ponto,
    )
    .await;
    assert!(
        wa.warnings()
            .iter()
            .any(|w| w
                .contains("não foi possível resolver o Endpoint servidor-que-nao-existe.invalid")),
        "{:?}",
        wa.warnings()
    );
    match wa.state() {
        State::Retrying(m) => assert!(m.contains("não foi possível resolver o Endpoint"), "{m}"),
        outro => panic!("estado inesperado: {outro:?}"),
    }
    wa.stop(Duration::from_secs(5)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn on_state_recebe_conectado_e_parado() {
    let rede = SimNet::new();
    let (a, b) = (Par::novo(), Par::novo());
    let vistos = Arc::new(Mutex::new(Vec::<State>::new()));
    let ponto_a = rede.host("203.0.113.1", 51820);
    let ponto_b = rede.host("203.0.113.2", 51820);
    let gravar = vistos.clone();
    let wb = iniciar(
        conf(&b, IP_B, &a, "203.0.113.1:51820", "10.66.0.2/32", None, ""),
        ponto_b,
    )
    .await;
    let wa = WireGuardBuilder::new(conf(
        &a,
        IP_A,
        &b,
        "203.0.113.2:51820",
        "10.66.0.1/32",
        None,
        "",
    ))
    .on_state(move |s| gravar.lock().unwrap().push(s))
    .start_on(ponto_a)
    .await
    .unwrap();
    conectado(&wa).await;
    wa.stop(Duration::from_secs(5)).await;
    wb.stop(Duration::from_secs(5)).await;
    let vistos = vistos.lock().unwrap();
    assert!(
        matches!(vistos.first(), Some(State::Connected { .. })),
        "{vistos:?}"
    );
    assert_eq!(vistos.last(), Some(&State::Stopped));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn conf_sem_endpoint_nao_sobe() {
    let rede = SimNet::new();
    let (a, b) = (Par::novo(), Par::novo());
    let texto = format!(
        "[Interface]\nPrivateKey = {}\nAddress = 10.66.0.2/32\n[Peer]\nPublicKey = {}\nAllowedIPs = 10.66.0.1/32\n",
        a.privada.to_base64().as_str(),
        b.publica()
    );
    let (config, _) = WgConfig::from_conf(&texto).unwrap();
    let erro = WireGuardBuilder::new(config)
        .start_on(rede.host("203.0.113.1", 1))
        .await
        .map(|_| ())
        .unwrap_err();
    assert!(
        erro.to_string().contains("nenhum par tem Endpoint"),
        "{erro}"
    );
}
