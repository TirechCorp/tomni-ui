//! Os certificados do Nebula, nas duas versões: a v1 (protobuf) e a v2 (DER). Portado do pacote
//! `cert` do Nebula (github.com/slackhq/nebula, licença MIT).
//!
//! Um certificado diz o nome do nó, os endereços dele na rede (`10.42.0.7/24`), as redes que
//! ele alcança por fora (`unsafe_networks`), os grupos, a validade e a chave pública, e leva a
//! assinatura da CA. [`CaPool`] guarda as CAs em que o config confia e confere cada certificado
//! (assinatura, validade e as restrições da CA); [`PrivateKey`] é a chave privada do nó.

mod key;
mod pool;
mod v1;
mod v2;

#[cfg(test)]
mod tests;

use std::fmt;

use sha2::{Digest, Sha256};
use tomni_ui_netstack::Cidr;

use crate::pem;
use crate::util::{format_date, hex};

pub use key::PrivateKey;
pub use pool::CaPool;

/// O tamanho máximo de um certificado, igual ao do Nebula (`MaxCertificateSize`).
pub(crate) const MAX_CERT_SIZE: usize = 65536;

/// A versão do formato do certificado.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Version {
    /// O formato original, em protobuf: só IPv4, uma rede por nó.
    V1,
    /// O formato novo, em DER: IPv4 e IPv6, várias redes.
    V2,
}

impl Version {
    /// O número da versão (1 ou 2), como vai no handshake.
    pub fn number(self) -> u8 {
        match self {
            Version::V1 => 1,
            Version::V2 => 2,
        }
    }

    /// A versão de um número do handshake (`0` vale como a v1, como no Nebula).
    pub fn from_number(numero: u32) -> Option<Version> {
        match numero {
            0 | 1 => Some(Version::V1),
            2 => Some(Version::V2),
            _ => None,
        }
    }
}

/// A curva das chaves do certificado.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Curve {
    /// Curve25519 (X25519 para o Noise, Ed25519 para as assinaturas da CA).
    Curve25519,
    /// NIST P-256. O certificado é lido, mas a rede ainda não a usa (M48 adiou).
    P256,
}

impl Curve {
    /// O valor do enum `Curve` do Nebula (`CURVE25519` = 0, `P256` = 1).
    pub(crate) fn code(self) -> u8 {
        match self {
            Curve::Curve25519 => 0,
            Curve::P256 => 1,
        }
    }

    pub(crate) fn from_code(codigo: u64) -> Option<Curve> {
        match codigo {
            0 => Some(Curve::Curve25519),
            1 => Some(Curve::P256),
            _ => None,
        }
    }
}

impl fmt::Display for Curve {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Curve::Curve25519 => "CURVE25519",
            Curve::P256 => "P256",
        })
    }
}

/// Por que um certificado, uma chave ou uma CA foi recusado. O texto já é uma mensagem para o
/// usuário, em português.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CertError {
    /// O texto PEM está malformado ou não traz o que se esperava.
    Pem(String),
    /// Os bytes do certificado estão malformados.
    BadFormat(String),
    /// O certificado tem uma propriedade inválida (uma rede com o endereço zero, por exemplo).
    InvalidProperties(String),
    /// Um recurso que o Tomni UI ainda não suporta (P-256, rede só IPv6, chave com senha).
    Unsupported(String),
    /// O certificado venceu.
    Expired {
        /// O nome do nó.
        name: String,
        /// O fim da validade, em segundos desde 1970.
        not_after: i64,
    },
    /// O certificado ainda não começou a valer.
    NotYetValid {
        /// O nome do nó.
        name: String,
        /// O início da validade, em segundos desde 1970.
        not_before: i64,
    },
    /// A CA que assinou o certificado venceu.
    RootExpired {
        /// O nome da CA.
        name: String,
        /// O fim da validade, em segundos desde 1970.
        not_after: i64,
    },
    /// Nenhuma CA do config assinou o certificado.
    CaNotFound {
        /// O nome do nó.
        name: String,
    },
    /// O certificado está na lista de bloqueio (`pki.blocklist`).
    Blocklisted {
        /// O nome do nó.
        name: String,
    },
    /// O certificado e a CA usam curvas diferentes.
    CurveMismatch {
        /// O nome do nó.
        name: String,
    },
    /// A assinatura da CA não confere com o certificado.
    SignatureMismatch {
        /// O nome do nó.
        name: String,
    },
    /// O certificado passa de algo que a CA permite (rede, grupo ou validade).
    Constraints {
        /// O nome do nó.
        name: String,
        /// O que passou.
        reason: String,
    },
    /// O certificado não é de uma CA.
    NotCa,
    /// A CA não é assinada por ela mesma.
    NotSelfSigned,
    /// A chave privada não é a do certificado.
    KeyMismatch,
    /// O certificado já traz a chave pública, e não devia (a forma do handshake não a leva).
    PubkeyPresent,
}

