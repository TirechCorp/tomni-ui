//! O `config.yml` do Nebula: as chaves que o motor usa, com os padrões do Nebula, e o que ele
//! ignora (a placa virtual, o `sshd`, as métricas) ou ainda não faz (o firewall de saída).
//! Portado do que o Nebula lê em `config/`, `pki.go`, `lighthouse.go` e `punchy.go`
//! (github.com/slackhq/nebula, licença MIT).

use std::collections::BTreeMap;
use std::fmt;
use std::net::IpAddr;
use std::path::Path;
use std::time::Duration;

use tomni_ui_netstack::Cidr;
use yaml_rust2::{Yaml, YamlLoader};
use zeroize::Zeroizing;

use crate::cert::{CaPool, CertError, Certificate, Curve, PrivateKey, Version};
use crate::util::format_date;

/// Um config inválido. O texto é a mensagem para o usuário, em português.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigError {
    message: String,
}

impl ConfigError {
    pub(crate) fn new(message: impl Into<String>) -> ConfigError {
        ConfigError {
            message: message.into(),
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ConfigError {}

impl From<CertError> for ConfigError {
    fn from(erro: CertError) -> ConfigError {
        ConfigError::new(erro.to_string())
    }
}

/// Algo do config que foi ignorado ou não é aplicado, e que o usuário merece saber.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Warning {
    /// A chave do config (`firewall.outbound`).
    pub key: String,
    /// O que aconteceu, em português.
    pub message: String,
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.key, self.message)
    }
}

/// A gravidade de um [`Problem`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    /// Com isto a rede não sobe.
    Error,
    /// A rede sobe, mas o usuário precisa saber.
    Warning,
}

/// Uma coisa errada (ou quase) que o [`Config::check`] achou.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    /// A gravidade.
    pub severity: Severity,
    /// A mensagem, em português.
    pub message: String,
}

/// A cifra do túnel (a chave `cipher`). Precisa ser a mesma em toda a rede.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cipher {
    /// AES-256-GCM (o padrão).
    Aes,
    /// ChaCha20-Poly1305.
    ChaChaPoly,
}

/// O `static_map.network`: que famílias de endereço valem ao resolver os nomes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StaticMapNetwork {
    /// Só IPv4 (o padrão).
    Ip4,
    /// Só IPv6.
    Ip6,
    /// As duas.
    Ip,
}

/// O que vem do `pki`: os textos PEM (já lidos dos arquivos) e as opções.
#[derive(Clone, Default)]
pub struct Pki {
    /// As CAs em que a rede confia (PEM).
    pub ca: Option<String>,
    /// O certificado do nó (PEM; pode trazer a v1 e a v2).
    pub cert: Option<String>,
    /// A chave privada do nó (PEM).
    pub key: Option<Zeroizing<String>>,
    /// As impressões digitais bloqueadas.
    pub blocklist: Vec<String>,
    /// A versão do certificado com que o nó inicia os handshakes; `None` é o padrão do Nebula
    /// (a v1 se há, senão a v2).
    pub initiating_version: Option<u8>,
    /// Derrubar o túnel quando o certificado do par vence (`pki.disconnect_invalid`).
    pub disconnect_invalid: bool,
}

impl fmt::Debug for Pki {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pki")
            .field("ca", &self.ca.is_some())
            .field("cert", &self.cert.is_some())
            .field("key", &self.key.is_some())
            .field("blocklist", &self.blocklist)
            .field("initiating_version", &self.initiating_version)
            .field("disconnect_invalid", &self.disconnect_invalid)
            .finish()
    }
}

/// O `lighthouse`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LighthouseConfig {
    /// Os endereços Nebula dos lighthouses (`lighthouse.hosts`).
    pub hosts: Vec<IpAddr>,
    /// De quanto em quanto tempo avisa os lighthouses (`lighthouse.interval`, em segundos no
    /// config; padrão 10 s).
    pub interval: Duration,
    /// Os endereços que o nó anuncia no lugar dos que descobre (`lighthouse.advertise_addrs`).
    pub advertise_addrs: Vec<String>,
}

/// O `listen`: onde o UDP escuta.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listen {
    /// O endereço (`0.0.0.0` e `::` valem como "todas as placas"; o underlay é IPv4).
    pub host: String,
    /// A porta; `0` deixa o sistema escolher (a recomendada para o app).
    pub port: u16,
}

