//! O VL1 inteiro sobre a rede simulada: o motor do app contra um raiz falso que fala o
//! protocolo de verdade (HELLO, OK, WHOIS, ECHO, relay). Tempo pausado do `tokio`: nada espera
//! de verdade.

use std::fs;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tomni_ui_netstack::{NatKind, SimEndpoint, SimNet, Underlay};
use tomni_ui_zerotier::engine::{Engine, Events};
use tomni_ui_zerotier::node::{NodeState, State, ZeroTier, ZeroTierBuilder};
use tomni_ui_zerotier::packet::Verb;
use tomni_ui_zerotier::peer::{PeerRole, RemoteVersion};
use tomni_ui_zerotier::testing::controller::{ControllerFake, ControllerStats, RedeTeste};
use tomni_ui_zerotier::testing::{RootFake, RootStats};
use tomni_ui_zerotier::vl1::{Config, Effect};
use tomni_ui_zerotier::world::Root;
use tomni_ui_zerotier::{Address, Identity, World, WorldKind};

fn identidade(n: u32) -> Identity {
    let caminho = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(format!("tests/fixtures/identidades/id{n}.secret"));
    Identity::from_secret_str(fs::read_to_string(caminho).unwrap().trim()).unwrap()
}

fn publica(i: &Identity) -> Identity {
    Identity::from_public_str(&i.to_public_string()).unwrap()
}

/// O planet de teste: um raiz (a identidade 1) em 10.0.0.1:9993, assinado pela identidade 3.
fn planet(timestamp: u64) -> World {
    let raiz = Root {
        identity: publica(&identidade(1)),
        endpoints: vec!["10.0.0.1:9993".parse().unwrap()],
    };
    World::make(
        WorldKind::Planet,
        149_604_618,
        timestamp,
        &identidade(3),
        vec![raiz],
        &identidade(3),
    )
    .unwrap()
}

fn config(identity: Identity, planet: World) -> Config {
    Config {
        identity,
        planet,
        moons: Vec::new(),
        orbits: Vec::new(),
        version: RemoteVersion {
            protocol: 11,
            major: 1,
            minor: 4,
            revision: 7,
        },
        direct_paths: true,
    }
}

struct Cenario {
    rede: SimNet,
    raiz: RootFake,
}

fn cenario(mundo_do_raiz: World) -> Cenario {
    let rede = SimNet::new();
    let raiz = RootFake::start(identidade(1), mundo_do_raiz, rede.host("10.0.0.1", 9993));
    Cenario { rede, raiz }
}

fn no(c: &Cenario, ip: &str, id: Identity, planet: World) -> (Engine, Events) {
    Engine::start(c.rede.host(ip, 9993), config(id, planet))
}

