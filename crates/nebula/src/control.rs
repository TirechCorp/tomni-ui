//! As mensagens de controle (`NebulaControl`, tipo 6 do cabeçalho): pedir e confirmar um relay.
//!
//! Portado do Nebula (github.com/slackhq/nebula, licença MIT, `nebula.proto` e
//! `relay_manager.go`). Elas vão cifradas no túnel com o relay. O formato v1 leva os endereços
//! como `uint32` (só IPv4); o v2, como `Addr` (16 bytes, o IPv4 mapeado em IPv6). A versão de
//! quem recebe é a do formato que chegou.

use std::net::{IpAddr, Ipv4Addr};

use crate::cert::Version;
use crate::meta::{read_addr, write_addr};
use crate::proto::{Reader, Value, Writer};

/// O tipo da mensagem.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ControlType {
    /// Um tipo que não conhecemos (ou o `None` do protobuf).
    Other(u64),
    /// "Abra um relay até `to` para mim."
    CreateRelayRequest,
    /// "O relay está pronto."
    CreateRelayResponse,
}

impl ControlType {
    fn from_number(n: u64) -> ControlType {
        match n {
            1 => ControlType::CreateRelayRequest,
            2 => ControlType::CreateRelayResponse,
            outro => ControlType::Other(outro),
        }
    }

    fn number(self) -> u64 {
        match self {
            ControlType::Other(n) => n,
            ControlType::CreateRelayRequest => 1,
            ControlType::CreateRelayResponse => 2,
        }
    }
}

/// Uma mensagem de controle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Control {
    pub(crate) kind: ControlType,
    /// O índice do relay de quem pediu.
    pub(crate) initiator_relay_index: u32,
    /// O índice do relay de quem responde.
    pub(crate) responder_relay_index: u32,
    old_relay_to: u32,
    old_relay_from: u32,
    relay_to: Option<IpAddr>,
    relay_from: Option<IpAddr>,
}

impl Control {
    /// Um pedido (`CreateRelayRequest`) ou uma resposta, no formato `versao`. `None` se o v1
    /// não comporta um dos endereços (IPv6).
    pub(crate) fn new(
        kind: ControlType,
        versao: Version,
        initiator_relay_index: u32,
        responder_relay_index: u32,
        from: IpAddr,
        to: IpAddr,
    ) -> Option<Control> {
        let mut c = Control {
            kind,
            initiator_relay_index,
            responder_relay_index,
            old_relay_to: 0,
            old_relay_from: 0,
            relay_to: None,
            relay_from: None,
        };
        match versao {
            Version::V1 => {
                let (IpAddr::V4(de), IpAddr::V4(para)) = (from, to) else {
                    return None;
                };
                c.old_relay_from = u32::from(de);
                c.old_relay_to = u32::from(para);
            }
            Version::V2 => {
                c.relay_from = Some(from);
                c.relay_to = Some(to);
            }
        }
        Some(c)
    }

    /// A versão do formato e os dois endereços (de quem, para quem), como o Nebula lê: um
    /// endereço v1 preenchido faz a mensagem v1. `None` se falta um deles.
    pub(crate) fn version_from_to(&self) -> Option<(Version, IpAddr, IpAddr)> {
        if self.old_relay_from > 0 || self.old_relay_to > 0 {
            return Some((
                Version::V1,
                IpAddr::V4(Ipv4Addr::from(self.old_relay_from)),
                IpAddr::V4(Ipv4Addr::from(self.old_relay_to)),
            ));
        }
        Some((Version::V2, self.relay_from?, self.relay_to?))
    }

    /// A codificação do protobuf, com os campos em ordem crescente, como o Go escreve.
    pub(crate) fn marshal(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.uint(1, self.kind.number());
        w.uint(2, u64::from(self.initiator_relay_index));
        w.uint(3, u64::from(self.responder_relay_index));
        w.uint(4, u64::from(self.old_relay_to));
        w.uint(5, u64::from(self.old_relay_from));
        if let Some(para) = self.relay_to {
            write_addr(&mut w, 6, para);
        }
        if let Some(de) = self.relay_from {
            write_addr(&mut w, 7, de);
        }
        w.finish()
    }