/// O `punchy`: furar o NAT.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Punchy {
    /// Mandar pacotes vazios para manter o NAT aberto e furar o do par (`punchy.punch`).
    pub punch: bool,
    /// Responder ao punch do par (`punchy.respond`).
    pub respond: bool,
    /// O atraso antes do punch (`punchy.delay`; padrão 1 s).
    pub delay: Duration,
    /// O atraso antes da resposta (`punchy.respond_delay`; padrão 5 s).
    pub respond_delay: Duration,
    /// Furar todos os endereços do par, e não só o último (`punchy.target_all_remotes`).
    pub target_all_remotes: bool,
}

/// O `relay`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelayConfig {
    /// Os relays pelos quais este nó aceita ser alcançado (`relay.relays`).
    pub relays: Vec<IpAddr>,
    /// Usar relays quando não há caminho direto (`relay.use_relays`; padrão `true`).
    pub use_relays: bool,
}

/// O `handshakes`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Handshakes {
    /// O intervalo entre as tentativas (`try_interval`; padrão 100 ms).
    pub try_interval: Duration,
    /// Quantas vezes repete antes de desistir (`retries`; padrão 10).
    pub retries: u32,
}

/// Uma rota para uma rede de fora do Nebula, alcançada por um par (`tun.unsafe_routes`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnsafeRoute {
    /// A rede de destino.
    pub route: Cidr,
    /// Os endereços Nebula dos pares que a alcançam.
    pub via: Vec<IpAddr>,
}

/// O que o `config.yml` define e o motor usa.
#[derive(Clone, Debug)]
pub struct Config {
    /// O `pki`.
    pub pki: Pki,
    /// O `static_host_map`: o endereço Nebula → os `host:porta` do UDP.
    pub static_host_map: BTreeMap<IpAddr, Vec<String>>,
    /// O `static_map.network`.
    pub static_map_network: StaticMapNetwork,
    /// O `static_map.cadence`: de quanto em quanto tempo os nomes são resolvidos de novo
    /// (padrão 30 s).
    pub static_map_cadence: Duration,
    /// O `lighthouse`.
    pub lighthouse: LighthouseConfig,
    /// O `listen`.
    pub listen: Listen,
    /// O `punchy`.
    pub punchy: Punchy,
    /// O `relay`.
    pub relay: RelayConfig,
    /// A `cipher`.
    pub cipher: Cipher,
    /// O `handshakes`.
    pub handshakes: Handshakes,
    /// O `tun.mtu`: o tamanho máximo do pacote IP dentro do túnel (padrão 1300).
    pub mtu: u16,
    /// O `tun.unsafe_routes`.
    pub unsafe_routes: Vec<UnsafeRoute>,
    /// O `timers.connection_alive_interval`: de quanto em quanto tempo o túnel é conferido
    /// (padrão 5 s).
    pub connection_alive_interval: Duration,
    /// O `timers.pending_deletion_interval`: quanto um túnel sem uso espera antes de cair
    /// (padrão 10 s).
    pub pending_deletion_interval: Duration,
    /// O `tunnels.inactivity_timeout` (padrão 10 min).
    pub inactivity_timeout: Duration,
    /// O `tunnels.drop_inactive` (padrão `false`).
    pub drop_inactive: bool,
}

/// As chaves do topo que o Nebula conhece. Qualquer outra vira aviso.
const TOP_LEVEL: &[&str] = &[
    "pki",
    "static_host_map",
    "static_map",
    "lighthouse",
    "listen",
    "punchy",
    "punch_back",
    "relay",
    "tun",
    "logging",
    "sshd",
    "firewall",
    "stats",
    "handshakes",
    "timers",
    "tunnels",
    "preferred_ranges",
    "cipher",
    "local_range",
    "routines",
    "counters",
    "remote_allow_list",
    "remote_allow_ranges",
];

