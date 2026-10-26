//! O `.ovpn`: o que entra, o que avisa, o que recusa e o que `without_secrets` tira.

mod comum;

use comum::{agora, ler, pasta};
use tomni_ui_openvpn::config::{
    AllowCompression, AuthUserPass, Config, ConfigErrorKind, Family, FilterAction, NameMode, Proto,
    TlsVersion, Warning,
};
use tomni_ui_openvpn::pki::{Severity, Wrap};

fn abrir(nome: &str) -> (Config, Vec<Warning>) {
    Config::from_ovpn(&ler(nome), Some(&pasta())).unwrap_or_else(|e| panic!("{nome}: {e}"))
}

fn erro(texto: &str) -> tomni_ui_openvpn::config::ConfigError {
    Config::from_ovpn(texto, Some(&pasta())).expect_err("devia recusar")
}

fn avisos(texto: &str) -> Vec<String> {
    let (_, w) = Config::from_ovpn(texto, None).expect("devia ler");
    w.into_iter().map(|w| w.message).collect()
}

#[test]
fn perfil_com_tudo_embutido_le_os_campos() {
    let (c, w) = abrir("tudo-embutido.ovpn");
    assert!(w.is_empty(), "avisos: {w:?}");
    assert!(c.is_client);
    assert_eq!(c.remotes.len(), 1);
    assert_eq!(c.remotes[0].host, "vpn.exemplo.test");
    assert_eq!(c.remotes[0].port, 1194);
    assert_eq!(c.remotes[0].proto, Proto::Udp);
    assert!(c.verify.remote_cert_tls);
    assert_eq!(c.cipher.as_deref(), Some("AES-256-GCM"));
    assert_eq!(c.data_ciphers.as_ref().map(Vec::len), Some(3));
    assert_eq!(c.pki.ca.len(), 1);
    assert_eq!(c.pki.cert.as_ref().map(Vec::len), Some(1));
    assert!(c.pki.key.is_some());
    assert_eq!(c.pki.wrap, Wrap::TlsAuth);
    assert_eq!(c.pki.key_direction, Some(1));
    assert!(c.pki.static_key.is_some());
}

#[test]
fn perfil_com_tudo_embutido_passa_na_conferencia() {
    let (c, _) = abrir("tudo-embutido.ovpn");
    let problemas = c.check(agora());
    assert!(problemas.is_empty(), "problemas: {problemas:?}");
}

#[test]
fn perfil_com_arquivos_le_da_pasta() {
    let (c, w) = abrir("com-arquivos.ovpn");
    assert!(w.is_empty(), "avisos: {w:?}");
    assert_eq!(c.pki.wrap, Wrap::TlsCrypt);
    assert_eq!(c.pki.ca.len(), 1);
    assert!(c.pki.key.is_some());
}

#[test]
fn arquivo_ausente_diz_o_nome_e_a_opcao() {
    let e = erro("client\nremote a.test\nca nao-existe.crt\n");
    assert_eq!(e.line, Some(3));
    assert!(
        matches!(&e.kind, ConfigErrorKind::MissingFile { file, option }
        if file == "nao-existe.crt" && option == "ca")
    );
    assert_eq!(
        e.to_string(),
        "linha 3: falta o arquivo nao-existe.crt, citado em ca"
    );
}

#[test]
fn sem_pasta_so_valem_os_blocos() {
    let e = Config::from_ovpn("client\nca ca.crt\n", None).expect_err("sem pasta");
    assert!(matches!(e.kind, ConfigErrorKind::MissingFile { .. }));
}

#[test]
fn arquivo_fora_da_pasta_do_perfil_e_recusado() {
    for caminho in ["/etc/passwd", "../fixtures/ca.crt", "sub/../../ca.crt"] {
        let e = erro(&format!("client\nauth-user-pass {caminho}\n"));
        assert!(
            matches!(e.kind, ConfigErrorKind::OutsideFolder { .. }),
            "{caminho}: {e}"
        );
        assert!(e.to_string().contains("fora da pasta do perfil"));
    }
}

#[test]
fn inline_aponta_para_o_bloco_de_mesmo_nome() {
    let ca = ler("ca.crt");
    let texto = format!("client\nremote a.test\nca [inline]\n<ca>\n{ca}</ca>\n");
    let (c, _) = Config::from_ovpn(&texto, None).expect("lê");
    assert_eq!(c.pki.ca.len(), 1);
}

