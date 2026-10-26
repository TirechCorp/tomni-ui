//! O certificado v2: DER, com tags automáticas (`AUTOMATIC TAGS`). Os detalhes ficam guardados
//! como vieram, porque a assinatura cobre esses bytes, e não uma recodificação.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use tomni_ui_netstack::Cidr;

use super::{CertError, Certificate, Curve, Version};
use crate::der::{self, OCTET_STRING, Reader, SEQUENCE, UTF8_STRING};

// As tags de contexto do `cert_v2.asn1` do Nebula.
const TAG_DETALHES: u8 = 0xa0;
const TAG_CURVA: u8 = 0x81;
const TAG_CHAVE: u8 = 0x82;
const TAG_ASSINATURA: u8 = 0x83;
const TAG_NOME: u8 = 0x80;
const TAG_REDES: u8 = 0xa1;
const TAG_REDES_DE_FORA: u8 = 0xa2;
const TAG_GRUPOS: u8 = 0xa3;
const TAG_CA: u8 = 0x84;
const TAG_DESDE: u8 = 0x85;
const TAG_ATE: u8 = 0x86;
const TAG_EMISSOR: u8 = 0x87;

/// O maior nome (`MaxNameLength` do Nebula).
const MAX_NAME: usize = 253;
/// O maior endereço com prefixo: 16 bytes de IPv6 mais o prefixo (`MaxNetworkLength`).
const MAX_NETWORK: usize = 17;

fn ruim(motivo: &str) -> CertError {
    CertError::BadFormat(format!("certificado v2 malformado: {motivo}"))
}

fn invalida(motivo: String) -> CertError {
    CertError::InvalidProperties(motivo)
}

fn rede(bytes: &[u8]) -> Result<Cidr, CertError> {
    let (endereco, prefixo) = bytes.split_at(bytes.len() - 1);
    let ip = match endereco.len() {
        4 => IpAddr::V4(Ipv4Addr::from(
            <[u8; 4]>::try_from(endereco).unwrap_or_default(),
        )),
        16 => IpAddr::V6(Ipv6Addr::from(
            <[u8; 16]>::try_from(endereco).unwrap_or_default(),
        )),
        _ => return Err(ruim("endereço com tamanho estranho")),
    };
    Cidr::new(ip, prefixo[0]).ok_or_else(|| ruim("prefixo maior que o endereço"))
}

fn lista_de_redes(conteudo: &[u8]) -> Result<Vec<Cidr>, CertError> {
    let mut leitor = Reader::new(conteudo);
    let mut redes = Vec::new();
    while !leitor.is_empty() {
        let bytes = leitor
            .read(OCTET_STRING)
            .filter(|bytes| !bytes.is_empty() && bytes.len() <= MAX_NETWORK)
            .ok_or_else(|| ruim("lista de redes"))?;
        if bytes.len() < 2 {
            return Err(ruim("rede curta demais"));
        }
        redes.push(rede(bytes)?);
    }
    Ok(redes)
}

/// A ordem do `comparePrefix` do Nebula: IPv4 antes do IPv6, depois o endereço, depois o prefixo.
fn ordem(rede: &Cidr) -> (bool, Vec<u8>, u8) {
    match rede.addr() {
        IpAddr::V4(ip) => (false, ip.octets().to_vec(), rede.prefix()),
        IpAddr::V6(ip) => (true, ip.octets().to_vec(), rede.prefix()),
    }
}

