//! O laço do mapa contra o `ControlFake`: o mapa inteiro com e sem `zstd`, os deltas, o sinal de
//! vida, o silêncio, a queda e a volta do fluxo, a chave de nó desconhecida e a atualização de
//! endpoints.

use std::net::SocketAddr;
use std::time::Duration;

use tokio::time::timeout;
use tomni_ui_tailscale::control::map::{self, MapAnnounce, MapConfig, MapEvent, MapHandle};
use tomni_ui_tailscale::control::{Backoff, Control, ControlConfig, ControlError};
use tomni_ui_tailscale::key::{DiscoKey, MachineKey, NodeKey, NodePublic};
use tomni_ui_tailscale::tailcfg::{EndpointType, Hostinfo, PeerChange};
use tomni_ui_tailscale::testing::{ControlFake, FakeOptions, fake_node};

struct Ambiente {
    fake: ControlFake,
    node_key: NodePublic,
    config: MapConfig,
    control: Control,
}

async fn ambiente(opcoes: FakeOptions) -> Ambiente {
    let fake = ControlFake::with_options(opcoes).await;
    let node_key = NodeKey::generate().public();
    fake.register_node(node_key);
    fake.set_self(fake_node(1, "eu", "100.64.0.1"));
    fake.set_peer(fake_node(2, "servidor", "100.64.0.2"));
    let mut c = ControlConfig::new(fake.url());
    c.backoff = Backoff::new(Duration::from_millis(30), Duration::from_millis(120));
    let control = Control::new(c, MachineKey::generate()).unwrap();
    let config = MapConfig::new(node_key, DiscoKey::generate().public());
    Ambiente {
        fake,
        node_key,
        config,
        control,
    }
}

async fn proximo(h: &mut MapHandle) -> MapEvent {
    timeout(Duration::from_secs(5), h.recv())
        .await
        .expect("o laço devia avisar em 5 s")
        .expect("o laço terminou")
}

/// Espera o próximo `Update` (pulando `Connected`) e o devolve.
async fn proximo_update(
    h: &mut MapHandle,
) -> (
    std::sync::Arc<tomni_ui_tailscale::netmap::NetMap>,
    tomni_ui_tailscale::netmap::Change,
    bool,
) {
    loop {
        match proximo(h).await {
            MapEvent::Update {
                netmap,
                change,
                first,
            } => return (netmap, change, first),
            MapEvent::Connected => {}
            outro => panic!("esperava Update, veio {outro:?}"),
        }
    }
}

async fn nada_por(h: &mut MapHandle, tempo: Duration) {
    if let Ok(e) = timeout(tempo, h.recv()).await {
        panic!("não devia avisar nada, veio {e:?}");
    }
}

#[tokio::test]
async fn recebe_o_mapa_inteiro_com_zstd() {
    let a = ambiente(FakeOptions::default()).await;
    let mut h = map::spawn(a.control, a.config);
    assert!(matches!(proximo(&mut h).await, MapEvent::Connected));
    let (netmap, change, first) = proximo_update(&mut h).await;
    assert!(first);
    assert!(change.me);
    assert_eq!(change.peers_added, vec![2]);
    assert_eq!(netmap.me().unwrap().id, 1);
    assert_eq!(netmap.peers().count(), 1);

    let pedidos = a.fake.map_requests();
    assert_eq!(pedidos.len(), 1);
    let p = &pedidos[0];
    assert_eq!(p.request.node_key, a.node_key);
    assert!(p.request.stream && p.request.keep_alive);
    assert_eq!(p.raw["Compress"], "zstd");
    assert_eq!(p.raw["Stream"], true);
    assert_eq!(p.raw["KeepAlive"], true);
    h.stop().await.unwrap();
}

#[tokio::test]
async fn sem_zstd_nao_pede_compressao() {
    let mut a = ambiente(FakeOptions::default()).await;
    a.config.compress = false;
    let mut h = map::spawn(a.control, a.config);
    let (netmap, _, first) = proximo_update(&mut h).await;
    assert!(first);
    assert_eq!(netmap.peers().count(), 1);
    let p = &a.fake.map_requests()[0];
    assert!(p.raw.get("Compress").is_none_or(|v| v == ""), "{}", p.raw);
    h.stop().await;
}

