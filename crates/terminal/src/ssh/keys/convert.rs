//! Os formatos das chaves: gravar a privada em OpenSSH, PKCS#8, PEM tradicional e `.ppk`, e
//! ler e gravar a pública em OpenSSH, RFC 4716, SPKI e PKCS#1.
//!
//! O OpenSSH e o PKCS#8 saem do `ssh-key` e do `russh`; o PPK é de `ppk.rs`. O PEM tradicional do
//! RSA vem do crate `rsa` (o PKCS#1 privado pede `dmp1` e `dmq1`, que são contas de inteiros
//! grandes). O resto é DER curto, escrito e lido aqui à mão: o SEC1 das chaves EC, o SPKI (Ed25519,
//! EC e RSA) e o PKCS#1 público. Assim não entra um crate por curva só para montar uns bytes.

use std::fs;
use std::path::Path;

use russh::keys::{decode_secret_key, encode_pkcs8_pem, encode_pkcs8_pem_encrypted};
use ssh_key::encoding::base64::{Base64, Encoding};
use ssh_key::private::KeypairData;
use ssh_key::public::{EcdsaPublicKey, Ed25519PublicKey, KeyData, RsaPublicKey};
use ssh_key::{Algorithm, EcdsaCurve, LineEnding, Mpint, PrivateKey, PublicKey};
use zeroize::Zeroizing;

use super::{
    KdfLevel, KeyError, KeyFile, KeyFormat, KeyInfo, ensure_parent, inspect_ppk, kind_and_bits,
    ppk, protect, write_file,
};

/// O formato de uma chave privada ao gravar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrivateFormat {
    /// `-----BEGIN OPENSSH PRIVATE KEY-----`: o do `ssh-keygen` hoje. Com ou sem passphrase.
    OpenSsh,
    /// `-----BEGIN PRIVATE KEY-----` ou `ENCRYPTED PRIVATE KEY` (PBES2, PBKDF2-SHA256,
    /// AES-256-CBC): Java, .NET, OpenSSL 3, Go.
    Pkcs8,
    /// `RSA PRIVATE KEY` (PKCS#1) ou `EC PRIVATE KEY` (SEC1), sem cifra: o "PEM" que AWS,
    /// Paramiko antigo e o `ssh-keygen -m PEM` usam. Não existe para Ed25519.
    PemLegacy,
    /// `PuTTY-User-Key-File-3`, com Argon2id: PuTTY 0.75 ou mais novo, WinSCP 5.20 ou mais novo.
    PpkV3,
    /// `PuTTY-User-Key-File-2`: PuTTY e WinSCP antigos, FileZilla antigo.
    PpkV2,
}

impl PrivateFormat {
    /// Todos os formatos, na ordem em que a tela os lista.
    pub const ALL: [PrivateFormat; 5] = [
        PrivateFormat::OpenSsh,
        PrivateFormat::Pkcs8,
        PrivateFormat::PemLegacy,
        PrivateFormat::PpkV3,
        PrivateFormat::PpkV2,
    ];

    /// Os formatos que servem para a chave: tira o `PemLegacy` do Ed25519 e deixa só o `OpenSsh`
    /// para as `sk-` (a privada delas é só um "key handle") e para o DSA.
    pub fn available(key: &PrivateKey) -> Vec<PrivateFormat> {
        match key.algorithm() {
            Algorithm::Ed25519 => vec![Self::OpenSsh, Self::Pkcs8, Self::PpkV3, Self::PpkV2],
            Algorithm::Rsa { .. } | Algorithm::Ecdsa { .. } => Self::ALL.to_vec(),
            _ => vec![Self::OpenSsh],
        }
    }

    /// Se o formato aceita passphrase (o `PemLegacy` não).
    pub fn encrypts(self) -> bool {
        self != Self::PemLegacy
    }

