//! O roteamento por chave criptográfica (`AllowedIPs`).
//!
//! Cada par declara as faixas de endereços que pode usar. Para mandar um pacote, o destino
//! escolhe o par (a faixa mais específica vence); para aceitar um pacote que chegou, o
//! endereço de origem tem de pertencer ao par que o mandou. A tabela é uma árvore binária de
//! prefixos, uma por família de endereços.

use std::fmt;
use std::net::IpAddr;
use std::str::FromStr;

/// Um erro ao ler uma faixa CIDR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CidrError {
    /// O texto não tem a forma `endereço/prefixo`.
    Formato(String),
    /// O prefixo passa do tamanho do endereço (32 no IPv4, 128 no IPv6).
    Prefixo(u8),
}

impl fmt::Display for CidrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CidrError::Formato(t) => write!(
                f,
                "faixa de endereços inválida: \"{t}\"; esperado: endereço/prefixo, como 10.0.0.0/24"
            ),
            CidrError::Prefixo(p) => write!(f, "o prefixo /{p} é maior que o endereço comporta"),
        }
    }
}

impl std::error::Error for CidrError {}

/// Uma faixa de endereços (`10.0.0.0/24`, `fd00::/64`). Os bits do endereço depois do prefixo
/// são zerados.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Cidr {
    addr: IpAddr,
    prefix: u8,
}

fn bits_de(ip: IpAddr) -> (u128, u8) {
    match ip {
        IpAddr::V4(v) => (u128::from(u32::from(v)), 32),
        IpAddr::V6(v) => (u128::from(v), 128),
    }
}

impl Cidr {
    /// Cria a faixa, zerando os bits do endereço depois do prefixo.
    pub fn new(addr: IpAddr, prefix: u8) -> Result<Self, CidrError> {
        let (bits, largura) = bits_de(addr);
        if prefix > largura {
            return Err(CidrError::Prefixo(prefix));
        }
        let normalizado = zerar_depois(bits, largura, prefix);
        let addr = match addr {
            IpAddr::V4(_) => IpAddr::V4(std::net::Ipv4Addr::from(normalizado as u32)),
            IpAddr::V6(_) => IpAddr::V6(std::net::Ipv6Addr::from(normalizado)),
        };
        Ok(Cidr { addr, prefix })
    }

    /// O endereço da faixa.
    pub fn addr(&self) -> IpAddr {
        self.addr
    }

    /// O tamanho do prefixo.
    pub fn prefix(&self) -> u8 {
        self.prefix
    }

    /// Se `ip` pertence à faixa.
    pub fn contains(&self, ip: IpAddr) -> bool {
        if ip.is_ipv4() != self.addr.is_ipv4() {
            return false;
        }
        let (bits, largura) = bits_de(ip);
        zerar_depois(bits, largura, self.prefix) == bits_de(self.addr).0
    }
}

/// Zera os bits de `bits` (de `largura` bits) depois dos `prefixo` primeiros.
fn zerar_depois(bits: u128, largura: u8, prefixo: u8) -> u128 {
    let resto = u32::from(largura - prefixo);
    if resto >= 128 {
        0
    } else {
        (bits >> resto) << resto
    }
}

impl fmt::Display for Cidr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.addr, self.prefix)
    }
}

impl FromStr for Cidr {
    type Err = CidrError;

    fn from_str(texto: &str) -> Result<Self, CidrError> {
        let erro = || CidrError::Formato(texto.to_string());
        let (ip, prefixo) = match texto.trim().split_once('/') {
            Some((ip, p)) => (ip, Some(p)),
            None => (texto.trim(), None),
        };
        let addr: IpAddr = ip.trim().parse().map_err(|_| erro())?;
        let prefixo = match prefixo {
            Some(p) => p.trim().parse::<u8>().map_err(|_| erro())?,
            None => bits_de(addr).1,
        };
        Cidr::new(addr, prefixo)
    }
}

struct No<V> {
    filhos: [Option<usize>; 2],
    valor: Option<V>,
}

/// A árvore de uma família de endereços.
struct Arvore<V> {
    nos: Vec<No<V>>,
}

impl<V> Arvore<V> {
    fn new() -> Self {
        Arvore {
            nos: vec![No {
                filhos: [None, None],
                valor: None,
            }],
        }
    }

