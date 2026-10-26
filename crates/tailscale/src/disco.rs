//! O disco: as mensagens que duas máquinas trocam, pelo DERP ou direto pelo UDP, para descobrir
//! um caminho direto entre elas (`Ping`, `Pong` e `CallMeMaybe`).
//!
//! Formato no fio: a marca `TS💬` (6 bytes), a chave de disco pública de quem manda (32), e uma
//! caixa NaCl (24 de nonce + cifra) da chave de disco de quem manda para a de quem recebe. Dentro
//! da caixa vão o tipo, a versão e a carga. Portado do Tailscale
//! (github.com/tailscale/tailscale, `disco/disco.go`, licença BSD-3-Clause).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use crate::key::{DiscoPublic, KEY_LEN, NodePublic};
use crate::nacl::{NONCE_LEN, NaclBox, OVERHEAD};

/// A marca que abre toda mensagem do disco: `TS` e o emoji de balão de fala.
pub const MAGIC: [u8; 6] = *b"TS\xf0\x9f\x92\xac";
/// O menor datagrama que pode ser uma mensagem do disco.
pub const MIN_LEN: usize = MAGIC.len() + KEY_LEN + NONCE_LEN + OVERHEAD;

const TIPO_PING: u8 = 1;
const TIPO_PONG: u8 = 2;
const TIPO_CALL_ME_MAYBE: u8 = 3;
/// A versão da carga (a única que existe).
const VERSAO: u8 = 0;
/// O tamanho de um endereço na lista: IPv6 (16) e a porta (2).
const TAMANHO_ENDERECO: usize = 18;
/// O mais que uma lista de endpoints leva (o que cabe num pacote com folga).
const MAX_ENDPOINTS: usize = 64;

/// O identificador de uma transação de ping.
pub type TxId = [u8; 12];

/// Uma mensagem do disco.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// "Você me escuta neste caminho?"
    Ping {
        /// Para casar com o `Pong`.
        tx_id: TxId,
        /// A chave de nó de quem pergunta (versões novas mandam).
        node_key: Option<NodePublic>,
    },
    /// A resposta: de que endereço o `Ping` pareceu vir (o que o NAT mostrou).
    Pong {
        /// O mesmo do `Ping`.
        tx_id: TxId,
        /// O endereço visto por quem respondeu.
        src: SocketAddr,
    },
    /// "Me ligue, talvez": os endpoints de quem manda, para o outro lado mandar pings e abrir o
    /// NAT dos dois lados.
    CallMeMaybe {
        /// Os endereços onde quem manda pode ser achado.
        endpoints: Vec<SocketAddr>,
    },
}

/// Se o datagrama tem cara de mensagem do disco.
pub fn looks_like_disco(pacote: &[u8]) -> bool {
    pacote.len() >= MIN_LEN && pacote.starts_with(&MAGIC)
}

/// A chave de disco de quem mandou (sem abrir a caixa).
pub fn sender(pacote: &[u8]) -> Option<DiscoPublic> {
    if !looks_like_disco(pacote) {
        return None;
    }
    let bytes: [u8; KEY_LEN] = pacote[MAGIC.len()..MAGIC.len() + KEY_LEN].try_into().ok()?;
    Some(DiscoPublic::from_bytes(bytes))
}

/// Monta o datagrama: a marca, a nossa chave de disco pública e a caixa.
pub fn seal(caixa: &NaclBox, nossa: &DiscoPublic, msg: &Message) -> Vec<u8> {
    seal_inner(caixa, nossa, msg, None)
}

/// Como [`seal`], com o nonce dado (para os vetores fixos).
pub fn seal_with(
    caixa: &NaclBox,
    nossa: &DiscoPublic,
    msg: &Message,
    nonce: &[u8; NONCE_LEN],
) -> Vec<u8> {
    seal_inner(caixa, nossa, msg, Some(nonce))
}

fn seal_inner(
    caixa: &NaclBox,
    nossa: &DiscoPublic,
    msg: &Message,
    nonce: Option<&[u8; NONCE_LEN]>,
) -> Vec<u8> {
    let claro = encode(msg);
    let cifrado = match nonce {
        Some(n) => caixa.seal_with(n, &claro),
        None => caixa.seal(&claro),
    };
    let mut saida = Vec::with_capacity(MAGIC.len() + KEY_LEN + cifrado.len());
    saida.extend_from_slice(&MAGIC);
    saida.extend_from_slice(nossa.as_bytes());
    saida.extend_from_slice(&cifrado);
    saida
}