impl Config {
    /// Lê um `config.yml`. A `pasta` resolve os caminhos relativos do `pki`; sem ela, o `pki`
    /// precisa vir embutido (texto PEM no próprio YAML) ou pelo [`Config::set_pki`].
    ///
    /// Os avisos dizem o que foi ignorado ou não é aplicado. Um config de lighthouse
    /// (`lighthouse.am_lighthouse: true`) é recusado.
    pub fn from_yaml(
        texto: &str,
        pasta: Option<&Path>,
    ) -> Result<(Config, Vec<Warning>), ConfigError> {
        let docs = YamlLoader::load_from_str(texto)
            .map_err(|erro| ConfigError::new(format!("o config não é um YAML válido: {erro}")))?;
        let raiz = docs.first().cloned().unwrap_or(Yaml::Null);
        let mut avisos = Vec::new();
        let leitor = Leitor { raiz: &raiz };
        let mut aviso = |chave: &str, mensagem: String| {
            avisos.push(Warning {
                key: chave.to_owned(),
                message: mensagem,
            });
        };

        if let Yaml::Hash(mapa) = &raiz {
            for chave in mapa.keys().filter_map(Yaml::as_str) {
                if !TOP_LEVEL.contains(&chave) {
                    aviso(chave, "chave desconhecida; foi ignorada".to_owned());
                }
            }
        } else if !matches!(raiz, Yaml::Null) {
            return Err(ConfigError::new("o config precisa ser um mapa de chaves"));
        }

        if leitor.boolean("lighthouse.am_lighthouse", false) {
            return Err(ConfigError::new(
                "este config é de um lighthouse (lighthouse.am_lighthouse: true); use o config de um nó",
            ));
        }

        // pki
        let mut pki = Pki {
            disconnect_invalid: leitor.boolean("pki.disconnect_invalid", true),
            ..Pki::default()
        };
        for (chave, destino) in [("pki.ca", &mut pki.ca), ("pki.cert", &mut pki.cert)] {
            if let Some(valor) = leitor.texto(chave) {
                match pem_ou_arquivo(chave, &valor, pasta)? {
                    Some(pem) => *destino = Some(pem),
                    None => aviso(
                        chave,
                        format!(
                            "aponta para o arquivo {valor}, que não foi lido (sem a pasta do config); use Config::set_pki"
                        ),
                    ),
                }
            }
        }
        if let Some(valor) = leitor.texto("pki.key") {
            match pem_ou_arquivo("pki.key", &valor, pasta)? {
                Some(pem) => pki.key = Some(Zeroizing::new(pem)),
                None => aviso(
                    "pki.key",
                    format!(
                        "aponta para o arquivo {valor}, que não foi lido (sem a pasta do config); use Config::set_pki"
                    ),
                ),
            }
        }
        pki.blocklist = leitor.lista("pki.blocklist");
        if let Some(versao) = leitor.numero("pki.initiating_version") {
            if versao != 1 && versao != 2 {
                return Err(ConfigError::new(format!(
                    "pki.initiating_version desconhecida: {versao} (use 1 ou 2)"
                )));
            }
            pki.initiating_version = Some(versao as u8);
        }

        // static_host_map
        let mut static_host_map = BTreeMap::new();
        if let Yaml::Hash(mapa) = leitor.no("static_host_map") {
            for (chave, valor) in mapa {
                let nome = escalar(chave).unwrap_or_default();
                let ip: IpAddr = nome.parse().map_err(|_| {
                    ConfigError::new(format!(
                        "static_host_map: {nome} não é um endereço Nebula (IP) válido"
                    ))
                })?;
                let destinos = match valor {
                    Yaml::Array(itens) => itens.iter().filter_map(escalar).collect(),
                    outro => escalar(outro).into_iter().collect(),
                };
                static_host_map.insert(ip, destinos);
            }
        }
        let static_map_network = match leitor.texto("static_map.network").as_deref() {
            None | Some("ip4") => StaticMapNetwork::Ip4,
            Some("ip6") => StaticMapNetwork::Ip6,
            Some("ip") => StaticMapNetwork::Ip,
            Some(_) => {
                return Err(ConfigError::new(
                    "static_map.network precisa ser ip, ip4 ou ip6",
                ));
            }
        };

        // lighthouse
        let mut hosts = Vec::new();
        for item in leitor.lista("lighthouse.hosts") {
            hosts.push(item.parse().map_err(|_| {
                ConfigError::new(format!(
                    "lighthouse.hosts: {item} não é um endereço Nebula (IP) válido"
                ))
            })?);
        }
        let lighthouse = LighthouseConfig {
            hosts,
            // Em segundos, como número inteiro (`GetInt` do Nebula).
            interval: Duration::from_secs(leitor.numero("lighthouse.interval").unwrap_or(10)),
            advertise_addrs: leitor.lista("lighthouse.advertise_addrs"),
        };

        // listen
        let host = leitor
            .texto("listen.host")
            .unwrap_or_else(|| "0.0.0.0".to_owned());
        let porta = leitor.numero("listen.port").unwrap_or(0);
        let porta = u16::try_from(porta)
            .map_err(|_| ConfigError::new(format!("listen.port inválida: {porta}")))?;
        let listen = Listen { host, port: porta };

        // punchy (as chaves antigas `punchy: true` e `punch_back` também valem)
        let punch_antigo = leitor
            .no("punchy")
            .as_bool()
            .or_else(|| leitor.no("punchy").as_str().map(|_| false));
        let punchy = Punchy {
            punch: leitor.boolean("punchy.punch", punch_antigo.unwrap_or(false)),
            respond: leitor.boolean("punchy.respond", leitor.boolean("punch_back", false)),
            delay: leitor.duracao("punchy.delay", Duration::from_secs(1), &mut avisos),
            respond_delay: leitor.duracao(
                "punchy.respond_delay",
                Duration::from_secs(5),
                &mut avisos,
            ),
            target_all_remotes: leitor.boolean("punchy.target_all_remotes", false),
        };

        // relay
        let mut relays = Vec::new();
        for item in leitor.lista("relay.relays") {
            relays.push(item.parse().map_err(|_| {
                ConfigError::new(format!(
                    "relay.relays: {item} não é um endereço Nebula (IP) válido"
                ))
            })?);
        }
        let relay = RelayConfig {
            relays,
            use_relays: leitor.boolean("relay.use_relays", true),
        };
        if leitor.boolean("relay.am_relay", false) {
            avisos.push(Warning {
                key: "relay.am_relay".to_owned(),
                message: "o app não faz papel de relay; foi ignorado".to_owned(),
            });
        }

        let cipher = match leitor.texto("cipher").as_deref() {
            None | Some("aes") => Cipher::Aes,
            Some("chachapoly") => Cipher::ChaChaPoly,
            Some(outra) => {
                return Err(ConfigError::new(format!(
                    "cifra desconhecida: {outra} (use aes ou chachapoly)"
                )));
            }
        };

        let handshakes = Handshakes {
            try_interval: leitor.duracao(
                "handshakes.try_interval",
                Duration::from_millis(100),
                &mut avisos,
            ),
            retries: leitor.numero("handshakes.retries").map_or(10, |n| n as u32),
        };

        let mtu = leitor.numero("tun.mtu").unwrap_or(1300);
        let mtu = u16::try_from(mtu)
            .ok()
            .filter(|mtu| *mtu >= 100)
            .ok_or_else(|| ConfigError::new(format!("tun.mtu inválido: {mtu}")))?;

        let unsafe_routes = unsafe_routes(leitor.no("tun.unsafe_routes"))?;

        // O que o Nebula usa e o app ignora, ou ainda não aplica.
        if leitor.boolean("sshd.enabled", false) {
            avisos.push(Warning {
                key: "sshd".to_owned(),
                message: "o servidor SSH de administração do Nebula não existe aqui; foi ignorado"
                    .to_owned(),
            });
        }
        if leitor
            .texto("stats.type")
            .is_some_and(|tipo| !tipo.is_empty())
        {
            avisos.push(Warning {
                key: "stats".to_owned(),
                message: "as métricas do Nebula não existem aqui; foram ignoradas".to_owned(),
            });
        }
        if leitor.boolean("lighthouse.serve_dns", false) {
            avisos.push(Warning {
                key: "lighthouse.serve_dns".to_owned(),
                message: "o DNS do lighthouse não existe aqui; foi ignorado".to_owned(),
            });
        }
        if saida_restrita(leitor.no("firewall.outbound")) {
            avisos.push(Warning {
                key: "firewall.outbound".to_owned(),
                message:
                    "regras de saída ainda não aplicadas: o app abre as conexões que o usuário pede"
                        .to_owned(),
            });
        }

        let config = Config {
            pki,
            static_host_map,
            static_map_network,
            static_map_cadence: leitor.duracao(
                "static_map.cadence",
                Duration::from_secs(30),
                &mut avisos,
            ),
            lighthouse,
            listen,
            punchy,
            relay,
            cipher,
            handshakes,
            mtu,
            unsafe_routes,
            connection_alive_interval: Duration::from_secs(
                leitor
                    .numero("timers.connection_alive_interval")
                    .unwrap_or(5),
            ),
            pending_deletion_interval: Duration::from_secs(
                leitor
                    .numero("timers.pending_deletion_interval")
                    .unwrap_or(10),
            ),
            inactivity_timeout: leitor.duracao(
                "tunnels.inactivity_timeout",
                Duration::from_secs(600),
                &mut avisos,
            ),
            drop_inactive: leitor.boolean("tunnels.drop_inactive", false),
        };
        Ok((config, avisos))
    }

