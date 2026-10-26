//! As chaves do WireGuard: privada, pública e pré-compartilhada.
//!
//! Todas têm 32 bytes e viajam em base64 padrão (44 caracteres), como no `.conf` do `wg-quick`.
//! O `Debug` das chaves secretas nunca mostra o conteúdo, e a memória delas é apagada ao soltar.

use std::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use x25519_dalek::{PublicKey as DalekPublic, StaticSecret};
use zeroize::{Zeroize, Zeroizing};

/// O tamanho de todas as chaves do WireGuard, em bytes.
pub const KEY_LEN: usize = 32;

/// Um erro ao ler uma chave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyError {
    /// O texto não é base64 válido.
    Base64,
    /// O base64 é válido, mas não tem 32 bytes (o tamanho achado vem no erro).
    Length(usize),
    /// O resultado do Diffie-Hellman foi todo zero: a chave pública do outro lado é inválida
    /// (um ponto de ordem pequena), e o `wireguard-go` também a recusa.
    WeakPublicKey,
    /// O sistema não deu números aleatórios.
    Random,
}

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeyError::Base64 => write!(f, "a chave não é base64 válido"),
            KeyError::Length(n) => write!(f, "a chave tem {n} bytes; esperado: 32 bytes em base64"),
            KeyError::WeakPublicKey => {
                write!(f, "a chave pública do par é inválida (Diffie-Hellman nulo)")
            }
            KeyError::Random => write!(f, "o sistema não forneceu números aleatórios"),
        }
    }
}

impl std::error::Error for KeyError {}

/// Enche `buf` de bytes aleatórios do sistema.
pub(crate) fn random_fill(buf: &mut [u8]) -> Result<(), KeyError> {
    getrandom::fill(buf).map_err(|_| KeyError::Random)
}

/// Um array de `N` bytes aleatórios.
pub(crate) fn random_bytes<const N: usize>() -> Result<[u8; N], KeyError> {
    let mut b = [0u8; N];
    random_fill(&mut b)?;
    Ok(b)
}

fn decode32(texto: &str) -> Result<[u8; KEY_LEN], KeyError> {
    let bytes = STANDARD
        .decode(texto.trim())
        .map_err(|_| KeyError::Base64)?;
    let mut bytes = Zeroizing::new(bytes);
    if bytes.len() != KEY_LEN {
        return Err(KeyError::Length(bytes.len()));
    }
    let mut out = [0u8; KEY_LEN];
    out.copy_from_slice(&bytes);
    bytes.zeroize();
    Ok(out)
}

/// A chave privada de um ponto (X25519).
#[derive(Clone)]
pub struct PrivateKey(Zeroizing<[u8; KEY_LEN]>);

impl PrivateKey {
    /// Gera uma chave nova: 32 bytes aleatórios com os bits ajustados (clamp) como o `wg genkey`.
    pub fn generate() -> Result<Self, KeyError> {
        let mut b = random_bytes::<KEY_LEN>()?;
        b[0] &= 248;
        b[31] &= 127;
        b[31] |= 64;
        Ok(PrivateKey(Zeroizing::new(b)))
    }

    /// Cria a chave a partir dos 32 bytes.
    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        PrivateKey(Zeroizing::new(bytes))
    }

    /// Lê a chave do base64 (o `PrivateKey = …` do `.conf`).
    pub fn from_base64(texto: &str) -> Result<Self, KeyError> {
        Ok(PrivateKey(Zeroizing::new(decode32(texto)?)))
    }

    /// Os 32 bytes da chave. Cuidado: é o segredo.
    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }

    /// O base64 da chave, numa string que se apaga ao soltar.
    pub fn to_base64(&self) -> Zeroizing<String> {
        Zeroizing::new(STANDARD.encode(*self.0))
    }

    /// A chave pública correspondente.
    pub fn public_key(&self) -> PublicKey {
        let s = StaticSecret::from(*self.0);
        PublicKey(DalekPublic::from(&s).to_bytes())
    }

    /// O Diffie-Hellman X25519 com a chave pública do outro lado. Recusa o resultado todo
    /// zero (chave pública de ordem pequena).
    pub fn diffie_hellman(&self, other: &PublicKey) -> Result<Zeroizing<[u8; KEY_LEN]>, KeyError> {
        let s = StaticSecret::from(*self.0);
        let shared = s.diffie_hellman(&DalekPublic::from(other.0));
        if !shared.was_contributory() {
            return Err(KeyError::WeakPublicKey);
        }
        Ok(Zeroizing::new(shared.to_bytes()))
    }
}

impl fmt::Debug for PrivateKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PrivateKey(..)")
    }
}

impl PartialEq for PrivateKey {
    fn eq(&self, other: &Self) -> bool {
        use subtle::ConstantTimeEq;
        self.0.ct_eq(&*other.0).into()
    }
}

impl Eq for PrivateKey {}

/// A chave pública de um ponto.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PublicKey([u8; KEY_LEN]);

impl PublicKey {
    /// Cria a chave a partir dos 32 bytes.
    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        PublicKey(bytes)
    }

    /// Lê a chave do base64 (o `PublicKey = …` do `.conf`).
    pub fn from_base64(texto: &str) -> Result<Self, KeyError> {
        Ok(PublicKey(decode32(texto)?))
    }

    /// Os 32 bytes da chave.
    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }

    /// O base64 da chave (44 caracteres).
    pub fn to_base64(&self) -> String {
        STANDARD.encode(self.0)
    }

    /// Os primeiros oito caracteres do base64 e reticências, para mensagens e telas.
    pub fn short(&self) -> String {
        let b = self.to_base64();
        format!("{}…", &b[..8])
    }
}

