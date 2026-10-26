//! A armadura estendida (protocolo 12 em diante): o pacote inteiro depois das flags, MAC
//! incluído, é cifrado com AES-256-CTR sob uma chave efêmera, e a chave pública efêmera
//! (X25519, 32 bytes) vai no fim. Hoje só o HELLO usa, com a suíte 0, para esconder a
//! identidade de quem o manda (`Packet::armor`/`dearmor`, `Packet.cpp`).
//!
//! A chave do AES é o SHA-512 do acordo X25519 entre a efêmera e a chave pública do destino
//! (os 32 primeiros bytes); o IV do CTR são os 12 primeiros bytes do pacote (o ID e o começo do
//! endereço de destino), com o contador de 32 bits zerado.

use aes::Aes256;
use ctr::Ctr32BE;
use ctr::cipher::{KeyIvInit, StreamCipher};
use zeroize::Zeroizing;

use crate::identity::{Identity, agree_hash, x25519_public};
use crate::packet::{FLAG_EXTENDED_ARMOR, idx};
use crate::{Error, Result};

/// Tamanho da chave pública efêmera no fim do pacote (`ZT_ECC_EPHEMERAL_PUBLIC_KEY_LEN`).
pub const EPHEMERAL_LEN: usize = 32;

/// Tamanho do IV do CTR tirado do começo do pacote.
const IV_BYTES: usize = 12;

fn cifrar_ctr(chave: &[u8; 32], pacote: &mut [u8], fim: usize) {
    let mut iv = [0u8; 16];
    iv[..IV_BYTES].copy_from_slice(&pacote[..IV_BYTES]);
    let mut ctr = Ctr32BE::<Aes256>::new(&(*chave).into(), &iv.into());
    ctr.apply_keystream(&mut pacote[idx::MAC..fim]);
}

/// Aplica a armadura estendida a um pacote **já armado** com a suíte 0: liga a flag? Não: a
/// flag precisa estar ligada **antes** da armadura Salsa, porque entra na mistura da chave.
/// Aqui só se cifra o pacote e se acrescenta a chave efêmera, usando a chave pública de `para`.
pub fn extended_armor(pacote: &mut Vec<u8>, para: &Identity) -> Result<()> {
    if pacote[idx::FLAGS] & FLAG_EXTENDED_ARMOR == 0 {
        return Err(Error::Packet(
            "faltou ligar a armadura estendida antes".into(),
        ));
    }
    let mut secreta = Zeroizing::new([0u8; 32]);
    getrandom::fill(&mut secreta[..])
        .map_err(|e| Error::Packet(format!("sem aleatoriedade: {e}")))?;
    let publica = x25519_public(&secreta);
    let destino: [u8; 32] = para.public_bytes()[..32].try_into().expect("32 bytes");
    let chave = Zeroizing::new(chave_do_aes(&agree_hash(&secreta, &destino)));
    let fim = pacote.len();
    cifrar_ctr(&chave, pacote, fim);
    pacote.extend_from_slice(&publica);
    Ok(())
}

/// Tira a armadura estendida de um pacote recebido, usando a nossa chave secreta: decifra do MAC
/// até antes da chave efêmera e remove esta do fim. Depois disso o pacote é o que a armadura
/// Salsa produziu e [`crate::crypto::salsa::dearmor`] o confere.
pub fn extended_dearmor(pacote: &mut Vec<u8>, nossa: &Identity) -> Result<()> {
    if pacote.len() < idx::PAYLOAD + EPHEMERAL_LEN {
        return Err(Error::Packet(
            "curto demais para a armadura estendida".into(),
        ));
    }
    let secreta = nossa.secret_x25519().ok_or(Error::Authentication)?;
    let fim = pacote.len() - EPHEMERAL_LEN;
    let efemera: [u8; 32] = pacote[fim..].try_into().expect("32 bytes");
    let chave = Zeroizing::new(chave_do_aes(&agree_hash(&secreta, &efemera)));
    cifrar_ctr(&chave, pacote, fim);
    pacote.truncate(fim);
    Ok(())
}

fn chave_do_aes(hash: &[u8; 64]) -> [u8; 32] {
    let mut chave = [0u8; 32];
    chave.copy_from_slice(&hash[..32]);
    chave
}

#[cfg(test)]
mod testes {
    use super::*;
    use crate::address::Address;
    use crate::crypto::salsa;
    use crate::packet::{Cipher, Packet, Verb};

    /// Uma das identidades que o `zerotier-idtool` gerou (as fixtures dos testes).
    fn identidade(n: u32) -> Identity {
        let texto = match n {
            1 => include_str!("../../tests/fixtures/identidades/id1.secret"),
            _ => include_str!("../../tests/fixtures/identidades/id2.secret"),
        };
        Identity::from_secret_str(texto).unwrap()
    }

    #[test]
    fn ida_e_volta_da_armadura_estendida() {
        let destino = Identity::generate().unwrap();
        let mut p = Packet::new(
            1,
            destino.address(),
            Address::new(0x66_7788_99aa),
            Verb::Hello,
        );
        p.append(b"identidade do remetente e outros campos do hello");
        let claro = p.clone();

        p.set_extended_armor(true);
        salsa::armor(&[3u8; 32], p.as_bytes_mut(), false);
        extended_armor(p.as_bytes_mut(), &destino).unwrap();
        assert_eq!(p.len(), claro.len() + EPHEMERAL_LEN);
        assert!(p.extended_armor());
        assert_eq!(p.cipher(), Cipher::None);
        assert!(!p.as_bytes().windows(8).any(|w| w == b"remetente"));

        extended_dearmor(p.as_bytes_mut(), &destino).unwrap();
        salsa::dearmor(&[3u8; 32], p.as_bytes_mut()).unwrap();
        assert_eq!(p.payload(), claro.payload());
    }

    #[test]
    fn com_a_chave_errada_o_mac_nao_confere() {
        let destino = identidade(1);
        let outro = identidade(2);
        let mut p = Packet::new(
            1,
            destino.address(),
            Address::new(0x66_7788_99aa),
            Verb::Hello,
        );
        p.append(b"hello");
        p.set_extended_armor(true);
        salsa::armor(&[3u8; 32], p.as_bytes_mut(), false);
        extended_armor(p.as_bytes_mut(), &destino).unwrap();
        extended_dearmor(p.as_bytes_mut(), &outro).unwrap();
        assert!(salsa::dearmor(&[3u8; 32], p.as_bytes_mut()).is_err());
    }
}
