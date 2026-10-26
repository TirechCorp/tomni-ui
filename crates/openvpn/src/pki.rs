//! Os certificados, a chave privada, as chaves estáticas e as conferências do `.ovpn` (PKI).
//!
//! O [`Pki`] guarda o que o `.ovpn` traz de segredo e de confiança: a CA, o certificado e a
//! chave do cliente, a chave estática do `tls-auth`/`tls-crypt` e a do `tls-crypt-v2`. O
//! `Debug` nunca mostra uma chave, e a memória das chaves é apagada ao largar.

use std::fmt;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use rustls::RootCertStore;
use rustls::crypto::ring as ring_provider;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{
    CertificateDer, PrivatePkcs1KeyDer, PrivatePkcs8KeyDer, PrivateSec1KeyDer, UnixTime,
};
use rustls::server::WebPkiClientVerifier;
use sha1::Sha1;
use sha2::{Digest, Sha256};
use x509_cert::Certificate;
use x509_cert::der::oid::ObjectIdentifier;
use x509_cert::der::{Decode, Tag, Tagged};
use x509_cert::ext::pkix::{ExtendedKeyUsage, KeyUsage, KeyUsages};
use x509_cert::name::Name;
use zeroize::Zeroizing;

use crate::static_key::{StaticKey, StaticKeyError};

/// Falta menos que isto para o certificado vencer: vira aviso (plano do M56, seção 4.4).
const EXPIRY_WARNING: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// O tamanho da chave `Kc` do `tls-crypt-v2` (duas metades de 128 bytes: formato do arquivo).
pub const TLS_CRYPT_V2_KC_LEN: usize = 256;
/// O maior `WKc` aceito (limite do protocolo: o `WKc` cabe num pacote de controle).
pub const TLS_CRYPT_V2_MAX_WKC_LEN: usize = 1024;
/// A marca de abertura do arquivo de chave de cliente do `tls-crypt-v2`.
const TCV2_BEGIN: &str = "-----BEGIN OpenVPN tls-crypt-v2 client key-----";
/// A marca de fechamento do arquivo de chave de cliente do `tls-crypt-v2`.
const TCV2_END: &str = "-----END OpenVPN tls-crypt-v2 client key-----";

/// Como o canal de controle é protegido além do TLS.
#[derive(Clone, Default, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)] // um valor só por conexão; boxar mexeria na API sem ganho
pub enum Wrap {
    /// Sem proteção extra.
    #[default]
    None,
    /// `tls-auth`: um HMAC em cada pacote de controle (usa `Pki::static_key`).
    TlsAuth,
    /// `tls-crypt`: cada pacote de controle cifrado e autenticado (usa `Pki::static_key`).
    TlsCrypt,
    /// `tls-crypt-v2`: a chave do cliente e a chave embrulhada que o servidor reconhece.
    TlsCryptV2 {
        /// A chave `Kc` do cliente (256 bytes).
        kc: Zeroizing<[u8; TLS_CRYPT_V2_KC_LEN]>,
        /// A chave `Kc` embrulhada para o servidor (`WKc`).
        wkc: Vec<u8>,
    },
}

impl fmt::Debug for Wrap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Wrap::None => f.write_str("Wrap::None"),
            Wrap::TlsAuth => f.write_str("Wrap::TlsAuth"),
            Wrap::TlsCrypt => f.write_str("Wrap::TlsCrypt"),
            Wrap::TlsCryptV2 { wkc, .. } => {
                write!(f, "Wrap::TlsCryptV2 {{ wkc: {} bytes, .. }}", wkc.len())
            }
        }
    }
}

/// O erro de ler ou montar o PKI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PkiError {
    /// O texto não tem nenhum certificado PEM.
    NoCertificate,
    /// O PEM de certificado está malformado.
    BadPem(String),
    /// O certificado não é um X.509 válido.
    BadCertificate(String),
    /// O texto não tem uma chave privada que o app entenda.
    NoPrivateKey,
    /// A chave privada tem senha (ainda não suportado).
    EncryptedKey,
    /// A chave estática não é válida.
    StaticKey(StaticKeyError),
    /// A chave do `tls-crypt-v2` não é válida.
    BadTlsCryptV2(String),
}

