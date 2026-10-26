//! O canal de controle contra um servidor OpenVPN de verdade (opcional, fora do CI).
//!
//! Os testes são `#[ignore]`: precisam dos servidores de `testes/openvpn/servidores.sh` no ar
//! (Docker, em `127.0.0.1`, portas 19194 a 19199). Rode com:
//!
//! ```sh
//! testes/openvpn/rodar.sh
//! ```
//!
//! Eles conferem o que os testes em memória não alcançam: que o `OpenVPN` aceita o texto de
//! opções e o `peer info` do cliente, que a direção das chaves do `tls-auth` e do `tls-crypt`
//! bate, e que o `PUSH_REPLY` real é lido. Os testes do fim da arquivo usam o motor inteiro
//! (`OpenVpn`): confirmam as chaves do canal de dados (os bytes do eco só voltam se as duas
//! pontas derivaram as mesmas chaves), o `ping`, o aviso de saída, o reinício e o DNS da rede.

mod comum;

use std::io::{Read, Write};
use std::net::{TcpStream, UdpSocket};
use std::time::{Duration, Instant};

use comum::{ler, pasta};
use tomni_ui_openvpn::auth::AuthState;
use tomni_ui_openvpn::channel::Clock;
use tomni_ui_openvpn::config::{Config, Proto, TlsVersion};
use tomni_ui_openvpn::control::{Control, ControlOptions, Event};
use tomni_ui_openvpn::packet::{TcpDeframer, tcp_frame};
use tomni_ui_openvpn::pki::Wrap;
use tomni_ui_openvpn::static_key::StaticKey;

/// O endereço dos servidores de teste (o `TOMNI_UI_OPENVPN_HOST` troca o padrão).
fn host() -> String {
    std::env::var("TOMNI_UI_OPENVPN_HOST").unwrap_or_else(|_| "127.0.0.1".to_owned())
}

fn opcoes(wrap: Wrap, arquivo: Option<&str>, tcp: bool) -> ControlOptions {
    let (config, _) =
        Config::from_ovpn(&ler("tudo-embutido.ovpn"), Some(&pasta())).expect("perfil");
    let transporte = if tcp { Proto::Tcp } else { Proto::Udp };
    let mut o = ControlOptions::from_config(
        &config,
        transporte,
        false,
        AuthState::none(),
        Clock::system(),
    )
    .expect("opções");
    o.pki.wrap = wrap;
    o.pki.static_key = arquivo.map(|a| StaticKey::from_pem(&ler(a)).expect("chave estática"));
    // O servidor usa a direção 0; o cliente, a 1.
    o.pki.key_direction = arquivo.map(|_| 1);
    o
}

/// Liga o cliente ao servidor de verdade e roda até as chaves da geração `ate` saírem.
fn conversar(options: ControlOptions, porta: u16, tcp: bool, renegociar: bool) -> Vec<Event> {
    let endereco = format!("{}:{porta}", host());
    let t0 = Instant::now();
    let mut cliente = Control::new(options, t0).expect("cliente");
    let mut eventos = Vec::new();
    let mut udp = None;
    let mut fluxo = None;
    if tcp {
        let s = TcpStream::connect(&endereco).expect("conexão TCP");
        s.set_read_timeout(Some(Duration::from_millis(30))).unwrap();
        fluxo = Some(s);
    } else {
        let s = UdpSocket::bind("0.0.0.0:0").expect("socket UDP");
        s.connect(&endereco).expect("endereço");
        s.set_read_timeout(Some(Duration::from_millis(30))).unwrap();
        udp = Some(s);
    }
    let mut deframer = TcpDeframer::new();
    let mut reneg_pedida = false;
    let fim = t0 + Duration::from_secs(30);
    let mut buf = vec![0u8; 65_536];
    while Instant::now() < fim {
        let agora = Instant::now();
        cliente.handle_timeout(agora);
        while let Some(p) = cliente.poll_transmit(agora) {
            if let Some(s) = &udp {
                s.send(&p).expect("envio UDP");
            } else if let Some(s) = fluxo.as_mut() {
                s.write_all(&tcp_frame(&p).expect("quadro"))
                    .expect("envio TCP");
            }
        }
        let mut chegou: Vec<Vec<u8>> = Vec::new();
        if let Some(s) = &udp {
            if let Ok(n) = s.recv(&mut buf) {
                chegou.push(buf[..n].to_vec());
            }
        } else if let Some(s) = fluxo.as_mut() {
            match s.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    deframer.push(&buf[..n]);
                    while let Some(p) = deframer.next_packet() {
                        chegou.push(p);
                    }
                }
                Err(_) => {}
            }
        }
        for d in chegou {
            cliente.handle_datagram(&d, Instant::now());
        }
        while let Some(e) = cliente.poll_event() {
            eventos.push(e);
        }
        let chaves = eventos
            .iter()
            .filter(|e| matches!(e, Event::DataKeys { .. }))
            .count();
        if cliente.is_finished() {
            break;
        }
        if renegociar {
            if chaves == 1 && !reneg_pedida {
                reneg_pedida = true;
                cliente.renegotiate(Instant::now());
            }
            if chaves >= 2 {
                break;
            }
        } else if chaves >= 1 && eventos.iter().any(|e| matches!(e, Event::Pushed(_))) {
            break;
        }
    }
    eventos
}

