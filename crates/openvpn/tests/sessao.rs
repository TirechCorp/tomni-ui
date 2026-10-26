//! A sessão do cliente (controle e dados) contra o servidor de teste, em memória: pacotes nos
//! dois sentidos, as três cifras, UDP e TCP, perda, renegociação, ping, saída e as falhas.
//!
//! O tempo é um `Instant` que o teste empurra; nada aqui usa rede.

mod comum;

use std::time::Duration;

use comum::rede::{ParSessao, aceita, cliente, empurra, id_do_pacote, pacote_ip, servidor};
use tomni_ui_openvpn::auth::{AuthFailure, AuthState};
use tomni_ui_openvpn::session::{Failure, SessionEvent};
use tomni_ui_openvpn::test_server::{Decision, ServerEvent};

const CIFRAS: [&str; 3] = ["AES-256-GCM", "AES-128-GCM", "CHACHA20-POLY1305"];

fn par(extra: &str, tcp: bool, push_extra: &str) -> ParSessao {
    ParSessao::novo(
        cliente(extra, tcp, AuthState::none()),
        servidor(tcp, empurra("10.8.0.2", push_extra)),
        aceita,
    )
}

fn conectado(extra: &str, tcp: bool, push_extra: &str) -> ParSessao {
    let mut p = par(extra, tcp, push_extra);
    p.conectar();
    p
}

/// Manda `n` pacotes do cliente para o servidor e `n` do servidor para o cliente, um a cada
/// 20 ms.
fn trocar(p: &mut ParSessao, n: u32, tamanho: usize) {
    for i in 0..n {
        let ip = pacote_ip(i, tamanho);
        p.cliente.send_ip(&ip, p.t);
        p.servidor.send_ip(&pacote_ip(1_000_000 + i, tamanho));
        p.rodar_por(Duration::from_millis(20));
    }
    p.drenar();
}

trait Avanca {
    fn rodar_por(&mut self, d: Duration);
}

impl Avanca for ParSessao {
    fn rodar_por(&mut self, d: Duration) {
        self.t += d;
        self.cliente.handle_timeout(self.t);
        self.servidor.handle_timeout(self.t);
        self.drenar();
    }
}

#[test]
fn conecta_e_leva_pacotes_nos_dois_sentidos_nas_tres_cifras_em_udp_e_tcp() {
    for tcp in [false, true] {
        for cifra in CIFRAS {
            let push = format!("cipher {cifra}");
            let mut p = conectado("", tcp, &push);
            assert_eq!(p.conexao().cipher.name(), cifra);
            trocar(&mut p, 50, 700);
            assert_eq!(p.ip_servidor.len(), 50, "{cifra} tcp={tcp}");
            assert_eq!(p.ip_cliente.len(), 50, "{cifra} tcp={tcp}");
            for (i, ip) in p.ip_servidor.iter().enumerate() {
                assert_eq!(*ip, pacote_ip(i as u32, 700));
            }
            for (i, ip) in p.ip_cliente.iter().enumerate() {
                assert_eq!(*ip, pacote_ip(1_000_000 + i as u32, 700));
            }
            assert_eq!(p.cliente.rejected_packets(), 0);
            assert_eq!(p.servidor.rejected_packets(), 0);
            assert!(p.falha().is_none(), "{:?}", p.falha());
        }
    }
}