impl fmt::Display for PkiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoCertificate => f.write_str("não há nenhum certificado PEM no texto"),
            Self::BadPem(motivo) => write!(f, "o PEM está malformado: {motivo}"),
            Self::BadCertificate(motivo) => write!(f, "o certificado é inválido: {motivo}"),
            Self::NoPrivateKey => f.write_str(
                "não há chave privada reconhecida no texto (esperado RSA PRIVATE KEY, PRIVATE KEY ou EC PRIVATE KEY)",
            ),
            Self::EncryptedKey => f.write_str(
                "a chave privada tem senha; converta para PEM sem senha (openssl pkey -in chave.pem -out chave-sem-senha.pem)",
            ),
            Self::StaticKey(e) => e.fmt(f),
            Self::BadTlsCryptV2(motivo) => write!(f, "a chave do tls-crypt-v2 é inválida: {motivo}"),
        }
    }
}

impl std::error::Error for PkiError {}

impl From<StaticKeyError> for PkiError {
    fn from(e: StaticKeyError) -> Self {
        PkiError::StaticKey(e)
    }
}

/// O formato em que a chave privada veio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyFormat {
    /// `RSA PRIVATE KEY`.
    Pkcs1,
    /// `PRIVATE KEY`.
    Pkcs8,
    /// `EC PRIVATE KEY`.
    Sec1,
}

/// A chave privada do cliente (DER em memória que se apaga ao largar).
#[derive(Clone, PartialEq, Eq)]
pub struct PrivateKey {
    der: Zeroizing<Vec<u8>>,
    format: KeyFormat,
}

impl fmt::Debug for PrivateKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PrivateKey({:?}, ..)", self.format)
    }
}

impl PrivateKey {
    /// Lê uma chave em PEM: `RSA PRIVATE KEY` (PKCS#1), `PRIVATE KEY` (PKCS#8) ou
    /// `EC PRIVATE KEY` (SEC1). PEM com senha dá [`PkiError::EncryptedKey`].
    pub fn from_pem(text: &str) -> Result<PrivateKey, PkiError> {
        if text.contains("ENCRYPTED PRIVATE KEY") || text.contains("Proc-Type: 4,ENCRYPTED") {
            return Err(PkiError::EncryptedKey);
        }
        let bytes = text.as_bytes();
        if let Ok(k) = PrivatePkcs8KeyDer::from_pem_slice(bytes) {
            return Ok(PrivateKey::new(k.secret_pkcs8_der(), KeyFormat::Pkcs8));
        }
        if let Ok(k) = PrivatePkcs1KeyDer::from_pem_slice(bytes) {
            return Ok(PrivateKey::new(k.secret_pkcs1_der(), KeyFormat::Pkcs1));
        }
        if let Ok(k) = PrivateSec1KeyDer::from_pem_slice(bytes) {
            return Ok(PrivateKey::new(k.secret_sec1_der(), KeyFormat::Sec1));
        }
        Err(PkiError::NoPrivateKey)
    }

    fn new(der: &[u8], format: KeyFormat) -> PrivateKey {
        PrivateKey {
            der: Zeroizing::new(der.to_vec()),
            format,
        }
    }

    /// O formato em que a chave veio.
    pub fn format(&self) -> KeyFormat {
        self.format
    }

    /// A chave como o `rustls` a entende, emprestada (sem outra cópia dos bytes).
    pub fn as_rustls(&self) -> rustls::pki_types::PrivateKeyDer<'_> {
        match self.format {
            KeyFormat::Pkcs1 => PrivatePkcs1KeyDer::from(&self.der[..]).into(),
            KeyFormat::Pkcs8 => PrivatePkcs8KeyDer::from(&self.der[..]).into(),
            KeyFormat::Sec1 => PrivateSec1KeyDer::from(&self.der[..]).into(),
        }
    }

    /// O assinador do provedor `ring` para esta chave.
    pub fn signing_key(&self) -> Result<Arc<dyn rustls::sign::SigningKey>, PkiError> {
        ring_provider::sign::any_supported_type(&self.as_rustls())
            .map_err(|e| PkiError::BadPem(format!("a chave privada não é aceita: {e}")))
    }
}

