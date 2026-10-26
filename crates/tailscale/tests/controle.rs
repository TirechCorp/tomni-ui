//! O cliente do plano de controle contra o `ControlFake`: `/key`, o handshake, o HTTP/2 e o
//! registro (com chave de autenticação e com URL de login).

use std::sync::Arc;
use std::time::Duration;

use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;
use tomni_ui_tailscale::control::noise::NoiseError;
use tomni_ui_tailscale::control::{
    Backoff, Control, ControlConfig, ControlError, RegisterOutcome, RegisterParams,
};
use tomni_ui_tailscale::key::{MachineKey, NodeKey};
use tomni_ui_tailscale::tailcfg::{CAPABILITY_VERSION, Hostinfo};
use tomni_ui_tailscale::testing::{ControlFake, FakeOptions};
use tomni_ui_tailscale::upgrade::Roots;

fn hostinfo() -> Hostinfo {
    Hostinfo {
        ipn_version: "tirech-omni/0.1".to_string(),
        os: "linux".to_string(),
        hostname: "maquina-de-teste".to_string(),
        shields_up: true,
        ..Default::default()
    }
}

fn parametros(chave: Option<&str>) -> RegisterParams {
    RegisterParams {
        node_key: NodeKey::generate().public(),
        old_node_key: None,
        auth_key: chave.map(|c| zeroize::Zeroizing::new(c.to_string())),
        hostinfo: hostinfo(),
        ephemeral: false,
    }
}

fn cliente(url: &str) -> Control {
    let mut config = ControlConfig::new(url);
    config.backoff = Backoff::new(Duration::from_millis(50), Duration::from_millis(200));
    Control::new(config, MachineKey::generate()).unwrap()
}

#[tokio::test]
async fn busca_a_chave_do_servidor() {
    let fake = ControlFake::start().await;
    let mut c = cliente(&fake.url());
    assert_eq!(c.server_key().await.unwrap(), fake.server_key());
}

#[tokio::test]
async fn registra_com_chave_de_autenticacao() {
    let fake = ControlFake::start().await;
    let mut c = cliente(&fake.url());
    let p = parametros(Some("tskey-auth-valida"));
    let r = c.register(&p, None).await.unwrap();
    let RegisterOutcome::Authorized(resp) = r else {
        panic!("esperava Authorized, veio {r:?}");
    };
    assert!(resp.machine_authorized);
    assert_eq!(resp.user.display_name, "Teste");

    let pedidos = fake.requests();
    assert_eq!(pedidos.len(), 1);
    let ped = &pedidos[0];
    assert_eq!(ped.handshake_version as u32, CAPABILITY_VERSION);
    assert_eq!(ped.request.version, CAPABILITY_VERSION);
    assert_eq!(ped.request.node_key, p.node_key);
    assert_eq!(
        ped.request.auth.as_ref().unwrap().auth_key,
        "tskey-auth-valida"
    );
    assert!(ped.request.old_node_key.is_zero());
}

#[tokio::test]
async fn o_pedido_declara_escudo_ligado_e_nenhuma_sub_rede() {
    let fake = ControlFake::start().await;
    let mut c = cliente(&fake.url());
    c.register(&parametros(Some("tskey-auth-valida")), None)
        .await
        .unwrap();
    let ped = &fake.requests()[0];
    let host = &ped.raw["Hostinfo"];
    assert_eq!(host["ShieldsUp"], true);
    assert!(host.get("RoutableIPs").is_none(), "{host}");
    assert_eq!(host["IPNVersion"], "tirech-omni/0.1");
    assert_eq!(host["Hostname"], "maquina-de-teste");
    assert!(
        ped.request
            .hostinfo
            .as_ref()
            .unwrap()
            .routable_ips
            .is_empty()
    );
}

#[tokio::test]
async fn chave_de_autenticacao_errada_e_recusada_com_o_motivo() {
    let fake = ControlFake::start().await;
    let mut c = cliente(&fake.url());
    let e = c
        .register(&parametros(Some("tskey-auth-errada")), None)
        .await
        .unwrap_err();
    assert!(matches!(e, ControlError::AuthKeyRejected(_)), "{e:?}");
    let texto = e.to_string();
    assert!(
        texto.starts_with("a chave de autenticação foi recusada: invalid key"),
        "{texto}"
    );
    assert!(!e.is_transient());
}

