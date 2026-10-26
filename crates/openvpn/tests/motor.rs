//! O motor inteiro (`OpenVpn`) contra o servidor de teste em memória: a ponte SOCKS5, o eco
//! pelo túnel, UDP e TCP, perda, renegociação, quedas, reconexão, as perguntas à pessoa e o
//! aviso de saída. O app só enxerga o que o app enxerga: a ponte e o `State`.

mod comum;

use std::io::{Read, Write};
use std::net::Ipv4Addr;
use std::sync::atomic::Ordering;
use std::time::Duration;

use base64::Engine as _;
use comum::mundo::{
    Cenario, Mundo, SERVIDOR, abrir_pela_ponte, ate, bytes_de_teste, conectado, eco_de, esperar,
    sha,
};
use comum::rede::{empurra, perfil};
use tomni_ui_net::Proxy;
use tomni_ui_openvpn::State;
use tomni_ui_openvpn::auth::{parse_dynamic_challenge_password, parse_static_challenge_password};
use tomni_ui_openvpn::test_server::Decision;
use tomni_ui_openvpn::{OpenVpnBuilder, PromptKind};
use zeroize::Zeroizing;

fn senha(texto: &str) -> Zeroizing<String> {
    Zeroizing::new(texto.to_owned())
}

#[test]
fn conecta_e_leva_quatro_mib_pelo_eco_nos_dois_sentidos() {
    let mundo = Mundo::novo(Cenario::novo());
    let vpn = mundo.abrir("");
    let endereco = vpn
        .wait_connected(Duration::from_secs(20))
        .expect("conecta");
    assert_eq!(endereco.to_string(), "10.8.0.2");
    match vpn.state() {
        State::Connected { cipher, remote, .. } => {
            assert_eq!(cipher, "AES-256-GCM");
            assert!(remote.starts_with("vpn.exemplo.test:1194"), "{remote}");
        }
        outro => panic!("{outro:?}"),
    }
    let dados = bytes_de_teste(4 * 1024 * 1024);
    let volta = eco_de(&vpn.proxy(), SERVIDOR, 7000, &dados).expect("eco");
    assert_eq!(sha(&volta), sha(&dados));
}

#[test]
fn udp_com_perda_de_datagramas_ainda_entrega_tudo() {
    let mundo = Mundo::novo(Cenario::novo());
    mundo.estat.perda_cada.store(9, Ordering::SeqCst);
    let vpn = mundo.abrir("");
    vpn.wait_connected(Duration::from_secs(30))
        .expect("conecta");
    let dados = bytes_de_teste(512 * 1024);
    let volta = eco_de(&vpn.proxy(), SERVIDOR, 7000, &dados).expect("eco");
    assert_eq!(sha(&volta), sha(&dados));
}

#[test]
fn tcp_leva_o_eco_com_as_mensagens_enquadradas() {
    let mundo = Mundo::novo(Cenario::novo().tcp());
    let vpn = mundo.abrir("proto tcp");
    vpn.wait_connected(Duration::from_secs(20))
        .expect("conecta");
    match vpn.state() {
        State::Connected { remote, .. } => assert!(remote.ends_with("/tcp"), "{remote}"),
        outro => panic!("{outro:?}"),
    }
    let dados = bytes_de_teste(1024 * 1024);
    let volta = eco_de(&vpn.proxy(), SERVIDOR, 7000, &dados).expect("eco");
    assert_eq!(sha(&volta), sha(&dados));
}

