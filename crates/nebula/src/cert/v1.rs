//! O certificado v1: protobuf (`RawNebulaCertificate`). A assinatura cobre a recodificação
//! canônica dos detalhes, então o que se lê é remontado campo a campo, como no Go.

use std::net::{IpAddr, Ipv4Addr};

use tomni_ui_netstack::Cidr;

use super::{CertError, Certificate, Curve, Version};
use crate::proto::{Reader, Value, Writer, push_u32s};

fn ruim(motivo: &str) -> CertError {
    CertError::BadFormat(format!("certificado v1 malformado: {motivo}"))
}

/// A máscara de um prefixo IPv4.
fn mask(prefixo: u8) -> u32 {
    u32::MAX.checked_shl(32 - u32::from(prefixo)).unwrap_or(0)
}

/// Os pares (endereço, máscara) → redes. Máscara que não é só uns seguidos de zeros é erro.
fn redes(pares: &[u32], rotulo: &str) -> Result<Vec<Cidr>, CertError> {
    if !pares.len().is_multiple_of(2) {
        return Err(ruim(&format!(
            "{rotulo} deveriam vir em pares (endereço, máscara) e veio um número ímpar"
        )));
    }
    pares
        .chunks(2)
        .map(|par| {
            let mascara = par[1];
            let uns = mascara.leading_ones();
            if mascara != mask(uns as u8) {
                return Err(CertError::InvalidProperties(format!(
                    "a máscara {} não é de uma rede",
                    Ipv4Addr::from(mascara)
                )));
            }
            Cidr::new(IpAddr::V4(Ipv4Addr::from(par[0])), uns as u8)
                .ok_or_else(|| ruim("prefixo fora do limite"))
        })
        .collect()
}

pub(super) fn unmarshal(bytes: &[u8], public_key: Option<&[u8]>) -> Result<Certificate, CertError> {
    let mut detalhes = None;
    let mut assinatura = Vec::new();
    for campo in Reader::new(bytes) {
        let (numero, valor) = campo.map_err(|()| ruim("bytes truncados ou fora do protobuf"))?;
        match (numero, valor) {
            (1, Value::Bytes(dados)) => detalhes = Some(dados),
            (2, Value::Bytes(dados)) => assinatura = dados.to_vec(),
            _ => {}
        }
    }
    let detalhes = detalhes.ok_or_else(|| ruim("sem os detalhes"))?;

    let mut nome = Vec::new();
    let (mut ips, mut subredes) = (Vec::new(), Vec::new());
    let mut grupos = Vec::new();
    let (mut desde, mut ate) = (0i64, 0i64);
    let mut chave = Vec::new();
    let mut ca = false;
    let mut emissor = Vec::new();
    let mut curva = Curve::Curve25519;
    for campo in Reader::new(detalhes) {
        let (numero, valor) = campo.map_err(|()| ruim("detalhes truncados"))?;
        match (numero, valor) {
            (1, Value::Bytes(dados)) => nome = dados.to_vec(),
            (2, valor) => push_u32s(&mut ips, valor).map_err(|()| ruim("endereços"))?,
            (3, valor) => push_u32s(&mut subredes, valor).map_err(|()| ruim("sub-redes"))?,
            (4, Value::Bytes(dados)) => grupos
                .push(String::from_utf8(dados.to_vec()).map_err(|_| ruim("grupo fora do UTF-8"))?),
            (5, Value::Varint(valor)) => desde = valor as i64,
            (6, Value::Varint(valor)) => ate = valor as i64,
            (7, Value::Bytes(dados)) => chave = dados.to_vec(),
            (8, Value::Varint(valor)) => ca = valor != 0,
            (9, Value::Bytes(dados)) => emissor = dados.to_vec(),
            (100, Value::Varint(valor)) => {
                curva = Curve::from_code(valor).ok_or_else(|| ruim("curva desconhecida"))?;
            }
            _ => {}
        }
    }
    let public_key = match public_key {
        Some(passada) if !passada.is_empty() => {
            if !chave.is_empty() {
                return Err(CertError::PubkeyPresent);
            }
            passada.to_vec()
        }
        _ => chave,
    };
    if public_key.is_empty() {
        return Err(CertError::InvalidProperties("sem chave pública".to_owned()));
    }
    let networks = redes(&ips, "os endereços")?;
    let unsafe_networks = redes(&subredes, "as sub-redes")?;
    if !ca && networks.is_empty() {
        return Err(CertError::InvalidProperties(
            "o certificado de um nó precisa de uma rede".to_owned(),
        ));
    }
    if networks.iter().any(|rede| rede.addr().is_unspecified()) {
        return Err(CertError::InvalidProperties(
            "o endereço zero não serve como rede".to_owned(),
        ));
    }
    Ok(Certificate {
        version: Version::V1,
        name: String::from_utf8(nome).map_err(|_| ruim("nome fora do UTF-8"))?,
        networks,
        unsafe_networks,
        groups: grupos,
        not_before: desde,
        not_after: ate,
        is_ca: ca,
        issuer: emissor,
        curve: curva,
        public_key,
        signature: assinatura,
        raw_details: Vec::new(),
    })
}

fn pares(redes: &[Cidr]) -> Vec<u32> {
    redes
        .iter()
        .flat_map(|rede| {
            let IpAddr::V4(ip) = rede.addr() else {
                // A v1 só tem IPv4 (a leitura já recusa o resto).
                return [0, 0];
            };
            [u32::from(ip), mask(rede.prefix())]
        })
        .collect()
}

/// Os detalhes em protobuf canônico (o que a CA assina); `com_chave` põe a chave pública.
pub(super) fn details(cert: &Certificate, com_chave: bool) -> Vec<u8> {
    let mut w = Writer::new();
    w.bytes(1, cert.name.as_bytes());
    w.packed_u32(2, &pares(&cert.networks));
    w.packed_u32(3, &pares(&cert.unsafe_networks));
    for grupo in &cert.groups {
        w.bytes_always(4, grupo.as_bytes());
    }
    w.int(5, cert.not_before);
    w.int(6, cert.not_after);
    if com_chave {
        w.bytes(7, &cert.public_key);
    }
    w.boolean(8, cert.is_ca);
    w.bytes(9, &cert.issuer);
    w.uint(100, u64::from(cert.curve.code()));
    w.finish()
}

pub(super) fn marshal(cert: &Certificate, com_chave: bool) -> Vec<u8> {
    let mut w = Writer::new();
    w.bytes_always(1, &details(cert, com_chave));
    w.bytes(2, &cert.signature);
    w.finish()
}