    /// O nome na tela.
    pub fn label(self) -> &'static str {
        match self {
            Self::OpenSsh => "OpenSSH (padrão do Linux e do macOS)",
            Self::Pkcs8 => "PKCS#8 (Java, .NET, OpenSSL)",
            Self::PemLegacy => "PEM tradicional (AWS, Paramiko)",
            Self::PpkV3 => "PuTTY (.ppk) versão 3",
            Self::PpkV2 => "PuTTY (.ppk) versão 2 (programas antigos)",
        }
    }

    /// A extensão sugerida: vazia (OpenSSH), `.pem` ou `.ppk`.
    pub fn extension(self) -> &'static str {
        match self {
            Self::OpenSsh => "",
            Self::Pkcs8 | Self::PemLegacy => ".pem",
            Self::PpkV3 | Self::PpkV2 => ".ppk",
        }
    }

    /// O nome curto, para as mensagens de erro.
    fn short(self) -> &'static str {
        match self {
            Self::OpenSsh => "OpenSSH",
            Self::Pkcs8 => "PKCS#8",
            Self::PemLegacy => "PEM tradicional",
            Self::PpkV3 | Self::PpkV2 => "PuTTY (.ppk)",
        }
    }
}

/// O formato de uma chave pública ao gravar ou ler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicFormat {
    /// `ssh-ed25519 AAAA… comentário`: `authorized_keys`, GitHub, GitLab.
    OpenSsh,
    /// `---- BEGIN SSH2 PUBLIC KEY ----` (RFC 4716): o "public" do `puttygen`, ssh.com,
    /// `ssh-keygen -e`.
    Rfc4716,
    /// `-----BEGIN PUBLIC KEY-----` (SubjectPublicKeyInfo): `ssh-keygen -e -m PKCS8`, OpenSSL.
    SpkiPem,
    /// `-----BEGIN RSA PUBLIC KEY-----`: `ssh-keygen -e -m PEM`, só RSA.
    Pkcs1Pem,
}

impl PublicFormat {
    /// Todos os formatos, na ordem em que a tela os lista.
    pub const ALL: [PublicFormat; 4] = [
        PublicFormat::OpenSsh,
        PublicFormat::Rfc4716,
        PublicFormat::SpkiPem,
        PublicFormat::Pkcs1Pem,
    ];

    /// Os formatos que servem para a chave (o PKCS#1 é só do RSA; o SPKI não existe para as
    /// `sk-`).
    pub fn available(public: &PublicKey) -> Vec<PublicFormat> {
        match public.key_data() {
            KeyData::Rsa(_) => Self::ALL.to_vec(),
            KeyData::Ed25519(_) | KeyData::Ecdsa(_) => {
                vec![Self::OpenSsh, Self::Rfc4716, Self::SpkiPem]
            }
            _ => vec![Self::OpenSsh, Self::Rfc4716],
        }
    }

    /// O nome na tela.
    pub fn label(self) -> &'static str {
        match self {
            Self::OpenSsh => "OpenSSH (authorized_keys, GitHub)",
            Self::Rfc4716 => "RFC 4716 (PuTTY, ssh.com)",
            Self::SpkiPem => "PEM / SPKI (OpenSSL)",
            Self::Pkcs1Pem => "PEM RSA (PKCS#1)",
        }
    }

    /// A extensão sugerida.
    pub fn extension(self) -> &'static str {
        match self {
            Self::OpenSsh => ".pub",
            Self::Rfc4716 => ".pub",
            Self::SpkiPem | Self::Pkcs1Pem => ".pem",
        }
    }
}

impl KeyFile {
    /// Como `open`, a partir do texto (o que se cola ou o que está no cofre). Detecta também o
    /// formato de uma pública e devolve [`KeyError::OnlyPublic`] em vez de "não reconheço".
    pub fn from_text(text: &str, passphrase: Option<&str>) -> Result<KeyFile, KeyError> {
        let info = Self::inspect_text(text)?;
        let passphrase = passphrase.filter(|p| !p.is_empty());
        if info.encrypted && passphrase.is_none() {
            return Err(KeyError::NeedsPassphrase);
        }
        let mut key = decode_secret_key(text, passphrase).map_err(|error| {
            if info.encrypted {
                KeyError::WrongPassphrase
            } else {
                KeyError::Invalid(error.to_string())
            }
        })?;
        if key.comment().is_empty() && !info.comment.is_empty() {
            key.set_comment(info.comment.clone());
        }
        Ok(KeyFile { key, info })
    }