#[tokio::test]
async fn sem_chave_o_servidor_devolve_a_url_de_login() {
    let fake = ControlFake::start().await;
    let mut c = cliente(&fake.url());
    let r = c.register(&parametros(None), None).await.unwrap();
    let RegisterOutcome::NeedsLogin { url } = r else {
        panic!("esperava NeedsLogin, veio {r:?}");
    };
    assert_eq!(fake.login_urls(), [url]);
}

#[tokio::test]
async fn o_login_pelo_navegador_termina_quando_o_usuario_aprova() {
    let fake = Arc::new(ControlFake::start().await);
    let mut c = cliente(&fake.url());
    let p = parametros(None);
    let f2 = fake.clone();
    let mut urls = Vec::new();
    let mut ao_pedir = |url: &str| {
        urls.push(url.to_string());
        // O "navegador": o usuário entra um instante depois.
        let f3 = f2.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            f3.approve();
        });
    };
    let fluxo = c.register_flow(&p, &mut ao_pedir);
    let r = tokio::time::timeout(Duration::from_secs(20), fluxo)
        .await
        .expect("o fluxo terminou")
        .unwrap();
    assert!(matches!(r, RegisterOutcome::Authorized(_)), "{r:?}");
    assert_eq!(urls.len(), 1);
    assert!(urls[0].contains("/a/"), "{urls:?}");
    let pedidos = fake.requests();
    assert_eq!(pedidos.len(), 2);
    assert_eq!(pedidos[0].request.followup, "");
    assert_eq!(pedidos[1].request.followup, urls[0]);
}

#[tokio::test]
async fn o_pedido_de_acompanhamento_repete_quando_o_tempo_acaba() {
    let fake = Arc::new(
        ControlFake::with_options(FakeOptions {
            followup_wait: Duration::from_millis(100),
            ..Default::default()
        })
        .await,
    );
    let mut c = cliente(&fake.url());
    let p = parametros(None);
    let f2 = fake.clone();
    let mut ao_pedir = |_: &str| {
        let f3 = f2.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(700)).await;
            f3.approve();
        });
    };
    let fluxo = c.register_flow(&p, &mut ao_pedir);
    let r = tokio::time::timeout(Duration::from_secs(20), fluxo)
        .await
        .expect("o fluxo terminou")
        .unwrap();
    assert!(matches!(r, RegisterOutcome::Authorized(_)), "{r:?}");
    let seguimentos = fake
        .requests()
        .iter()
        .filter(|r| !r.request.followup.is_empty())
        .count();
    assert!(
        seguimentos >= 2,
        "só {seguimentos} pedidos de acompanhamento"
    );
}

#[tokio::test]
async fn depois_do_login_o_administrador_ainda_precisa_aprovar() {
    let fake = Arc::new(
        ControlFake::with_options(FakeOptions {
            admin_approval: true,
            ..Default::default()
        })
        .await,
    );
    let mut c = cliente(&fake.url());
    let p = parametros(None);
    let f2 = fake.clone();
    let mut ao_pedir = |_: &str| f2.approve();
    let fluxo = c.register_flow(&p, &mut ao_pedir);
    let r = tokio::time::timeout(Duration::from_secs(20), fluxo)
        .await
        .expect("o fluxo terminou")
        .unwrap();
    let RegisterOutcome::NeedsApproval(resp) = r else {
        panic!("esperava NeedsApproval, veio {r:?}");
    };
    assert!(!resp.machine_authorized);
}

#[tokio::test]
async fn a_chave_do_no_vencida_e_avisada() {
    let fake = ControlFake::with_options(FakeOptions {
        expire_node_key: true,
        ..Default::default()
    })
    .await;
    let mut c = cliente(&fake.url());
    let r = c.register(&parametros(None), None).await.unwrap();
    assert_eq!(r, RegisterOutcome::KeyExpired);
}

#[tokio::test]
async fn tailnet_lock_nao_e_suportado() {
    let fake = ControlFake::with_options(FakeOptions {
        tailnet_lock: true,
        ..Default::default()
    })
    .await;
    let mut c = cliente(&fake.url());
    let e = c.register(&parametros(None), None).await.unwrap_err();
    assert_eq!(
        e.to_string(),
        "esta tailnet usa o Tailnet Lock, que o Tirech Omni ainda não suporta"
    );
}

