//! As rotas que mudam com o túnel no ar (`PacketTunnel::poll_routes`), como as do Tailscale.

use std::io;
use std::net::SocketAddr;
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tomni_ui_netstack::pipe::{PipeOptions, PipeTunnel};
use tomni_ui_netstack::{Cidr, NetStack, PacketTunnel};

/// Um túnel de memória que troca as rotas quando o teste manda.
struct Dinamico {
    dentro: PipeTunnel,
    rotas: Vec<Cidr>,
    novas: mpsc::UnboundedReceiver<Vec<Cidr>>,
}

impl PacketTunnel for Dinamico {
    fn mtu(&self) -> usize {
        self.dentro.mtu()
    }
    fn addresses(&self) -> Vec<Cidr> {
        self.dentro.addresses()
    }
    fn routes(&self) -> Vec<Cidr> {
        self.rotas.clone()
    }
    fn poll_routes(&mut self, cx: &mut Context<'_>) -> Poll<Vec<Cidr>> {
        self.novas.poll_recv(cx).map(Option::unwrap_or_default)
    }
    fn send(&mut self, packet: &[u8]) {
        self.dentro.send(packet);
    }
    fn poll_recv(&mut self, cx: &mut Context<'_>, buf: &mut Vec<u8>) -> Poll<io::Result<()>> {
        self.dentro.poll_recv(cx, buf)
    }
}

fn cidr(texto: &str) -> Cidr {
    texto.parse().unwrap()
}

#[tokio::test]
async fn uma_rota_nova_vale_para_as_conexoes_seguintes() {
    let (a, b) = PipeTunnel::pair("10.42.0.1/24", "10.42.0.2/24", PipeOptions::default());
    let (avisa, novas) = mpsc::unbounded_channel();
    let a = Dinamico {
        dentro: a,
        // Começa sem alcançar ninguém.
        rotas: vec![cidr("192.0.2.0/24")],
        novas,
    };
    let pilha_a = NetStack::spawn(a);
    let pilha_b = NetStack::spawn(b);
    let mut escuta = pilha_b.listen(7000).await.unwrap();
    tokio::spawn(async move {
        while let Ok(mut conexao) = escuta.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 4];
                if conexao.read_exact(&mut buf).await.is_ok() {
                    let _ = conexao.write_all(&buf).await;
                }
            });
        }
    });
    let destino: SocketAddr = "10.42.0.2:7000".parse().unwrap();

    assert_eq!(pilha_a.routes().as_ref(), &[cidr("192.0.2.0/24")]);
    assert!(!pilha_a.routes_to(destino.ip()));
    let erro = pilha_a
        .connect_timeout(destino, Duration::from_secs(2))
        .await
        .unwrap_err();
    assert_eq!(erro.kind(), io::ErrorKind::NetworkUnreachable, "{erro}");

    // O mapa mudou: a máquina do outro lado apareceu.
    avisa
        .send(vec![cidr("192.0.2.0/24"), cidr("10.42.0.2/32")])
        .unwrap();
    let inicio = std::time::Instant::now();
    while !pilha_a.routes_to(destino.ip()) {
        assert!(
            inicio.elapsed() < Duration::from_secs(5),
            "a rota não chegou"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(pilha_a.routes().len(), 2);
    let mut conexao = pilha_a
        .connect_timeout(destino, Duration::from_secs(5))
        .await
        .unwrap();
    conexao.write_all(b"ping").await.unwrap();
    let mut volta = [0u8; 4];
    conexao.read_exact(&mut volta).await.unwrap();
    assert_eq!(&volta, b"ping");

    // E a rota some: as conexões novas voltam a ser recusadas na hora.
    avisa.send(vec![cidr("192.0.2.0/24")]).unwrap();
    let inicio = std::time::Instant::now();
    while pilha_a.routes_to(destino.ip()) {
        assert!(inicio.elapsed() < Duration::from_secs(5), "a rota não saiu");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let erro = pilha_a
        .connect_timeout(destino, Duration::from_secs(2))
        .await
        .unwrap_err();
    assert_eq!(erro.kind(), io::ErrorKind::NetworkUnreachable, "{erro}");
}

#[tokio::test]
async fn sem_poll_routes_as_rotas_ficam_como_nasceram() {
    let (a, _b) = PipeTunnel::pair("10.42.0.1/24", "10.42.0.2/24", PipeOptions::default());
    let pilha = NetStack::spawn(a);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(pilha.routes().as_ref(), &[cidr("10.42.0.2/24")]);
}
