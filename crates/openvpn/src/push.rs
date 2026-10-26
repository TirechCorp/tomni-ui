//! O `PUSH_REPLY`: a configuração que o servidor empurra para o cliente.
//!
//! Depois do TLS e da troca de chaves, o cliente manda `PUSH_REQUEST` e o servidor responde
//! `PUSH_REPLY,opção,opção,...` com o que o cliente deve usar: o endereço dentro da VPN, as
//! rotas, o DNS, a cifra, os tempos do ping... Uma resposta grande é partida em várias
//! mensagens; cada uma termina em `push-continuation 2` (vem mais) e a última em
//! `push-continuation 1`.
//!
//! O `.ovpn` tem a última palavra: `pull-filter` descarta ou recusa linhas pelo prefixo, na
//! ordem em que foram escritos (a primeira que combina decide), e `route-nopull` descarta o que
//! mexe em rotas e DNS. Aqui não há tabela de rotas do sistema: as rotas só dizem à pilha do app
//! o que mandar pelo túnel.
//!
//! O que o app ainda não usa (IPv6 dentro da VPN, `register-dns`...) é ignorado com um aviso
//! ([`PushWarning`]); nada disso derruba a conexão.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use base64::Engine as _;
use zeroize::Zeroizing;

use crate::config::{Compress, Config, FilterAction, PullFilter};

/// A porta de DNS quando o servidor não diz outra.
const DNS_PORT: u16 = 53;

/// O maior `peer-id` (24 bits, formato do protocolo). O valor `0xFFFFFF` quer dizer "sem id".
const MAX_PEER_ID: u32 = 0x00FF_FFFE;

/// Como o servidor numera os endereços dos clientes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Topology {
    /// Um endereço para o cliente e a máscara da rede (o `ifconfig` traz `<ip> <máscara>`).
    Subnet,
    /// Blocos de 4 endereços por cliente (o `ifconfig` traz `<ip> <outra ponta>`). É o padrão
    /// do OpenVPN quando o servidor não diz.
    Net30,
    /// Ponto a ponto (o `ifconfig` traz `<ip> <outra ponta>`).
    P2p,
}

/// O endereço que o servidor deu ao cliente.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IfConfig {
    /// O endereço do cliente dentro da VPN.
    pub address: Ipv4Addr,
    /// O tamanho do prefixo: o da máscara em `subnet`, 32 em `net30` e `p2p`.
    pub prefix_len: u8,
    /// A outra ponta (só em `net30` e `p2p`).
    pub peer: Option<Ipv4Addr>,
}

/// Uma rota: o que vai pelo túnel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpRoute {
    /// O endereço da rede.
    pub network: Ipv4Addr,
    /// O tamanho do prefixo.
    pub prefix_len: u8,
}

/// Um aviso sobre uma opção empurrada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushWarning {
    /// A opção, como o servidor a mandou.
    pub option: String,
    /// O que o app fez com ela.
    pub message: String,
}

/// O que dá errado ao ler o `PUSH_REPLY`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushError {
    /// A mensagem não começa com `PUSH_REPLY`.
    NotPushReply,
    /// Um `pull-filter reject` recusou esta linha.
    Rejected(String),
    /// Uma opção está malformada.
    Malformed {
        /// A opção.
        option: String,
        /// O que há de errado.
        message: String,
    },
}

impl fmt::Display for PushError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotPushReply => f.write_str("a mensagem do servidor não é um PUSH_REPLY"),
            Self::Rejected(l) => write!(
                f,
                "o servidor empurrou \"{l}\", que um pull-filter reject do perfil recusa"
            ),
            Self::Malformed { option, message } => {
                write!(f, "a opção empurrada \"{option}\" é inválida: {message}")
            }
        }
    }
}

impl std::error::Error for PushError {}

fn malformed(option: &str, message: &str) -> PushError {
    PushError::Malformed {
        option: option.to_owned(),
        message: message.to_owned(),
    }
}

/// O resultado: tudo o que o servidor empurrou, já interpretado.
#[derive(Clone, Default)]
pub struct Pushed {
    /// O endereço do cliente. Sem ele a VPN não tem como funcionar.
    pub ifconfig: Option<IfConfig>,
    /// O `topology` empurrado.
    pub topology: Option<Topology>,
    /// O `route-gateway`.
    pub route_gateway: Option<Ipv4Addr>,
    /// As rotas empurradas, em ordem.
    pub routes: Vec<IpRoute>,
    /// Se o servidor mandou `redirect-gateway` (todo o tráfego pelo túnel).
    pub redirect_gateway: bool,
    /// Os servidores de DNS, em ordem de prioridade.
    pub dns_servers: Vec<SocketAddr>,
    /// Os domínios de busca do DNS.
    pub search_domains: Vec<String>,
    /// O `peer-id` do `P_DATA_V2`.
    pub peer_id: Option<u32>,
    /// A cifra do canal de dados.
    pub cipher: Option<String>,
    /// O digest do HMAC do canal de dados.
    pub auth: Option<String>,
    /// Se as chaves do canal de dados saem do exportador do TLS (`key-derivation tls-ekm`).
    pub key_derivation_ekm: bool,
    /// Os `protocol-flags` que o servidor empurrou.
    pub protocol_flags: Vec<String>,
    /// O `ping`, em segundos.
    pub ping: Option<u32>,
    /// O `ping-restart`, em segundos.
    pub ping_restart: Option<u32>,
    /// O `ping-exit`, em segundos.
    pub ping_exit: Option<u32>,
    /// O `auth-token`: a senha de uso único para as renegociações.
    pub auth_token: Option<Zeroizing<String>>,
    /// O usuário que vai com o `auth-token`.
    pub auth_token_user: Option<String>,
    /// O `tun-mtu`.
    pub tun_mtu: Option<u32>,
    /// O `explicit-exit-notify` (quantas vezes avisar a saída).
    pub explicit_exit_notify: Option<u32>,
    /// O enquadramento de compressão (`compress`, `comp-lzo`).
    pub compress: Option<Compress>,
    /// Os textos do `echo`, para o registro.
    pub echo: Vec<String>,
    /// O que foi ignorado, e por quê.
    pub warnings: Vec<PushWarning>,
}

