//! O cliente DERP contra o `DerpFake`: a conexão, a troca de pacotes entre dois clientes, o aviso
//! de que o outro lado não está, a região de casa e a volta depois da queda.

use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::timeout;
use tomni_ui_tailscale::derp::{Client, DerpTarget, Event};
use tomni_ui_tailscale::key::NodeKey;
use tomni_ui_tailscale::testing::DerpFake;
use tomni_ui_tailscale::upgrade::Roots;

type Eventos = mpsc::UnboundedReceiver<(i32, Event)>;

fn cliente(fake: &DerpFake, chave: &NodeKey, casa: bool) -> (Client, Eventos) {
    let (tx, rx) = mpsc::unbounded_channel();
    let alvo = DerpTarget::from_node(&fake.node(1)).expect("alvo");
    (
        Client::spawn(alvo, chave.clone(), Roots::system(), casa, tx),
        rx,
    )
}

async fn espera<F: Fn(&Event) -> bool>(rx: &mut Eventos, serve: F) -> Event {
    timeout(Duration::from_secs(5), async {
        loop {
            let (regiao, e) = rx.recv().await.expect("canal fechado");
            assert_eq!(regiao, 1);
            if serve(&e) {
                return e;
            }
        }
    })
    .await
    .expect("o evento devia chegar em 5 s")
}

#[tokio::test]
async fn dois_clientes_trocam_pacotes() {
    let fake = DerpFake::start().await;
    let (ka, kb) = (NodeKey::generate(), NodeKey::generate());
    let (a, mut ea) = cliente(&fake, &ka, true);
    let (b, mut eb) = cliente(&fake, &kb, false);
    espera(&mut ea, |e| matches!(e, Event::Connected)).await;
    espera(&mut eb, |e| matches!(e, Event::Connected)).await;
    assert!(a.send(kb.public(), b"ola".to_vec()));
    match espera(&mut eb, |e| matches!(e, Event::Packet { .. })).await {
        Event::Packet { from, data } => {
            assert_eq!(from, ka.public());
            assert_eq!(data, b"ola");
        }
        _ => unreachable!(),
    }
    assert!(b.send(ka.public(), vec![7; 1200]));
    match espera(&mut ea, |e| matches!(e, Event::Packet { .. })).await {
        Event::Packet { from, data } => {
            assert_eq!(from, kb.public());
            assert_eq!(data.len(), 1200);
        }
        _ => unreachable!(),
    }
    assert!(fake.relayed_packets() >= 2);
}

#[tokio::test]
async fn destino_ausente_vira_peer_gone() {
    let fake = DerpFake::start().await;
    let ka = NodeKey::generate();
    let (a, mut ea) = cliente(&fake, &ka, true);
    espera(&mut ea, |e| matches!(e, Event::Connected)).await;
    let ausente = NodeKey::generate().public();
    assert!(a.send(ausente, b"x".to_vec()));
    match espera(&mut ea, |e| matches!(e, Event::PeerGone(_))).await {
        Event::PeerGone(k) => assert_eq!(k, ausente),
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn a_regiao_de_casa_chega_ao_servidor() {
    let fake = DerpFake::start().await;
    let ka = NodeKey::generate();
    let (a, mut ea) = cliente(&fake, &ka, false);
    espera(&mut ea, |e| matches!(e, Event::Connected)).await;
    assert!(!fake.is_preferred(&ka.public()));
    a.note_preferred(true);
    timeout(Duration::from_secs(5), async {
        while !fake.is_preferred(&ka.public()) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("o servidor devia saber da região de casa");
}

#[tokio::test]
async fn reconecta_depois_da_queda() {
    let fake = DerpFake::start().await;
    let (ka, kb) = (NodeKey::generate(), NodeKey::generate());
    let (a, mut ea) = cliente(&fake, &ka, true);
    let (_b, mut eb) = cliente(&fake, &kb, false);
    espera(&mut ea, |e| matches!(e, Event::Connected)).await;
    espera(&mut eb, |e| matches!(e, Event::Connected)).await;
    fake.drop_connections();
    espera(&mut ea, |e| matches!(e, Event::Disconnected(_))).await;
    espera(&mut ea, |e| matches!(e, Event::Connected)).await;
    espera(&mut eb, |e| matches!(e, Event::Connected)).await;
    assert!(a.send(kb.public(), b"de novo".to_vec()));
    match espera(&mut eb, |e| matches!(e, Event::Packet { .. })).await {
        Event::Packet { data, .. } => assert_eq!(data, b"de novo"),
        _ => unreachable!(),
    }
    assert!(fake.connections() >= 4);
}
