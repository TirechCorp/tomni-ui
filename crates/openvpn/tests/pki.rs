//! O PKI: as chaves em cada formato, a conferência contra o certificado e as impressões.

mod comum;

use std::time::{Duration, UNIX_EPOCH};

use comum::{agora, ler, referencias};
use tomni_ui_openvpn::pki::{
    KeyFormat, Pki, PkiError, PrivateKey, Problem, Severity, Wrap, hex, inspect,
    parse_certificates, parse_tls_crypt_v2_client,
};
use tomni_ui_openvpn::static_key::StaticKey;

fn pki(ca: &str, cert: &str, key: &str) -> Pki {
    Pki {
        ca: parse_certificates(&ler(ca)).unwrap(),
        cert: Some(parse_certificates(&ler(cert)).unwrap()),
        key: Some(PrivateKey::from_pem(&ler(key)).unwrap()),
        ..Pki::default()
    }
}

fn mensagens(p: &[Problem], gravidade: Severity) -> Vec<&str> {
    p.iter()
        .filter(|p| p.severity == gravidade)
        .map(|p| p.message.as_str())
        .collect()
}

#[test]
fn cada_formato_de_chave_e_lido_e_bate_com_o_certificado() {
    for (cert, key, formato) in [
        ("cliente-rsa.crt", "cliente-rsa.key", KeyFormat::Pkcs1),
        ("cliente-ec.crt", "cliente-ec.key", KeyFormat::Sec1),
        ("cliente-pkcs8.crt", "cliente-pkcs8.key", KeyFormat::Pkcs8),
    ] {
        let k = PrivateKey::from_pem(&ler(key)).unwrap();
        assert_eq!(k.format(), formato, "{key}");
        let ca = if cert == "cliente-ec.crt" {
            "ca-ec.crt"
        } else {
            "ca.crt"
        };
        let p = pki(ca, cert, key).check(agora());
        assert!(p.is_empty(), "{cert}: {p:?}");
    }
}

#[test]
fn chave_cifrada_pede_para_tirar_a_senha() {
    for nome in ["cliente-cifrado.key", "cliente-cifrado-legado.key"] {
        let e = PrivateKey::from_pem(&ler(nome)).unwrap_err();
        assert_eq!(e, PkiError::EncryptedKey, "{nome}");
        assert!(e.to_string().contains("openssl pkey"), "{e}");
    }
}

#[test]
fn texto_sem_chave_ou_sem_certificado_e_erro() {
    assert_eq!(
        PrivateKey::from_pem("nada\n").unwrap_err(),
        PkiError::NoPrivateKey
    );
    assert_eq!(
        parse_certificates("nada\n").unwrap_err(),
        PkiError::NoCertificate
    );
    assert!(matches!(
        parse_certificates("-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n"),
        Err(PkiError::BadCertificate(_) | PkiError::BadPem(_))
    ));
}

#[test]
fn certificado_com_texto_antes_do_pem_e_lido() {
    let texto = format!(
        "Certificate:\n    Data:\n        Version: 3\n{}",
        ler("cliente-rsa.crt")
    );
    assert_eq!(parse_certificates(&texto).unwrap().len(), 1);
}

#[test]
fn varios_certificados_no_mesmo_texto() {
    let texto = format!("{}{}", ler("ca.crt"), ler("outra-ca.crt"));
    assert_eq!(parse_certificates(&texto).unwrap().len(), 2);
}

#[test]
fn impressoes_e_nomes_batem_com_o_openssl() {
    let r = referencias();
    for nome in [
        "ca",
        "ca-ec",
        "ca-sha1",
        "outra-ca",
        "servidor",
        "servidor-sem-eku",
        "servidor-ec",
        "servidor-outra-ca",
        "cliente-rsa",
        "cliente-ec",
        "cliente-vencido",
        "cliente-outra-ca",
        "cliente-sha1",
    ] {
        let der = &parse_certificates(&ler(&format!("{nome}.crt"))).unwrap()[0];
        let info = inspect(der).unwrap();
        assert_eq!(
            hex(&info.sha256),
            r[&format!("{nome}.sha256")],
            "sha256 de {nome}"
        );
        assert_eq!(
            hex(&info.sha1),
            r[&format!("{nome}.sha1")],
            "sha1 de {nome}"
        );
        assert_eq!(
            info.subject,
            r[&format!("{nome}.subject")],
            "sujeito de {nome}"
        );
        let depois: u64 = r[&format!("{nome}.notAfter")].parse().unwrap();
        let antes: u64 = r[&format!("{nome}.notBefore")].parse().unwrap();
        assert_eq!(
            info.not_after,
            UNIX_EPOCH + Duration::from_secs(depois),
            "notAfter de {nome}"
        );
        assert_eq!(
            info.not_before,
            UNIX_EPOCH + Duration::from_secs(antes),
            "notBefore de {nome}"
        );
    }
}

#[test]
fn certificado_de_servidor_exige_ku_e_eku() {
    let com = &parse_certificates(&ler("servidor.crt")).unwrap()[0];
    assert!(inspect(com).unwrap().is_server_certificate());
    let sem = &parse_certificates(&ler("servidor-sem-eku.crt")).unwrap()[0];
    assert!(!inspect(sem).unwrap().is_server_certificate());
    let cliente = &parse_certificates(&ler("cliente-rsa.crt")).unwrap()[0];
    assert!(!inspect(cliente).unwrap().is_server_certificate());
}

