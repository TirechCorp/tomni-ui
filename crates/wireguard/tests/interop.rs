//! Interoperabilidade: o nosso `Device` conversa com uma implementação independente do
//! WireGuard (o `Tunn` do `boringtun`, só como dependência de teste), nos dois sentidos de
//! iniciação, com e sem chave pré-compartilhada.
//!
//! O `boringtun` lê o relógio do sistema, então os testes não esperam os prazos de minutos: eles
//! conferem o handshake, os dados nos dois sentidos, o keepalive e uma segunda troca de chaves.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use boringtun::noise::{Tunn, TunnResult};
use boringtun::x25519::{PublicKey as BtPublic, StaticSecret};
use tomni_ui_wireguard::{
    Cidr, Device, Event, PeerConfig, PresharedKey, PrivateKey, REKEY_AFTER_TIME, TAMANHO_KEEPALIVE,
};

const IP_NOSSO: Ipv4Addr = Ipv4Addr::new(10, 7, 0, 1);
const IP_DELE: Ipv4Addr = Ipv4Addr::new(10, 7, 0, 2);

struct Par {
    nosso: Device,
    nosso_addr: SocketAddr,
    dele: Tunn,
    dele_addr: SocketAddr,
    t0: Instant,
}

/// O que o `boringtun` produziu ao processar algo.
#[derive(Default)]
struct Saida {
    rede: Vec<Vec<u8>>,
    ip: Vec<Vec<u8>>,
}

fn dele_processa(tunn: &mut Tunn, datagrama: &[u8], de: SocketAddr) -> Saida {
    let mut saida = Saida::default();
    let mut buf = vec![0u8; 4096];
    let mut dados = datagrama;
    loop {
        match tunn.decapsulate(Some(de.ip()), dados, &mut buf) {
            TunnResult::Done => break,
            TunnResult::Err(e) => panic!("o boringtun recusou o datagrama: {e:?}"),
            TunnResult::WriteToNetwork(d) => {
                saida.rede.push(d.to_vec());
                // Chamar de novo, vazio, até acabar o que ele tem a mandar.
                dados = &[];
            }
            TunnResult::WriteToTunnelV4(d, _) => {
                saida.ip.push(d.to_vec());
                break;
            }
            TunnResult::WriteToTunnelV6(d, _) => {
                saida.ip.push(d.to_vec());
                break;
            }
        }
    }
    saida
}

fn dele_manda(tunn: &mut Tunn, pacote: &[u8]) -> Vec<Vec<u8>> {
    let mut buf = vec![0u8; pacote.len() + 256];
    match tunn.encapsulate(pacote, &mut buf) {
        TunnResult::WriteToNetwork(d) => vec![d.to_vec()],
        TunnResult::Done => vec![],
        TunnResult::Err(e) => panic!("o boringtun não conseguiu mandar: {e:?}"),
        _ => unreachable!(),
    }
}

/// Entrega um datagrama ao nosso lado e junta tudo o que ele produz.
fn nosso_processa(par: &mut Par, datagrama: &[u8], now: Instant) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
    let (mut rede, mut ip) = (Vec::new(), Vec::new());
    let origem = par.dele_addr;
    let mut ev = par.nosso.receive(datagrama, origem, now);
    loop {
        match ev {
            Event::Nothing => break,
            Event::Transmit { data, .. } => rede.push(data.to_vec()),
            Event::Ip { data, .. } => ip.push(data.to_vec()),
            Event::PeerError { error, .. } => panic!("erro do par: {error}"),
        }
        ev = par.nosso.poll(now);
    }
    (rede, ip)
}

fn nosso_drena(par: &mut Par, now: Instant) -> Vec<Vec<u8>> {
    let mut rede = Vec::new();
    loop {
        match par.nosso.poll(now) {
            Event::Nothing => break,
            Event::Transmit { data, .. } => rede.push(data.to_vec()),
            _ => {}
        }
    }
    rede
}

fn montar(psk: Option<[u8; 32]>) -> Par {
    let t0 = Instant::now();
    let nossa = PrivateKey::generate().unwrap();
    let dele_priv = StaticSecret::from(chave_aleatoria());
    let dele_pub = BtPublic::from(&dele_priv);
    let nosso_pub = nossa.public_key();
    let nosso_addr: SocketAddr = "192.0.2.10:51820".parse().unwrap();
    let dele_addr: SocketAddr = "192.0.2.20:51820".parse().unwrap();

    let mut nosso = Device::new(nossa, t0).unwrap();
    let mut config = PeerConfig::new(tomni_ui_wireguard::PublicKey::from_bytes(
        *dele_pub.as_bytes(),
    ));
    if let Some(p) = psk {
        config = config.with_preshared_key(PresharedKey::from_bytes(p));
    }
    nosso
        .add_peer(
            config,
            Some(dele_addr),
            &[Cidr::new(IpAddr::V4(IP_DELE), 32).unwrap()],
            t0,
        )
        .unwrap();
    let nosso_pub_bt = BtPublic::from(*nosso_pub.as_bytes());
    let dele = Tunn::new(dele_priv, nosso_pub_bt, psk, None, 7, None);
    Par {
        nosso,
        nosso_addr,
        dele,
        dele_addr,
        t0,
    }
}