    /// Troca o `pki` pelos textos PEM guardados em outro lugar (o cofre do app).
    pub fn set_pki(&mut self, ca: &str, cert: &str, key: &str) {
        self.pki.ca = Some(ca.to_owned());
        self.pki.cert = Some(cert.to_owned());
        self.pki.key = Some(Zeroizing::new(key.to_owned()));
    }

    /// Lê o `pki` (CAs, certificados e chave) e confere que formam um conjunto: a chave é a de
    /// todos os certificados, e a versão de início existe.
    pub fn credentials(&self) -> Result<Credentials, ConfigError> {
        let ca = self
            .pki
            .ca
            .as_deref()
            .ok_or_else(|| ConfigError::new("falta a CA (pki.ca)"))?;
        let cert = self
            .pki
            .cert
            .as_deref()
            .ok_or_else(|| ConfigError::new("falta o certificado do nó (pki.cert)"))?;
        let key = self
            .pki
            .key
            .as_deref()
            .ok_or_else(|| ConfigError::new("falta a chave privada do nó (pki.key)"))?;

        let mut pool = CaPool::from_pem(ca)?;
        for impressao in &self.pki.blocklist {
            pool.block(impressao);
        }
        let (mut v1, mut v2) = (None, None);
        for c in Certificate::from_pem(cert)? {
            let vaga = match c.version() {
                Version::V1 => &mut v1,
                Version::V2 => &mut v2,
            };
            if vaga.replace(c).is_some() {
                return Err(ConfigError::new(
                    "o pki.cert tem dois certificados da mesma versão",
                ));
            }
        }
        let key = PrivateKey::from_pem(key)?;
        for c in v1.iter().chain(v2.iter()) {
            if c.curve() == Curve::P256 || key.curve() == Curve::P256 {
                return Err(crate::cert::unsupported_p256().into());
            }
            key.matches(c).map_err(|_| {
                ConfigError::new(format!(
                    "a chave privada não corresponde ao certificado {}",
                    c.name()
                ))
            })?;
        }
        let padrao = if v1.is_some() { 1 } else { 2 };
        let initiating = match self.pki.initiating_version.unwrap_or(padrao) {
            1 if v1.is_none() => {
                return Err(ConfigError::new(
                    "pki.initiating_version 1 exige um certificado v1 no pki.cert",
                ));
            }
            1 => Version::V1,
            2 if v2.is_none() => {
                return Err(ConfigError::new(
                    "pki.initiating_version 2 exige um certificado v2 no pki.cert",
                ));
            }
            _ => Version::V2,
        };
        Ok(Credentials {
            pool,
            v1,
            v2,
            key,
            initiating,
            cipher: self.cipher,
        })
    }