#[test]
fn inline_sem_bloco_e_erro() {
    let e = Config::from_ovpn("client\nca [inline]\n", None).expect_err("sem bloco");
    assert!(matches!(e.kind, ConfigErrorKind::MissingFile { .. }));
}

#[test]
fn blocos_connection_acumulam_e_herdam_porta_e_protocolo() {
    let (c, w) = abrir("connection-blocks.ovpn");
    assert!(c.remote_random);
    assert_eq!(c.remotes.len(), 3);
    assert_eq!(
        (
            c.remotes[0].host.as_str(),
            c.remotes[0].port,
            c.remotes[0].proto
        ),
        ("a.exemplo.test", 1194, Proto::Udp)
    );
    assert_eq!(
        (
            c.remotes[1].host.as_str(),
            c.remotes[1].port,
            c.remotes[1].proto
        ),
        ("b.exemplo.test", 443, Proto::Tcp)
    );
    assert_eq!(
        (
            c.remotes[2].host.as_str(),
            c.remotes[2].port,
            c.remotes[2].proto
        ),
        ("192.0.2.10", 4443, Proto::Tcp)
    );
    assert!(
        w.iter().any(|w| w.option == "http-proxy-option"),
        "avisos: {w:?}"
    );
    assert_eq!(c.pki.wrap, Wrap::TlsCrypt);
}

#[test]
fn protocolo_e_porta_globais_valem_para_todos_os_remotes() {
    let (c, _) = Config::from_ovpn(
        "client\nremote a.test\nremote b.test 80 udp\nproto tcp4-client\nport 8443\n",
        None,
    )
    .unwrap();
    assert_eq!(
        (c.remotes[0].port, c.remotes[0].proto, c.remotes[0].family),
        (8443, Proto::Tcp, Family::V4)
    );
    assert_eq!((c.remotes[1].port, c.remotes[1].proto), (80, Proto::Udp));
}

#[test]
fn porta_padrao_e_1194_e_protocolo_padrao_e_udp() {
    let (c, _) = Config::from_ovpn("client\nremote a.test\n", None).unwrap();
    assert_eq!((c.remotes[0].port, c.remotes[0].proto), (1194, Proto::Udp));
}

#[test]
fn aspas_escapes_e_comentarios() {
    let (c, _) = Config::from_ovpn(
        "client # comentário\n; outro comentário\nremote \"a b.test\" 1194\nstatic-challenge \"Código do app:\" 1\n--verb 4\n",
        None,
    )
    .unwrap();
    assert_eq!(c.remotes[0].host, "a b.test");
    let sc = c.static_challenge.unwrap();
    assert_eq!((sc.text.as_str(), sc.echo), ("Código do app:", true));
}

#[test]
fn aspas_sem_fechar_e_erro_com_a_linha() {
    let e = erro("client\nremote \"a.test\n");
    assert_eq!(e.line, Some(2));
    assert!(matches!(e.kind, ConfigErrorKind::Syntax(_)));
}

#[test]
fn bloco_sem_fechar_e_erro() {
    let e = erro("client\n<ca>\nxxx\n");
    assert_eq!(e.line, Some(2));
    assert!(matches!(e.kind, ConfigErrorKind::Syntax(_)));
}

#[test]
fn dev_tap_e_recusado() {
    let e = erro(&ler("tap.ovpn"));
    assert!(matches!(e.kind, ConfigErrorKind::Unsupported(_)));
    assert!(e.to_string().contains("dev tap"));
}

#[test]
fn chave_estatica_e_recusada() {
    let e = erro(&ler("secret.ovpn"));
    assert!(e.to_string().contains("secret"), "{e}");
}

#[test]
fn perfil_de_servidor_e_recusado() {
    let e = erro(&ler("servidor.conf"));
    assert!(matches!(e.kind, ConfigErrorKind::Unsupported(_)), "{e}");
    assert!(e.to_string().contains("servidor"));
}

