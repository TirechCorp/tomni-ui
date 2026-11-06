//! A escrita do `.ppk` do PuTTY (`PuTTY-User-Key-File-2` e `-3`). A leitura é do `ssh-key`
//! (`PrivateKey::from_ppk`), que o `KeyFile` já usa, com uma exceção: o `ssh-key` lê a semente
//! Ed25519 como `mpint`, que recusa um zero à esquerda sem o bit seguinte ligado (uma chave em
//! cada 512, do `puttygen` ou nossa). [`read_ed25519`] lê essas.
//!
//! O formato está no apêndice C do manual do PuTTY ("PPK file format"). A parte privada de cada
//! algoritmo, sem o cabeçalho:
//!
//! - Ed25519: um `string` com a semente de 32 bytes (conferido com o `puttygen` 0.81: sai sempre
//!   com 32 bytes, mesmo com o primeiro bit ligado, então não é um `mpint`);
//! - RSA: `mpint d`, `mpint p`, `mpint q`, `mpint iqmp`;
//! - ECDSA: `mpint` do escalar.
//!
//! Com passphrase, a parte privada é completada até um múltiplo de 16 bytes e cifrada com
//! AES-256-CBC. A v3 deriva a chave com Argon2id e assina com HMAC-SHA-256; a v2, com SHA-1.

use std::collections::HashMap;

use argon2::{Algorithm, Argon2, Params, Version};
use hmac::digest::KeyInit;
use hmac::{Hmac, Mac};
use rand::RngExt;
use sha1::{Digest, Sha1};
use sha2::Sha256;
use ssh_key::encoding::base64::{Base64, Encoding};
use ssh_key::private::{Ed25519Keypair, KeypairData};
use ssh_key::{Cipher, PrivateKey};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use super::KeyError;

/// Os parâmetros do Argon2id do `puttygen` 0.81: 8 MiB de memória, 13 passadas, 1 faixa.
const ARGON2_MEMORY_KIB: u32 = 8192;
const ARGON2_PASSES: u32 = 13;
const ARGON2_LANES: u32 = 1;

/// O prefixo da chave do MAC da v2.
const V2_MAC_PREFIX: &[u8] = b"putty-private-key-file-mac-key";

/// Escreve a chave num `.ppk` da `version` (2 ou 3). `passphrase` vazia ou `None` grava sem
/// cifra.
pub(super) fn write(
    key: &PrivateKey,
    version: u8,
    passphrase: Option<&str>,
) -> Result<Zeroizing<String>, KeyError> {
    let passphrase = passphrase.filter(|p| !p.is_empty());
    let algorithm = key.algorithm();
    let algorithm = algorithm.as_str();
    let public = key.public_key().to_bytes()?;
    let comment = key.comment().to_string().replace(['\r', '\n'], " ");
    let encryption = if passphrase.is_some() {
        "aes256-cbc"
    } else {
        "none"
    };
    let mut private = private_blob(key)?;
    let mut rng = rand::rng();

    // O sal só vai no cabeçalho da v3 cifrada.
    let mut argon_salt = [0u8; 16];
    if version == 3 && passphrase.is_some() {
        rng.fill(&mut argon_salt[..]);
    }
    let (aes_key, iv, mac_key) = derive_keys(
        version,
        passphrase,
        Some(ArgonParams {
            algorithm: Algorithm::Argon2id,
            memory: ARGON2_MEMORY_KIB,
            passes: ARGON2_PASSES,
            lanes: ARGON2_LANES,
            salt: &argon_salt,
        }),
    )?;

    if passphrase.is_some() {
        // Preenche até o bloco do AES com bytes aleatórios: o leitor descarta o excesso.
        while private.len() % 16 != 0 {
            private.push(rng.random::<u8>());
        }
    }
    // O MAC cobre a parte privada já preenchida, mas ainda em claro.
    let mac = compute_mac(
        version,
        &mac_key,
        [
            algorithm.as_bytes(),
            encryption.as_bytes(),
            comment.as_bytes(),
            &public,
            &private,
        ],
    )?;

    if passphrase.is_some() {
        Cipher::Aes256Cbc
            .encrypt(&aes_key, &iv, &mut private)
            .map_err(|error| KeyError::Unsupported(format!("o AES-CBC: {error}")))?;
    }

    let mut text = Zeroizing::new(String::new());
    text.push_str(&format!("PuTTY-User-Key-File-{version}: {algorithm}\n"));
    text.push_str(&format!("Encryption: {encryption}\n"));
    text.push_str(&format!("Comment: {comment}\n"));
    push_lines(&mut text, "Public-Lines", &public);
    if version == 3 && passphrase.is_some() {
        text.push_str("Key-Derivation: Argon2id\n");
        text.push_str(&format!("Argon2-Memory: {ARGON2_MEMORY_KIB}\n"));
        text.push_str(&format!("Argon2-Passes: {ARGON2_PASSES}\n"));
        text.push_str(&format!("Argon2-Parallelism: {ARGON2_LANES}\n"));
        text.push_str(&format!("Argon2-Salt: {}\n", hex(&argon_salt)));
    }
    push_lines(&mut text, "Private-Lines", &private);
    text.push_str(&format!("Private-MAC: {mac}\n"));
    Ok(text)
}