    /// Como `inspect`, a partir do texto: o formato e se tem passphrase, sem pedi-la. O
    /// `path` do resultado fica vazio.
    pub fn inspect_text(text: &str) -> Result<KeyInfo, KeyError> {
        let trimmed = text.trim_start();
        let (format, encrypted, public, comment) = if trimmed.starts_with("PuTTY-User-Key-File-") {
            let (public, encrypted, comment) = inspect_ppk(text)?;
            let version = trimmed
                .strip_prefix("PuTTY-User-Key-File-")
                .and_then(|rest| rest.split(':').next());
            let format = if version == Some("2") {
                KeyFormat::PpkV2
            } else {
                KeyFormat::PpkV3
            };
            (format, encrypted, Some(public), comment)
        } else if text.contains("-----BEGIN OPENSSH PRIVATE KEY-----") {
            let key = PrivateKey::from_openssh(text)?;
            let comment = key.comment().to_string();
            (
                KeyFormat::OpenSsh,
                key.is_encrypted(),
                Some(key.public_key().clone()),
                comment,
            )
        } else if text.contains("PRIVATE KEY-----") {
            let encrypted = text.contains("ENCRYPTED") || text.contains("DEK-Info:");
            let format = if text.contains("-----BEGIN PRIVATE KEY-----")
                || text.contains("-----BEGIN ENCRYPTED PRIVATE KEY-----")
            {
                KeyFormat::Pkcs8
            } else {
                KeyFormat::Pem
            };
            if encrypted {
                (format, true, None, String::new())
            } else {
                let key = decode_secret_key(text, None)
                    .map_err(|error| KeyError::Invalid(error.to_string()))?;
                let comment = key.comment().to_string();
                (format, false, Some(key.public_key().clone()), comment)
            }
        } else if parse_public(text).is_ok() {
            return Err(KeyError::OnlyPublic);
        } else {
            return Err(KeyError::Invalid("não parece uma chave privada".to_owned()));
        };
        let (kind, bits) = public.as_ref().map(kind_and_bits).unwrap_or_default();
        Ok(KeyInfo {
            path: Default::default(),
            format,
            kind,
            bits,
            comment,
            encrypted,
            public,
        })
    }
}

