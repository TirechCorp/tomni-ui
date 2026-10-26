//! O STUN (RFC 5389) mínimo: o pedido `Binding` e a leitura do endereço que o servidor viu, para
//! o nó saber qual é o seu endereço público. Só o que o cliente do Tailscale usa: IPv4,
//! `XOR-MAPPED-ADDRESS` (e `MAPPED-ADDRESS` de servidores antigos).

use std::net::{Ipv4Addr, SocketAddr};

/// O número fixo que todo STUN traz nos bytes 4 a 7.
pub const COOKIE: u32 = 0x2112_A442;
/// A porta padrão do STUN.
pub const PORTA_PADRAO: u16 = 3478;
/// O identificador de uma transação.
pub type TxId = [u8; 12];

const CABECALHO: usize = 20;
const TIPO_PEDIDO: u16 = 0x0001;
const TIPO_RESPOSTA: u16 = 0x0101;
const ATRIBUTO_MAPPED: u16 = 0x0001;
const ATRIBUTO_XOR_MAPPED: u16 = 0x0020;
const FAMILIA_IPV4: u8 = 0x01;

/// O pedido `Binding` com o identificador dado.
pub fn request(tx_id: &TxId) -> Vec<u8> {
    let mut v = Vec::with_capacity(CABECALHO);
    v.extend_from_slice(&TIPO_PEDIDO.to_be_bytes());
    v.extend_from_slice(&0u16.to_be_bytes());
    v.extend_from_slice(&COOKIE.to_be_bytes());
    v.extend_from_slice(tx_id);
    v
}

/// Se o datagrama parece STUN: os dois primeiros bits zero e o cookie nos bytes 4 a 7.
pub fn looks_like_stun(pacote: &[u8]) -> bool {
    pacote.len() >= CABECALHO && pacote[0] & 0xC0 == 0 && pacote[4..8] == COOKIE.to_be_bytes()
}

/// Lê uma resposta `Binding` bem-sucedida: o identificador e o endereço público.
pub fn parse_response(pacote: &[u8]) -> Option<(TxId, SocketAddr)> {
    if !looks_like_stun(pacote) {
        return None;
    }
    let tipo = u16::from_be_bytes([pacote[0], pacote[1]]);
    if tipo != TIPO_RESPOSTA {
        return None;
    }
    let tamanho = usize::from(u16::from_be_bytes([pacote[2], pacote[3]]));
    let tx_id: TxId = pacote[8..CABECALHO].try_into().ok()?;
    let mut atributos = pacote.get(CABECALHO..CABECALHO + tamanho)?;
    let mut mapeado = None;
    while atributos.len() >= 4 {
        let t = u16::from_be_bytes([atributos[0], atributos[1]]);
        let n = usize::from(u16::from_be_bytes([atributos[2], atributos[3]]));
        let valor = atributos.get(4..4 + n)?;
        match t {
            ATRIBUTO_XOR_MAPPED => {
                if let Some(a) = ler_endereco(valor, true) {
                    return Some((tx_id, a));
                }
            }
            ATRIBUTO_MAPPED => mapeado = ler_endereco(valor, false).or(mapeado),
            _ => {}
        }
        // Os atributos vão alinhados em 4 bytes.
        let passo = 4 + n.div_ceil(4) * 4;
        atributos = atributos.get(passo..).unwrap_or(&[]);
    }
    mapeado.map(|a| (tx_id, a))
}

fn ler_endereco(valor: &[u8], xor: bool) -> Option<SocketAddr> {
    if valor.len() < 8 || valor[1] != FAMILIA_IPV4 {
        return None;
    }
    let mut porta = u16::from_be_bytes([valor[2], valor[3]]);
    let mut ip = u32::from_be_bytes([valor[4], valor[5], valor[6], valor[7]]);
    if xor {
        porta ^= (COOKIE >> 16) as u16;
        ip ^= COOKIE;
    }
    Some(SocketAddr::new(Ipv4Addr::from(ip).into(), porta))
}