/// O Argon2 de um `.ppk` v3.
struct ArgonParams<'a> {
    algorithm: Algorithm,
    memory: u32,
    passes: u32,
    lanes: u32,
    salt: &'a [u8],
}

/// A chave do AES, o IV e a chave do MAC de um `.ppk`.
type DerivedKeys = (Zeroizing<Vec<u8>>, [u8; 16], Zeroizing<Vec<u8>>);

/// As chaves de um `.ppk`. `argon` só importa na v3 cifrada.
fn derive_keys(
    version: u8,
    passphrase: Option<&str>,
    argon: Option<ArgonParams>,
) -> Result<DerivedKeys, KeyError> {
    match (version, passphrase) {
        (3, Some(pass)) => {
            let argon =
                argon.ok_or_else(|| KeyError::Unsupported("o PPK sem o Argon2".to_owned()))?;
            let params = Params::new(argon.memory, argon.passes, argon.lanes, Some(80))
                .map_err(|error| KeyError::Unsupported(format!("o Argon2: {error}")))?;
            let argon2 = Argon2::new(argon.algorithm, Version::V0x13, params);
            let mut out = Zeroizing::new(vec![0u8; 80]);
            argon2
                .hash_password_into(pass.as_bytes(), argon.salt, &mut out)
                .map_err(|error| KeyError::Unsupported(format!("o Argon2: {error}")))?;
            let mut iv = [0u8; 16];
            iv.copy_from_slice(&out[32..48]);
            Ok((
                Zeroizing::new(out[..32].to_vec()),
                iv,
                Zeroizing::new(out[48..80].to_vec()),
            ))
        }
        (3, None) => Ok((Zeroizing::default(), [0; 16], Zeroizing::default())),
        (2, Some(pass)) => {
            let mut aes = Zeroizing::new(Vec::with_capacity(40));
            for counter in 0u32..2 {
                let mut hash = Sha1::new();
                hash.update(counter.to_be_bytes());
                hash.update(pass.as_bytes());
                aes.extend_from_slice(&hash.finalize());
            }
            aes.truncate(32);
            let mut hash = Sha1::new();
            hash.update(V2_MAC_PREFIX);
            hash.update(pass.as_bytes());
            Ok((aes, [0; 16], Zeroizing::new(hash.finalize().to_vec())))
        }
        (2, None) => {
            let mut hash = Sha1::new();
            hash.update(V2_MAC_PREFIX);
            Ok((
                Zeroizing::default(),
                [0; 16],
                Zeroizing::new(hash.finalize().to_vec()),
            ))
        }
        _ => Err(KeyError::Unsupported(format!("o PPK versão {version}"))),
    }
}

/// O MAC de um `.ppk`, em hexadecimal, sobre o algoritmo, a cifra, o comentário, a pública e a
/// parte privada já preenchida, mas ainda em claro.
fn compute_mac(version: u8, mac_key: &[u8], fields: [&[u8]; 5]) -> Result<String, KeyError> {
    let mut mac_data = Zeroizing::new(Vec::new());
    for field in fields {
        put_string(&mut mac_data, field);
    }
    let bad_key = |_| KeyError::Unsupported("a chave do MAC".to_owned());
    Ok(if version == 3 {
        let mut hmac = Hmac::<Sha256>::new_from_slice(mac_key).map_err(bad_key)?;
        hmac.update(&mac_data);
        hex(&hmac.finalize().into_bytes())
    } else {
        let mut hmac = Hmac::<Sha1>::new_from_slice(mac_key).map_err(bad_key)?;
        hmac.update(&mac_data);
        hex(&hmac.finalize().into_bytes())
    })
}

