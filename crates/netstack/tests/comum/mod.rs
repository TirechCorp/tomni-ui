//! Pacotes IPv4 montados à mão, com as somas de verificação certas, para os testes.
#![allow(dead_code)]

/// A soma de verificação da internet (RFC 1071).
pub fn checksum(dados: &[u8]) -> u16 {
    let mut soma: u32 = 0;
    for par in dados.chunks(2) {
        let palavra = if par.len() == 2 {
            u16::from_be_bytes([par[0], par[1]])
        } else {
            u16::from_be_bytes([par[0], 0])
        };
        soma += u32::from(palavra);
    }
    while soma >> 16 != 0 {
        soma = (soma & 0xFFFF) + (soma >> 16);
    }
    !(soma as u16)
}

pub fn ipv4(proto: u8, origem: [u8; 4], destino: [u8; 4], carga: &[u8]) -> Vec<u8> {
    let total = 20 + carga.len();
    let mut p = vec![0u8; 20];
    p[0] = 0x45;
    p[2..4].copy_from_slice(&(total as u16).to_be_bytes());
    p[4..6].copy_from_slice(&1u16.to_be_bytes());
    p[8] = 64;
    p[9] = proto;
    p[12..16].copy_from_slice(&origem);
    p[16..20].copy_from_slice(&destino);
    let c = checksum(&p);
    p[10..12].copy_from_slice(&c.to_be_bytes());
    p.extend_from_slice(carga);
    p
}

pub fn syn(origem: [u8; 4], destino: [u8; 4], porta: u16) -> Vec<u8> {
    let mut tcp = vec![0u8; 20];
    tcp[0..2].copy_from_slice(&40000u16.to_be_bytes());
    tcp[2..4].copy_from_slice(&porta.to_be_bytes());
    tcp[4..8].copy_from_slice(&1000u32.to_be_bytes());
    tcp[12] = 5 << 4;
    tcp[13] = 0x02; // SYN
    tcp[14..16].copy_from_slice(&8192u16.to_be_bytes());
    let mut pseudo = Vec::new();
    pseudo.extend_from_slice(&origem);
    pseudo.extend_from_slice(&destino);
    pseudo.extend_from_slice(&[0, 6]);
    pseudo.extend_from_slice(&(tcp.len() as u16).to_be_bytes());
    pseudo.extend_from_slice(&tcp);
    let c = checksum(&pseudo);
    tcp[16..18].copy_from_slice(&c.to_be_bytes());
    ipv4(6, origem, destino, &tcp)
}

pub fn echo_icmp(origem: [u8; 4], destino: [u8; 4]) -> Vec<u8> {
    let mut icmp = vec![8, 0, 0, 0, 0, 1, 0, 1, 1, 2, 3, 4];
    let c = checksum(&icmp);
    icmp[2..4].copy_from_slice(&c.to_be_bytes());
    ipv4(1, origem, destino, &icmp)
}

/// O RST+ACK que um servidor sem nada na porta responde a um `syn` (um pacote IPv4 com TCP).
pub fn rst_ack(syn: &[u8]) -> Vec<u8> {
    let origem: [u8; 4] = syn[12..16].try_into().unwrap();
    let destino: [u8; 4] = syn[16..20].try_into().unwrap();
    let seq = u32::from_be_bytes(syn[24..28].try_into().unwrap());
    let mut tcp = vec![0u8; 20];
    tcp[0..2].copy_from_slice(&syn[22..24]);
    tcp[2..4].copy_from_slice(&syn[20..22]);
    tcp[8..12].copy_from_slice(&seq.wrapping_add(1).to_be_bytes());
    tcp[12] = 5 << 4;
    tcp[13] = 0x14; // RST + ACK
    let mut pseudo = Vec::new();
    pseudo.extend_from_slice(&destino);
    pseudo.extend_from_slice(&origem);
    pseudo.extend_from_slice(&[0, 6]);
    pseudo.extend_from_slice(&(tcp.len() as u16).to_be_bytes());
    pseudo.extend_from_slice(&tcp);
    let c = checksum(&pseudo);
    tcp[16..18].copy_from_slice(&c.to_be_bytes());
    ipv4(6, destino, origem, &tcp)
}