/// 32 bytes aleatórios do sistema, como fonte para a chave do `boringtun`.
fn chave_aleatoria() -> [u8; 32] {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).unwrap();
    b
}

fn pacote(origem: Ipv4Addr, destino: Ipv4Addr, carga: &[u8]) -> Vec<u8> {
    let mut p = vec![0u8; 20];
    p[0] = 0x45;
    p[2..4].copy_from_slice(&((20 + carga.len()) as u16).to_be_bytes());
    p[8] = 64;
    p[9] = 17;
    p[12..16].copy_from_slice(&origem.octets());
    p[16..20].copy_from_slice(&destino.octets());
    p.extend_from_slice(carga);
    p
}

/// Leva os datagramas de um lado a outro até os dois calarem; devolve os pacotes IP que cada
/// lado entregou à pilha: `(nosso, dele)`.
fn conversar(
    par: &mut Par,
    mut para_ele: Vec<Vec<u8>>,
    mut para_nos: Vec<Vec<u8>>,
    now: Instant,
) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
    let (mut ip_nosso, mut ip_dele) = (Vec::new(), Vec::new());
    for _ in 0..20 {
        if para_ele.is_empty() && para_nos.is_empty() {
            break;
        }
        let mut proximo_para_nos = Vec::new();
        for d in std::mem::take(&mut para_ele) {
            let s = dele_processa(&mut par.dele, &d, par.nosso_addr);
            proximo_para_nos.extend(s.rede);
            ip_dele.extend(s.ip);
        }
        let mut proximo_para_ele = Vec::new();
        for d in std::mem::take(&mut para_nos)
            .into_iter()
            .chain(proximo_para_nos)
        {
            let (rede, ip) = nosso_processa(par, &d, now);
            proximo_para_ele.extend(rede);
            ip_nosso.extend(ip);
        }
        para_ele = proximo_para_ele;
    }
    (ip_nosso, ip_dele)
}

fn ida_e_volta(psk: Option<[u8; 32]>, nos_iniciamos: bool) {
    let mut par = montar(psk);
    let agora = par.t0 + Duration::from_secs(1);
    let ida = pacote(IP_NOSSO, IP_DELE, b"de nos para ele");
    let volta = pacote(IP_DELE, IP_NOSSO, b"dele para nos");
    let (mut ip_nosso, mut ip_dele);
    if nos_iniciamos {
        par.nosso.send(&ida).unwrap();
        let saida = nosso_drena(&mut par, agora);
        assert!(!saida.is_empty(), "devia haver uma iniciação");
        (ip_nosso, ip_dele) = conversar(&mut par, saida, vec![], agora);
        assert_eq!(ip_dele, vec![ida.clone()]);
        assert!(ip_nosso.is_empty());
        // Ele responde pelo túnel já aberto.
        let d = dele_manda(&mut par.dele, &volta);
        (ip_nosso, _) = conversar(&mut par, vec![], d, agora);
        assert_eq!(ip_nosso, vec![volta]);
    } else {
        let d = dele_manda(&mut par.dele, &volta);
        assert_eq!(d.len(), 1, "o boringtun devia iniciar o handshake");
        (ip_nosso, ip_dele) = conversar(&mut par, vec![], d, agora);
        assert_eq!(ip_nosso, vec![volta]);
        assert!(ip_dele.is_empty());
        // Agora nós respondemos pelo túnel já aberto.
        par.nosso.send(&ida).unwrap();
        let saida = nosso_drena(&mut par, agora);
        (_, ip_dele) = conversar(&mut par, saida, vec![], agora);
        assert_eq!(ip_dele, vec![ida]);
    }
}

#[test]
fn nos_iniciamos_sem_chave_pre_compartilhada() {
    ida_e_volta(None, true);
}

#[test]
fn nos_iniciamos_com_chave_pre_compartilhada() {
    ida_e_volta(Some([0x5a; 32]), true);
}

#[test]
fn ele_inicia_sem_chave_pre_compartilhada() {
    ida_e_volta(None, false);
}

#[test]
fn ele_inicia_com_chave_pre_compartilhada() {
    ida_e_volta(Some([0xa5; 32]), false);
}