pub(super) fn unmarshal(
    bytes: &[u8],
    public_key: Option<&[u8]>,
    curva_padrao: Curve,
) -> Result<Certificate, CertError> {
    let mut fora = Reader::new(bytes);
    let dentro = fora.read(SEQUENCE).filter(|dentro| !dentro.is_empty());
    let mut entrada = Reader::new(dentro.ok_or_else(|| ruim("sem o envelope"))?);

    let raw_details = entrada
        .read_element(TAG_DETALHES)
        .ok_or_else(|| ruim("sem os detalhes"))?;

    let curve = match entrada
        .read_optional(TAG_CURVA)
        .map_err(|()| ruim("curva"))?
    {
        None => curva_padrao,
        Some([codigo]) => {
            Curve::from_code(u64::from(*codigo)).ok_or_else(|| ruim("curva desconhecida"))?
        }
        Some(_) => return Err(ruim("curva com tamanho estranho")),
    };

    let chave = match public_key {
        Some(passada) if !passada.is_empty() => {
            if entrada.peek_tag() == Some(TAG_CHAVE) {
                return Err(CertError::PubkeyPresent);
            }
            passada.to_vec()
        }
        _ => entrada
            .read_optional(TAG_CHAVE)
            .map_err(|()| ruim("chave pública"))?
            .unwrap_or_default()
            .to_vec(),
    };
    if chave.is_empty() {
        return Err(ruim("sem chave pública"));
    }
    let assinatura = entrada
        .read(TAG_ASSINATURA)
        .filter(|assinatura| !assinatura.is_empty())
        .ok_or_else(|| ruim("sem assinatura"))?
        .to_vec();

    // Os detalhes.
    let miolo = Reader::new(raw_details)
        .read(TAG_DETALHES)
        .filter(|miolo| !miolo.is_empty())
        .ok_or_else(|| ruim("detalhes vazios"))?;
    let mut leitor = Reader::new(miolo);
    let nome = leitor
        .read(TAG_NOME)
        .filter(|nome| !nome.is_empty() && nome.len() <= MAX_NAME)
        .ok_or_else(|| ruim("nome"))?;
    let nome = std::str::from_utf8(nome).map_err(|_| ruim("nome fora do UTF-8"))?;
    let mut networks = match leitor
        .read_optional(TAG_REDES)
        .map_err(|()| ruim("redes"))?
    {
        Some(conteudo) => lista_de_redes(conteudo)?,
        None => Vec::new(),
    };
    let mut unsafe_networks = match leitor
        .read_optional(TAG_REDES_DE_FORA)
        .map_err(|()| ruim("redes de fora"))?
    {
        Some(conteudo) => lista_de_redes(conteudo)?,
        None => Vec::new(),
    };
    let mut groups = Vec::new();
    if let Some(conteudo) = leitor
        .read_optional(TAG_GRUPOS)
        .map_err(|()| ruim("grupos"))?
    {
        let mut grupos = Reader::new(conteudo);
        while !grupos.is_empty() {
            let grupo = grupos
                .read(UTF8_STRING)
                .filter(|grupo| !grupo.is_empty())
                .ok_or_else(|| ruim("grupo"))?;
            groups
                .push(String::from_utf8(grupo.to_vec()).map_err(|_| ruim("grupo fora do UTF-8"))?);
        }
    }
    let is_ca = match leitor
        .read_optional(TAG_CA)
        .map_err(|()| ruim("campo de CA"))?
    {
        None => false,
        Some([0x00]) => false,
        Some([0xff]) => true,
        Some(_) => return Err(ruim("campo de CA")),
    };
    let not_before = leitor
        .read(TAG_DESDE)
        .and_then(der::parse_int)
        .ok_or_else(|| ruim("início da validade"))?;
    let not_after = leitor
        .read(TAG_ATE)
        .and_then(der::parse_int)
        .ok_or_else(|| ruim("fim da validade"))?;
    let issuer = leitor
        .read_optional(TAG_EMISSOR)
        .map_err(|()| ruim("emissor"))?
        .unwrap_or_default()
        .to_vec();

    // As regras do `validate` do Nebula.
    if !is_ca && networks.is_empty() {
        return Err(invalida(
            "o certificado de um nó precisa de pelo menos uma rede".to_owned(),
        ));
    }
    let (mut tem_v4, mut tem_v6) = (false, false);
    for rede in &networks {
        if rede.addr().is_unspecified() {
            return Err(invalida(format!(
                "o endereço zero não serve como rede: {rede}"
            )));
        }
        match rede.addr() {
            IpAddr::V6(ip) if ip.to_ipv4_mapped().is_some() => {
                return Err(invalida(format!("redes 4in6 não são permitidas: {rede}")));
            }
            IpAddr::V4(_) => tem_v4 = true,
            IpAddr::V6(_) => tem_v6 = true,
        }
    }
    if !is_ca {
        for rede in &unsafe_networks {
            let (tem, familia) = match rede.addr() {
                IpAddr::V4(_) => (tem_v4, "IPv4"),
                IpAddr::V6(_) => (tem_v6, "IPv6"),
            };
            if !tem {
                return Err(invalida(format!(
                    "a rede de fora {rede} precisa de um endereço {familia} entre as redes"
                )));
            }
        }
    }
    for lista in [&mut networks, &mut unsafe_networks] {
        lista.sort_by_key(ordem);
        if let Some(par) = lista.windows(2).find(|par| par[0] == par[1]) {
            return Err(invalida(format!("rede repetida: {}", par[0])));
        }
    }

    Ok(Certificate {
        version: Version::V2,
        name: nome.to_owned(),
        networks,
        unsafe_networks,
        groups,
        not_before,
        not_after,
        is_ca,
        issuer,
        curve,
        public_key: chave,
        signature: assinatura,
        raw_details: raw_details.to_vec(),
    })
}

pub(super) fn marshal(cert: &Certificate, completo: bool) -> Vec<u8> {
    let mut miolo = cert.raw_details.clone();
    if completo {
        if cert.curve != Curve::Curve25519 {
            miolo.extend(der::tlv(TAG_CURVA, &[cert.curve.code()]));
        }
        miolo.extend(der::tlv(TAG_CHAVE, &cert.public_key));
    }
    miolo.extend(der::tlv(TAG_ASSINATURA, &cert.signature));
    der::tlv(SEQUENCE, &miolo)
}