    /// Confere o config inteiro antes de salvar ou subir: o `pki` forma um conjunto, a CA
    /// assinou o certificado e ele não venceu (ou vence em menos de 30 dias), há um lighthouse,
    /// a curva e a rede são suportadas. `agora` são segundos desde 1970.
    pub fn check(&self, agora: i64) -> Vec<Problem> {
        let mut problemas = Vec::new();
        let mut erro = |mensagem: String| {
            problemas.push(Problem {
                severity: Severity::Error,
                message: mensagem,
            });
        };
        match self.credentials() {
            Err(e) => erro(e.to_string()),
            Ok(cred) => {
                for c in cred.v1.iter().chain(cred.v2.iter()) {
                    if let Err(e) = cred.pool.verify(c, agora) {
                        erro(e.to_string());
                    }
                }
                if cred.address().is_none() {
                    erro("rede só IPv6 ainda não é suportada".to_owned());
                }
            }
        }
        if self.lighthouse.hosts.is_empty() && self.static_host_map.is_empty() {
            erro(
                "o config não tem lighthouse nem static_host_map: não há como achar os outros nós"
                    .to_owned(),
            );
        }
        for host in &self.lighthouse.hosts {
            if !self.static_host_map.contains_key(host) {
                erro(format!(
                    "o lighthouse {host} não está no static_host_map, então não há endereço para falar com ele"
                ));
            }
        }
        if let Ok(cred) = self.credentials() {
            const TRINTA_DIAS: i64 = 30 * 24 * 3600;
            for c in cred.v1.iter().chain(cred.v2.iter()) {
                if c.not_after() >= agora && c.not_after() - agora < TRINTA_DIAS {
                    problemas.push(Problem {
                        severity: Severity::Warning,
                        message: format!(
                            "o certificado {} vence em {}",
                            c.name(),
                            format_date(c.not_after())
                        ),
                    });
                }
            }
        }
        problemas
    }
}