impl fmt::Debug for Pushed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pushed")
            .field("ifconfig", &self.ifconfig)
            .field("topology", &self.topology)
            .field("routes", &self.routes)
            .field("redirect_gateway", &self.redirect_gateway)
            .field("dns_servers", &self.dns_servers)
            .field("search_domains", &self.search_domains)
            .field("peer_id", &self.peer_id)
            .field("cipher", &self.cipher)
            .field("auth", &self.auth)
            .field("key_derivation_ekm", &self.key_derivation_ekm)
            .field("ping", &self.ping)
            .field("ping_restart", &self.ping_restart)
            .field("auth_token", &self.auth_token.as_ref().map(|_| "(oculto)"))
            .field("tun_mtu", &self.tun_mtu)
            .finish_non_exhaustive()
    }
}

impl Pushed {
    /// As rotas empurradas, mais a rota padrão (`0.0.0.0/0`) quando há `redirect-gateway`. O
    /// `def1` do OpenVPN divide a rota padrão em duas metades só para não trocar a rota do
    /// sistema; aqui não há rota do sistema, então a rota inteira basta.
    pub fn effective_routes(&self) -> Vec<IpRoute> {
        let mut rotas = self.routes.clone();
        if self.redirect_gateway {
            rotas.push(IpRoute {
                network: Ipv4Addr::UNSPECIFIED,
                prefix_len: 0,
            });
        }
        rotas
    }
}

/// Junta as mensagens `PUSH_REPLY` e as interpreta quando a última chega.
#[derive(Debug, Clone)]
pub struct PushReader {
    filters: Vec<PullFilter>,
    route_nopull: bool,
    /// As linhas aceitas até agora.
    lines: Vec<String>,
    /// O que os filtros descartaram.
    filtered: Vec<PushWarning>,
}

impl PushReader {
    /// Um leitor com os `pull-filter` e o `route-nopull` do perfil.
    pub fn new(filters: &[PullFilter], route_nopull: bool) -> PushReader {
        PushReader {
            filters: filters.to_vec(),
            route_nopull,
            lines: Vec::new(),
            filtered: Vec::new(),
        }
    }

    /// Um leitor com os filtros da configuração.
    pub fn from_config(config: &Config) -> PushReader {
        PushReader::new(&config.pull_filters, config.route_nopull)
    }

    /// Lê uma mensagem `PUSH_REPLY`. Devolve `Some` quando a resposta está completa (a mensagem
    /// sem `push-continuation 2`) e `None` quando vêm mais partes.
    pub fn feed(&mut self, message: &str) -> Result<Option<Pushed>, PushError> {
        let message = message.trim_end_matches('\0');
        let resto = message
            .strip_prefix("PUSH_REPLY")
            .ok_or(PushError::NotPushReply)?;
        let mut mais_vem = false;
        for opcao in resto.split(',').skip(1) {
            let opcao = opcao.trim();
            if opcao.is_empty() {
                continue;
            }
            if let Some(n) = opcao.strip_prefix("push-continuation") {
                mais_vem = n.trim() == "2";
                continue;
            }
            match self.filter(opcao) {
                Filtered::Keep => {}
                Filtered::Drop => continue,
                Filtered::Reject => return Err(PushError::Rejected(opcao.to_owned())),
            }
            if self.route_nopull && is_route_option(opcao) {
                self.filtered.push(PushWarning {
                    option: opcao.to_owned(),
                    message: "descartada pelo route-nopull do perfil".into(),
                });
                continue;
            }
            self.lines.push(opcao.to_owned());
        }
        if mais_vem {
            return Ok(None);
        }
        let mut pushed = interpret(&self.lines)?;
        let mut avisos = std::mem::take(&mut self.filtered);
        avisos.append(&mut pushed.warnings);
        pushed.warnings = avisos;
        self.lines.clear();
        Ok(Some(pushed))
    }

    fn filter(&mut self, linha: &str) -> Filtered {
        for f in &self.filters {
            if linha.starts_with(f.prefix.as_str()) {
                return match f.action {
                    FilterAction::Accept => Filtered::Keep,
                    FilterAction::Ignore => {
                        self.filtered.push(PushWarning {
                            option: linha.to_owned(),
                            message: format!("descartada pelo pull-filter ignore \"{}\"", f.prefix),
                        });
                        Filtered::Drop
                    }
                    FilterAction::Reject => Filtered::Reject,
                };
            }
        }
        Filtered::Keep
    }
}

