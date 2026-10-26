//! Rotas e IPs gerenciados da configuração da rede. Nos dois, o `InetAddress` do fio guarda o
//! tamanho do prefixo onde iria a porta (`ManagedRoute`, `NetworkConfig.cpp`).

use std::net::{IpAddr, SocketAddr};

use tomni_ui_netstack::Cidr;

use crate::inet;

/// O máximo de rotas (`ZT_MAX_NETWORK_ROUTES`).
pub const MAX_ROUTES: usize = 128;
/// O máximo de IPs atribuídos (`ZT_MAX_ZT_ASSIGNED_ADDRESSES`).
pub const MAX_ADDRESSES: usize = 32;

/// Uma rota que o controlador manda.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManagedRoute {
    /// A rede de destino.
    pub target: Cidr,
    /// O gateway. `None`: a rede está no enlace da rede virtual.
    pub via: Option<IpAddr>,
    /// As flags (sem uso hoje).
    pub flags: u16,
    /// A métrica.
    pub metric: u16,
}

fn cidr_de(endereco: SocketAddr) -> Option<Cidr> {
    let prefixo = u8::try_from(endereco.port()).ok()?;
    Cidr::new(endereco.ip(), prefixo)
}

fn escreve_cidr(saida: &mut Vec<u8>, cidr: &Cidr) {
    inet::write(
        saida,
        Some(SocketAddr::new(cidr.addr(), u16::from(cidr.prefix()))),
    );
}

/// Lê a lista de rotas (`RT`). Entradas que não são IP ou com prefixo inválido são puladas.
/// `None` se o fim cai no meio de uma entrada.
pub fn parse_routes(bytes: &[u8]) -> Option<Vec<ManagedRoute>> {
    let mut rotas = Vec::new();
    let mut p = 0;
    while p < bytes.len() && rotas.len() < MAX_ROUTES {
        let (alvo, n) = inet::read(&bytes[p..]).ok()?;
        p += n;
        let (via, n) = inet::read(bytes.get(p..)?).ok()?;
        p += n;
        let resto = bytes.get(p..p + 4)?;
        p += 4;
        let flags = u16::from_be_bytes([resto[0], resto[1]]);
        let metric = u16::from_be_bytes([resto[2], resto[3]]);
        let Some(target) = alvo.and_then(cidr_de) else {
            continue;
        };
        // Um gateway 0.0.0.0 (ou ::) quer dizer "no enlace", como a falta de gateway.
        let via = via.map(|v| v.ip()).filter(|ip| !ip.is_unspecified());
        rotas.push(ManagedRoute {
            target,
            via,
            flags,
            metric,
        });
    }
    Some(rotas)
}

/// Escreve a lista de rotas.
pub fn write_routes(rotas: &[ManagedRoute]) -> Vec<u8> {
    let mut saida = Vec::new();
    for r in rotas {
        escreve_cidr(&mut saida, &r.target);
        inet::write(&mut saida, r.via.map(|ip| SocketAddr::new(ip, 0)));
        saida.extend_from_slice(&r.flags.to_be_bytes());
        saida.extend_from_slice(&r.metric.to_be_bytes());
    }
    saida
}

/// Lê os IPs gerenciados (`I`): endereços em sequência, cada um com o prefixo onde iria a porta.
pub fn parse_addresses(bytes: &[u8]) -> Option<Vec<Cidr>> {
    let mut enderecos = Vec::new();
    let mut p = 0;
    while p < bytes.len() && enderecos.len() < MAX_ADDRESSES {
        let (e, n) = inet::read(&bytes[p..]).ok()?;
        p += n;
        if let Some(c) = e.and_then(cidr_de) {
            enderecos.push(c);
        }
    }
    Some(enderecos)
}

/// Escreve os IPs gerenciados.
pub fn write_addresses(enderecos: &[Cidr]) -> Vec<u8> {
    let mut saida = Vec::new();
    for c in enderecos {
        escreve_cidr(&mut saida, c);
    }
    saida
}

#[cfg(test)]
mod testes {
    use super::*;

    fn cidr(texto: &str, prefixo: u8) -> Cidr {
        Cidr::new(texto.parse().unwrap(), prefixo).unwrap()
    }

    #[test]
    fn rotas_vao_e_voltam() {
        let rotas = vec![
            ManagedRoute {
                target: cidr("10.61.0.0", 24),
                via: None,
                flags: 0,
                metric: 0,
            },
            ManagedRoute {
                target: cidr("172.31.62.0", 24),
                via: Some("10.61.0.12".parse().unwrap()),
                flags: 0,
                metric: 10,
            },
        ];
        assert_eq!(parse_routes(&write_routes(&rotas)).unwrap(), rotas);
    }

    #[test]
    fn gateway_zerado_e_no_enlace() {
        let mut bytes = Vec::new();
        escreve_cidr(&mut bytes, &cidr("10.1.0.0", 16));
        inet::write(&mut bytes, Some("0.0.0.0:0".parse().unwrap()));
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        assert_eq!(parse_routes(&bytes).unwrap()[0].via, None);
    }

    #[test]
    fn ips_gerenciados_vao_e_voltam() {
        let ips = vec![cidr("10.61.0.7", 24), cidr("fd61::7", 64)];
        assert_eq!(parse_addresses(&write_addresses(&ips)).unwrap(), ips);
    }

    #[test]
    fn entrada_cortada_e_recusada() {
        let mut bytes = write_routes(&[ManagedRoute {
            target: cidr("10.1.0.0", 16),
            via: None,
            flags: 0,
            metric: 0,
        }]);
        bytes.pop();
        assert!(parse_routes(&bytes).is_none());
    }
}
