//! Os testes dos certificados contra as fixtures geradas pelo `nebula-cert` oficial
//! (`testes/nebula/gerar-fixtures.sh`) e contra certificados montados à mão.

use std::fs;
use std::path::PathBuf;

use serde_json::Value;

use super::pool::check_constraints;
use super::*;
use crate::der::{self, OCTET_STRING, SEQUENCE};

/// 2027-01-01, bem dentro da validade das fixtures (geradas em 2026-10-22, com 100 anos).
const AGORA: i64 = 1_798_761_600;

fn caminho(nome: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(nome)
}

fn texto(nome: &str) -> String {
    fs::read_to_string(caminho(nome)).unwrap_or_else(|erro| panic!("{nome}: {erro}"))
}

fn cert(nome: &str) -> Vec<Certificate> {
    Certificate::from_pem(&texto(&format!("{nome}.crt"))).unwrap()
}

fn cert1(nome: &str) -> Certificate {
    let mut certs = cert(nome);
    assert_eq!(certs.len(), 1, "{nome}");
    certs.remove(0)
}

fn json(nome: &str) -> Vec<Value> {
    serde_json::from_str(&texto(&format!("{nome}.json"))).unwrap()
}

fn pool(nome: &str) -> CaPool {
    CaPool::from_pem(&texto(&format!("{nome}.crt"))).unwrap()
}

/// `2026-10-22T03:58:11Z` → segundos desde 1970.
fn rfc3339(texto: &str) -> i64 {
    let n = |i: usize, j: usize| texto[i..j].parse::<i64>().unwrap();
    let (ano, mes, dia) = (n(0, 4), n(5, 7), n(8, 10));
    let ano = if mes <= 2 { ano - 1 } else { ano };
    let era = ano.div_euclid(400);
    let yoe = ano - era * 400;
    let doy = (153 * (if mes > 2 { mes - 3 } else { mes + 9 }) + 2) / 5 + dia - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let dias = era * 146_097 + doe - 719_468;
    dias * 86_400 + n(11, 13) * 3600 + n(14, 16) * 60 + n(17, 19)
}

fn hex_de(valor: &Value) -> String {
    valor.as_str().unwrap().to_owned()
}

const NOS: &[&str] = &[
    "no-v2",
    "no-v2-b",
    "no-v1",
    "no-v1-b",
    "no-v1-ca-v1",
    "no-v1v2",
    "no-v1v2-b",
    "no-chacha",
    "no-p256",
    "no-p256-b",
    "no-unsafe",
    "no-grupos",
    "no-outra-ca",
    "no-vencido",
    "lighthouse",
    "direto",
    "app",
    "lighthouse-chacha",
    "direto-chacha",
    "app-chacha",
    "lighthouse-v1",
    "direto-v1",
    "app-v1",
];
const CAS: &[&str] = &["ca-25519", "ca-25519-v1", "ca-p256", "ca-outra"];

#[test]
fn cada_fixture_confere_com_o_json_do_nebula_cert() {
    for nome in NOS.iter().chain(CAS) {
        let certs = cert(nome);
        let jsons = json(nome);
        assert_eq!(certs.len(), jsons.len(), "{nome}");
        for (c, j) in certs.iter().zip(&jsons) {
            let versao = j["version"].as_u64().unwrap_or(1);
            assert_eq!(u64::from(c.version().number()), versao, "{nome}: versão");
            // A v1 traz os detalhes dentro de `details`; a v2 também.
            let d = &j["details"];
            assert_eq!(c.name(), d["name"].as_str().unwrap(), "{nome}: nome");
            let redes: Vec<String> = c.networks().iter().map(ToString::to_string).collect();
            let esperadas: Vec<String> = d["networks"]
                .as_array()
                .map(|v| v.iter().map(hex_de).collect())
                .unwrap_or_default();
            assert_eq!(redes, esperadas, "{nome}: redes");
            let fora: Vec<String> = c
                .unsafe_networks()
                .iter()
                .map(ToString::to_string)
                .collect();
            let fora_esperadas: Vec<String> = d["unsafeNetworks"]
                .as_array()
                .map(|v| v.iter().map(hex_de).collect())
                .unwrap_or_default();
            assert_eq!(fora, fora_esperadas, "{nome}: redes de fora");
            let grupos: Vec<String> = d["groups"]
                .as_array()
                .map(|v| v.iter().map(hex_de).collect())
                .unwrap_or_default();
            assert_eq!(c.groups(), grupos, "{nome}: grupos");
            assert_eq!(
                c.not_before(),
                rfc3339(d["notBefore"].as_str().unwrap()),
                "{nome}"
            );
            assert_eq!(
                c.not_after(),
                rfc3339(d["notAfter"].as_str().unwrap()),
                "{nome}"
            );
            assert_eq!(c.is_ca(), d["isCa"].as_bool().unwrap(), "{nome}: isCa");
            assert_eq!(c.issuer(), d["issuer"].as_str().unwrap(), "{nome}: emissor");
            // A v1 põe a curva e a chave nos detalhes; a v2, na raiz.
            let curva = d["curve"].as_str().or(j["curve"].as_str()).unwrap();
            assert_eq!(c.curve().to_string(), curva, "{nome}: curva");
            let chave = d["publicKey"].as_str().or(j["publicKey"].as_str()).unwrap();
            assert_eq!(crate::util::hex(c.public_key()), chave, "{nome}: chave");
            assert_eq!(
                crate::util::hex(c.signature()),
                j["signature"].as_str().unwrap(),
                "{nome}: assinatura"
            );
            assert_eq!(
                c.fingerprint(),
                j["fingerprint"].as_str().unwrap(),
                "{nome}: impressão digital"
            );
        }
    }
}

