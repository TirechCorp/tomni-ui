//! Os testes do meio Ethernet (M61): o adaptador `EthernetTunnel` sobre um fio de quadros em
//! memória. Só com a feature `ethernet`.
#![cfg(feature = "ethernet")]

use std::io;
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};
use std::time::Duration;

mod comum;

use comum::syn;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tomni_ui_netstack::ethernet::{EthernetTunnel, FrameTunnel, Route};
use tomni_ui_netstack::{Cidr, NetStack, PacketTunnel};

const MAC_EU: [u8; 6] = [0x02, 0, 0, 0, 0, 0x05];
const MAC_B: [u8; 6] = [0x02, 0, 0, 0, 0, 0x0b];
const MAC_GW: [u8; 6] = [0x02, 0, 0, 0, 0, 0x01];
const MAC_V: [u8; 6] = [0x02, 0, 0, 0, 0, 0x09];
const BROADCAST: [u8; 6] = [0xff; 6];
const EU: [u8; 4] = [10, 0, 0, 5];

/// Um dos lados do fio de quadros.
struct Fio {
    mac: [u8; 6],
    enderecos: Vec<Cidr>,
    rotas: Vec<Route>,
    saida: mpsc::UnboundedSender<Vec<u8>>,
    entrada: mpsc::UnboundedReceiver<Vec<u8>>,
    registro: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl FrameTunnel for Fio {
    fn mtu(&self) -> usize {
        1400
    }

    fn mac(&self) -> [u8; 6] {
        self.mac
    }

    fn addresses(&self) -> Vec<Cidr> {
        self.enderecos.clone()
    }

    fn routes(&self) -> Vec<Route> {
        self.rotas.clone()
    }

    fn send_frame(&mut self, quadro: &[u8]) {
        self.registro
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(quadro.to_vec());
        let _ = self.saida.send(quadro.to_vec());
    }

    fn poll_recv_frame(
        &mut self,
        cx: &mut Context<'_>,
        quadro: &mut Vec<u8>,
    ) -> Poll<io::Result<()>> {
        match self.entrada.poll_recv(cx) {
            Poll::Ready(Some(recebido)) => {
                quadro.clear();
                quadro.extend_from_slice(&recebido);
                Poll::Ready(Ok(()))
            }
            Poll::Ready(None) => Poll::Ready(Err(io::ErrorKind::BrokenPipe.into())),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// O lado "cru" do fio: o teste injeta e lê quadros à mão.
struct Cru {
    para: mpsc::UnboundedSender<Vec<u8>>,
    de: mpsc::UnboundedReceiver<Vec<u8>>,
}

impl Cru {
    fn manda(&self, quadro: Vec<u8>) {
        let _ = self.para.send(quadro);
    }

    /// O próximo quadro que o adaptador mandou (espera até 5 s).
    async fn proximo(&mut self) -> Vec<u8> {
        tokio::time::timeout(Duration::from_secs(5), self.de.recv())
            .await
            .expect("saiu um quadro")
            .expect("o fio ainda existe")
    }

    /// Todos os quadros que já saíram.
    fn tudo(&mut self) -> Vec<Vec<u8>> {
        let mut todos = Vec::new();
        while let Ok(quadro) = self.de.try_recv() {
            todos.push(quadro);
        }
        todos
    }
}

fn cidr(texto: &str) -> Cidr {
    texto.parse().expect("rede de teste")
}

fn rota(rede: &str, via: Option<[u8; 4]>) -> Route {
    Route {
        net: cidr(rede),
        via: via.map(Ipv4Addr::from),
    }
}

fn fio_cru(mac: [u8; 6], endereco: &str, rotas: Vec<Route>) -> (Fio, Cru) {
    let (para_fio, entrada) = mpsc::unbounded_channel();
    let (saida, de_fio) = mpsc::unbounded_channel();
    (
        Fio {
            mac,
            enderecos: vec![cidr(endereco)],
            rotas,
            saida,
            entrada,
            registro: Arc::default(),
        },
        Cru {
            para: para_fio,
            de: de_fio,
        },
    )
}

/// Dois fios ligados entre si.
fn par(a: ([u8; 6], &str), b: ([u8; 6], &str)) -> (Fio, Fio) {
    let (a_para_b, b_entrada) = mpsc::unbounded_channel();
    let (b_para_a, a_entrada) = mpsc::unbounded_channel();
    let monta = |(mac, endereco): ([u8; 6], &str), saida, entrada| Fio {
        mac,
        enderecos: vec![cidr(endereco)],
        rotas: vec![Route {
            net: cidr(endereco).network_cidr(),
            via: None,
        }],
        saida,
        entrada,
        registro: Arc::default(),
    };
    (monta(a, a_para_b, a_entrada), monta(b, b_para_a, b_entrada))
}

trait RedeDoEndereco {
    fn network_cidr(&self) -> Cidr;
}

impl RedeDoEndereco for Cidr {
    fn network_cidr(&self) -> Cidr {
        Cidr::new(self.network(), self.prefix()).expect("prefixo válido")
    }
}

fn arp_quadro(
    operacao: u16,
    destino: [u8; 6],
    sha: [u8; 6],
    spa: [u8; 4],
    tha: [u8; 6],
    tpa: [u8; 4],
) -> Vec<u8> {
    let mut q = Vec::new();
    q.extend_from_slice(&destino);
    q.extend_from_slice(&sha);
    q.extend_from_slice(&0x0806u16.to_be_bytes());
    q.extend_from_slice(&[0, 1, 8, 0, 6, 4]);
    q.extend_from_slice(&operacao.to_be_bytes());
    q.extend_from_slice(&sha);
    q.extend_from_slice(&spa);
    q.extend_from_slice(&tha);
    q.extend_from_slice(&tpa);
    q
}

fn ip_quadro(destino: [u8; 6], origem: [u8; 6], pacote: &[u8]) -> Vec<u8> {
    let mut q = Vec::new();
    q.extend_from_slice(&destino);
    q.extend_from_slice(&origem);
    q.extend_from_slice(&0x0800u16.to_be_bytes());
    q.extend_from_slice(pacote);
    q
}

/// `(operação, sha, spa, tha, tpa)` de um quadro ARP.
struct Arp {
    operacao: u16,
    destino: [u8; 6],
    sha: [u8; 6],
    spa: [u8; 4],
    tpa: [u8; 4],
}

fn arp(quadro: &[u8]) -> Option<Arp> {
    (quadro.len() == 42 && quadro[12..14] == [0x08, 0x06]).then(|| Arp {
        operacao: u16::from_be_bytes([quadro[20], quadro[21]]),
        destino: quadro[0..6].try_into().unwrap(),
        sha: quadro[22..28].try_into().unwrap(),
        spa: quadro[28..32].try_into().unwrap(),
        tpa: quadro[38..42].try_into().unwrap(),
    })
}

fn e_ip(quadro: &[u8]) -> bool {
    quadro.len() > 14 && quadro[12..14] == [0x08, 0x00]
}

/// Roda o adaptador por `por` (no relógio pausado do teste): processa os quadros que chegam e os
/// prazos do ARP.
async fn gira(adaptador: &mut EthernetTunnel<Fio>, por: Duration) {
    let mut buf = Vec::new();
    let _ = tokio::time::timeout(
        por,
        std::future::poll_fn(|cx| adaptador.poll_recv(cx, &mut buf)),
    )
    .await;
}

#[tokio::test(start_paused = true)]
async fn duas_pilhas_ligadas_por_quadros_fazem_arp_tcp_e_eco() {
    let (a, b) = par((MAC_EU, "10.42.0.1/24"), (MAC_B, "10.42.0.2/24"));
    let (registro_a, registro_b) = (a.registro.clone(), b.registro.clone());
    let pilha_a = NetStack::spawn(EthernetTunnel::new(a));
    // O lado que escuta responde ao ARP de qualquer um: o app de verdade não faz isso.
    let pilha_b = NetStack::spawn(EthernetTunnel::new(b).answer_all_arp(true));
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

    let mut conexao = pilha_a
        .connect("10.42.0.2:7000".parse().unwrap())
        .await
        .expect("conecta pelo ARP");
    let enviado: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
    let (mut leitura, mut escrita) = tokio::io::split(&mut conexao);
    let mut recebido = vec![0u8; enviado.len()];
    let (e, l) = tokio::join!(
        escrita.write_all(&enviado),
        leitura.read_exact(&mut recebido)
    );
    e.expect("envia");
    l.expect("recebe");
    assert_eq!(recebido, enviado, "o eco volta igual");

    let a_mandou = registro_a.lock().unwrap().clone();
    let pedidos: Vec<_> = a_mandou.iter().filter_map(|q| arp(q)).collect();
    assert_eq!(pedidos.len(), 1, "um único pedido ARP: {}", pedidos.len());
    assert_eq!(pedidos[0].operacao, 1);
    assert_eq!(pedidos[0].destino, BROADCAST);
    assert_eq!(pedidos[0].tpa, [10, 42, 0, 2]);
    assert!(
        a_mandou
            .iter()
            .filter(|q| e_ip(q))
            .all(|q| q[0..6] == MAC_B && q[6..12] == MAC_EU),
        "todo pacote IP sai com o MAC do vizinho"
    );
    let b_mandou = registro_b.lock().unwrap().clone();
    assert!(
        b_mandou
            .iter()
            .filter_map(|q| arp(q))
            .all(|p| p.operacao == 2),
        "o outro lado só respondeu: aprendeu o MAC pelo pedido"
    );
}

#[tokio::test(start_paused = true)]
async fn rota_com_gateway_o_arp_vai_ao_gateway_e_o_quadro_leva_o_ip_do_destino() {
    let (fio, mut cru) = fio_cru(
        MAC_EU,
        "10.0.0.5/24",
        vec![
            rota("10.0.0.0/24", None),
            rota("0.0.0.0/0", Some([10, 0, 0, 1])),
        ],
    );
    let pilha = NetStack::spawn(EthernetTunnel::new(fio));
    let tarefa = tokio::spawn(async move {
        let _ = pilha.connect("8.8.8.8:80".parse().unwrap()).await;
    });

    let pedido = arp(&cru.proximo().await).expect("o primeiro quadro é um ARP");
    assert_eq!(pedido.operacao, 1);
    assert_eq!(pedido.destino, BROADCAST);
    assert_eq!(
        pedido.tpa,
        [10, 0, 0, 1],
        "pergunta pelo gateway, não pelo destino"
    );
    assert_eq!(pedido.spa, EU);
    cru.manda(arp_quadro(2, MAC_EU, MAC_GW, [10, 0, 0, 1], MAC_EU, EU));

    let quadro = cru.proximo().await;
    assert!(e_ip(&quadro));
    assert_eq!(quadro[0..6], MAC_GW, "sai para o MAC do gateway");
    assert_eq!(
        quadro[14 + 16..14 + 20],
        [8, 8, 8, 8],
        "com o IP do destino"
    );
    assert_eq!(quadro[14 + 9], 6, "é TCP (o SYN)");
    tarefa.abort();
}

#[tokio::test(start_paused = true)]
async fn sem_resposta_ao_arp_sao_tres_pedidos_a_fila_cai_e_a_pilha_e_avisada() {
    let (fio, mut cru) = fio_cru(MAC_EU, "10.0.0.5/24", vec![rota("10.0.0.0/24", None)]);
    let pilha = NetStack::spawn(EthernetTunnel::new(fio));
    let erro = tokio::time::timeout(
        Duration::from_secs(30),
        pilha.connect("10.0.0.9:80".parse().unwrap()),
    )
    .await
    .expect("a pilha desiste antes do prazo do teste")
    .expect_err("ninguém respondeu");
    assert!(
        erro.to_string().contains("não respondeu ao ARP"),
        "a mensagem diz o porquê: {erro}"
    );
    tokio::time::sleep(Duration::from_secs(20)).await;
    let quadros = cru.tudo();
    let pedidos: Vec<_> = quadros.iter().filter_map(|q| arp(q)).collect();
    assert_eq!(pedidos.len(), 3, "exatamente três pedidos ARP");
    assert!(
        pedidos
            .iter()
            .all(|p| p.destino == BROADCAST && p.tpa == [10, 0, 0, 9])
    );
    assert!(
        quadros.iter().all(|q| !e_ip(q)),
        "nenhum pacote IP chegou a sair"
    );
}

#[tokio::test(start_paused = true)]
async fn invisivel_arp_de_quem_ninguem_procurou_e_syn_injetado_nao_tem_resposta() {
    let (fio, mut cru) = fio_cru(MAC_EU, "10.0.0.5/24", vec![rota("10.0.0.0/24", None)]);
    let pilha = NetStack::spawn(EthernetTunnel::new(fio));
    cru.manda(arp_quadro(1, BROADCAST, MAC_V, [10, 0, 0, 77], [0; 6], EU));
    cru.manda(ip_quadro(MAC_EU, MAC_V, &syn([10, 0, 0, 77], EU, 22)));
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        cru.tudo().is_empty(),
        "nem o ARP nem o SYN de um desconhecido recebem resposta"
    );
    pilha.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn controle_quem_o_app_procurou_recebe_resposta_ao_arp() {
    let (fio, mut cru) = fio_cru(MAC_EU, "10.0.0.5/24", vec![rota("10.0.0.0/24", None)]);
    let pilha = NetStack::spawn(EthernetTunnel::new(fio));
    let tarefa = tokio::spawn(async move {
        let _ = pilha.connect("10.0.0.77:80".parse().unwrap()).await;
    });
    let pedido = arp(&cru.proximo().await).expect("o app procura o 10.0.0.77");
    assert_eq!(pedido.tpa, [10, 0, 0, 77]);
    // O 10.0.0.77 pergunta pelo nosso MAC: agora responde, porque nós o procuramos antes.
    cru.manda(arp_quadro(1, BROADCAST, MAC_V, [10, 0, 0, 77], [0; 6], EU));
    let mut resposta = None;
    for _ in 0..3 {
        let quadro = cru.proximo().await;
        if let Some(p) = arp(&quadro) {
            resposta = Some(p);
            break;
        }
        assert!(e_ip(&quadro), "o SYN que esperava sai para o MAC aprendido");
        assert_eq!(quadro[0..6], MAC_V);
    }
    let resposta = resposta.expect("a resposta ARP");
    assert_eq!(resposta.operacao, 2);
    assert_eq!(resposta.destino, MAC_V);
    assert_eq!(resposta.sha, MAC_EU);
    assert_eq!(resposta.spa, EU);
    tarefa.abort();
}

#[tokio::test(start_paused = true)]
async fn controle_o_syn_numa_porta_em_escuta_so_sai_depois_de_o_app_procurar_o_mac() {
    let (fio, mut cru) = fio_cru(MAC_EU, "10.0.0.5/24", vec![rota("10.0.0.0/24", None)]);
    let pilha = NetStack::spawn(EthernetTunnel::new(fio));
    let _escuta = pilha.listen(9000).await.expect("escuta");
    cru.manda(ip_quadro(MAC_EU, MAC_V, &syn([10, 0, 0, 77], EU, 9000)));
    let pedido = arp(&cru.proximo().await).expect("o SYN-ACK precisa do MAC de quem pediu");
    assert_eq!(pedido.tpa, [10, 0, 0, 77]);
    cru.manda(arp_quadro(2, MAC_EU, MAC_V, [10, 0, 0, 77], MAC_EU, EU));
    let quadro = cru.proximo().await;
    assert!(e_ip(&quadro));
    assert_eq!(quadro[0..6], MAC_V);
    assert_eq!(quadro[14 + 20 + 13] & 0x12, 0x12, "SYN-ACK");
    pilha.shutdown().await;
}

fn pacote_para(destino: [u8; 4]) -> Vec<u8> {
    syn(EU, destino, 80)
}

#[tokio::test(start_paused = true)]
async fn o_proximo_salto_e_a_rota_de_mascara_mais_longa() {
    let (fio, mut cru) = fio_cru(
        MAC_EU,
        "10.0.0.5/24",
        vec![
            rota("10.0.0.0/24", None),
            rota("10.0.0.128/25", Some([10, 0, 0, 1])),
            rota("0.0.0.0/0", Some([10, 0, 0, 2])),
        ],
    );
    let mut t = EthernetTunnel::new(fio);
    t.send(&pacote_para([10, 0, 0, 200]));
    t.send(&pacote_para([10, 0, 0, 9]));
    t.send(&pacote_para([1, 1, 1, 1]));
    let alvos: Vec<[u8; 4]> = cru
        .tudo()
        .iter()
        .filter_map(|q| arp(q))
        .map(|p| p.tpa)
        .collect();
    assert_eq!(
        alvos,
        vec![[10, 0, 0, 1], [10, 0, 0, 9], [10, 0, 0, 2]],
        "/25 vence o /24, o /24 vence o padrão, o padrão pega o resto"
    );
}

#[tokio::test(start_paused = true)]
async fn sem_rota_mas_na_rede_de_um_endereco_nosso_vai_direto() {
    let (fio, mut cru) = fio_cru(MAC_EU, "10.0.0.5/24", Vec::new());
    let mut t = EthernetTunnel::new(fio);
    t.send(&pacote_para([10, 0, 0, 33]));
    t.send(&pacote_para([192, 168, 0, 1]));
    let alvos: Vec<[u8; 4]> = cru
        .tudo()
        .iter()
        .filter_map(|q| arp(q))
        .map(|p| p.tpa)
        .collect();
    assert_eq!(
        alvos,
        vec![[10, 0, 0, 33]],
        "fora das redes, o pacote não sai"
    );
}

#[tokio::test(start_paused = true)]
async fn as_rotas_do_tunel_sao_as_das_rotas_e_as_dos_enderecos() {
    let (fio, _cru) = fio_cru(
        MAC_EU,
        "10.0.0.5/24",
        vec![rota("172.16.0.0/16", Some([10, 0, 0, 1]))],
    );
    let t = EthernetTunnel::new(fio);
    let rotas = PacketTunnel::routes(&t);
    assert!(rotas.contains(&cidr("172.16.0.0/16")));
    assert!(
        rotas
            .iter()
            .any(|r| r.contains("10.0.0.99".parse().unwrap()))
    );
    assert_eq!(t.mtu(), 1400);
}

/// Aprende o MAC do `10.0.0.9` e devolve o adaptador com o cache cheio, no instante 0.
async fn com_vizinho() -> (EthernetTunnel<Fio>, Cru) {
    let (fio, mut cru) = fio_cru(MAC_EU, "10.0.0.5/24", vec![rota("10.0.0.0/24", None)]);
    let mut t = EthernetTunnel::new(fio);
    t.send(&pacote_para([10, 0, 0, 9]));
    let quadros = cru.tudo();
    assert_eq!(quadros.len(), 1, "só o pedido ARP");
    assert_eq!(arp(&quadros[0]).expect("ARP").tpa, [10, 0, 0, 9]);
    cru.manda(arp_quadro(2, MAC_EU, MAC_V, [10, 0, 0, 9], MAC_EU, EU));
    gira(&mut t, Duration::from_millis(10)).await;
    let quadros = cru.tudo();
    assert_eq!(
        quadros.len(),
        1,
        "a fila esvaziou: o pacote que esperava saiu"
    );
    assert!(e_ip(&quadros[0]));
    assert_eq!(quadros[0][0..6], MAC_V);
    (t, cru)
}

#[tokio::test(start_paused = true)]
async fn o_cache_vale_e_depois_de_um_minuto_em_uso_e_revalidado_por_pedido_unicast() {
    let (mut t, mut cru) = com_vizinho().await;

    // Aos 30 s: o cache vale, sai direto, sem ARP.
    gira(&mut t, Duration::from_secs(30)).await;
    t.send(&pacote_para([10, 0, 0, 9]));
    let quadros = cru.tudo();
    assert_eq!(quadros.len(), 1);
    assert!(e_ip(&quadros[0]), "sem ARP dentro do prazo");

    // Aos 61 s: em uso, mas ninguém o confirma há mais de 60 s: sai o pacote e um pedido unicast.
    gira(&mut t, Duration::from_secs(31)).await;
    t.send(&pacote_para([10, 0, 0, 9]));
    let quadros = cru.tudo();
    assert_eq!(quadros.len(), 2, "o pedido de revalidação e o pacote");
    let pedido = arp(&quadros[0]).expect("revalida");
    assert_eq!(pedido.operacao, 1);
    assert_eq!(pedido.destino, MAC_V, "unicast, para o MAC que já tínhamos");
    assert_eq!(pedido.tpa, [10, 0, 0, 9]);
    assert!(
        e_ip(&quadros[1]) && quadros[1][0..6] == MAC_V,
        "sem esperar a resposta"
    );

    // O vizinho confirma: acabam os pedidos, e o cache segue valendo.
    cru.manda(arp_quadro(2, MAC_EU, MAC_V, [10, 0, 0, 9], MAC_EU, EU));
    gira(&mut t, Duration::from_secs(5)).await;
    assert!(cru.tudo().is_empty(), "confirmado: nada mais de ARP");
    t.send(&pacote_para([10, 0, 0, 9]));
    let quadros = cru.tudo();
    assert_eq!(quadros.len(), 1);
    assert!(e_ip(&quadros[0]));
}

#[tokio::test(start_paused = true)]
async fn a_revalidacao_sem_resposta_esquece_o_vizinho_e_volta_ao_broadcast() {
    let (mut t, mut cru) = com_vizinho().await;
    // Em uso aos 30 s, para não ser esquecido por falta de uso.
    gira(&mut t, Duration::from_secs(30)).await;
    t.send(&pacote_para([10, 0, 0, 9]));
    cru.tudo();
    gira(&mut t, Duration::from_secs(31)).await;
    // Usado aos 61 s: revalida (1 pedido) e ninguém responde.
    t.send(&pacote_para([10, 0, 0, 9]));
    gira(&mut t, Duration::from_secs(5)).await;
    let pedidos: Vec<_> = cru.tudo().iter().filter_map(|q| arp(q)).collect();
    assert_eq!(pedidos.len(), 3, "o pedido de agora e mais dois, e desiste");
    assert!(pedidos.iter().all(|p| p.destino == MAC_V));
    // O próximo pacote procura o vizinho de novo, em broadcast, e espera.
    t.send(&pacote_para([10, 0, 0, 9]));
    let quadros = cru.tudo();
    assert_eq!(quadros.len(), 1);
    assert_eq!(arp(&quadros[0]).expect("ARP").destino, BROADCAST);
}

#[tokio::test(start_paused = true)]
async fn o_vizinho_que_ninguem_usa_por_um_minuto_e_esquecido() {
    let (mut t, mut cru) = com_vizinho().await;
    gira(&mut t, Duration::from_secs(61)).await;
    t.send(&pacote_para([10, 0, 0, 9]));
    let quadros = cru.tudo();
    assert_eq!(quadros.len(), 1, "só o pedido");
    let pedido = arp(&quadros[0]).expect("ARP");
    assert_eq!(pedido.destino, BROADCAST, "esquecido: procura de novo");
}

#[tokio::test(start_paused = true)]
async fn o_mac_que_muda_atualiza_o_cache() {
    let (mut t, mut cru) = com_vizinho().await;
    let novo = [0x02, 0, 0, 0, 0, 0x99];
    cru.manda(arp_quadro(1, BROADCAST, novo, [10, 0, 0, 9], [0; 6], EU));
    gira(&mut t, Duration::from_millis(10)).await;
    let resposta = cru.tudo();
    assert_eq!(resposta.len(), 1);
    assert_eq!(arp(&resposta[0]).expect("resposta").destino, novo);
    t.send(&pacote_para([10, 0, 0, 9]));
    let quadros = cru.tudo();
    assert_eq!(quadros[0][0..6], novo, "o pacote sai para o MAC novo");
}