enum Filtered {
    Keep,
    Drop,
    Reject,
}

/// Se a opção mexe em rotas ou DNS, que é o que o `route-nopull` barra.
fn is_route_option(linha: &str) -> bool {
    let nome = linha.split_whitespace().next().unwrap_or("");
    matches!(
        nome,
        "route"
            | "route-ipv6"
            | "redirect-gateway"
            | "redirect-private"
            | "dhcp-option"
            | "dns"
            | "block-outside-dns"
    )
}

/// Lê uma mensagem `PUSH_REPLY` completa, sem continuação.
pub fn parse_push_reply(
    message: &str,
    filters: &[PullFilter],
    route_nopull: bool,
) -> Result<Pushed, PushError> {
    let mut leitor = PushReader::new(filters, route_nopull);
    leitor
        .feed(message)?
        .ok_or_else(|| malformed("push-continuation", "a resposta não terminou"))
}

/// Separa uma linha em palavras. Aspas duplas agrupam; não há escapes.
pub fn split_args(linha: &str) -> Vec<String> {
    let mut palavras = Vec::new();
    let mut atual = String::new();
    let mut tem = false;
    let mut aspas = false;
    for c in linha.chars() {
        match c {
            '"' => {
                aspas = !aspas;
                tem = true;
            }
            c if c.is_whitespace() && !aspas => {
                if tem {
                    palavras.push(std::mem::take(&mut atual));
                    tem = false;
                }
            }
            c => {
                atual.push(c);
                tem = true;
            }
        }
    }
    if tem {
        palavras.push(atual);
    }
    palavras
}

/// O tamanho do prefixo de uma máscara de rede. `None` se os bits não são contíguos.
pub fn prefix_len(mask: Ipv4Addr) -> Option<u8> {
    let bits = u32::from(mask);
    let uns = bits.leading_ones();
    let esperado = if uns == 0 { 0 } else { u32::MAX << (32 - uns) };
    (bits == esperado).then_some(uns as u8)
}

/// Lê os argumentos de um `route` (`<rede> [máscara] [gateway] [métrica]`), do `.ovpn` ou do
/// servidor. `Ok(None)` quando a rota não se aplica (nome de máquina, `net_gateway`...); o
/// motivo vai em `Err(Some(texto))` apenas quando a opção está errada.
pub fn parse_route(args: &[String]) -> Result<Option<IpRoute>, String> {
    let rede = args.first().ok_or("falta a rede")?;
    let network = match rede.as_str() {
        "default" => Ipv4Addr::UNSPECIFIED,
        "vpn_gateway" | "net_gateway" | "remote_host" => return Ok(None),
        outro => match outro.parse::<Ipv4Addr>() {
            Ok(ip) => ip,
            // Pode ser um nome ou um IPv6; o motor não resolve nomes na rota.
            Err(_) => return Ok(None),
        },
    };
    let prefix_len = match args.get(1).map(String::as_str) {
        None | Some("default") => 32,
        Some(m) => {
            let mascara: Ipv4Addr = m
                .parse()
                .map_err(|_| format!("a máscara \"{m}\" não é um endereço"))?;
            prefix_len(mascara).ok_or_else(|| format!("a máscara {m} não é contínua"))?
        }
    };
    Ok(Some(IpRoute {
        network,
        prefix_len,
    }))
}

fn number(opcao: &str, valor: Option<&String>) -> Result<u32, PushError> {
    valor
        .ok_or_else(|| malformed(opcao, "falta o valor"))?
        .parse::<u32>()
        .map_err(|_| malformed(opcao, "o valor não é um número"))
}

fn address_with_port(texto: &str) -> Option<SocketAddr> {
    if let Ok(a) = texto.parse::<SocketAddr>() {
        return Some(a);
    }
    texto
        .parse::<IpAddr>()
        .ok()
        .map(|ip| SocketAddr::new(ip, DNS_PORT))
}

/// Um servidor da opção `dns server <n> ...`.
#[derive(Default)]
struct DnsServerOption {
    priority: i64,
    addresses: Vec<SocketAddr>,
    plain: bool,
}

