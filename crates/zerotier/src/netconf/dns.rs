//! O DNS da rede (`DNS` na configuração): o domínio, em 128 bytes com zeros no fim, e até
//! quatro servidores (`InetAddress`).

use std::net::{IpAddr, SocketAddr};

use crate::inet;

/// O máximo de servidores (`ZT_MAX_DNS_SERVERS`).
pub const MAX_SERVERS: usize = 4;
const DOMAIN_LEN: usize = 128;

/// O DNS que a rede oferece.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dns {
    /// O domínio de busca da rede (`zt.example`).
    pub domain: String,
    /// Os servidores, em ordem.
    pub servers: Vec<IpAddr>,
}

impl Dns {
    /// Lê do fio. `None` se está truncado; sem domínio ou sem servidor devolve `None` também
    /// (não há o que resolver).
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let dominio = bytes.get(..DOMAIN_LEN)?;
        let fim = dominio.iter().position(|b| *b == 0).unwrap_or(DOMAIN_LEN);
        let domain = String::from_utf8_lossy(&dominio[..fim]).into_owned();
        let mut p = DOMAIN_LEN;
        let mut servers = Vec::new();
        for _ in 0..MAX_SERVERS {
            let Some(resto) = bytes.get(p..) else { break };
            if resto.is_empty() {
                break;
            }
            let (e, n) = inet::read(resto).ok()?;
            p += n;
            if let Some(e) = e {
                servers.push(e.ip());
            }
        }
        if domain.is_empty() || servers.is_empty() {
            return None;
        }
        Some(Self { domain, servers })
    }

    /// Escreve no fio.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut saida = vec![0u8; DOMAIN_LEN];
        let d = self.domain.as_bytes();
        let n = d.len().min(DOMAIN_LEN - 1);
        saida[..n].copy_from_slice(&d[..n]);
        for i in 0..MAX_SERVERS {
            inet::write(
                &mut saida,
                self.servers.get(i).map(|ip| SocketAddr::new(*ip, 53)),
            );
        }
        saida
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn vai_e_volta() {
        let dns = Dns {
            domain: "zt.exemplo".into(),
            servers: vec!["10.61.0.1".parse().unwrap(), "fd61::1".parse().unwrap()],
        };
        assert_eq!(Dns::parse(&dns.to_bytes()).unwrap(), dns);
    }

    #[test]
    fn sem_servidor_ou_sem_dominio_nao_vale() {
        let sem_servidor = Dns {
            domain: "zt.exemplo".into(),
            servers: vec![],
        };
        assert!(Dns::parse(&sem_servidor.to_bytes()).is_none());
        let sem_dominio = Dns {
            domain: String::new(),
            servers: vec!["10.0.0.1".parse().unwrap()],
        };
        assert!(Dns::parse(&sem_dominio.to_bytes()).is_none());
        assert!(Dns::parse(&[0u8; 10]).is_none());
    }
}