/// Abre o datagrama com a caixa de quem o mandou. `None` se não é do disco, a caixa não abre
/// ou a mensagem é de um tipo que não conhecemos.
pub fn open(caixa: &NaclBox, pacote: &[u8]) -> Option<Message> {
    if !looks_like_disco(pacote) {
        return None;
    }
    let claro = caixa.open(&pacote[MAGIC.len() + KEY_LEN..])?;
    decode(&claro)
}

fn encode(msg: &Message) -> Vec<u8> {
    let mut v = Vec::new();
    match msg {
        Message::Ping { tx_id, node_key } => {
            v.extend_from_slice(&[TIPO_PING, VERSAO]);
            v.extend_from_slice(tx_id);
            if let Some(k) = node_key {
                v.extend_from_slice(k.as_bytes());
            }
        }
        Message::Pong { tx_id, src } => {
            v.extend_from_slice(&[TIPO_PONG, VERSAO]);
            v.extend_from_slice(tx_id);
            escrever_endereco(&mut v, *src);
        }
        Message::CallMeMaybe { endpoints } => {
            v.extend_from_slice(&[TIPO_CALL_ME_MAYBE, VERSAO]);
            for e in endpoints.iter().take(MAX_ENDPOINTS) {
                escrever_endereco(&mut v, *e);
            }
        }
    }
    v
}

fn decode(claro: &[u8]) -> Option<Message> {
    let (&tipo, resto) = claro.split_first()?;
    // A versão vem logo depois; versões novas só acrescentam campos no fim.
    let (_versao, corpo) = resto.split_first()?;
    match tipo {
        TIPO_PING => {
            let tx_id: TxId = corpo.get(..12)?.try_into().ok()?;
            let node_key = corpo
                .get(12..12 + KEY_LEN)
                .and_then(|b| <[u8; KEY_LEN]>::try_from(b).ok())
                .map(NodePublic::from_bytes);
            Some(Message::Ping { tx_id, node_key })
        }
        TIPO_PONG => {
            let tx_id: TxId = corpo.get(..12)?.try_into().ok()?;
            let src = ler_endereco(corpo.get(12..12 + TAMANHO_ENDERECO)?)?;
            Some(Message::Pong { tx_id, src })
        }
        TIPO_CALL_ME_MAYBE => {
            if corpo.len() % TAMANHO_ENDERECO != 0 {
                return None;
            }
            let endpoints = corpo
                .chunks_exact(TAMANHO_ENDERECO)
                .take(MAX_ENDPOINTS)
                .filter_map(ler_endereco)
                .collect();
            Some(Message::CallMeMaybe { endpoints })
        }
        _ => None,
    }
}

/// IPv6 de 16 bytes (o IPv4 vai "mapeado": `::ffff:a.b.c.d`) e a porta em big-endian.
fn escrever_endereco(v: &mut Vec<u8>, a: SocketAddr) {
    let ip6 = match a.ip() {
        IpAddr::V4(ip) => ip.to_ipv6_mapped(),
        IpAddr::V6(ip) => ip,
    };
    v.extend_from_slice(&ip6.octets());
    v.extend_from_slice(&a.port().to_be_bytes());
}

fn ler_endereco(b: &[u8]) -> Option<SocketAddr> {
    let octetos: [u8; 16] = b.get(..16)?.try_into().ok()?;
    let porta = u16::from_be_bytes([*b.get(16)?, *b.get(17)?]);
    let ip6 = Ipv6Addr::from(octetos);
    let ip = match ip6.to_ipv4_mapped() {
        Some(v4) => IpAddr::V4(v4),
        None => IpAddr::V6(ip6),
    };
    Some(SocketAddr::new(ip, porta))
}

/// Só endereços IPv4 servem a este crate (a pilha não fala IPv6): tira os outros e os
/// inúteis (porta 0, `0.0.0.0`, multicast).
pub fn usable_v4(a: &SocketAddr) -> Option<SocketAddr> {
    match a.ip() {
        IpAddr::V4(ip)
            if a.port() != 0
                && ip != Ipv4Addr::UNSPECIFIED
                && !ip.is_multicast()
                && !ip.is_broadcast() =>
        {
            Some(*a)
        }
        _ => None,
    }
}

#[cfg(test)]
mod testes {
    use super::*;
    use crate::key::DiscoKey;

    fn par() -> (DiscoKey, DiscoKey, NaclBox, NaclBox) {
        let a = DiscoKey::generate();
        let b = DiscoKey::generate();
        let ab = NaclBox::new(&a.to_bytes(), b.public().as_bytes());
        let ba = NaclBox::new(&b.to_bytes(), a.public().as_bytes());
        (a, b, ab, ba)
    }