fn interpret(lines: &[String]) -> Result<Pushed, PushError> {
    let mut p = Pushed::default();
    let mut ifconfig_bruto: Option<(Ipv4Addr, Ipv4Addr)> = None;
    let mut dhcp_dns: Vec<SocketAddr> = Vec::new();
    let mut dhcp_domains: Vec<String> = Vec::new();
    let mut dns_servers: Vec<DnsServerOption> = Vec::new();
    let mut dns_search: Vec<String> = Vec::new();

    let aviso = |p: &mut Pushed, opcao: &str, msg: &str| {
        p.warnings.push(PushWarning {
            option: opcao.to_owned(),
            message: msg.to_owned(),
        });
    };

    for linha in lines {
        let args = split_args(linha);
        let Some(nome) = args.first().map(String::as_str) else {
            continue;
        };
        let resto = &args[1..];
        match nome {
            "ifconfig" => {
                let a = resto
                    .first()
                    .ok_or_else(|| malformed(linha, "falta o endereço"))?;
                let b = resto
                    .get(1)
                    .ok_or_else(|| malformed(linha, "falta a máscara ou a outra ponta"))?;
                let a = a
                    .parse()
                    .map_err(|_| malformed(linha, "o endereço é inválido"))?;
                let b = b
                    .parse()
                    .map_err(|_| malformed(linha, "a máscara ou a outra ponta é inválida"))?;
                ifconfig_bruto = Some((a, b));
            }
            "topology" => {
                p.topology = Some(match resto.first().map(String::as_str) {
                    Some("subnet") => Topology::Subnet,
                    Some("net30") => Topology::Net30,
                    Some("p2p") => Topology::P2p,
                    _ => return Err(malformed(linha, "topologia desconhecida")),
                });
            }
            "route-gateway" => {
                if let Some(Ok(ip)) = resto.first().map(|s| s.parse::<Ipv4Addr>()) {
                    p.route_gateway = Some(ip);
                }
            }
            "route" => match parse_route(resto) {
                Ok(Some(r)) => p.routes.push(r),
                Ok(None) => aviso(
                    &mut p,
                    linha,
                    "rota que o app não usa (nome, gateway ou IPv6); ignorada",
                ),
                Err(e) => return Err(malformed(linha, &e)),
            },
            "redirect-gateway" => {
                if resto.iter().any(|f| f == "ipv6" || f == "!ipv4") {
                    aviso(
                        &mut p,
                        linha,
                        "o redirecionamento do IPv6 ainda não é suportado",
                    );
                }
                if !resto.iter().any(|f| f == "!ipv4") {
                    p.redirect_gateway = true;
                }
            }
            "redirect-private" => {}
            "dhcp-option" => match resto.first().map(String::as_str) {
                Some("DNS") | Some("DNS6") => match resto.get(1).and_then(|s| address_with_port(s))
                {
                    Some(a) => dhcp_dns.push(a),
                    None => return Err(malformed(linha, "o endereço do DNS é inválido")),
                },
                Some("DOMAIN") | Some("DOMAIN-SEARCH") | Some("ADAPTER_DOMAIN_SUFFIX") => {
                    if let Some(d) = resto.get(1) {
                        dhcp_domains.push(d.clone());
                    }
                }
                // WINS, NBT, NTP, PROXY...: nada a fazer sem uma interface do sistema.
                _ => {}
            },
            "dns" => match resto.first().map(String::as_str) {
                Some("server") => {
                    let n: i64 = resto
                        .get(1)
                        .and_then(|s| s.parse().ok())
                        .ok_or_else(|| malformed(linha, "falta o número do servidor"))?;
                    let indice = match dns_servers.iter().position(|s| s.priority == n) {
                        Some(i) => i,
                        None => {
                            dns_servers.push(DnsServerOption {
                                priority: n,
                                plain: true,
                                ..Default::default()
                            });
                            dns_servers.len() - 1
                        }
                    };
                    let servidor = &mut dns_servers[indice];
                    match resto.get(2).map(String::as_str) {
                        Some("address") => {
                            for a in &resto[3..] {
                                match address_with_port(a) {
                                    Some(a) => servidor.addresses.push(a),
                                    None => {
                                        return Err(malformed(
                                            linha,
                                            "o endereço do DNS é inválido",
                                        ));
                                    }
                                }
                            }
                        }
                        Some("transport") => {
                            servidor.plain =
                                matches!(resto.get(3).map(String::as_str), Some("plain"));
                        }
                        // Domínios de resolução, DNSSEC e SNI: o app usa o servidor para tudo.
                        _ => {}
                    }
                }
                Some("search-domains") => dns_search.extend(resto[1..].iter().cloned()),
                _ => aviso(&mut p, linha, "opção dns desconhecida; ignorada"),
            },
            "peer-id" => {
                let n = number(linha, resto.first())?;
                if n > MAX_PEER_ID {
                    return Err(malformed(linha, "o peer-id passa de 24 bits"));
                }
                p.peer_id = Some(n);
            }
            "cipher" => p.cipher = resto.first().cloned(),
            "auth" => p.auth = resto.first().cloned(),
            "key-derivation" => match resto.first().map(String::as_str) {
                Some("tls-ekm") => p.key_derivation_ekm = true,
                _ => return Err(malformed(linha, "derivação de chaves desconhecida")),
            },
            "protocol-flags" => {
                for flag in resto {
                    match flag.as_str() {
                        "cc-exit"
                        | "tls-ekm"
                        | "dyn-tls-crypt"
                        | "force-tls-key-material-export" => {}
                        "aead-epoch" => aviso(
                            &mut p,
                            linha,
                            "o formato de dados com época não é suportado",
                        ),
                        _ => aviso(&mut p, linha, "flag desconhecida; ignorada"),
                    }
                    if flag == "tls-ekm" || flag == "force-tls-key-material-export" {
                        p.key_derivation_ekm = true;
                    }
                    p.protocol_flags.push(flag.clone());
                }
            }
            "ping" => p.ping = Some(number(linha, resto.first())?),
            "ping-restart" => p.ping_restart = Some(number(linha, resto.first())?),
            "ping-exit" => p.ping_exit = Some(number(linha, resto.first())?),
            "auth-token" => {
                p.auth_token = resto.first().map(|t| Zeroizing::new(t.clone()));
            }
            "auth-token-user" => {
                let bruto = resto
                    .first()
                    .ok_or_else(|| malformed("auth-token-user", "falta o valor"))?;
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(bruto)
                    .map_err(|_| malformed("auth-token-user", "não é base64"))?;
                let usuario = String::from_utf8(bytes)
                    .map_err(|_| malformed("auth-token-user", "o usuário não é UTF-8"))?;
                p.auth_token_user = Some(usuario);
            }
            "tun-mtu" => p.tun_mtu = Some(number(linha, resto.first())?),
            "explicit-exit-notify" => {
                p.explicit_exit_notify = Some(match resto.first() {
                    Some(n) => n
                        .parse()
                        .map_err(|_| malformed(linha, "o número é inválido"))?,
                    None => 1,
                });
            }
            "comp-lzo" => {
                p.compress = Some(match resto.first().map(String::as_str) {
                    Some("no") => Compress::LzoOff,
                    _ => Compress::Lzo,
                });
            }
            "compress" => {
                p.compress = Some(match resto.first().map(String::as_str) {
                    None => Compress::Stub,
                    Some("stub") => Compress::Stub,
                    Some("stub-v2") => Compress::StubV2,
                    Some("lzo") => Compress::Lzo,
                    Some("lz4") => Compress::Lz4,
                    Some("lz4-v2") => Compress::Lz4V2,
                    Some(_) => {
                        return Err(malformed(linha, "algoritmo de compressão desconhecido"));
                    }
                });
            }
            "ifconfig-ipv6" | "route-ipv6" => {
                aviso(
                    &mut p,
                    linha,
                    "IPv6 dentro da VPN ainda não é suportado; ignorada",
                );
            }
            "echo" => p.echo.push(resto.join(" ")),
            "register-dns" | "block-outside-dns" | "setenv" | "setenv-safe" | "ifconfig-nowarn"
            | "route-delay" | "route-method" | "ip-win32" | "socket-flags" | "block-ipv6"
            | "sndbuf" | "rcvbuf" | "mssfix" | "tun-ipv6" | "persist-key" | "persist-tun"
            | "aead-epoch" | "dco" => {}
            _ => aviso(&mut p, linha, "opção empurrada desconhecida; ignorada"),
        }
    }

    if let Some((a, b)) = ifconfig_bruto {
        // Sem `topology`, o servidor usa a topologia padrão do OpenVPN: net30.
        let topologia = p.topology.unwrap_or(Topology::Net30);
        p.ifconfig = Some(match topologia {
            Topology::Subnet => IfConfig {
                address: a,
                prefix_len: prefix_len(b)
                    .ok_or_else(|| malformed("ifconfig", "a máscara não é contínua"))?,
                peer: None,
            },
            Topology::Net30 | Topology::P2p => IfConfig {
                address: a,
                prefix_len: 32,
                peer: Some(b),
            },
        });
    }

    // A opção `dns` tem prioridade sobre o `dhcp-option` e ordena os servidores pelo número.
    dns_servers.retain(|s| {
        if !s.plain {
            p.warnings.push(PushWarning {
                option: format!("dns server {}", s.priority),
                message: "o servidor usa DNS cifrado (DoH/DoT), que o app ainda não fala; ignorado"
                    .into(),
            });
        }
        s.plain
    });
    dns_servers.sort_by_key(|s| s.priority);
    if dns_servers.is_empty() {
        p.dns_servers = dhcp_dns;
    } else {
        p.dns_servers = dns_servers.into_iter().flat_map(|s| s.addresses).collect();
    }
    let dominios = if dns_search.is_empty() {
        dhcp_domains
    } else {
        dns_search
    };
    for d in dominios {
        if !p.search_domains.contains(&d) {
            p.search_domains.push(d);
        }
    }
    Ok(p)
}

