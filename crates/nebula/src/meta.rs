//! O protocolo do lighthouse: as mensagens `NebulaMeta` (protobuf) que viajam dentro do túnel.
//!
//! Portado do Nebula (github.com/slackhq/nebula, licença MIT, `nebula.proto` e `lighthouse.go`).
//! Os túneis de certificado v1 usam os campos "Old" (endereços IPv4 em 32 bits) e os de v2 usam
//! `VpnAddr` e `RelayVpnAddrs`.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use crate::proto::{Reader, Value, Writer, push_u32s};

/// O tipo da mensagem (o enum `NebulaMeta.MessageType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MetaType {
    /// "Onde está este nó?" (pergunta ao lighthouse).
    HostQuery,
    /// A resposta com os endereços.
    HostQueryReply,
    /// "Estes são os meus endereços" (o nó avisa o lighthouse).
    HostUpdateNotification,
    /// Reservado; o Nebula ignora.
    HostMovedNotification,
    /// "Fure o NAT na direção destes endereços" (o lighthouse avisa o alvo de uma pergunta).
    HostPunchNotification,
    /// O lighthouse confirmou o `HostUpdateNotification`.
    HostUpdateNotificationAck,
    /// Qualquer outro valor (inclusive os que o Nebula 1.11 define e ignora).
    Other(u64),
}

impl MetaType {
    fn from_number(n: u64) -> MetaType {
        match n {
            1 => MetaType::HostQuery,
            2 => MetaType::HostQueryReply,
            3 => MetaType::HostUpdateNotification,
            4 => MetaType::HostMovedNotification,
            5 => MetaType::HostPunchNotification,
            10 => MetaType::HostUpdateNotificationAck,
            other => MetaType::Other(other),
        }
    }

    fn number(self) -> u64 {
        match self {
            MetaType::HostQuery => 1,
            MetaType::HostQueryReply => 2,
            MetaType::HostUpdateNotification => 3,
            MetaType::HostMovedNotification => 4,
            MetaType::HostPunchNotification => 5,
            MetaType::HostUpdateNotificationAck => 10,
            MetaType::Other(n) => n,
        }
    }
}

/// Os detalhes de uma mensagem (`NebulaMetaDetails`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Details {
    /// O endereço da VPN no formato v1 (IPv4 em 32 bits); zero se não há.
    pub(crate) old_vpn_addr: u32,
    /// O endereço da VPN no formato v2.
    pub(crate) vpn_addr: Option<IpAddr>,
    /// Os relays no formato v1.
    pub(crate) old_relays: Vec<u32>,
    /// Os relays no formato v2.
    pub(crate) relays: Vec<IpAddr>,
    /// Os endereços do underlay (IPv4 e IPv6).
    pub(crate) addrs: Vec<SocketAddr>,
}

/// Uma mensagem do lighthouse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Meta {
    pub(crate) kind: MetaType,
    /// `None` é uma mensagem sem detalhes (o Nebula as descarta).
    pub(crate) details: Option<Details>,
}

/// O `Addr{Hi, Lo}` do protobuf: o endereço em 16 bytes (o IPv4 vai mapeado em IPv6).
pub(crate) fn write_addr(escritor: &mut Writer, campo: u32, endereco: IpAddr) {
    let octetos = match endereco {
        IpAddr::V4(v4) => v4.to_ipv6_mapped().octets(),
        IpAddr::V6(v6) => v6.octets(),
    };
    let mut interno = Writer::new();
    interno.uint(
        1,
        u64::from_be_bytes(octetos[..8].try_into().unwrap_or([0; 8])),
    );
    interno.uint(
        2,
        u64::from_be_bytes(octetos[8..].try_into().unwrap_or([0; 8])),
    );
    escritor.bytes_always(campo, &interno.finish());
}

pub(crate) fn read_addr(bytes: &[u8]) -> Option<IpAddr> {
    let (mut hi, mut lo) = (0u64, 0u64);
    for item in Reader::new(bytes) {
        let (campo, valor) = item.ok()?;
        match (campo, valor) {
            (1, Value::Varint(v)) => hi = v,
            (2, Value::Varint(v)) => lo = v,
            _ => {}
        }
    }
    let mut octetos = [0u8; 16];
    octetos[..8].copy_from_slice(&hi.to_be_bytes());
    octetos[8..].copy_from_slice(&lo.to_be_bytes());
    Some(IpAddr::V6(Ipv6Addr::from(octetos)).to_canonical())
}

