//! A criptografia do VL1: a chave do par, a armadura dos pacotes (Salsa20/12 + Poly1305 e a
//! armadura estendida do HELLO) e, nas fatias seguintes, o AES-GMAC-SIV.

pub mod armor;
pub mod gmac_siv;
pub mod salsa;

use std::sync::Arc;

use zeroize::Zeroizing;

/// A chave que dois nós compartilham, tirada do acordo de chaves de 48 bytes.
#[derive(Clone)]
pub struct PeerKey {
    /// Os 32 primeiros bytes: a chave do Salsa20/12 + Poly1305 (antes da mistura por pacote).
    pub salsa: Zeroizing<[u8; 32]>,
    /// As chaves do AES-GMAC-SIV, tiradas da chave inteira de 48 bytes.
    pub aes: Arc<gmac_siv::GmacSivKeys>,
}

impl PeerKey {
    /// A chave do par a partir do resultado de [`crate::Identity::agree`].
    pub fn from_agreement(acordo: &[u8; 48]) -> Self {
        let mut salsa = Zeroizing::new([0u8; 32]);
        salsa.copy_from_slice(&acordo[..32]);
        Self {
            salsa,
            aes: Arc::new(gmac_siv::GmacSivKeys::derive(acordo)),
        }
    }
}

/// Compara em tempo constante.
pub(crate) fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}
