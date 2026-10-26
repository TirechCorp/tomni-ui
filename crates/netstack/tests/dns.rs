//! O DNS dentro do túnel (`DnsOverTcp`) e a ponte que sobe antes da pilha (`serve_later`).

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream as Local;
use tomni_ui_netstack::pipe::{PipeOptions, PipeTunnel};
use tomni_ui_netstack::{DnsOverTcp, DnsScope, NetStack, SocksAccess, SocksListener};

const A: &str = "10.42.0.1/24";
const B: &str = "10.42.0.2/24";

/// Um servidor DNS de mentira em `B:53` (TCP) com uma tabela de nomes; devolve o contador de
/// consultas recebidas.
async fn servidor_dns(b: &NetStack, tabela: &[(&str, [u8; 4])]) -> Arc<AtomicUsize> {
    let tabela: HashMap<String, [u8; 4]> = tabela
        .iter()
        .map(|(n, ip)| ((*n).to_owned(), *ip))
        .collect();
    let consultas = Arc::new(AtomicUsize::new(0));
    let contador = consultas.clone();
    let mut escuta = b.listen(53).await.expect("escuta 53");
    tokio::spawn(async move {
        while let Ok(mut conexao) = escuta.accept().await {
            let (tabela, contador) = (tabela.clone(), contador.clone());
            tokio::spawn(async move {
                let mut tamanho = [0u8; 2];
                conexao.read_exact(&mut tamanho).await.ok()?;
                let mut consulta = vec![0u8; usize::from(u16::from_be_bytes(tamanho))];
                conexao.read_exact(&mut consulta).await.ok()?;
                contador.fetch_add(1, Ordering::SeqCst);
                // O nome em rótulos, a partir do byte 12.
                let (mut pos, mut nome) = (12, Vec::new());
                while consulta[pos] != 0 {
                    let n = usize::from(consulta[pos]);
                    nome.push(
                        String::from_utf8_lossy(&consulta[pos + 1..pos + 1 + n]).into_owned(),
                    );
                    pos += 1 + n;
                }
                let fim_pergunta = pos + 5;
                let nome = nome.join(".");
                let mut resposta = Vec::new();
                resposta.extend_from_slice(&consulta[0..2]);
                match tabela.get(&nome) {
                    Some(ip) => {
                        resposta.extend_from_slice(&[0x81, 0x80, 0, 1, 0, 1, 0, 0, 0, 0]);
                        resposta.extend_from_slice(&consulta[12..fim_pergunta]);
                        resposta.extend_from_slice(&[0xC0, 12, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4]);
                        resposta.extend_from_slice(ip);
                    }
                    None => {
                        resposta.extend_from_slice(&[0x81, 0x83, 0, 1, 0, 0, 0, 0, 0, 0]);
                        resposta.extend_from_slice(&consulta[12..fim_pergunta]);
                    }
                }
                let mut quadro = (resposta.len() as u16).to_be_bytes().to_vec();
                quadro.extend_from_slice(&resposta);
                conexao.write_all(&quadro).await.ok()?;
                let _ = conexao.shutdown().await;
                Some(())
            });
        }
    });
    consultas
}

async fn eco(b: &NetStack) {
    let mut escuta = b.listen(7000).await.expect("escuta");
    tokio::spawn(async move {
        while let Ok(mut conexao) = escuta.accept().await {
            tokio::spawn(async move {
                let (mut l, mut e) = tokio::io::split(&mut conexao);
                let _ = tokio::io::copy(&mut l, &mut e).await;
            });
        }
    });
}