/// A privada no formato pedido, pronta para gravar. `passphrase` vazia ou `None` grava sem
/// cifra. `kdf` vale para a passphrase: as rodadas do `bcrypt-pbkdf` do OpenSSH, e no PKCS#8 as
/// do PBKDF2 (6 250 por rodada do `bcrypt`, de modo que o padrão dá 100 000). O PPK usa os
/// parâmetros fixos do `puttygen`.
///
/// A chave precisa estar aberta (sem a passphrase de arquivo). Só o OpenSSH guarda o comentário
/// no arquivo, com o PPK; o PKCS#8 e o PEM não têm onde.
pub fn export_private(
    key: &PrivateKey,
    format: PrivateFormat,
    passphrase: Option<&str>,
    kdf: KdfLevel,
) -> Result<Zeroizing<String>, KeyError> {
    let passphrase = passphrase.filter(|p| !p.is_empty());
    if !PrivateFormat::available(key).contains(&format) {
        return Err(KeyError::FormatNotSupported {
            format: format.short(),
            kind: kind_and_bits(key.public_key()).0,
        });
    }
    if passphrase.is_some() && !format.encrypts() {
        return Err(KeyError::PassphraseNotSupported);
    }
    match format {
        PrivateFormat::OpenSsh => {
            let stored = protect(key, passphrase.unwrap_or_default(), kdf)?;
            Ok(stored.to_openssh(LineEnding::LF)?)
        }
        PrivateFormat::Pkcs8 if matches!(key.key_data(), KeypairData::Ed25519(_)) => {
            pkcs8_ed25519(key, passphrase, kdf)
        }
        PrivateFormat::Pkcs8 => {
            let mut out = Zeroizing::new(Vec::new());
            let result = match passphrase {
                None => encode_pkcs8_pem(key, &mut *out),
                Some(pass) => {
                    if kdf.rounds() == 0 {
                        return Err(KeyError::Unsupported("zero rodadas".to_owned()));
                    }
                    let rounds = kdf.rounds().saturating_mul(6_250);
                    encode_pkcs8_pem_encrypted(key, pass.as_bytes(), rounds, &mut *out)
                }
            };
            result.map_err(|error| KeyError::Unsupported(format!("o PKCS#8: {error}")))?;
            // O `russh` quebra o base64 em 76 colunas; o PEM de verdade tem 64.
            let text = String::from_utf8_lossy(&out);
            let (label, der) = pem_decode(&text)
                .ok_or_else(|| KeyError::Invalid("o PKCS#8 saiu torto".to_owned()))?;
            Ok(Zeroizing::new(pem_encode(&label, &der)))
        }
        PrivateFormat::PemLegacy => match key.key_data() {
            KeypairData::Rsa(pair) => {
                use rsa::pkcs1::EncodeRsaPrivateKey;
                let rsa = rsa::RsaPrivateKey::try_from(pair)
                    .map_err(|error| KeyError::Invalid(error.to_string()))?;
                let der = rsa
                    .to_pkcs1_der()
                    .map_err(|error| KeyError::Invalid(error.to_string()))?;
                Ok(Zeroizing::new(pem_encode(
                    "RSA PRIVATE KEY",
                    der.as_bytes(),
                )))
            }
            KeypairData::Ecdsa(pair) => {
                let curve = pair.curve();
                let mut point = vec![0u8];
                point.extend_from_slice(pair.public_key_bytes());
                let der = Zeroizing::new(der_seq(&[
                    der_uint(&[1]),
                    der(0x04, pair.private_key_bytes()),
                    der(0xA0, &curve_oid(curve)),
                    der(0xA1, &der(0x03, &point)),
                ]));
                Ok(Zeroizing::new(pem_encode("EC PRIVATE KEY", &der)))
            }
            _ => Err(KeyError::FormatNotSupported {
                format: format.short(),
                kind: kind_and_bits(key.public_key()).0,
            }),
        },
        PrivateFormat::PpkV3 => ppk::write(key, 3, passphrase),
        PrivateFormat::PpkV2 => ppk::write(key, 2, passphrase),
    }
}

