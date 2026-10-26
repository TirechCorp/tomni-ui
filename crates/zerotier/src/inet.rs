//! O endereço IP e porta na forma que vai nos pacotes e no mundo (`InetAddress::serialize`,
//! `InetAddress.hpp`): um byte de tipo (4 ou 6), o endereço em ordem de rede e a porta em
//! 16 bits big-endian. O tipo 0 é "sem endereço".

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use crate::{Error, Result};

/// Tipo "sem endereço".
const TIPO_NENHUM: u8 = 0;
/// Tipo dos endereços Ethernet, aceitos e ignorados para compatibilidade futura (7 bytes).
const TIPO_ETHERNET: u8 = 1;
/// Tipo dos endereços Bluetooth, aceitos e ignorados (7 bytes).
const TIPO_BLUETOOTH: u8 = 2;
/// Tipo dos "outros" endereços: um tamanho de 16 bits e o conteúdo, aceitos e ignorados.
const TIPO_OUTRO: u8 = 3;
/// Tipo IPv4.
const TIPO_V4: u8 = 4;
/// Tipo IPv6.
const TIPO_V6: u8 = 6;

/// Escreve `endereco` no fim de `saida`; `None` escreve o tipo "sem endereço".
pub fn write(saida: &mut Vec<u8>, endereco: Option<SocketAddr>) {
    match endereco {
        None => saida.push(TIPO_NENHUM),
        Some(SocketAddr::V4(v4)) => {
            saida.push(TIPO_V4);
            saida.extend_from_slice(&v4.ip().octets());
            saida.extend_from_slice(&v4.port().to_be_bytes());
        }
        Some(SocketAddr::V6(v6)) => {
            saida.push(TIPO_V6);
            saida.extend_from_slice(&v6.ip().octets());
            saida.extend_from_slice(&v6.port().to_be_bytes());
        }
    }
}

/// Lê um endereço de `bytes`. Devolve o endereço (`None` para os tipos sem IP, que o protocolo
/// manda ignorar) e quantos bytes consumiu.
pub fn read(bytes: &[u8]) -> Result<(Option<SocketAddr>, usize)> {
    let curto = || Error::Packet("endereço curto demais".into());
    let tipo = *bytes.first().ok_or_else(curto)?;
    match tipo {
        TIPO_NENHUM => Ok((None, 1)),
        TIPO_ETHERNET | TIPO_BLUETOOTH => {
            if bytes.len() < 7 {
                return Err(curto());
            }
            Ok((None, 7))
        }
        TIPO_OUTRO => {
            let tamanho = bytes.get(1..3).ok_or_else(curto)?;
            let consumido = usize::from(u16::from_be_bytes([tamanho[0], tamanho[1]])) + 3;
            if bytes.len() < consumido {
                return Err(curto());
            }
            Ok((None, consumido))
        }
        TIPO_V4 => {
            let corpo = bytes.get(1..7).ok_or_else(curto)?;
            let ip = Ipv4Addr::new(corpo[0], corpo[1], corpo[2], corpo[3]);
            let porta = u16::from_be_bytes([corpo[4], corpo[5]]);
            Ok((Some(SocketAddr::new(IpAddr::V4(ip), porta)), 7))
        }
        TIPO_V6 => {
            let corpo = bytes.get(1..19).ok_or_else(curto)?;
            let ip = Ipv6Addr::from(<[u8; 16]>::try_from(&corpo[..16]).expect("16 bytes"));
            let porta = u16::from_be_bytes([corpo[16], corpo[17]]);
            Ok((Some(SocketAddr::new(IpAddr::V6(ip), porta)), 19))
        }
        outro => Err(Error::Packet(format!(
            "tipo de endereço {outro} desconhecido"
        ))),
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn v4_e_v6_voltam_iguais() {
        for texto in ["104.194.8.134:9993", "[2607:f8b0::1]:443"] {
            let endereco: SocketAddr = texto.parse().unwrap();
            let mut bytes = Vec::new();
            write(&mut bytes, Some(endereco));
            let (lido, consumido) = read(&bytes).unwrap();
            assert_eq!(lido, Some(endereco));
            assert_eq!(consumido, bytes.len());
        }
    }

    #[test]
    fn os_tipos_sem_ip_sao_pulados_pelo_tamanho_certo() {
        assert_eq!(read(&[0, 9]).unwrap(), (None, 1));
        assert_eq!(read(&[1, 0, 0, 0, 0, 0, 0, 9]).unwrap(), (None, 7));
        assert_eq!(read(&[3, 0, 2, 0xaa, 0xbb, 9]).unwrap(), (None, 5));
    }

    #[test]
    fn curto_e_tipo_desconhecido_sao_erros() {
        assert!(read(&[]).is_err());
        assert!(read(&[4, 1, 2, 3]).is_err());
        assert!(read(&[6, 1, 2, 3]).is_err());
        assert!(read(&[3, 0, 9, 1]).is_err());
        assert!(read(&[9, 0, 0]).is_err());
    }
}