async fn espera_ate<F: FnMut() -> bool>(mut condicao: F) {
    for _ in 0..600 {
        if condicao() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("a condição não chegou a tempo");
}

#[tokio::test(start_paused = true)]
async fn o_hello_ao_raiz_volta_com_ok_e_o_caminho_e_a_latencia() {
    let c = cenario(planet(100));
    let (motor, mut eventos) = no(&c, "10.0.0.2", identidade(2), planet(100));

    espera_ate(|| RootStats::get(&c.raiz.stats().hellos) >= 1).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let foto = motor.snapshot().await.unwrap();
    let raiz = foto
        .peers
        .iter()
        .find(|p| p.address == c.raiz.address())
        .expect("o raiz é um par");
    assert_eq!(raiz.role, PeerRole::Root);
    assert!(raiz.alive, "o OK(HELLO) abriu o caminho");
    assert_eq!(raiz.paths, vec![c.raiz.socket_addr()]);
    assert_eq!(raiz.version.unwrap().protocol, 11);
    assert!(raiz.latency.is_some());

    // O raiz nos viu no endereço certo: vira um evento para o app anunciar.
    let mut visto = None;
    while let Ok(e) = eventos.try_recv() {
        if let Effect::SeenAddress(a) = e {
            visto = Some(a);
        }
    }
    assert_eq!(visto, Some("10.0.0.2:9993".parse().unwrap()));
}

#[tokio::test(start_paused = true)]
async fn o_whois_traz_a_identidade_e_o_pacote_que_esperava_sai() {
    let c = cenario(planet(100));
    let (a, _ea) = no(&c, "10.0.0.2", identidade(2), planet(100));
    let (b, mut eb) = no(&c, "10.0.0.3", identidade(3), planet(100));
    let endereco_b = identidade(3).address();
    let endereco_a = identidade(2).address();

    // Os dois se apresentam ao raiz.
    espera_ate(|| c.raiz.known().len() == 2).await;
    tokio::time::sleep(Duration::from_secs(1)).await;

    // A não conhece B: o envio espera o WHOIS e depois sai pelo raiz.
    a.send(endereco_b, Verb::Nop, Vec::new());
    espera_ate(|| RootStats::get(&c.raiz.stats().whois) >= 1).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let foto = a.snapshot().await.unwrap();
    assert!(foto.peers.iter().any(|p| p.address == endereco_b));
    assert!(RootStats::get(&c.raiz.stats().relayed) >= 1);
    drop((b, &mut eb, endereco_a));
}

#[tokio::test(start_paused = true)]
async fn um_echo_passa_pelo_raiz_e_so_e_respondido_a_quem_procuramos() {
    let c = cenario(planet(100));
    // Sem o RENDEZVOUS do raiz: ninguém apresentou A a B.
    c.raiz.introduce(false);
    let (a, _ea) = no(&c, "10.0.0.2", identidade(2), planet(100));
    let (b, _eb) = no(&c, "10.0.0.3", identidade(3), planet(100));
    let endereco_a = identidade(2).address();
    let endereco_b = identidade(3).address();

    espera_ate(|| c.raiz.known().len() == 2).await;
    tokio::time::sleep(Duration::from_secs(1)).await;

    // A procura B: WHOIS, depois o ECHO sai pelo raiz até B.
    a.send(endereco_b, Verb::Echo, b"ola".to_vec());
    espera_ate(|| RootStats::get(&c.raiz.stats().relayed) >= 1).await;
    // B não procurou A: pergunta ao raiz quem é, mas não responde (o app é invisível).
    espera_ate(|| RootStats::get(&c.raiz.stats().whois) >= 2).await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(RootStats::get(&c.raiz.stats().relayed), 1, "B ficou calado");

    // Agora B procura A, que já tinha procurado B: A responde e o OK volta pelo raiz.
    b.send(endereco_a, Verb::Echo, b"oi".to_vec());
    espera_ate(|| RootStats::get(&c.raiz.stats().relayed) >= 3).await;
}

#[tokio::test(start_paused = true)]
async fn o_ok_do_hello_traz_o_planet_mais_novo() {
    let c = cenario(planet(500));
    let (motor, mut eventos) = no(&c, "10.0.0.2", identidade(2), planet(100));

    espera_ate(|| RootStats::get(&c.raiz.stats().hellos) >= 1).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let foto = motor.snapshot().await.unwrap();
    assert_eq!(foto.planet.1, 500, "o planet foi atualizado");
    let mut mudou = false;
    while let Ok(e) = eventos.try_recv() {
        mudou |= matches!(e, Effect::WorldChanged);
    }
    assert!(mudou, "o app é avisado para gravar o planet");
}

#[tokio::test(start_paused = true)]
async fn o_planet_assinado_por_outra_chave_e_recusado() {
    let falso = {
        let raiz = Root {
            identity: publica(&identidade(1)),
            endpoints: vec!["10.0.0.1:9993".parse().unwrap()],
        };
        // Mesmo id e mais novo, mas assinado pela identidade 2, não pela 3.
        World::make(
            WorldKind::Planet,
            149_604_618,
            900,
            &identidade(2),
            vec![raiz],
            &identidade(2),
        )
        .unwrap()
    };
    let c = cenario(falso);
    let (motor, _e) = no(&c, "10.0.0.2", identidade(2), planet(100));
    espera_ate(|| RootStats::get(&c.raiz.stats().hellos) >= 1).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(motor.snapshot().await.unwrap().planet.1, 100);
}

#[tokio::test(start_paused = true)]
async fn o_app_e_invisivel_um_hello_de_desconhecido_nao_tem_resposta() {
    let c = cenario(planet(100));
    let (motor, _e) = no(&c, "10.0.0.2", identidade(2), planet(100));
    espera_ate(|| RootStats::get(&c.raiz.stats().hellos) >= 1).await;

    // Um nó que não é raiz, nem foi procurado, manda HELLO ao app: nada volta.
    let intruso = c.rede.host("10.0.0.9", 9993);
    let sem_resposta = tomni_ui_zerotier::testing::hello_de_intruso(
        &intruso,
        identidade(3),
        identidade(2),
        "10.0.0.2:9993".parse().unwrap(),
    )
    .await;
    assert!(sem_resposta, "o app não responde a quem não conhece");
    drop(motor);

    // Controle: o mesmo HELLO, mandado a um raiz, tem resposta. Sem isso o silêncio acima
    // poderia ser um HELLO mal formado.
    let outro = c.rede.host("10.0.0.8", 9993);
    let sem_resposta = tomni_ui_zerotier::testing::hello_de_intruso(
        &outro,
        identidade(2),
        identidade(1),
        c.raiz.socket_addr(),
    )
    .await;
    assert!(!sem_resposta, "o raiz responde ao mesmo HELLO");
}

/// Um HELLO na armadura estendida de `de` para `para`, com a Salsa armada pela chave que
/// `de` combinou com `chave_com` (a certa é a de `para`).
fn hello_estendido(de: &Identity, para: &Identity, chave_com: &Identity) -> Vec<u8> {
    use tomni_ui_zerotier::crypto::{PeerKey, armor::extended_armor, salsa};
    use tomni_ui_zerotier::packet::Packet;
    let chave = PeerKey::from_agreement(&de.agree(&publica(chave_com)).unwrap());
    let mut p = Packet::new(0x1234, para.address(), de.address(), Verb::Hello);
    p.set_extended_armor(true);
    p.append(&[13, 1, 14]);
    p.append(&2u16.to_be_bytes());
    p.append(&1000u64.to_be_bytes());
    p.append(&de.to_wire());
    let mut fim = Vec::new();
    tomni_ui_zerotier::inet::write(&mut fim, None);
    p.append(&fim);
    p.append(&planet(100).id().to_be_bytes());
    p.append(&planet(100).timestamp().to_be_bytes());
    let inicio_cifrado = p.len();
    p.append(&0u16.to_be_bytes());
    let mut bytes = p.into_bytes();
    salsa::crypt_field(&chave.salsa, &mut bytes, inicio_cifrado);
    salsa::armor(&chave.salsa, &mut bytes, false);
    extended_armor(&mut bytes, &publica(para)).unwrap();
    bytes
}

#[test]
fn hello_na_armadura_estendida_com_o_mac_da_salsa_errado_e_recusado() {
    let mut vl1 = tomni_ui_zerotier::vl1::Vl1::new(Config {
        version: RemoteVersion {
            protocol: 13,
            major: 1,
            minor: 14,
            revision: 2,
        },
        ..config(identidade(2), planet(100))
    });
    let de_raiz: SocketAddr = "10.0.0.1:9993".parse().unwrap();
    // O mesmo pacote, só com a Salsa armada pela chave de outra identidade: a camada
    // estendida abre, mas o MAC de dentro não confere, e nada é respondido.
    let falso = hello_estendido(&identidade(1), &identidade(2), &identidade(3));
    vl1.receive(1_000_000, de_raiz, &falso);
    assert!(
        !vl1.take_effects()
            .iter()
            .any(|e| matches!(e, Effect::Send { .. })),
        "um HELLO estendido sem o MAC da Salsa certo foi respondido"
    );
    // Controle: o certo é aceito e respondido.
    let certo = hello_estendido(&identidade(1), &identidade(2), &identidade(2));
    vl1.receive(1_000_100, de_raiz, &certo);
    assert!(
        vl1.take_effects()
            .iter()
            .any(|e| matches!(e, Effect::Send { .. })),
        "o HELLO estendido certo não foi respondido"
    );
}

#[tokio::test(start_paused = true)]
async fn um_pacote_grande_chega_remontado_depois_do_relay() {
    let c = cenario(planet(100));
    let (a, _ea) = no(&c, "10.0.0.2", identidade(2), planet(100));
    let (_b, mut eb) = no(&c, "10.0.0.3", identidade(3), planet(100));
    let endereco_a = identidade(2).address();
    let endereco_b = identidade(3).address();

    espera_ate(|| c.raiz.known().len() == 2).await;
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Três mil bytes que não comprimem: vão em três pedaços e o MAC cobre o pacote inteiro.
    let mut carga = vec![0u8; 3000];
    let mut x = 0x1234_5678_u32;
    for byte in &mut carga {
        x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        *byte = (x >> 24) as u8;
    }
    a.send(endereco_b, Verb::ExtFrame, carga.clone());

    let entregue = loop {
        match tokio::time::timeout(Duration::from_secs(30), eb.recv()).await {
            Ok(Some(Effect::Deliver(d))) => break d,
            Ok(Some(_)) => {}
            _ => panic!("o pacote grande não chegou a B"),
        }
    };
    assert_eq!(entregue.from, endereco_a);
    assert_eq!(entregue.verb, Verb::ExtFrame);
    assert_eq!(entregue.payload, carga);
    assert_eq!(entregue.hops, 1, "passou pelo raiz");

    // Comprimível: a mesma coisa, mas a carga vai em LZ4 e chega inteira.
    let repetida = vec![7u8; 3000];
    a.send_with(endereco_b, Verb::ExtFrame, repetida.clone(), true);
    let entregue = loop {
        match tokio::time::timeout(Duration::from_secs(30), eb.recv()).await {
            Ok(Some(Effect::Deliver(d))) => break d,
            Ok(Some(_)) => {}
            _ => panic!("o pacote comprimido não chegou a B"),
        }
    };
    assert_eq!(entregue.payload, repetida);
}

// ---------------------------------------------------------------------------------------------
// A rede virtual (VL2) inteira: raiz, controlador e dois nós do app na rede simulada.
// ---------------------------------------------------------------------------------------------

/// O raiz, o controlador (identidade 4, rede privada padrão) e a rede simulada.
struct Mundo {
    rede: SimNet,
    raiz: RootFake,
    ctrl: ControllerFake,
}

fn mundo(rede_teste: RedeTeste) -> Mundo {
    let rede = SimNet::new();
    let raiz = RootFake::start(identidade(1), planet(100), rede.host("10.0.0.1", 9993));
    let ctrl = ControllerFake::start(
        identidade(4),
        planet(100),
        1,
        rede_teste,
        rede.host("10.0.0.9", 9993),
    );
    Mundo { rede, raiz, ctrl }
}

fn estado_de(n: u32) -> NodeState {
    NodeState {
        identity: Some(identidade(n)),
        ..NodeState::default()
    }
}

fn construtor(m: &Mundo, estado: NodeState) -> ZeroTierBuilder {
    ZeroTierBuilder::new(m.ctrl.network(), estado).planet(planet(100).to_bytes())
}

async fn inicia(m: &Mundo, ip: &str, n: u32) -> ZeroTier {
    inicia_com(m, ip, n, |b| b).await
}

/// Como `inicia`, deixando o teste ajustar o construtor. O lado que escuta responde ao ARP
/// de qualquer um, como no teste do adaptador.
async fn inicia_com(
    m: &Mundo,
    ip: &str,
    n: u32,
    ajusta: impl FnOnce(ZeroTierBuilder) -> ZeroTierBuilder,
) -> ZeroTier {
    ajusta(construtor(m, estado_de(n)).answer_all_arp(true))
        .start_on(m.rede.host(ip, 9993))
        .await
        .expect("o nó sobe")
}

/// Espera o estado do nó satisfazer a condição (o tempo é virtual: 300 s custam nada).
async fn espera_estado(zt: &ZeroTier, ok: impl Fn(&State) -> bool) -> State {
    for _ in 0..3000 {
        let e = zt.state();
        if ok(&e) {
            return e;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("o estado não chegou a tempo; agora: {:?}", zt.state());
}

/// Sobe um servidor de eco na porta `porta` da pilha do nó.
async fn eco(zt: &ZeroTier, porta: u16) {
    let mut escuta = zt
        .stack()
        .expect("pilha")
        .listen(porta)
        .await
        .expect("escuta");
    tokio::spawn(async move {
        while let Ok(mut conexao) = escuta.accept().await {
            tokio::spawn(async move {
                let (mut leitura, mut escrita) = tokio::io::split(&mut conexao);
                let _ = tokio::io::copy(&mut leitura, &mut escrita).await;
                let _ = escrita.shutdown().await;
            });
        }
    });
}

/// Manda `n` bytes pseudoaleatórios pelo eco e confere que voltam iguais.
async fn confere_eco(origem: &ZeroTier, destino: std::net::Ipv4Addr, porta: u16, n: usize) {
    let mut conexao = origem
        .stack()
        .expect("pilha")
        .connect_timeout(
            std::net::SocketAddr::new(destino.into(), porta),
            Duration::from_secs(120),
        )
        .await
        .expect("conecta");
    let mut x = 0x9e37_79b9_u32;
    let enviado: Vec<u8> = (0..n)
        .map(|_| {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (x >> 24) as u8
        })
        .collect();
    let mut recebido = vec![0u8; n];
    let (mut leitura, mut escrita) = tokio::io::split(&mut conexao);
    let (e, l) = tokio::join!(
        escrita.write_all(&enviado),
        leitura.read_exact(&mut recebido)
    );
    e.expect("envia");
    l.expect("recebe");
    assert!(recebido == enviado, "o eco volta igual");
}

#[tokio::test(start_paused = true)]
async fn sem_autorizacao_espera_e_ao_autorizar_conecta_sem_reiniciar() {
    let m = mundo(RedeTeste::padrao("teste"));
    let a = inicia(&m, "10.0.0.2", 2).await;
    let e = espera_estado(&a, |e| matches!(e, State::NotAuthorized { .. })).await;
    assert_eq!(
        e,
        State::NotAuthorized {
            address: a.address()
        }
    );
    assert!(ControllerStats::get(&m.ctrl.stats().recusas) >= 1);

    m.ctrl.authorize(a.address());
    let e = espera_estado(&a, |e| matches!(e, State::Connected { .. })).await;
    let State::Connected { address, name, .. } = e else {
        unreachable!()
    };
    assert_eq!(address, "10.61.0.2".parse::<std::net::Ipv4Addr>().unwrap());
    assert_eq!(name, "teste");
}

/// Sobe `a` e `b` autorizados e conectados, `b` com o eco na porta 7.
async fn dois_conectados(m: &Mundo) -> (ZeroTier, ZeroTier, std::net::Ipv4Addr) {
    let a = inicia(m, "10.0.0.2", 2).await;
    let b = inicia(m, "10.0.0.3", 5).await;
    m.ctrl.authorize(a.address());
    m.ctrl.authorize(b.address());
    let ip_b = b
        .wait_connected(Duration::from_secs(300))
        .await
        .expect("b conecta");
    a.wait_connected(Duration::from_secs(300))
        .await
        .expect("a conecta");
    eco(&b, 7).await;
    (a, b, ip_b)
}

#[tokio::test(start_paused = true)]
async fn eco_tcp_entre_dois_nos_pelo_ip_gerenciado() {
    let m = mundo(RedeTeste::padrao("teste"));
    let (a, _b, ip_b) = dois_conectados(&m).await;
    confere_eco(&a, ip_b, 7, 1024 * 1024).await;
    assert!(
        RootStats::get(&m.raiz.stats().relayed) > 100,
        "o começo passa pelo raiz, até o RENDEZVOUS abrir o caminho direto"
    );
}

#[tokio::test(start_paused = true)]
async fn so_pelo_raiz_o_eco_funciona_e_o_raiz_conta_os_repasses() {
    let m = mundo(RedeTeste::padrao("teste"));
    let a = inicia_com(&m, "10.0.0.2", 2, |b| b.direct_paths(false)).await;
    let b = inicia_com(&m, "10.0.0.3", 5, |b| b.direct_paths(false)).await;
    m.ctrl.authorize(a.address());
    m.ctrl.authorize(b.address());
    let ip_b = b.wait_connected(Duration::from_secs(300)).await.unwrap();
    a.wait_connected(Duration::from_secs(300)).await.unwrap();
    eco(&b, 7).await;
    let antes = RootStats::get(&m.raiz.stats().relayed);
    confere_eco(&a, ip_b, 7, 200_000).await;
    assert!(RootStats::get(&m.raiz.stats().relayed) > antes + 100);
}

#[tokio::test(start_paused = true)]
async fn rota_com_gateway_o_quadro_chega_ao_gateway() {
    let mut rede = RedeTeste::padrao("teste");
    rede.rotas
        .push(tomni_ui_zerotier::netconf::route::ManagedRoute {
            target: "192.168.50.0/24".parse().unwrap(),
            via: Some("10.61.0.20".parse().unwrap()),
            flags: 0,
            metric: 0,
        });
    let m = mundo(rede);
    let a = inicia(&m, "10.0.0.2", 2).await;
    // `b` é o gateway: aceita pacotes para qualquer IP e responde como se fosse o destino.
    let b = inicia_com(&m, "10.0.0.3", 5, |b| b.any_ip(true)).await;
    m.ctrl.assign_ip(b.address(), "10.61.0.20".parse().unwrap());
    m.ctrl.authorize(a.address());
    m.ctrl.authorize(b.address());
    b.wait_connected(Duration::from_secs(300)).await.unwrap();
    a.wait_connected(Duration::from_secs(300)).await.unwrap();
    eco(&b, 7).await;
    confere_eco(&a, "192.168.50.7".parse().unwrap(), 7, 50_000).await;
}

/// Um underlay que conta a suíte dos pacotes que saem endereçados a `alvo` (a suíte está em
/// claro nas flags): `[Salsa20/12, AES-GMAC-SIV, armadura estendida]`.
struct Espia {
    dentro: SimEndpoint,
    alvo: Address,
    suites: Arc<[AtomicU64; 3]>,
}

impl Underlay for Espia {
    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.dentro.local_addr()
    }

    fn try_send_to(&self, buf: &[u8], to: SocketAddr) -> io::Result<()> {
        // Os fragmentos seguintes trazem 0xff no lugar da origem: só contam os pacotes inteiros.
        if buf.len() > 28 && buf[13] != 0xff && Address::from_bytes(&buf[8..]) == Some(self.alvo) {
            match (buf[18] >> 3) & 7 {
                1 => self.suites[0].fetch_add(1, Ordering::Relaxed),
                3 => self.suites[1].fetch_add(1, Ordering::Relaxed),
                _ => 0,
            };
            if buf[18] & 0x80 != 0 {
                self.suites[2].fetch_add(1, Ordering::Relaxed);
            }
        }
        self.dentro.try_send_to(buf, to)
    }

    fn poll_recv_from(
        &self,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<(usize, SocketAddr)>> {
        self.dentro.poll_recv_from(cx, buf)
    }

    fn local_ip_toward(&self, remote: SocketAddr) -> Option<IpAddr> {
        self.dentro.local_ip_toward(remote)
    }
}

/// Dois nós com os protocolos anunciados `pa` e `pb` (`ajusta` mexe nos dois construtores),
/// trocando 200 kB de eco: devolve as suítes dos pacotes que `a` mandou a `b` depois que os
/// dois se conheceram e passou um tempo de HELLOs periódicos.
async fn cifras_com(
    pa: u8,
    pb: u8,
    ajusta: impl Fn(ZeroTierBuilder) -> ZeroTierBuilder,
) -> [u64; 3] {
    let m = mundo(RedeTeste::padrao("teste"));
    let suites = Arc::new([AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)]);
    let a = ajusta(
        construtor(&m, estado_de(2))
            .answer_all_arp(true)
            .protocol_version(pa),
    )
    .start_on(Espia {
        dentro: m.rede.host("10.0.0.2", 9993),
        alvo: identidade(5).address(),
        suites: suites.clone(),
    })
    .await
    .unwrap();
    let b = inicia_com(&m, "10.0.0.3", 5, |b| ajusta(b.protocol_version(pb))).await;
    m.ctrl.authorize(a.address());
    m.ctrl.authorize(b.address());
    let ip_b = b.wait_connected(Duration::from_secs(300)).await.unwrap();
    a.wait_connected(Duration::from_secs(300)).await.unwrap();
    eco(&b, 7).await;
    // O começo ainda não sabe a versão do outro: só o que vem depois vale.
    confere_eco(&a, ip_b, 7, 1000).await;
    tokio::time::sleep(Duration::from_secs(30)).await;
    let antes = [
        suites[0].load(Ordering::Relaxed),
        suites[1].load(Ordering::Relaxed),
        suites[2].load(Ordering::Relaxed),
    ];
    confere_eco(&a, ip_b, 7, 200_000).await;
    tokio::time::sleep(Duration::from_secs(300)).await;
    confere_eco(&a, ip_b, 7, 1000).await;
    [
        suites[0].load(Ordering::Relaxed) - antes[0],
        suites[1].load(Ordering::Relaxed) - antes[1],
        suites[2].load(Ordering::Relaxed) - antes[2],
    ]
}

async fn cifras_entre(pa: u8, pb: u8) -> [u64; 2] {
    let [salsa, aes, _] = cifras_com(pa, pb, |b| b).await;
    [salsa, aes]
}

/// A moon de teste: o id 0x77, uma semente (a identidade 7, em 10.0.0.7:9993) e dono (a
/// identidade 7, que também assina).
fn moon_de_teste(timestamp: u64) -> World {
    let raiz = Root {
        identity: publica(&identidade(7)),
        endpoints: vec!["10.0.0.7:9993".parse().unwrap()],
    };
    World::make(
        WorldKind::Moon,
        0x77,
        timestamp,
        &identidade(7),
        vec![raiz],
        &identidade(7),
    )
    .unwrap()
}

#[tokio::test(start_paused = true)]
async fn orbitar_uma_moon_traz_o_mundo_da_semente_e_guarda_para_a_proxima_vez() {
    let m = mundo(RedeTeste::padrao("teste"));
    let semente = RootFake::start(identidade(7), planet(100), m.rede.host("10.0.0.7", 9993));
    semente.serve_moon(moon_de_teste(500));
    // O raiz do planet conhece a semente, como um raiz de verdade conheceria.
    m.raiz
        .know(publica(&identidade(7)), "10.0.0.7:9993".parse().unwrap());
    let guardado = Arc::new(Mutex::new(None::<NodeState>));
    let g = guardado.clone();
    let a = construtor(&m, estado_de(2))
        .orbit(0x77, identidade(7).address())
        .on_node_state(move |e| *g.lock().unwrap() = Some(e.clone()))
        .start_on(m.rede.host("10.0.0.2", 9993))
        .await
        .unwrap();
    m.ctrl.authorize(a.address());
    let mut achou = false;
    for _ in 0..600 {
        if a.roots()
            .await
            .iter()
            .any(|r| r.address == identidade(7).address() && r.role == PeerRole::Moon)
        {
            achou = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        achou,
        "a semente não virou raiz de moon: {:?}",
        a.roots().await
    );
    // A moon vai para o estado que o app grava.
    tokio::time::sleep(Duration::from_secs(5)).await;
    let estado = guardado
        .lock()
        .unwrap()
        .clone()
        .expect("o estado foi entregue");
    assert_eq!(estado.moons.len(), 1, "a moon não foi guardada");
    assert_eq!(World::parse(&estado.moons[0]).unwrap().id(), 0x77);
    a.stop(Duration::from_secs(2)).await;

    // Na próxima vez a moon já vem do estado, sem a semente responder ao pedido.
    let b = construtor(&m, estado)
        .start_on(m.rede.host("10.0.0.3", 9993))
        .await
        .unwrap();
    let roots = b.roots().await;
    assert!(
        roots
            .iter()
            .any(|r| r.address == identidade(7).address() && r.role == PeerRole::Moon),
        "{roots:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn moon_que_ninguem_pediu_ou_com_assinatura_ruim_nao_entra() {
    let m = mundo(RedeTeste::padrao("teste"));
    let semente = RootFake::start(identidade(7), planet(100), m.rede.host("10.0.0.7", 9993));
    // Sem `orbit`, a moon que a semente oferece (se pedisse) não vale; e a do id pedido, com
    // a assinatura de outro, também não.
    let raiz = Root {
        identity: publica(&identidade(7)),
        endpoints: vec!["10.0.0.7:9993".parse().unwrap()],
    };
    let falsa = World::make(
        WorldKind::Moon,
        0x77,
        500,
        &identidade(6),
        vec![raiz],
        &identidade(7),
    )
    .unwrap();
    // Declara a chave de atualização do 6 mas foi assinada pelo 7: não confere.
    semente.serve_moon(falsa);
    m.raiz
        .know(publica(&identidade(7)), "10.0.0.7:9993".parse().unwrap());
    let a = construtor(&m, estado_de(2))
        .orbit(0x77, identidade(7).address())
        .start_on(m.rede.host("10.0.0.2", 9993))
        .await
        .unwrap();
    m.ctrl.authorize(a.address());
    tokio::time::sleep(Duration::from_secs(120)).await;
    assert!(
        !a.roots()
            .await
            .iter()
            .any(|r| r.address == identidade(7).address()),
        "uma moon inválida virou raiz"
    );
}

#[tokio::test(start_paused = true)]
async fn o_hello_na_armadura_estendida_so_vai_a_quem_anuncia_o_13() {
    // Desligado (o padrão), nada sai na armadura estendida.
    let [_, _, estendidos] = cifras_com(13, 13, |b| b).await;
    assert_eq!(estendidos, 0);
    // Ligado nos dois, depois do primeiro HELLO em claro os seguintes vão estendidos e o par
    // continua alcançável (o eco do fim passou).
    let [_, aes, estendidos] = cifras_com(13, 13, |b| b.extended_hello(true)).await;
    assert!(estendidos > 0, "nenhum HELLO estendido");
    assert!(aes > 50, "o resto segue em AES: {aes}");
    // O outro lado no 11 não entende: tudo em claro.
    let [_, _, estendidos] = cifras_com(13, 11, |b| b.extended_hello(true)).await;
    assert_eq!(estendidos, 0, "para quem anuncia 11");
}

#[tokio::test(start_paused = true)]
async fn dois_nos_com_protocolo_13_falam_em_aes_gmac_siv() {
    let [salsa, aes] = cifras_entre(13, 13).await;
    assert!(aes > 50, "aes {aes}");
    assert_eq!(salsa, 0, "nada em Salsa depois que os dois se conhecem");
}

#[tokio::test(start_paused = true)]
async fn com_um_lado_no_protocolo_11_a_conversa_e_em_salsa() {
    for (pa, pb) in [(11, 13), (13, 11), (11, 11)] {
        let [salsa, aes] = cifras_entre(pa, pb).await;
        assert!(salsa > 50, "{pa}/{pb}: salsa {salsa}");
        assert_eq!(aes, 0, "{pa}/{pb}: um lado sem AES");
    }
}

#[tokio::test(start_paused = true)]
async fn as_regras_da_rede_descartam_a_porta_7_e_deixam_a_8() {
    let mut rede = RedeTeste::padrao("teste");
    rede.regras = vec![
        tomni_ui_zerotier::netconf::rules::Rule::dest_port(7, 7, false),
        tomni_ui_zerotier::netconf::rules::Rule::drop(),
        tomni_ui_zerotier::netconf::rules::Rule::accept(),
    ];
    let m = mundo(rede);
    let (a, b, ip_b) = dois_conectados(&m).await;
    eco(&b, 8).await;
    confere_eco(&a, ip_b, 8, 20_000).await;
    let erro = a
        .stack()
        .unwrap()
        .connect_timeout(
            std::net::SocketAddr::new(ip_b.into(), 7),
            Duration::from_secs(20),
        )
        .await
        .expect_err("a porta 7 está descartada");
    assert_eq!(erro.kind(), std::io::ErrorKind::TimedOut, "{erro}");
}

#[tokio::test(start_paused = true)]
async fn o_controlador_muda_o_ip_e_a_pilha_e_refeita() {
    let m = mundo(RedeTeste::padrao("teste"));
    let (a, b, ip_b) = dois_conectados(&m).await;
    let antigo = a.wait_connected(Duration::from_secs(5)).await.unwrap();
    let novo: std::net::Ipv4Addr = "10.61.0.50".parse().unwrap();
    assert_ne!(antigo, novo);
    m.ctrl.assign_ip(a.address(), novo);
    m.ctrl.push(a.address());
    espera_estado(
        &a,
        |e| matches!(e, State::Connected { address, .. } if *address == novo),
    )
    .await;
    confere_eco(&a, ip_b, 7, 30_000).await;
    drop(b);
}

#[tokio::test(start_paused = true)]
async fn com_o_controlador_fora_do_ar_sobe_com_a_configuracao_guardada() {
    let m = mundo(RedeTeste::padrao("teste"));
    let guardado = std::sync::Arc::new(std::sync::Mutex::new(NodeState::default()));
    let copia = guardado.clone();
    let a = construtor(&m, estado_de(2))
        .on_node_state(move |e| *copia.lock().unwrap() = e.clone())
        .start_on(m.rede.host("10.0.0.2", 9993))
        .await
        .unwrap();
    m.ctrl.authorize(a.address());
    let ip = a.wait_connected(Duration::from_secs(300)).await.unwrap();
    assert!(
        !guardado.lock().unwrap().network_config.is_empty(),
        "a configuração foi guardada"
    );
    a.stop(Duration::from_secs(5)).await;

    // O controlador some. O nó reinicia com o que guardou.
    let pedidos = ControllerStats::get(&m.ctrl.stats().pedidos);
    let rede_id = m.ctrl.network();
    drop(m.ctrl);
    let estado = guardado.lock().unwrap().clone();
    let a = ZeroTierBuilder::new(rede_id, estado)
        .planet(planet(100).to_bytes())
        .start_on(m.rede.host("10.0.0.2", 9993))
        .await
        .unwrap();
    let ip2 = a.wait_connected(Duration::from_secs(300)).await.unwrap();
    assert_eq!(ip2, ip, "o mesmo IP, sem o controlador");
    assert!(pedidos > 0);
}

#[tokio::test(start_paused = true)]
async fn invisivel_quem_nao_procuramos_nao_tem_resposta_ao_arp_nem_ao_syn() {
    let m = mundo(RedeTeste::padrao("teste"));
    let a = inicia(&m, "10.0.0.2", 2).await;
    let b = inicia(&m, "10.0.0.3", 5).await;
    m.ctrl.authorize(a.address());
    m.ctrl.authorize(b.address());
    let ip_a = a.wait_connected(Duration::from_secs(300)).await.unwrap();
    b.wait_connected(Duration::from_secs(300)).await.unwrap();
    // `b` procura `a` (ARP + SYN), mas `a` nunca procurou `b`: nada volta, nem RST.
    let erro = b
        .stack()
        .unwrap()
        .connect_timeout(
            std::net::SocketAddr::new(ip_a.into(), 22),
            Duration::from_secs(30),
        )
        .await
        .expect_err("a não responde a quem não procurou");
    assert_eq!(erro.kind(), std::io::ErrorKind::TimedOut, "{erro}");
}

/// Sobe `a` e `b` cada um atrás do seu NAT (do tipo dado), o raiz e o controlador públicos.
async fn atras_de_nats(m: &Mundo, tipo: NatKind) -> (ZeroTier, ZeroTier, std::net::Ipv4Addr) {
    atras_de_nats_com(m, tipo, true).await
}

/// Como `atras_de_nats`, ligando ou desligando o PUSH_DIRECT_PATHS dos dois nós.
async fn atras_de_nats_com(
    m: &Mundo,
    tipo: NatKind,
    push: bool,
) -> (ZeroTier, ZeroTier, std::net::Ipv4Addr) {
    let nat_a = m.rede.nat("203.0.113.1", tipo);
    let nat_b = m.rede.nat("203.0.113.2", tipo);
    let a = construtor(m, estado_de(2))
        .answer_all_arp(true)
        .push_direct_paths(push)
        .start_on(nat_a.endpoint("192.168.1.10", 9993))
        .await
        .unwrap();
    let b = construtor(m, estado_de(5))
        .answer_all_arp(true)
        .push_direct_paths(push)
        .start_on(nat_b.endpoint("192.168.2.10", 9993))
        .await
        .unwrap();
    m.ctrl.authorize(a.address());
    m.ctrl.authorize(b.address());
    let ip_b = b.wait_connected(Duration::from_secs(300)).await.unwrap();
    a.wait_connected(Duration::from_secs(300)).await.unwrap();
    eco(&b, 7).await;
    (a, b, ip_b)
}

/// O caminho que `a` usa para `b`: `direto (…)` ou `pelo raiz`.
async fn caminho_ate(a: &ZeroTier, b: &ZeroTier) -> String {
    a.peers()
        .await
        .into_iter()
        .find(|p| p.address == b.address())
        .map(|p| p.path)
        .unwrap_or_default()
}

#[tokio::test(start_paused = true)]
async fn nat_de_cone_nos_dois_lados_o_rendezvous_leva_ao_caminho_direto() {
    let m = mundo(RedeTeste::padrao("teste"));
    let (a, b, ip_b) = atras_de_nats(&m, NatKind::FullCone).await;
    confere_eco(&a, ip_b, 7, 20_000).await;
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert!(RootStats::get(&m.raiz.stats().rendezvous) >= 2);
    assert!(caminho_ate(&a, &b).await.starts_with("direto"));
    assert!(caminho_ate(&b, &a).await.starts_with("direto"));

    // Com o caminho direto, o raiz para de repassar.
    let antes = RootStats::get(&m.raiz.stats().relayed);
    confere_eco(&a, ip_b, 7, 200_000).await;
    let depois = RootStats::get(&m.raiz.stats().relayed);
    assert!(
        depois - antes < 10,
        "o raiz repassou {} pacotes",
        depois - antes
    );
}

#[tokio::test(start_paused = true)]
async fn nat_com_restricao_de_endereco_e_porta_tambem_fura() {
    let m = mundo(RedeTeste::padrao("teste"));
    let (a, b, ip_b) = atras_de_nats(&m, NatKind::Restricted).await;
    confere_eco(&a, ip_b, 7, 20_000).await;
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert!(caminho_ate(&a, &b).await.starts_with("direto"));
    let antes = RootStats::get(&m.raiz.stats().relayed);
    confere_eco(&a, ip_b, 7, 200_000).await;
    assert!(RootStats::get(&m.raiz.stats().relayed) - antes < 10);
}

#[tokio::test(start_paused = true)]
async fn nat_simetrico_fica_pelo_raiz_e_funciona() {
    let m = mundo(RedeTeste::padrao("teste"));
    let (a, b, ip_b) = atras_de_nats(&m, NatKind::Symmetric).await;
    let antes = RootStats::get(&m.raiz.stats().relayed);
    confere_eco(&a, ip_b, 7, 200_000).await;
    tokio::time::sleep(Duration::from_secs(20)).await;
    assert_eq!(caminho_ate(&a, &b).await, "pelo raiz");
    assert!(RootStats::get(&m.raiz.stats().relayed) - antes > 100);
    confere_eco(&a, ip_b, 7, 20_000).await;
}

#[tokio::test(start_paused = true)]
async fn so_o_rendezvous_do_raiz_ja_fura_o_nat() {
    let m = mundo(RedeTeste::padrao("teste"));
    let (a, b, ip_b) = atras_de_nats_com(&m, NatKind::Restricted, false).await;
    confere_eco(&a, ip_b, 7, 20_000).await;
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert!(RootStats::get(&m.raiz.stats().rendezvous) >= 2);
    assert!(caminho_ate(&a, &b).await.starts_with("direto"));
    assert!(caminho_ate(&b, &a).await.starts_with("direto"));
}

#[tokio::test(start_paused = true)]
async fn so_o_push_dos_enderecos_ja_fura_o_nat() {
    let m = mundo(RedeTeste::padrao("teste"));
    m.raiz.introduce(false);
    let (a, b, ip_b) = atras_de_nats(&m, NatKind::Restricted).await;
    confere_eco(&a, ip_b, 7, 20_000).await;
    // O push sai a cada 15 s enquanto não há caminho direto.
    tokio::time::sleep(Duration::from_secs(40)).await;
    assert_eq!(RootStats::get(&m.raiz.stats().rendezvous), 0);
    assert!(caminho_ate(&a, &b).await.starts_with("direto"));
    assert!(caminho_ate(&b, &a).await.starts_with("direto"));
}

#[tokio::test(start_paused = true)]
async fn o_caminho_direto_fica_memorizado_e_e_tentado_ao_reiniciar() {
    let m = mundo(RedeTeste::padrao("teste"));
    let nat_a = m.rede.nat("203.0.113.1", NatKind::FullCone);
    let nat_b = m.rede.nat("203.0.113.2", NatKind::FullCone);
    let guardado = std::sync::Arc::new(std::sync::Mutex::new(NodeState::default()));
    let copia = guardado.clone();
    let a = construtor(&m, estado_de(2))
        .answer_all_arp(true)
        .on_node_state(move |e| *copia.lock().unwrap() = e.clone())
        .start_on(nat_a.endpoint("192.168.1.10", 9993))
        .await
        .unwrap();
    let b = construtor(&m, estado_de(5))
        .answer_all_arp(true)
        .start_on(nat_b.endpoint("192.168.2.10", 9993))
        .await
        .unwrap();
    m.ctrl.authorize(a.address());
    m.ctrl.authorize(b.address());
    let ip_b = b.wait_connected(Duration::from_secs(300)).await.unwrap();
    a.wait_connected(Duration::from_secs(300)).await.unwrap();
    eco(&b, 7).await;
    confere_eco(&a, ip_b, 7, 20_000).await;
    tokio::time::sleep(Duration::from_secs(5)).await;
    let direto = caminho_ate(&a, &b).await;
    assert!(direto.starts_with("direto"), "{direto}");
    let estado = guardado.lock().unwrap().clone();
    assert!(
        estado
            .memorized_paths
            .iter()
            .any(|(par, _)| *par == b.address()),
        "o caminho direto foi memorizado: {:?}",
        estado.memorized_paths
    );
    a.stop(Duration::from_secs(5)).await;
    b.stop(Duration::from_secs(5)).await;

    // Sem o raiz apresentar e sem o push: só a memória leva ao caminho direto.
    m.raiz.introduce(false);
    let reinicia = |estado: NodeState| {
        let nat_a = &nat_a;
        let m = &m;
        async move {
            construtor(m, estado)
                .answer_all_arp(true)
                .push_direct_paths(false)
                .start_on(nat_a.endpoint("192.168.1.10", 9993))
                .await
                .unwrap()
        }
    };
    // O B volta sem o push, para que só a memória do A leve ao caminho direto.
    let b = construtor(&m, estado_de(5))
        .answer_all_arp(true)
        .push_direct_paths(false)
        .start_on(nat_b.endpoint("192.168.2.10", 9993))
        .await
        .unwrap();
    b.wait_connected(Duration::from_secs(300)).await.unwrap();
    eco(&b, 7).await;
    let sem_memoria = NodeState {
        memorized_paths: Vec::new(),
        ..estado.clone()
    };
    let a2 = reinicia(sem_memoria).await;
    a2.wait_connected(Duration::from_secs(300)).await.unwrap();
    confere_eco(&a2, ip_b, 7, 20_000).await;
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(caminho_ate(&a2, &b).await, "pelo raiz");
    a2.stop(Duration::from_secs(5)).await;

    // O B ainda guarda o caminho do A de antes de reiniciar (a porta do NAT mudou): espera expirar.
    tokio::time::sleep(Duration::from_secs(300)).await;
    let a3 = reinicia(estado).await;
    a3.wait_connected(Duration::from_secs(300)).await.unwrap();
    confere_eco(&a3, ip_b, 7, 20_000).await;
    tokio::time::sleep(Duration::from_secs(5)).await;
    let caminho = caminho_ate(&a3, &b).await;
    assert!(caminho.starts_with("direto"), "{caminho}");
}