#[test]
fn a_recodificacao_canonica_devolve_o_pem_original() {
    for nome in NOS.iter().chain(CAS) {
        let original = texto(&format!("{nome}.crt"));
        let recodificado: String = cert(nome).iter().map(Certificate::to_pem).collect();
        assert_eq!(recodificado.trim(), original.trim(), "{nome}");
    }
}

#[test]
fn a_forma_do_handshake_volta_inteira_com_a_chave_do_noise() {
    for nome in NOS {
        for c in cert(nome) {
            let sem_chave = c.marshal_for_handshake();
            assert!(sem_chave.len() < c.marshal().len(), "{nome}");
            let volta =
                Certificate::unmarshal(c.version(), &sem_chave, Some(c.public_key()), c.curve())
                    .unwrap();
            assert_eq!(volta, c, "{nome}");
            assert_eq!(volta.fingerprint(), c.fingerprint(), "{nome}");
            // Com a chave já dentro do certificado, o handshake é recusado.
            let erro =
                Certificate::unmarshal(c.version(), &c.marshal(), Some(c.public_key()), c.curve())
                    .unwrap_err();
            assert_eq!(erro, CertError::PubkeyPresent, "{nome}");
        }
    }
}

#[test]
fn certificado_malformado_e_recusado_sem_panico() {
    let c = cert1("no-v2");
    let bytes = c.marshal();
    for corte in [0, 1, 5, bytes.len() / 2, bytes.len() - 1] {
        assert!(
            Certificate::unmarshal(Version::V2, &bytes[..corte], None, Curve::Curve25519).is_err(),
            "corte em {corte}"
        );
    }
    let c = cert1("no-v1");
    let bytes = c.marshal();
    for corte in [0, 1, 5, bytes.len() / 2, bytes.len() - 1] {
        assert!(
            Certificate::unmarshal(Version::V1, &bytes[..corte], None, Curve::Curve25519).is_err(),
            "corte em {corte}"
        );
    }
    assert!(
        Certificate::unmarshal(Version::V1, &vec![0x0a; 70_000], None, Curve::Curve25519).is_err()
    );
    assert!(Certificate::from_pem("nada aqui").is_err());
}

#[test]
fn v1_com_uma_rede_de_fora_e_grupos() {
    let c = cert1("no-unsafe");
    assert_eq!(c.unsafe_networks().len(), 1);
    assert_eq!(c.unsafe_networks()[0].to_string(), "192.168.50.0/24");
    let c = cert1("no-grupos");
    assert_eq!(c.groups(), ["admin", "dev"]);
}

#[test]
fn a_v1v2_traz_as_duas_versoes_do_mesmo_no() {
    let certs = cert("no-v1v2");
    assert_eq!(certs.len(), 2);
    assert_eq!(certs[0].version(), Version::V1);
    assert_eq!(certs[1].version(), Version::V2);
    assert_eq!(certs[0].public_key(), certs[1].public_key());
    assert_ne!(certs[0].fingerprint(), certs[1].fingerprint());
}