    fn inserir(&mut self, bits: u128, largura: u8, prefixo: u8, valor: V) {
        let mut atual = 0;
        for i in 0..prefixo {
            let bit = ((bits >> (largura - 1 - i)) & 1) as usize;
            atual = match self.nos[atual].filhos[bit] {
                Some(f) => f,
                None => {
                    self.nos.push(No {
                        filhos: [None, None],
                        valor: None,
                    });
                    let novo = self.nos.len() - 1;
                    self.nos[atual].filhos[bit] = Some(novo);
                    novo
                }
            };
        }
        self.nos[atual].valor = Some(valor);
    }

    fn buscar(&self, bits: u128, largura: u8) -> Option<&V> {
        let mut atual = 0;
        let mut melhor = self.nos[0].valor.as_ref();
        for i in 0..largura {
            let bit = ((bits >> (largura - 1 - i)) & 1) as usize;
            match self.nos[atual].filhos[bit] {
                Some(f) => atual = f,
                None => break,
            }
            if let Some(v) = self.nos[atual].valor.as_ref() {
                melhor = Some(v);
            }
        }
        melhor
    }

    /// Percorre as entradas: `(bits, prefixo, valor)`.
    fn percorrer<'a>(&'a self, largura: u8, saida: &mut Vec<(u128, u8, &'a V)>) {
        let mut pilha = vec![(0usize, 0u128, 0u8)];
        while let Some((no, bits, profundidade)) = pilha.pop() {
            if let Some(v) = self.nos[no].valor.as_ref() {
                saida.push((bits << (largura - profundidade), profundidade, v));
            }
            for (bit, filho) in self.nos[no].filhos.iter().enumerate() {
                if let Some(f) = filho {
                    pilha.push((*f, (bits << 1) | bit as u128, profundidade + 1));
                }
            }
        }
    }
}

/// A tabela de `AllowedIPs`: faixa de endereços para o valor `V` (em geral, a chave do par).
pub struct Router<V> {
    v4: Arvore<V>,
    v6: Arvore<V>,
}

impl<V> Default for Router<V> {
    fn default() -> Self {
        Router::new()
    }
}

impl<V> Router<V> {
    /// Uma tabela vazia.
    pub fn new() -> Self {
        Router {
            v4: Arvore::new(),
            v6: Arvore::new(),
        }
    }

    /// Põe a faixa na tabela. Se a faixa idêntica já existia, o valor é trocado.
    pub fn insert(&mut self, faixa: Cidr, valor: V) {
        let (bits, largura) = bits_de(faixa.addr);
        match faixa.addr {
            IpAddr::V4(_) => self.v4.inserir(bits, largura, faixa.prefix, valor),
            IpAddr::V6(_) => self.v6.inserir(bits, largura, faixa.prefix, valor),
        }
    }

    /// O valor da faixa mais específica que contém `ip`.
    pub fn lookup(&self, ip: IpAddr) -> Option<&V> {
        let (bits, largura) = bits_de(ip);
        match ip {
            IpAddr::V4(_) => self.v4.buscar(bits, largura),
            IpAddr::V6(_) => self.v6.buscar(bits, largura),
        }
    }

    /// Todas as faixas e seus valores.
    pub fn entries(&self) -> Vec<(Cidr, &V)> {
        let mut saida = Vec::new();
        let mut v4 = Vec::new();
        self.v4.percorrer(32, &mut v4);
        for (bits, prefixo, v) in v4 {
            let addr = IpAddr::V4(std::net::Ipv4Addr::from(bits as u32));
            saida.push((
                Cidr {
                    addr,
                    prefix: prefixo,
                },
                v,
            ));
        }
        let mut v6 = Vec::new();
        self.v6.percorrer(128, &mut v6);
        for (bits, prefixo, v) in v6 {
            let addr = IpAddr::V6(std::net::Ipv6Addr::from(bits));
            saida.push((
                Cidr {
                    addr,
                    prefix: prefixo,
                },
                v,
            ));
        }
        saida
    }

    /// Quantas faixas há.
    pub fn len(&self) -> usize {
        self.entries().len()
    }