/// Lê todos os certificados PEM do texto (o `easy-rsa` grava texto antes do PEM; é ignorado).
pub fn parse_certificates(text: &str) -> Result<Vec<CertificateDer<'static>>, PkiError> {
    let mut out = Vec::new();
    for item in CertificateDer::pem_slice_iter(text.as_bytes()) {
        let der = item.map_err(|e| PkiError::BadPem(e.to_string()))?;
        // Falha cedo, com a mensagem, se o DER não for um X.509.
        inspect(&der)?;
        out.push(der);
    }
    if out.is_empty() {
        return Err(PkiError::NoCertificate);
    }
    Ok(out)
}

/// Lê o arquivo de chave de cliente do `tls-crypt-v2`.
pub fn parse_tls_crypt_v2_client(text: &str) -> Result<Wrap, PkiError> {
    let mut inside = false;
    let mut seen_end = false;
    let mut b64 = Zeroizing::new(String::new());
    for line in text.lines() {
        let line = line.trim();
        if !inside {
            inside = line == TCV2_BEGIN;
            continue;
        }
        if line == TCV2_END {
            seen_end = true;
            break;
        }
        b64.push_str(line);
    }
    if !inside || !seen_end {
        return Err(PkiError::BadTlsCryptV2(
            "faltam as marcas -----BEGIN OpenVPN tls-crypt-v2 client key----- e END".into(),
        ));
    }
    let raw = Zeroizing::new(
        STANDARD
            .decode(b64.as_bytes())
            .map_err(|_| PkiError::BadTlsCryptV2("o corpo não é base64".into()))?,
    );
    if raw.len() <= TLS_CRYPT_V2_KC_LEN + 2 {
        return Err(PkiError::BadTlsCryptV2(
            "falta o pedaço WKc depois da chave".into(),
        ));
    }
    let (kc_bytes, wkc) = raw.split_at(TLS_CRYPT_V2_KC_LEN);
    if wkc.len() > TLS_CRYPT_V2_MAX_WKC_LEN {
        return Err(PkiError::BadTlsCryptV2(format!(
            "o WKc tem {} bytes, mais que o máximo de {TLS_CRYPT_V2_MAX_WKC_LEN}",
            wkc.len()
        )));
    }
    // O WKc termina com o próprio tamanho (2 bytes, big-endian).
    let declared = usize::from(u16::from_be_bytes([wkc[wkc.len() - 2], wkc[wkc.len() - 1]]));
    if declared != wkc.len() {
        return Err(PkiError::BadTlsCryptV2(format!(
            "o WKc diz ter {declared} bytes, mas tem {}",
            wkc.len()
        )));
    }
    let mut kc = Zeroizing::new([0u8; TLS_CRYPT_V2_KC_LEN]);
    kc.copy_from_slice(kc_bytes);
    Ok(Wrap::TlsCryptV2 {
        kc,
        wkc: wkc.to_vec(),
    })
}

/// O que o `.ovpn` traz de certificados e chaves.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Pki {
    /// As CAs em que o app confia para o servidor (e para conferir o certificado do cliente).
    pub ca: Vec<CertificateDer<'static>>,
    /// O certificado do cliente, seguido das intermediárias (`extra-certs`).
    pub cert: Option<Vec<CertificateDer<'static>>>,
    /// A chave privada do cliente.
    pub key: Option<PrivateKey>,
    /// A chave estática do `tls-auth` ou do `tls-crypt`.
    pub static_key: Option<StaticKey>,
    /// O `key-direction` do `.ovpn` (0 ou 1; `None` é bidirecional).
    pub key_direction: Option<u8>,
    /// Como o canal de controle é protegido.
    pub wrap: Wrap,
}

