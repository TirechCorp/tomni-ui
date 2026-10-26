//! A identidade de um nó do ZeroTier: um par X25519 (acordo de chaves) e um par Ed25519
//! (assinaturas), com o endereço de 40 bits derivado de uma prova de trabalho que custa 2 MiB de
//! memória e algumas dezenas de milissegundos (a identidade gerada demora segundos).

use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use salsa20::Salsa20;
use salsa20::cipher::{KeyIvInit, StreamCipher};
use sha2::{Digest, Sha512};
use zeroize::Zeroizing;

use crate::address::{ADDRESS_LEN, Address};
use crate::{Error, Result};

/// Memória da prova de trabalho, em bytes (`ZT_IDENTITY_GEN_MEMORY`, `Identity.cpp`).
const MEMORIA_PROVA: usize = 2 * 1024 * 1024;

/// A prova de trabalho só vale se o primeiro byte do hash for menor que isto
/// (`ZT_IDENTITY_GEN_HASHCASH_FIRST_BYTE_LESS_THAN`, `Identity.cpp`).
const LIMITE_PRIMEIRO_BYTE: u8 = 17;

/// Tamanho da chave pública: 32 bytes de X25519 e 32 de Ed25519 (`ZT_ECC_PUBLIC_KEY_SET_LEN`).
pub const PUBLIC_LEN: usize = 64;

/// Tamanho da chave secreta: 32 bytes de X25519 e 32 de semente Ed25519
/// (`ZT_ECC_PRIVATE_KEY_SET_LEN`).
pub const SECRET_LEN: usize = 64;

/// Tamanho da assinatura: 64 bytes de Ed25519 e os 32 do hash assinado (`ZT_ECC_SIGNATURE_LEN`).
pub const SIGNATURE_LEN: usize = 96;

/// Tamanho do hash que se assina (os primeiros 32 bytes do SHA-512 da mensagem).
const HASH_ASSINADO: usize = 32;

/// A identidade de um nó, com a chave secreta quando é a nossa.
#[derive(Clone)]
pub struct Identity {
    address: Address,
    public: [u8; PUBLIC_LEN],
    secret: Option<Zeroizing<[u8; SECRET_LEN]>>,
}

impl Identity {
    /// Gera uma identidade nova: sorteia pares de chaves até a prova de trabalho passar e o
    /// endereço não ser reservado.
    pub fn generate() -> Result<Self> {
        let mut memoria = vec![0u8; MEMORIA_PROVA];
        loop {
            let mut secret = Zeroizing::new([0u8; SECRET_LEN]);
            getrandom::fill(&mut secret[..])
                .map_err(|e| Error::Identity(format!("sem aleatoriedade: {e}")))?;
            let public = public_from_secret(&secret);
            let hash = memory_hard_hash(&public, &mut memoria);
            if hash[0] >= LIMITE_PRIMEIRO_BYTE {
                continue;
            }
            let address = address_from_hash(&hash);
            if address.is_reserved() {
                continue;
            }
            return Ok(Self {
                address,
                public,
                secret: Some(secret),
            });
        }
    }

    /// Lê a forma de texto, com ou sem a parte secreta (`endereço:0:pública[:secreta]`), sem
    /// conferir a prova de trabalho: use [`Identity::validate`] para isso.
    pub fn from_secret_str(texto: &str) -> Result<Self> {
        let texto = texto.trim();
        let mut campos = texto.split(':');
        let address = campos
            .next()
            .and_then(Address::parse)
            .filter(|a| !a.is_reserved())
            .ok_or_else(|| Error::Identity("endereço".into()))?;
        if campos.next() != Some("0") {
            return Err(Error::Identity("tipo de identidade desconhecido".into()));
        }
        let public = campos
            .next()
            .and_then(hex_fixed::<PUBLIC_LEN>)
            .ok_or_else(|| Error::Identity("chave pública".into()))?;
        let secret = match campos.next() {
            None => None,
            Some(h) => Some(Zeroizing::new(
                hex_fixed::<SECRET_LEN>(h)
                    .ok_or_else(|| Error::Identity("chave secreta".into()))?,
            )),
        };
        if campos.next().is_some() {
            return Err(Error::Identity("campos sobrando".into()));
        }
        Ok(Self {
            address,
            public,
            secret,
        })
    }