#[tokio::test]
async fn servidor_que_ignora_o_zstd_tambem_serve() {
    let opcoes = FakeOptions {
        map_compress: false,
        ..FakeOptions::default()
    };
    let a = ambiente(opcoes).await;
    let mut h = map::spawn(a.control, a.config);
    let (netmap, _, _) = proximo_update(&mut h).await;
    assert_eq!(netmap.peers().count(), 1);
    // O cliente pediu zstd mesmo assim.
    assert_eq!(a.fake.map_requests()[0].raw["Compress"], "zstd");
    h.stop().await;
}

#[tokio::test]
async fn deltas_de_pares_chegam_como_mudancas() {
    let a = ambiente(FakeOptions::default()).await;
    let mut h = map::spawn(a.control, a.config);
    proximo_update(&mut h).await;

    a.fake.set_peer(fake_node(3, "banco", "100.64.0.3"));
    let (netmap, change, first) = proximo_update(&mut h).await;
    assert!(!first);
    assert_eq!(change.peers_added, vec![3]);
    assert!(change.routes);
    assert_eq!(netmap.peers().count(), 2);

    a.fake.patch_peer(PeerChange {
        node_id: 3,
        derp_region: Some(7),
        ..PeerChange::default()
    });
    let (netmap, change, _) = proximo_update(&mut h).await;
    assert_eq!(change.peers_updated, vec![3]);
    assert!(!change.routes);
    assert_eq!(netmap.peer(3).unwrap().home_derp, 7);

    a.fake.remove_peer(2);
    let (netmap, change, _) = proximo_update(&mut h).await;
    assert_eq!(change.peers_removed, vec![2]);
    assert!(netmap.peer(2).is_none());
    assert_eq!(netmap.peers().count(), 1);
    h.stop().await;
}

#[tokio::test]
async fn sinal_de_vida_nao_vira_aviso() {
    let opcoes = FakeOptions {
        map_keepalive: Some(Duration::from_millis(30)),
        ..FakeOptions::default()
    };
    let a = ambiente(opcoes).await;
    let mut h = map::spawn(a.control, a.config);
    proximo_update(&mut h).await;
    nada_por(&mut h, Duration::from_millis(300)).await;
    assert_eq!(a.fake.map_streams_opened(), 1);
    h.stop().await;
}

#[tokio::test]
async fn silencio_derruba_o_fluxo_e_reconecta_com_o_mapa_inteiro() {
    let mut a = ambiente(FakeOptions::default()).await;
    a.config.silence = Duration::from_millis(250);
    let mut h = map::spawn(a.control, a.config);
    proximo_update(&mut h).await;
    match proximo(&mut h).await {
        MapEvent::Disconnected(e) => assert!(matches!(*e, ControlError::Silent), "{e:?}"),
        outro => panic!("esperava Disconnected, veio {outro:?}"),
    }
    assert!(matches!(proximo(&mut h).await, MapEvent::Connected));
    let (netmap, _, first) = proximo_update(&mut h).await;
    assert!(first);
    assert_eq!(netmap.peers().count(), 1);
    assert_eq!(a.fake.map_streams_opened(), 2);
    h.stop().await;
}

#[tokio::test]
async fn fluxo_derrubado_volta_e_o_que_mudou_enquanto_isso_aparece() {
    let a = ambiente(FakeOptions::default()).await;
    let mut h = map::spawn(a.control, a.config);
    proximo_update(&mut h).await;

    a.fake.close_map_streams();
    // Muda o mapa com o fluxo fora do ar: o par 2 sai e o 4 entra.
    a.fake.remove_peer(2);
    a.fake.set_peer(fake_node(4, "novo", "100.64.0.4"));
    assert!(matches!(proximo(&mut h).await, MapEvent::Disconnected(_)));
    assert!(matches!(proximo(&mut h).await, MapEvent::Connected));
    let (netmap, change, first) = proximo_update(&mut h).await;
    assert!(first);
    assert!(netmap.peer(2).is_none(), "o par removido tem de sumir");
    assert!(netmap.peer(4).is_some());
    assert_eq!(change.peers_removed, vec![2]);
    assert_eq!(change.peers_added, vec![4]);
    assert_eq!(a.fake.map_streams_opened(), 2);
    h.stop().await;
}

