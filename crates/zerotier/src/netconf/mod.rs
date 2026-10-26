//! A configuração da rede virtual: o que o controlador manda e o que o nó pede (`NetworkConfig`,
//! `Dictionary` de chaves curtas). Aqui ficam o dicionário, o certificado de pertinência, as
//! rotas, as regras e o DNS; o pedido e a remontagem dos pedaços ficam em [`request`].

pub mod com;
pub mod dns;
pub mod request;
pub mod route;
pub mod rules;

use std::fmt;

use tomni_ui_netstack::Cidr;

use crate::dictionary::Dictionary;
use crate::{Address, Error, NetworkId, Result};

pub use com::Com;
pub use dns::Dns;
pub use route::ManagedRoute;
pub use rules::Rule;

/// A versão da configuração que sabemos ler (`ZT_NETWORKCONFIG_VERSION`). A 6 e as anteriores
/// são de controladores antigos demais.
pub const CONFIG_VERSION: u64 = 7;

/// O MTU mínimo e o máximo que o protocolo aceita, e o padrão (`ZT_MIN_MTU`, `ZT_MAX_MTU`,
/// `ZT_DEFAULT_MTU`).
pub const MIN_MTU: u64 = 1280;
/// O MTU máximo.
pub const MAX_MTU: u64 = 10_000;
/// O MTU padrão.
pub const DEFAULT_MTU: u64 = 2800;

/// Flag: o broadcast Ethernet está ligado.
pub const FLAG_ENABLE_BROADCAST: u64 = 0x02;
/// Flag: o NDP (IPv6) está ligado.
pub const FLAG_ENABLE_NDP: u64 = 0x04;
/// Flag: as regras não couberam inteiras (o controlador cortou).
pub const FLAG_RULES_INCOMPLETE: u64 = 0x08;
/// Flag: sem compressão LZ4 nos pacotes da rede.
pub const FLAG_NO_COMPRESSION: u64 = 0x10;

/// Bit do especialista: ponte ativa.
pub const SPECIALIST_ACTIVE_BRIDGE: u64 = 0x0000_0200_0000_0000;
/// Bit do especialista: replicador de multicast.
pub const SPECIALIST_MULTICAST_REPLICATOR: u64 = 0x0000_0800_0000_0000;
/// Bit do especialista: relay da rede.
pub const SPECIALIST_RELAY: u64 = 0x0000_1000_0000_0000;

/// Algo da configuração que o app não trata. A tela mostra como aviso.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Warning {
    /// A rede define capabilities, que o app ignora.
    Capabilities,
    /// A rede define tags, que o app trata como ausentes.
    Tags,
    /// A rede usa certificados de propriedade de IP, que o app não valida.
    OwnershipCertificates,
    /// O controlador cortou as regras: o que não coube não vale.
    RulesIncomplete,
    /// A rede pede SSO, que o app não faz.
    SingleSignOn,
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Warning::Capabilities => {
                "A rede define capabilities, que o Tirech Omni ignora: as regras que dependem delas não casam."
            }
            Warning::Tags => {
                "A rede define tags, que o Tirech Omni não tem: as regras que dependem delas não casam."
            }
            Warning::OwnershipCertificates => {
                "A rede usa certificados de propriedade de IP, que o Tirech Omni não confere."
            }
            Warning::RulesIncomplete => {
                "O controlador cortou as regras da rede por tamanho: o que não coube não vale."
            }
            Warning::SingleSignOn => "A rede pede login único (SSO), que o Tirech Omni não faz.",
        })
    }
}

/// A configuração de uma rede, já lida.
#[derive(Clone, Debug)]
pub struct NetworkConfig {
    /// A rede.
    pub network: NetworkId,
    /// O carimbo de tempo da configuração, em ms (do relógio do controlador).
    pub timestamp: u64,
    /// A revisão da configuração.
    pub revision: u64,
    /// A quem a configuração foi emitida.
    pub issued_to: Address,
    /// As flags (`FLAG_*`).
    pub flags: u64,
    /// O limite de multicast por grupo (0 desliga o multicast).
    pub multicast_limit: u32,
    /// A rede é pública (sem certificado de pertinência).
    pub public: bool,
    /// O nome da rede.
    pub name: String,
    /// O MTU.
    pub mtu: u16,
    /// O nosso certificado de pertinência (redes privadas).
    pub com: Option<Com>,
    /// Os especialistas: endereço e bits (`SPECIALIST_*`).
    pub specialists: Vec<(Address, u64)>,
    /// As rotas.
    pub routes: Vec<ManagedRoute>,
    /// Os IPs atribuídos a nós (com o prefixo).
    pub addresses: Vec<Cidr>,
    /// As regras.
    pub rules: Vec<Rule>,
    /// O DNS da rede.
    pub dns: Option<Dns>,
    /// O que o app não trata.
    pub warnings: Vec<Warning>,
}