#[cfg(test)]
mod testes {
    use super::*;

    fn ip(s: &str) -> Ipv4Addr {
        s.parse().unwrap()
    }

    fn filtro(action: FilterAction, prefix: &str) -> PullFilter {
        PullFilter {
            action,
            prefix: prefix.into(),
        }
    }

    fn ler(msg: &str) -> Pushed {
        parse_push_reply(msg, &[], false).unwrap()
    }

    #[test]
    fn resposta_tipica_do_servidor_subnet() {
        let p = ler(
            "PUSH_REPLY,route 10.8.0.0 255.255.255.0,route 192.168.10.0 255.255.255.0,dhcp-option DNS 10.8.0.1,\
             dhcp-option DOMAIN corp.exemplo,topology subnet,ping 10,ping-restart 120,\
             ifconfig 10.8.0.6 255.255.255.0,peer-id 3,cipher AES-256-GCM,key-derivation tls-ekm",
        );
        assert_eq!(
            p.ifconfig,
            Some(IfConfig {
                address: ip("10.8.0.6"),
                prefix_len: 24,
                peer: None
            })
        );
        assert_eq!(p.topology, Some(Topology::Subnet));
        assert_eq!(
            p.routes,
            vec![
                IpRoute {
                    network: ip("10.8.0.0"),
                    prefix_len: 24
                },
                IpRoute {
                    network: ip("192.168.10.0"),
                    prefix_len: 24
                },
            ]
        );
        assert_eq!(p.dns_servers, vec!["10.8.0.1:53".parse().unwrap()]);
        assert_eq!(p.search_domains, vec!["corp.exemplo"]);
        assert_eq!(
            (p.ping, p.ping_restart, p.peer_id),
            (Some(10), Some(120), Some(3))
        );
        assert_eq!(p.cipher.as_deref(), Some("AES-256-GCM"));
        assert!(p.key_derivation_ekm);
        assert!(p.warnings.is_empty(), "{:?}", p.warnings);
    }