#[test]
fn a_conexao_traz_endereco_rotas_e_mtu() {
    let p = conectado(
        "",
        false,
        "dhcp-option DNS 10.8.0.1,dhcp-option DOMAIN vpn.exemplo",
    );
    let c = p.conexao();
    assert_eq!(c.address.to_string(), "10.8.0.2");
    assert_eq!(c.prefix_len, 24);
    assert!(
        c.routes
            .iter()
            .any(|r| r.network.to_string() == "10.9.0.0" && r.prefix_len == 24)
    );
    assert_eq!(c.dns_servers, vec!["10.8.0.1:53".parse().unwrap()]);
    assert_eq!(c.search_domains, vec!["vpn.exemplo"]);
    assert!(!c.redirect_gateway);
    // UDP: o mssfix padrão (1450) menos os 24 bytes do cabeçalho e da etiqueta.
    assert_eq!(c.mtu, 1450 - 24);
    let tcp = conectado("", true, "");
    assert_eq!(tcp.conexao().mtu, 1500);
    let pequeno = conectado("mssfix 1300", false, "");
    assert_eq!(pequeno.conexao().mtu, 1300 - 24);
    let sem = conectado("mssfix 0", false, "");
    assert_eq!(sem.conexao().mtu, 1500);
    let empurrado = conectado("", true, "tun-mtu 1400");
    assert_eq!(empurrado.conexao().mtu, 1400);
}

#[test]
fn rotas_e_dns_do_ovpn_somam_com_os_empurrados() {
    let p = conectado(
        "route 172.16.0.0 255.255.0.0\ndhcp-option DNS 10.8.0.9\nredirect-gateway def1",
        false,
        "dhcp-option DNS 10.8.0.1",
    );
    let c = p.conexao();
    assert!(
        c.routes
            .iter()
            .any(|r| r.network.to_string() == "172.16.0.0" && r.prefix_len == 16)
    );
    assert!(c.routes.iter().any(|r| r.prefix_len == 0), "rota padrão");
    assert!(c.redirect_gateway);
    assert_eq!(
        c.dns_servers,
        vec![
            "10.8.0.1:53".parse().unwrap(),
            "10.8.0.9:53".parse().unwrap()
        ]
    );
}

#[test]
fn compressao_stub_stub_v2_e_lzo_desligado_andam_juntas() {
    for extra in [
        "compress",
        "compress stub",
        "compress stub-v2",
        "comp-lzo no",
    ] {
        let mut p = conectado("", false, extra);
        trocar(&mut p, 10, 300);
        assert_eq!(p.ip_servidor.len(), 10, "{extra}");
        assert_eq!(p.ip_cliente.len(), 10, "{extra}");
        // O MTU leva em conta o byte do enquadramento.
        let esperado = if extra == "compress stub-v2" {
            1450 - 26
        } else {
            1450 - 25
        };
        assert_eq!(p.conexao().mtu, esperado, "{extra}");
    }
}

#[test]
fn cifra_que_o_app_nao_faz_vira_erro_com_o_nome() {
    let mut p = par("", false, "cipher BF-CBC");
    p.rodar(60, |p| p.falha().is_some());
    match p.falha() {
        Some(Failure::Unsupported(m)) => {
            assert!(m.contains("BF-CBC") && m.contains("data-ciphers"), "{m}");
        }
        outro => panic!("{outro:?}"),
    }
    assert!(p.cliente.is_finished());
}

#[test]
fn compressao_de_verdade_do_servidor_vira_erro_claro() {
    for (extra, forjado, algoritmo) in [
        ("compress stub-v2", vec![0x50, 0x02, 1, 2, 3], "LZO"),
        ("compress stub-v2", vec![0x50, 0x01, 1, 2, 3], "LZ4"),
        ("comp-lzo no", vec![0x66, 1, 2, 3], "LZO"),
        ("compress", vec![0x69, 1, 2, 3], "LZ4"),
    ] {
        let mut p = conectado("", false, extra);
        p.servidor.send_raw(&forjado);
        p.rodar(5, |p| p.falha().is_some());
        match p.falha() {
            Some(Failure::Unsupported(m)) => {
                assert!(m.contains(&format!("comprime com {algoritmo}")), "{m}");
            }
            outro => panic!("{extra}: {outro:?}"),
        }
    }
}