impl fmt::Display for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_base64())
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PublicKey({})", self.short())
    }
}

/// A chave pré-compartilhada (PSK) de um par, opcional no WireGuard.
#[derive(Clone)]
pub struct PresharedKey(Zeroizing<[u8; KEY_LEN]>);

impl PresharedKey {
    /// Cria a chave a partir dos 32 bytes.
    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        PresharedKey(Zeroizing::new(bytes))
    }

    /// Lê a chave do base64 (o `PresharedKey = …` do `.conf`).
    pub fn from_base64(texto: &str) -> Result<Self, KeyError> {
        Ok(PresharedKey(Zeroizing::new(decode32(texto)?)))
    }

    /// Gera uma chave nova (o `wg genpsk`).
    pub fn generate() -> Result<Self, KeyError> {
        Ok(PresharedKey(Zeroizing::new(random_bytes::<KEY_LEN>()?)))
    }

    /// Os 32 bytes da chave. Cuidado: é o segredo.
    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }

    /// O base64 da chave, numa string que se apaga ao soltar.
    pub fn to_base64(&self) -> Zeroizing<String> {
        Zeroizing::new(STANDARD.encode(*self.0))
    }
}

impl fmt::Debug for PresharedKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PresharedKey(..)")
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    // Par de chaves do exemplo do `wg(8)` / vetor conhecido da RFC 7748 (seção 6.1), em base64.
    const PRIVADA_ALICE: [u8; 32] = [
        0x77, 0x07, 0x6d, 0x0a, 0x73, 0x18, 0xa5, 0x7d, 0x3c, 0x16, 0xc1, 0x72, 0x51, 0xb2, 0x66,
        0x45, 0xdf, 0x4c, 0x2f, 0x87, 0xeb, 0xc0, 0x99, 0x2a, 0xb1, 0x77, 0xfb, 0xa5, 0x1d, 0xb9,
        0x2c, 0x2a,
    ];
    const PUBLICA_ALICE: [u8; 32] = [
        0x85, 0x20, 0xf0, 0x09, 0x89, 0x30, 0xa7, 0x54, 0x74, 0x8b, 0x7d, 0xdc, 0xb4, 0x3e, 0xf7,
        0x5a, 0x0d, 0xbf, 0x3a, 0x0d, 0x26, 0x38, 0x1a, 0xf4, 0xeb, 0xa4, 0xa9, 0x8e, 0xaa, 0x9b,
        0x4e, 0x6a,
    ];

    #[test]
    fn a_publica_sai_da_privada_pelo_vetor_da_rfc_7748() {
        let privada = PrivateKey::from_bytes(PRIVADA_ALICE);
        assert_eq!(privada.public_key().as_bytes(), &PUBLICA_ALICE);
    }

    #[test]
    fn base64_de_ida_e_volta() {
        let privada = PrivateKey::generate().unwrap();
        let texto = privada.to_base64();
        assert_eq!(texto.len(), 44);
        assert_eq!(PrivateKey::from_base64(&texto).unwrap(), privada);
        let publica = privada.public_key();
        assert_eq!(
            PublicKey::from_base64(&publica.to_base64()).unwrap(),
            publica
        );
    }

    #[test]
    fn base64_invalido_e_tamanho_errado_sao_recusados() {
        assert_eq!(
            PublicKey::from_base64("isto não é base64!").unwrap_err(),
            KeyError::Base64
        );
        assert_eq!(
            PublicKey::from_base64("YWJj").unwrap_err(),
            KeyError::Length(3)
        );
        assert!(
            KeyError::Length(3)
                .to_string()
                .contains("esperado: 32 bytes em base64")
        );
    }

    #[test]
    fn o_debug_nao_mostra_o_segredo() {
        let privada = PrivateKey::from_bytes(PRIVADA_ALICE);
        let texto = format!("{privada:?}");
        assert_eq!(texto, "PrivateKey(..)");
        let psk = PresharedKey::from_bytes([7; 32]);
        assert_eq!(format!("{psk:?}"), "PresharedKey(..)");
    }

    #[test]
    fn a_chave_gerada_tem_os_bits_ajustados() {
        let k = PrivateKey::generate().unwrap();
        let b = k.as_bytes();
        assert_eq!(b[0] & 7, 0);
        assert_eq!(b[31] & 0x80, 0);
        assert_eq!(b[31] & 0x40, 0x40);
    }

    #[test]
    fn o_diffie_hellman_e_simetrico_e_recusa_o_ponto_nulo() {
        let a = PrivateKey::generate().unwrap();
        let b = PrivateKey::generate().unwrap();
        let ab = a.diffie_hellman(&b.public_key()).unwrap();
        let ba = b.diffie_hellman(&a.public_key()).unwrap();
        assert_eq!(*ab, *ba);
        let zero = PublicKey::from_bytes([0; 32]);
        assert_eq!(
            a.diffie_hellman(&zero).unwrap_err(),
            KeyError::WeakPublicKey
        );
    }

    #[test]
    fn a_chave_curta_tem_oito_caracteres() {
        let k = PublicKey::from_bytes(PUBLICA_ALICE);
        assert_eq!(k.short(), "hSDwCYkw…");
    }
}
