//! AES-GMAC-SIV, a suíte 3 dos pacotes (protocolo 12 em diante). Não é o AES-GCM-SIV do RFC 8452:
//! é o esquema do ZeroTier (`AES::GMACSIVEncryptor` e `Decryptor`), montado com GHASH, um bloco
//! de AES e AES-CTR.
//!
//! Das 48 bytes da chave do par saem duas chaves AES-256 (`K0` e `K1`). O GMAC com `K0` cobre o
//! cabeçalho (destino, origem e flags, sem os saltos) e a carga em claro; a etiqueta de 16 bytes
//! é encurtada para 8, junta-se ao ID do pacote e o bloco, cifrado com `K1`, é a etiqueta SIV: a
//! primeira metade vai no lugar do ID, a segunda no lugar do MAC. A carga é cifrada em CTR com
//! `K1`, tendo a etiqueta SIV como contador inicial.

use aes::Aes256;
use aes::cipher::{BlockCipherDecrypt, BlockCipherEncrypt, InnerIvInit, KeyInit, StreamCipher};
use ctr::cipher::StreamCipherCoreWrapper;
use ctr::{Ctr32BE, CtrCore, flavors};
use ghash::GHash;
use ghash::universal_hash::UniversalHash;
use hmac::{Hmac, Mac};
use sha2::Sha384;

use crate::crypto::ct_eq;
use crate::packet::{Cipher, MASK_CIPHER, MASK_HOPS, idx};
use crate::{Error, Result};

/// O AAD: destino, origem e flags (`ZT_PACKET_IDX_DEST` até antes do MAC).
const AAD_LEN: usize = 11;

/// As chaves AES de um par.
#[derive(Clone)]
pub struct GmacSivKeys {
    k0: Aes256,
    k1: Aes256,
    /// A chave do GHASH: `K0` aplicada ao bloco zero.
    h: [u8; 16],
}

/// O `KBKDFHMACSHA384` do ZeroTier com o rótulo de um caractere, o contexto 0 e a saída de 384
/// bits: HMAC-SHA384 sobre `iter (4, big-endian) 'Z' 'T' rótulo 0x00 contexto 0x00 0x00 0x01 0x80`.
fn kbkdf(chave: &[u8; 48], rotulo: u8) -> [u8; 32] {
    let mut mac =
        <Hmac<Sha384> as hmac::KeyInit>::new_from_slice(chave).expect("HMAC aceita qualquer chave");
    mac.update(&0u32.to_be_bytes());
    mac.update(&[b'Z', b'T', rotulo, 0, 0, 0, 0, 0x01, 0x80]);
    let saida = mac.finalize().into_bytes();
    let mut k = [0u8; 32];
    k.copy_from_slice(&saida[..32]);
    k
}

impl GmacSivKeys {
    /// As chaves a partir da chave do par de 48 bytes.
    pub fn derive(chave_do_par: &[u8; 48]) -> Self {
        let k0 = Aes256::new(&kbkdf(chave_do_par, b'0').into());
        let k1 = Aes256::new(&kbkdf(chave_do_par, b'1').into());
        let mut h = [0u8; 16].into();
        k0.encrypt_block(&mut h);
        Self {
            k0,
            k1,
            h: h.into(),
        }
    }

    /// O GMAC de `cabecalho` (preenchido com zeros até 16) e `dados`, com o IV de 12 bytes = `id`
    /// e quatro zeros. O ZeroTier trata tudo como uma só cadeia de dados: o tamanho do bloco final
    /// é o da cadeia inteira, com o cabeçalho já preenchido.
    fn gmac(&self, id: &[u8; 8], cabecalho: &[u8; AAD_LEN], dados: &[u8]) -> [u8; 16] {
        let mut gh = GHash::new(&self.h.into());
        gh.update_padded(cabecalho);
        gh.update_padded(dados);
        let total = (16 + dados.len()) as u64 * 8;
        let mut tamanho = [0u8; 16];
        tamanho[..8].copy_from_slice(&total.to_be_bytes());
        gh.update(&[tamanho.into()]);
        let mut tag: [u8; 16] = gh.finalize().into();
        // O contador J0 do GCM: o IV de 96 bits e o contador 1.
        let mut j0 = [0u8; 16];
        j0[..8].copy_from_slice(id);
        j0[15] = 1;
        let mut bloco = j0.into();
        self.k0.encrypt_block(&mut bloco);
        for (t, b) in tag.iter_mut().zip(bloco.iter()) {
            *t ^= b;
        }
        tag
    }
}