#[test]
fn com_perda_o_udp_negocia_e_os_pacotes_que_chegam_estao_inteiros() {
    let mut p = par("", false, "");
    p.perda = Some(Box::new(|_, n| n % 3 == 2));
    p.conectar();
    p.perda = Some(Box::new(|_, n| n % 4 == 1));
    trocar(&mut p, 200, 500);
    let chegados = p.ip_servidor.len();
    assert!(chegados > 100 && chegados < 200, "{chegados}");
    let mut ids: Vec<u32> = p.ip_servidor.iter().map(|ip| id_do_pacote(ip)).collect();
    let antes = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), antes, "nenhum pacote chega duas vezes");
    for ip in &p.ip_servidor {
        assert_eq!(*ip, pacote_ip(id_do_pacote(ip), 500));
    }
    assert!(p.falha().is_none());
}

#[test]
fn renegocia_pelo_reneg_sec_sem_perder_pacote() {
    let mut p = conectado("reneg-sec 2", false, "");
    let inicio = p.t;
    let mut mandados = 0u32;
    while p.t < inicio + Duration::from_secs(7) {
        p.cliente.send_ip(&pacote_ip(mandados, 200), p.t);
        p.servidor.send_ip(&pacote_ip(1_000_000 + mandados, 200));
        mandados += 1;
        p.rodar_por(Duration::from_millis(25));
    }
    p.drenar();
    assert!(p.trocas_de_chave() >= 2, "trocas: {}", p.trocas_de_chave());
    assert_eq!(p.ip_servidor.len() as u32, mandados);
    assert_eq!(p.ip_cliente.len() as u32, mandados);
    assert!(p.falha().is_none(), "{:?}", p.falha());
    assert_eq!(p.cliente.rejected_packets(), 0);
}

#[test]
fn renegocia_depois_de_reneg_pkts_pacotes() {
    let mut p = conectado("reneg-sec 0\nreneg-pkts 40", false, "");
    for i in 0..60 {
        p.cliente.send_ip(&pacote_ip(i, 100), p.t);
        p.rodar_por(Duration::from_millis(20));
    }
    p.rodar(10, |p| p.trocas_de_chave() >= 1);
    assert!(p.trocas_de_chave() >= 1);
    assert_eq!(p.ip_servidor.len(), 60);
}

#[test]
fn a_linha_parada_troca_pings_nos_dois_sentidos() {
    let mut p = conectado("", false, "");
    p.rodar(200, |_| false);
    assert!(
        p.servidor.pings_received() >= 15,
        "{}",
        p.servidor.pings_received()
    );
    // Os pings do servidor mantêm o cliente vivo além do ping-restart de 60 s.
    assert!(p.falha().is_none(), "{:?}", p.falha());
    assert!(
        p.ip_cliente.is_empty() && p.ip_servidor.is_empty(),
        "ping não vira pacote IP"
    );
}

#[test]
fn o_ping_restart_derruba_a_sessao_quando_ninguem_responde() {
    let mut p = conectado("", false, "");
    p.corte = true;
    p.rodar(90, |p| p.falha().is_some());
    assert_eq!(p.falha(), Some(&Failure::PingRestart(60)));
    assert!(p.cliente.is_finished());
    // O prazo foi respeitado: nem antes de 60 s.
    assert!(p.t >= p.t0 && p.cliente.next_timeout().is_none());
}

#[test]
fn ping_exit_encerra_em_vez_de_reiniciar() {
    let mut p = conectado("ping-exit 30\nping-restart 0", false, "ping-restart 0");
    p.corte = true;
    p.rodar(90, |p| p.falha().is_some());
    assert_eq!(p.falha(), Some(&Failure::PingExit(30)));
}

#[test]
fn avisa_a_saida_no_udp_com_occ_exit() {
    let mut p = conectado("", false, "");
    p.cliente.close(p.t);
    p.drenar();
    assert_eq!(p.servidor.exits_received(), 1);
    let mut p = conectado("explicit-exit-notify 3", false, "");
    p.cliente.close(p.t);
    p.drenar();
    assert_eq!(p.servidor.exits_received(), 3);
    // Depois de fechar, nada mais sai.
    p.cliente.send_ip(&pacote_ip(1, 10), p.t);
    p.drenar();
    assert!(p.ip_servidor.is_empty());
}

