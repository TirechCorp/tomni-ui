//! A conferência do certificado do servidor.
//!
//! Os certificados de servidor do OpenVPN quase nunca têm o nome da máquina (o CN costuma ser
//! `server`), então o verificador padrão do `rustls`, que confere o nome, não serve. O
//! [`ServerVerifier`] faz o que o `.ovpn` pede:
//!
//! 1. a cadeia até uma das CAs do `ca`, na data atual, **sem** conferir o nome da máquina;
//! 2. `remote-cert-tls server`, `remote-cert-ku` e `remote-cert-eku`;
//! 3. `verify-x509-name` (sujeito inteiro, CN igual ou CN com prefixo);
//! 4. `peer-fingerprint` e `verify-hash`.
//!
//! Com `peer-fingerprint` e sem `ca`, a cadeia é dispensada e vale a impressão. Um `.ovpn` que não
//! pede nenhuma conferência é aceito (é o que a pessoa configurou), mas [`ServerVerifier::warnings`]
//! devolve o aviso. As assinaturas do handshake vão para o provedor `ring`.

use std::fmt;
use std::sync::{Arc, Mutex};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{WebPkiSupportedAlgorithms, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::server::ParsedCertificate;
use rustls::{
    CertificateError, DigitallySignedStruct, Error, OtherError, RootCertStore, SignatureScheme,
};

use crate::config::{NameMode, ServerVerify};
use crate::pki::{CertInfo, format_date, hex, inspect};

/// O OID do `serverAuth` no `extendedKeyUsage`.
const OID_SERVER_AUTH: &str = "1.3.6.1.5.5.7.3.1";
/// O OID do `clientAuth` no `extendedKeyUsage`.
const OID_CLIENT_AUTH: &str = "1.3.6.1.5.5.7.3.2";

/// Por que o certificado do servidor foi recusado. A mensagem já está pronta para a pessoa.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyError {
    /// Não há CA nem impressão para conferir o servidor.
    NoTrust,
    /// O certificado não pôde ser lido.
    BadCertificate(String),
    /// A cadeia não chega a nenhuma CA do `.ovpn`.
    UnknownIssuer {
        /// O nome do certificado (CN, ou o sujeito).
        name: String,
    },
    /// O certificado venceu.
    Expired {
        /// O nome do certificado.
        name: String,
        /// Quando venceu (`AAAA-MM-DD`).
        date: String,
    },
    /// O certificado ainda não começou a valer.
    NotYetValid {
        /// O nome do certificado.
        name: String,
        /// Quando começa (`AAAA-MM-DD`).
        date: String,
    },
    /// O certificado (ou a cadeia) usa uma assinatura fraca demais.
    Weak {
        /// O nome do certificado.
        name: String,
        /// O que há de fraco (`SHA1`, `MD5`...).
        algorithm: String,
    },
    /// Outro problema da cadeia.
    Chain {
        /// O nome do certificado.
        name: String,
        /// O que o verificador disse.
        detail: String,
    },
    /// O certificado não é de servidor (`remote-cert-tls server`).
    NotServerCertificate {
        /// O nome do certificado.
        name: String,
    },
    /// O `keyUsage` não está entre os de `remote-cert-ku`.
    KeyUsage {
        /// O nome do certificado.
        name: String,
    },
    /// O `extendedKeyUsage` não tem o valor de `remote-cert-eku`.
    ExtendedKeyUsage {
        /// O nome do certificado.
        name: String,
        /// O que o `.ovpn` exige.
        wanted: String,
    },
    /// O nome não confere com o `verify-x509-name`.
    NameMismatch {
        /// O nome do certificado.
        name: String,
        /// O valor do `.ovpn`.
        wanted: String,
    },
    /// A impressão não está entre as do `.ovpn`.
    FingerprintMismatch {
        /// O nome do certificado.
        name: String,
        /// A impressão SHA-256 do certificado, em hexadecimal.
        fingerprint: String,
    },
}