/// A etiqueta de 16 bytes encurtada para 8: as duas metades com XOR.
fn encurtar(tag: &[u8; 16]) -> [u8; 8] {
    let mut curta = [0u8; 8];
    for i in 0..8 {
        curta[i] = tag[i] ^ tag[i + 8];
    }
    curta
}

fn cabecalho(pacote: &[u8]) -> [u8; AAD_LEN] {
    let mut aad = [0u8; AAD_LEN];
    aad.copy_from_slice(&pacote[idx::DEST..idx::MAC]);
    // Quem repassa mexe nos saltos: eles não entram.
    aad[AAD_LEN - 1] &= !MASK_HOPS;
    aad
}

/// O AES-CTR de `K1` com o contador de 32 bits big-endian no fim do bloco.
fn fluxo(k1: &Aes256, iv: &[u8; 16]) -> Ctr32BE<Aes256> {
    StreamCipherCoreWrapper::from_core(CtrCore::<Aes256, flavors::Ctr32BE>::inner_iv_init(
        k1.clone(),
        &(*iv).into(),
    ))
}

fn contador(etiqueta: &[u8; 16]) -> [u8; 16] {
    let mut iv = *etiqueta;
    // O bit mais alto do contador de 32 bits fica zerado (`& 0xffffffff7fffffff`).
    iv[12] &= 0x7f;
    iv
}

/// Arma o pacote no lugar: marca a suíte 3, escreve a etiqueta SIV (o ID do pacote passa a ser a
/// primeira metade dela) e cifra a carga. As flags (fragmentado) precisam estar certas antes.
pub fn armor(chaves: &GmacSivKeys, pacote: &mut [u8]) {
    pacote[idx::FLAGS] = (pacote[idx::FLAGS] & !MASK_CIPHER) | (Cipher::AesGmacSiv.to_bits() << 3);
    let id: [u8; 8] = pacote[idx::IV..idx::IV + 8].try_into().expect("8 bytes");
    let aad = cabecalho(pacote);
    let curta = encurtar(&chaves.gmac(&id, &aad, &pacote[idx::VERB..]));
    let mut bloco = [0u8; 16];
    bloco[..8].copy_from_slice(&id);
    bloco[8..].copy_from_slice(&curta);
    let mut siv = bloco.into();
    chaves.k1.encrypt_block(&mut siv);
    let etiqueta: [u8; 16] = siv.into();
    let mut ctr = fluxo(&chaves.k1, &contador(&etiqueta));
    ctr.apply_keystream(&mut pacote[idx::VERB..]);
    pacote[idx::IV..idx::IV + 8].copy_from_slice(&etiqueta[..8]);
    pacote[idx::MAC..idx::MAC + 8].copy_from_slice(&etiqueta[8..]);
}

/// Decifra a carga no lugar e confere a etiqueta. `Err(Authentication)` se não fechar. O ID do
/// pacote continua sendo o que viajou (a metade da etiqueta): é a ele que as respostas se referem.
pub fn dearmor(chaves: &GmacSivKeys, pacote: &mut [u8]) -> Result<()> {
    if pacote.len() < idx::PAYLOAD {
        return Err(Error::Packet("curto demais".into()));
    }
    let mut etiqueta = [0u8; 16];
    etiqueta[..8].copy_from_slice(&pacote[idx::IV..idx::IV + 8]);
    etiqueta[8..].copy_from_slice(&pacote[idx::MAC..idx::MAC + 8]);
    let aad = cabecalho(pacote);
    let mut ctr = fluxo(&chaves.k1, &contador(&etiqueta));
    ctr.apply_keystream(&mut pacote[idx::VERB..]);
    let mut bloco = etiqueta.into();
    chaves.k1.decrypt_block(&mut bloco);
    let bloco: [u8; 16] = bloco.into();
    let id: [u8; 8] = bloco[..8].try_into().expect("8 bytes");
    let esperada = encurtar(&chaves.gmac(&id, &aad, &pacote[idx::VERB..]));
    if ct_eq(&esperada, &bloco[8..]) {
        Ok(())
    } else {
        Err(Error::Authentication)
    }
}
