//! Os testes da pilha: duas pilhas ligadas em memória, uma abre e a outra escuta (eco).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io;
use std::net::SocketAddr;
use std::time::Duration;

mod comum;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tomni_ui_netstack::pipe::{PipeOptions, PipeTunnel};
use tomni_ui_netstack::{NetStack, TcpStream};

const A: &str = "10.42.0.1/24";
const B: &str = "10.42.0.2/24";

/// Sobe duas pilhas e um servidor de eco na `B`, na porta 7000.
async fn rede(
    options: PipeOptions,
) -> (
    NetStack,
    NetStack,
    std::sync::Arc<tomni_ui_netstack::pipe::PipeStats>,
) {
    let (a, b) = PipeTunnel::pair(A, B, options);
    let stats = a.stats();
    let pilha_a = NetStack::spawn(a);
    let pilha_b = NetStack::spawn(b);
    let mut escuta = pilha_b.listen(7000).await.expect("escuta");
    tokio::spawn(async move {
        while let Ok(mut conexao) = escuta.accept().await {
            tokio::spawn(async move {
                let (mut leitura, mut escrita) = tokio::io::split(&mut conexao);
                let _ = tokio::io::copy(&mut leitura, &mut escrita).await;
                let _ = escrita.shutdown().await;
            });
        }
    });
    (pilha_a, pilha_b, stats)
}

fn destino() -> SocketAddr {
    "10.42.0.2:7000".parse().unwrap()
}

fn dados(len: usize, semente: u8) -> Vec<u8> {
    (0..len)
        .map(|i| {
            (i as u8)
                .wrapping_mul(31)
                .wrapping_add(semente)
                .wrapping_add((i >> 8) as u8)
        })
        .collect()
}