    #[test]
    fn net30_e_p2p_dao_prefixo_32_e_a_outra_ponta() {
        for topo in ["topology net30,", "topology p2p,", ""] {
            let p = ler(&format!("PUSH_REPLY,{topo}ifconfig 10.8.0.6 10.8.0.5"));
            assert_eq!(
                p.ifconfig,
                Some(IfConfig {
                    address: ip("10.8.0.6"),
                    prefix_len: 32,
                    peer: Some(ip("10.8.0.5"))
                }),
                "{topo}"
            );
        }
    }

    #[test]
    fn a_ordem_do_topology_e_do_ifconfig_nao_importa() {
        let a = ler("PUSH_REPLY,ifconfig 10.8.0.6 255.255.255.0,topology subnet");
        let b = ler("PUSH_REPLY,topology subnet,ifconfig 10.8.0.6 255.255.255.0");
        assert_eq!(a.ifconfig, b.ifconfig);
        assert_eq!(a.ifconfig.unwrap().prefix_len, 24);
    }

    #[test]
    fn mascara_que_nao_e_continua_e_erro() {
        let e = parse_push_reply(
            "PUSH_REPLY,topology subnet,ifconfig 10.0.0.2 255.0.255.0",
            &[],
            false,
        );
        assert!(matches!(e, Err(PushError::Malformed { .. })));
    }

    #[test]
    fn route_com_e_sem_mascara_e_com_gateway_e_metrica() {
        let p = ler(
            "PUSH_REPLY,route 10.1.2.3,route 172.16.0.0 255.240.0.0,route 10.9.0.0 255.255.0.0 10.8.0.1 100,\
             route default,route 0.0.0.0 0.0.0.0",
        );
        assert_eq!(
            p.routes,
            vec![
                IpRoute {
                    network: ip("10.1.2.3"),
                    prefix_len: 32
                },
                IpRoute {
                    network: ip("172.16.0.0"),
                    prefix_len: 12
                },
                IpRoute {
                    network: ip("10.9.0.0"),
                    prefix_len: 16
                },
                IpRoute {
                    network: ip("0.0.0.0"),
                    prefix_len: 32
                },
                IpRoute {
                    network: ip("0.0.0.0"),
                    prefix_len: 0
                },
            ]
        );
    }

    #[test]
    fn rota_de_nome_ou_net_gateway_vira_aviso() {
        let p = ler("PUSH_REPLY,route servidor.exemplo,route net_gateway,route 10.0.0.0 255.0.0.0");
        assert_eq!(p.routes.len(), 1);
        assert_eq!(p.warnings.len(), 2);
    }

    #[test]
    fn redirect_gateway_vira_rota_padrao() {
        for flags in ["", " def1", " def1 bypass-dhcp", " local def1"] {
            let p = ler(&format!(
                "PUSH_REPLY,redirect-gateway{flags},route 10.0.0.0 255.0.0.0"
            ));
            assert!(p.redirect_gateway, "{flags}");
            let rotas = p.effective_routes();
            assert_eq!(
                rotas.last(),
                Some(&IpRoute {
                    network: Ipv4Addr::UNSPECIFIED,
                    prefix_len: 0
                })
            );
            assert_eq!(rotas.len(), 2);
        }
        let p = ler("PUSH_REPLY,route 10.0.0.0 255.0.0.0");
        assert!(!p.redirect_gateway);
        assert_eq!(p.effective_routes().len(), 1);
    }

    #[test]
    fn dhcp_option_dns_e_dominios() {
        let p = ler(
            "PUSH_REPLY,dhcp-option DNS 10.8.0.1,dhcp-option DNS 10.8.0.2,dhcp-option DNS6 2001:db8::1,\
             dhcp-option DOMAIN a.exemplo,dhcp-option DOMAIN-SEARCH b.exemplo,dhcp-option DOMAIN a.exemplo,\
             dhcp-option WINS 10.8.0.9,dhcp-option NBT 2,dhcp-option DISABLE-NBT",
        );
        assert_eq!(
            p.dns_servers,
            vec![
                "10.8.0.1:53".parse().unwrap(),
                "10.8.0.2:53".parse().unwrap(),
                "[2001:db8::1]:53".parse().unwrap()
            ]
        );
        assert_eq!(p.search_domains, vec!["a.exemplo", "b.exemplo"]);
        assert!(p.warnings.is_empty());
    }

    #[test]
    fn a_opcao_dns_vence_o_dhcp_option_e_ordena_pelo_numero() {
        let p = ler(
            "PUSH_REPLY,dhcp-option DNS 1.1.1.1,dhcp-option DOMAIN velho.exemplo,\
             dns server 2 address 10.8.0.20 10.8.0.21:5353,dns server 1 address 10.8.0.10,\
             dns server 1 resolve-domains corp.exemplo,dns search-domains novo.exemplo outro.exemplo",
        );
        assert_eq!(
            p.dns_servers,
            vec![
                "10.8.0.10:53".parse().unwrap(),
                "10.8.0.20:53".parse().unwrap(),
                "10.8.0.21:5353".parse().unwrap()
            ]
        );
        assert_eq!(p.search_domains, vec!["novo.exemplo", "outro.exemplo"]);
    }