#[test]
fn a_ca_confere_os_nos_das_duas_versoes() {
    // A CA v2 assina certificados v1 e v2 (o `ca-25519` serve aos dois formatos).
    let ca = pool("ca-25519");
    for nome in [
        "no-v2",
        "no-v2-b",
        "no-v1",
        "no-v1-b",
        "no-chacha",
        "no-grupos",
        "no-unsafe",
    ] {
        ca.verify(&cert1(nome), AGORA)
            .unwrap_or_else(|erro| panic!("{nome}: {erro}"));
    }
    for c in cert("no-v1v2") {
        ca.verify(&c, AGORA).unwrap();
    }
    pool("ca-25519-v1")
        .verify(&cert1("no-v1-ca-v1"), AGORA)
        .unwrap();
    pool("ca-outra")
        .verify(&cert1("no-outra-ca"), AGORA)
        .unwrap();
}

#[test]
fn a_ca_recusa_com_a_mensagem_certa() {
    let ca = pool("ca-25519");
    let erro = ca.verify(&cert1("no-outra-ca"), AGORA).unwrap_err();
    assert_eq!(
        erro.to_string(),
        "o certificado no-outra-ca não foi assinado por nenhuma CA do config"
    );
    let erro = ca.verify(&cert1("no-vencido"), AGORA).unwrap_err();
    assert!(matches!(erro, CertError::Expired { .. }), "{erro}");
    assert!(
        erro.to_string()
            .starts_with("o certificado no-vencido venceu em 22/10/2026"),
        "{erro}"
    );
    // Antes da geração das fixtures nada vale ainda (a CA, nem o nó).
    let erro = ca.verify(&cert1("no-v2"), 1_700_000_000).unwrap_err();
    assert!(matches!(erro, CertError::NotYetValid { .. }), "{erro}");
    // Depois dos 100 anos a CA venceu.
    let erro = ca.verify(&cert1("no-v2"), 5_000_000_000).unwrap_err();
    assert!(matches!(erro, CertError::RootExpired { .. }), "{erro}");
    assert!(erro.to_string().contains("venceu em 28/09/2126"), "{erro}");
}

#[test]
fn bloqueio_assinatura_adulterada_e_curva_trocada() {
    let c = cert1("no-v2");
    let mut ca = pool("ca-25519");
    ca.block(&c.fingerprint().to_uppercase());
    assert!(matches!(
        ca.verify(&c, AGORA),
        Err(CertError::Blocklisted { .. })
    ));

    let ca = pool("ca-25519");
    for nome in ["no-v2", "no-v1"] {
        let mut ruim = cert1(nome);
        ruim.signature[0] ^= 1;
        assert!(
            matches!(
                ca.verify(&ruim, AGORA),
                Err(CertError::SignatureMismatch { .. })
            ),
            "{nome}"
        );
    }
    let mut ruim = cert1("no-v1");
    ruim.name.push('x');
    assert!(matches!(
        ca.verify(&ruim, AGORA),
        Err(CertError::SignatureMismatch { .. })
    ));
    // Mudar a chave pública também invalida a assinatura.
    let mut ruim = cert1("no-v2");
    ruim.public_key[0] ^= 1;
    assert!(matches!(
        ca.verify(&ruim, AGORA),
        Err(CertError::SignatureMismatch { .. })
    ));

    let mut troca = cert1("no-v2");
    troca.curve = Curve::P256;
    assert!(matches!(
        ca.verify(&troca, AGORA),
        Err(CertError::CurveMismatch { .. })
    ));
}

#[test]
fn p256_le_mas_a_rede_recusa_com_a_mensagem_certa() {
    let c = cert1("no-p256");
    assert_eq!(c.curve(), Curve::P256);
    let ca = pool("ca-p256");
    assert_eq!(ca.len(), 1);
    let erro = ca.verify(&c, AGORA).unwrap_err();
    assert_eq!(erro.to_string(), "certificado P-256 ainda não é suportado");
    let chave = PrivateKey::from_pem(&texto("no-p256.key")).unwrap();
    assert_eq!(chave.curve(), Curve::P256);
    assert!(chave.matches(&c).is_err());
}