/// O PKCS#8 de uma Ed25519, na versão 1 (RFC 5208), sem a pública dentro. O `russh` grava a
/// versão 2 (RFC 5958, com a pública), que o OpenSSL 3.0 e o `ssh-keygen` não leem; a mão é mais
/// curta que consertar isso lá. Com `passphrase`, cifra no PBES2 (PBKDF2-HMAC-SHA256 e
/// AES-256-CBC), o mesmo do `openssl pkcs8 -topk8` e do restante do PKCS#8 cifrado daqui.
fn pkcs8_ed25519(
    key: &PrivateKey,
    passphrase: Option<&str>,
    kdf: KdfLevel,
) -> Result<Zeroizing<String>, KeyError> {
    use hmac::digest::KeyInit;
    use hmac::{Hmac, Mac};
    use rand::RngExt;

    let KeypairData::Ed25519(pair) = key.key_data() else {
        unreachable!("só chamado para Ed25519");
    };
    let seed = Zeroizing::new(pair.private.to_bytes());
    let plain = Zeroizing::new(der_seq(&[
        der_uint(&[0]),
        der_seq(&[der(0x06, OID_ED25519)]),
        der(0x04, &der(0x04, &*seed)),
    ]));
    let Some(pass) = passphrase else {
        return Ok(Zeroizing::new(pem_encode("PRIVATE KEY", &plain)));
    };
    if kdf.rounds() == 0 {
        return Err(KeyError::Unsupported("zero rodadas".to_owned()));
    }
    let rounds = kdf.rounds().saturating_mul(6_250);
    let mut rng = rand::rng();
    let mut salt = [0u8; 32];
    let mut iv = [0u8; 16];
    rng.fill(&mut salt[..]);
    rng.fill(&mut iv[..]);

    // PBKDF2-HMAC-SHA256 de um bloco só (a chave do AES-256 tem 32 bytes).
    let hmac = Hmac::<sha2::Sha256>::new_from_slice(pass.as_bytes())
        .map_err(|_| KeyError::Unsupported("a chave do PBKDF2".to_owned()))?;
    let mut block = hmac.clone();
    block.update(&salt);
    block.update(&1u32.to_be_bytes());
    let mut u = block.finalize().into_bytes();
    let mut derived = Zeroizing::new([0u8; 32]);
    derived.copy_from_slice(&u);
    for _ in 1..rounds {
        let mut round = hmac.clone();
        round.update(&u);
        u = round.finalize().into_bytes();
        for (out, byte) in derived.iter_mut().zip(u.iter()) {
            *out ^= byte;
        }
    }

    // Preenchimento PKCS#7.
    let mut buf = Zeroizing::new(plain.to_vec());
    let pad = 16 - buf.len() % 16;
    buf.extend(std::iter::repeat_n(pad as u8, pad));
    ssh_key::Cipher::Aes256Cbc
        .encrypt(&*derived, &iv, &mut buf)
        .map_err(|error| KeyError::Unsupported(format!("o AES-CBC: {error}")))?;

    const OID_PBES2: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x05, 0x0D];
    const OID_PBKDF2: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x05, 0x0C];
    const OID_HMAC_SHA256: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x02, 0x09];
    const OID_AES256_CBC: &[u8] = &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x01, 0x2A];
    let encrypted = der_seq(&[
        der_seq(&[
            der(0x06, OID_PBES2),
            der_seq(&[
                der_seq(&[
                    der(0x06, OID_PBKDF2),
                    der_seq(&[
                        der(0x04, &salt),
                        der_uint(&rounds.to_be_bytes()),
                        der_seq(&[der(0x06, OID_HMAC_SHA256), vec![0x05, 0x00]]),
                    ]),
                ]),
                der_seq(&[der(0x06, OID_AES256_CBC), der(0x04, &iv)]),
            ]),
        ]),
        der(0x04, &buf),
    ]);
    Ok(Zeroizing::new(pem_encode(
        "ENCRYPTED PRIVATE KEY",
        &encrypted,
    )))
}

/// A pública no formato pedido. O comentário vai na linha OpenSSH e no `Comment:` do RFC 4716;
/// o SPKI e o PKCS#1 não têm onde guardá-lo.
pub fn export_public(
    public: &PublicKey,
    comment: &str,
    format: PublicFormat,
) -> Result<String, KeyError> {
    let unsupported = |format: &'static str| KeyError::FormatNotSupported {
        format,
        kind: kind_and_bits(public).0,
    };
    match format {
        PublicFormat::OpenSsh => {
            let mut public = public.clone();
            public.set_comment(comment);
            Ok(public.to_openssh()?)
        }
        PublicFormat::Rfc4716 => {
            let blob = public.to_bytes()?;
            let mut out = String::from("---- BEGIN SSH2 PUBLIC KEY ----\n");
            if !comment.is_empty() {
                let header = format!("Comment: \"{}\"", comment.replace(['\r', '\n', '"'], " "));
                // As linhas do cabeçalho têm no máximo 72 caracteres; o resto continua com `\`.
                let chars: Vec<char> = header.chars().collect();
                let chunks: Vec<String> = chars.chunks(71).map(|c| c.iter().collect()).collect();
                for (index, chunk) in chunks.iter().enumerate() {
                    out.push_str(chunk);
                    out.push_str(if index + 1 < chunks.len() {
                        "\\\n"
                    } else {
                        "\n"
                    });
                }
            }
            let encoded = Base64::encode_string(&blob);
            for chunk in encoded.as_bytes().chunks(70) {
                out.push_str(std::str::from_utf8(chunk).unwrap_or_default());
                out.push('\n');
            }
            out.push_str("---- END SSH2 PUBLIC KEY ----\n");
            Ok(out)
        }
        PublicFormat::SpkiPem => {
            let der = spki_der(public).ok_or_else(|| unsupported("PEM / SPKI"))?;
            Ok(pem_encode("PUBLIC KEY", &der))
        }
        PublicFormat::Pkcs1Pem => match public.key_data() {
            KeyData::Rsa(rsa) => Ok(pem_encode("RSA PUBLIC KEY", &pkcs1_public_der(rsa))),
            _ => Err(unsupported("PEM RSA (PKCS#1)")),
        },
    }
}

