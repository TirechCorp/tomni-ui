//! A caixa NaCl (`crypto_box`): curve25519, HSalsa20 e XSalsa20-Poly1305. É a cifra do
//! `ClientInfo` e do `ServerInfo` do DERP e das mensagens do disco.
//!
//! O formato no fio é `nonce (24 bytes) || caixa`, o mesmo do `box.Seal` do Go com o nonce na
//! frente.

use crypto_box::aead::generic_array::GenericArray;
use crypto_box::aead::{Aead, AeadCore};
use crypto_box::{PublicKey, SalsaBox, SecretKey};

use crate::key::KEY_LEN;

/// O tamanho do nonce.
pub const NONCE_LEN: usize = 24;
/// O que a caixa acrescenta ao texto: a etiqueta do Poly1305.
pub const OVERHEAD: usize = 16;

/// Uma caixa entre a nossa chave privada e a pública do outro lado (a chave compartilhada é
/// calculada uma vez).
pub struct NaclBox {
    caixa: SalsaBox,
}

impl std::fmt::Debug for NaclBox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NaclBox(..)")
    }
}

impl NaclBox {
    /// Monta a caixa com os 32 bytes da nossa chave privada e os 32 da pública do outro lado.
    pub fn new(privada: &[u8; KEY_LEN], publica: &[u8; KEY_LEN]) -> Self {
        let sk = SecretKey::from(*privada);
        let pk = PublicKey::from(*publica);
        NaclBox {
            caixa: SalsaBox::new(&pk, &sk),
        }
    }

    /// Cifra com um nonce aleatório; devolve `nonce || caixa`.
    pub fn seal(&self, claro: &[u8]) -> Vec<u8> {
        let mut nonce = [0u8; NONCE_LEN];
        getrandom::fill(&mut nonce).expect("o sistema deu números aleatórios");
        self.seal_with(&nonce, claro)
    }

    /// Cifra com o nonce dado (para os testes com vetor fixo).
    pub fn seal_with(&self, nonce: &[u8; NONCE_LEN], claro: &[u8]) -> Vec<u8> {
        let n = GenericArray::from_slice(nonce);
        let caixa = self
            .caixa
            .encrypt(n, claro)
            .expect("cifrar na memória não falha");
        let mut saida = Vec::with_capacity(NONCE_LEN + caixa.len());
        saida.extend_from_slice(nonce);
        saida.extend_from_slice(&caixa);
        saida
    }

    /// Abre `nonce || caixa`. `None` se for curto demais ou a etiqueta não confere.
    pub fn open(&self, dados: &[u8]) -> Option<Vec<u8>> {
        if dados.len() < NONCE_LEN + OVERHEAD {
            return None;
        }
        let (nonce, caixa) = dados.split_at(NONCE_LEN);
        self.caixa
            .decrypt(GenericArray::from_slice(nonce), caixa)
            .ok()
    }
}

// O tipo do nonce do `SalsaBox` tem de ter 24 bytes.
const _: () = assert!(<SalsaBox as AeadCore>::NonceSize::USIZE == NONCE_LEN);
use crypto_box::aead::generic_array::typenum::Unsigned as _;

#[cfg(test)]
mod testes {
    use super::*;
    use crate::key::NodeKey;

    #[test]
    fn as_duas_pontas_se_entendem() {
        let a = NodeKey::generate();
        let b = NodeKey::generate();
        let ab = NaclBox::new(&a.to_bytes(), b.public().as_bytes());
        let ba = NaclBox::new(&b.to_bytes(), a.public().as_bytes());
        let caixa = ab.seal(b"ola, tailnet");
        assert_eq!(caixa.len(), NONCE_LEN + OVERHEAD + 12);
        assert_eq!(ba.open(&caixa).unwrap(), b"ola, tailnet");
    }

    #[test]
    fn caixa_adulterada_ou_curta_nao_abre() {
        let a = NodeKey::generate();
        let b = NodeKey::generate();
        let ab = NaclBox::new(&a.to_bytes(), b.public().as_bytes());
        let ba = NaclBox::new(&b.to_bytes(), a.public().as_bytes());
        let mut caixa = ab.seal(b"x");
        let ultimo = caixa.len() - 1;
        caixa[ultimo] ^= 1;
        assert!(ba.open(&caixa).is_none());
        assert!(ba.open(&caixa[..30]).is_none());
        let c = NodeKey::generate();
        let ca = NaclBox::new(&c.to_bytes(), a.public().as_bytes());
        assert!(ca.open(&ab.seal(b"y")).is_none());
    }

    /// Vetor gerado pelo PyNaCl (libsodium), que é independente deste código: chave privada
    /// `1..=32`, chave pública derivada de `101..=132`, nonce `200..=223`.
    #[test]
    fn vetor_fixo_do_nacl() {
        let privada: [u8; 32] = std::array::from_fn(|i| i as u8 + 1);
        let publica: [u8; 32] = [
            87, 20, 118, 157, 17, 107, 247, 100, 54, 174, 116, 188, 121, 61, 44, 48, 173, 25, 3,
            197, 154, 197, 39, 56, 5, 199, 226, 105, 139, 65, 12, 54,
        ];
        let nonce: [u8; 24] = std::array::from_fn(|i| i as u8 + 200);
        let esperado = "c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfc03368b8a88d085a4eef09bae5261971bd9844309cca72bd5d9b18119e";
        let caixa = NaclBox::new(&privada, &publica).seal_with(&nonce, br#"{"version":2}"#);
        let hex: String = caixa.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, esperado);
        // O outro lado (a chave privada de 101..=132) abre.
        let priv_b: [u8; 32] = std::array::from_fn(|i| i as u8 + 101);
        let pub_a = crate::key::NodeKey::from_bytes(privada).public();
        let aberta = NaclBox::new(&priv_b, pub_a.as_bytes())
            .open(&caixa)
            .unwrap();
        assert_eq!(aberta, br#"{"version":2}"#);
    }
}