#[test]
fn restricoes_da_ca_para_grupos_redes_e_validade() {
    let ca_original = cert1("ca-25519");
    let no = cert1("no-grupos");
    check_constraints(&ca_original, &no).unwrap();

    let mut ca = ca_original.clone();
    ca.groups = vec!["admin".to_owned()];
    let erro = check_constraints(&ca, &no).unwrap_err();
    assert!(
        erro.to_string().contains("o grupo dev não está na CA"),
        "{erro}"
    );

    let mut ca = ca_original.clone();
    ca.networks = vec!["10.42.0.0/24".parse().unwrap()];
    check_constraints(&ca, &no).unwrap();
    ca.networks = vec!["10.99.0.0/24".parse().unwrap()];
    let erro = check_constraints(&ca, &no).unwrap_err();
    assert!(
        erro.to_string().contains("a rede 10.42.0.26/24 está fora"),
        "{erro}"
    );
    // Uma rede mais larga que a da CA também não vale (`/16` dentro de `/24`).
    ca.networks = vec!["10.42.0.0/24".parse().unwrap()];
    let mut largo = no.clone();
    largo.networks = vec!["10.42.0.26/16".parse().unwrap()];
    assert!(check_constraints(&ca, &largo).is_err());

    let mut ca = ca_original.clone();
    ca.unsafe_networks = vec!["172.16.0.0/12".parse().unwrap()];
    let fora = cert1("no-unsafe");
    assert!(check_constraints(&ca, &fora).is_err());
    ca.unsafe_networks = vec!["192.168.0.0/16".parse().unwrap()];
    check_constraints(&ca, &fora).unwrap();

    let mut ca = ca_original.clone();
    ca.not_after = no.not_after - 1;
    assert!(check_constraints(&ca, &no).is_err());
    let mut ca = ca_original;
    ca.not_before = no.not_before + 1;
    assert!(check_constraints(&ca, &no).is_err());
}

#[test]
fn ca_precisa_ser_de_ca_e_estar_autoassinada() {
    let mut pool = CaPool::new();
    assert_eq!(pool.add(cert1("no-v2")).unwrap_err(), CertError::NotCa);
    let mut ca = cert1("ca-25519");
    ca.signature[0] ^= 1;
    assert_eq!(pool.add(ca).unwrap_err(), CertError::NotSelfSigned);
    assert!(pool.is_empty());
    // Um `pki.ca` com várias CAs.
    let dois = format!("{}{}", texto("ca-25519.crt"), texto("ca-outra.crt"));
    assert_eq!(CaPool::from_pem(&dois).unwrap().len(), 2);
}

#[test]
fn a_chave_privada_confere_com_o_certificado() {
    let chave = PrivateKey::from_pem(&texto("no-v2.key")).unwrap();
    assert_eq!(chave.curve(), Curve::Curve25519);
    chave.matches(&cert1("no-v2")).unwrap();
    chave.matches(&cert1("no-v2-b")).unwrap_err();
    assert_eq!(
        chave.matches(&cert1("no-v2-b")).unwrap_err(),
        CertError::KeyMismatch
    );
    // A v1v2 usa a mesma chave nas duas.
    let chave = PrivateKey::from_pem(&texto("no-v1v2.key")).unwrap();
    for c in cert("no-v1v2") {
        chave.matches(&c).unwrap();
    }
    assert!(!format!("{chave:?}").contains(&crate::util::hex(chave.secret())));
    // A chave de uma CA não serve de chave de nó.
    let erro = PrivateKey::from_pem(&texto("ca-25519.key")).unwrap_err();
    assert!(erro.to_string().contains("chave de uma CA"), "{erro}");
    assert!(PrivateKey::from_pem("-----BEGIN NEBULA X25519 PRIVATE KEY-----\nAAAA\n-----END NEBULA X25519 PRIVATE KEY-----").is_err());
}

// --- certificados v2 montados à mão ---

fn rede_der(ip: &[u8], prefixo: u8) -> Vec<u8> {
    let mut bytes = ip.to_vec();
    bytes.push(prefixo);
    der::tlv(OCTET_STRING, &bytes)
}

/// Um certificado v2 sem assinatura válida, só para o formato.
fn v2_a_mao(redes: &[Vec<u8>], fora: &[Vec<u8>], ca: bool) -> Vec<u8> {
    let mut detalhes = der::tlv(0x80, b"mao");
    if !redes.is_empty() {
        detalhes.extend(der::tlv(0xa1, &redes.concat()));
    }
    if !fora.is_empty() {
        detalhes.extend(der::tlv(0xa2, &fora.concat()));
    }
    detalhes.extend(der::tlv(0xa3, &der::tlv(der::UTF8_STRING, b"g1")));
    if ca {
        detalhes.extend(der::tlv(0x84, &[0xff]));
    }
    detalhes.extend(der::tlv(0x85, &der::int_content(1_000)));
    detalhes.extend(der::tlv(0x86, &der::int_content(2_000_000_000)));
    let mut miolo = der::tlv(0xa0, &detalhes);
    miolo.extend(der::tlv(0x82, &[7; 32]));
    miolo.extend(der::tlv(0x83, &[9; 64]));
    der::tlv(SEQUENCE, &miolo)
}