impl fmt::Debug for Pki {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pki")
            .field("ca", &self.ca.len())
            .field("cert", &self.cert.as_ref().map(Vec::len))
            .field("key", &self.key)
            .field("static_key", &self.static_key)
            .field("key_direction", &self.key_direction)
            .field("wrap", &self.wrap)
            .finish()
    }
}

/// A gravidade de um [`Problem`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Impede a conexão.
    Error,
    /// A conexão segue, mas a pessoa deve saber.
    Warning,
}

/// Um achado da conferência de um perfil ([`Pki::check`] e `Config::check`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// A gravidade.
    pub severity: Severity,
    /// A mensagem, em português, com o dado que ajuda.
    pub message: String,
}

impl Problem {
    pub(crate) fn error(message: impl Into<String>) -> Problem {
        Problem {
            severity: Severity::Error,
            message: message.into(),
        }
    }

    pub(crate) fn warning(message: impl Into<String>) -> Problem {
        Problem {
            severity: Severity::Warning,
            message: message.into(),
        }
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

/// O que mostrar do certificado do cliente.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PkiSummary {
    /// O CN do certificado do cliente.
    pub client_common_name: Option<String>,
    /// Quando o certificado do cliente vence.
    pub not_after: SystemTime,
    /// A impressão SHA-256 do certificado do cliente, em hexadecimal minúsculo.
    pub fingerprint_sha256: String,
    /// O CN da primeira CA.
    pub ca_common_name: Option<String>,
}

impl Pki {
    /// Confere o PKI e devolve o que está errado ou perto de vencer.
    ///
    /// `agora` é a hora da conferência (os testes passam uma fixa).
    pub fn check(&self, agora: SystemTime) -> Vec<Problem> {
        let mut out = Vec::new();

        if self.ca.is_empty() {
            out.push(Problem::error("falta a CA do servidor (ca)"));
        }
        let mut cas = Vec::new();
        for der in &self.ca {
            match inspect(der) {
                Ok(info) => {
                    check_validity(&info, agora, &mut out);
                    check_weak(&info, false, &mut out);
                    cas.push(der.clone());
                }
                Err(e) => out.push(Problem::error(format!("a CA é inválida: {e}"))),
            }
        }

        match (&self.cert, &self.key) {
            (Some(chain), Some(key)) if !chain.is_empty() => {
                self.check_client(chain, key, agora, &cas, &mut out);
            }
            (None, None) => {}
            (Some(_), None) => out.push(Problem::error("falta a chave privada do cliente (key)")),
            (None, Some(_)) => out.push(Problem::error("falta o certificado do cliente (cert)")),
            (Some(_), Some(_)) => out.push(Problem::error("o certificado do cliente está vazio")),
        }

        match &self.wrap {
            Wrap::None => {}
            Wrap::TlsAuth | Wrap::TlsCrypt => {
                if self.static_key.is_none() {
                    out.push(Problem::error(
                        "falta a chave estática do tls-auth/tls-crypt",
                    ));
                }
            }
            Wrap::TlsCryptV2 { wkc, .. } => {
                if wkc.len() <= 2 {
                    out.push(Problem::error(
                        "a chave do tls-crypt-v2 não tem os dois pedaços (Kc e WKc)",
                    ));
                }
            }
        }
        out
    }

