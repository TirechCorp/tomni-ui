//! Um endereço com o tamanho da rede (`10.42.0.7/24`): o que o túnel diz de si e das redes que
//! alcança.

use std::fmt;
use std::net::IpAddr;
use std::str::FromStr;

/// Um endereço IP com o prefixo da rede a que pertence (`10.42.0.7/24`).
///
/// ```
/// use tomni_ui_netstack::Cidr;
///
/// let rede: Cidr = "10.42.0.0/24".parse().unwrap();
/// assert!(rede.contains("10.42.0.9".parse().unwrap()));
/// assert!(!rede.contains("10.43.0.9".parse().unwrap()));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Cidr {
    addr: IpAddr,
    prefix: u8,
}

impl Cidr {
    /// Monta o endereço com o prefixo; `None` se o prefixo passa de 32 (IPv4) ou de 128 (IPv6).
    pub fn new(addr: IpAddr, prefix: u8) -> Option<Cidr> {
        let max = if addr.is_ipv4() { 32 } else { 128 };
        (prefix <= max).then_some(Cidr { addr, prefix })
    }

    /// O endereço, com os bits do host.
    pub fn addr(&self) -> IpAddr {
        self.addr
    }

    /// O tamanho do prefixo, em bits.
    pub fn prefix(&self) -> u8 {
        self.prefix
    }

    /// O endereço da rede (os bits do host zerados).
    pub fn network(&self) -> IpAddr {
        match self.addr {
            IpAddr::V4(addr) => IpAddr::V4((u32::from(addr) & mask32(self.prefix)).into()),
            IpAddr::V6(addr) => IpAddr::V6((u128::from(addr) & mask128(self.prefix)).into()),
        }
    }

    /// `true` se `ip` está dentro da rede. Um IPv4 nunca está numa rede IPv6, e o contrário.
    pub fn contains(&self, ip: IpAddr) -> bool {
        match (self.addr, ip) {
            (IpAddr::V4(rede), IpAddr::V4(ip)) => {
                u32::from(rede) & mask32(self.prefix) == u32::from(ip) & mask32(self.prefix)
            }
            (IpAddr::V6(rede), IpAddr::V6(ip)) => {
                u128::from(rede) & mask128(self.prefix) == u128::from(ip) & mask128(self.prefix)
            }
            _ => false,
        }
    }
}

fn mask32(prefix: u8) -> u32 {
    u32::MAX.checked_shl(32 - u32::from(prefix)).unwrap_or(0)
}

fn mask128(prefix: u8) -> u128 {
    u128::MAX.checked_shl(128 - u32::from(prefix)).unwrap_or(0)
}

impl fmt::Display for Cidr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.addr, self.prefix)
    }
}

impl FromStr for Cidr {
    type Err = String;

    /// Lê `10.42.0.7/24`. Sem a barra, o endereço vale como host (`/32` ou `/128`).
    fn from_str(texto: &str) -> Result<Cidr, String> {
        let (endereco, prefixo) = match texto.split_once('/') {
            Some((endereco, prefixo)) => (endereco, Some(prefixo)),
            None => (texto, None),
        };
        let addr: IpAddr = endereco
            .trim()
            .parse()
            .map_err(|_| format!("endereço inválido: {texto}"))?;
        let prefix = match prefixo {
            Some(p) => p
                .trim()
                .parse::<u8>()
                .map_err(|_| format!("prefixo inválido: {texto}"))?,
            None if addr.is_ipv4() => 32,
            None => 128,
        };
        Cidr::new(addr, prefix).ok_or_else(|| format!("prefixo inválido: {texto}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_e_escreve_o_endereco_com_prefixo() {
        let cidr: Cidr = "10.42.0.7/24".parse().unwrap();
        assert_eq!(cidr.to_string(), "10.42.0.7/24");
        assert_eq!(cidr.network().to_string(), "10.42.0.0");
        assert_eq!("10.1.1.1".parse::<Cidr>().unwrap().prefix(), 32);
        assert!("10.1.1.1/33".parse::<Cidr>().is_err());
        assert!("nada".parse::<Cidr>().is_err());
    }

    #[test]
    fn contem_so_o_que_esta_na_rede() {
        let rede: Cidr = "192.168.4.0/22".parse().unwrap();
        assert!(rede.contains("192.168.7.255".parse().unwrap()));
        assert!(!rede.contains("192.168.8.0".parse().unwrap()));
        assert!(!rede.contains("::1".parse().unwrap()));
        let tudo: Cidr = "0.0.0.0/0".parse().unwrap();
        assert!(tudo.contains("8.8.8.8".parse().unwrap()));
        let v6: Cidr = "fd00::/8".parse().unwrap();
        assert!(v6.contains("fd12::1".parse().unwrap()));
    }
}