/// Monta uma resposta `Binding` (para o servidor STUN dos testes).
pub fn response(tx_id: &TxId, visto: SocketAddr) -> Vec<u8> {
    let SocketAddr::V4(v4) = visto else {
        return Vec::new();
    };
    let mut atributo = vec![0u8, FAMILIA_IPV4];
    atributo.extend_from_slice(&(v4.port() ^ (COOKIE >> 16) as u16).to_be_bytes());
    atributo.extend_from_slice(&(u32::from(*v4.ip()) ^ COOKIE).to_be_bytes());
    let mut v = Vec::new();
    v.extend_from_slice(&TIPO_RESPOSTA.to_be_bytes());
    v.extend_from_slice(&((4 + atributo.len()) as u16).to_be_bytes());
    v.extend_from_slice(&COOKIE.to_be_bytes());
    v.extend_from_slice(tx_id);
    v.extend_from_slice(&ATRIBUTO_XOR_MAPPED.to_be_bytes());
    v.extend_from_slice(&(atributo.len() as u16).to_be_bytes());
    v.extend_from_slice(&atributo);
    v
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn o_pedido_tem_o_cabecalho_do_rfc() {
        let tx: TxId = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
        let p = request(&tx);
        assert_eq!(p.len(), 20);
        assert_eq!(&p[..8], &[0, 1, 0, 0, 0x21, 0x12, 0xa4, 0x42]);
        assert_eq!(&p[8..], &tx);
        assert!(looks_like_stun(&p));
    }

    /// O endereço da resposta de exemplo do RFC 5769 (seção 2.2): 192.0.2.1, porta 32853, com o
    /// identificador `b7e7a701bc34d686fa87dfae`. Os demais atributos do exemplo (software,
    /// integridade, impressão digital) ficam de fora: o cliente não os confere.
    #[test]
    fn le_o_exemplo_do_rfc_5769() {
        let mut p = vec![0x01, 0x01, 0x00, 0x2c, 0x21, 0x12, 0xa4, 0x42];
        p.extend_from_slice(&[
            0xb7, 0xe7, 0xa7, 0x01, 0xbc, 0x34, 0xd6, 0x86, 0xfa, 0x87, 0xdf, 0xae,
        ]);
        // SOFTWARE "test vector " (11 bytes + 1 de enchimento é "test vector" + espaço).
        p.extend_from_slice(&[0x80, 0x22, 0x00, 0x0b]);
        p.extend_from_slice(b"test vector\x20");
        // XOR-MAPPED-ADDRESS.
        p.extend_from_slice(&[
            0x00, 0x20, 0x00, 0x08, 0x00, 0x01, 0xa1, 0x47, 0xe1, 0x12, 0xa6, 0x43,
        ]);
        // 4 + 12 (software) + 12 (xor-mapped) = 28; o campo diz 0x2c = 44: corrige.
        let corpo = p.len() - 20;
        p[2..4].copy_from_slice(&(corpo as u16).to_be_bytes());
        let (tx, a) = parse_response(&p).unwrap();
        assert_eq!(
            tx,
            [
                0xb7, 0xe7, 0xa7, 0x01, 0xbc, 0x34, 0xd6, 0x86, 0xfa, 0x87, 0xdf, 0xae
            ]
        );
        assert_eq!(a, "192.0.2.1:32853".parse().unwrap());
    }

    #[test]
    fn a_resposta_montada_volta_igual() {
        let tx: TxId = [7; 12];
        let visto: SocketAddr = "203.0.113.50:54321".parse().unwrap();
        assert_eq!(parse_response(&response(&tx, visto)), Some((tx, visto)));
    }

    #[test]
    fn mapped_address_simples_tambem_serve() {
        let tx: TxId = [5; 12];
        let mut p = vec![0x01, 0x01, 0x00, 0x0c, 0x21, 0x12, 0xa4, 0x42];
        p.extend_from_slice(&tx);
        p.extend_from_slice(&[0x00, 0x01, 0x00, 0x08, 0x00, 0x01, 0x1f, 0x90, 10, 0, 0, 9]);
        assert_eq!(
            parse_response(&p),
            Some((tx, "10.0.0.9:8080".parse().unwrap()))
        );
    }

    #[test]
    fn lixo_e_pedido_nao_sao_resposta() {
        assert_eq!(parse_response(&[0u8; 30]), None);
        assert_eq!(parse_response(&request(&[0; 12])), None);
        assert!(!looks_like_stun(&[0x01u8; 10]));
        // O disco e o WireGuard não parecem STUN.
        assert!(!looks_like_stun(&[
            1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
        ]));
        // Cortado no meio de um atributo.
        let mut p = response(&[1; 12], "1.2.3.4:5".parse().unwrap());
        p.truncate(24);
        assert_eq!(parse_response(&p), None);
    }
}
