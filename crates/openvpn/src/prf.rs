//! A derivação das chaves do canal de dados.
//!
//! Depois do TLS, os dois lados precisam de chaves para o canal de dados (que não passa pelo
//! TLS). O OpenVPN tem dois jeitos de obtê-las, e ambos terminam num bloco de 256 bytes:
//!
//! 1. **PRF do OpenVPN** (o padrão): os dois lados trocam números aleatórios dentro do TLS
//!    (a troca de chaves "key-method 2", em [`crate::keymethod`]) e os passam pela PRF do TLS 1.0
//!    (`P_MD5 XOR P_SHA1`), em duas etapas: um segredo mestre de 48 bytes e depois o bloco.
//! 2. **Exportador do TLS** (`key-derivation tls-ekm`, que o servidor pede no `PUSH_REPLY`): o
//!    bloco sai direto do `export_keying_material` do TLS, com o rótulo [`EKM_LABEL`], sem
//!    contexto.
//!
//! O bloco tem quatro pedaços de 64 bytes: chave de cifra e chave de HMAC do sentido
//! cliente→servidor, e as mesmas do sentido servidor→cliente. Cada cifra usa o começo do
//! pedaço (32 bytes para o AES-256 e o ChaCha20-Poly1305); no modo AEAD, os 8 primeiros bytes
//! do pedaço de HMAC viram a parte fixa do nonce.

use hmac::{Hmac, KeyInit, Mac};
use md5::Md5;
use sha1::Sha1;
use zeroize::Zeroizing;

use crate::packet::SessionId;
use crate::wrap::Role;

/// O rótulo do exportador do TLS para as chaves do canal de dados (formato do protocolo).
pub const EKM_LABEL: &str = "EXPORTER-OpenVPN-datakeys";

/// O tamanho do bloco de chaves (formato do protocolo).
pub const KEY_BLOCK_LEN: usize = 256;

/// O tamanho de cada pedaço do bloco (formato do protocolo).
pub const KEY_PART_LEN: usize = 64;

/// O tamanho do segredo "pre-master" que o cliente envia (formato do protocolo).
pub const PRE_MASTER_LEN: usize = 48;

/// O tamanho de cada número aleatório trocado (formato do protocolo).
pub const RANDOM_LEN: usize = 32;

const MASTER_LABEL: &[u8] = b"OpenVPN master secret";
const EXPANSION_LABEL: &[u8] = b"OpenVPN key expansion";

/// `P_hash` da RFC 2246 com o HMAC `M`: `HMAC(segredo, A(i) ‖ semente)` com `A(i) = HMAC(segredo, A(i-1))`.
macro_rules! p_hash {
    ($mac:ty, $segredo:expr, $semente:expr, $saida:expr) => {{
        let mut a: Vec<u8> = $semente.to_vec();
        let mut feito = 0;
        while feito < $saida.len() {
            let mut mac =
                <$mac as KeyInit>::new_from_slice($segredo).expect("HMAC aceita qualquer chave");
            mac.update(&a);
            a = mac.finalize().into_bytes().to_vec();
            let mut mac =
                <$mac as KeyInit>::new_from_slice($segredo).expect("HMAC aceita qualquer chave");
            mac.update(&a);
            mac.update($semente);
            let bloco = mac.finalize().into_bytes();
            let n = bloco.len().min($saida.len() - feito);
            for (dest, b) in $saida[feito..feito + n].iter_mut().zip(bloco.iter()) {
                *dest ^= b;
            }
            feito += n;
        }
    }};
}

/// A PRF do TLS 1.0 e 1.1 (RFC 2246, seção 5): `P_MD5(S1, rótulo ‖ semente) XOR P_SHA1(S2, rótulo ‖ semente)`.
///
/// O segredo é dividido ao meio; se o tamanho for ímpar, o byte do meio vai nas duas metades.
pub fn tls1_prf(secret: &[u8], label: &[u8], seed: &[u8], out_len: usize) -> Zeroizing<Vec<u8>> {
    let metade = secret.len().div_ceil(2);
    let s1 = &secret[..metade];
    let s2 = &secret[secret.len() - metade..];
    let rotulo_e_semente: Vec<u8> = [label, seed].concat();
    let mut saida = Zeroizing::new(vec![0u8; out_len]);
    p_hash!(Hmac<Md5>, s1, &rotulo_e_semente, saida);
    p_hash!(Hmac<Sha1>, s2, &rotulo_e_semente, saida);
    saida
}