    #[test]
    fn ping_pong_e_call_me_maybe_vao_e_voltam() {
        let (a, _, ab, ba) = par();
        let tx_id = [9u8; 12];
        let nk = NodePublic::from_bytes([3; 32]);
        let msgs = [
            Message::Ping {
                tx_id,
                node_key: Some(nk),
            },
            Message::Ping {
                tx_id,
                node_key: None,
            },
            Message::Pong {
                tx_id,
                src: "203.0.113.7:41641".parse().unwrap(),
            },
            Message::CallMeMaybe {
                endpoints: vec![
                    "192.0.2.1:1".parse().unwrap(),
                    "198.51.100.9:65535".parse().unwrap(),
                ],
            },
            Message::CallMeMaybe { endpoints: vec![] },
        ];
        for m in msgs {
            let pacote = seal(&ab, &a.public(), &m);
            assert!(looks_like_disco(&pacote));
            assert_eq!(sender(&pacote), Some(a.public()));
            assert_eq!(open(&ba, &pacote), Some(m));
        }
    }

    #[test]
    fn so_o_destinatario_abre() {
        let (a, _, ab, _) = par();
        let (_, _, _, outra) = par();
        let pacote = seal(
            &ab,
            &a.public(),
            &Message::Ping {
                tx_id: [0; 12],
                node_key: None,
            },
        );
        assert_eq!(open(&outra, &pacote), None);
    }

    #[test]
    fn lixo_nao_e_disco() {
        let (_, _, _, ba) = par();
        assert!(!looks_like_disco(&[0u8; 200]));
        assert_eq!(sender(b"TS"), None);
        assert_eq!(open(&ba, &[0u8; 200]), None);
        // Com a marca certa, mas cortado.
        let mut curto = MAGIC.to_vec();
        curto.extend_from_slice(&[1; 40]);
        assert_eq!(open(&ba, &curto), None);
    }

    /// Vetores gerados pelo PyNaCl: disco de quem manda `1..=32`, de quem recebe `101..=132`,
    /// nonce `200..=223`, `TxID` `1..=12`.
    #[test]
    fn vetores_fixos_do_ping_e_do_pong() {
        let a: [u8; 32] = std::array::from_fn(|i| i as u8 + 1);
        let b: [u8; 32] = std::array::from_fn(|i| i as u8 + 101);
        let nonce: [u8; 24] = std::array::from_fn(|i| i as u8 + 200);
        let tx_id: TxId = std::array::from_fn(|i| i as u8 + 1);
        let a_pub = DiscoKey::from_bytes(a).public();
        let b_pub = DiscoKey::from_bytes(b).public();
        let ab = NaclBox::new(&a, b_pub.as_bytes());
        let ba = NaclBox::new(&b, a_pub.as_bytes());
        let hex = |v: &[u8]| v.iter().map(|x| format!("{x:02x}")).collect::<String>();

        let ping = Message::Ping {
            tx_id,
            node_key: Some(NodePublic::from_bytes([7; 32])),
        };
        let pacote = seal_with(&ab, &a_pub, &ping, &nonce);
        assert_eq!(
            hex(&pacote),
            "5453f09f92ac07a37cbc142093c8b755dc1b10e86cb426374ad16aa853ed0bdfc0b2b86d1c7cc8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedf65ce3212d58a86b68d38884173889465c7ba3357edbd1ed434b12b29e8027a1475fba0ada035d1fa69a2bccaca66b4372fc5c8fbe330f3d699b797349e7e"
        );
        assert_eq!(open(&ba, &pacote), Some(ping));

        let pong = Message::Pong {
            tx_id,
            src: "192.0.2.1:32853".parse().unwrap(),
        };
        let pacote = seal_with(&ab, &a_pub, &pong, &nonce);
        assert_eq!(
            hex(&pacote),
            "5453f09f92ac07a37cbc142093c8b755dc1b10e86cb426374ad16aa853ed0bdfc0b2b86d1c7cc8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedf46158aa5a47982cca34120992338ad3ec4ba3357edbd1ed434b12b29e8027d1372fca7aaa732d6fd915a7bcdcf603365"
        );
        assert_eq!(open(&ba, &pacote), Some(pong));
    }

    #[test]
    fn so_ipv4_util_serve() {
        assert!(usable_v4(&"192.0.2.1:80".parse().unwrap()).is_some());
        assert!(usable_v4(&"192.0.2.1:0".parse().unwrap()).is_none());
        assert!(usable_v4(&"0.0.0.0:80".parse().unwrap()).is_none());
        assert!(usable_v4(&"[2001:db8::1]:80".parse().unwrap()).is_none());
        assert!(usable_v4(&"224.0.0.1:80".parse().unwrap()).is_none());
    }
}