/// Pede um `CONNECT` por nome (ATYP 3) e devolve o código da resposta; se abriu, confere o eco.
async fn pede_nome(acesso: &SocksAccess, nome: &str, porta: u16) -> u8 {
    let mut s = Local::connect(acesso.addr).await.expect("conecta na ponte");
    s.write_all(&[5, 1, 2]).await.unwrap();
    let mut metodo = [0u8; 2];
    s.read_exact(&mut metodo).await.unwrap();
    let mut auth = vec![1, acesso.user.len() as u8];
    auth.extend_from_slice(acesso.user.as_bytes());
    auth.push(acesso.password.len() as u8);
    auth.extend_from_slice(acesso.password.as_bytes());
    s.write_all(&auth).await.unwrap();
    let mut ok = [0u8; 2];
    s.read_exact(&mut ok).await.unwrap();
    let mut pedido = vec![5, 1, 0, 3, nome.len() as u8];
    pedido.extend_from_slice(nome.as_bytes());
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

struct Rede {
    a: NetStack,
    b: NetStack,
    consultas: Arc<AtomicUsize>,
}

async fn rede() -> Rede {
    let (ta, tb) = PipeTunnel::pair(A, B, PipeOptions::default());
    let (a, b) = (NetStack::spawn(ta), NetStack::spawn(tb));
    eco(&b).await;
    let consultas = servidor_dns(
        &b,
        &[
            ("alvo.rede.local", [10, 42, 0, 2]),
            ("fixo.corp", [10, 42, 0, 2]),
        ],
    )
    .await;
    Rede { a, b, consultas }
}

fn dns(escopo: DnsScope) -> Arc<DnsOverTcp> {
    Arc::new(DnsOverTcp::new(
        vec![Ipv4Addr::new(10, 42, 0, 2)],
        vec!["rede.local".into()],
        escopo,
    ))
}

#[tokio::test(flavor = "multi_thread")]
async fn nome_sem_ponto_usa_o_dominio_de_busca_e_o_cache_evita_perguntar_de_novo() {
    let r = rede().await;
    let ponte = SocksListener::bind()
        .unwrap()
        .resolver(dns(DnsScope::Domains(vec![])))
        .serve(r.a.clone())
        .unwrap();
    let acesso = ponte.access().clone();
    assert_eq!(pede_nome(&acesso, "alvo", 7000).await, 0);
    assert_eq!(r.consultas.load(Ordering::SeqCst), 1);
    assert_eq!(pede_nome(&acesso, "alvo", 7000).await, 0);
    assert_eq!(pede_nome(&acesso, "alvo.rede.local", 7000).await, 0);
    assert_eq!(r.consultas.load(Ordering::SeqCst), 1, "o cache respondeu");
    ponte.stop().await;
    r.a.shutdown().await;
    r.b.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn com_escopo_de_dominios_o_nome_de_fora_nao_vai_para_o_dns_da_vpn() {
    let r = rede().await;
    let ponte = SocksListener::bind()
        .unwrap()
        .resolver(dns(DnsScope::Domains(vec!["corp".into()])))
        .serve(r.a.clone())
        .unwrap();
    let acesso = ponte.access().clone();
    assert_eq!(
        pede_nome(&acesso, "fixo.corp", 7000).await,
        0,
        "domínio empurrado"
    );
    assert_eq!(r.consultas.load(Ordering::SeqCst), 1);
    // Fora do escopo, a VPN não é perguntada (vai para o resolvedor do computador, que não
    // conhece o nome: host inalcançável).
    assert_eq!(pede_nome(&acesso, "nao-existe.invalid", 7000).await, 4);
    assert_eq!(r.consultas.load(Ordering::SeqCst), 1);
    ponte.stop().await;
    r.a.shutdown().await;
    r.b.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn com_escopo_total_nome_que_a_vpn_nao_conhece_nao_vaza_e_falha() {
    let r = rede().await;
    let ponte = SocksListener::bind()
        .unwrap()
        .resolver(dns(DnsScope::All))
        .serve(r.a.clone())
        .unwrap();
    let acesso = ponte.access().clone();
    // Nem `localhost` escapa para o resolvedor do computador quando a VPN cuida de tudo.
    assert_eq!(pede_nome(&acesso, "localhost", 7000).await, 4);
    assert!(r.consultas.load(Ordering::SeqCst) >= 1);
    ponte.stop().await;
    r.a.shutdown().await;
    r.b.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_ponte_sobe_antes_da_pilha_e_as_conexoes_esperam_o_attach() {
    let r = rede().await;
    let ponte = SocksListener::bind()
        .unwrap()
        .connect_timeout(Duration::from_secs(5))
        .serve_later()
        .unwrap();
    let acesso = ponte.access().clone();
    let a = r.a.clone();
    let pedido = tokio::spawn(async move { pede_nome(&acesso, "10.42.0.2", 7000).await });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!pedido.is_finished(), "a conexão espera a pilha");
    ponte.attach(a);
    assert_eq!(pedido.await.unwrap(), 0);
    // Sem pilha de novo, a próxima desiste com "rede inalcançável".
    ponte.detach();
    let curta = SocksListener::bind()
        .unwrap()
        .connect_timeout(Duration::from_millis(300))
        .serve_later()
        .unwrap();
    assert_eq!(
        pede_nome(&curta.access().clone(), "10.42.0.2", 7000).await,
        3
    );
    curta.stop().await;
    ponte.stop().await;
    r.a.shutdown().await;
    r.b.shutdown().await;
}
