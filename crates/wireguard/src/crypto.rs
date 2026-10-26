//! As primitivas do WireGuard (artigo, seção 5.4): `Hash`, `Mac`, `Hmac`, `Kdf` e `Aead`.
//!
//! O WireGuard usa BLAKE2s para tudo: o hash de 32 bytes, o MAC de 16 bytes (BLAKE2s com chave)
//! e o HMAC-BLAKE2s do KDF. A caixa autenticada é o ChaCha20-Poly1305, e o cookie usa o
//! XChaCha20-Poly1305.

use blake2::digest::consts::U16;
use blake2::digest::{FixedOutput, Update};
use blake2::{Blake2s256, Blake2sMac, Digest};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce, XChaCha20Poly1305, XNonce};
use zeroize::Zeroizing;

/// `Hash(a ‖ b ‖ …)`: BLAKE2s de 32 bytes.
pub(crate) fn hash(partes: &[&[u8]]) -> [u8; 32] {
    let mut h = Blake2s256::new();
    for p in partes {
        Digest::update(&mut h, p);
    }
    h.finalize().into()
}

/// `Mac(chave, a ‖ b ‖ …)`: BLAKE2s com chave e saída de 16 bytes. A chave tem de 1 a 32
/// bytes (o `mac1` usa 32, o `mac2` usa o cookie de 16).
pub(crate) fn mac16(chave: &[u8], partes: &[&[u8]]) -> [u8; 16] {
    // O tamanho da chave é sempre 16 ou 32 aqui; `new_with_salt_and_personal` só falha com
    // mais de 32 bytes.
    let mut m = Blake2sMac::<U16>::new_with_salt_and_personal(Some(chave), &[], &[])
        .expect("chave do MAC com no máximo 32 bytes");
    for p in partes {
        Update::update(&mut m, p);
    }
    m.finalize_fixed().into()
}

/// O HMAC-BLAKE2s com chave de 32 bytes (menor que o bloco de 64 do BLAKE2s).
pub(crate) fn hmac(chave: &[u8; 32], partes: &[&[u8]]) -> [u8; 32] {
    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for i in 0..32 {
        ipad[i] ^= chave[i];
        opad[i] ^= chave[i];
    }
    let mut interno = Blake2s256::new();
    Digest::update(&mut interno, ipad);
    for p in partes {
        Digest::update(&mut interno, p);
    }
    let ih: [u8; 32] = interno.finalize().into();
    let mut externo = Blake2s256::new();
    Digest::update(&mut externo, opad);
    Digest::update(&mut externo, ih);
    externo.finalize().into()
}

/// `Kdf1(chave, entrada)`.
pub(crate) fn kdf1(chave: &[u8; 32], entrada: &[u8]) -> Zeroizing<[u8; 32]> {
    let t0 = Zeroizing::new(hmac(chave, &[entrada]));
    Zeroizing::new(hmac(&t0, &[&[1]]))
}

/// `Kdf2(chave, entrada)`.
pub(crate) fn kdf2(chave: &[u8; 32], entrada: &[u8]) -> (Zeroizing<[u8; 32]>, Zeroizing<[u8; 32]>) {
    let t0 = Zeroizing::new(hmac(chave, &[entrada]));
    let t1 = Zeroizing::new(hmac(&t0, &[&[1]]));
    let t2 = Zeroizing::new(hmac(&t0, &[&t1[..], &[2]]));
    (t1, t2)
}

/// `Kdf3(chave, entrada)`.
#[allow(clippy::type_complexity)]
pub(crate) fn kdf3(
    chave: &[u8; 32],
    entrada: &[u8],
) -> (
    Zeroizing<[u8; 32]>,
    Zeroizing<[u8; 32]>,
    Zeroizing<[u8; 32]>,
) {
    let t0 = Zeroizing::new(hmac(chave, &[entrada]));
    let t1 = Zeroizing::new(hmac(&t0, &[&[1]]));
    let t2 = Zeroizing::new(hmac(&t0, &[&t1[..], &[2]]));
    let t3 = Zeroizing::new(hmac(&t0, &[&t2[..], &[3]]));
    (t1, t2, t3)
}

/// O nonce do ChaCha20-Poly1305 do WireGuard: 4 zeros e o contador de 64 bits em
/// little-endian (artigo, seção 5.4.6).
pub(crate) fn nonce_from_counter(contador: u64) -> [u8; 12] {
    let mut n = [0u8; 12];
    n[4..].copy_from_slice(&contador.to_le_bytes());
    n
}

/// O `Aead(chave, contador, texto, dado associado)` do handshake: ChaCha20-Poly1305. Devolve
/// o texto cifrado com a etiqueta de 16 bytes no fim.
pub(crate) fn aead_seal(chave: &[u8; 32], contador: u64, texto: &[u8], aad: &[u8]) -> Vec<u8> {
    let cifra = ChaCha20Poly1305::new(&Key::from(*chave));
    cifra
        .encrypt(
            &Nonce::from(nonce_from_counter(contador)),
            Payload { msg: texto, aad },
        )
        .expect("a cifragem do ChaCha20-Poly1305 não falha")
}