#[test]
fn opcoes_nao_suportadas_dao_erro() {
    for (texto, trecho) in [
        ("fragment 1300\n", "fragment"),
        ("pkcs11-providers /x.so\n", "pkcs11"),
        ("cryptoapicert SUBJ:x\n", "cryptoapicert"),
        ("management-external-key\n", "management-external-key"),
        ("pkcs12 perfil.p12\n", "pkcs12"),
        ("tls-version-min 1.0\n", "1.2"),
        ("mode server\n", "servidor"),
    ] {
        let e = erro(&format!("client\n{texto}"));
        assert!(e.to_string().contains(trecho), "{texto}: {e}");
    }
}

#[test]
fn scripts_e_plugins_so_avisam() {
    let (_, w) = abrir("scripts.ovpn");
    let opcoes: Vec<&str> = w.iter().map(|w| w.option.as_str()).collect();
    for o in ["script-security", "up", "down", "plugin"] {
        assert!(opcoes.contains(&o), "{o} em {opcoes:?}");
    }
    assert!(
        w.iter()
            .all(|w| w.message == "o app não executa scripts nem plugins do .ovpn")
    );
}

#[test]
fn opcao_desconhecida_avisa_com_o_nome_e_a_linha() {
    let (_, w) = Config::from_ovpn("client\nremote a.test\nopcao-inventada 1\n", None).unwrap();
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].message, "opção desconhecida: opcao-inventada");
    assert_eq!(w[0].line, 3);
}

#[test]
fn opcoes_ignoradas_nao_avisam() {
    let texto = "client\ndev tun\nremote a.test\nresolv-retry infinite\nnobind\npersist-key\npersist-tun\nauth-nocache\nauth-retry nointeract\nmute 20\nverb 3\nmute-replay-warnings\nfloat\npull\ntls-client\n";
    assert!(avisos(texto).is_empty());
}

#[test]
fn avisos_de_tls_e_ns_cert_type() {
    let w = avisos(
        "client\ntls-cipher DEFAULT\ntls-groups X25519\ntls-cert-profile preferred\nns-cert-type server\ncrl-verify crl.pem\n",
    );
    assert_eq!(w.len(), 5, "{w:?}");
}

#[test]
fn ns_cert_type_vira_remote_cert_tls() {
    let (c, _) = Config::from_ovpn("client\nns-cert-type server\n", None).unwrap();
    assert!(c.verify.remote_cert_tls);
}

#[test]
fn verificacao_do_servidor_le_os_tres_modos_de_nome() {
    for (arg, modo) in [
        ("verify-x509-name servidor name", NameMode::Name),
        ("verify-x509-name serv name-prefix", NameMode::NamePrefix),
        (
            "verify-x509-name \"C=BR, CN=servidor\" subject",
            NameMode::Subject,
        ),
        ("verify-x509-name \"C=BR, CN=servidor\"", NameMode::Subject),
    ] {
        let (c, _) = Config::from_ovpn(&format!("client\n{arg}\n"), None).unwrap();
        assert_eq!(c.verify.x509_name.unwrap().mode, modo, "{arg}");
    }
}

#[test]
fn peer_fingerprint_em_linha_e_em_bloco() {
    let h = "4a:97:5e:aa:22:d8:0c:54:a5:f9:5a:28:17:91:f5:67:18:10:b1:97:fd:3c:8a:6f:02:33:63:52:e1:00:03:90";
    let (c, _) = Config::from_ovpn(&format!("client\npeer-fingerprint {h}\n"), None).unwrap();
    assert_eq!(c.verify.peer_fingerprints.len(), 1);
    assert_eq!(c.verify.peer_fingerprints[0][0], 0x4a);
    let (c, _) = Config::from_ovpn(
        &format!("client\n<peer-fingerprint>\n{h}\n{h}\n</peer-fingerprint>\n"),
        None,
    )
    .unwrap();
    assert_eq!(c.verify.peer_fingerprints.len(), 2);
    assert!(Config::from_ovpn("client\npeer-fingerprint 1234\n", None).is_err());
}

