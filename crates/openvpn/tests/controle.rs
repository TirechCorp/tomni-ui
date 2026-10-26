//! O canal de controle do cliente contra o servidor de teste, em memória: a troca de chaves, a
//! verificação do certificado, a senha, o `PUSH_REPLY`, os tempos e a robustez.
//!
//! Nada aqui usa rede nem relógio de verdade: o tempo é um `Instant` que o teste empurra, e a
//! hora dos certificados é o `Clock::fixed` das fixtures.

mod comum;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant, UNIX_EPOCH};

use comum::{agora, ler, pasta, referencias};
use tomni_ui_openvpn::auth::{
    AuthFailure, AuthState, Credentials, parse_dynamic_challenge_password,
    parse_static_challenge_password,
};
use tomni_ui_openvpn::channel::{Clock, Datagram};
use tomni_ui_openvpn::config::{Config, NameMode, Proto, ServerVerify, TlsVersion, X509Name};
use tomni_ui_openvpn::control::{Control, ControlError, ControlOptions, Event};
use tomni_ui_openvpn::packet::{TcpDeframer, tcp_frame};
use tomni_ui_openvpn::pki::{Pki, PrivateKey, Wrap, parse_certificates};
use tomni_ui_openvpn::prf::DataKeys;
use tomni_ui_openvpn::static_key::StaticKey;
use tomni_ui_openvpn::test_server::{Attempt, Decision, ServerControl, ServerEvent, ServerOptions};
use tomni_ui_openvpn::verify::VerifyError;

// ---------------------------------------------------------------------------------------------
// O cenário e o par cliente/servidor.
// ---------------------------------------------------------------------------------------------

/// Um cliente e um servidor de teste, prontos para conversar.
struct Cenario {
    cliente: ControlOptions,
    servidor: ServerOptions,
}

fn certs(nome: &str) -> Vec<rustls::pki_types::CertificateDer<'static>> {
    parse_certificates(&ler(nome)).unwrap_or_else(|e| panic!("{nome}: {e}"))
}

fn chave(nome: &str) -> PrivateKey {
    PrivateKey::from_pem(&ler(nome)).unwrap_or_else(|e| panic!("{nome}: {e}"))
}

fn chave_estatica(nome: &str) -> StaticKey {
    StaticKey::from_pem(&ler(nome)).unwrap_or_else(|e| panic!("{nome}: {e}"))
}

impl Cenario {
    /// RSA, UDP, `tls-auth`, `auth SHA256`, como o `tudo-embutido.ovpn`.
    fn novo() -> Cenario {
        let (config, _) =
            Config::from_ovpn(&ler("tudo-embutido.ovpn"), Some(&pasta())).expect("perfil");
        let cliente = ControlOptions::from_config(
            &config,
            Proto::Udp,
            false,
            AuthState::none(),
            Clock::fixed(agora()),
        )
        .expect("opções do cliente");
        let mut servidor = ServerOptions::new(
            certs("servidor.crt"),
            chave("servidor.key"),
            certs("ca.crt"),
            Clock::fixed(agora()),
        );
        servidor.auth_digest = tomni_ui_openvpn::config::AuthDigest::Sha256;
        let mut c = Cenario { cliente, servidor };
        c.wrap(Wrap::TlsAuth);
        c
    }

    /// A proteção do canal de controle nos dois lados, com a chave certa para cada um.
    fn wrap(&mut self, wrap: Wrap) {
        let (arquivo, dir_cliente, dir_servidor) = match &wrap {
            Wrap::None => (None, None, None),
            Wrap::TlsAuth => (Some("ta.key"), Some(1), Some(1)),
            Wrap::TlsCrypt => (Some("tc.key"), None, None),
            Wrap::TlsCryptV2 { .. } => panic!("o servidor de teste não faz tls-crypt-v2"),
        };
        let chave = arquivo.map(chave_estatica);
        self.cliente.pki.wrap = wrap.clone();
        self.cliente.pki.static_key = chave.clone();
        self.cliente.pki.key_direction = dir_cliente;
        self.servidor.wrap = Pki {
            wrap,
            static_key: chave,
            key_direction: dir_servidor,
            ..Pki::default()
        };
    }

    fn tcp(&mut self) {
        self.cliente.transport = Proto::Tcp;
        self.servidor.transport = Proto::Tcp;
    }

    fn ekm(&mut self) {
        self.servidor.ekm = true;
        self.servidor.push = vec![ServerOptions::default_push(true)];
    }

    fn versao(&mut self, v: TlsVersion) {
        self.cliente.tls_min = Some(v);
        self.cliente.tls_max = Some(v);
        self.servidor.tls_min = Some(v);
        self.servidor.tls_max = Some(v);
    }

    fn tls_mtu(&mut self, mtu: usize) {
        self.cliente.tls_mtu = mtu;
        self.servidor.tls_mtu = mtu;
    }

    /// Troca o par de certificados para o de curva elíptica (`ca-ec`).
    fn ec(&mut self) {
        self.cliente.pki.ca = certs("ca-ec.crt");
        self.cliente.pki.cert = Some(certs("cliente-ec.crt"));
        self.cliente.pki.key = Some(chave("cliente-ec.key"));
        self.servidor.chain = certs("servidor-ec.crt");
        self.servidor.key = chave("servidor-ec.key");
        self.servidor.client_ca = certs("ca-ec.crt");
    }

    /// O servidor apresenta este certificado (e o cliente continua confiando na `ca`).
    fn servidor_com(&mut self, cert: &str, key: &str) {
        self.servidor.chain = certs(cert);
        self.servidor.key = chave(key);
    }