/// Lê uma pública em qualquer um dos quatro formatos e devolve a chave e o comentário. A linha
/// OpenSSH pode vir com as opções do `authorized_keys` na frente (`from="…" ssh-ed25519 …`).
pub fn parse_public(text: &str) -> Result<(PublicKey, String), KeyError> {
    let invalid = || KeyError::Invalid("não parece uma chave pública".to_owned());
    let trimmed = text.trim();
    if trimmed.contains("---- BEGIN SSH2 PUBLIC KEY ----") {
        return parse_rfc4716(trimmed).ok_or_else(invalid);
    }
    if let Some((label, der)) = pem_decode(trimmed) {
        let key = match label.as_str() {
            "PUBLIC KEY" => parse_spki(&der),
            "RSA PUBLIC KEY" => parse_pkcs1_public(&der),
            _ => None,
        };
        return key.map(|key| (key, String::new())).ok_or_else(invalid);
    }
    for line in trimmed.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let tokens = tokenize(line);
        for start in 0..tokens.len() {
            if let Ok(key) = PublicKey::from_openssh(&tokens[start..].join(" ")) {
                let comment = key.comment().to_string();
                return Ok((key, comment));
            }
        }
    }
    Err(invalid())
}

/// Grava `text` em `path`: atômico (um `.tmp` ao lado e `rename`), 0600 no Unix se for privada
/// (0644 se não) e cria a pasta se faltar. Sem `overwrite`, um arquivo que já existe é um
/// [`KeyError::Exists`]; com ele, o arquivo antigo só some quando o novo está inteiro.
pub fn write_key_text(
    path: &Path,
    text: &str,
    private: bool,
    overwrite: bool,
) -> Result<(), KeyError> {
    if !overwrite && path.exists() {
        return Err(KeyError::Exists(path.to_owned()));
    }
    ensure_parent(path)?;
    let temp = {
        let mut name = path.as_os_str().to_owned();
        name.push(".tmp");
        std::path::PathBuf::from(name)
    };
    write_file(
        &temp,
        text.as_bytes(),
        if private { 0o600 } else { 0o644 },
        true,
    )?;
    if let Err(error) = fs::rename(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(error.into());
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// RFC 4716

fn parse_rfc4716(text: &str) -> Option<(PublicKey, String)> {
    let mut lines = text.lines().map(str::trim);
    lines.find(|line| *line == "---- BEGIN SSH2 PUBLIC KEY ----")?;
    let (mut comment, mut subject) = (String::new(), String::new());
    let mut body = String::new();
    let mut pending: Option<String> = None;
    for line in lines {
        if line == "---- END SSH2 PUBLIC KEY ----" {
            break;
        }
        // Cabeçalho `Nome: valor`, que pode continuar na linha seguinte se terminar em `\`.
        let header = match pending.take() {
            Some(mut head) => {
                head.push_str(line.strip_suffix('\\').unwrap_or(line));
                if line.ends_with('\\') {
                    pending = Some(head);
                    continue;
                }
                Some(head)
            }
            None if body.is_empty() && line.contains(':') => {
                if let Some(rest) = line.strip_suffix('\\') {
                    pending = Some(rest.to_owned());
                    continue;
                }
                Some(line.to_owned())
            }
            None => None,
        };
        match header {
            Some(header) => {
                let (name, value) = header.split_once(':')?;
                let value = value.trim();
                let value = value
                    .strip_prefix('"')
                    .and_then(|v| v.strip_suffix('"'))
                    .unwrap_or(value);
                match name.trim().to_ascii_lowercase().as_str() {
                    "comment" => comment = value.to_owned(),
                    "subject" => subject = value.to_owned(),
                    _ => {}
                }
            }
            None => body.push_str(line),
        }
    }
    let blob = Base64::decode_vec(&body).ok()?;
    let mut key = PublicKey::from_bytes(&blob).ok()?;
    let comment = if comment.is_empty() { subject } else { comment };
    key.set_comment(comment.clone());
    Some((key, comment))
}

/// Separa a linha em palavras, mantendo juntas as que estão entre aspas (as opções do
/// `authorized_keys`, como `command="ssh -v"`).
fn tokenize(line: &str) -> Vec<String> {
    let (mut tokens, mut current, mut quoted) = (Vec::new(), String::new(), false);
    for ch in line.chars() {
        match ch {
            '"' => {
                quoted = !quoted;
                current.push(ch);
            }
            ch if ch.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            ch => current.push(ch),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

// ---------------------------------------------------------------------------------------------
// SPKI e PKCS#1

/// Os bytes do OID de cada curva (sem o tag e o tamanho).
fn curve_oid(curve: EcdsaCurve) -> Vec<u8> {
    let bytes: &[u8] = match curve {
        EcdsaCurve::NistP256 => &[0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x03, 0x01, 0x07],
        EcdsaCurve::NistP384 => &[0x2B, 0x81, 0x04, 0x00, 0x22],
        EcdsaCurve::NistP521 => &[0x2B, 0x81, 0x04, 0x00, 0x23],
    };
    der(0x06, bytes)
}

const OID_ED25519: &[u8] = &[0x2B, 0x65, 0x70];
const OID_EC_PUBLIC_KEY: &[u8] = &[0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x02, 0x01];
const OID_RSA: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x01, 0x01];

fn pkcs1_public_der(rsa: &RsaPublicKey) -> Vec<u8> {
    der_seq(&[
        der_uint(rsa.n().as_positive_bytes().unwrap_or_default()),
        der_uint(rsa.e().as_positive_bytes().unwrap_or_default()),
    ])
}

fn spki_der(public: &PublicKey) -> Option<Vec<u8>> {
    let (algorithm, key_bits) = match public.key_data() {
        KeyData::Ed25519(key) => (der_seq(&[der(0x06, OID_ED25519)]), key.0.to_vec()),
        KeyData::Ecdsa(key) => (
            der_seq(&[der(0x06, OID_EC_PUBLIC_KEY), curve_oid(key.curve())]),
            key.as_sec1_bytes().to_vec(),
        ),
        KeyData::Rsa(key) => (
            der_seq(&[der(0x06, OID_RSA), vec![0x05, 0x00]]),
            pkcs1_public_der(key),
        ),
        _ => return None,
    };
    let mut bits = vec![0u8];
    bits.extend_from_slice(&key_bits);
    Some(der_seq(&[algorithm, der(0x03, &bits)]))
}

fn parse_spki(bytes: &[u8]) -> Option<PublicKey> {
    let mut outer = Reader(der_expect(&mut Reader(bytes), 0x30)?);
    let mut algorithm = Reader(der_expect(&mut outer, 0x30)?);
    let oid = der_expect(&mut algorithm, 0x06)?;
    let bits = der_expect(&mut outer, 0x03)?;
    let key_bits = bits.strip_prefix(&[0])?;
    let data = if oid == OID_ED25519 {
        KeyData::Ed25519(Ed25519PublicKey(key_bits.try_into().ok()?))
    } else if oid == OID_EC_PUBLIC_KEY {
        // O parâmetro é o OID da curva; o ponto já diz o tamanho, mas conferimos.
        let curve_oid_bytes = der_expect(&mut algorithm, 0x06)?;
        let expected = match key_bits.len() {
            65 => EcdsaCurve::NistP256,
            97 => EcdsaCurve::NistP384,
            133 => EcdsaCurve::NistP521,
            _ => return None,
        };
        if curve_oid(expected)[2..] != *curve_oid_bytes {
            return None;
        }
        KeyData::Ecdsa(EcdsaPublicKey::from_sec1_bytes(key_bits).ok()?)
    } else if oid == OID_RSA {
        return parse_pkcs1_public(key_bits);
    } else {
        return None;
    };
    Some(PublicKey::new(data, ""))
}

fn parse_pkcs1_public(bytes: &[u8]) -> Option<PublicKey> {
    let mut seq = Reader(der_expect(&mut Reader(bytes), 0x30)?);
    let n = Mpint::from_positive_bytes(der_expect(&mut seq, 0x02)?);
    let e = Mpint::from_positive_bytes(der_expect(&mut seq, 0x02)?);
    let rsa = RsaPublicKey::new(e, n).ok()?;
    Some(PublicKey::new(KeyData::Rsa(rsa), ""))
}

// ---------------------------------------------------------------------------------------------
// DER mínimo

/// Um elemento DER: o tag, o tamanho e o conteúdo.
fn der(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let len = content.len();
    if len < 0x80 {
        out.push(len as u8);
    } else {
        let bytes = len.to_be_bytes();
        let start = bytes
            .iter()
            .position(|&b| b != 0)
            .unwrap_or(bytes.len() - 1);
        out.push(0x80 | (bytes.len() - start) as u8);
        out.extend_from_slice(&bytes[start..]);
    }
    out.extend_from_slice(content);
    out
}

/// Uma `SEQUENCE` com os elementos já codificados.
fn der_seq(items: &[Vec<u8>]) -> Vec<u8> {
    der(0x30, &items.concat())
}

/// Um `INTEGER` positivo a partir dos bytes em big-endian.
fn der_uint(bytes: &[u8]) -> Vec<u8> {
    let start = bytes.iter().position(|&b| b != 0).unwrap_or(bytes.len());
    let bytes = &bytes[start..];
    let mut content = Vec::with_capacity(bytes.len() + 1);
    if bytes.first().is_none_or(|&b| b & 0x80 != 0) {
        content.push(0);
    }
    content.extend_from_slice(bytes);
    der(0x02, &content)
}

/// O leitor de DER: o que falta ler.
struct Reader<'a>(&'a [u8]);

/// Lê o próximo elemento, que precisa ter o `tag`, e devolve o conteúdo.
fn der_expect<'a>(reader: &mut Reader<'a>, tag: u8) -> Option<&'a [u8]> {
    let (&found, rest) = reader.0.split_first()?;
    let (&first, rest) = rest.split_first()?;
    let (len, rest) = if first < 0x80 {
        (usize::from(first), rest)
    } else {
        let count = usize::from(first & 0x7F);
        if count == 0 || count > 4 || rest.len() < count {
            return None;
        }
        let len = rest[..count]
            .iter()
            .fold(0usize, |acc, &b| (acc << 8) | usize::from(b));
        (len, &rest[count..])
    };
    if found != tag || rest.len() < len {
        return None;
    }
    reader.0 = &rest[len..];
    Some(&rest[..len])
}

// ---------------------------------------------------------------------------------------------
// PEM

/// O envelope PEM, com o base64 em 64 colunas.
fn pem_encode(label: &str, der: &[u8]) -> String {
    let encoded = Base64::encode_string(der);
    let mut out = format!("-----BEGIN {label}-----\n");
    for chunk in encoded.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(chunk).unwrap_or_default());
        out.push('\n');
    }
    out.push_str(&format!("-----END {label}-----\n"));
    out
}

/// Abre o primeiro bloco PEM do texto: o rótulo e o conteúdo.
fn pem_decode(text: &str) -> Option<(String, Zeroizing<Vec<u8>>)> {
    let start = text.find("-----BEGIN ")?;
    let rest = &text[start + "-----BEGIN ".len()..];
    let (label, rest) = rest.split_once("-----")?;
    let end = rest.find("-----END ")?;
    let body: String = rest[..end]
        .lines()
        .map(str::trim)
        // Cabeçalhos do PEM antigo cifrado (`Proc-Type:`, `DEK-Info:`) não são base64.
        .filter(|line| !line.contains(':'))
        .collect();
    let der = Base64::decode_vec(&body).ok()?;
    Some((label.to_owned(), Zeroizing::new(der)))
}