impl fmt::Display for VerifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoTrust => f.write_str(
                "o .ovpn não tem a CA do servidor (ca) nem a impressão dele (peer-fingerprint)",
            ),
            Self::BadCertificate(m) => write!(f, "o certificado do servidor é inválido: {m}"),
            Self::UnknownIssuer { name } => write!(
                f,
                "o certificado do servidor ({name}) não foi emitido pela CA do .ovpn"
            ),
            Self::Expired { name, date } => {
                write!(f, "o certificado do servidor ({name}) venceu em {date}")
            }
            Self::NotYetValid { name, date } => write!(
                f,
                "o certificado do servidor ({name}) só vale a partir de {date}; confira o relógio do computador"
            ),
            Self::Weak { name, algorithm } => write!(
                f,
                "o certificado {name} usa {algorithm}, recusado por segurança; peça ao administrador um certificado novo"
            ),
            Self::Chain { name, detail } => write!(
                f,
                "o certificado do servidor ({name}) não foi aceito: {detail}"
            ),
            Self::NotServerCertificate { name } => write!(
                f,
                "o certificado do servidor ({name}) não é de servidor (remote-cert-tls server)"
            ),
            Self::KeyUsage { name } => write!(
                f,
                "o certificado do servidor ({name}) não tem o uso de chave pedido em remote-cert-ku"
            ),
            Self::ExtendedKeyUsage { name, wanted } => write!(
                f,
                "o certificado do servidor ({name}) não tem o uso estendido {wanted} (remote-cert-eku)"
            ),
            Self::NameMismatch { name, wanted } => write!(
                f,
                "o certificado do servidor ({name}) não confere com verify-x509-name {wanted}"
            ),
            Self::FingerprintMismatch { name, fingerprint } => write!(
                f,
                "o certificado do servidor ({name}, SHA-256 {fingerprint}) não está entre as impressões aceitas no .ovpn"
            ),
        }
    }
}

impl std::error::Error for VerifyError {}

impl VerifyError {
    /// Recupera o motivo de um erro do `rustls` que veio deste verificador.
    pub fn from_tls(error: &Error) -> Option<VerifyError> {
        match error {
            Error::InvalidCertificate(CertificateError::Other(OtherError(e))) => {
                e.downcast_ref::<VerifyError>().cloned()
            }
            _ => None,
        }
    }

    fn into_tls(self) -> Error {
        Error::InvalidCertificate(CertificateError::Other(OtherError(Arc::new(self))))
    }
}

/// O verificador do certificado do servidor.
pub struct ServerVerifier {
    roots: RootCertStore,
    rules: ServerVerify,
    algs: WebPkiSupportedAlgorithms,
    /// O último certificado de servidor aceito (para o app mostrar).
    seen: Arc<Mutex<Option<CertInfo>>>,
}

impl fmt::Debug for ServerVerifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerVerifier")
            .field("cas", &self.roots.roots.len())
            .field("rules", &self.rules)
            .finish()
    }
}

impl ServerVerifier {
    /// Monta o verificador com as CAs do `.ovpn` e o que ele pede para conferir.
    pub fn new(ca: &[CertificateDer<'static>], rules: &ServerVerify) -> Result<Self, VerifyError> {
        let mut roots = RootCertStore::empty();
        let (_, ignoradas) = roots.add_parsable_certificates(ca.iter().cloned());
        if ignoradas > 0 && roots.is_empty() {
            return Err(VerifyError::BadCertificate(
                "a CA do .ovpn não pôde ser lida".into(),
            ));
        }
        let tem_impressao = !rules.peer_fingerprints.is_empty() || !rules.verify_hash.is_empty();
        if roots.is_empty() && !tem_impressao {
            return Err(VerifyError::NoTrust);
        }
        Ok(ServerVerifier {
            roots,
            rules: rules.clone(),
            algs: rustls::crypto::ring::default_provider().signature_verification_algorithms,
            seen: Arc::new(Mutex::new(None)),
        })
    }

    /// Os avisos para a pessoa (hoje, só o de quando o `.ovpn` não confere o servidor).
    pub fn warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.rules.checks_nothing() {
            out.push(
                "o .ovpn não confere o certificado do servidor (falta remote-cert-tls server); qualquer certificado emitido pela mesma CA é aceito"
                    .to_owned(),
            );
        }
        out
    }

    /// O certificado do servidor aceito na última conexão.
    pub fn server_certificate(&self) -> Option<CertInfo> {
        self.seen.lock().ok().and_then(|g| g.clone())
    }

    /// Um atalho para o mesmo dado, sem precisar do verificador (ele fica dentro do `rustls`).
    pub fn certificate_handle(&self) -> Arc<Mutex<Option<CertInfo>>> {
        self.seen.clone()
    }