impl fmt::Display for CertError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CertError::Pem(motivo) | CertError::BadFormat(motivo) => f.write_str(motivo),
            CertError::InvalidProperties(motivo) => {
                write!(f, "o certificado tem uma propriedade inválida: {motivo}")
            }
            CertError::Unsupported(motivo) => f.write_str(motivo),
            CertError::Expired { name, not_after } => write!(
                f,
                "o certificado {name} venceu em {}",
                format_date(*not_after)
            ),
            CertError::NotYetValid { name, not_before } => write!(
                f,
                "o certificado {name} só começa a valer em {}",
                format_date(*not_before)
            ),
            CertError::RootExpired { name, not_after } => {
                write!(f, "a CA {name} venceu em {}", format_date(*not_after))
            }
            CertError::CaNotFound { name } => write!(
                f,
                "o certificado {name} não foi assinado por nenhuma CA do config"
            ),
            CertError::Blocklisted { name } => {
                write!(
                    f,
                    "o certificado {name} está na lista de bloqueio do config"
                )
            }
            CertError::CurveMismatch { name } => write!(
                f,
                "o certificado {name} usa uma curva diferente da CA que o assinou"
            ),
            CertError::SignatureMismatch { name } => {
                write!(f, "a assinatura da CA não confere com o certificado {name}")
            }
            CertError::Constraints { name, reason } => write!(
                f,
                "o certificado {name} passa do que a CA permite: {reason}"
            ),
            CertError::NotCa => f.write_str("o certificado não é de uma CA"),
            CertError::NotSelfSigned => f.write_str("a CA não está assinada por ela mesma"),
            CertError::KeyMismatch => {
                f.write_str("a chave privada não é a do certificado (as chaves não formam um par)")
            }
            CertError::PubkeyPresent => {
                f.write_str("o certificado do handshake traz uma chave pública que não devia")
            }
        }
    }
}

impl std::error::Error for CertError {}

/// Um certificado do Nebula (v1 ou v2), já lido e conferido no formato (a assinatura da CA só é
/// conferida por [`CaPool::verify`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Certificate {
    pub(crate) version: Version,
    pub(crate) name: String,
    pub(crate) networks: Vec<Cidr>,
    pub(crate) unsafe_networks: Vec<Cidr>,
    pub(crate) groups: Vec<String>,
    pub(crate) not_before: i64,
    pub(crate) not_after: i64,
    pub(crate) is_ca: bool,
    pub(crate) issuer: Vec<u8>,
    pub(crate) curve: Curve,
    pub(crate) public_key: Vec<u8>,
    pub(crate) signature: Vec<u8>,
    /// A v2: os detalhes como vieram (a assinatura cobre exatamente esses bytes).
    pub(crate) raw_details: Vec<u8>,
}

impl Certificate {
    /// Lê todos os certificados de um texto PEM (`NEBULA CERTIFICATE` e `NEBULA CERTIFICATE
    /// V2`). Um `pki.cert` pode trazer a v1 e a v2 do mesmo nó, e um `pki.ca`, várias CAs.
    pub fn from_pem(texto: &str) -> Result<Vec<Certificate>, CertError> {
        let blocos = pem::decode(texto).map_err(CertError::Pem)?;
        let mut certificados = Vec::new();
        for bloco in blocos {
            let versao = match bloco.label.as_str() {
                "NEBULA CERTIFICATE" => Version::V1,
                "NEBULA CERTIFICATE V2" => Version::V2,
                _ => continue,
            };
            certificados.push(Certificate::unmarshal(
                versao,
                &bloco.data,
                None,
                Curve::Curve25519,
            )?);
        }
        if certificados.is_empty() {
            return Err(CertError::Pem(
                "o texto não tem nenhum bloco NEBULA CERTIFICATE".to_owned(),
            ));
        }
        Ok(certificados)
    }

    /// Lê os bytes de um certificado. Com `public_key`, é a forma do handshake (sem a chave
    /// pública, que vem do Noise): o certificado não pode trazer a chave. `curve` só vale para
    /// a v2 sem a curva escrita (o padrão, Curve25519).
    pub fn unmarshal(
        versao: Version,
        bytes: &[u8],
        public_key: Option<&[u8]>,
        curve: Curve,
    ) -> Result<Certificate, CertError> {
        if bytes.is_empty() || bytes.len() > MAX_CERT_SIZE {
            return Err(CertError::BadFormat(
                "o certificado está vazio ou grande demais".to_owned(),
            ));
        }
        match versao {
            Version::V1 => v1::unmarshal(bytes, public_key),
            Version::V2 => v2::unmarshal(bytes, public_key, curve),
        }
    }

    /// A versão do formato.
    pub fn version(&self) -> Version {
        self.version
    }

