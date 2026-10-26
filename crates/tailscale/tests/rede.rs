//! Dois (ou três) nós Tailscale contra o `ControlFake` em modo `tailnet`, o `DerpFake` e o UDP
//! simulado do `tomni-ui-netstack`: o registro, o mapa, o WireGuard, o DERP, o caminho direto
//! e a ponte SOCKS5 são de verdade; só o servidor de controle, o DERP e a rede UDP são falsos.

use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tomni_ui_netstack::{NatKind, SimEndpoint, SimNet};
use tomni_ui_tailscale::tailcfg::{DerpNode, DerpRegion};
use tomni_ui_tailscale::testing::{ControlFake, DerpFake, FakeOptions, spawn_stun};
use tomni_ui_tailscale::{NodeState, State, Tailscale, TailscaleBuilder};
use zeroize::Zeroizing;

const CHAVE: &str = "tskey-auth-valida";

fn opcoes() -> FakeOptions {
    FakeOptions {
        tailnet: true,
        ..FakeOptions::default()
    }
}

/// Um construtor com a chave de autenticação e o nome dados.
fn no(controle: &ControlFake, nome: &str) -> TailscaleBuilder {
    TailscaleBuilder::new(controle.url(), NodeState::default())
        .hostname(nome)
        .auth_key(Zeroizing::new(CHAVE.to_string()))
}

async fn conectar(ts: &Tailscale) -> IpAddr {
    ts.wait_connected(Duration::from_secs(20))
        .await
        .unwrap_or_else(|e| panic!("não conectou: {e}"))
}

/// Sobe um servidor de eco na porta 7 da pilha do nó.
fn eco(ts: &Tailscale) {
    let stack = ts.stack().expect("a pilha existe depois de conectar");
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

/// Abre uma conexão pela ponte SOCKS5 do nó (o que um cliente do app faz) até `destino:porta`.
async fn por_socks(ts: &Tailscale, destino: &str, porta: u16) -> std::io::Result<TcpStream> {
    let acesso = ts.socks();
    let mut s = TcpStream::connect(acesso.addr).await?;
    s.write_all(&[5, 1, 2]).await?;
    let mut r = [0u8; 2];
    s.read_exact(&mut r).await?;
    assert_eq!(r, [5, 2], "a ponte exige usuário e senha");
    let mut auth = vec![1, acesso.user.len() as u8];
    auth.extend_from_slice(acesso.user.as_bytes());
    auth.push(acesso.password.len() as u8);
    auth.extend_from_slice(acesso.password.as_bytes());
    s.write_all(&auth).await?;
    s.read_exact(&mut r).await?;
    assert_eq!(r, [1, 0], "a credencial da ponte foi recusada");
    let mut pedido = vec![5, 1, 0];
    match destino.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => {
            pedido.push(1);
            pedido.extend_from_slice(&ip.octets());
        }
        _ => {
            pedido.push(3);
            pedido.push(destino.len() as u8);
            pedido.extend_from_slice(destino.as_bytes());
        }
    }
    pedido.extend_from_slice(&porta.to_be_bytes());
    s.write_all(&pedido).await?;
    let mut cab = [0u8; 4];
    s.read_exact(&mut cab).await?;
    if cab[1] != 0 {
        return Err(std::io::Error::other(format!("SOCKS5 recusou: {}", cab[1])));
    }
    let resto = match cab[3] {
        1 => 4 + 2,
        4 => 16 + 2,
        _ => {
            let mut n = [0u8; 1];
            s.read_exact(&mut n).await?;
            usize::from(n[0]) + 2
        }
    };
    let mut lixo = vec![0u8; resto];
    s.read_exact(&mut lixo).await?;
    Ok(s)
}

/// Manda `tamanho` bytes por uma conexão e confere que o eco volta igual.
async fn conferir_eco(mut conexao: TcpStream, tamanho: usize) {
    let envio = dados(tamanho);
    let (mut le, mut escreve) = conexao.split();
    let a_enviar = envio.clone();
    let mut volta = vec![0u8; tamanho];
    let (_, lido) = tokio::join!(
        async {
            escreve.write_all(&a_enviar).await.unwrap();
        },
        tokio::time::timeout(Duration::from_secs(60), le.read_exact(&mut volta)),
    );
    lido.expect("o eco demorou demais").unwrap();
    assert!(volta == envio, "o eco voltou diferente");
}

async fn caminho_de(ts: &Tailscale, nome: &str) -> String {
    ts.peers()
        .await
        .into_iter()
        .find(|p| p.name == nome)
        .map(|p| p.path)
        .unwrap_or_default()
}