fn erro(motivo: &str) -> Error {
    Error::Packet(format!("configuração da rede: {motivo}"))
}

impl NetworkConfig {
    /// Lê a configuração do dicionário. `rede` e `nosso` conferem o `nwid` e o `id` (a quem foi
    /// emitida): uma configuração de outra rede ou de outro nó não vale.
    pub fn from_dictionary(dict: &Dictionary, rede: NetworkId, nosso: Address) -> Result<Self> {
        let versao = dict.get_u64("v").unwrap_or(0);
        if versao < 6 {
            return Err(erro("versão não suportada"));
        }
        let nwid = dict
            .get_u64("nwid")
            .ok_or_else(|| erro("sem o id da rede"))?;
        if nwid != rede.as_u64() {
            return Err(erro("é de outra rede"));
        }
        let emitida = dict.get_u64("id").ok_or_else(|| erro("sem o dono"))?;
        if Address::new(emitida) != nosso {
            return Err(erro("é de outro membro"));
        }
        let flags = dict.get_u64("f").unwrap_or(0);
        let mut warnings = Vec::new();
        if flags & FLAG_RULES_INCOMPLETE != 0 {
            warnings.push(Warning::RulesIncomplete);
        }
        let mtu = dict
            .get_u64("mtu")
            .map_or(DEFAULT_MTU, |m| m.clamp(MIN_MTU, MAX_MTU)) as u16;
        let public = dict.get_u64("t").unwrap_or(0) == 1;
        let com = match dict.get("C") {
            Some(b) => Some(
                Com::from_bytes(b)
                    .ok_or_else(|| erro("certificado ilegível"))?
                    .0,
            ),
            None => None,
        };
        if !public && com.is_none() {
            return Err(erro("rede privada sem certificado de pertinência"));
        }
        let specialists = dict
            .get("S")
            .map(|b| {
                b.chunks_exact(8)
                    .take(256)
                    .map(|c| {
                        let v = u64::from_be_bytes(c.try_into().expect("8 bytes"));
                        (Address::new(v), v & !0xff_ffff_ffff)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let routes = match dict.get("RT") {
            Some(b) => route::parse_routes(b).ok_or_else(|| erro("rotas ilegíveis"))?,
            None => vec![],
        };
        let addresses = match dict.get("I") {
            Some(b) => route::parse_addresses(b).ok_or_else(|| erro("IPs ilegíveis"))?,
            None => vec![],
        };
        let rules = match dict.get("R") {
            Some(b) => rules::parse(b).ok_or_else(|| erro("regras ilegíveis"))?,
            None => vec![],
        };
        if dict.get("CAP").is_some_and(|b| !b.is_empty()) {
            warnings.push(Warning::Capabilities);
        }
        if dict.get("TAG").is_some_and(|b| !b.is_empty()) {
            warnings.push(Warning::Tags);
        }
        if dict.get("COO").is_some_and(|b| !b.is_empty()) {
            warnings.push(Warning::OwnershipCertificates);
        }
        if dict.get("ssoe").is_some_and(|b| b.first() == Some(&b'1')) {
            warnings.push(Warning::SingleSignOn);
        }
        let dns = dict.get("DNS").and_then(Dns::parse);
        Ok(Self {
            network: rede,
            timestamp: dict.get_u64("ts").unwrap_or(0),
            revision: dict.get_u64("r").unwrap_or(0),
            issued_to: nosso,
            flags,
            multicast_limit: dict.get_u64("ml").unwrap_or(32) as u32,
            public,
            name: dict.get_str("n").unwrap_or_default().to_string(),
            mtu,
            com,
            specialists,
            routes,
            addresses,
            rules,
            dns,
            warnings,
        })
    }

    /// Escreve no dicionário, como o controlador faz. Usado pelo `ControllerFake`.
    pub fn to_dictionary(&self) -> Dictionary {
        let mut d = Dictionary::new();
        d.add_u64("v", CONFIG_VERSION);
        d.add_u64("nwid", self.network.as_u64());
        d.add_u64("ts", self.timestamp);
        d.add_u64("r", self.revision);
        d.add_u64("id", self.issued_to.as_u64());
        d.add_u64("f", self.flags);
        d.add_u64("ml", u64::from(self.multicast_limit));
        d.add_u64("t", u64::from(self.public));
        d.add_str("n", &self.name);
        d.add_u64("mtu", u64::from(self.mtu));
        if let Some(c) = &self.com {
            d.add("C", &c.to_bytes());
        }
        if !self.specialists.is_empty() {
            let mut s = Vec::new();
            for (a, bits) in &self.specialists {
                s.extend_from_slice(&(a.as_u64() | bits).to_be_bytes());
            }
            d.add("S", &s);
        }
        if !self.routes.is_empty() {
            d.add("RT", &route::write_routes(&self.routes));
        }
        if !self.addresses.is_empty() {
            d.add("I", &route::write_addresses(&self.addresses));
        }
        if !self.rules.is_empty() {
            d.add("R", &rules::serialize(&self.rules));
        }
        if let Some(dns) = &self.dns {
            d.add("DNS", &dns.to_bytes());
        }
        d
    }

    /// O broadcast Ethernet está ligado?
    pub fn broadcast_enabled(&self) -> bool {
        self.flags & FLAG_ENABLE_BROADCAST != 0
    }

    /// Os endereços de quem faz o papel de `bits` na rede.
    pub fn specialists_with(&self, bits: u64) -> impl Iterator<Item = Address> + '_ {
        self.specialists
            .iter()
            .filter(move |(_, b)| b & bits != 0)
            .map(|(a, _)| *a)
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    fn base(nosso: Address, rede: NetworkId) -> NetworkConfig {
        NetworkConfig {
            network: rede,
            timestamp: 1234,
            revision: 3,
            issued_to: nosso,
            flags: FLAG_ENABLE_BROADCAST,
            multicast_limit: 32,
            public: true,
            name: "teste".into(),
            mtu: 2800,
            com: None,
            specialists: vec![(Address::new(0x1122334455), SPECIALIST_MULTICAST_REPLICATOR)],
            routes: vec![ManagedRoute {
                target: Cidr::new("10.61.0.0".parse().unwrap(), 24).unwrap(),
                via: None,
                flags: 0,
                metric: 0,
            }],
            addresses: vec![Cidr::new("10.61.0.7".parse().unwrap(), 24).unwrap()],
            rules: vec![Rule::accept()],
            dns: None,
            warnings: vec![],
        }
    }

    #[test]
    fn a_configuracao_vai_e_volta_pelo_dicionario() {
        let nosso = Address::new(0xaabbccddee);
        let rede = NetworkId::new(0x1122_3344_5500_0001);
        let cfg = base(nosso, rede);
        let lida = NetworkConfig::from_dictionary(
            &Dictionary::parse(&cfg.to_dictionary().to_bytes()),
            rede,
            nosso,
        )
        .unwrap();
        assert_eq!(lida.name, "teste");
        assert_eq!(lida.mtu, 2800);
        assert_eq!(lida.routes, cfg.routes);
        assert_eq!(lida.addresses, cfg.addresses);
        assert_eq!(lida.rules, cfg.rules);
        assert_eq!(lida.specialists, cfg.specialists);
        assert!(lida.broadcast_enabled());
        assert_eq!(
            lida.specialists_with(SPECIALIST_MULTICAST_REPLICATOR)
                .collect::<Vec<_>>(),
            vec![Address::new(0x1122334455)]
        );
    }

    #[test]
    fn config_de_outra_rede_ou_de_outro_membro_nao_vale() {
        let nosso = Address::new(0xaabbccddee);
        let rede = NetworkId::new(0x1122_3344_5500_0001);
        let d = Dictionary::parse(&base(nosso, rede).to_dictionary().to_bytes());
        assert!(
            NetworkConfig::from_dictionary(&d, NetworkId::new(0x1122_3344_5500_0002), nosso)
                .is_err()
        );
        assert!(NetworkConfig::from_dictionary(&d, rede, Address::new(1)).is_err());
    }

    #[test]
    fn versao_antiga_e_rede_privada_sem_certificado_sao_recusadas() {
        let nosso = Address::new(0xaabbccddee);
        let rede = NetworkId::new(0x1122_3344_5500_0001);
        let mut d = base(nosso, rede).to_dictionary();
        let mut velho = Dictionary::new();
        velho.add_u64("v", 5);
        for chave in ["nwid", "id"] {
            velho.add_u64(chave, d.get_u64(chave).unwrap());
        }
        assert!(NetworkConfig::from_dictionary(&velho, rede, nosso).is_err());

        let mut privada = base(nosso, rede);
        privada.public = false;
        d = privada.to_dictionary();
        assert!(NetworkConfig::from_dictionary(&d, rede, nosso).is_err());
    }

    #[test]
    fn mtu_fora_dos_limites_e_ajustado() {
        let nosso = Address::new(0xaabbccddee);
        let rede = NetworkId::new(0x1122_3344_5500_0001);
        let mut cfg = base(nosso, rede);
        cfg.mtu = 100;
        let mut d = cfg.to_dictionary();
        let lida = NetworkConfig::from_dictionary(&d, rede, nosso).unwrap();
        assert_eq!(u64::from(lida.mtu), MIN_MTU);
        d = Dictionary::parse(&d.to_bytes());
        assert!(d.get("mtu").is_some());
    }
}