    /// Se a tabela está vazia.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<V: Clone + PartialEq> Router<V> {
    /// Tira da tabela todas as faixas que levam a `valor` (ao remover um par).
    pub fn remove_value(&mut self, valor: &V) {
        let restantes: Vec<(Cidr, V)> = self
            .entries()
            .into_iter()
            .filter(|(_, v)| *v != valor)
            .map(|(c, v)| (c, v.clone()))
            .collect();
        *self = Router::new();
        for (c, v) in restantes {
            self.insert(c, v);
        }
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    fn c(t: &str) -> Cidr {
        t.parse().unwrap()
    }

    fn ip(t: &str) -> IpAddr {
        t.parse().unwrap()
    }

    #[test]
    fn a_faixa_mais_especifica_vence() {
        let mut r = Router::new();
        r.insert(c("0.0.0.0/0"), 1);
        r.insert(c("10.0.0.0/8"), 2);
        r.insert(c("10.1.0.0/16"), 3);
        r.insert(c("10.1.2.3/32"), 4);
        assert_eq!(r.lookup(ip("8.8.8.8")), Some(&1));
        assert_eq!(r.lookup(ip("10.9.9.9")), Some(&2));
        assert_eq!(r.lookup(ip("10.1.9.9")), Some(&3));
        assert_eq!(r.lookup(ip("10.1.2.3")), Some(&4));
        assert_eq!(r.lookup(ip("10.1.2.4")), Some(&3));
    }

    #[test]
    fn sem_faixa_que_cubra_nao_ha_par() {
        let mut r = Router::new();
        r.insert(c("10.0.0.0/24"), 1);
        assert_eq!(r.lookup(ip("10.0.1.1")), None);
        assert_eq!(r.lookup(ip("fd00::1")), None);
    }

    #[test]
    fn as_familias_sao_separadas() {
        let mut r = Router::new();
        r.insert(c("0.0.0.0/0"), 1);
        r.insert(c("::/0"), 2);
        r.insert(c("fd00::/64"), 3);
        assert_eq!(r.lookup(ip("1.2.3.4")), Some(&1));
        assert_eq!(r.lookup(ip("2001:db8::1")), Some(&2));
        assert_eq!(r.lookup(ip("fd00::5")), Some(&3));
        assert_eq!(r.lookup(ip("fd00:0:0:1::5")), Some(&2));
    }

    #[test]
    fn inserir_a_mesma_faixa_troca_o_valor() {
        let mut r = Router::new();
        r.insert(c("10.0.0.0/24"), 1);
        r.insert(c("10.0.0.0/24"), 2);
        assert_eq!(r.lookup(ip("10.0.0.7")), Some(&2));
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn remover_um_valor_tira_todas_as_faixas_dele() {
        let mut r = Router::new();
        r.insert(c("10.0.0.0/24"), 1);
        r.insert(c("10.0.1.0/24"), 1);
        r.insert(c("fd00::/64"), 1);
        r.insert(c("10.0.2.0/24"), 2);
        r.remove_value(&1);
        assert_eq!(r.lookup(ip("10.0.0.7")), None);
        assert_eq!(r.lookup(ip("fd00::7")), None);
        assert_eq!(r.lookup(ip("10.0.2.7")), Some(&2));
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn o_cidr_zera_os_bits_do_host_e_confere_a_pertinencia() {
        let faixa = c("10.1.2.99/24");
        assert_eq!(faixa.to_string(), "10.1.2.0/24");
        assert!(faixa.contains(ip("10.1.2.200")));
        assert!(!faixa.contains(ip("10.1.3.1")));
        assert!(!faixa.contains(ip("fd00::1")));
        assert_eq!(c("1.2.3.4").prefix(), 32);
        assert_eq!(c("::1").prefix(), 128);
        assert_eq!(c("fd00::1/64").to_string(), "fd00::/64");
        assert!(c("0.0.0.0/0").contains(ip("200.1.1.1")));
    }

    #[test]
    fn cidr_invalido_tem_mensagem_em_portugues() {
        let e = "isto".parse::<Cidr>().unwrap_err();
        assert!(e.to_string().contains("faixa de endereços inválida"));
        assert_eq!(
            "10.0.0.0/33".parse::<Cidr>().unwrap_err(),
            CidrError::Prefixo(33)
        );
        assert!("10.0.0.0/x".parse::<Cidr>().is_err());
    }

    #[test]
    fn as_entradas_voltam_com_o_endereco_certo() {
        let mut r = Router::new();
        r.insert(c("10.1.0.0/16"), 1);
        r.insert(c("192.168.5.0/24"), 2);
        r.insert(c("fd00:1::/32"), 3);
        let mut e: Vec<String> = r
            .entries()
            .iter()
            .map(|(c, v)| format!("{c}={v}"))
            .collect();
        e.sort();
        assert_eq!(e, ["10.1.0.0/16=1", "192.168.5.0/24=2", "fd00:1::/32=3"]);
    }
}