    fn check_client(
        &self,
        chain: &[CertificateDer<'static>],
        key: &PrivateKey,
        agora: SystemTime,
        cas: &[CertificateDer<'static>],
        out: &mut Vec<Problem>,
    ) {
        let leaf = &chain[0];
        let info = match inspect(leaf) {
            Ok(i) => i,
            Err(e) => {
                out.push(Problem::error(format!(
                    "o certificado do cliente é inválido: {e}"
                )));
                return;
            }
        };
        let name = info.display_name();

        match key.signing_key() {
            Ok(signing) => match signing.public_key() {
                Some(spki) => {
                    let same = spki_key_bits(spki.as_ref())
                        .is_some_and(|bits| bits == info.public_key_bits);
                    if !same {
                        out.push(Problem::error(format!(
                            "a chave privada não corresponde ao certificado {name}"
                        )));
                    }
                }
                None => out.push(Problem::warning(
                    "não foi possível conferir a chave privada contra o certificado",
                )),
            },
            Err(e) => out.push(Problem::error(e.to_string())),
        }

        check_validity(&info, agora, out);
        check_weak(&info, true, out);

        if cas.is_empty() {
            return;
        }
        let mut roots = RootCertStore::empty();
        for ca in cas {
            if roots.add(ca.clone()).is_err() {
                out.push(Problem::error(
                    "a CA do .ovpn não pôde ser usada como âncora",
                ));
                return;
            }
        }
        let provider = Arc::new(ring_provider::default_provider());
        let verifier =
            match WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider).build() {
                Ok(v) => v,
                Err(e) => {
                    out.push(Problem::error(format!(
                        "não foi possível montar o verificador: {e}"
                    )));
                    return;
                }
            };
        let now =
            UnixTime::since_unix_epoch(agora.duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO));
        if let Err(e) = verifier.verify_client_cert(leaf, &chain[1..], now) {
            let ja_dito = out
                .iter()
                .any(|p| p.severity == Severity::Error && p.message.contains("venceu"));
            if !ja_dito {
                out.push(Problem::error(describe_chain_error(&name, &e)));
            }
        }
    }

    /// O CN, a validade e a impressão do certificado do cliente, para mostrar na tela.
    pub fn summary(&self) -> Option<PkiSummary> {
        let leaf = self.cert.as_ref()?.first()?;
        let info = inspect(leaf).ok()?;
        let ca_common_name = self
            .ca
            .first()
            .and_then(|c| inspect(c).ok())
            .and_then(|c| c.common_name);
        Some(PkiSummary {
            client_common_name: info.common_name,
            not_after: info.not_after,
            fingerprint_sha256: hex(&info.sha256),
            ca_common_name,
        })
    }
}

fn describe_chain_error(name: &str, e: &rustls::Error) -> String {
    use rustls::CertificateError as C;
    match e {
        rustls::Error::InvalidCertificate(C::UnknownIssuer) => {
            format!("o certificado {name} não foi emitido pela CA do .ovpn")
        }
        rustls::Error::InvalidCertificate(C::UnsupportedSignatureAlgorithmContext { .. })
        | rustls::Error::InvalidCertificate(C::BadSignature) => format!(
            "o certificado {name} tem uma assinatura que o app recusa por segurança; peça ao administrador um certificado novo ({e})"
        ),
        other => format!("o certificado {name} não passou na conferência da cadeia: {other}"),
    }
}

fn check_validity(info: &CertInfo, agora: SystemTime, out: &mut Vec<Problem>) {
    let name = info.display_name();
    if agora >= info.not_after {
        out.push(Problem::error(format!(
            "o certificado {name} venceu em {}",
            format_date(info.not_after)
        )));
    } else if agora < info.not_before {
        out.push(Problem::error(format!(
            "o certificado {name} só vale a partir de {}",
            format_date(info.not_before)
        )));
    } else if info
        .not_after
        .duration_since(agora)
        .unwrap_or(Duration::ZERO)
        <= EXPIRY_WARNING
    {
        out.push(Problem::warning(format!(
            "o certificado {name} vence em {}",
            format_date(info.not_after)
        )));
    }
}

/// `leaf_signature`: confere também o algoritmo da assinatura do certificado (o da raiz
/// autoassinada não importa).
fn check_weak(info: &CertInfo, leaf_signature: bool, out: &mut Vec<Problem>) {
    let name = info.display_name();
    if leaf_signature && let Some(weak) = info.weak_signature() {
        out.push(Problem::error(format!(
            "o certificado {name} usa {weak}, recusado por segurança; peça ao administrador um certificado novo"
        )));
    }
    if let Some(bits) = info.rsa_bits
        && bits < 2048
    {
        out.push(Problem::error(format!(
            "o certificado {name} usa RSA de {bits} bits, recusado por segurança; peça ao administrador um certificado novo"
        )));
    }
}

// --- O certificado por dentro ---------------------------------------------------------------------

const OID_CN: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.4.3");
const OID_SERVER_AUTH: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.6.1.5.5.7.3.1");
const OID_RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.1");
const OID_SHA1_RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.5");
const OID_MD5_RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.4");
const OID_MD2_RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.2");
const OID_ECDSA_SHA1: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.1");
const OID_KEY_USAGE: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.29.15");
const OID_EXT_KEY_USAGE: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.29.37");

/// Os dados do certificado que o app usa.
#[derive(Debug, Clone)]
pub struct CertInfo {
    /// O sujeito no formato do OpenVPN (`C=BR, ST=SP, CN=servidor`).
    pub subject: String,
    /// O emissor no mesmo formato.
    pub issuer: String,
    /// O primeiro CN do sujeito.
    pub common_name: Option<String>,
    /// Início da validade.
    pub not_before: SystemTime,
    /// Fim da validade.
    pub not_after: SystemTime,
    /// A impressão SHA-256 do DER.
    pub sha256: [u8; 32],
    /// A impressão SHA-1 do DER (só para o `verify-hash` antigo).
    pub sha1: [u8; 20],
    /// Os bits da chave pública (o conteúdo do BIT STRING do SPKI).
    pub public_key_bits: Vec<u8>,
    /// O tamanho do módulo, se a chave for RSA.
    pub rsa_bits: Option<usize>,
    /// O OID do algoritmo de assinatura do certificado.
    pub signature_oid: String,
    /// O `keyUsage`, se a extensão existir.
    pub key_usage: Option<KeyUsageBits>,
    /// Os OIDs do `extendedKeyUsage`, se a extensão existir.
    pub ext_key_usage: Option<Vec<String>>,
}

/// Os bits de uso de chave que importam ao app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyUsageBits {
    /// `digitalSignature`.
    pub digital_signature: bool,
    /// `keyEncipherment`.
    pub key_encipherment: bool,
    /// `keyAgreement`.
    pub key_agreement: bool,
    /// Todos os bits do `keyUsage` no formato que o `remote-cert-ku` usa: o bit
    /// `digitalSignature` vale `0x80`, o `nonRepudiation` `0x40`... até o `encipherOnly`
    /// (`0x01`), e o `decipherOnly` vale `0x8000`.
    pub mask: u16,
}