impl Meta {
    /// A codificação do protobuf, como o Go escreve (campos em ordem crescente).
    pub(crate) fn marshal(&self) -> Vec<u8> {
        let mut fora = Writer::new();
        fora.uint(1, self.kind.number());
        if let Some(detalhes) = &self.details {
            fora.bytes_always(2, &detalhes.marshal());
        }
        fora.finish()
    }

    /// Lê uma mensagem. Os campos desconhecidos são ignorados.
    pub(crate) fn unmarshal(bytes: &[u8]) -> Option<Meta> {
        let mut meta = Meta {
            kind: MetaType::Other(0),
            details: None,
        };
        for item in Reader::new(bytes) {
            let (campo, valor) = item.ok()?;
            match (campo, valor) {
                (1, Value::Varint(n)) => meta.kind = MetaType::from_number(n),
                (2, Value::Bytes(detalhes)) => meta.details = Some(Details::unmarshal(detalhes)?),
                _ => {}
            }
        }
        Some(meta)
    }
}

impl Details {
    fn marshal(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.uint(1, u64::from(self.old_vpn_addr));
        for endereco in &self.addrs {
            if let SocketAddr::V4(v4) = endereco {
                let mut interno = Writer::new();
                interno.uint(1, u64::from(u32::from(*v4.ip())));
                interno.uint(2, u64::from(v4.port()));
                w.bytes_always(2, &interno.finish());
            }
        }
        for endereco in &self.addrs {
            if let SocketAddr::V6(v6) = endereco {
                let octetos = v6.ip().octets();
                let mut interno = Writer::new();
                interno.uint(
                    1,
                    u64::from_be_bytes(octetos[..8].try_into().unwrap_or([0; 8])),
                );
                interno.uint(
                    2,
                    u64::from_be_bytes(octetos[8..].try_into().unwrap_or([0; 8])),
                );
                interno.uint(3, u64::from(v6.port()));
                w.bytes_always(4, &interno.finish());
            }
        }
        w.packed_u32(5, &self.old_relays);
        if let Some(endereco) = self.vpn_addr {
            write_addr(&mut w, 6, endereco);
        }
        for relay in &self.relays {
            write_addr(&mut w, 7, *relay);
        }
        w.finish()
    }

    fn unmarshal(bytes: &[u8]) -> Option<Details> {
        let mut d = Details::default();
        for item in Reader::new(bytes) {
            let (campo, valor) = item.ok()?;
            match (campo, valor) {
                (1, Value::Varint(n)) => d.old_vpn_addr = u32::try_from(n).ok()?,
                (2, Value::Bytes(interno)) => {
                    let (mut ip, mut porta) = (0u64, 0u64);
                    for par in Reader::new(interno) {
                        match par.ok()? {
                            (1, Value::Varint(v)) => ip = v,
                            (2, Value::Varint(v)) => porta = v,
                            _ => {}
                        }
                    }
                    d.addrs.push(SocketAddr::new(
                        IpAddr::V4(Ipv4Addr::from(u32::try_from(ip).ok()?)),
                        u16::try_from(porta).ok()?,
                    ));
                }
                (4, Value::Bytes(interno)) => {
                    let (mut hi, mut lo, mut porta) = (0u64, 0u64, 0u64);
                    for par in Reader::new(interno) {
                        match par.ok()? {
                            (1, Value::Varint(v)) => hi = v,
                            (2, Value::Varint(v)) => lo = v,
                            (3, Value::Varint(v)) => porta = v,
                            _ => {}
                        }
                    }
                    let mut octetos = [0u8; 16];
                    octetos[..8].copy_from_slice(&hi.to_be_bytes());
                    octetos[8..].copy_from_slice(&lo.to_be_bytes());
                    d.addrs.push(SocketAddr::new(
                        IpAddr::V6(Ipv6Addr::from(octetos)).to_canonical(),
                        u16::try_from(porta).ok()?,
                    ));
                }
                (5, valor @ (Value::Bytes(_) | Value::Varint(_))) => {
                    push_u32s(&mut d.old_relays, valor).ok()?;
                }
                (6, Value::Bytes(interno)) => d.vpn_addr = Some(read_addr(interno)?),
                (7, Value::Bytes(interno)) => d.relays.push(read_addr(interno)?),
                _ => {}
            }
        }
        Some(d)
    }

    /// O endereço da VPN de que a mensagem fala e a versão do formato: `OldVpnAddr` é v1,
    /// `VpnAddr` é v2. `None` se não traz nenhum.
    pub(crate) fn vpn_addr_and_version(&self) -> Option<(IpAddr, crate::cert::Version)> {
        if self.old_vpn_addr != 0 {
            return Some((
                IpAddr::V4(Ipv4Addr::from(self.old_vpn_addr)),
                crate::cert::Version::V1,
            ));
        }
        self.vpn_addr.map(|a| (a, crate::cert::Version::V2))
    }

