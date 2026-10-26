//! A armadura Salsa20/12 + Poly1305 dos pacotes (suítes 0 e 1 do protocolo).
//!
//! A chave de 32 bytes do par é misturada com o cabeçalho de cada pacote (IV, endereços, flags
//! sem os saltos e tamanho), então cada pacote usa uma chave só dele e quem repassa o pacote
//! pode mexer nos saltos sem estragar a autenticação. O primeiro bloco do fluxo dá a chave do
//! Poly1305 e a carga é cifrada a partir do segundo (`Packet::armor`/`dearmor`, `Packet.cpp`).

use poly1305::Poly1305;
use poly1305::universal_hash::KeyInit;
use salsa20::Salsa12;
use salsa20::cipher::{KeyIvInit, StreamCipher};

use crate::crypto::ct_eq;
use crate::packet::{Cipher, MASK_HOPS, idx};
use crate::{Error, Result};

/// Tamanho do MAC no pacote, em bytes: os 8 primeiros da etiqueta de 16 do Poly1305.
pub const MAC_LEN: usize = 8;

/// Mistura a chave do par com o cabeçalho do pacote (`_salsa20MangleKey`, `Packet.hpp`).
pub(crate) fn mangle_key(chave: &[u8; 32], pacote: &[u8]) -> [u8; 32] {
    let mut saida = *chave;
    // IV e os dois endereços: divide o espaço de chaves em A→B e B→A.
    for i in 0..idx::FLAGS {
        saida[i] ^= pacote[i];
    }
    // As flags sem os saltos, que quem repassa altera.
    saida[idx::FLAGS] ^= pacote[idx::FLAGS] & !MASK_HOPS;
    // O tamanho do pacote, em 16 bits little-endian: cada tamanho é um espaço de chaves.
    saida[19] ^= (pacote.len() & 0xff) as u8;
    saida[20] ^= ((pacote.len() >> 8) & 0xff) as u8;
    saida
}

/// O fluxo do pacote: o primeiro bloco já consumido para a chave do Poly1305.
fn fluxo(chave: &[u8; 32], pacote: &[u8]) -> (Salsa12, [u8; 32]) {
    let mangled = mangle_key(chave, pacote);
    let iv: [u8; 8] = pacote[idx::IV..idx::IV + 8].try_into().expect("8 bytes");
    let mut salsa = Salsa12::new(&mangled.into(), &iv.into());
    let mut bloco0 = [0u8; 64];
    salsa.apply_keystream(&mut bloco0);
    let mut chave_mac = [0u8; 32];
    chave_mac.copy_from_slice(&bloco0[..32]);
    (salsa, chave_mac)
}

fn mac(chave_mac: &[u8; 32], carga: &[u8]) -> [u8; MAC_LEN] {
    let etiqueta = Poly1305::new(&(*chave_mac).into()).compute_unpadded(carga);
    let mut saida = [0u8; MAC_LEN];
    saida.copy_from_slice(&etiqueta[..MAC_LEN]);
    saida
}

/// Arma o pacote no lugar: marca a suíte (1 se `cifrar`, 0 se só autentica), cifra a carga e
/// escreve o MAC. As flags precisam estar certas (armadura estendida, fragmentado) antes.
pub fn armor(chave: &[u8; 32], pacote: &mut [u8], cifrar: bool) {
    let cifra = if cifrar {
        Cipher::Salsa2012
    } else {
        Cipher::None
    };
    pacote[idx::FLAGS] =
        (pacote[idx::FLAGS] & !crate::packet::MASK_CIPHER) | (cifra.to_bits() << 3);
    let (mut salsa, chave_mac) = fluxo(chave, pacote);
    if cifrar {
        salsa.apply_keystream(&mut pacote[idx::VERB..]);
    }
    let etiqueta = mac(&chave_mac, &pacote[idx::VERB..]);
    pacote[idx::MAC..idx::MAC + MAC_LEN].copy_from_slice(&etiqueta);
}

/// Cifra (ou decifra, é o mesmo XOR) do byte `inicio` até o fim com o Salsa20/12 sob a chave do
/// par **sem mistura**, com o IV dos 8 primeiros bytes do pacote e os 3 bits baixos zerados
/// (`Packet::cryptField`, `Packet.cpp`). O HELLO usa para esconder o que vem depois da hora e da
/// identidade; o MAC do pacote continua valendo por cima.
pub fn crypt_field(chave: &[u8; 32], pacote: &mut [u8], inicio: usize) {
    let mut iv: [u8; 8] = pacote[idx::IV..idx::IV + 8].try_into().expect("8 bytes");
    // Os 3 bits baixos do ID não valem aqui: o remetente ainda não os decidiu quando cifra.
    iv[7] &= 0xf8;
    let mut salsa = Salsa12::new(&(*chave).into(), &iv.into());
    salsa.apply_keystream(&mut pacote[inicio..]);
}