#[test]
fn certificado_vencido_diz_quando_venceu() {
    let p = pki("ca.crt", "cliente-vencido.crt", "cliente-vencido.key").check(agora());
    let erros = mensagens(&p, Severity::Error);
    assert_eq!(erros.len(), 1, "{p:?}");
    assert_eq!(
        erros[0],
        "o certificado cliente-vencido venceu em 2020-02-01"
    );
}

#[test]
fn certificado_de_outra_ca_e_recusado() {
    let p = pki("ca.crt", "cliente-outra-ca.crt", "cliente-outra-ca.key").check(agora());
    let erros = mensagens(&p, Severity::Error);
    assert_eq!(erros.len(), 1, "{p:?}");
    assert!(
        erros[0].contains("não foi emitido pela CA do .ovpn"),
        "{}",
        erros[0]
    );
}

#[test]
fn chave_trocada_nao_corresponde_ao_certificado() {
    let p = pki("ca.crt", "cliente-rsa.crt", "chave-trocada.key").check(agora());
    let erros = mensagens(&p, Severity::Error);
    assert_eq!(erros.len(), 1, "{p:?}");
    assert!(erros[0].contains("a chave privada não corresponde ao certificado cliente-rsa"));
}

#[test]
fn assinatura_sha1_e_recusada() {
    let p = pki("ca-sha1.crt", "cliente-sha1.crt", "cliente-sha1.key").check(agora());
    let erros = mensagens(&p, Severity::Error);
    assert!(!erros.is_empty(), "{p:?}");
    assert!(
        erros
            .iter()
            .any(|m| m.contains("recusa") || m.contains("recusado")),
        "{erros:?}"
    );
}

#[test]
fn certificado_perto_de_vencer_avisa() {
    let p = pki("ca.crt", "cliente-rsa.crt", "cliente-rsa.key");
    let r = referencias();
    let depois: u64 = r["cliente-rsa.notAfter"].parse().unwrap();
    let quase = UNIX_EPOCH + Duration::from_secs(depois - 10 * 86_400);
    let achados = p.check(quase);
    assert!(
        mensagens(&achados, Severity::Error).is_empty(),
        "{achados:?}"
    );
    let avisos = mensagens(&achados, Severity::Warning);
    assert!(
        avisos
            .iter()
            .any(|m| m.starts_with("o certificado cliente-rsa vence em ")),
        "{avisos:?}"
    );
}

#[test]
fn falta_ca_cert_ou_chave() {
    let vazio = Pki::default().check(agora());
    assert_eq!(
        mensagens(&vazio, Severity::Error),
        vec!["falta a CA do servidor (ca)"]
    );
    let mut p = pki("ca.crt", "cliente-rsa.crt", "cliente-rsa.key");
    p.key = None;
    assert!(
        mensagens(&p.check(agora()), Severity::Error)
            .iter()
            .any(|m| m.contains("falta a chave privada"))
    );
    let mut p = pki("ca.crt", "cliente-rsa.crt", "cliente-rsa.key");
    p.cert = None;
    assert!(
        mensagens(&p.check(agora()), Severity::Error)
            .iter()
            .any(|m| m.contains("falta o certificado"))
    );
}

#[test]
fn resumo_mostra_cn_validade_e_impressao() {
    let s = pki("ca.crt", "cliente-rsa.crt", "cliente-rsa.key")
        .summary()
        .unwrap();
    let r = referencias();
    assert_eq!(s.client_common_name.as_deref(), Some("cliente-rsa"));
    assert_eq!(s.fingerprint_sha256, r["cliente-rsa.sha256"]);
    assert_eq!(s.ca_common_name.as_deref(), Some("Tomni Teste CA"));
    assert!(Pki::default().summary().is_none());
}

#[test]
fn debug_do_pki_nao_mostra_segredos() {
    let mut p = pki("ca.crt", "cliente-rsa.crt", "cliente-rsa.key");
    p.static_key = Some(StaticKey::from_pem(&ler("ta.key")).unwrap());
    let texto = format!("{p:?}");
    assert!(!texto.contains("BEGIN"), "{texto}");
    assert!(!texto.to_lowercase().contains("private key"), "{texto}");
}

#[test]
fn chave_estatica_do_tls_auth_e_lida() {
    let k = StaticKey::from_pem(&ler("ta.key")).unwrap();
    assert_eq!(k.as_bytes().len(), 256);
    let de_novo = StaticKey::from_pem(&k.to_pem()).unwrap();
    assert_eq!(de_novo.as_bytes(), k.as_bytes());
    assert!(StaticKey::from_pem(&ler("tc.key")).is_ok());
}

#[test]
fn tls_crypt_v2_do_cliente_e_lido() {
    match parse_tls_crypt_v2_client(&ler("tc2-cliente.key")).unwrap() {
        Wrap::TlsCryptV2 { kc, wkc } => {
            assert_eq!(kc.len(), 256);
            assert!(wkc.len() > 2 && wkc.len() <= 1024);
            assert_eq!(
                usize::from(u16::from_be_bytes([wkc[wkc.len() - 2], wkc[wkc.len() - 1]])),
                wkc.len()
            );
        }
        outro => panic!("{outro:?}"),
    }
}

#[test]
fn tls_crypt_v2_do_servidor_nao_serve_de_chave_de_cliente() {
    assert!(parse_tls_crypt_v2_client(&ler("tc2-servidor.key")).is_err());
    assert!(parse_tls_crypt_v2_client("nada").is_err());
}