#[tokio::test]
async fn um_erro_http_do_registro_traz_o_codigo_e_o_texto() {
    let fake = ControlFake::with_options(FakeOptions {
        http_error: Some((403, "acesso negado".to_string())),
        ..Default::default()
    })
    .await;
    let mut c = cliente(&fake.url());
    let e = c.register(&parametros(None), None).await.unwrap_err();
    assert!(
        matches!(&e, ControlError::Http { status: 403, body } if body == "acesso negado"),
        "{e:?}"
    );
    assert_eq!(
        e.to_string(),
        "o servidor de controle respondeu HTTP 403: acesso negado"
    );
    assert!(!e.is_transient());
}

#[tokio::test]
async fn muitos_pedidos_viram_espera() {
    let fake = ControlFake::with_options(FakeOptions {
        http_error: Some((429, "devagar".to_string())),
        ..Default::default()
    })
    .await;
    let mut c = cliente(&fake.url());
    let e = c.register(&parametros(None), None).await.unwrap_err();
    assert!(matches!(e, ControlError::RateLimited { .. }), "{e:?}");
    assert!(e.is_transient());
}

#[tokio::test]
async fn um_servidor_que_nao_e_do_tailscale_e_reconhecido() {
    let fake = ControlFake::with_options(FakeOptions {
        not_tailscale: true,
        ..Default::default()
    })
    .await;
    let mut c = cliente(&fake.url());
    let e = c.server_key().await.unwrap_err();
    assert_eq!(
        e.to_string(),
        format!(
            "o servidor {} não fala o protocolo do Tailscale (/key respondeu 404)",
            fake.url()
        )
    );
}

#[tokio::test]
async fn um_servidor_fora_do_ar_da_erro_de_rede_que_se_repete() {
    // Reserva uma porta e a solta: ninguém escuta nela.
    let porta = {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        l.local_addr().unwrap().port()
    };
    let url = format!("http://127.0.0.1:{porta}");
    let mut c = cliente(&url);
    let e = c.server_key().await.unwrap_err();
    assert!(
        e.to_string().starts_with(&format!(
            "não foi possível falar com o servidor de controle {url}: "
        )),
        "{e}"
    );
    assert!(e.is_transient());
}

#[tokio::test]
async fn a_versao_recusada_pede_para_atualizar() {
    let fake = ControlFake::with_options(FakeOptions {
        min_version: 9999,
        ..Default::default()
    })
    .await;
    let mut c = cliente(&fake.url());
    let e = c.register(&parametros(None), None).await.unwrap_err();
    assert!(matches!(e, ControlError::VersionRejected { .. }), "{e:?}");
    assert_eq!(
        e.to_string(),
        "o servidor recusou esta versão do cliente (capacidade 113); atualize o Tirech Omni"
    );
    assert_eq!(fake.refused_handshakes(), 1);
}

#[tokio::test]
async fn o_handshake_recusado_mostra_o_texto_do_servidor() {
    let fake = ControlFake::with_options(FakeOptions {
        refuse_handshake: Some("machine key not authorized".to_string()),
        ..Default::default()
    })
    .await;
    let mut c = cliente(&fake.url());
    let e = c.register(&parametros(None), None).await.unwrap_err();
    assert!(
        matches!(&e, ControlError::Noise(NoiseError::Server(t)) if t == "machine key not authorized"),
        "{e:?}"
    );
    assert!(!e.is_transient());
}

#[tokio::test]
async fn a_carga_antecipada_aparece_quando_o_servidor_manda() {
    let fake = ControlFake::start().await;
    let mut c = cliente(&fake.url());
    c.register(&parametros(Some("tskey-auth-valida")), None)
        .await
        .unwrap();
    let early = c.connection().await.unwrap().early().get();
    let early = early.expect("a carga antecipada chegou");
    assert_eq!(
        early.node_key_challenge.to_string(),
        format!("chalpub:{}", "07".repeat(32))
    );
}

#[tokio::test]
async fn sem_carga_antecipada_o_registro_funciona_igual() {
    let fake = ControlFake::with_options(FakeOptions {
        early_payload: false,
        ..Default::default()
    })
    .await;
    let mut c = cliente(&fake.url());
    let r = c
        .register(&parametros(Some("tskey-auth-valida")), None)
        .await
        .unwrap();
    assert!(matches!(r, RegisterOutcome::Authorized(_)));
    assert!(c.connection().await.unwrap().early().get().is_none());
}