    #[test]
    fn dns_cifrado_e_ignorado_com_aviso() {
        let p = ler(
            "PUSH_REPLY,dns server 1 address 10.8.0.10,dns server 1 transport DoH,dns server 2 address 10.8.0.11",
        );
        assert_eq!(p.dns_servers, vec!["10.8.0.11:53".parse().unwrap()]);
        assert_eq!(p.warnings.len(), 1);
    }

    #[test]
    fn continuacao_junta_as_mensagens() {
        let mut r = PushReader::new(&[], false);
        assert_eq!(
            r.feed("PUSH_REPLY,route 10.1.0.0 255.255.0.0,push-continuation 2")
                .unwrap()
                .map(|_| ()),
            None
        );
        assert_eq!(
            r.feed("PUSH_REPLY,route 10.2.0.0 255.255.0.0,push-continuation 2")
                .unwrap()
                .map(|_| ()),
            None
        );
        let p = r
            .feed("PUSH_REPLY,topology subnet,ifconfig 10.8.0.2 255.255.255.0,push-continuation 1")
            .unwrap()
            .unwrap();
        assert_eq!(p.routes.len(), 2);
        assert_eq!(p.ifconfig.unwrap().address, ip("10.8.0.2"));
        // O leitor pode ser reaproveitado para uma nova resposta (reconexão): nada sobra.
        let p = r.feed("PUSH_REPLY,ping 5").unwrap().unwrap();
        assert!(p.routes.is_empty());
        assert_eq!(p.ping, Some(5));
    }

    #[test]
    fn mensagem_que_nao_e_push_reply() {
        let mut r = PushReader::new(&[], false);
        assert_eq!(r.feed("AUTH_FAILED").unwrap_err(), PushError::NotPushReply);
    }

    #[test]
    fn resposta_vazia_e_com_terminador() {
        let p = ler("PUSH_REPLY");
        assert!(p.ifconfig.is_none());
        let p = ler("PUSH_REPLY,ping 3\0");
        assert_eq!(p.ping, Some(3));
    }

    #[test]
    fn pull_filter_em_ordem_a_primeira_que_combina_decide() {
        let filtros = [
            filtro(FilterAction::Accept, "route 10."),
            filtro(FilterAction::Ignore, "route"),
            filtro(FilterAction::Ignore, "dhcp-option DNS"),
        ];
        let p = parse_push_reply(
            "PUSH_REPLY,route 10.0.0.0 255.0.0.0,route 192.168.0.0 255.255.0.0,dhcp-option DNS 1.1.1.1,dhcp-option DOMAIN x.exemplo",
            &filtros,
            false,
        )
        .unwrap();
        assert_eq!(
            p.routes,
            vec![IpRoute {
                network: ip("10.0.0.0"),
                prefix_len: 8
            }]
        );
        assert!(p.dns_servers.is_empty());
        assert_eq!(p.search_domains, vec!["x.exemplo"]);
        assert_eq!(p.warnings.len(), 2);
        // Com a ordem trocada, o `ignore` genérico pega tudo.
        let filtros = [
            filtro(FilterAction::Ignore, "route"),
            filtro(FilterAction::Accept, "route 10."),
        ];
        let p = parse_push_reply("PUSH_REPLY,route 10.0.0.0 255.0.0.0", &filtros, false).unwrap();
        assert!(p.routes.is_empty());
    }

    #[test]
    fn pull_filter_reject_derruba() {
        let filtros = [filtro(FilterAction::Reject, "redirect-gateway")];
        let e = parse_push_reply("PUSH_REPLY,ping 10,redirect-gateway def1", &filtros, false)
            .unwrap_err();
        assert_eq!(e, PushError::Rejected("redirect-gateway def1".into()));
        assert!(e.to_string().contains("pull-filter reject"));
        // Sem a linha ofensora, passa.
        assert!(parse_push_reply("PUSH_REPLY,ping 10", &filtros, false).is_ok());
    }

    #[test]
    fn o_filtro_vale_em_cada_parte_da_continuacao() {
        let filtros = [filtro(FilterAction::Reject, "redirect-gateway")];
        let mut r = PushReader::new(&filtros, false);
        assert!(r.feed("PUSH_REPLY,ping 10,push-continuation 2").is_ok());
        assert!(
            r.feed("PUSH_REPLY,redirect-gateway def1,push-continuation 1")
                .is_err()
        );
    }

    #[test]
    fn route_nopull_descarta_rotas_e_dns_mas_deixa_o_endereco() {
        let p = parse_push_reply(
            "PUSH_REPLY,route 10.0.0.0 255.0.0.0,redirect-gateway def1,dhcp-option DNS 1.1.1.1,dns server 1 address 1.1.1.1,\
             route-ipv6 2001:db8::/32,topology subnet,ifconfig 10.8.0.2 255.255.255.0,ping 10",
            &[],
            true,
        )
        .unwrap();
        assert!(p.routes.is_empty());
        assert!(!p.redirect_gateway);
        assert!(p.dns_servers.is_empty());
        assert_eq!(p.ifconfig.unwrap().prefix_len, 24);
        assert_eq!(p.ping, Some(10));
        assert_eq!(p.warnings.len(), 5);
    }