/// Lê um `.ppk` Ed25519, com a semente como o formato manda: um `string` de 32 bytes. É o recurso
/// para as chaves que o `ssh-key` recusa (ver o começo do arquivo). `None`: não é um `.ppk`
/// Ed25519, a passphrase não confere ou o arquivo está adulterado.
pub(super) fn read_ed25519(text: &str, passphrase: Option<&str>) -> Option<PrivateKey> {
    let mut lines = text.lines();
    let (version, algorithm) = lines
        .next()?
        .strip_prefix("PuTTY-User-Key-File-")?
        .split_once(':')?;
    let algorithm = algorithm.trim();
    if algorithm != "ssh-ed25519" {
        return None;
    }
    let version: u8 = version.parse().ok()?;
    let mut fields = HashMap::new();
    let (mut public, mut private) = (None, None);
    while let Some(line) = lines.next() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.strip_prefix(' ').unwrap_or(value);
        if matches!(name, "Public-Lines" | "Private-Lines") {
            let mut encoded = Zeroizing::new(String::new());
            for _ in 0..value.trim().parse::<usize>().ok()? {
                encoded.push_str(lines.next()?.trim());
            }
            let bytes = Zeroizing::new(Base64::decode_vec(&encoded).ok()?);
            if name == "Public-Lines" {
                public = Some(bytes);
            } else {
                private = Some(bytes);
            }
        } else {
            fields.insert(name, value);
        }
    }
    let (public, mut private) = (public?, private?);
    let encryption = *fields.get("Encryption")?;
    let passphrase = match encryption {
        "none" => None,
        "aes256-cbc" => Some(passphrase.filter(|p| !p.is_empty())?),
        _ => return None,
    };
    let salt = fields.get("Argon2-Salt").and_then(|salt| from_hex(salt));
    let argon = match (version, passphrase) {
        (3, Some(_)) => Some(ArgonParams {
            algorithm: match *fields.get("Key-Derivation")? {
                "Argon2id" => Algorithm::Argon2id,
                "Argon2i" => Algorithm::Argon2i,
                "Argon2d" => Algorithm::Argon2d,
                _ => return None,
            },
            memory: fields.get("Argon2-Memory")?.trim().parse().ok()?,
            passes: fields.get("Argon2-Passes")?.trim().parse().ok()?,
            lanes: fields.get("Argon2-Parallelism")?.trim().parse().ok()?,
            salt: salt.as_deref()?,
        }),
        _ => None,
    };
    let (aes_key, iv, mac_key) = derive_keys(version, passphrase, argon).ok()?;
    if passphrase.is_some() {
        Cipher::Aes256Cbc
            .decrypt(&aes_key, &iv, &mut private, None)
            .ok()?;
    }
    let comment = fields.get("Comment").copied().unwrap_or_default();
    let mac = compute_mac(
        version,
        &mac_key,
        [
            algorithm.as_bytes(),
            encryption.as_bytes(),
            comment.as_bytes(),
            &public,
            &private,
        ],
    )
    .ok()?;
    let expected = fields.get("Private-MAC")?.trim().to_ascii_lowercase();
    if !bool::from(mac.as_bytes().ct_eq(expected.as_bytes())) {
        return None;
    }
    let (size, rest) = private.split_first_chunk::<4>()?;
    if u32::from_be_bytes(*size) != 32 {
        return None;
    }
    let seed = Zeroizing::new(*rest.first_chunk::<32>()?);
    let mut key = PrivateKey::from(Ed25519Keypair::from_seed(&seed));
    if key.public_key().to_bytes().ok()? != **public {
        return None;
    }
    key.set_comment(comment);
    Some(key)
}

fn from_hex(text: &str) -> Option<Vec<u8>> {
    let text = text.trim().as_bytes();
    if !text.len().is_multiple_of(2) {
        return None;
    }
    text.chunks(2)
        .map(|pair| {
            let digit = |byte: u8| char::from(byte).to_digit(16);
            u8::try_from(digit(pair[0])? * 16 + digit(pair[1])?).ok()
        })
        .collect()
}

/// A parte privada do algoritmo, sem o preenchimento.
fn private_blob(key: &PrivateKey) -> Result<Zeroizing<Vec<u8>>, KeyError> {
    let mut blob = Zeroizing::new(Vec::new());
    match key.key_data() {
        KeypairData::Ed25519(pair) => {
            let seed = Zeroizing::new(pair.private.to_bytes());
            put_string(&mut blob, &*seed);
        }
        KeypairData::Rsa(pair) => {
            let private = pair.private();
            for number in [private.d(), private.p(), private.q(), private.iqmp()] {
                put_string(&mut blob, number.as_bytes());
            }
        }
        KeypairData::Ecdsa(pair) => put_mpint(&mut blob, pair.private_key_bytes()),
        _ => {
            return Err(KeyError::FormatNotSupported {
                format: "PuTTY (.ppk)",
                kind: key.algorithm().to_string(),
            });
        }
    }
    Ok(blob)
}

/// Um campo `string` do SSH: o tamanho em quatro bytes e o conteúdo.
fn put_string(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
}

/// Um `mpint` do SSH a partir de um inteiro positivo em big-endian: sem zeros à esquerda e com
/// um zero na frente se o primeiro bit estiver ligado.
fn put_mpint(out: &mut Vec<u8>, bytes: &[u8]) {
    let start = bytes.iter().position(|&b| b != 0).unwrap_or(bytes.len());
    let bytes = &bytes[start..];
    if bytes.first().is_some_and(|&b| b & 0x80 != 0) {
        out.extend_from_slice(&(bytes.len() as u32 + 1).to_be_bytes());
        out.push(0);
    } else {
        out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    }
    out.extend_from_slice(bytes);
}

/// `Nome: linhas` e o conteúdo em base64, 64 colunas por linha.
fn push_lines(text: &mut String, name: &str, bytes: &[u8]) {
    let encoded = Base64::encode_string(bytes);
    let lines: Vec<&str> = encoded
        .as_bytes()
        .chunks(64)
        .map(|chunk| std::str::from_utf8(chunk).unwrap_or_default())
        .collect();
    text.push_str(&format!("{name}: {}\n", lines.len()));
    for line in lines {
        text.push_str(line);
        text.push('\n');
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