/// A abertura do `Aead`; `None` se a etiqueta não confere.
pub(crate) fn aead_open(
    chave: &[u8; 32],
    contador: u64,
    cifrado: &[u8],
    aad: &[u8],
) -> Option<Zeroizing<Vec<u8>>> {
    let cifra = ChaCha20Poly1305::new(&Key::from(*chave));
    cifra
        .decrypt(
            &Nonce::from(nonce_from_counter(contador)),
            Payload { msg: cifrado, aad },
        )
        .ok()
        .map(Zeroizing::new)
}

/// O `Xaead` do cookie: XChaCha20-Poly1305 com nonce de 24 bytes.
pub(crate) fn xaead_seal(chave: &[u8; 32], nonce: &[u8; 24], texto: &[u8], aad: &[u8]) -> Vec<u8> {
    let cifra = XChaCha20Poly1305::new(&Key::from(*chave));
    cifra
        .encrypt(&XNonce::from(*nonce), Payload { msg: texto, aad })
        .expect("a cifragem do XChaCha20-Poly1305 não falha")
}

/// A abertura do `Xaead`; `None` se a etiqueta não confere.
pub(crate) fn xaead_open(
    chave: &[u8; 32],
    nonce: &[u8; 24],
    cifrado: &[u8],
    aad: &[u8],
) -> Option<Zeroizing<Vec<u8>>> {
    let cifra = XChaCha20Poly1305::new(&Key::from(*chave));
    cifra
        .decrypt(&XNonce::from(*nonce), Payload { msg: cifrado, aad })
        .ok()
        .map(Zeroizing::new)
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn o_hmac_blake2s_segue_a_construcao_da_rfc_2104() {
        // A conferência cruzada com o `hmac` do Python fica no teste de interoperabilidade
        // (o handshake com o `boringtun` só fecha se o KDF estiver certo); aqui conferimos
        // a construção ipad/opad contra o hash simples.
        let chave = [0x0bu8; 32];
        let r = hmac(&chave, &[b"abc"]);
        let mut ipad = [0x36u8; 64];
        let mut opad = [0x5cu8; 64];
        for i in 0..32 {
            ipad[i] ^= chave[i];
            opad[i] ^= chave[i];
        }
        let interno = hash(&[&ipad, b"abc"]);
        let externo = hash(&[&opad, &interno]);
        assert_eq!(r, externo);
    }

    #[test]
    fn o_nonce_do_transporte_e_quatro_zeros_e_o_contador_little_endian() {
        let n = nonce_from_counter(0x0102_0304_0506_0708);
        assert_eq!(n, [0, 0, 0, 0, 8, 7, 6, 5, 4, 3, 2, 1]);
    }

    #[test]
    fn o_aead_vai_e_volta_e_recusa_dado_associado_diferente() {
        let chave = [9u8; 32];
        let c = aead_seal(&chave, 5, b"ola", b"cabecalho");
        assert_eq!(c.len(), 3 + 16);
        assert_eq!(&aead_open(&chave, 5, &c, b"cabecalho").unwrap()[..], b"ola");
        assert!(aead_open(&chave, 5, &c, b"outro").is_none());
        assert!(aead_open(&chave, 6, &c, b"cabecalho").is_none());
    }

    #[test]
    fn o_xaead_do_cookie_vai_e_volta() {
        let chave = [3u8; 32];
        let nonce = [4u8; 24];
        let c = xaead_seal(&chave, &nonce, &[1u8; 16], b"mac1");
        assert_eq!(c.len(), 32);
        assert_eq!(
            &xaead_open(&chave, &nonce, &c, b"mac1").unwrap()[..],
            &[1u8; 16]
        );
        assert!(xaead_open(&chave, &nonce, &c, b"mac2").is_none());
    }

    #[test]
    fn o_mac16_depende_da_chave_e_tem_16_bytes() {
        let a = mac16(&[1u8; 32], &[b"dado"]);
        let b = mac16(&[2u8; 32], &[b"dado"]);
        let c = mac16(&[1u8; 16], &[b"da", b"do"]);
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_eq!(mac16(&[1u8; 16], &[b"dado"]), c);
    }

    #[test]
    fn o_kdf_encadeia_como_o_artigo() {
        let chave = [5u8; 32];
        let (a, b) = kdf2(&chave, b"x");
        let a1 = kdf1(&chave, b"x");
        assert_eq!(*a, *a1);
        let (a3, b3, _) = kdf3(&chave, b"x");
        assert_eq!(*a, *a3);
        assert_eq!(*b, *b3);
    }
}