    /// Lê a forma de texto só com a parte pública (aceita a com secreta e a ignora).
    pub fn from_public_str(texto: &str) -> Result<Self> {
        let mut id = Self::from_secret_str(texto)?;
        id.secret = None;
        Ok(id)
    }

    /// Lê a forma binária que vai nos pacotes (`Identity::deserialize`): endereço (5 bytes), tipo
    /// (1 byte, 0 = Curve25519/Ed25519), chave pública (64) e o tamanho da secreta (1 byte, que
    /// aqui precisa ser 0: um pacote nunca leva a chave secreta). Devolve a identidade e quantos
    /// bytes consumiu.
    pub fn from_wire(bytes: &[u8]) -> Result<(Self, usize)> {
        const TIPO: usize = ADDRESS_LEN;
        const PUBLICA: usize = TIPO + 1;
        const TAMANHO_SECRETA: usize = PUBLICA + PUBLIC_LEN;
        if bytes.len() <= TAMANHO_SECRETA {
            return Err(Error::Identity("identidade curta demais".into()));
        }
        if bytes[TIPO] != 0 {
            return Err(Error::Identity("tipo de identidade desconhecido".into()));
        }
        if bytes[TAMANHO_SECRETA] != 0 {
            return Err(Error::Identity("pacote com chave secreta".into()));
        }
        let address = Address::from_bytes(&bytes[..ADDRESS_LEN])
            .filter(|a| !a.is_reserved())
            .ok_or_else(|| Error::Identity("endereço".into()))?;
        let public = bytes[PUBLICA..PUBLICA + PUBLIC_LEN]
            .try_into()
            .expect("64 bytes");
        Ok((
            Self {
                address,
                public,
                secret: None,
            },
            TAMANHO_SECRETA + 1,
        ))
    }

    /// Escreve a forma binária dos pacotes (só a parte pública), a que [`Identity::from_wire`] lê.
    pub fn to_wire(&self) -> Vec<u8> {
        let mut saida = Vec::with_capacity(ADDRESS_LEN + 1 + PUBLIC_LEN + 1);
        saida.extend_from_slice(&self.address.to_bytes());
        saida.push(0);
        saida.extend_from_slice(&self.public);
        saida.push(0);
        saida
    }

    /// A forma de texto com a chave secreta (para guardar no cofre).
    pub fn to_secret_string(&self) -> Option<Zeroizing<String>> {
        let secret = self.secret.as_ref()?;
        Some(Zeroizing::new(format!(
            "{}:0:{}:{}",
            self.address,
            to_hex(&self.public),
            to_hex(&secret[..])
        )))
    }

    /// A forma de texto só com a parte pública (a que se mostra e se manda aos outros nós).
    pub fn to_public_string(&self) -> String {
        format!("{}:0:{}", self.address, to_hex(&self.public))
    }

    /// O endereço do nó.
    pub fn address(&self) -> Address {
        self.address
    }

    /// A chave pública: X25519 nos primeiros 32 bytes, Ed25519 nos 32 últimos.
    pub fn public_bytes(&self) -> &[u8; PUBLIC_LEN] {
        &self.public
    }

    /// Esta identidade tem a chave secreta.
    pub fn has_secret(&self) -> bool {
        self.secret.is_some()
    }

    /// A chave secreta X25519, para a armadura estendida (só dentro do crate).
    pub(crate) fn secret_x25519(&self) -> Option<Zeroizing<[u8; 32]>> {
        let secret = self.secret.as_ref()?;
        let mut chave = Zeroizing::new([0u8; 32]);
        chave.copy_from_slice(&secret[..32]);
        Some(chave)
    }