fn hash(bytes: &[u8]) -> u64 {
    let mut h = DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

/// Manda os bytes e lê de volta a mesma quantidade; só então fecha. Sem meio-fechamento no
/// meio do eco: o `smoltcp` 0.12 não reenvia dado perdido depois que o outro lado fechou
/// (CLOSE-WAIT), e o servidor do eco é uma pilha dele.
async fn eco(mut conexao: TcpStream, enviado: Vec<u8>) -> io::Result<Vec<u8>> {
    let (mut leitura, mut escrita) = tokio::io::split(&mut conexao);
    let envio = escrita.write_all(&enviado);
    let mut recebido = vec![0u8; enviado.len()];
    let volta = leitura.read_exact(&mut recebido);
    let (e, v) = tokio::join!(envio, volta);
    e?;
    v?;
    escrita.shutdown().await?;
    Ok(recebido)
}

#[tokio::test]
async fn eco_de_4_mib_volta_igual() {
    let (a, b, stats) = rede(PipeOptions::default()).await;
    let enviado = dados(4 * 1024 * 1024, 7);
    let conexao = a.connect(destino()).await.expect("conecta");
    let recebido = tokio::time::timeout(Duration::from_secs(60), eco(conexao, enviado.clone()))
        .await
        .expect("sem estourar o tempo")
        .expect("eco");
    assert_eq!(recebido.len(), enviado.len());
    assert_eq!(hash(&recebido), hash(&enviado));
    assert!(
        stats.max_len() <= 1300,
        "nenhum pacote passa do MTU: viu {}",
        stats.max_len()
    );
    a.shutdown().await;
    b.shutdown().await;
}

#[tokio::test]
async fn cinquenta_conexoes_simultaneas() {
    let (a, b, _) = rede(PipeOptions::default()).await;
    let mut tarefas = Vec::new();
    for n in 0..50u8 {
        let a = a.clone();
        tarefas.push(tokio::spawn(async move {
            let enviado = dados(64 * 1024, n);
            let conexao = a.connect(destino()).await?;
            let recebido = eco(conexao, enviado.clone()).await?;
            io::Result::Ok(hash(&recebido) == hash(&enviado) && recebido.len() == enviado.len())
        }));
    }
    for tarefa in tarefas {
        let ok = tokio::time::timeout(Duration::from_secs(60), tarefa)
            .await
            .expect("sem estourar o tempo")
            .expect("tarefa")
            .expect("eco");
        assert!(ok, "cada conexão recebe de volta o que mandou");
    }
    a.shutdown().await;
    b.shutdown().await;
}

#[tokio::test]
async fn eco_com_perda_de_5_por_cento_e_atraso_de_30_ms() {
    let options = PipeOptions {
        loss: 0.05,
        delay: Duration::from_millis(30),
        seed: 42,
        ..PipeOptions::default()
    };
    let (a, b, stats) = rede(options).await;
    // 1 MiB, e não os 4 MiB do teste sem perda: o `smoltcp` retransmite em bloco (sem ACK
    // seletivo), e com 5% de perda cada janela cheia perde algum pacote.
    let enviado = dados(1024 * 1024, 3);
    let conexao = a.connect(destino()).await.expect("conecta");
    let recebido = tokio::time::timeout(Duration::from_secs(180), eco(conexao, enviado.clone()))
        .await
        .expect("sem estourar o tempo")
        .expect("eco");
    assert_eq!(hash(&recebido), hash(&enviado));
    assert!(stats.dropped() > 0, "o sorteio perdeu pacotes de verdade");
    a.shutdown().await;
    b.shutdown().await;
}

#[tokio::test]
async fn rst_do_servidor_recusa_a_conexao() {
    // O app é invisível, então quem responde RST a uma porta fechada é o servidor do outro lado
    // da rede; aqui, o lado cru do canal faz esse papel.
    let (tunel, mut cru) = PipeTunnel::raw(A, PipeOptions::default());
    let pilha = NetStack::spawn(tunel);
    let servidor = tokio::spawn(async move {
        while let Some(pacote) = cru.recv().await {
            cru.send(comum::rst_ack(&pacote));
        }
    });
    let erro = pilha
        .connect("10.42.0.2:7001".parse().unwrap())
        .await
        .expect_err("o servidor recusou");
    assert_eq!(erro.kind(), io::ErrorKind::ConnectionRefused);
    assert!(
        erro.to_string().contains("recusou a conexão"),
        "mensagem em português: {erro}"
    );
    servidor.abort();
    pilha.shutdown().await;
}

#[tokio::test]
async fn fora_das_redes_do_tunel_falha_na_hora_sem_pacote() {
    let (a, b, stats) = rede(PipeOptions::default()).await;
    let antes = stats.sent();
    let erro = a
        .connect("192.168.1.10:22".parse().unwrap())
        .await
        .expect_err("fora da rede");
    assert_eq!(erro.kind(), io::ErrorKind::NetworkUnreachable);
    assert!(
        erro.to_string().contains("fora das redes do túnel"),
        "{erro}"
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(stats.sent(), antes, "nenhum pacote saiu");
    a.shutdown().await;
    b.shutdown().await;
}

#[tokio::test]
async fn os_dois_lados_fecham_e_a_pilha_segue() {
    let (a, b, _) = rede(PipeOptions::default()).await;
    for _ in 0..3 {
        let conexao = a.connect(destino()).await.expect("conecta");
        let recebido = eco(conexao, b"ola".to_vec()).await.expect("eco");
        assert_eq!(recebido, b"ola");
    }
    // Depois de fechar tudo, ainda abre conexões novas.
    let conexao = a.connect(destino()).await.expect("conecta de novo");
    drop(conexao);
    a.shutdown().await;
    b.shutdown().await;
}

#[tokio::test]
async fn host_inalcancavel_avisado_pelo_tunel_falha_a_conexao() {
    let (a, _b) = PipeTunnel::pair(A, B, PipeOptions::default());
    let a = a.with_dead_end("10.42.0.9".parse().unwrap());
    let pilha = NetStack::spawn(a);
    let erro = tokio::time::timeout(
        Duration::from_secs(5),
        pilha.connect("10.42.0.9:22".parse().unwrap()),
    )
    .await
    .expect("falha logo, sem esperar o tempo do TCP")
    .expect_err("inalcançável");
    assert_eq!(erro.kind(), io::ErrorKind::HostUnreachable);
    assert!(erro.to_string().contains("handshake"), "{erro}");
    pilha.shutdown().await;
}

#[tokio::test]
async fn connect_timeout_desiste_no_prazo() {
    // O outro lado do canal existe mas ninguém o lê: o SYN some e a conexão não anda.
    let (a, b) = PipeTunnel::pair(A, B, PipeOptions::default());
    let pilha = NetStack::spawn(a);
    let erro = pilha
        .connect_timeout(
            "10.42.0.2:7000".parse().unwrap(),
            Duration::from_millis(300),
        )
        .await
        .expect_err("ninguém responde");
    assert_eq!(erro.kind(), io::ErrorKind::TimedOut);
    assert!(erro.to_string().contains("não respondeu em"), "{erro}");
    drop(b);
    pilha.shutdown().await;
}