    fn check_chain(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        info: &CertInfo,
        now: UnixTime,
    ) -> Result<(), VerifyError> {
        let name = info.display_name();
        let parsed = ParsedCertificate::try_from(end_entity)
            .map_err(|e| VerifyError::BadCertificate(e.to_string()))?;
        match rustls::client::verify_server_cert_signed_by_trust_anchor(
            &parsed,
            &self.roots,
            intermediates,
            now,
            self.algs.all,
        ) {
            Ok(()) => Ok(()),
            Err(Error::InvalidCertificate(CertificateError::UnknownIssuer)) => {
                Err(VerifyError::UnknownIssuer { name })
            }
            Err(Error::InvalidCertificate(CertificateError::Expired))
            | Err(Error::InvalidCertificate(CertificateError::ExpiredContext { .. })) => {
                Err(VerifyError::Expired {
                    name,
                    date: format_date(info.not_after),
                })
            }
            Err(Error::InvalidCertificate(CertificateError::NotValidYet))
            | Err(Error::InvalidCertificate(CertificateError::NotValidYetContext { .. })) => {
                Err(VerifyError::NotYetValid {
                    name,
                    date: format_date(info.not_before),
                })
            }
            Err(e) => match info.weak_signature() {
                Some(algorithm) => Err(VerifyError::Weak {
                    name,
                    algorithm: algorithm.to_owned(),
                }),
                None => Err(VerifyError::Chain {
                    name,
                    detail: e.to_string(),
                }),
            },
        }
    }

    fn check_rules(&self, info: &CertInfo) -> Result<(), VerifyError> {
        let name = info.display_name();
        let r = &self.rules;
        if r.remote_cert_tls && !info.is_server_certificate() {
            return Err(VerifyError::NotServerCertificate { name });
        }
        if !r.remote_cert_ku.is_empty() {
            let mask = info.key_usage.map(|k| k.mask);
            if !mask.is_some_and(|m| r.remote_cert_ku.contains(&m)) {
                return Err(VerifyError::KeyUsage { name });
            }
        }
        if let Some(wanted) = &r.remote_cert_eku {
            let oid = match wanted.as_str() {
                "TLS Web Server Authentication" => OID_SERVER_AUTH,
                "TLS Web Client Authentication" => OID_CLIENT_AUTH,
                outro => outro,
            };
            let tem = info
                .ext_key_usage
                .as_ref()
                .is_some_and(|l| l.iter().any(|o| o == oid));
            if !tem {
                return Err(VerifyError::ExtendedKeyUsage {
                    name,
                    wanted: wanted.clone(),
                });
            }
        }
        if let Some(x) = &r.x509_name {
            let ok = match x.mode {
                NameMode::Subject => info.subject == x.value,
                NameMode::Name => info.common_name.as_deref() == Some(x.value.as_str()),
                NameMode::NamePrefix => info
                    .common_name
                    .as_deref()
                    .is_some_and(|cn| cn.starts_with(x.value.as_str())),
            };
            if !ok {
                return Err(VerifyError::NameMismatch {
                    name,
                    wanted: x.value.clone(),
                });
            }
        }
        if !r.peer_fingerprints.is_empty() || !r.verify_hash.is_empty() {
            let por_impressao = r.peer_fingerprints.contains(&info.sha256);
            let por_hash = r.verify_hash.iter().any(|h| match h.len() {
                20 => h.as_slice() == info.sha1,
                32 => h.as_slice() == info.sha256,
                _ => false,
            });
            if !por_impressao && !por_hash {
                return Err(VerifyError::FingerprintMismatch {
                    name,
                    fingerprint: hex(&info.sha256),
                });
            }
        }
        Ok(())
    }
}

impl ServerCertVerifier for ServerVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        let info = inspect(end_entity)
            .map_err(|e| VerifyError::BadCertificate(e.to_string()).into_tls())?;
        // O nome da máquina não é conferido: o certificado de servidor do OpenVPN raramente o tem.
        if !self.roots.is_empty() {
            self.check_chain(end_entity, intermediates, &info, now)
                .map_err(VerifyError::into_tls)?;
        }
        self.check_rules(&info).map_err(VerifyError::into_tls)?;
        if let Ok(mut visto) = self.seen.lock() {
            *visto = Some(info);
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls12_signature(message, cert, dss, &self.algs)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls13_signature(message, cert, dss, &self.algs)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algs.supported_schemes()
    }
}
