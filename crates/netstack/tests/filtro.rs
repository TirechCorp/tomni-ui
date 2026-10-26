//! O filtro de entrada: para a rede, o app é invisível.

use std::time::Duration;

use tomni_ui_netstack::NetStack;
use tomni_ui_netstack::pipe::{PipeOptions, PipeTunnel};

mod comum;
use comum::{echo_icmp, syn};

const EU: [u8; 4] = [10, 42, 0, 1];
const OUTRO: [u8; 4] = [10, 42, 0, 2];

#[tokio::test]
async fn syn_e_ping_para_o_app_nao_recebem_resposta() {
    let (tunel, mut cru) = PipeTunnel::raw("10.42.0.1/24", PipeOptions::default());
    let pilha = NetStack::spawn(tunel);
    cru.send(syn(OUTRO, EU, 22));
    cru.send(syn(OUTRO, EU, 8080));
    cru.send(echo_icmp(OUTRO, EU));
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        cru.try_recv().is_none(),
        "o app não responde SYN, RST nem ping: é invisível"
    );
    pilha.shutdown().await;
}

#[tokio::test]
async fn controle_a_mesma_syn_numa_porta_em_escuta_recebe_syn_ack() {
    let (tunel, mut cru) = PipeTunnel::raw("10.42.0.1/24", PipeOptions::default());
    let pilha = NetStack::spawn(tunel);
    let _escuta = pilha.listen(9000).await.expect("escuta");
    cru.send(syn(OUTRO, EU, 9000));
    let resposta = tokio::time::timeout(Duration::from_secs(2), cru.recv())
        .await
        .expect("responde")
        .expect("pacote");
    // IPv4 + TCP com SYN e ACK (0x12).
    assert_eq!(resposta[9], 6, "é TCP");
    assert_eq!(resposta[20 + 13] & 0x12, 0x12, "SYN-ACK");
    pilha.shutdown().await;
}