    #[test]
    fn opcoes_desconhecidas_viram_aviso_e_as_conhecidas_ignoradas_nao() {
        let p = ler(
            "PUSH_REPLY,register-dns,block-outside-dns,setenv FOO bar,echo ola mundo,ifconfig-ipv6 fd00::2/64 fd00::1,\
             opcao-nova 1 2,aead-epoch,sndbuf 0,ping 7",
        );
        assert_eq!(p.echo, vec!["ola mundo"]);
        assert_eq!(p.ping, Some(7));
        let opcoes: Vec<&str> = p.warnings.iter().map(|w| w.option.as_str()).collect();
        assert_eq!(
            opcoes,
            vec!["ifconfig-ipv6 fd00::2/64 fd00::1", "opcao-nova 1 2"]
        );
    }

    #[test]
    fn auth_token_e_usuario() {
        let p = ler("PUSH_REPLY,auth-token SESS_ID_abc,auth-token-user bWFyaWE=");
        assert_eq!(
            p.auth_token.as_deref().map(String::as_str),
            Some("SESS_ID_abc")
        );
        assert_eq!(p.auth_token_user.as_deref(), Some("maria"));
        let texto = format!("{p:?}");
        assert!(!texto.contains("SESS_ID_abc"));
        assert!(parse_push_reply("PUSH_REPLY,auth-token-user ???", &[], false).is_err());
    }

    #[test]
    fn numeros_e_opcoes_invalidas() {
        for ruim in [
            "PUSH_REPLY,ping dez",
            "PUSH_REPLY,ping",
            "PUSH_REPLY,peer-id 16777215",
            "PUSH_REPLY,peer-id x",
            "PUSH_REPLY,ifconfig 10.0.0.1",
            "PUSH_REPLY,ifconfig abc def",
            "PUSH_REPLY,topology estranha",
            "PUSH_REPLY,key-derivation outra",
            "PUSH_REPLY,compress zzz",
            "PUSH_REPLY,dhcp-option DNS naoeip",
            "PUSH_REPLY,route 10.0.0.0 255.0.255.0",
        ] {
            assert!(parse_push_reply(ruim, &[], false).is_err(), "{ruim}");
        }
        assert_eq!(
            ler("PUSH_REPLY,peer-id 16777214").peer_id,
            Some(0x00FF_FFFE)
        );
    }

    #[test]
    fn mtu_saida_e_compressao() {
        let p = ler("PUSH_REPLY,tun-mtu 1400,explicit-exit-notify,compress stub-v2");
        assert_eq!(p.tun_mtu, Some(1400));
        assert_eq!(p.explicit_exit_notify, Some(1));
        assert_eq!(p.compress, Some(Compress::StubV2));
        assert_eq!(
            ler("PUSH_REPLY,explicit-exit-notify 3").explicit_exit_notify,
            Some(3)
        );
        assert_eq!(
            ler("PUSH_REPLY,comp-lzo no").compress,
            Some(Compress::LzoOff)
        );
        assert_eq!(
            ler("PUSH_REPLY,comp-lzo adaptive").compress,
            Some(Compress::Lzo)
        );
        assert_eq!(ler("PUSH_REPLY,compress").compress, Some(Compress::Stub));
        assert_eq!(
            ler("PUSH_REPLY,compress lz4-v2").compress,
            Some(Compress::Lz4V2)
        );
    }

    #[test]
    fn protocol_flags() {
        let p = ler("PUSH_REPLY,protocol-flags cc-exit tls-ekm,ping 1");
        assert!(p.key_derivation_ekm);
        assert_eq!(p.protocol_flags, vec!["cc-exit", "tls-ekm"]);
        assert!(p.warnings.is_empty());
        let p = ler("PUSH_REPLY,protocol-flags aead-epoch futuro");
        assert_eq!(p.warnings.len(), 2);
    }

    #[test]
    fn o_ultimo_valor_vence_nas_opcoes_unicas() {
        let p = ler("PUSH_REPLY,ping 5,ping 9,cipher AES-128-GCM,cipher AES-256-GCM");
        assert_eq!(p.ping, Some(9));
        assert_eq!(p.cipher.as_deref(), Some("AES-256-GCM"));
    }

    #[test]
    fn aspas_agrupam_palavras() {
        assert_eq!(
            split_args(r#"echo "dois  espaços" fim"#),
            vec!["echo", "dois  espaços", "fim"]
        );
        assert_eq!(split_args("  a   b "), vec!["a", "b"]);
        assert_eq!(split_args(r#"x """#), vec!["x", ""]);
    }

    #[test]
    fn tamanho_do_prefixo() {
        assert_eq!(prefix_len(ip("255.255.255.0")), Some(24));
        assert_eq!(prefix_len(ip("0.0.0.0")), Some(0));
        assert_eq!(prefix_len(ip("255.255.255.255")), Some(32));
        assert_eq!(prefix_len(ip("255.0.255.0")), None);
        assert_eq!(prefix_len(ip("0.255.255.255")), None);
    }
}