/// O pki conferido: o que o motor precisa para falar com os pares.
#[derive(Debug, Clone)]
pub struct Credentials {
    /// As CAs e a lista de bloqueio.
    pub pool: CaPool,
    /// O certificado v1 do nó, se há.
    pub v1: Option<Certificate>,
    /// O certificado v2 do nó, se há.
    pub v2: Option<Certificate>,
    /// A chave privada.
    pub key: PrivateKey,
    /// A versão com que o nó inicia os handshakes.
    pub initiating: Version,
    /// A cifra da rede.
    pub cipher: Cipher,
}

impl Credentials {
    /// O certificado de uma versão.
    pub fn cert(&self, versao: Version) -> Option<&Certificate> {
        match versao {
            Version::V1 => self.v1.as_ref(),
            Version::V2 => self.v2.as_ref(),
        }
    }

    /// O certificado com que o nó inicia os handshakes.
    pub fn default_cert(&self) -> &Certificate {
        self.cert(self.initiating)
            .or(self.v1.as_ref())
            .or(self.v2.as_ref())
            .unwrap_or_else(|| unreachable!("o pki tem pelo menos um certificado"))
    }

    /// O endereço IPv4 do nó na rede, com o tamanho da rede. `None` se o certificado só tem
    /// endereços IPv6.
    pub fn address(&self) -> Option<Cidr> {
        self.default_cert()
            .networks()
            .iter()
            .find(|rede| rede.addr().is_ipv4())
            .copied()
    }
}

/// Um texto PEM inline vira ele mesmo; um caminho é lido da `pasta`. `None` se é um caminho e não
/// há pasta.
fn pem_ou_arquivo(
    chave: &str,
    valor: &str,
    pasta: Option<&Path>,
) -> Result<Option<String>, ConfigError> {
    if valor.contains("-----BEGIN") {
        return Ok(Some(valor.to_owned()));
    }
    let Some(pasta) = pasta else {
        return Ok(None);
    };
    let caminho = pasta.join(valor.trim());
    std::fs::read_to_string(&caminho).map(Some).map_err(|erro| {
        ConfigError::new(format!(
            "{chave}: não foi possível ler {}: {erro}",
            caminho.display()
        ))
    })
}

fn escalar(valor: &Yaml) -> Option<String> {
    match valor {
        Yaml::String(texto) | Yaml::Real(texto) => Some(texto.clone()),
        Yaml::Integer(n) => Some(n.to_string()),
        Yaml::Boolean(b) => Some(b.to_string()),
        _ => None,
    }
}

fn unsafe_routes(no: &Yaml) -> Result<Vec<UnsafeRoute>, ConfigError> {
    let Yaml::Array(itens) = no else {
        return Ok(Vec::new());
    };
    let mut rotas = Vec::new();
    for (indice, item) in itens.iter().enumerate() {
        let posicao = indice + 1;
        let route: Cidr = escalar(&item["route"])
            .ok_or_else(|| {
                ConfigError::new(format!("tun.unsafe_routes: a rota {posicao} não tem route"))
            })?
            .parse()
            .map_err(|erro| {
                ConfigError::new(format!(
                    "tun.unsafe_routes: a rota {posicao} é inválida: {erro}"
                ))
            })?;
        let via_texto = |texto: &str| -> Result<IpAddr, ConfigError> {
            texto.parse().map_err(|_| {
                ConfigError::new(format!(
                    "tun.unsafe_routes: o via {texto} da rota {posicao} não é um endereço Nebula (IP)"
                ))
            })
        };
        let via = match &item["via"] {
            Yaml::Array(gateways) => gateways
                .iter()
                .filter_map(|gateway| escalar(&gateway["gateway"]))
                .map(|texto| via_texto(&texto))
                .collect::<Result<Vec<_>, _>>()?,
            outro => match escalar(outro) {
                Some(texto) => vec![via_texto(&texto)?],
                None => {
                    return Err(ConfigError::new(format!(
                        "tun.unsafe_routes: a rota {posicao} não tem via"
                    )));
                }
            },
        };
        rotas.push(UnsafeRoute { route, via });
    }
    Ok(rotas)
}