#[test]
fn auth_user_pass_nos_tres_jeitos() {
    let (c, _) = Config::from_ovpn("client\nauth-user-pass\n", None).unwrap();
    assert_eq!(c.auth_user_pass, AuthUserPass::Ask);
    let (c, _) = Config::from_ovpn(
        "client\n<auth-user-pass>\nmaria\nsegredo\n</auth-user-pass>\n",
        None,
    )
    .unwrap();
    match &c.auth_user_pass {
        AuthUserPass::Credentials { user, password } => {
            assert_eq!(user, "maria");
            assert_eq!(password.as_ref().map(|p| p.as_str()), Some("segredo"));
        }
        outro => panic!("{outro:?}"),
    }
    let texto = format!("{:?}", c);
    assert!(!texto.contains("segredo"), "o Debug vazou a senha: {texto}");
}

#[test]
fn debug_da_config_nao_mostra_chaves() {
    let (c, _) = abrir("tudo-embutido.ovpn");
    let texto = format!("{c:?}");
    assert!(!texto.contains("BEGIN"));
    assert!(!texto.contains("PRIVATE"));
}

#[test]
fn keepalive_vira_ping_e_ping_restart() {
    let (c, _) = Config::from_ovpn("client\nkeepalive 10 60\n", None).unwrap();
    assert_eq!((c.ping, c.ping_restart), (Some(10), Some(60)));
}

#[test]
fn numeros_do_perfil_antigo_e_cbc() {
    let (c, w) = abrir("antigo-cbc.ovpn");
    assert_eq!(c.tls_version_min, Some(TlsVersion::V1_2));
    assert_eq!(c.auth_user_pass, AuthUserPass::Ask);
    assert_eq!(c.remotes[0].proto, Proto::Tcp);
    assert!(w.iter().any(|w| w.option == "comp-lzo"));
    let p: Vec<String> = c.check(agora()).into_iter().map(|p| p.message).collect();
    assert!(
        p.iter().any(|m| m == "a cifra AES-256-CBC é antiga e não é suportada; peça ao administrador para usar AES-GCM"),
        "{p:?}"
    );
}

#[test]
fn pull_filter_route_e_compressao() {
    let (c, _) = Config::from_ovpn(
        "client\npull-filter ignore \"route \"\npull-filter accept dhcp-option\nroute 10.0.0.0 255.0.0.0\nroute-nopull\nallow-compression no\n",
        None,
    )
    .unwrap();
    assert_eq!(c.pull_filters.len(), 2);
    assert_eq!(c.pull_filters[0].action, FilterAction::Ignore);
    assert_eq!(c.pull_filters[0].prefix, "route ");
    assert_eq!(
        c.routes,
        vec![vec!["10.0.0.0".to_string(), "255.0.0.0".to_string()]]
    );
    assert!(c.route_nopull);
    assert_eq!(c.allow_compression, Some(AllowCompression::No));
}

#[test]
fn tls_auth_pega_a_direcao_do_segundo_argumento() {
    let (c, _) = Config::from_ovpn("client\ntls-auth ta.key 1\n", Some(&pasta())).unwrap();
    assert_eq!(c.pki.key_direction, Some(1));
    assert_eq!(c.pki.wrap, Wrap::TlsAuth);
}

#[test]
fn tls_crypt_v2_le_a_chave_do_cliente() {
    let (c, _) =
        Config::from_ovpn("client\ntls-crypt-v2 tc2-cliente.key\n", Some(&pasta())).unwrap();
    assert!(matches!(c.pki.wrap, Wrap::TlsCryptV2 { .. }));
}

#[test]
fn varios_envoltorios_juntos_sao_erro() {
    let e = erro("client\ntls-auth ta.key\ntls-crypt tc.key\n");
    assert!(e.to_string().contains("não se combinam"));
}

#[test]
fn check_aponta_o_que_falta() {
    let (c, _) = Config::from_ovpn("dev tun\n", None).unwrap();
    let m: Vec<String> = c.check(agora()).into_iter().map(|p| p.message).collect();
    assert!(m.iter().any(|m| m.contains("não é de um cliente")));
    assert!(m.iter().any(|m| m.contains("nenhum remote")));
    assert!(m.iter().any(|m| m.contains("falta a CA")));
}

#[test]
fn check_avisa_quando_nada_do_servidor_e_conferido() {
    let (mut c, _) = abrir("tudo-embutido.ovpn");
    c.verify = Default::default();
    let p = c.check(agora());
    assert!(
        p.iter()
            .any(|p| p.severity == Severity::Warning && p.message.contains("não confere"))
    );
}