    /// Confere o endereço contra a chave pública, refazendo a prova de trabalho (cerca de 2 MiB
    /// de memória): é o que impede alguém de se passar por outro endereço.
    pub fn validate(&self) -> bool {
        if self.address.is_reserved() {
            return false;
        }
        let mut memoria = vec![0u8; MEMORIA_PROVA];
        let hash = memory_hard_hash(&self.public, &mut memoria);
        hash[0] < LIMITE_PRIMEIRO_BYTE && address_from_hash(&hash) == self.address
    }

    /// A chave simétrica de 48 bytes que este nó e `outro` compartilham: X25519 e depois SHA-512
    /// (os 48 primeiros bytes). `None` se esta identidade não tem a chave secreta.
    pub fn agree(&self, outro: &Identity) -> Option<Zeroizing<[u8; 48]>> {
        let secret = self.secret.as_ref()?;
        let compartilhado = Zeroizing::new(agree_hash(
            secret[..32].try_into().ok()?,
            outro.public[..32].try_into().ok()?,
        ));
        let mut chave = Zeroizing::new([0u8; 48]);
        chave.copy_from_slice(&compartilhado[..48]);
        Some(chave)
    }

    /// Assina `mensagem`: Ed25519 sobre os primeiros 32 bytes do SHA-512 dela, seguido desses 32
    /// bytes. `None` se esta identidade não tem a chave secreta.
    pub fn sign(&self, mensagem: &[u8]) -> Option<[u8; SIGNATURE_LEN]> {
        let secret = self.secret.as_ref()?;
        let hash = Sha512::digest(mensagem);
        let semente: [u8; 32] = secret[32..].try_into().ok()?;
        let assinatura = SigningKey::from_bytes(&semente).sign(&hash[..HASH_ASSINADO]);
        let mut saida = [0u8; SIGNATURE_LEN];
        saida[..64].copy_from_slice(&assinatura.to_bytes());
        saida[64..].copy_from_slice(&hash[..HASH_ASSINADO]);
        Some(saida)
    }

    /// Confere uma assinatura de [`Identity::sign`] (de qualquer implementação do protocolo).
    pub fn verify(&self, mensagem: &[u8], assinatura: &[u8]) -> bool {
        verify_with_public(&self.public, mensagem, assinatura)
    }
}

/// Confere uma assinatura de 96 bytes contra um conjunto de chaves públicas de 64 bytes (a chave
/// Ed25519 são os 32 últimos). É o que o mundo (planet e moons) usa: a chave que assina as
/// atualizações não tem endereço de nó.
pub fn verify_with_public(public: &[u8; PUBLIC_LEN], mensagem: &[u8], assinatura: &[u8]) -> bool {
    let Ok(assinatura): std::result::Result<&[u8; SIGNATURE_LEN], _> = assinatura.try_into() else {
        return false;
    };
    let hash = Sha512::digest(mensagem);
    if assinatura[64..] != hash[..HASH_ASSINADO] {
        return false;
    }
    let Ok(chave) = <[u8; 32]>::try_from(&public[32..]) else {
        return false;
    };
    let Ok(chave) = VerifyingKey::from_bytes(&chave) else {
        return false;
    };
    let Ok(assinatura) = ed25519_dalek::Signature::from_slice(&assinatura[..64]) else {
        return false;
    };
    ed25519_dalek::Verifier::verify(&chave, &hash[..HASH_ASSINADO], &assinatura).is_ok()
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity")
            .field("address", &self.address)
            .field("secret", &self.secret.is_some())
            .finish()
    }
}

/// O hash X25519 + SHA-512 do acordo de chaves (`ECC::agree`, `ECC.cpp`): os 64 bytes completos.
pub(crate) fn agree_hash(secreta: &[u8; 32], publica: &[u8; 32]) -> [u8; 64] {
    let bruto = Zeroizing::new(x25519_dalek::x25519(*secreta, *publica));
    Sha512::digest(&bruto[..]).into()
}