impl CertInfo {
    /// O CN, ou o sujeito inteiro se não houver CN: o nome que as mensagens mostram.
    pub fn display_name(&self) -> String {
        self.common_name
            .clone()
            .unwrap_or_else(|| self.subject.clone())
    }

    /// `Some("SHA1")` ou `Some("MD5")` se a assinatura do certificado for dessas.
    pub fn weak_signature(&self) -> Option<&'static str> {
        let oid = ObjectIdentifier::new(&self.signature_oid).ok()?;
        if oid == OID_SHA1_RSA || oid == OID_ECDSA_SHA1 {
            Some("SHA1")
        } else if oid == OID_MD5_RSA || oid == OID_MD2_RSA {
            Some("MD5")
        } else {
            None
        }
    }

    /// O certificado serve a um servidor TLS, como o `remote-cert-tls server` do OpenVPN
    /// exige: a extensão `keyUsage` existe e permite assinar ou trocar chaves, e o
    /// `extendedKeyUsage` tem `serverAuth`.
    pub fn is_server_certificate(&self) -> bool {
        let ku_ok = self
            .key_usage
            .is_some_and(|k| k.digital_signature || k.key_encipherment || k.key_agreement);
        let eku_ok = self
            .ext_key_usage
            .as_ref()
            .is_some_and(|l| l.iter().any(|o| *o == OID_SERVER_AUTH.to_string()));
        ku_ok && eku_ok
    }
}

