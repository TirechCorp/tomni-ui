//! O transporte (mensagem tipo 4): a chave de sessão, o contador e o preenchimento.
//!
//! Formato (artigo, seção 5.4.6): tipo `u32` LE (4), índice do receptor `u32` LE, contador
//! `u64` LE e o texto cifrado (ChaCha20-Poly1305, nonce de 4 zeros e o contador em LE, sem
//! dado associado). O texto claro é o pacote IP preenchido com zeros até um múltiplo de 16;
//! um pacote vazio é um keepalive (32 bytes na rede).

use std::time::Instant;

use chacha20poly1305::aead::{AeadInOut, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use zeroize::Zeroizing;

use crate::crypto::nonce_from_counter;
use crate::handshake::{SessionKeys, TIPO_TRANSPORTE};
use crate::indices::IndexGuard;
use crate::replay::Replay;
use crate::timers::REJECT_AFTER_MESSAGES;

/// O tamanho do cabeçalho do transporte.
pub const CABECALHO: usize = 16;
/// O tamanho da etiqueta de autenticação.
pub const ETIQUETA: usize = 16;
/// O tamanho de um keepalive na rede.
pub const TAMANHO_KEEPALIVE: usize = CABECALHO + ETIQUETA;

/// Uma chave de sessão com seu estado.
pub(crate) struct Keypair {
    send: Zeroizing<[u8; 32]>,
    recv: Zeroizing<[u8; 32]>,
    /// O nosso índice: vem nos pacotes que recebemos.
    pub(crate) local: IndexGuard,
    /// O índice do outro lado: vai nos pacotes que mandamos.
    pub(crate) remote_index: u32,
    pub(crate) created: Instant,
    pub(crate) is_initiator: bool,
    /// O próximo contador de envio.
    pub(crate) send_counter: u64,
    replay: Replay,
}

impl Keypair {
    pub(crate) fn new(
        chaves: SessionKeys,
        local: IndexGuard,
        remote_index: u32,
        created: Instant,
        is_initiator: bool,
    ) -> Self {
        Keypair {
            send: chaves.send,
            recv: chaves.recv,
            local,
            remote_index,
            created,
            is_initiator,
            send_counter: 0,
            replay: Replay::new(),
        }
    }

    /// Cifra `claro` (vazio para keepalive) em `saida`, com preenchimento até múltiplo de 16
    /// limitado a `mtu`. Devolve `false` se o contador esgotou (a sessão não manda mais).
    pub(crate) fn sealar(&mut self, claro: &[u8], mtu: usize, saida: &mut Vec<u8>) -> bool {
        if self.send_counter >= REJECT_AFTER_MESSAGES {
            return false;
        }
        let contador = self.send_counter;
        self.send_counter += 1;

        let preenchido = if claro.is_empty() {
            0
        } else {
            // Múltiplo de 16, sem passar da MTU (a menos que o pacote já passe).
            let ate16 = claro.len().div_ceil(16) * 16;
            ate16.min(mtu.max(claro.len()))
        };
        saida.clear();
        saida.extend_from_slice(&TIPO_TRANSPORTE.to_le_bytes());
        saida.extend_from_slice(&self.remote_index.to_le_bytes());
        saida.extend_from_slice(&contador.to_le_bytes());
        saida.extend_from_slice(claro);
        saida.resize(CABECALHO + preenchido, 0);

        let cifra = ChaCha20Poly1305::new(&Key::from(*self.send));
        let nonce = Nonce::from(nonce_from_counter(contador));
        let corpo = &mut saida[CABECALHO..];
        let etiqueta = cifra
            .encrypt_inout_detached(&nonce, &[], corpo.into())
            .expect("a cifragem do transporte não falha");
        saida.extend_from_slice(etiqueta.as_slice());
        true
    }

    /// Abre um pacote de transporte inteiro em `claro` (o texto claro, com o preenchimento).
    /// Só marca o contador na janela anti-repetição depois que a etiqueta confere.
    pub(crate) fn abrir(
        &mut self,
        pacote: &[u8],
        claro: &mut Vec<u8>,
    ) -> Result<(), ErroDeTransporte> {
        if pacote.len() < TAMANHO_KEEPALIVE {
            return Err(ErroDeTransporte::Curto);
        }
        let mut c = [0u8; 8];
        c.copy_from_slice(&pacote[8..16]);
        let contador = u64::from_le_bytes(c);
        if contador >= REJECT_AFTER_MESSAGES {
            return Err(ErroDeTransporte::Repetido);
        }
        claro.clear();
        claro.extend_from_slice(&pacote[CABECALHO..]);
        let cifra = ChaCha20Poly1305::new(&Key::from(*self.recv));
        let nonce = Nonce::from(nonce_from_counter(contador));
        cifra
            .decrypt_in_place(&nonce, &[], claro)
            .map_err(|_| ErroDeTransporte::Autenticacao)?;
        if !self.replay.validar(contador, REJECT_AFTER_MESSAGES) {
            return Err(ErroDeTransporte::Repetido);
        }
        Ok(())
    }
}

/// Por que um pacote de transporte foi descartado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ErroDeTransporte {
    /// Menor que um keepalive.
    Curto,
    /// A etiqueta não confere.
    Autenticacao,
    /// Contador repetido ou fora da janela.
    Repetido,
}