/// A chave pública X25519 de uma secreta.
pub(crate) fn x25519_public(secreta: &[u8; 32]) -> [u8; 32] {
    x25519_dalek::x25519(*secreta, x25519_dalek::X25519_BASEPOINT_BYTES)
}

fn public_from_secret(secret: &[u8; SECRET_LEN]) -> [u8; PUBLIC_LEN] {
    let mut public = [0u8; PUBLIC_LEN];
    let x25519: [u8; 32] = secret[..32].try_into().expect("32 bytes");
    let semente: [u8; 32] = secret[32..].try_into().expect("32 bytes");
    public[..32].copy_from_slice(&x25519_public(&x25519));
    public[32..].copy_from_slice(SigningKey::from_bytes(&semente).verifying_key().as_bytes());
    public
}

/// O endereço são os últimos 5 bytes do hash da prova de trabalho.
fn address_from_hash(hash: &[u8; 64]) -> Address {
    Address::from_bytes(&hash[64 - ADDRESS_LEN..]).expect("5 bytes")
}

/// A prova de trabalho (`_computeMemoryHardHash`, `Identity.cpp`): SHA-512 da chave pública
/// vira a chave e o IV de um Salsa20/20; o fluxo enche 2 MiB encadeando blocos de 64 bytes; e
/// então 2^18 trocas de palavras entre a memória e o hash, cada uma seguida de um bloco de
/// Salsa20 sobre o hash, embaralham tudo. O resultado é o hash final de 64 bytes.
fn memory_hard_hash(public: &[u8; PUBLIC_LEN], memoria: &mut [u8]) -> [u8; 64] {
    debug_assert_eq!(memoria.len(), MEMORIA_PROVA);
    let mut hash: [u8; 64] = Sha512::digest(public).into();
    let chave: [u8; 32] = hash[..32].try_into().expect("32 bytes");
    let iv: [u8; 8] = hash[32..40].try_into().expect("8 bytes");
    let mut salsa = Salsa20::new(&chave.into(), &iv.into());

    memoria.fill(0);
    salsa.apply_keystream(&mut memoria[..64]);
    for i in (64..MEMORIA_PROVA).step_by(64) {
        memoria.copy_within(i - 64..i, i);
        salsa.apply_keystream(&mut memoria[i..i + 64]);
    }

    let palavras = MEMORIA_PROVA / 8;
    let mut i = 0;
    while i < palavras {
        let idx1 = (palavra_be(memoria, i) % 8) as usize;
        let idx2 = (palavra_be(memoria, i + 1) % palavras as u64) as usize;
        i += 2;
        let mut troca = [0u8; 8];
        troca.copy_from_slice(&memoria[idx2 * 8..idx2 * 8 + 8]);
        memoria[idx2 * 8..idx2 * 8 + 8].copy_from_slice(&hash[idx1 * 8..idx1 * 8 + 8]);
        hash[idx1 * 8..idx1 * 8 + 8].copy_from_slice(&troca);
        salsa.apply_keystream(&mut hash);
    }
    hash
}

fn palavra_be(memoria: &[u8], indice: usize) -> u64 {
    let mut p = [0u8; 8];
    p.copy_from_slice(&memoria[indice * 8..indice * 8 + 8]);
    u64::from_be_bytes(p)
}

fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn hex_fixed<const N: usize>(texto: &str) -> Option<[u8; N]> {
    if texto.len() != N * 2 || !texto.is_ascii() {
        return None;
    }
    let mut saida = [0u8; N];
    for (i, par) in texto.as_bytes().chunks(2).enumerate() {
        saida[i] = u8::from_str_radix(std::str::from_utf8(par).ok()?, 16).ok()?;
    }
    Some(saida)
}