    /// Os relays das duas formas, juntos.
    pub(crate) fn all_relays(&self) -> Vec<IpAddr> {
        self.old_relays
            .iter()
            .map(|r| IpAddr::V4(Ipv4Addr::from(*r)))
            .chain(self.relays.iter().copied())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v4(texto: &str) -> SocketAddr {
        texto.parse().unwrap()
    }

    #[test]
    fn a_consulta_v1_tem_o_formato_do_go() {
        // HostQuery de 10.42.0.10: Type=1, Details{OldVpnAddr=0x0a2a000a}.
        let meta = Meta {
            kind: MetaType::HostQuery,
            details: Some(Details {
                old_vpn_addr: 0x0a2a_000a,
                ..Details::default()
            }),
        };
        let bytes = meta.marshal();
        assert_eq!(
            bytes,
            [0x08, 0x01, 0x12, 0x05, 0x08, 0x8a, 0x80, 0xa8, 0x51]
        );
        assert_eq!(Meta::unmarshal(&bytes).unwrap(), meta);
    }

    #[test]
    fn o_endereco_v2_vai_mapeado_em_ipv6() {
        let meta = Meta {
            kind: MetaType::HostQuery,
            details: Some(Details {
                vpn_addr: Some("10.42.0.10".parse().unwrap()),
                ..Details::default()
            }),
        };
        let bytes = meta.marshal();
        // Details{VpnAddr{Hi=0 (omitido, como no proto3), Lo=0x0000ffff0a2a000a}}.
        assert_eq!(
            bytes,
            [
                0x08, 0x01, 0x12, 0x0a, 0x32, 0x08, 0x10, 0x8a, 0x80, 0xa8, 0xd1, 0xf0, 0xff, 0x3f
            ]
        );
        let volta = Meta::unmarshal(&bytes).unwrap();
        assert_eq!(
            volta.details.unwrap().vpn_addr,
            Some("10.42.0.10".parse().unwrap())
        );
    }

    #[test]
    fn a_resposta_com_enderecos_e_relays_volta_inteira() {
        let meta = Meta {
            kind: MetaType::HostQueryReply,
            details: Some(Details {
                old_vpn_addr: 0x0a2a_0001,
                vpn_addr: None,
                old_relays: vec![0x0a2a_0001, 0x0a2a_0002],
                relays: vec!["10.42.0.5".parse().unwrap(), "fd00::1".parse().unwrap()],
                addrs: vec![
                    v4("203.0.113.7:4242"),
                    v4("[2001:db8::1]:4242"),
                    v4("10.0.0.2:1"),
                ],
            }),
        };
        let volta = Meta::unmarshal(&meta.marshal()).unwrap();
        assert_eq!(volta.kind, MetaType::HostQueryReply);
        let d = volta.details.unwrap();
        // Os IPv4 saem antes dos IPv6 (campos 2 e 4), então a ordem entre as famílias muda.
        assert_eq!(
            d.addrs,
            [
                v4("203.0.113.7:4242"),
                v4("10.0.0.2:1"),
                v4("[2001:db8::1]:4242")
            ]
        );
        assert_eq!(d.old_relays, [0x0a2a_0001, 0x0a2a_0002]);
        assert_eq!(d.all_relays().len(), 4);
        assert_eq!(d.relays[1], "fd00::1".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn lixo_e_tipos_desconhecidos() {
        assert!(Meta::unmarshal(&[0x08]).is_none());
        let meta = Meta::unmarshal(&[0x08, 0x08]).unwrap();
        assert_eq!(meta.kind, MetaType::Other(8));
        assert!(meta.details.is_none());
        // Sem detalhes (o Nebula descarta, e quem trata decide).
        let meta = Meta::unmarshal(&[0x08, 0x01]).unwrap();
        assert!(meta.details.is_none());
    }

    #[test]
    fn a_versao_vem_do_campo_que_esta_preenchido() {
        let v1 = Details {
            old_vpn_addr: 1,
            ..Details::default()
        };
        assert_eq!(
            v1.vpn_addr_and_version().unwrap().1,
            crate::cert::Version::V1
        );
        let v2 = Details {
            vpn_addr: Some("10.0.0.1".parse().unwrap()),
            ..Details::default()
        };
        assert_eq!(
            v2.vpn_addr_and_version().unwrap().1,
            crate::cert::Version::V2
        );
        assert!(Details::default().vpn_addr_and_version().is_none());
    }
}