#[tokio::test]
async fn chave_de_no_desconhecida_e_falha_definitiva() {
    let a = ambiente(FakeOptions::default()).await;
    let mut config = a.config;
    config.node_key = NodeKey::generate().public();
    let mut h = map::spawn(a.control, config);
    match proximo(&mut h).await {
        MapEvent::Failed(e) => {
            assert!(!e.is_transient(), "{e:?}");
            assert!(e.to_string().contains("401"), "{e}");
        }
        outro => panic!("esperava Failed, veio {outro:?}"),
    }
    assert!(
        timeout(Duration::from_secs(2), h.recv())
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn endpoints_novos_vao_em_outro_pedido_sem_derrubar_o_fluxo() {
    let a = ambiente(FakeOptions::default()).await;
    let mut h = map::spawn(a.control, a.config);
    proximo_update(&mut h).await;

    let ep: SocketAddr = "203.0.113.9:41641".parse().unwrap();
    h.announce(MapAnnounce {
        endpoints: vec![(ep, EndpointType::STUN)],
        hostinfo: Hostinfo {
            hostname: "maquina".into(),
            shields_up: true,
            ..Hostinfo::default()
        },
    });
    let mut atualizacao = None;
    for _ in 0..100 {
        if let Some(p) = a
            .fake
            .map_requests()
            .into_iter()
            .find(|p| !p.request.stream)
        {
            atualizacao = Some(p);
            break;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    let p = atualizacao.expect("o pedido de atualização não chegou");
    assert!(p.request.omit_peers);
    assert!(!p.request.read_only);
    assert_eq!(p.request.endpoints, vec![ep]);
    assert_eq!(p.request.endpoint_types, vec![EndpointType::STUN]);
    assert_eq!(p.raw["Hostinfo"]["ShieldsUp"], true);
    assert_eq!(a.fake.map_streams_opened(), 1);
    assert_eq!(a.fake.open_map_streams(), 1);
    // Anunciar a mesma coisa de novo não manda nada.
    let antes = a.fake.map_requests().len();
    h.announce(MapAnnounce {
        endpoints: vec![(ep, EndpointType::STUN)],
        hostinfo: Hostinfo {
            hostname: "maquina".into(),
            shields_up: true,
            ..Hostinfo::default()
        },
    });
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(a.fake.map_requests().len(), antes);
    h.stop().await;
}

#[tokio::test]
async fn rota_nova_no_meio_do_caminho_aparece_em_routes() {
    let a = ambiente(FakeOptions::default()).await;
    let mut h = map::spawn(a.control, a.config);
    proximo_update(&mut h).await;

    let mut no = fake_node(2, "servidor", "100.64.0.2");
    no.allowed_ips.push("192.168.50.0/24".into());
    no.primary_routes.push("192.168.50.0/24".into());
    a.fake.set_peer(no);
    let (netmap, change, _) = proximo_update(&mut h).await;
    assert!(change.routes);
    assert_eq!(change.peers_updated, vec![2]);
    let rotas = netmap.routes();
    assert!(
        rotas.iter().any(|c| c.to_string() == "192.168.50.0/24"),
        "{rotas:?}"
    );
    h.stop().await;
}

#[tokio::test]
async fn parar_devolve_o_control_ainda_conectado() {
    let a = ambiente(FakeOptions::default()).await;
    let mut h = map::spawn(a.control, a.config);
    proximo_update(&mut h).await;
    let control = h.stop().await.expect("o laço devolve o Control");
    assert!(control.is_connected());
    assert_eq!(a.fake.connections(), 1);
}