#[test]
fn com_cc_exit_o_aviso_vai_pelo_canal_de_controle() {
    let mut p = conectado("", false, "protocol-flags cc-exit");
    p.cliente.close(p.t);
    p.rodar(5, |p| {
        p.eventos_s
            .iter()
            .any(|e| matches!(e, ServerEvent::Message(m) if m == "EXIT"))
    });
    assert!(
        p.eventos_s
            .iter()
            .any(|e| matches!(e, ServerEvent::Message(m) if m == "EXIT"))
    );
    assert_eq!(p.servidor.exits_received(), 0);
}

#[test]
fn no_tcp_fechar_nao_manda_aviso() {
    let mut p = conectado("explicit-exit-notify", true, "");
    p.cliente.close(p.t);
    p.drenar();
    assert_eq!(p.servidor.exits_received(), 0);
}

#[test]
fn restart_e_halt_do_servidor_acabam_a_sessao() {
    let mut p = conectado("", false, "");
    p.servidor.send_message("RESTART,reinicio de teste");
    p.rodar(5, |p| p.cliente.is_finished());
    assert!(
        p.eventos
            .iter()
            .any(|e| matches!(e, SessionEvent::Restart(t) if t.contains("reinicio")))
    );
    let mut p = conectado("", false, "");
    p.servidor.send_message("HALT,fim");
    p.rodar(5, |p| p.cliente.is_finished());
    assert!(p.eventos.iter().any(|e| matches!(e, SessionEvent::Halt(_))));
}

#[test]
fn senha_recusada_chega_como_auth_failed() {
    let mut p = ParSessao::novo(
        cliente("", false, AuthState::none()),
        servidor(false, empurra("10.8.0.2", "")),
        |_| Decision::Fail("AUTH_FAILED,senha errada".to_owned()),
    );
    p.rodar(30, |p| p.cliente.is_finished());
    assert!(p.eventos.iter().any(|e| matches!(
        e,
        SessionEvent::AuthFailed(AuthFailure::Rejected { reason: Some(r) }) if r == "senha errada"
    )));
    assert!(!p.conectado());
}

#[test]
fn lixo_e_pacote_adulterado_sao_contados_e_nao_atrapalham() {
    let mut p = conectado("", false, "");
    // Datagramas de dados forjados: opcode de dados, key_id vivo, etiqueta que não confere.
    for i in 0..20u8 {
        let mut forjado = vec![9 << 3, 0, 0, 0, 0, 0, 0, i + 1];
        forjado.extend((0..40).map(|b| b ^ i));
        p.cliente.handle_datagram(&forjado, p.t);
    }
    p.cliente.handle_datagram(&[1, 2, 3], p.t);
    assert_eq!(p.cliente.rejected_packets(), 20);
    trocar(&mut p, 5, 100);
    assert_eq!(p.ip_cliente.len(), 5);
    assert!(p.falha().is_none());
}

#[test]
fn sem_chave_o_pacote_e_descartado_sem_erro() {
    let mut p = par("", false, "");
    p.cliente.send_ip(&pacote_ip(1, 10), p.t);
    p.drenar();
    assert!(p.ip_servidor.is_empty());
    assert!(p.falha().is_none());
}

#[test]
fn os_pacotes_do_servidor_com_replay_sao_recusados() {
    let mut p = conectado("", false, "");
    p.servidor.send_ip(&pacote_ip(7, 100));
    let dg = p.servidor.poll_transmit(p.t).expect("datagrama de dados");
    p.cliente.handle_datagram(&dg, p.t);
    p.cliente.handle_datagram(&dg, p.t);
    p.drenar();
    assert_eq!(p.ip_cliente.len(), 1);
    assert_eq!(p.cliente.rejected_packets(), 1);
}