#[cfg(test)]
mod testes {
    use super::*;
    use crate::indices::Indices;
    use crate::key::PublicKey;

    fn par_de_chaves() -> (Keypair, Keypair, Indices) {
        let reg = Indices::new();
        let pk = PublicKey::from_bytes([9; 32]);
        let ia = reg.allocate(pk).unwrap();
        let ib = reg.allocate(pk).unwrap();
        let (ra, rb) = (ia.id(), ib.id());
        let agora = Instant::now();
        let a = Keypair::new(
            SessionKeys {
                send: Zeroizing::new([1; 32]),
                recv: Zeroizing::new([2; 32]),
            },
            ia,
            rb,
            agora,
            true,
        );
        let b = Keypair::new(
            SessionKeys {
                send: Zeroizing::new([2; 32]),
                recv: Zeroizing::new([1; 32]),
            },
            ib,
            ra,
            agora,
            false,
        );
        (a, b, reg)
    }

    #[test]
    fn o_pacote_vai_com_preenchimento_ate_multiplo_de_16() {
        let (mut a, mut b, _reg) = par_de_chaves();
        let ip = vec![0x45u8; 21];
        let mut rede = Vec::new();
        assert!(a.sealar(&ip, 1420, &mut rede));
        // Cabeçalho 16 + 32 (21 preenchido) + etiqueta 16.
        assert_eq!(rede.len(), 16 + 32 + 16);
        assert_eq!(&rede[0..4], &[4, 0, 0, 0]);
        assert_eq!(&rede[4..8], &b.local.id().to_le_bytes());
        assert_eq!(&rede[8..16], &0u64.to_le_bytes());
        let mut claro = Vec::new();
        b.abrir(&rede, &mut claro).unwrap();
        assert_eq!(claro.len(), 32);
        assert_eq!(&claro[..21], &ip[..]);
        assert!(claro[21..].iter().all(|&x| x == 0));
    }

    #[test]
    fn o_keepalive_tem_32_bytes_na_rede() {
        let (mut a, mut b, _reg) = par_de_chaves();
        let mut rede = Vec::new();
        assert!(a.sealar(&[], 1420, &mut rede));
        assert_eq!(rede.len(), TAMANHO_KEEPALIVE);
        let mut claro = Vec::new();
        b.abrir(&rede, &mut claro).unwrap();
        assert!(claro.is_empty());
    }

    #[test]
    fn o_nonce_e_quatro_zeros_e_o_contador_em_little_endian() {
        // Vetor fixo: o mesmo pacote cifrado à mão com o ChaCha20-Poly1305 puro, nonce
        // 00 00 00 00 + contador LE, sem dado associado.
        use chacha20poly1305::aead::Aead;
        let (mut a, _b, _reg) = par_de_chaves();
        a.send_counter = 0x0102;
        let mut rede = Vec::new();
        assert!(a.sealar(&[0xaa; 16], 1420, &mut rede));
        let cifra = ChaCha20Poly1305::new(&Key::from([1u8; 32]));
        let mut nonce = [0u8; 12];
        nonce[4..].copy_from_slice(&0x0102u64.to_le_bytes());
        assert_eq!(&nonce[..], &[0, 0, 0, 0, 2, 1, 0, 0, 0, 0, 0, 0]);
        let esperado = cifra.encrypt(&Nonce::from(nonce), &[0xaa; 16][..]).unwrap();
        assert_eq!(&rede[16..], &esperado[..]);
        assert_eq!(&rede[8..16], &nonce[4..]);
    }

    #[test]
    fn o_preenchimento_nao_passa_da_mtu() {
        let (mut a, _b, _reg) = par_de_chaves();
        let mut rede = Vec::new();
        // 1417 bytes: o múltiplo de 16 seguinte é 1424, mas a MTU é 1420.
        assert!(a.sealar(&vec![1u8; 1417], 1420, &mut rede));
        assert_eq!(rede.len(), 16 + 1420 + 16);
    }

    #[test]
    fn repeticao_e_adulteracao_sao_recusadas() {
        let (mut a, mut b, _reg) = par_de_chaves();
        let mut rede = Vec::new();
        a.sealar(&[7; 40], 1420, &mut rede);
        let mut claro = Vec::new();
        b.abrir(&rede, &mut claro).unwrap();
        assert_eq!(b.abrir(&rede, &mut claro), Err(ErroDeTransporte::Repetido));
        let mut adulterado = rede.clone();
        adulterado[20] ^= 1;
        let mut c = Vec::new();
        a.sealar(&[7; 40], 1420, &mut c);
        c[20] ^= 1;
        assert_eq!(b.abrir(&c, &mut claro), Err(ErroDeTransporte::Autenticacao));
        // Um pacote adulterado não gasta o contador: o legítimo seguinte ainda passa.
        let mut d = Vec::new();
        a.sealar(&[7; 40], 1420, &mut d);
        b.abrir(&d, &mut claro).unwrap();
        assert_eq!(
            b.abrir(&rede[..20], &mut claro),
            Err(ErroDeTransporte::Curto)
        );
    }

    #[test]
    fn o_contador_esgotado_nao_manda_mais() {
        let (mut a, _b, _reg) = par_de_chaves();
        a.send_counter = REJECT_AFTER_MESSAGES;
        let mut rede = Vec::new();
        assert!(!a.sealar(&[1; 16], 1420, &mut rede));
    }
}