fn confere(eventos: &[Event], geracoes: usize) {
    let falhas: Vec<_> = eventos
        .iter()
        .filter(|e| {
            matches!(
                e,
                Event::Failed(_) | Event::AuthFailed(_) | Event::Restart(_) | Event::Halt(_)
            )
        })
        .collect();
    assert!(falhas.is_empty(), "{falhas:?} / {eventos:?}");
    let pushed = eventos
        .iter()
        .find_map(|e| match e {
            Event::Pushed(p) => Some(p),
            _ => None,
        })
        .unwrap_or_else(|| panic!("sem PUSH_REPLY: {eventos:?}"));
    let ifc = pushed.ifconfig.as_ref().expect("ifconfig empurrado");
    assert!(ifc.address.to_string().starts_with("10.8."), "{ifc:?}");
    assert!(
        pushed
            .routes
            .iter()
            .any(|r| r.network.to_string() == "10.9.0.0" && r.prefix_len == 24),
        "{:?}",
        pushed.routes
    );
    let chaves = eventos
        .iter()
        .filter(|e| matches!(e, Event::DataKeys { .. }))
        .count();
    assert_eq!(chaves, geracoes, "{eventos:?}");
}

#[test]
#[ignore = "precisa dos servidores de testes/openvpn/servidores.sh"]
fn tls_auth_contra_o_openvpn() {
    let o = opcoes(Wrap::TlsAuth, Some("ta.key"), false);
    confere(&conversar(o, 19194, false, false), 1);
}

#[test]
#[ignore = "precisa dos servidores de testes/openvpn/servidores.sh"]
fn tls_crypt_contra_o_openvpn() {
    let o = opcoes(Wrap::TlsCrypt, Some("tc.key"), false);
    confere(&conversar(o, 19195, false, false), 1);
}

#[test]
#[ignore = "precisa dos servidores de testes/openvpn/servidores.sh"]
fn tcp_sem_protecao_contra_o_openvpn() {
    let o = opcoes(Wrap::None, None, true);
    confere(&conversar(o, 19196, true, false), 1);
}

#[test]
#[ignore = "precisa dos servidores de testes/openvpn/servidores.sh"]
fn tls_1_3_e_exportador_contra_o_openvpn() {
    let mut o = opcoes(Wrap::TlsCrypt, Some("tc.key"), false);
    o.tls_min = Some(TlsVersion::V1_3);
    let eventos = conversar(o, 19197, false, false);
    confere(&eventos, 1);
    let ekm = eventos.iter().any(|e| match e {
        Event::Pushed(p) => p.key_derivation_ekm,
        _ => false,
    });
    assert!(ekm, "o servidor devia empurrar key-derivation tls-ekm");
}

#[test]
#[ignore = "precisa dos servidores de testes/openvpn/servidores.sh"]
fn renegociacao_contra_o_openvpn() {
    let o = opcoes(Wrap::TlsAuth, Some("ta.key"), false);
    confere(&conversar(o, 19194, false, true), 2);
}

// ---- o motor inteiro contra o servidor de verdade ----

use tomni_ui_net::{Proxy, connect};
use tomni_ui_openvpn::config::{Family, Remote};
use tomni_ui_openvpn::{OpenVpn, OpenVpnBuilder, State};

/// O perfil do cliente apontando para `porta` do servidor de teste.
fn perfil_real(wrap: Wrap, arquivo: Option<&str>, tcp: bool, porta: u16) -> Config {
    let (mut config, _) =
        Config::from_ovpn(&ler("tudo-embutido.ovpn"), Some(&pasta())).expect("perfil");
    config.pki.wrap = wrap;
    config.pki.static_key = arquivo.map(|a| StaticKey::from_pem(&ler(a)).expect("chave estática"));
    config.pki.key_direction = arquivo.map(|_| 1);
    config.remotes = vec![Remote {
        host: host(),
        port: porta,
        proto: if tcp { Proto::Tcp } else { Proto::Udp },
        family: Family::V4,
    }];
    config
}