fn le(bytes: &[u8]) -> Result<Certificate, CertError> {
    Certificate::unmarshal(Version::V2, bytes, None, Curve::Curve25519)
}

#[test]
fn v2_ipv4_e_ipv6_saem_ordenados() {
    let v6 = rede_der(
        &"fd00::1".parse::<std::net::Ipv6Addr>().unwrap().octets(),
        64,
    );
    let v4 = rede_der(&[10, 0, 0, 1], 8);
    let v4b = rede_der(&[10, 0, 0, 1], 24);
    let c = le(&v2_a_mao(&[v6, v4b, v4], &[], false)).unwrap();
    let redes: Vec<String> = c.networks().iter().map(ToString::to_string).collect();
    assert_eq!(redes, ["10.0.0.1/8", "10.0.0.1/24", "fd00::1/64"]);
    assert_eq!(c.groups(), ["g1"]);
    assert_eq!(c.not_before(), 1_000);
    assert_eq!(c.not_after(), 2_000_000_000);
    // A recodificação devolve os mesmos bytes, mesmo com as redes fora de ordem.
    assert_eq!(le(&c.marshal()).unwrap(), c);
}

#[test]
fn v2_aplica_as_regras_de_validacao() {
    let ok = rede_der(&[10, 0, 0, 1], 24);
    let zero = rede_der(&[0, 0, 0, 0], 0);
    let mapeada = rede_der(
        &"::ffff:10.0.0.1"
            .parse::<std::net::Ipv6Addr>()
            .unwrap()
            .octets(),
        120,
    );
    let v6 = rede_der(
        &"fd00::1".parse::<std::net::Ipv6Addr>().unwrap().octets(),
        64,
    );
    let grande = rede_der(&[10, 0, 0, 1], 33);

    assert!(
        le(&v2_a_mao(&[], &[], false))
            .unwrap_err()
            .to_string()
            .contains("pelo menos uma rede")
    );
    assert!(le(&v2_a_mao(&[], &[], true)).is_ok());
    assert!(
        le(&v2_a_mao(&[zero], &[], false))
            .unwrap_err()
            .to_string()
            .contains("endereço zero")
    );
    assert!(
        le(&v2_a_mao(&[mapeada], &[], false))
            .unwrap_err()
            .to_string()
            .contains("4in6")
    );
    assert!(
        le(&v2_a_mao(&[ok.clone(), ok.clone()], &[], false))
            .unwrap_err()
            .to_string()
            .contains("repetida")
    );
    assert!(le(&v2_a_mao(&[grande], &[], false)).is_err());
    // Uma rede de fora IPv6 exige um endereço IPv6 entre as redes (e só em nós, não em CAs).
    assert!(
        le(&v2_a_mao(
            std::slice::from_ref(&ok),
            std::slice::from_ref(&v6),
            false
        ))
        .is_err()
    );
    assert!(le(&v2_a_mao(&[], &[v6], true)).is_ok());
    assert!(
        le(&v2_a_mao(
            std::slice::from_ref(&ok),
            &[rede_der(&[192, 168, 0, 0], 16)],
            false
        ))
        .is_ok()
    );
}

#[test]
fn v2_recusa_inteiros_fora_do_menor_tamanho_e_tags_fora_de_ordem() {
    let ok = rede_der(&[10, 0, 0, 1], 24);
    let mut bytes = v2_a_mao(&[ok], &[], false);
    // O início da validade com um zero a mais na frente (1000 = 0x03e8 → 0x0003e8).
    let alvo = [0x85, 0x02, 0x03, 0xe8];
    let posicao = bytes.windows(4).position(|janela| janela == alvo).unwrap();
    assert!(le(&bytes).is_ok());
    bytes[posicao + 1] = 3;
    bytes.insert(posicao + 2, 0x00);
    // Os comprimentos externos ficaram um byte menores que o conteúdo: malformado.
    assert!(le(&bytes).is_err());
}

#[test]
fn datas_das_mensagens_usam_o_dia_do_utc() {
    let c = cert1("no-vencido");
    assert_eq!(crate::util::format_date(c.not_after()), "22/10/2026");
}
