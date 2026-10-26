//! A ponte SOCKS5: o cliente de verdade da lib (`tomni_ui_net::Proxy::Socks5`) alcança a rede do
//! túnel, e quem não tem a credencial não passa.

use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream as Local;
use tomni_ui_net::Proxy;
use tomni_ui_netstack::pipe::{PipeOptions, PipeTunnel};
use tomni_ui_netstack::{NetStack, SocksAccess, SocksBridge};

mod comum;

const A: &str = "10.42.0.1/24";
const B: &str = "10.42.0.2/24";
const ESPERA: Duration = Duration::from_secs(5);

struct Rede {
    a: NetStack,
    b: NetStack,
    ponte: SocksBridge,
}

async fn rede(tunel_a: PipeTunnel, tunel_b: PipeTunnel) -> Rede {
    let a = NetStack::spawn(tunel_a);
    let b = NetStack::spawn(tunel_b);
    let mut escuta = b.listen(7000).await.expect("escuta");
    tokio::spawn(async move {
        while let Ok(mut conexao) = escuta.accept().await {
            tokio::spawn(async move {
                let (mut leitura, mut escrita) = tokio::io::split(&mut conexao);
                let _ = tokio::io::copy(&mut leitura, &mut escrita).await;
            });
        }
    });
    let ponte = SocksBridge::start(a.clone()).await.expect("ponte");
    Rede { a, b, ponte }
}

async fn padrao() -> Rede {
    let (a, b) = PipeTunnel::pair(A, B, PipeOptions::default());
    rede(a, b).await
}

fn proxy(rede: &Rede) -> Proxy {
    com_senha(rede.ponte.access(), None)
}

/// O proxy da lib apontando para a ponte, com a senha da ponte ou a dada.
fn com_senha(acesso: &SocksAccess, senha: Option<&str>) -> Proxy {
    Proxy::Socks5 {
        host: acesso.addr.ip().to_string(),
        port: acesso.addr.port(),
        user: Some(acesso.user.clone()),
        password: Some(senha.map_or_else(|| acesso.password.to_string(), str::to_owned)),
    }
}