/// Tudo o que a PRF do OpenVPN usa. Os campos "client" e "server" são de cada lado, como
/// aparecem na troca de chaves.
pub struct PrfInputs<'a> {
    /// O segredo de 48 bytes que o cliente enviou.
    pub pre_master: &'a [u8; PRE_MASTER_LEN],
    /// O primeiro aleatório do cliente.
    pub client_random1: &'a [u8; RANDOM_LEN],
    /// O primeiro aleatório do servidor.
    pub server_random1: &'a [u8; RANDOM_LEN],
    /// O segundo aleatório do cliente.
    pub client_random2: &'a [u8; RANDOM_LEN],
    /// O segundo aleatório do servidor.
    pub server_random2: &'a [u8; RANDOM_LEN],
    /// A sessão do cliente.
    pub client_session: SessionId,
    /// A sessão do servidor.
    pub server_session: SessionId,
}

/// Calcula o bloco de 256 bytes pela PRF do OpenVPN.
pub fn derive_key_block(input: &PrfInputs<'_>) -> Zeroizing<[u8; KEY_BLOCK_LEN]> {
    let master = tls1_prf(
        input.pre_master,
        MASTER_LABEL,
        &[&input.client_random1[..], &input.server_random1[..]].concat(),
        PRE_MASTER_LEN,
    );
    let seed = [
        &input.client_random2[..],
        &input.server_random2[..],
        &input.client_session.0[..],
        &input.server_session.0[..],
    ]
    .concat();
    let bloco = tls1_prf(&master, EXPANSION_LABEL, &seed, KEY_BLOCK_LEN);
    let mut out = Zeroizing::new([0u8; KEY_BLOCK_LEN]);
    out.copy_from_slice(&bloco);
    out
}

/// As duas chaves de um sentido do canal de dados.
#[derive(Clone, PartialEq, Eq)]
pub struct DirectionKeys {
    /// 64 bytes de chave de cifra.
    pub cipher: Zeroizing<[u8; KEY_PART_LEN]>,
    /// 64 bytes de chave de HMAC (no AEAD, só os 8 primeiros bytes servem, para o nonce).
    pub hmac: Zeroizing<[u8; KEY_PART_LEN]>,
}

impl std::fmt::Debug for DirectionKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DirectionKeys(..)")
    }
}

/// As chaves do canal de dados, do ponto de vista de quem as usa.
#[derive(Clone, PartialEq, Eq)]
pub struct DataKeys {
    /// As chaves de quem envia.
    pub send: DirectionKeys,
    /// As chaves de quem recebe.
    pub recv: DirectionKeys,
}

impl std::fmt::Debug for DataKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DataKeys(..)")
    }
}

impl DataKeys {
    /// Corta o bloco de 256 bytes. O cliente envia com a primeira metade e recebe com a segunda;
    /// o servidor, o contrário.
    pub fn from_block(block: &[u8; KEY_BLOCK_LEN], role: Role) -> DataKeys {
        let parte = |i: usize| {
            let mut p = Zeroizing::new([0u8; KEY_PART_LEN]);
            p.copy_from_slice(&block[i * KEY_PART_LEN..(i + 1) * KEY_PART_LEN]);
            p
        };
        let c2s = DirectionKeys {
            cipher: parte(0),
            hmac: parte(1),
        };
        let s2c = DirectionKeys {
            cipher: parte(2),
            hmac: parte(3),
        };
        match role {
            Role::Client => DataKeys {
                send: c2s,
                recv: s2c,
            },
            Role::Server => DataKeys {
                send: s2c,
                recv: c2s,
            },
        }
    }
}

#[cfg(test)]
mod testes {
    use super::*;
    use crate::pki::hex;

    #[test]
    fn vetor_conhecido_da_prf_do_tls_1_0() {
        // O vetor de teste público da PRF do TLS 1.0 (secret 48 × 0xab, seed 64 × 0xcd).
        let saida = tls1_prf(&[0xab; 48], b"PRF Testvector", &[0xcd; 64], 104);
        let h = hex(&saida);
        assert!(h.starts_with("d3d4d1e349b5d515"), "{h}");
        assert!(h.ends_with("c655a8c9a013"), "{h}");
    }

    #[test]
    fn saida_vazia_e_prefixo_estavel() {
        assert!(tls1_prf(b"abc", b"x", b"y", 0).is_empty());
        let curta = tls1_prf(b"abc", b"x", b"y", 5);
        let longa = tls1_prf(b"abc", b"x", b"y", 100);
        assert_eq!(&longa[..5], &curta[..]);
    }

    #[test]
    fn segredo_de_tamanho_impar_repete_o_byte_do_meio() {
        // Com 3 bytes, S1 = [0,1] e S2 = [1,2]: o byte do meio entra nas duas.
        let a = tls1_prf(&[0, 1, 2], b"r", b"s", 40);
        let b = tls1_prf(&[0, 1, 2], b"r", b"s", 40);
        assert_eq!(a, b);
        let diferente = tls1_prf(&[0, 9, 2], b"r", b"s", 40);
        assert_ne!(a, diferente);
    }