#[tokio::test]
async fn a_mesma_conexao_serve_varios_pedidos_e_reabre_depois_de_largada() {
    let fake = ControlFake::start().await;
    let mut c = cliente(&fake.url());
    for _ in 0..3 {
        c.register(&parametros(Some("tskey-auth-valida")), None)
            .await
            .unwrap();
    }
    assert_eq!(fake.connections(), 1);
    c.disconnect();
    c.register(&parametros(Some("tskey-auth-valida")), None)
        .await
        .unwrap();
    assert_eq!(fake.connections(), 2);
    assert_eq!(fake.requests().len(), 4);
}

#[tokio::test]
async fn varios_registros_em_sequencia_na_mesma_conexao() {
    let fake = ControlFake::start().await;
    let mut c = cliente(&fake.url());
    c.connect().await.unwrap();
    // Confere que o servidor guarda todos, na ordem.
    for i in 0..10 {
        let mut p = parametros(Some("tskey-auth-valida"));
        p.hostinfo.hostname = format!("m{i}");
        c.register(&p, None).await.unwrap();
    }
    let nomes: Vec<String> = fake
        .requests()
        .iter()
        .map(|r| r.request.hostinfo.as_ref().unwrap().hostname.clone())
        .collect();
    assert_eq!(nomes.len(), 10);
    assert_eq!(nomes[9], "m9");
}

// ---------------------------------------------------------------------------------------------
// TLS: um servidor HTTPS na frente do falso, com uma CA própria.
// ---------------------------------------------------------------------------------------------

/// Sobe um terminador TLS que repassa tudo para o `ControlFake` em HTTP puro. Devolve a URL
/// `https://` e o PEM da CA que assinou o certificado.
async fn terminador_tls(destino: String) -> (String, String) {
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    use rustls::{ServerConfig, version};

    let mut ca_params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    ca_params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "CA do controle de teste");
    let ca_chave = rcgen::KeyPair::generate().unwrap();
    let ca = ca_params.self_signed(&ca_chave).unwrap();
    let emissor = rcgen::Issuer::new(ca_params, ca_chave);
    let folha_chave = rcgen::KeyPair::generate().unwrap();
    let folha = rcgen::CertificateParams::new(vec!["localhost".to_owned()])
        .unwrap()
        .signed_by(&folha_chave, &emissor)
        .unwrap();
    let cadeia: Vec<CertificateDer<'static>> = vec![folha.der().clone()];
    let chave = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(folha_chave.serialize_der()));
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&version::TLS13, &version::TLS12])
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(cadeia, chave)
        .unwrap();
    let aceitador = TlsAcceptor::from(Arc::new(config));
    let escuta = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let porta = escuta.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = escuta.accept().await else {
                break;
            };
            let aceitador = aceitador.clone();
            let destino = destino.clone();
            tokio::spawn(async move {
                let Ok(mut tls) = aceitador.accept(tcp).await else {
                    return;
                };
                let Ok(mut atras) = TcpStream::connect(&destino).await else {
                    return;
                };
                let _ = tokio::io::copy_bidirectional(&mut tls, &mut atras).await;
            });
        }
    });
    (format!("https://localhost:{porta}"), ca.pem())
}

#[tokio::test]
async fn registra_por_https_com_a_ca_do_perfil() {
    let fake = ControlFake::start().await;
    let destino = fake.url().trim_start_matches("http://").to_string();
    let (url, pem) = terminador_tls(destino).await;
    let mut config = ControlConfig::new(&url);
    config.roots = Roots::system().with_pem(&pem).unwrap();
    let mut c = Control::new(config, MachineKey::generate()).unwrap();
    let r = c
        .register(&parametros(Some("tskey-auth-valida")), None)
        .await
        .unwrap();
    assert!(matches!(r, RegisterOutcome::Authorized(_)));
}

#[tokio::test]
async fn por_https_sem_a_ca_o_certificado_nao_e_confiavel() {
    let fake = ControlFake::start().await;
    let destino = fake.url().trim_start_matches("http://").to_string();
    let (url, _pem) = terminador_tls(destino).await;
    let mut c = cliente(&url);
    let e = c.server_key().await.unwrap_err();
    let texto = e.to_string();
    assert!(
        texto.contains("o certificado de localhost não é confiável"),
        "{texto}"
    );
    assert!(texto.contains("CA própria"), "{texto}");
    assert!(!e.is_transient());
}