fn conectar(config: Config) -> OpenVpn {
    let vpn = OpenVpnBuilder::new(config)
        .name("real")
        .start()
        .expect("abre");
    vpn.wait_connected(Duration::from_secs(30))
        .unwrap_or_else(|e| panic!("não conectou: {e}"));
    vpn
}

/// Manda bytes ao eco do servidor (porta 7000, no endereço dele dentro da VPN) e confere a volta.
fn eco(proxy: &Proxy, alvo: &str, bytes: usize) {
    let dados: Vec<u8> = (0..bytes).map(|i| (i * 31 % 251) as u8).collect();
    let mut s = connect(alvo, 7000, Some(proxy), Duration::from_secs(10)).expect("pela ponte");
    s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    let mut escrita = s.try_clone().unwrap();
    let enviado = dados.clone();
    let escritor = std::thread::spawn(move || escrita.write_all(&enviado));
    let mut volta = vec![0u8; bytes];
    s.read_exact(&mut volta).expect("a volta do eco");
    escritor.join().unwrap().unwrap();
    assert_eq!(volta, dados);
}

fn verifica(wrap: Wrap, arquivo: Option<&str>, tcp: bool, porta: u16, alvo: &str) -> OpenVpn {
    let vpn = conectar(perfil_real(wrap, arquivo, tcp, porta));
    eco(&vpn.proxy(), alvo, 512 * 1024);
    vpn
}

#[test]
#[ignore = "precisa dos servidores de testes/openvpn/servidores.sh"]
fn dados_pelo_tls_auth_udp_e_o_banner() {
    let vpn = verifica(Wrap::TlsAuth, Some("ta.key"), false, 19194, "10.8.1.1");
    let State::Connected { cipher, .. } = vpn.state() else {
        panic!("{:?}", vpn.state())
    };
    assert_eq!(cipher, "AES-256-GCM");
    let s = connect(
        "10.8.1.1",
        7001,
        Some(&vpn.proxy()),
        Duration::from_secs(10),
    )
    .unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut banner = String::new();
    std::io::BufRead::read_line(&mut std::io::BufReader::new(s), &mut banner).unwrap();
    assert!(banner.starts_with("SSH-2.0-"), "{banner}");
}

#[test]
#[ignore = "precisa dos servidores de testes/openvpn/servidores.sh"]
fn dados_pelo_tls_crypt_udp() {
    verifica(Wrap::TlsCrypt, Some("tc.key"), false, 19195, "10.8.2.1");
}

#[test]
#[ignore = "precisa dos servidores de testes/openvpn/servidores.sh"]
fn dados_por_tcp() {
    verifica(Wrap::None, None, true, 19196, "10.8.3.1");
}

#[test]
#[ignore = "precisa dos servidores de testes/openvpn/servidores.sh"]
fn dados_com_tls_1_3_e_chaves_do_exportador() {
    let mut config = perfil_real(Wrap::TlsCrypt, Some("tc.key"), false, 19197);
    config.tls_version_min = Some(TlsVersion::V1_3);
    let vpn = conectar(config);
    eco(&vpn.proxy(), "10.8.4.1", 256 * 1024);
}

#[test]
#[ignore = "precisa dos servidores de testes/openvpn/servidores.sh"]
fn dados_com_chacha20_poly1305() {
    let vpn = verifica(Wrap::TlsCrypt, Some("tc.key"), false, 19198, "10.8.5.1");
    let State::Connected { cipher, .. } = vpn.state() else {
        panic!("{:?}", vpn.state())
    };
    assert_eq!(cipher, "CHACHA20-POLY1305");
}

#[test]
#[ignore = "precisa dos servidores de testes/openvpn/servidores.sh"]
fn dados_com_compress_stub_v2() {
    verifica(Wrap::TlsCrypt, Some("tc.key"), false, 19199, "10.8.6.1");
}