/// Lê um certificado DER.
pub fn inspect(der: &CertificateDer<'_>) -> Result<CertInfo, PkiError> {
    let cert =
        Certificate::from_der(der.as_ref()).map_err(|e| PkiError::BadCertificate(e.to_string()))?;
    let tbs = &cert.tbs_certificate;
    let to_time = |t: x509_cert::time::Time| UNIX_EPOCH + t.to_unix_duration();
    let public_key_bits = tbs
        .subject_public_key_info
        .subject_public_key
        .raw_bytes()
        .to_vec();
    let rsa_bits = (tbs.subject_public_key_info.algorithm.oid == OID_RSA)
        .then(|| rsa_modulus_bits(&public_key_bits))
        .flatten();

    let mut key_usage = None;
    let mut ext_key_usage = None;
    for ext in tbs.extensions.iter().flatten() {
        if ext.extn_id == OID_KEY_USAGE {
            if let Ok(ku) = KeyUsage::from_der(ext.extn_value.as_bytes()) {
                key_usage = Some(KeyUsageBits {
                    digital_signature: ku.0.contains(KeyUsages::DigitalSignature),
                    key_encipherment: ku.0.contains(KeyUsages::KeyEncipherment),
                    key_agreement: ku.0.contains(KeyUsages::KeyAgreement),
                    mask: key_usage_mask(ku.0.bits()),
                });
            }
        } else if ext.extn_id == OID_EXT_KEY_USAGE
            && let Ok(eku) = ExtendedKeyUsage::from_der(ext.extn_value.as_bytes())
        {
            ext_key_usage = Some(eku.0.iter().map(ToString::to_string).collect());
        }
    }

    Ok(CertInfo {
        subject: format_name(&tbs.subject),
        issuer: format_name(&tbs.issuer),
        common_name: common_name(&tbs.subject),
        not_before: to_time(tbs.validity.not_before),
        not_after: to_time(tbs.validity.not_after),
        sha256: Sha256::digest(der.as_ref()).into(),
        sha1: Sha1::digest(der.as_ref()).into(),
        public_key_bits,
        rsa_bits,
        signature_oid: cert.signature_algorithm.oid.to_string(),
        key_usage,
        ext_key_usage,
    })
}

/// Converte os bits do `keyUsage` (o bit 0 é `digitalSignature`) para a máscara do
/// `remote-cert-ku` (o bit mais alto do primeiro byte é `digitalSignature`).
fn key_usage_mask(bits: u16) -> u16 {
    let mut mask = 0u16;
    for i in 0..8 {
        if bits & (1 << i) != 0 {
            mask |= 0x80 >> i;
        }
    }
    if bits & (1 << 8) != 0 {
        mask |= 0x8000;
    }
    mask
}

/// O conteúdo do BIT STRING de um SPKI em DER.
fn spki_key_bits(spki: &[u8]) -> Option<Vec<u8>> {
    let info = x509_cert::spki::SubjectPublicKeyInfoOwned::from_der(spki).ok()?;
    Some(info.subject_public_key.raw_bytes().to_vec())
}

/// Os bits do módulo de um `RSAPublicKey` (SEQUENCE { INTEGER n, INTEGER e }).
fn rsa_modulus_bits(rsa_public_key: &[u8]) -> Option<usize> {
    fn read_len(data: &[u8]) -> Option<(usize, &[u8])> {
        let (&first, rest) = data.split_first()?;
        if first < 0x80 {
            return Some((usize::from(first), rest));
        }
        let n = usize::from(first & 0x7f);
        if n == 0 || n > 4 || rest.len() < n {
            return None;
        }
        let len = rest[..n]
            .iter()
            .fold(0usize, |acc, b| (acc << 8) | usize::from(*b));
        Some((len, &rest[n..]))
    }
    let (&tag, rest) = rsa_public_key.split_first()?;
    if tag != 0x30 {
        return None;
    }
    let (_, rest) = read_len(rest)?;
    let (&tag, rest) = rest.split_first()?;
    if tag != 0x02 {
        return None;
    }
    let (len, rest) = read_len(rest)?;
    let modulus = rest.get(..len)?;
    let trimmed = modulus
        .iter()
        .position(|b| *b != 0)
        .map(|i| &modulus[i..])?;
    let first = *trimmed.first()?;
    Some((trimmed.len() - 1) * 8 + (8 - first.leading_zeros() as usize))
}