#[test]
fn varios_pacotes_seguidos_e_de_tamanhos_diferentes() {
    let mut par = montar(None);
    let agora = par.t0 + Duration::from_secs(1);
    let tamanhos = [0usize, 1, 15, 16, 17, 100, 1000, 1380];
    let mut esperados = Vec::new();
    for (i, n) in tamanhos.iter().enumerate() {
        let p = pacote(IP_NOSSO, IP_DELE, &vec![i as u8; *n]);
        par.nosso.send(&p).unwrap();
        esperados.push(p);
    }
    let saida = nosso_drena(&mut par, agora);
    let (_, ip_dele) = conversar(&mut par, saida, vec![], agora);
    assert_eq!(ip_dele, esperados);
    // E ao contrário, já com a sessão aberta.
    let mut volta_esperada = Vec::new();
    let mut para_nos = Vec::new();
    for (i, n) in tamanhos.iter().enumerate() {
        let p = pacote(IP_DELE, IP_NOSSO, &vec![i as u8; *n]);
        para_nos.extend(dele_manda(&mut par.dele, &p));
        volta_esperada.push(p);
    }
    let (ip_nosso, _) = conversar(&mut par, vec![], para_nos, agora);
    assert_eq!(ip_nosso, volta_esperada);
}

#[test]
fn o_keepalive_do_boringtun_nao_vira_pacote() {
    let mut par = montar(None);
    let agora = par.t0 + Duration::from_secs(1);
    par.nosso.send(&pacote(IP_NOSSO, IP_DELE, b"abre")).unwrap();
    let saida = nosso_drena(&mut par, agora);
    conversar(&mut par, saida, vec![], agora);
    // Um pacote vazio do `boringtun` é o keepalive (32 bytes no fio).
    let ka = dele_manda(&mut par.dele, &[]);
    assert_eq!(ka.len(), 1);
    assert_eq!(ka[0].len(), TAMANHO_KEEPALIVE);
    let (rede, ip) = nosso_processa(&mut par, &ka[0], agora);
    assert!(ip.is_empty());
    assert!(rede.is_empty());
    // Nosso keepalive também chega a ele sem virar pacote.
    par.nosso
        .peer_mut(&par_dele_pub(&par))
        .unwrap()
        .send_keepalive();
    let saida = nosso_drena(&mut par, agora);
    assert_eq!(saida.len(), 1);
    assert_eq!(saida[0].len(), TAMANHO_KEEPALIVE);
    let (ip_nosso, ip_dele) = conversar(&mut par, saida, vec![], agora);
    assert!(ip_nosso.is_empty() && ip_dele.is_empty());
}

fn par_dele_pub(par: &Par) -> tomni_ui_wireguard::PublicKey {
    *par.nosso.peers().next().unwrap()
}

#[test]
fn o_boringtun_troca_as_chaves_de_novo_e_o_tunel_segue() {
    let mut par = montar(Some([7; 32]));
    let agora = par.t0 + Duration::from_secs(1);
    par.nosso.send(&pacote(IP_NOSSO, IP_DELE, b"1")).unwrap();
    let saida = nosso_drena(&mut par, agora);
    let (_, ip_dele) = conversar(&mut par, saida, vec![], agora);
    assert_eq!(ip_dele.len(), 1);

    // Ele força uma iniciação nova (um segundo handshake) enquanto a sessão está de pé.
    let mut buf = vec![0u8; 512];
    let ini = match par.dele.format_handshake_initiation(&mut buf, true) {
        TunnResult::WriteToNetwork(d) => d.to_vec(),
        outro => panic!("esperava uma iniciação, veio {outro:?}"),
    };
    let depois = par.t0 + Duration::from_secs(5);
    let (_, _) = conversar(&mut par, vec![], vec![ini], depois);
    let volta = pacote(IP_DELE, IP_NOSSO, b"depois da troca");
    let d = dele_manda(&mut par.dele, &volta);
    let (ip_nosso, _) = conversar(&mut par, vec![], d, depois);
    assert_eq!(ip_nosso, vec![volta]);
    let ida = pacote(IP_NOSSO, IP_DELE, b"2");
    par.nosso.send(&ida).unwrap();
    let saida = nosso_drena(&mut par, depois);
    let (_, ip_dele) = conversar(&mut par, saida, vec![], depois);
    assert_eq!(ip_dele, vec![ida]);
}

#[test]
fn passados_120_s_nos_trocamos_a_chave_com_o_boringtun() {
    let mut par = montar(None);
    let agora = par.t0 + Duration::from_secs(1);
    par.nosso.send(&pacote(IP_NOSSO, IP_DELE, b"1")).unwrap();
    let saida = nosso_drena(&mut par, agora);
    let (_, ip_dele) = conversar(&mut par, saida, vec![], agora);
    assert_eq!(ip_dele.len(), 1);
    // O nosso relógio anda além de `REKEY_AFTER_TIME`: o próximo envio inicia outro handshake.
    let depois = par.t0 + REKEY_AFTER_TIME + Duration::from_secs(2);
    let ida = pacote(IP_NOSSO, IP_DELE, b"2");
    par.nosso.send(&ida).unwrap();
    let saida = nosso_drena(&mut par, depois);
    let (_, ip_dele) = conversar(&mut par, saida, vec![], depois);
    assert_eq!(ip_dele, vec![ida]);
    let volta = pacote(IP_DELE, IP_NOSSO, b"3");
    let d = dele_manda(&mut par.dele, &volta);
    let (ip_nosso, _) = conversar(&mut par, vec![], d, depois);
    assert_eq!(ip_nosso, vec![volta]);
}