#[test]
#[ignore = "precisa dos servidores de testes/openvpn/servidores.sh"]
fn dns_empurrado_pelo_servidor_resolve_pela_ponte() {
    let vpn = conectar(perfil_real(Wrap::TlsAuth, Some("ta.key"), false, 19194));
    // O dnsmasq do servidor conhece `alvo.corp.test`; `alvo` sozinho usa o domínio empurrado.
    for nome in ["alvo.corp.test", "alvo"] {
        let mut s = connect(nome, 7000, Some(&vpn.proxy()), Duration::from_secs(10))
            .unwrap_or_else(|e| panic!("{nome}: {e}"));
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        s.write_all(b"ok").unwrap();
        let mut b = [0u8; 2];
        s.read_exact(&mut b).unwrap();
        assert_eq!(&b, b"ok");
    }
}

/// Roda um comando dentro do contêiner dos servidores (o `rodar.sh` dá o nome).
fn no_conteiner(args: &[&str]) -> String {
    let nome = std::env::var("TOMNI_UI_OPENVPN_CONTAINER")
        .unwrap_or_else(|_| "tomni-ui-openvpn-teste".to_owned());
    let saida = std::process::Command::new("docker")
        .arg("exec")
        .arg(&nome)
        .args(args)
        .output()
        .expect("docker exec");
    String::from_utf8_lossy(&[saida.stdout, saida.stderr].concat()).into_owned()
}

fn log_do_conteiner() -> String {
    let nome = std::env::var("TOMNI_UI_OPENVPN_CONTAINER")
        .unwrap_or_else(|_| "tomni-ui-openvpn-teste".to_owned());
    let saida = std::process::Command::new("docker")
        .args(["logs", &nome])
        .output()
        .expect("docker logs");
    String::from_utf8_lossy(&[saida.stdout, saida.stderr].concat()).into_owned()
}

#[test]
#[ignore = "precisa dos servidores de testes/openvpn/servidores.sh"]
fn o_aviso_de_saida_chega_ao_servidor() {
    let vpn = conectar(perfil_real(Wrap::TlsAuth, Some("ta.key"), false, 19194));
    vpn.stop(Duration::from_secs(5));
    let fim = Instant::now() + Duration::from_secs(10);
    loop {
        let log = log_do_conteiner();
        if log.contains("exit message received by peer") {
            break;
        }
        assert!(
            Instant::now() < fim,
            "o servidor não registrou o aviso de saída:\n{}",
            log.lines().rev().take(15).collect::<Vec<_>>().join("\n")
        );
        std::thread::sleep(Duration::from_millis(300));
    }
}

#[test]
#[ignore = "precisa dos servidores de testes/openvpn/servidores.sh"]
fn reinicio_do_servidor_reconecta_e_os_dados_voltam() {
    let vpn = conectar(perfil_real(Wrap::TlsAuth, Some("ta.key"), false, 19194));
    eco(&vpn.proxy(), "10.8.1.1", 64 * 1024);
    // O USR1 faz o servidor mandar RESTART aos clientes (`explicit-exit-notify`).
    no_conteiner(&["sh", "-c", "kill -USR1 $(cat /tmp/ovpn-19194.pid)"]);
    let fim = Instant::now() + Duration::from_secs(10);
    while !matches!(vpn.state(), State::Retrying(_) | State::Connecting { .. })
        && Instant::now() < fim
    {
        std::thread::sleep(Duration::from_millis(50));
    }
    let fim = Instant::now() + Duration::from_secs(40);
    while !matches!(vpn.state(), State::Connected { .. }) {
        assert!(Instant::now() < fim, "não reconectou: {:?}", vpn.state());
        std::thread::sleep(Duration::from_millis(100));
    }
    eco(&vpn.proxy(), "10.8.1.1", 64 * 1024);
}

#[test]
#[ignore = "precisa dos servidores de testes/openvpn/servidores.sh"]
fn o_ping_dos_dois_lados_mantem_a_sessao_viva() {
    // O servidor da 19199 usa `keepalive 2 8`: se ele não reconhecesse o ping do cliente, a
    // instância reiniciaria em 8 s; se o cliente não reconhecesse o do servidor, o
    // `ping-restart` empurrado (16 s) derrubaria a sessão.
    let vpn = conectar(perfil_real(Wrap::TlsCrypt, Some("tc.key"), false, 19199));
    std::thread::sleep(Duration::from_secs(20));
    assert!(
        matches!(vpn.state(), State::Connected { .. }),
        "{:?}",
        vpn.state()
    );
    eco(&vpn.proxy(), "10.8.6.1", 32 * 1024);
    assert!(
        !log_do_conteiner().contains("Inactivity timeout"),
        "o servidor deu a sessão por parada"
    );
}