    /// Lê uma mensagem. Os campos desconhecidos são ignorados.
    pub(crate) fn unmarshal(bytes: &[u8]) -> Option<Control> {
        let mut c = Control {
            kind: ControlType::Other(0),
            initiator_relay_index: 0,
            responder_relay_index: 0,
            old_relay_to: 0,
            old_relay_from: 0,
            relay_to: None,
            relay_from: None,
        };
        for item in Reader::new(bytes) {
            let (campo, valor) = item.ok()?;
            match (campo, valor) {
                (1, Value::Varint(n)) => c.kind = ControlType::from_number(n),
                (2, Value::Varint(n)) => c.initiator_relay_index = u32::try_from(n).ok()?,
                (3, Value::Varint(n)) => c.responder_relay_index = u32::try_from(n).ok()?,
                (4, Value::Varint(n)) => c.old_relay_to = u32::try_from(n).ok()?,
                (5, Value::Varint(n)) => c.old_relay_from = u32::try_from(n).ok()?,
                (6, Value::Bytes(interno)) => c.relay_to = Some(read_addr(interno)?),
                (7, Value::Bytes(interno)) => c.relay_from = Some(read_addr(interno)?),
                _ => {}
            }
        }
        Some(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(texto: &str) -> IpAddr {
        texto.parse().unwrap()
    }

    #[test]
    fn o_pedido_v1_tem_o_formato_do_go() {
        let c = Control::new(
            ControlType::CreateRelayRequest,
            Version::V1,
            0x0102_0304,
            0,
            ip("10.42.0.20"),
            ip("10.42.0.42"),
        )
        .unwrap();
        // Type=1, InitiatorRelayIndex, OldRelayToAddr (4) e OldRelayFromAddr (5) como varints.
        let mut esperado = vec![0x08, 0x01, 0x10];
        esperado.extend_from_slice(&[0x84, 0x86, 0x88, 0x08]);
        esperado.extend_from_slice(&[0x20, 0xaa, 0x80, 0xa8, 0x51]);
        esperado.extend_from_slice(&[0x28, 0x94, 0x80, 0xa8, 0x51]);
        assert_eq!(c.marshal(), esperado);
        let lido = Control::unmarshal(&esperado).unwrap();
        assert_eq!(lido, c);
        assert_eq!(
            lido.version_from_to(),
            Some((Version::V1, ip("10.42.0.20"), ip("10.42.0.42")))
        );
    }

    #[test]
    fn a_resposta_v2_leva_os_enderecos_mapeados_e_volta_inteira() {
        let c = Control::new(
            ControlType::CreateRelayResponse,
            Version::V2,
            7,
            9,
            ip("10.42.0.20"),
            ip("fd00::42"),
        )
        .unwrap();
        let bytes = c.marshal();
        // O RelayToAddr (6) vem antes do RelayFromAddr (7).
        assert_eq!(&bytes[..6], &[0x08, 0x02, 0x10, 0x07, 0x18, 0x09]);
        assert_eq!(bytes[6], 0x32);
        let lido = Control::unmarshal(&bytes).unwrap();
        assert_eq!(lido, c);
        assert_eq!(
            lido.version_from_to(),
            Some((Version::V2, ip("10.42.0.20"), ip("fd00::42")))
        );
    }

    #[test]
    fn v1_nao_leva_ipv6_e_falta_de_endereco_e_descartada() {
        assert!(
            Control::new(
                ControlType::CreateRelayRequest,
                Version::V1,
                1,
                0,
                ip("10.42.0.20"),
                ip("fd00::1"),
            )
            .is_none()
        );
        let so_tipo = Control::unmarshal(&[0x08, 0x01]).unwrap();
        assert_eq!(so_tipo.kind, ControlType::CreateRelayRequest);
        assert_eq!(so_tipo.version_from_to(), None);
        assert!(Control::unmarshal(&[0x08]).is_none());
        assert_eq!(
            Control::unmarshal(&[0x08, 0x05]).unwrap().kind,
            ControlType::Other(5)
        );
    }
}