    /// O cliente apresenta este certificado.
    fn cliente_com(&mut self, cert: &str, key: &str) {
        self.cliente.pki.cert = Some(certs(cert));
        self.cliente.pki.key = Some(chave(key));
    }

    fn sem_conferir(&mut self) {
        self.cliente.verify = ServerVerify::default();
    }
}

/// Tudo aceito, sem token.
fn aceita(_: &Attempt) -> Decision {
    Decision::Accept { token: None }
}

type Perda = Box<dyn FnMut(bool, u64) -> bool>;

/// O cliente e o servidor ligados por uma "rede" que o teste controla.
struct Par {
    cliente: Control,
    servidor: ServerControl,
    t0: Instant,
    t: Instant,
    tcp: bool,
    eventos: Vec<Event>,
    eventos_s: Vec<ServerEvent>,
    /// Decide se um datagrama se perde: recebe "é do cliente" e quantos esse lado já enviou.
    perda: Option<Perda>,
    contagem: [u64; 2],
    capturados: Vec<(bool, Vec<u8>)>,
    desquadrador_cliente: TcpDeframer,
    desquadrador_servidor: TcpDeframer,
}

impl Par {
    fn novo(c: Cenario, autenticar: impl FnMut(&Attempt) -> Decision + 'static) -> Par {
        let t0 = Instant::now();
        let tcp = c.cliente.transport == Proto::Tcp;
        let cliente = Control::new(c.cliente, t0).expect("cliente");
        let servidor = ServerControl::new(c.servidor, autenticar).expect("servidor");
        Par {
            cliente,
            servidor,
            t0,
            t: t0,
            tcp,
            eventos: Vec::new(),
            eventos_s: Vec::new(),
            perda: None,
            contagem: [0, 0],
            capturados: Vec::new(),
            desquadrador_cliente: TcpDeframer::new(),
            desquadrador_servidor: TcpDeframer::new(),
        }
    }

    fn entregar(&mut self, do_cliente: bool, pacote: Vec<u8>) {
        self.capturados.push((do_cliente, pacote.clone()));
        let lado = usize::from(do_cliente);
        self.contagem[lado] += 1;
        let n = self.contagem[lado];
        if let Some(f) = self.perda.as_mut()
            && f(do_cliente, n)
        {
            return;
        }
        let chegaram = if self.tcp {
            let quadro = tcp_frame(&pacote).expect("quadro TCP");
            let d = if do_cliente {
                &mut self.desquadrador_servidor
            } else {
                &mut self.desquadrador_cliente
            };
            // Em pedaços de 7 bytes, para exercitar a remontagem.
            for pedaco in quadro.chunks(7) {
                d.push(pedaco);
            }
            let mut v = Vec::new();
            while let Some(p) = d.next_packet() {
                v.push(p);
            }
            v
        } else {
            vec![pacote]
        };
        for d in chegaram {
            if do_cliente {
                self.servidor.handle_datagram(&d, self.t);
            } else {
                self.cliente.handle_datagram(&d, self.t);
            }
        }
    }

    /// Passa tudo o que está pronto de um lado para o outro, até acalmar.
    fn drenar(&mut self) {
        loop {
            let mut moveu = false;
            while let Some(p) = self.cliente.poll_transmit(self.t) {
                moveu = true;
                self.entregar(true, p);
            }
            while let Some(p) = self.servidor.poll_transmit(self.t) {
                moveu = true;
                self.entregar(false, p);
            }
            while let Some(e) = self.cliente.poll_event() {
                self.eventos.push(e);
            }
            while let Some(e) = self.servidor.poll_event() {
                self.eventos_s.push(e);
            }
            if !moveu {
                break;
            }
        }
    }

    /// Roda até `parar` ser verdadeiro, o cliente acabar, ou passarem `limite_s` segundos (do
    /// tempo do teste) sem nada a fazer.
    fn rodar(&mut self, limite_s: u64, parar: impl Fn(&Par) -> bool) {
        let fim = self.t + Duration::from_secs(limite_s);
        for _ in 0..200_000 {
            self.drenar();
            if parar(self) || self.cliente.is_finished() {
                return;
            }
            let proximo = [self.cliente.next_timeout(), self.servidor.next_timeout()]
                .into_iter()
                .flatten()
                .min();
            match proximo {
                Some(p) if p <= fim => {
                    self.t = p.max(self.t);
                    self.cliente.handle_timeout(self.t);
                }
                _ => {
                    self.t = fim.max(self.t);
                    self.cliente.handle_timeout(self.t);
                    self.drenar();
                    return;
                }
            }
        }
        panic!("o laço de simulação não terminou");
    }

    fn chaves_cliente(&self, id: u8) -> Option<&DataKeys> {
        self.eventos.iter().find_map(|e| match e {
            Event::DataKeys { key_id, keys } if *key_id == id => Some(keys),
            _ => None,
        })
    }

    fn chaves_servidor(&self, id: u8) -> Option<&DataKeys> {
        self.eventos_s.iter().find_map(|e| match e {
            ServerEvent::DataKeys { key_id, keys } if *key_id == id => Some(&**keys),
            _ => None,
        })
    }

    fn falha(&self) -> Option<&ControlError> {
        self.eventos.iter().find_map(|e| match e {
            Event::Failed(f) => Some(f),
            _ => None,
        })
    }