/// O primeiro CN do nome.
fn common_name(name: &Name) -> Option<String> {
    name.0
        .iter()
        .flat_map(|rdn| rdn.0.iter())
        .find(|atv| atv.oid == OID_CN)
        .map(|atv| attribute_text(&atv.value))
}

/// Formata o nome como o OpenVPN: campos na ordem do certificado, nomes curtos, `=` sem
/// espaços, `, ` entre os campos, `+` entre valores do mesmo campo, controles escapados.
pub fn format_name(name: &Name) -> String {
    let mut out = String::new();
    for (i, rdn) in name.0.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        for (j, atv) in rdn.0.iter().enumerate() {
            if j > 0 {
                out.push('+');
            }
            out.push_str(&short_name(&atv.oid));
            out.push('=');
            for c in attribute_text(&atv.value).chars() {
                if c.is_control() {
                    // O formato do OpenSSL para um controle: barra e o byte em hexadecimal.
                    out.push_str(&format!("\\{:02X}", u32::from(c)));
                } else {
                    out.push(c);
                }
            }
        }
    }
    out
}

fn attribute_text(value: &x509_cert::der::Any) -> String {
    match value.tag() {
        Tag::BmpString => {
            let units: Vec<u16> = value
                .value()
                .chunks_exact(2)
                .map(|p| u16::from_be_bytes([p[0], p[1]]))
                .collect();
            String::from_utf16_lossy(&units)
        }
        _ => String::from_utf8_lossy(value.value()).into_owned(),
    }
}

fn short_name(oid: &ObjectIdentifier) -> String {
    // Os nomes curtos do OpenSSL (`sn`) dos campos de nome mais comuns.
    let name = match oid.to_string().as_str() {
        "2.5.4.3" => "CN",
        "2.5.4.4" => "SN",
        "2.5.4.5" => "serialNumber",
        "2.5.4.6" => "C",
        "2.5.4.7" => "L",
        "2.5.4.8" => "ST",
        "2.5.4.9" => "street",
        "2.5.4.10" => "O",
        "2.5.4.11" => "OU",
        "2.5.4.12" => "title",
        "2.5.4.17" => "postalCode",
        "2.5.4.41" => "name",
        "2.5.4.42" => "GN",
        "2.5.4.43" => "initials",
        "2.5.4.46" => "dnQualifier",
        "1.2.840.113549.1.9.1" => "emailAddress",
        "0.9.2342.19200300.100.1.1" => "UID",
        "0.9.2342.19200300.100.1.25" => "DC",
        _ => return oid.to_string(),
    };
    name.to_string()
}

/// Hexadecimal minúsculo.
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from(DIGITS[usize::from(b >> 4)]));
        s.push(char::from(DIGITS[usize::from(b & 0x0f)]));
    }
    s
}

/// A data como `AAAA-MM-DD` (UTC).
pub fn format_date(t: SystemTime) -> String {
    let secs = match t.duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_secs()).unwrap_or(i64::MAX / 2),
        Err(e) => -i64::try_from(e.duration().as_secs()).unwrap_or(i64::MAX / 2),
    };
    let days = secs.div_euclid(86_400);
    // Dias desde 1970-01-01 para ano, mês e dia (calendário gregoriano proléptico).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datas() {
        assert_eq!(format_date(UNIX_EPOCH), "1970-01-01");
        assert_eq!(
            format_date(UNIX_EPOCH + Duration::from_secs(1_580_515_200)),
            "2020-02-01"
        );
        assert_eq!(
            format_date(UNIX_EPOCH + Duration::from_secs(951_782_400)),
            "2000-02-29"
        );
    }

    #[test]
    fn bits_do_modulo_rsa() {
        // SEQUENCE { INTEGER 0x00C5 (módulo de 8 bits com o zero à frente), INTEGER 3 }
        let der = [0x30, 0x08, 0x02, 0x02, 0x00, 0xc5, 0x02, 0x02, 0x01, 0x03];
        assert_eq!(rsa_modulus_bits(&der), Some(8));
    }

    #[test]
    fn hexadecimal() {
        assert_eq!(hex(&[0, 0xab, 0x10]), "00ab10");
    }
}