#[test]
fn data_ciphers_sem_nenhuma_suportada_e_erro() {
    let (mut c, _) = abrir("tudo-embutido.ovpn");
    c.data_ciphers = Some(vec!["AES-256-CBC".into()]);
    let p = c.check(agora());
    assert!(
        p.iter()
            .any(|p| p.severity == Severity::Error && p.message.contains("nenhuma cifra"))
    );
}

// --- without_secrets -----------------------------------------------------------------------

#[test]
fn without_secrets_tira_os_blocos_e_deixa_o_resto_igual() {
    let texto = ler("tudo-embutido.ovpn");
    let limpo = Config::without_secrets(&texto);
    assert!(!limpo.contains("-----BEGIN"), "sobrou segredo:\n{limpo}");
    for tag in ["<ca>", "<cert>", "<key>", "<tls-auth>"] {
        assert!(!limpo.contains(tag), "{tag}");
    }
    assert!(limpo.contains("remote vpn.exemplo.test 1194\n"));
    assert!(limpo.contains("# Perfil de teste do tomni-ui-openvpn. Chaves só de teste.\n"));
    assert!(limpo.contains("key-direction 1\n"));
    // Sem os segredos o texto ainda é um perfil legível.
    let (c, _) = Config::from_ovpn(&limpo, None).unwrap();
    assert_eq!(c.remotes.len(), 1);
    assert!(c.pki.ca.is_empty());
}

#[test]
fn without_secrets_e_idempotente_e_nao_mexe_no_que_nao_tem_segredo() {
    let sem = "client\n# oi\nremote a.test 1194   \n\n  dev tun\n";
    assert_eq!(Config::without_secrets(sem), sem);
    let texto = ler("tudo-embutido.ovpn");
    let uma = Config::without_secrets(&texto);
    assert_eq!(Config::without_secrets(&uma), uma);
}

#[test]
fn without_secrets_tira_linhas_de_arquivo_e_a_senha() {
    let texto = "client\nca ca.crt\ncert c.crt\nkey c.key\ntls-crypt tc.key\nextra-certs e.crt\nauth-user-pass senha.txt\nremote a.test\n";
    let limpo = Config::without_secrets(texto);
    assert_eq!(limpo, "client\nauth-user-pass\nremote a.test\n");
}

#[test]
fn without_secrets_tira_o_bloco_auth_user_pass() {
    let texto = "client\n<auth-user-pass>\nmaria\nsegredo\n</auth-user-pass>\nremote a.test\n";
    let limpo = Config::without_secrets(texto);
    assert!(!limpo.contains("segredo") && !limpo.contains("maria"));
    assert_eq!(limpo, "client\nauth-user-pass\nremote a.test\n");
}

#[test]
fn without_secrets_com_texto_ilegivel_devolve_inteiro() {
    let texto = "client\nremote \"a.test\n";
    assert_eq!(Config::without_secrets(texto), texto);
}

#[test]
fn without_secrets_nos_blocos_connection_tira_so_os_segredos() {
    let texto = ler("connection-blocks.ovpn");
    let limpo = Config::without_secrets(&texto);
    assert!(!limpo.contains("-----BEGIN"));
    assert!(limpo.contains("<connection>"));
    assert!(limpo.contains("http-proxy-option EXT1 \"X-Teste: sim\""));
}

#[test]
fn set_pki_troca_os_segredos() {
    let (mut c, _) = abrir("tudo-embutido.ovpn");
    let sem = Config::without_secrets(&ler("tudo-embutido.ovpn"));
    let (mut vazio, _) = Config::from_ovpn(&sem, None).unwrap();
    assert!(
        vazio
            .check(agora())
            .iter()
            .any(|p| p.message.contains("falta a CA"))
    );
    vazio.set_pki(std::mem::take(&mut c.pki));
    assert!(vazio.check(agora()).is_empty());
}

#[test]
fn inline_antes_do_bloco_nao_duplica() {
    let ca = ler("ca.crt");
    let texto = format!("client\n<ca>\n{ca}</ca>\nca [inline]\n");
    let (c, _) = Config::from_ovpn(&texto, None).expect("lê");
    assert_eq!(c.pki.ca.len(), 1);
}