/// Abre pelo cliente da lib e confere o eco de uma frase.
fn eco_por(proxy: &Proxy, host: &str, porta: u16) -> Result<(), String> {
    let erro = |e: io::Error| e.to_string();
    let mut conexao = proxy.open(host, porta, ESPERA)?;
    conexao.write_all(b"oi, rede").map_err(erro)?;
    let mut volta = [0u8; 8];
    conexao.read_exact(&mut volta).map_err(erro)?;
    assert_eq!(&volta, b"oi, rede");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn o_cliente_da_lib_alcanca_o_eco_com_a_credencial_certa() {
    let r = padrao().await;
    let p = proxy(&r);
    // O cliente de `tomni-ui-net` é síncrono e manda sempre o endereço como nome (ATYP 3).
    tokio::task::spawn_blocking(move || eco_por(&p, "10.42.0.2", 7000))
        .await
        .unwrap()
        .expect("eco pela ponte");
    r.ponte.stop().await;
    r.a.shutdown().await;
    r.b.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn senha_errada_e_sem_autenticacao_nao_passam() {
    let r = padrao().await;
    let acesso = r.ponte.access().clone();
    let errada = com_senha(&acesso, Some("senha-errada"));
    let sem_auth = Proxy::Socks5 {
        host: acesso.addr.ip().to_string(),
        port: acesso.addr.port(),
        user: None,
        password: None,
    };
    let (e1, e2) = tokio::task::spawn_blocking(move || {
        (
            errada.open("10.42.0.2", 7000, ESPERA).err(),
            sem_auth.open("10.42.0.2", 7000, ESPERA).err(),
        )
    })
    .await
    .unwrap();
    assert!(e1.is_some(), "senha errada é recusada");
    assert!(e2.is_some(), "sem autenticação é recusado");
    r.ponte.stop().await;
    r.a.shutdown().await;
    r.b.shutdown().await;
}

/// Fala o SOCKS5 à mão, para os tipos de endereço que o cliente da lib não manda.
async fn pedido_cru(rede: &Rede, endereco: &[u8], porta: u16) -> u8 {
    let acesso = rede.ponte.access();
    let mut s = Local::connect(acesso.addr).await.expect("conecta na ponte");
    s.write_all(&[5, 1, 2]).await.unwrap();
    let mut metodo = [0u8; 2];
    s.read_exact(&mut metodo).await.unwrap();
    assert_eq!(metodo, [5, 2]);
    let mut auth = vec![1, acesso.user.len() as u8];
    auth.extend_from_slice(acesso.user.as_bytes());
    auth.push(acesso.password.len() as u8);
    auth.extend_from_slice(acesso.password.as_bytes());
    s.write_all(&auth).await.unwrap();
    let mut ok = [0u8; 2];
    s.read_exact(&mut ok).await.unwrap();
    assert_eq!(ok, [1, 0]);
    let mut pedido = vec![5, 1, 0];
    pedido.extend_from_slice(endereco);
    pedido.extend_from_slice(&porta.to_be_bytes());
    s.write_all(&pedido).await.unwrap();
    let mut resposta = [0u8; 10];
    tokio::time::timeout(Duration::from_secs(15), s.read_exact(&mut resposta))
        .await
        .expect("a ponte responde")
        .unwrap();
    if resposta[1] == 0 {
        s.write_all(b"ping").await.unwrap();
        let mut volta = [0u8; 4];
        s.read_exact(&mut volta).await.unwrap();
        assert_eq!(&volta, b"ping");
    }
    resposta[1]
}

#[tokio::test(flavor = "multi_thread")]
async fn endereco_ipv4_atyp_1_funciona() {
    let r = padrao().await;
    assert_eq!(pedido_cru(&r, &[1, 10, 42, 0, 2], 7000).await, 0);
    r.ponte.stop().await;
    r.a.shutdown().await;
    r.b.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn nome_resolvido_pelo_tunel_atyp_3_funciona() {
    let (a, b) = PipeTunnel::pair(A, B, PipeOptions::default());
    let a = a.with_name("servidor.rede", Ipv4Addr::new(10, 42, 0, 2));
    let r = rede(a, b).await;
    let mut nome = vec![3, "servidor.rede".len() as u8];
    nome.extend_from_slice(b"servidor.rede");
    assert_eq!(pedido_cru(&r, &nome, 7000).await, 0);
    // Um nome que ninguém resolve vira "host inalcançável" (0x04).
    let mut desconhecido = vec![3, "nao-existe.invalido".len() as u8];
    desconhecido.extend_from_slice(b"nao-existe.invalido");
    assert_eq!(pedido_cru(&r, &desconhecido, 7000).await, 4);
    r.ponte.stop().await;
    r.a.shutdown().await;
    r.b.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn endereco_ipv6_atyp_4_fora_das_redes_responde_rede_inalcancavel() {
    let r = padrao().await;
    let mut v6 = vec![4];
    v6.extend_from_slice(&[0; 15]);
    v6.push(1);
    assert_eq!(pedido_cru(&r, &v6, 7000).await, 3);
    r.ponte.stop().await;
    r.a.shutdown().await;
    r.b.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn destino_fora_das_redes_responde_rede_inalcancavel() {
    let r = padrao().await;
    assert_eq!(pedido_cru(&r, &[1, 192, 168, 7, 7], 22).await, 3);
    r.ponte.stop().await;
    r.a.shutdown().await;
    r.b.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn porta_recusada_pelo_servidor_responde_0x05() {
    // O app é invisível, então quem recusa é o servidor do outro lado: o lado cru faz o RST.
    let (tunel, mut cru) = PipeTunnel::raw(A, PipeOptions::default());
    let a = NetStack::spawn(tunel);
    tokio::spawn(async move {
        while let Some(pacote) = cru.recv().await {
            cru.send(comum::rst_ack(&pacote));
        }
    });
    let ponte = SocksBridge::start(a.clone()).await.expect("ponte");
    let acesso = ponte.access().clone();
    let proxy = com_senha(&acesso, None);
    let erro = tokio::task::spawn_blocking(move || proxy.open("10.42.0.2", 7001, ESPERA).err())
        .await
        .unwrap()
        .expect("recusada");
    assert!(erro.contains("recusou"), "{erro}");
    ponte.stop().await;
    a.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_ponte_so_escuta_no_loopback_e_para_de_aceitar_ao_parar() {
    let r = padrao().await;
    let endereco: SocketAddr = r.ponte.access().addr;
    assert!(endereco.ip().is_loopback());
    r.ponte.stop().await;
    assert!(
        Local::connect(endereco).await.is_err(),
        "depois do stop a porta está fechada"
    );
    r.a.shutdown().await;
    r.b.shutdown().await;
}