/// `true` se o `firewall.outbound` limita algo (diferente de uma regra que deixa tudo passar).
fn saida_restrita(no: &Yaml) -> bool {
    let Yaml::Array(regras) = no else {
        return false;
    };
    let qualquer =
        |regra: &Yaml, chave: &str| escalar(&regra[chave]).is_none_or(|valor| valor == "any");
    !regras.iter().any(|regra| {
        qualquer(regra, "port")
            && qualquer(regra, "proto")
            && qualquer(regra, "host")
            && regra["groups"].is_badvalue()
            && regra["group"].is_badvalue()
            && regra["cidr"].is_badvalue()
            && regra["ca_name"].is_badvalue()
            && regra["ca_sha"].is_badvalue()
    })
}

/// Lê as chaves do YAML por caminho (`pki.ca`), com a leniência do Nebula: um valor que não
/// serve volta ao padrão.
struct Leitor<'a> {
    raiz: &'a Yaml,
}

impl Leitor<'_> {
    fn no(&self, caminho: &str) -> &Yaml {
        let mut atual = self.raiz;
        for parte in caminho.split('.') {
            atual = &atual[parte];
        }
        atual
    }

    fn texto(&self, caminho: &str) -> Option<String> {
        escalar(self.no(caminho))
    }

    fn numero(&self, caminho: &str) -> Option<u64> {
        self.texto(caminho)?.trim().parse().ok()
    }

    fn boolean(&self, caminho: &str, padrao: bool) -> bool {
        match self.texto(caminho).map(|t| t.to_lowercase()).as_deref() {
            Some("true" | "1" | "t" | "yes" | "y") => true,
            Some("false" | "0" | "f" | "no" | "n") => false,
            _ => padrao,
        }
    }

    fn lista(&self, caminho: &str) -> Vec<String> {
        match self.no(caminho) {
            Yaml::Array(itens) => itens.iter().filter_map(escalar).collect(),
            _ => Vec::new(),
        }
    }

    fn duracao(&self, caminho: &str, padrao: Duration, avisos: &mut Vec<Warning>) -> Duration {
        let Some(texto) = self.texto(caminho) else {
            return padrao;
        };
        parse_duration(&texto).unwrap_or_else(|| {
            avisos.push(Warning {
                key: caminho.to_owned(),
                message: format!(
                    "a duração {texto} não é válida (use 1s, 500ms, 5m); vale o padrão"
                ),
            });
            padrao
        })
    }
}

/// Lê uma duração no formato do Go (`1s`, `500ms`, `1h30m`, `1.5s`). `None` se não é válida.
pub fn parse_duration(texto: &str) -> Option<Duration> {
    let texto = texto.trim();
    if texto == "0" {
        return Some(Duration::ZERO);
    }
    if texto.is_empty() {
        return None;
    }
    let mut resto = texto;
    let mut total = 0f64;
    while !resto.is_empty() {
        let fim_numero = resto
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(resto.len());
        if fim_numero == 0 {
            return None;
        }
        let numero: f64 = resto[..fim_numero].parse().ok()?;
        resto = &resto[fim_numero..];
        let fim_unidade = resto
            .find(|c: char| c.is_ascii_digit() || c == '.')
            .unwrap_or(resto.len());
        let segundos = match &resto[..fim_unidade] {
            "ns" => 1e-9,
            "us" | "µs" | "μs" => 1e-6,
            "ms" => 1e-3,
            "s" => 1.0,
            "m" => 60.0,
            "h" => 3600.0,
            _ => return None,
        };
        total += numero * segundos;
        resto = &resto[fim_unidade..];
    }
    Duration::try_from_secs_f64(total).ok()
}