#[test]
fn renegocia_pelo_reneg_sec_sem_derrubar_o_fluxo() {
    let mundo = Mundo::novo(Cenario::novo());
    let vpn = mundo.abrir("reneg-sec 2");
    vpn.wait_connected(Duration::from_secs(20))
        .expect("conecta");
    let mut fluxo = abrir_pela_ponte(&vpn.proxy(), SERVIDOR, 7000).expect("fluxo");
    fluxo
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    // Conversa por uns 6 s: passa por pelo menos duas renegociações.
    for rodada in 0..30u8 {
        let msg = [rodada; 200];
        fluxo.write_all(&msg).expect("escreve");
        let mut volta = [0u8; 200];
        fluxo.read_exact(&mut volta).expect("lê");
        assert_eq!(volta, msg);
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(
        mundo.estat.n(&mundo.estat.chaves) >= 3,
        "poucas trocas de chave"
    );
    assert_eq!(mundo.conexoes(), 1, "não reconectou");
    assert!(conectado(&vpn.state()));
}

#[test]
fn ping_restart_reconecta_e_mantem_a_pilha_quando_o_ip_e_o_mesmo() {
    let mundo = Mundo::novo(Cenario::novo().push(|_| empurra("10.8.0.2", "ping 1,ping-restart 3")));
    let vpn = mundo.abrir("connect-retry 1");
    vpn.wait_connected(Duration::from_secs(20))
        .expect("conecta");
    let mut fluxo = abrir_pela_ponte(&vpn.proxy(), SERVIDOR, 7000).expect("fluxo");
    fluxo
        .set_read_timeout(Some(Duration::from_secs(40)))
        .unwrap();
    fluxo.write_all(b"antes").unwrap();
    let mut b = [0u8; 5];
    fluxo.read_exact(&mut b).unwrap();
    assert_eq!(&b, b"antes");

    mundo.estat.mudo.store(true, Ordering::SeqCst);
    esperar(&vpn, 15, |s| matches!(s, State::Retrying(_)));
    mundo.estat.mudo.store(false, Ordering::SeqCst);
    esperar(&vpn, 20, conectado);
    assert_eq!(mundo.conexoes(), 2);

    // O mesmo fluxo segue: a pilha não foi trocada.
    fluxo.write_all(b"depois").unwrap();
    let mut d = [0u8; 6];
    fluxo.read_exact(&mut d).unwrap();
    assert_eq!(&d, b"depois");
}

#[test]
fn restart_com_outro_ip_troca_a_pilha_e_derruba_os_fluxos_velhos() {
    let mundo = Mundo::novo(
        Cenario::novo().push(|n| empurra(if n == 1 { "10.8.0.2" } else { "10.8.0.3" }, "")),
    );
    let vpn = mundo.abrir("connect-retry 1");
    assert_eq!(
        vpn.wait_connected(Duration::from_secs(20))
            .unwrap()
            .to_string(),
        "10.8.0.2"
    );
    let mut velho = abrir_pela_ponte(&vpn.proxy(), SERVIDOR, 7000).expect("fluxo");
    velho
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    velho.write_all(b"oi").unwrap();
    let mut b = [0u8; 2];
    velho.read_exact(&mut b).unwrap();

    mundo.estat.mensagem("RESTART,reiniciando");
    let estado = esperar(
        &vpn,
        20,
        |s| matches!(s, State::Connected { address, .. } if address.to_string() == "10.8.0.3"),
    );
    assert!(conectado(&estado));
    assert_eq!(mundo.conexoes(), 2);
    // O fluxo da pilha velha acabou.
    let mut resto = Vec::new();
    let fim = velho.read_to_end(&mut resto);
    assert!(fim.is_err() || resto.is_empty(), "{fim:?} {resto:?}");
    // Um fluxo novo funciona.
    let volta = eco_de(&vpn.proxy(), SERVIDOR, 7000, b"novo").expect("eco");
    assert_eq!(volta, b"novo");
}

#[test]
fn senha_recusada_pergunta_de_novo_e_a_resposta_vale() {
    let mundo = Mundo::novo(Cenario::novo().regra(|a| {
        if a.username.as_deref() == Some("ana") && a.password.as_deref() == Some("certa") {
            Decision::Accept { token: None }
        } else {
            Decision::Fail("AUTH_FAILED".into())
        }
    }));
    let vpn = mundo.abrir_com(perfil("auth-user-pass"), |b| {
        b.credentials("ana".into(), senha("errada"))
    });
    let State::Asking(pergunta) = esperar(&vpn, 20, |s| matches!(s, State::Asking(_))) else {
        unreachable!()
    };
    assert_eq!(pergunta.kind, PromptKind::Password);
    assert!(!pergunta.echo);
    vpn.answer(pergunta.id, Some(senha("certa")));
    vpn.wait_connected(Duration::from_secs(20))
        .expect("conecta");
    let tentativas = mundo.estat.tentativas();
    assert_eq!(tentativas.len(), 2);
    assert_eq!(tentativas[1].1.as_deref(), Some("certa"));
}

#[test]
fn cancelar_a_pergunta_da_senha_termina_em_falha() {
    let mundo = Mundo::novo(Cenario::novo().regra(|_| Decision::Fail("AUTH_FAILED".into())));
    let vpn = mundo.abrir_com(perfil("auth-user-pass"), |b| {
        b.credentials("ana".into(), senha("x"))
    });
    let State::Asking(pergunta) = esperar(&vpn, 20, |s| matches!(s, State::Asking(_))) else {
        unreachable!()
    };
    vpn.answer(pergunta.id, None);
    let estado = esperar(&vpn, 10, |s| matches!(s, State::Failed(_)));
    assert!(
        matches!(estado, State::Failed(m) if m.contains("recusados")),
        "{:?}",
        vpn.state()
    );
    assert!(vpn.wait_connected(Duration::from_secs(1)).is_err());
}

#[test]
fn desafio_dinamico_crv1_vira_uma_pergunta_e_a_resposta_reconecta() {
    let usuario = base64::engine::general_purpose::STANDARD.encode("ana");
    let pergunta = format!("AUTH_FAILED,CRV1:R,E:estado-123:{usuario}:Digite o código");
    let mundo = Mundo::novo(Cenario::novo().regra(move |a| match a.password.as_deref() {
        Some(p) => match parse_dynamic_challenge_password(p) {
            Some((estado, resposta)) if estado == "estado-123" && *resposta == "987654" => {
                Decision::Accept { token: None }
            }
            Some(_) => Decision::Fail("AUTH_FAILED".into()),
            None => Decision::Fail(pergunta.clone()),
        },
        None => Decision::Fail("AUTH_FAILED".into()),
    }));
    let vpn = mundo.abrir_com(perfil("auth-user-pass"), |b| {
        b.credentials("ana".into(), senha("segredo"))
    });
    let State::Asking(p) = esperar(&vpn, 20, |s| matches!(s, State::Asking(_))) else {
        unreachable!()
    };
    assert_eq!(p.kind, PromptKind::Challenge);
    assert_eq!(p.text, "Digite o código");
    assert!(p.echo);
    vpn.answer(p.id, Some(senha("987654")));
    vpn.wait_connected(Duration::from_secs(20))
        .expect("conecta");
}

#[test]
fn desafio_estatico_scrv1_pergunta_antes_de_conectar() {
    let mundo = Mundo::novo(Cenario::novo().regra(|a| {
        let ok = a
            .password
            .as_deref()
            .and_then(parse_static_challenge_password)
            .is_some_and(|(senha, resposta)| *senha == "segredo" && *resposta == "123456");
        if ok && a.username.as_deref() == Some("ana") {
            Decision::Accept { token: None }
        } else {
            Decision::Fail("AUTH_FAILED".into())
        }
    }));
    let vpn = mundo.abrir_com(
        perfil("auth-user-pass\nstatic-challenge \"Código do aplicativo\" 1"),
        |b| b.credentials("ana".into(), senha("segredo")),
    );
    let State::Asking(p) = esperar(&vpn, 20, |s| matches!(s, State::Asking(_))) else {
        unreachable!()
    };
    assert_eq!(p.kind, PromptKind::StaticChallenge);
    assert_eq!(p.text, "Código do aplicativo");
    assert!(p.echo);
    assert_eq!(mundo.conexoes(), 0, "pergunta antes de abrir o enlace");
    vpn.answer(p.id, Some(senha("123456")));
    vpn.wait_connected(Duration::from_secs(20))
        .expect("conecta");
}

#[test]
fn auth_pending_e_open_url_ficam_em_espera_ate_o_servidor_decidir() {
    let mundo = Mundo::novo(Cenario::novo().regra(|_| Decision::Pending { timeout: 60 }));
    let vpn = mundo.abrir_com(perfil("auth-user-pass"), |b| {
        b.credentials("ana".into(), senha("x"))
    });
    esperar(&vpn, 20, |s| matches!(s, State::Waiting { url: None, .. }));
    mundo
        .estat
        .mensagem("INFO_PRE,OPEN_URL:https://login.exemplo.test/x");
    let estado = esperar(&vpn, 10, |s| {
        matches!(s, State::Waiting { url: Some(_), .. })
    });
    assert!(
        matches!(estado, State::Waiting { url: Some(u), .. } if u == "https://login.exemplo.test/x")
    );
    mundo.estat.resolve(Decision::Accept { token: None });
    esperar(&vpn, 20, conectado);
}

#[test]
fn dns_da_vpn_resolve_nomes_pela_ponte_com_o_dominio_de_busca() {
    let mundo = Mundo::novo(
        Cenario::novo()
            .push(|_| {
                empurra(
                    "10.8.0.2",
                    "dhcp-option DNS 10.9.0.1,dhcp-option DOMAIN corp.test",
                )
            })
            .dns("alvo.corp.test", [10, 9, 0, 1]),
    );
    let vpn = mundo.abrir("");
    vpn.wait_connected(Duration::from_secs(20))
        .expect("conecta");
    let proxy = vpn.proxy();
    // Nome completo e nome sem ponto (o domínio de busca completa).
    for nome in ["alvo.corp.test", "alvo"] {
        let mut s = abrir_pela_ponte(&proxy, nome, 7000).unwrap_or_else(|e| panic!("{nome}: {e}"));
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        s.write_all(b"ola").unwrap();
        let mut b = [0u8; 3];
        s.read_exact(&mut b).unwrap();
        assert_eq!(&b, b"ola");
    }
    // Dentro do domínio, mas desconhecido: falha, sem perguntar ao DNS do sistema.
    assert!(abrir_pela_ponte(&proxy, "nada.corp.test", 7000).is_err());
}

#[test]
fn pacotes_nao_pedidos_pelo_app_nao_recebem_resposta() {
    let mundo = Mundo::novo(Cenario::novo());
    let vpn = mundo.abrir("");
    vpn.wait_connected(Duration::from_secs(20))
        .expect("conecta");
    // Deixa o ping inicial passar e anota quantos pacotes IP o servidor já viu.
    std::thread::sleep(Duration::from_millis(200));
    let antes = mundo.estat.ip_do_cliente.load(Ordering::SeqCst);
    let origem = Ipv4Addr::new(10, 9, 0, 1);
    let destino = Ipv4Addr::new(10, 8, 0, 2);
    mundo.estat.ip(pacote_syn(origem, destino, 22));
    mundo.estat.ip(pacote_ping(origem, destino));
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(
        mundo.estat.ip_do_cliente.load(Ordering::SeqCst),
        antes,
        "o SYN e o ping não podem ter resposta (nem RST)"
    );
}

#[test]
fn stop_avisa_o_servidor_e_termina() {
    let mundo = Mundo::novo(Cenario::novo());
    let vpn = mundo.abrir("");
    vpn.wait_connected(Duration::from_secs(20))
        .expect("conecta");
    vpn.stop(Duration::from_secs(5));
    ate(5, "o aviso de saída chegar", || {
        mundo.estat.saidas.load(Ordering::SeqCst) == 1
    });
}

#[test]
fn enlace_que_falha_tenta_de_novo_e_passa_ao_proximo_remote() {
    let mundo = Mundo::novo(Cenario::novo());
    mundo.falhas.store(1, Ordering::SeqCst);
    let vpn = mundo.abrir("connect-retry 1\nremote segundo.exemplo.test 1195");
    let estado = esperar(&vpn, 20, conectado);
    let State::Connected { remote, .. } = estado else {
        unreachable!()
    };
    assert!(remote.starts_with("segundo.exemplo.test:1195"), "{remote}");
    let alvos = mundo.alvos.lock().unwrap().clone();
    assert_eq!(alvos[0], "vpn.exemplo.test:1194");
    assert_eq!(alvos[1], "segundo.exemplo.test:1195");
}

#[test]
fn halt_do_servidor_para_de_vez() {
    let mundo = Mundo::novo(Cenario::novo());
    let vpn = mundo.abrir("");
    vpn.wait_connected(Duration::from_secs(20))
        .expect("conecta");
    mundo.estat.mensagem("HALT,manutenção");
    let estado = esperar(&vpn, 10, |s| matches!(s, State::Failed(_)));
    assert!(matches!(estado, State::Failed(m) if m.contains("manutenção")));
    // Sem pilha: a ponte recusa de imediato.
    assert!(abrir_pela_ponte(&vpn.proxy(), SERVIDOR, 7000).is_err());
    assert_eq!(mundo.conexoes(), 1, "não tenta de novo");
}

#[test]
fn a_ponte_so_atende_quem_tem_a_credencial() {
    let mundo = Mundo::novo(Cenario::novo());
    let vpn = mundo.abrir("");
    vpn.wait_connected(Duration::from_secs(20))
        .expect("conecta");
    assert!(vpn.socks().addr.ip().is_loopback());
    let Proxy::Socks5 { host, port, .. } = vpn.proxy() else {
        unreachable!()
    };
    let sem = Proxy::Socks5 {
        host: host.clone(),
        port,
        user: None,
        password: None,
    };
    assert!(abrir_pela_ponte(&sem, SERVIDOR, 7000).is_err());
    let errada = Proxy::Socks5 {
        host,
        port,
        user: Some("outro".into()),
        password: Some("outra".into()),
    };
    assert!(abrir_pela_ponte(&errada, SERVIDOR, 7000).is_err());
}

#[test]
fn perfis_que_nao_servem_nao_abrem() {
    let mundo = Mundo::novo(Cenario::novo());
    // Pede usuário e senha e ninguém deu.
    let erro = OpenVpnBuilder::new(perfil("auth-user-pass"))
        .connector(mundo.conector())
        .start()
        .expect_err("sem credenciais");
    assert!(erro.0.contains("auth-user-pass"), "{}", erro.0);
}

// ---- pacotes crus para o teste do filtro ----

fn soma(dados: &[u8], inicio: u32) -> u16 {
    let mut total = inicio;
    for par in dados.chunks(2) {
        total += u32::from(u16::from_be_bytes([par[0], *par.get(1).unwrap_or(&0)]));
    }
    while total >> 16 != 0 {
        total = (total & 0xFFFF) + (total >> 16);
    }
    !(total as u16)
}

fn cabecalho_ip(protocolo: u8, tamanho: usize, origem: Ipv4Addr, destino: Ipv4Addr) -> Vec<u8> {
    let mut h = vec![0x45, 0, 0, 0, 0, 1, 0, 0, 64, protocolo, 0, 0];
    h[2..4].copy_from_slice(&((20 + tamanho) as u16).to_be_bytes());
    h.extend_from_slice(&origem.octets());
    h.extend_from_slice(&destino.octets());
    let c = soma(&h, 0);
    h[10..12].copy_from_slice(&c.to_be_bytes());
    h
}

fn pacote_syn(origem: Ipv4Addr, destino: Ipv4Addr, porta: u16) -> Vec<u8> {
    let mut tcp = vec![0u8; 20];
    tcp[0..2].copy_from_slice(&40_000u16.to_be_bytes());
    tcp[2..4].copy_from_slice(&porta.to_be_bytes());
    tcp[4..8].copy_from_slice(&1000u32.to_be_bytes());
    tcp[12] = 0x50;
    tcp[13] = 0x02;
    tcp[14..16].copy_from_slice(&8192u16.to_be_bytes());
    let mut pseudo = Vec::new();
    pseudo.extend_from_slice(&origem.octets());
    pseudo.extend_from_slice(&destino.octets());
    pseudo.extend_from_slice(&[0, 6, 0, 20]);
    pseudo.extend_from_slice(&tcp);
    let c = soma(&pseudo, 0);
    tcp[16..18].copy_from_slice(&c.to_be_bytes());
    let mut p = cabecalho_ip(6, tcp.len(), origem, destino);
    p.extend_from_slice(&tcp);
    p
}

fn pacote_ping(origem: Ipv4Addr, destino: Ipv4Addr) -> Vec<u8> {
    let mut icmp = vec![8, 0, 0, 0, 0, 7, 0, 1, b'o', b'i', b'!', b'!'];
    let c = soma(&icmp, 0);
    icmp[2..4].copy_from_slice(&c.to_be_bytes());
    let mut p = cabecalho_ip(1, icmp.len(), origem, destino);
    p.extend_from_slice(&icmp);
    p
}