/// Confere o MAC e, se a suíte for a 1, decifra a carga no lugar. `Err(Authentication)` se o MAC
/// não bater ou a suíte não for 0 nem 1.
pub fn dearmor(chave: &[u8; 32], pacote: &mut [u8]) -> Result<()> {
    if pacote.len() < idx::PAYLOAD {
        return Err(Error::Packet("curto demais".into()));
    }
    let cifra = Cipher::from_bits((pacote[idx::FLAGS] & crate::packet::MASK_CIPHER) >> 3);
    if !matches!(cifra, Cipher::None | Cipher::Salsa2012) {
        return Err(Error::Authentication);
    }
    let (mut salsa, chave_mac) = fluxo(chave, pacote);
    let esperado = mac(&chave_mac, &pacote[idx::VERB..]);
    if !ct_eq(&esperado, &pacote[idx::MAC..idx::MAC + MAC_LEN]) {
        return Err(Error::Authentication);
    }
    if cifra == Cipher::Salsa2012 {
        salsa.apply_keystream(&mut pacote[idx::VERB..]);
    }
    Ok(())
}

#[cfg(test)]
mod testes {
    use super::*;
    use crate::address::Address;
    use crate::packet::{Packet, Verb};

    fn pacote(carga: &[u8]) -> Packet {
        let mut p = Packet::new(
            7,
            Address::new(0x11_2233_4455),
            Address::new(0x66_7788_99aa),
            Verb::Echo,
        );
        p.append(carga);
        p
    }

    #[test]
    fn arma_e_desarma_cifrando() {
        let chave = [9u8; 32];
        let mut p = pacote(b"conteudo secreto do eco");
        let claro = p.clone();
        armor(&chave, p.as_bytes_mut(), true);
        assert_eq!(p.cipher(), Cipher::Salsa2012);
        assert_ne!(p.payload(), claro.payload());
        dearmor(&chave, p.as_bytes_mut()).unwrap();
        assert_eq!(p.payload(), claro.payload());
        assert_eq!(p.verb(), Verb::Echo);
    }

    #[test]
    fn o_campo_cifrado_do_hello_volta_ao_aplicar_de_novo_e_deixa_o_inicio_a_vista() {
        let chave = [3u8; 32];
        let mut p = pacote(b"parte clara|parte cifrada das moons");
        let inicio = idx::PAYLOAD + b"parte clara|".len();
        let original = p.as_bytes().to_vec();
        crypt_field(&chave, p.as_bytes_mut(), inicio);
        assert_eq!(&p.as_bytes()[..inicio], &original[..inicio]);
        assert_ne!(&p.as_bytes()[inicio..], &original[inicio..]);
        crypt_field(&chave, p.as_bytes_mut(), inicio);
        assert_eq!(p.as_bytes(), &original[..]);
    }

    #[test]
    fn sem_cifrar_a_carga_fica_a_vista_mas_autenticada() {
        let chave = [9u8; 32];
        let mut p = pacote(b"visivel");
        armor(&chave, p.as_bytes_mut(), false);
        assert_eq!(p.cipher(), Cipher::None);
        assert_eq!(p.payload(), b"visivel");
        dearmor(&chave, p.as_bytes_mut()).unwrap();
    }

    #[test]
    fn mac_errado_chave_errada_ou_byte_trocado_sao_recusados() {
        let chave = [9u8; 32];
        let mut p = pacote(b"conteudo");
        armor(&chave, p.as_bytes_mut(), true);

        let mut outra = p.clone();
        assert_eq!(
            dearmor(&[8u8; 32], outra.as_bytes_mut()),
            Err(Error::Authentication)
        );

        for posicao in [0, 8, 13, idx::MAC, idx::VERB, p.len() - 1] {
            let mut ruim = p.clone();
            ruim.as_bytes_mut()[posicao] ^= 1;
            assert_eq!(
                dearmor(&chave, ruim.as_bytes_mut()),
                Err(Error::Authentication),
                "byte {posicao}"
            );
        }
    }

    #[test]
    fn os_saltos_podem_mudar_no_caminho_sem_estragar_a_autenticacao() {
        let chave = [9u8; 32];
        let mut p = pacote(b"passa por um raiz");
        armor(&chave, p.as_bytes_mut(), true);
        assert!(p.increment_hops());
        assert!(p.increment_hops());
        dearmor(&chave, p.as_bytes_mut()).unwrap();
        assert_eq!(p.payload(), b"passa por um raiz");
    }
}