    /// O nome do nó.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Os endereços do nó na rede, com o tamanho da rede (`10.42.0.7/24`).
    pub fn networks(&self) -> &[Cidr] {
        &self.networks
    }

    /// As redes que o nó alcança por fora do Nebula.
    pub fn unsafe_networks(&self) -> &[Cidr] {
        &self.unsafe_networks
    }

    /// Os grupos do nó.
    pub fn groups(&self) -> &[String] {
        &self.groups
    }

    /// O início da validade, em segundos desde 1970.
    pub fn not_before(&self) -> i64 {
        self.not_before
    }

    /// O fim da validade, em segundos desde 1970.
    pub fn not_after(&self) -> i64 {
        self.not_after
    }

    /// `true` se é o certificado de uma CA.
    pub fn is_ca(&self) -> bool {
        self.is_ca
    }

    /// A impressão digital (hexadecimal) da CA que assinou; vazia numa CA.
    pub fn issuer(&self) -> String {
        hex(&self.issuer)
    }

    /// A curva das chaves.
    pub fn curve(&self) -> Curve {
        self.curve
    }

    /// A chave pública do nó (a do Noise) ou da CA (a das assinaturas).
    pub fn public_key(&self) -> &[u8] {
        &self.public_key
    }

    /// A assinatura da CA.
    pub fn signature(&self) -> &[u8] {
        &self.signature
    }

    /// `true` se o instante (segundos desde 1970) está fora da validade.
    pub fn expired(&self, agora: i64) -> bool {
        self.not_before > agora || self.not_after < agora
    }

    /// A impressão digital: o SHA-256, em hexadecimal, que identifica o certificado (é a que o
    /// `issuer` dos certificados assinados por esta CA carrega, e a da lista de bloqueio).
    pub fn fingerprint(&self) -> String {
        hex(&self.fingerprint_bytes())
    }

    pub(crate) fn fingerprint_bytes(&self) -> [u8; 32] {
        let mut sha = Sha256::new();
        match self.version {
            Version::V1 => sha.update(v1::marshal(self, true)),
            Version::V2 => {
                sha.update(&self.raw_details);
                sha.update([self.curve.code()]);
                sha.update(&self.public_key);
                sha.update(&self.signature);
            }
        }
        sha.finalize().into()
    }

    /// O certificado inteiro, com a chave pública (o que vai no arquivo `.crt`).
    pub fn marshal(&self) -> Vec<u8> {
        match self.version {
            Version::V1 => v1::marshal(self, true),
            Version::V2 => v2::marshal(self, true),
        }
    }

    /// O certificado como vai no handshake: sem a chave pública (e, na v2, sem a curva), que o
    /// outro lado tira do Noise. Volta inteiro com [`Certificate::unmarshal`].
    pub fn marshal_for_handshake(&self) -> Vec<u8> {
        match self.version {
            Version::V1 => v1::marshal(self, false),
            Version::V2 => v2::marshal(self, false),
        }
    }

    /// O certificado em PEM.
    pub fn to_pem(&self) -> String {
        let rotulo = match self.version {
            Version::V1 => "NEBULA CERTIFICATE",
            Version::V2 => "NEBULA CERTIFICATE V2",
        };
        pem::encode(rotulo, &self.marshal())
    }

    /// Os bytes que a assinatura da CA cobre.
    fn signed_bytes(&self) -> Vec<u8> {
        match self.version {
            Version::V1 => v1::details(self, true),
            Version::V2 => {
                let mut bytes = self.raw_details.clone();
                bytes.push(self.curve.code());
                bytes.extend_from_slice(&self.public_key);
                bytes
            }
        }
    }

    /// Confere a assinatura com a chave pública da CA.
    pub(crate) fn check_signature(&self, ca_key: &[u8]) -> Result<(), CertError> {
        if self.curve == Curve::P256 {
            return Err(unsupported_p256());
        }
        let falhou = || CertError::SignatureMismatch {
            name: self.name.clone(),
        };
        let chave: [u8; 32] = ca_key.try_into().map_err(|_| falhou())?;
        let chave = ed25519_dalek::VerifyingKey::from_bytes(&chave).map_err(|_| falhou())?;
        let assinatura =
            ed25519_dalek::Signature::from_slice(&self.signature).map_err(|_| falhou())?;
        // O Go confere sem o modo estrito (aceita pontos de ordem pequena): igual aqui.
        ed25519_dalek::Verifier::verify(&chave, &self.signed_bytes(), &assinatura)
            .map_err(|_| falhou())
    }

    /// Junta o certificado a uma chave privada: a pública derivada precisa ser a dele.
    pub fn verify_private_key(&self, chave: &PrivateKey) -> Result<(), CertError> {
        chave.matches(self)
    }
}

/// A mensagem de tudo o que depende da curva P-256.
pub(crate) fn unsupported_p256() -> CertError {
    CertError::Unsupported("certificado P-256 ainda não é suportado".to_owned())
}