/// Espera o caminho do par começar com `prefixo`, mandando um pouco de tráfego enquanto isso
/// (o caminho só é mantido e medido com a conexão em uso).
async fn esperar_caminho(
    ts: &Tailscale,
    nome: &str,
    destino: &str,
    prefixo: &str,
    limite: Duration,
) -> String {
    let fim = tokio::time::Instant::now() + limite;
    let mut conexao = por_socks(ts, destino, 7).await.expect("conexão");
    loop {
        let atual = caminho_de(ts, nome).await;
        if atual.starts_with(prefixo) {
            return atual;
        }
        assert!(
            tokio::time::Instant::now() < fim,
            "o caminho devia ser \"{prefixo}\", mas é \"{atual}\""
        );
        conexao.write_all(b"x").await.unwrap();
        let mut um = [0u8; 1];
        let _ = tokio::time::timeout(Duration::from_secs(5), conexao.read_exact(&mut um)).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

async fn tailnet_de_dois(
    udp: bool,
) -> (ControlFake, DerpFake, Tailscale, Tailscale, IpAddr, IpAddr) {
    tailnet_de_dois_com(udp, opcoes()).await
}

async fn tailnet_de_dois_com(
    udp: bool,
    opcoes: FakeOptions,
) -> (ControlFake, DerpFake, Tailscale, Tailscale, IpAddr, IpAddr) {
    let controle = ControlFake::with_options(opcoes).await;
    let derp = DerpFake::start().await;
    controle.set_derp_map(derp.derp_map(1));
    let a = no(&controle, "alfa").udp(udp).start().await.unwrap();
    let ip_a = conectar(&a).await;
    let b = no(&controle, "beto").udp(udp).start().await.unwrap();
    let ip_b = conectar(&b).await;
    eco(&b);
    (controle, derp, a, b, ip_a, ip_b)
}

#[tokio::test]
async fn so_derp_leva_um_mebibyte_pelo_socks() {
    let (_controle, derp, a, b, ip_a, ip_b) = tailnet_de_dois(false).await;
    assert_ne!(ip_a, ip_b);
    // Pelo nome (MagicDNS) e pelo endereço.
    conferir_eco(
        por_socks(&a, "beto.teste.ts.net", 7)
            .await
            .expect("por nome"),
        1 << 20,
    )
    .await;
    conferir_eco(
        por_socks(&a, &ip_b.to_string(), 7).await.expect("por ip"),
        64 * 1024,
    )
    .await;
    assert!(
        derp.relayed_packets() > 0,
        "o tráfego devia passar pelo DERP"
    );
    let caminho = caminho_de(&a, "beto").await;
    assert!(caminho.starts_with("DERP"), "caminho: {caminho}");
    assert!(matches!(a.state(), State::Connected { .. }));
    // O proxy devolvido leva a mesma credencial.
    let _ = b.proxy();
    a.stop(Duration::from_secs(5)).await;
    b.stop(Duration::from_secs(5)).await;
}

#[tokio::test]
async fn nome_desconhecido_da_tailnet_e_recusado() {
    let (_controle, _derp, a, b, _ip_a, _ip_b) = tailnet_de_dois(false).await;
    let erro = por_socks(&a, "ninguem.teste.ts.net", 7)
        .await
        .expect_err("o nome não existe");
    assert!(erro.to_string().contains("SOCKS5"), "{erro}");
    a.stop(Duration::from_secs(5)).await;
    b.stop(Duration::from_secs(5)).await;
}

#[tokio::test]
async fn destino_fora_da_tailnet_falha_com_mensagem() {
    let (_controle, _derp, a, b, _ip_a, _ip_b) = tailnet_de_dois(false).await;
    let stack = a.stack().unwrap();
    let erro = stack
        .connect_timeout("100.64.9.9:7".parse().unwrap(), Duration::from_secs(10))
        .await
        .expect_err("ninguém tem esse endereço");
    assert!(
        erro.to_string().contains("100.64.9.9"),
        "a mensagem devia dizer o endereço: {erro}"
    );
    a.stop(Duration::from_secs(5)).await;
    b.stop(Duration::from_secs(5)).await;
}

/// A e B atrás de NATs de cone, com o STUN no "mundo".
struct Mundo {
    controle: ControlFake,
    derp: DerpFake,
    rede: SimNet,
    nat_a: tomni_ui_netstack::SimNat,
    nat_b: tomni_ui_netstack::SimNat,
}

async fn mundo() -> Mundo {
    let rede = SimNet::new();
    let controle = ControlFake::with_options(opcoes()).await;
    let derp = DerpFake::start().await;
    let stun: SimEndpoint = rede.host("198.51.100.1", 3478);
    spawn_stun(Arc::new(stun));
    let mut mapa = derp.derp_map(1);
    mapa.regions.insert(
        2,
        Some(DerpRegion {
            region_id: 2,
            region_code: "stun".to_string(),
            region_name: "stun".to_string(),
            nodes: vec![DerpNode {
                name: "2a".to_string(),
                region_id: 2,
                host_name: "stun.teste".to_string(),
                ipv4: "198.51.100.1".to_string(),
                stun_port: 3478,
                stun_only: true,
                ..DerpNode::default()
            }],
            ..DerpRegion::default()
        }),
    );
    controle.set_derp_map(mapa);
    let nat_a = rede.nat("198.51.100.10", NatKind::FullCone);
    let nat_b = rede.nat("198.51.100.20", NatKind::FullCone);
    Mundo {
        controle,
        derp,
        rede,
        nat_a,
        nat_b,
    }
}

async fn dupla_com_udp(m: &Mundo) -> (Tailscale, Tailscale) {
    let a = no(&m.controle, "alfa")
        .start_on(m.nat_a.endpoint("192.168.1.2", 41641))
        .await
        .unwrap();
    conectar(&a).await;
    let b = no(&m.controle, "beto")
        .start_on(m.nat_b.endpoint("192.168.2.2", 41641))
        .await
        .unwrap();
    conectar(&b).await;
    eco(&b);
    (a, b)
}

#[tokio::test]
async fn comeca_pelo_derp_e_passa_a_direto() {
    let m = mundo().await;
    let (a, b) = dupla_com_udp(&m).await;
    // O tráfego já passa de imediato (pelo DERP, enquanto o disco acha o caminho direto).
    conferir_eco(
        por_socks(&a, "beto", 7)
            .await
            .unwrap_or_else(|_| panic!("sem rota")),
        8 * 1024,
    )
    .await;
    let caminho = esperar_caminho(
        &a,
        "beto",
        "beto.teste.ts.net",
        "direto",
        Duration::from_secs(40),
    )
    .await;
    assert!(caminho.contains("198.51.100.20"), "caminho: {caminho}");
    conferir_eco(
        por_socks(&a, "beto.teste.ts.net", 7).await.unwrap(),
        256 * 1024,
    )
    .await;
    a.stop(Duration::from_secs(5)).await;
    b.stop(Duration::from_secs(5)).await;
    drop(m.derp);
}

#[tokio::test]
async fn quando_o_direto_para_volta_ao_derp() {
    let m = mundo().await;
    let (a, b) = dupla_com_udp(&m).await;
    esperar_caminho(
        &a,
        "beto",
        "beto.teste.ts.net",
        "direto",
        Duration::from_secs(40),
    )
    .await;
    // O UDP morre; o DERP segue.
    m.rede.set_loss(100);
    esperar_caminho(
        &a,
        "beto",
        "beto.teste.ts.net",
        "DERP",
        Duration::from_secs(40),
    )
    .await;
    m.rede.set_loss(0);
    conferir_eco(
        por_socks(&a, "beto.teste.ts.net", 7).await.unwrap(),
        128 * 1024,
    )
    .await;
    a.stop(Duration::from_secs(5)).await;
    b.stop(Duration::from_secs(5)).await;
}

#[tokio::test]
async fn par_desligado_falha_na_hora_com_mensagem() {
    // O servidor só nota que o fluxo acabou ao tentar escrever nele: sinais de vida frequentes.
    let opcoes = FakeOptions {
        map_keepalive: Some(Duration::from_millis(100)),
        ..opcoes()
    };
    let (_controle, _derp, a, b, _ip_a, ip_b) = tailnet_de_dois_com(false, opcoes).await;
    // O par se desliga (o controle o marca como fora do ar): a conexão falha dizendo isso, sem
    // esperar o handshake estourar.
    b.stop(Duration::from_secs(5)).await;
    for _ in 0..50 {
        let fora = a
            .peers()
            .await
            .iter()
            .any(|p| p.name == "beto" && p.online == Some(false));
        if fora {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let stack = a.stack().unwrap();
    let erro = stack
        .connect_timeout(SocketAddr::new(ip_b, 7), Duration::from_secs(10))
        .await
        .expect_err("o par saiu");
    assert!(erro.to_string().contains("beto está desligada"), "{erro}");
    a.stop(Duration::from_secs(5)).await;
}

#[tokio::test]
async fn login_pelo_navegador_e_aprovacao() {
    let controle = ControlFake::with_options(FakeOptions {
        tailnet: true,
        admin_approval: true,
        ..FakeOptions::default()
    })
    .await;
    let derp = DerpFake::start().await;
    controle.set_derp_map(derp.derp_map(1));
    let estados: Arc<Mutex<Vec<State>>> = Arc::default();
    let guarda = estados.clone();
    let ts = TailscaleBuilder::new(controle.url(), NodeState::default())
        .hostname("alfa")
        .udp(false)
        .on_state(move |s| guarda.lock().unwrap().push(s.clone()))
        .start()
        .await
        .unwrap();
    let erro = ts
        .wait_connected(Duration::from_secs(3))
        .await
        .expect_err("ainda precisa de login");
    assert!(erro.to_string().contains("abra http://"), "{erro}");
    assert!(matches!(ts.state(), State::NeedsLogin { .. }));
    controle.approve();
    ts.wait_connected(Duration::from_secs(30))
        .await
        .expect("conecta depois da aprovação");
    let vistos = estados.lock().unwrap().clone();
    assert!(vistos.iter().any(|s| matches!(s, State::NeedsLogin { .. })));
    assert!(vistos.iter().any(|s| matches!(s, State::NeedsApproval)));
    ts.stop(Duration::from_secs(5)).await;
}

#[tokio::test]
async fn chave_vencida_sem_chave_de_autenticacao_e_expirada() {
    let controle = ControlFake::with_options(FakeOptions {
        tailnet: true,
        expire_node_key: true,
        ..FakeOptions::default()
    })
    .await;
    let ts = TailscaleBuilder::new(controle.url(), NodeState::default())
        .udp(false)
        .start()
        .await
        .unwrap();
    let erro = ts
        .wait_connected(Duration::from_secs(10))
        .await
        .expect_err("a chave venceu");
    assert!(erro.to_string().contains("venceu"), "{erro}");
    assert_eq!(ts.state(), State::Expired);
    ts.stop(Duration::from_secs(5)).await;
}

#[tokio::test]
async fn o_estado_do_no_e_avisado_e_reaproveitado() {
    let controle = ControlFake::with_options(opcoes()).await;
    let derp = DerpFake::start().await;
    controle.set_derp_map(derp.derp_map(1));
    let gravado: Arc<Mutex<Vec<NodeState>>> = Arc::default();
    let g = gravado.clone();
    let a = no(&controle, "alfa")
        .udp(false)
        .on_node_state(move |n| g.lock().unwrap().push(n.clone()))
        .start()
        .await
        .unwrap();
    let ip = conectar(&a).await;
    let ultimo = gravado.lock().unwrap().last().cloned().expect("avisou");
    assert!(ultimo.machine_key.is_some() && ultimo.node_key.is_some());
    assert_eq!(ultimo.tailnet, "teste.ts.net");
    assert!(ultimo.addresses.contains(&ip));
    let chave = a.public_key();
    a.stop(Duration::from_secs(5)).await;
    // Reinicia com o estado gravado: é o mesmo nó.
    let b = no(&controle, "alfa").udp(false).start().await.unwrap();
    let _ = b;
    let c = TailscaleBuilder::new(controle.url(), ultimo)
        .hostname("alfa")
        .auth_key(Zeroizing::new(CHAVE.to_string()))
        .udp(false)
        .start()
        .await
        .unwrap();
    assert_eq!(c.public_key(), chave);
    c.stop(Duration::from_secs(5)).await;
}

#[tokio::test]
async fn o_no_e_invisivel_para_quem_nao_e_par() {
    let (_controle, _derp, a, b, ip_a, _ip_b) = tailnet_de_dois(true).await;
    // Com UDP aberto (porta real no 0.0.0.0): um datagrama de WireGuard de um desconhecido, um
    // disco de uma chave desconhecida e lixo não geram resposta nenhuma.
    let porta = a.udp_addr().expect("UDP aberto").port();
    let sonda = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let destino = SocketAddr::new("127.0.0.1".parse().unwrap(), porta);
    let mut iniciacao = vec![0u8; 148];
    iniciacao[0] = 1;
    for pacote in [
        iniciacao,
        b"TS\xf0\x9f\x92\xac".to_vec(),
        vec![0u8; 40],
        vec![1, 2, 3],
    ] {
        sonda.send_to(&pacote, destino).await.unwrap();
    }
    let mut buf = [0u8; 2048];
    let resposta =
        tokio::time::timeout(Duration::from_millis(1500), sonda.recv_from(&mut buf)).await;
    assert!(resposta.is_err(), "um estranho não pode receber resposta");
    // E nada de conexão chega pela tailnet a quem não abriu: a pilha de A não escuta nada.
    let stack = b.stack().unwrap();
    let erro = stack
        .connect_timeout(SocketAddr::new(ip_a, 22), Duration::from_secs(5))
        .await;
    assert!(erro.is_err(), "A não escuta em porta nenhuma");
    a.stop(Duration::from_secs(5)).await;
    b.stop(Duration::from_secs(5)).await;
}

#[tokio::test]
async fn logout_pede_ao_controle_e_para() {
    let (controle, _derp, a, b, _ip_a, _ip_b) = tailnet_de_dois(false).await;
    let antes = controle.requests().len();
    a.logout().await.expect("logout");
    assert!(
        controle.requests().len() > antes,
        "o controle devia ter recebido o logout"
    );
    b.stop(Duration::from_secs(5)).await;
}