    fn entradas() -> ([u8; 48], [[u8; 32]; 4]) {
        let mut pre = [0u8; 48];
        for (i, b) in pre.iter_mut().enumerate() {
            *b = i as u8;
        }
        let mut r = [[0u8; 32]; 4];
        for (n, bloco) in r.iter_mut().enumerate() {
            for (i, b) in bloco.iter_mut().enumerate() {
                *b = (n * 40 + i) as u8;
            }
        }
        (pre, r)
    }

    /// A derivação em duas etapas, conferida com `openssl kdf -kdfopt digest:MD5-SHA1 TLS1-PRF`
    /// (o mesmo algoritmo, calculado por outra implementação).
    #[test]
    fn as_duas_etapas_conferem_com_o_openssl() {
        let (pre, r) = entradas();
        let master = tls1_prf(&pre, MASTER_LABEL, &[r[0], r[1]].concat(), 48);
        assert_eq!(
            hex(&master),
            "8d5fba5da5181297b7ac25dd57bf483fba8ecbaf3b3af15bd590ecd348764d183d6d8bf9b839c42f2efef82c39723120"
        );
        let bloco = derive_key_block(&PrfInputs {
            pre_master: &pre,
            client_random1: &r[0],
            server_random1: &r[1],
            client_random2: &r[2],
            server_random2: &r[3],
            client_session: SessionId([1; 8]),
            server_session: SessionId([2; 8]),
        });
        let esperado = "d0b70c3d382ae92de4f81b8f62db9938e7ca637b2c126e51d344c29182faee97121bfd86c5c3fe6f4fe154cb9ee6df851ae597af0406388382b7618ad3fc50f881d52e182700d54fa915fda8243c750c69e7b6a39a3229b09c944139ab79ae8e2d0a205b3c4c64bf81bc316238da93f9ff41e5d17bf8ccca32f147ab5af9f2ffc995e780bf6be1212220e9b8d6b124de5ea348829734a7dd4c2cc84f7cee5d1fbfcefa1f3f0a37649dbbe8c885f4bfc4467cc3c7b2f5d99539eb72aaf4204d2650000475bdb3c64fed18405ab999b6663037c17501d02c783b29b709b013f51ca64f95345dddf692516eba004880e3553fcd35bcb54e1b0cafbad1de2f62bed0";
        assert_eq!(hex(&bloco[..]), esperado);
    }

    #[test]
    fn o_bloco_depende_de_cada_entrada() {
        let (pre, r) = entradas();
        let base = |pre: &[u8; 48], r: &[[u8; 32]; 4], cs: u8, ss: u8| {
            derive_key_block(&PrfInputs {
                pre_master: pre,
                client_random1: &r[0],
                server_random1: &r[1],
                client_random2: &r[2],
                server_random2: &r[3],
                client_session: SessionId([cs; 8]),
                server_session: SessionId([ss; 8]),
            })
        };
        let referencia = base(&pre, &r, 1, 2);
        assert_eq!(referencia.len(), 256);
        assert_eq!(*base(&pre, &r, 1, 2), *referencia, "determinística");
        assert_ne!(*base(&pre, &r, 9, 2), *referencia);
        assert_ne!(*base(&pre, &r, 1, 9), *referencia);
        // As sessões só entram na segunda etapa e os aleatórios 1 só na primeira: todos mudam o bloco.
        for i in 0..4 {
            let mut outro = r;
            outro[i][0] ^= 1;
            assert_ne!(*base(&pre, &outro, 1, 2), *referencia, "aleatório {i}");
        }
        let mut outro_pre = pre;
        outro_pre[47] ^= 1;
        assert_ne!(*base(&outro_pre, &r, 1, 2), *referencia);
    }

    #[test]
    fn as_duas_pontas_cortam_o_bloco_em_sentidos_opostos() {
        let mut bloco = [0u8; 256];
        for (i, b) in bloco.iter_mut().enumerate() {
            *b = i as u8;
        }
        let c = DataKeys::from_block(&bloco, Role::Client);
        let s = DataKeys::from_block(&bloco, Role::Server);
        assert_eq!(c.send, s.recv);
        assert_eq!(c.recv, s.send);
        assert_eq!(c.send.cipher[0], 0);
        assert_eq!(c.send.hmac[0], 64);
        assert_eq!(c.recv.cipher[0], 128);
        assert_eq!(c.recv.hmac[0], 192);
    }

    #[test]
    fn o_debug_nao_mostra_chave() {
        let k = DataKeys::from_block(&[7; 256], Role::Client);
        assert_eq!(format!("{k:?}"), "DataKeys(..)");
    }
}