    fn falha_auth(&self) -> Option<&AuthFailure> {
        self.eventos.iter().find_map(|e| match e {
            Event::AuthFailed(f) => Some(f),
            _ => None,
        })
    }

    fn avisos(&self) -> Vec<&str> {
        self.eventos
            .iter()
            .filter_map(|e| match e {
                Event::Warning(w) => Some(w.as_str()),
                _ => None,
            })
            .collect()
    }

    fn pushed(&self) -> Option<&tomni_ui_openvpn::push::Pushed> {
        self.eventos.iter().find_map(|e| match e {
            Event::Pushed(p) => Some(&**p),
            _ => None,
        })
    }

    fn mensagens_do_servidor(&self) -> Vec<&str> {
        self.eventos_s
            .iter()
            .filter_map(|e| match e {
                ServerEvent::Message(m) => Some(m.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Confere que as chaves de uma geração batem nos dois lados (o que um envia, o outro recebe).
    fn confere_chaves(&self, id: u8) {
        let c = self
            .chaves_cliente(id)
            .unwrap_or_else(|| panic!("o cliente não tem as chaves {id}: {:?}", self.eventos));
        let s = self
            .chaves_servidor(id)
            .unwrap_or_else(|| panic!("o servidor não tem as chaves {id}: {:?}", self.eventos_s));
        assert!(
            c.send == s.recv,
            "o que o cliente envia não é o que o servidor recebe"
        );
        assert!(
            c.recv == s.send,
            "o que o servidor envia não é o que o cliente recebe"
        );
        assert!(
            c.send != c.recv,
            "as duas direções não podem ter a mesma chave"
        );
    }
}

/// As chaves da geração `id` existem nos dois lados.
fn pronto(id: u8) -> impl Fn(&Par) -> bool {
    move |p| p.chaves_cliente(id).is_some() && p.chaves_servidor(id).is_some()
}

/// Conecta e confere a primeira geração de chaves.
fn conectar(c: Cenario) -> Par {
    conectar_com(c, aceita)
}

fn conectar_com(c: Cenario, autenticar: impl FnMut(&Attempt) -> Decision + 'static) -> Par {
    let mut par = Par::novo(c, autenticar);
    par.rodar(50, pronto(0));
    assert!(
        par.falha().is_none(),
        "a negociação falhou: {:?} / servidor: {:?}",
        par.falha(),
        par.eventos_s
    );
    par.confere_chaves(0);
    par
}

/// Roda até o cliente acabar e devolve o par (para olhar o erro).
fn falhar(c: Cenario) -> Par {
    let mut par = Par::novo(c, aceita);
    par.rodar(90, |_| false);
    par
}

fn hex(texto: &str) -> Vec<u8> {
    (0..texto.len() / 2)
        .map(|i| u8::from_str_radix(&texto[2 * i..2 * i + 2], 16).expect("hexadecimal"))
        .collect()
}

fn sha256(nome: &str) -> [u8; 32] {
    hex(&referencias()[&format!("{nome}.sha256")])
        .try_into()
        .expect("32 bytes")
}

// ---------------------------------------------------------------------------------------------
// A negociação e as chaves.
// ---------------------------------------------------------------------------------------------

#[test]
fn negocia_em_todas_as_combinacoes_de_protecao_transporte_e_derivacao() {
    for wrap in [Wrap::None, Wrap::TlsAuth, Wrap::TlsCrypt] {
        for tcp in [false, true] {
            for ekm in [false, true] {
                let mut c = Cenario::novo();
                c.wrap(wrap.clone());
                if tcp {
                    c.tcp();
                }
                if ekm {
                    c.ekm();
                }
                let par = conectar(c);
                let p = par.pushed().expect("recebeu a configuração");
                assert_eq!(p.key_derivation_ekm, ekm);
                let ifc = p.ifconfig.as_ref().expect("ifconfig");
                assert_eq!(ifc.address.to_string(), "10.8.0.2");
                assert_eq!(ifc.prefix_len, 24);
                assert_eq!(p.routes.len(), 1);
                assert!(
                    par.eventos_s
                        .iter()
                        .any(|e| matches!(e, ServerEvent::Authenticated { key_id: 0, .. }))
                );
                let cert = par.cliente.server_certificate().expect("certificado visto");
                assert_eq!(cert.common_name.as_deref(), Some("servidor"));
            }
        }
    }
}

#[test]
fn as_chaves_pelo_exportador_e_pela_prf_sao_diferentes_entre_si() {
    let prf = conectar(Cenario::novo());
    let mut c = Cenario::novo();
    c.ekm();
    let ekm = conectar(c);
    // Sessões diferentes têm chaves diferentes de qualquer jeito; o que importa é que cada uma
    // fechou nos dois lados (conferido por `conectar`).
    assert!(prf.chaves_cliente(0).unwrap() != ekm.chaves_cliente(0).unwrap());
}

#[test]
fn negocia_so_em_tls_1_2_e_so_em_tls_1_3() {
    for v in [TlsVersion::V1_2, TlsVersion::V1_3] {
        for ekm in [false, true] {
            let mut c = Cenario::novo();
            c.versao(v);
            if ekm {
                c.ekm();
            }
            conectar(c);
        }
    }
}

#[test]
fn negocia_com_certificados_de_curva_eliptica() {
    let mut c = Cenario::novo();
    c.ec();
    let par = conectar(c);
    assert_eq!(
        par.cliente
            .server_certificate()
            .and_then(|c| c.common_name)
            .as_deref(),
        Some("servidor-ec")
    );
}

#[test]
fn fragmenta_o_tls_em_pacotes_pequenos() {
    for tcp in [false, true] {
        let mut c = Cenario::novo();
        c.tls_mtu(300);
        if tcp {
            c.tcp();
        }
        let par = conectar(c);
        let maior = par
            .capturados
            .iter()
            .map(|(_, p)| p.len())
            .max()
            .expect("pacotes");
        assert!(maior <= 300, "um pacote passou do tls-mtu: {maior} bytes");
    }
}

#[test]
fn o_servidor_recebe_o_peer_info_do_cliente() {
    let vistas: Rc<RefCell<Vec<Attempt>>> = Rc::default();
    let dentro = Rc::clone(&vistas);
    let _ = conectar_com(Cenario::novo(), move |a| {
        dentro.borrow_mut().push(a.clone());
        Decision::Accept { token: None }
    });
    let v = vistas.borrow();
    assert_eq!(v.len(), 1);
    let a = &v[0];
    assert!(a.initial);
    assert_eq!(a.key_id, 0);
    assert_eq!(a.peer("IV_VER"), Some("2.6.0"));
    assert!(a.peer("IV_PLAT").is_some());
    let proto: u32 = a
        .peer("IV_PROTO")
        .expect("IV_PROTO")
        .parse()
        .expect("número");
    assert_ne!(proto & 2, 0, "DATA_V2");
    assert_ne!(proto & 4, 0, "REQUEST_PUSH");
    assert_ne!(proto & 8, 0, "TLS_KEY_EXPORT");
    assert!(a.options.contains("key-method 2"), "{}", a.options);
    assert!(a.username.is_none() && a.password.is_none());
}

#[test]
fn renegocia_e_o_key_id_volta_do_7_para_o_1() {
    let mut par = conectar(Cenario::novo());
    let mut vistos = Vec::new();
    for _ in 0..8 {
        par.cliente.renegotiate(par.t);
        let antes = vistos.len();
        par.rodar(50, |p| {
            p.eventos
                .iter()
                .filter(|e| matches!(e, Event::DataKeys { .. }))
                .count()
                > antes + 1
        });
        let id = par
            .eventos
            .iter()
            .filter_map(|e| match e {
                Event::DataKeys { key_id, .. } => Some(*key_id),
                _ => None,
            })
            .next_back()
            .expect("novas chaves");
        par.confere_chaves(id);
        vistos.push(id);
        // Deixa a geração anterior se aposentar (`tran-window`), para o `key_id` ficar livre.
        par.rodar(70, |_| false);
    }
    assert_eq!(vistos, [1, 2, 3, 4, 5, 6, 7, 1]);
    assert!(par.falha().is_none(), "{:?}", par.falha());
}

// ---------------------------------------------------------------------------------------------
// A verificação do certificado do servidor e a do cliente.
// ---------------------------------------------------------------------------------------------

#[test]
fn remote_cert_tls_recusa_servidor_sem_eku() {
    let mut c = Cenario::novo();
    c.servidor_com("servidor-sem-eku.crt", "servidor-sem-eku.key");
    let par = falhar(c);
    assert!(
        matches!(
            par.falha(),
            Some(ControlError::Verify(
                VerifyError::NotServerCertificate { .. }
            ))
        ),
        "{:?}",
        par.falha()
    );
    assert!(par.cliente.is_finished());
    assert!(par.chaves_cliente(0).is_none());
}

#[test]
fn sem_nenhuma_conferencia_avisa_mas_conecta() {
    let mut c = Cenario::novo();
    c.sem_conferir();
    c.servidor_com("servidor-sem-eku.crt", "servidor-sem-eku.key");
    let par = conectar(c);
    assert!(
        par.avisos().iter().any(|a| a.contains("não confere")),
        "{:?}",
        par.avisos()
    );
}

#[test]
fn com_remote_cert_tls_nao_ha_aviso_de_falta_de_conferencia() {
    let par = conectar(Cenario::novo());
    assert!(par.avisos().iter().all(|a| !a.contains("não confere")));
}

#[test]
fn certificado_de_outra_ca_e_recusado() {
    let mut c = Cenario::novo();
    c.servidor_com("servidor-outra-ca.crt", "servidor-outra-ca.key");
    let par = falhar(c);
    assert!(
        matches!(
            par.falha(),
            Some(ControlError::Verify(VerifyError::UnknownIssuer { .. }))
        ),
        "{:?}",
        par.falha()
    );
    assert!(!par.falha().unwrap().to_string().is_empty());
}

fn com_nome(valor: &str, mode: NameMode) -> Cenario {
    let mut c = Cenario::novo();
    c.cliente.verify.x509_name = Some(X509Name {
        value: valor.to_owned(),
        mode,
    });
    c
}

#[test]
fn verify_x509_name_nos_tres_modos() {
    let assunto = referencias()["servidor.subject"].clone();
    conectar(com_nome(&assunto, NameMode::Subject));
    conectar(com_nome("servidor", NameMode::Name));
    conectar(com_nome("serv", NameMode::NamePrefix));
    for (valor, modo) in [
        ("C=BR, CN=servidor", NameMode::Subject),
        ("outro", NameMode::Name),
        ("xyz", NameMode::NamePrefix),
        // O modo `name` não aceita só um prefixo.
        ("serv", NameMode::Name),
    ] {
        let par = falhar(com_nome(valor, modo));
        assert!(
            matches!(
                par.falha(),
                Some(ControlError::Verify(VerifyError::NameMismatch { .. }))
            ),
            "{valor}: {:?}",
            par.falha()
        );
    }
}

#[test]
fn peer_fingerprint_dispensa_a_ca() {
    let mut c = Cenario::novo();
    c.cliente.pki.ca = Vec::new();
    c.cliente.verify = ServerVerify {
        peer_fingerprints: vec![sha256("servidor")],
        ..ServerVerify::default()
    };
    let par = conectar(c);
    assert!(par.avisos().iter().all(|a| !a.contains("não confere")));

    let mut c = Cenario::novo();
    c.cliente.pki.ca = Vec::new();
    c.cliente.verify = ServerVerify {
        peer_fingerprints: vec![sha256("servidor-ec")],
        ..ServerVerify::default()
    };
    let par = falhar(c);
    assert!(
        matches!(
            par.falha(),
            Some(ControlError::Verify(
                VerifyError::FingerprintMismatch { .. }
            ))
        ),
        "{:?}",
        par.falha()
    );
}

#[test]
fn sem_ca_e_sem_impressao_nao_ha_em_que_confiar() {
    let mut c = Cenario::novo();
    c.cliente.pki.ca = Vec::new();
    c.cliente.verify = ServerVerify::default();
    let erro = Control::new(c.cliente, Instant::now()).err();
    assert!(
        matches!(erro, Some(ControlError::Verify(VerifyError::NoTrust))),
        "{erro:?}"
    );
}

#[test]
fn verify_hash_aceita_sha1_e_sha256() {
    let r = referencias();
    for alvo in [&r["servidor.sha1"], &r["servidor.sha256"]] {
        let mut c = Cenario::novo();
        c.cliente.verify.verify_hash = vec![hex(alvo)];
        conectar(c);
    }
    let mut c = Cenario::novo();
    c.cliente.verify.verify_hash = vec![hex(&r["servidor-ec.sha1"])];
    let par = falhar(c);
    assert!(
        matches!(
            par.falha(),
            Some(ControlError::Verify(
                VerifyError::FingerprintMismatch { .. }
            ))
        ),
        "{:?}",
        par.falha()
    );
}

#[test]
fn certificado_vencido_e_ainda_nao_valido() {
    let mut c = Cenario::novo();
    c.wrap(Wrap::None);
    c.cliente.clock = Clock::fixed(UNIX_EPOCH + Duration::from_secs(2_200_000_000));
    let par = falhar(c);
    assert!(
        matches!(
            par.falha(),
            Some(ControlError::Verify(VerifyError::Expired { .. }))
        ),
        "{:?}",
        par.falha()
    );

    let mut c = Cenario::novo();
    c.wrap(Wrap::None);
    c.cliente.clock = Clock::fixed(UNIX_EPOCH + Duration::from_secs(1_600_000_000));
    let par = falhar(c);
    assert!(
        matches!(
            par.falha(),
            Some(ControlError::Verify(VerifyError::NotYetValid { .. }))
        ),
        "{:?}",
        par.falha()
    );
}

#[test]
fn assinatura_sha1_e_recusada_como_fraca() {
    let mut c = Cenario::novo();
    c.sem_conferir();
    c.cliente.pki.ca = certs("ca-sha1.crt");
    c.servidor_com("cliente-sha1.crt", "cliente-sha1.key");
    let par = falhar(c);
    assert!(
        matches!(
            par.falha(),
            Some(ControlError::Verify(VerifyError::Weak { .. }))
        ),
        "{:?}",
        par.falha()
    );
}

#[test]
fn o_servidor_recusa_certificado_de_cliente_vencido_ou_de_outra_ca() {
    for (cert, key) in [
        ("cliente-vencido.crt", "cliente-vencido.key"),
        ("cliente-outra-ca.crt", "cliente-outra-ca.key"),
    ] {
        for v in [TlsVersion::V1_2, TlsVersion::V1_3] {
            let mut c = Cenario::novo();
            c.versao(v);
            c.cliente_com(cert, key);
            let par = falhar(c);
            assert!(
                matches!(
                    par.falha(),
                    Some(ControlError::ClientCertificateRejected(_))
                ),
                "{cert} {v:?}: {:?}",
                par.falha()
            );
            assert!(
                par.falha()
                    .unwrap()
                    .to_string()
                    .contains("certificado do cliente")
            );
            assert!(
                par.eventos_s
                    .iter()
                    .any(|e| matches!(e, ServerEvent::Failed(_))),
                "o servidor devia registrar a falha"
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Senha, desafios, token e AUTH_PENDING.
// ---------------------------------------------------------------------------------------------

fn com_senha(usuario: &str, senha: &str) -> Cenario {
    let mut c = Cenario::novo();
    c.cliente.auth = AuthState::new(Credentials::new(usuario, senha));
    c
}

#[test]
fn senha_certa_e_senha_errada() {
    let regra = |a: &Attempt| {
        if a.username.as_deref() == Some("ana") && a.password.as_deref() == Some("segredo") {
            Decision::Accept { token: None }
        } else {
            Decision::Fail("AUTH_FAILED".to_owned())
        }
    };
    let par = conectar_com(com_senha("ana", "segredo"), regra);
    assert!(par.falha_auth().is_none());

    let mut par = Par::novo(com_senha("ana", "errada"), regra);
    par.rodar(50, |_| false);
    assert!(matches!(
        par.falha_auth(),
        Some(AuthFailure::Rejected { reason: None })
    ));
    assert!(par.cliente.is_finished());
    assert!(
        par.eventos_s
            .iter()
            .any(|e| matches!(e, ServerEvent::Rejected { .. }))
    );
    assert!(par.chaves_cliente(0).is_none());
}

#[test]
fn recusa_com_motivo_e_recusa_passageira() {
    let mut par = Par::novo(com_senha("ana", "x"), |_| {
        Decision::Fail("AUTH_FAILED,conta bloqueada".to_owned())
    });
    par.rodar(50, |_| false);
    match par.falha_auth() {
        Some(AuthFailure::Rejected { reason }) => {
            assert_eq!(reason.as_deref(), Some("conta bloqueada"));
        }
        outro => panic!("{outro:?}"),
    }

    let mut par = Par::novo(com_senha("ana", "x"), |_| {
        Decision::Fail("AUTH_FAILED,TEMP[backoff 7,advance no]:tente depois".to_owned())
    });
    par.rodar(50, |_| false);
    match par.falha_auth() {
        Some(AuthFailure::Temporary {
            reason, backoff, ..
        }) => {
            assert_eq!(reason.as_deref(), Some("tente depois"));
            assert_eq!(*backoff, Some(Duration::from_secs(7)));
        }
        outro => panic!("{outro:?}"),
    }
}

#[test]
fn desafio_estatico_scrv1() {
    let c = Credentials::new("ana", "segredo:com:dois-pontos");
    let mut cen = Cenario::novo();
    cen.cliente.auth = AuthState::with_static_challenge(&c, "123456");
    let par = conectar_com(cen, |a| {
        let (senha, resposta) = a
            .password
            .as_deref()
            .and_then(parse_static_challenge_password)
            .expect("SCRV1");
        if a.username.as_deref() == Some("ana")
            && *senha == "segredo:com:dois-pontos"
            && *resposta == "123456"
        {
            Decision::Accept { token: None }
        } else {
            Decision::Fail("AUTH_FAILED".to_owned())
        }
    });
    assert!(par.falha_auth().is_none());
}

#[test]
fn desafio_dinamico_crv1_e_a_resposta() {
    use base64::Engine as _;
    let usuario_b64 = base64::engine::general_purpose::STANDARD.encode("ana");
    let pergunta = format!("AUTH_FAILED,CRV1:R,E:estado-123:{usuario_b64}:Digite o código");
    let regra = move |a: &Attempt| match a.password.as_deref() {
        Some(p) => match parse_dynamic_challenge_password(p) {
            Some((estado, resposta)) if estado == "estado-123" && *resposta == "987654" => {
                Decision::Accept { token: None }
            }
            Some(_) => Decision::Fail("AUTH_FAILED".to_owned()),
            None => Decision::Fail(pergunta.clone()),
        },
        None => Decision::Fail("AUTH_FAILED".to_owned()),
    };

    let mut par = Par::novo(com_senha("ana", "segredo"), regra.clone());
    par.rodar(50, |_| false);
    let desafio = match par.falha_auth() {
        Some(AuthFailure::Challenge(d)) => d.clone(),
        outro => panic!("{outro:?}"),
    };
    assert!(desafio.echo && desafio.response_required);
    assert_eq!(desafio.state_id, "estado-123");
    assert_eq!(desafio.user, "ana");
    assert_eq!(desafio.text, "Digite o código");

    // A resposta vai numa nova conexão.
    let mut cen = Cenario::novo();
    cen.cliente.auth = AuthState::new(desafio.answer("987654"));
    conectar_com(cen, regra);
}

#[test]
fn auth_token_nas_renegociacoes_e_o_fim_dele() {
    let tentativas: Rc<RefCell<Vec<Attempt>>> = Rc::default();
    let dentro = Rc::clone(&tentativas);
    let regra = move |a: &Attempt| {
        dentro.borrow_mut().push(a.clone());
        if a.initial {
            Decision::Accept {
                token: Some("tok-1".to_owned()),
            }
        } else if a.password.as_deref() == Some("tok-1") {
            Decision::Accept { token: None }
        } else {
            Decision::Fail("AUTH_FAILED".to_owned())
        }
    };
    let mut par = conectar_com(com_senha("ana", "segredo"), regra.clone());
    assert!(par.cliente.auth_state().has_token());
    assert_eq!(
        tentativas.borrow()[0].password.as_deref(),
        Some("segredo"),
        "a primeira vez vai a senha"
    );

    par.cliente.renegotiate(par.t);
    par.rodar(50, pronto(1));
    par.confere_chaves(1);
    {
        let t = tentativas.borrow();
        assert_eq!(t.len(), 2);
        assert!(!t[1].initial);
        assert_eq!(t[1].key_id, 1);
        assert_eq!(t[1].password.as_deref(), Some("tok-1"));
        assert_eq!(t[1].username.as_deref(), Some("ana"));
    }

    // O servidor desfaz o token: a sessão acaba, o token some e a senha volta a valer.
    par.servidor
        .send_message("AUTH_FAILED,SESSION:o token venceu");
    par.rodar(10, |p| p.cliente.is_finished());
    assert!(matches!(
        par.falha_auth(),
        Some(AuthFailure::Session { reason: Some(r) }) if r == "o token venceu"
    ));
    assert!(!par.cliente.auth_state().has_token());

    let mut cen = Cenario::novo();
    cen.cliente.auth = par.cliente.auth_state().clone();
    let nova = conectar_com(cen, regra);
    assert_eq!(
        tentativas.borrow().last().unwrap().password.as_deref(),
        Some("segredo")
    );
    assert!(nova.cliente.auth_state().has_token());
}

#[test]
fn auth_pending_espera_alem_do_hand_window_e_depois_aceita() {
    let mut par = Par::novo(com_senha("ana", "segredo"), |_| Decision::Pending {
        timeout: 200,
    });
    // O hand-window é de 60 s; com o AUTH_PENDING, o prazo passa a ser o do servidor.
    par.rodar(100, |_| false);
    assert!(par.falha().is_none(), "{:?}", par.falha());
    assert!(par.eventos.iter().any(|e| matches!(
        e,
        Event::AuthPending {
            timeout: Some(t)
        } if *t == Duration::from_secs(200)
    )));
    assert!(par.chaves_cliente(0).is_none());

    par.servidor
        .resolve_pending(Decision::Accept { token: None });
    par.rodar(50, pronto(0));
    assert!(par.falha().is_none(), "{:?}", par.falha());
    par.confere_chaves(0);
}

#[test]
fn auth_pending_que_ninguem_resolve_estoura_no_prazo_do_servidor() {
    let mut par = Par::novo(com_senha("ana", "segredo"), |_| Decision::Pending {
        timeout: 20,
    });
    par.rodar(200, |_| false);
    assert!(
        matches!(
            par.falha(),
            Some(ControlError::HandshakeTimeout {
                heard_server: true,
                ..
            })
        ),
        "{:?}",
        par.falha()
    );
    let decorrido = par.t - par.t0;
    assert!(decorrido >= Duration::from_secs(20), "{decorrido:?}");
    assert!(decorrido < Duration::from_secs(60), "{decorrido:?}");
}

// ---------------------------------------------------------------------------------------------
// O PUSH_REPLY.
// ---------------------------------------------------------------------------------------------

#[test]
fn sem_empurrar_sozinho_o_cliente_pede_com_push_request() {
    let mut c = Cenario::novo();
    c.servidor.auto_push = false;
    let par = conectar(c);
    // O servidor só empurra depois do pedido; o pedido sai uma vez, depois de 1 s.
    let p = par.pushed().expect("configuração");
    assert_eq!(p.ifconfig.as_ref().unwrap().address.to_string(), "10.8.0.2");
    assert!(par.t - par.t0 >= Duration::from_secs(1));
}

#[test]
fn push_reply_em_duas_mensagens() {
    let mut c = Cenario::novo();
    c.servidor.push = vec![
        "PUSH_REPLY,route 10.9.0.0 255.255.255.0,ifconfig 10.8.0.2 255.255.255.0,topology subnet,push-continuation 2".to_owned(),
        "PUSH_REPLY,route 10.10.0.0 255.255.255.0,cipher AES-256-GCM,push-continuation 1".to_owned(),
    ];
    let par = conectar(c);
    let p = par.pushed().expect("configuração");
    assert_eq!(p.routes.len(), 2, "{:?}", p.routes);
    assert_eq!(p.routes[1].network.to_string(), "10.10.0.0");
}

#[test]
fn pull_filter_ignore_tira_uma_rota() {
    let (config, _) = Config::from_ovpn(
        &format!(
            "{}\npull-filter ignore \"route 10.9.\"\n",
            ler("tudo-embutido.ovpn")
        ),
        Some(&pasta()),
    )
    .expect("perfil");
    let mut c = Cenario::novo();
    c.cliente.pull_filters = config.pull_filters;
    let par = conectar(c);
    assert!(par.pushed().expect("configuração").routes.is_empty());
}

// ---------------------------------------------------------------------------------------------
// Tempo, perda e robustez.
// ---------------------------------------------------------------------------------------------

#[test]
fn servidor_mudo_estoura_o_hand_window_e_a_mensagem_orienta() {
    let t0 = Instant::now();
    let c = Cenario::novo();
    let mut cliente = Control::new(c.cliente, t0).expect("cliente");
    let mut t = t0;
    let mut enviados = 0;
    let mut erro = None;
    for _ in 0..1000 {
        while cliente.poll_transmit(t).is_some() {
            enviados += 1;
        }
        while let Some(e) = cliente.poll_event() {
            if let Event::Failed(f) = e {
                erro = Some(f);
            }
        }
        if erro.is_some() {
            break;
        }
        t = cliente.next_timeout().expect("um prazo").max(t);
        cliente.handle_timeout(t);
    }
    let erro = erro.expect("devia estourar");
    assert_eq!(
        erro,
        ControlError::HandshakeTimeout {
            seconds: 60,
            heard_server: false
        }
    );
    assert!(erro.to_string().contains("tls-auth"));
    assert!(t - t0 >= Duration::from_secs(60));
    assert!(enviados > 1, "o reset devia ser reenviado");
    assert!(cliente.is_finished());
}

#[test]
fn chave_de_tls_auth_errada_o_servidor_descarta_tudo() {
    let mut c = Cenario::novo();
    c.servidor.wrap.static_key = Some(chave_estatica("tc.key"));
    let mut par = Par::novo(c, aceita);
    par.rodar(90, |_| false);
    assert!(par.servidor.dropped_datagrams() > 0);
    assert!(matches!(
        par.falha(),
        Some(ControlError::HandshakeTimeout {
            heard_server: false,
            ..
        })
    ));
}

#[test]
fn perdendo_um_terco_dos_pacotes_o_udp_se_recupera() {
    for wrap in [Wrap::None, Wrap::TlsAuth, Wrap::TlsCrypt] {
        let mut c = Cenario::novo();
        c.wrap(wrap);
        let mut par = Par::novo(c, aceita);
        par.perda = Some(Box::new(|_, n| n % 3 == 0));
        par.rodar(55, pronto(0));
        assert!(par.falha().is_none(), "{:?}", par.falha());
        par.confere_chaves(0);
        assert!(
            par.t - par.t0 >= Duration::from_secs(2),
            "devia ter reenviado"
        );
    }
}

#[test]
fn mensagens_do_servidor_em_andamento() {
    let mut par = conectar(Cenario::novo());
    par.servidor.send_message("INFO,bem-vindo");
    par.servidor
        .send_message("INFO_PRE,OPEN_URL:https://login.exemplo.test/x");
    par.rodar(5, |_| false);
    assert!(
        par.eventos
            .iter()
            .any(|e| matches!(e, Event::Info(i) if i.contains("bem-vindo")))
    );
    assert!(par.eventos.iter().any(|e| matches!(
        e,
        Event::OpenUrl(u) if u == "https://login.exemplo.test/x"
    )));
    assert!(
        !par.cliente.is_finished(),
        "info e página não acabam a sessão"
    );

    par.servidor.send_message("RESTART,trocou o servidor");
    par.rodar(5, |_| false);
    assert!(
        par.eventos
            .iter()
            .any(|e| matches!(e, Event::Restart(t) if t.contains("trocou")))
    );
    assert!(par.cliente.is_finished());
}

#[test]
fn halt_acaba_a_sessao() {
    let mut par = conectar(Cenario::novo());
    par.servidor.send_message("HALT,fim");
    par.rodar(5, |_| false);
    assert!(par.eventos.iter().any(|e| matches!(e, Event::Halt(_))));
    assert!(par.cliente.is_finished());
}

#[test]
fn o_cliente_manda_mensagens_pelo_canal_de_controle() {
    let mut par = conectar(Cenario::novo());
    par.cliente.send_message("EXIT");
    par.rodar(5, |p| !p.mensagens_do_servidor().is_empty());
    assert_eq!(par.mensagens_do_servidor(), ["EXIT"]);
}

#[test]
fn pacote_de_dados_vai_para_o_motor_de_dados() {
    let mut par = conectar(Cenario::novo());
    let mut dados = vec![(9 << 3)];
    dados.extend_from_slice(&[0, 0, 0, 5, 1, 2, 3, 4]);
    assert_eq!(
        par.cliente.handle_datagram(&dados, par.t),
        Datagram::Data(dados.clone())
    );
    assert_eq!(
        par.servidor.handle_datagram(&dados, par.t),
        Datagram::Data(dados)
    );
    let mut antigo = vec![(6 << 3)];
    antigo.extend_from_slice(&[1, 2, 3]);
    assert_eq!(
        par.cliente.handle_datagram(&antigo, par.t),
        Datagram::Data(antigo)
    );
}

#[test]
fn datagrama_repetido_e_descartado() {
    let mut par = conectar(Cenario::novo());
    let (_, repetido) = par
        .capturados
        .iter()
        .rev()
        .find(|(do_cliente, _)| *do_cliente)
        .cloned()
        .expect("pacote do cliente");
    let antes = par.servidor.dropped_datagrams();
    assert_eq!(
        par.servidor.handle_datagram(&repetido, par.t),
        Datagram::Dropped
    );
    assert_eq!(par.servidor.dropped_datagrams(), antes + 1);
}

#[test]
fn lixo_nao_derruba_nem_atrapalha() {
    let mut par = Par::novo(Cenario::novo(), aceita);
    let mut lixos: Vec<Vec<u8>> = vec![
        vec![],
        vec![0],
        vec![0xff; 3],
        vec![7 << 3],
        vec![(8 << 3) | 7; 40],
        vec![(4 << 3) | 2; 100],
        vec![5 << 3; 20],
        (0..=255).collect(),
    ];
    // Pedaços cortados de pacotes de verdade.
    par.drenar();
    for (_, p) in par.capturados.clone() {
        for corte in [1, 5, 9, p.len() / 2, p.len().saturating_sub(1)] {
            lixos.push(p[..corte.min(p.len())].to_vec());
        }
    }
    for l in &lixos {
        par.cliente.handle_datagram(l, par.t);
        par.servidor.handle_datagram(l, par.t);
    }
    par.rodar(50, pronto(0));
    assert!(par.falha().is_none(), "{:?}", par.falha());
    par.confere_chaves(0);
}

// ---------------------------------------------------------------------------------------------
// As opções do perfil.
// ---------------------------------------------------------------------------------------------

#[test]
fn perfil_que_pede_senha_exige_a_senha() {
    let texto = format!("{}\nauth-user-pass\n", ler("tudo-embutido.ovpn"));
    let (config, _) = Config::from_ovpn(&texto, Some(&pasta())).expect("perfil");
    let erro = ControlOptions::from_config(
        &config,
        Proto::Udp,
        false,
        AuthState::none(),
        Clock::system(),
    )
    .err();
    assert!(matches!(erro, Some(ControlError::Setup(_))), "{erro:?}");
    assert!(
        ControlOptions::from_config(
            &config,
            Proto::Udp,
            false,
            AuthState::new(Credentials::new("a", "b")),
            Clock::system(),
        )
        .is_ok()
    );
}

#[test]
fn as_sessoes_tem_ids_diferentes_e_o_cliente_conhece_a_do_servidor() {
    let par = conectar(Cenario::novo());
    assert_ne!(
        Some(par.cliente.local_session_id()),
        par.cliente.remote_session_id()
    );
    assert!(par.cliente.remote_session_id().is_some());
}
