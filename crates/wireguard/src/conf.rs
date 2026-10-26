//! O arquivo `.conf` do WireGuard (o formato do `wg-quick`), lido à mão.
//!
//! O que não se aplica a um app que não mexe no sistema (`Table`, `PostUp`, `FwMark`…) é
//! ignorado com um [`Aviso`]; o que o app não conhece é um erro, para um erro de digitação não
//! passar em silêncio.

use std::fmt;
use std::net::{IpAddr, SocketAddr};

use crate::key::{PresharedKey, PrivateKey, PublicKey};
use crate::peer::MTU_PADRAO;
use crate::router::Cidr;

/// Um erro ao ler ou conferir o `.conf`, com a linha quando há.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfError {
    /// A linha (a partir de 1) onde está o problema, quando é de uma linha só.
    pub line: Option<usize>,
    /// O que houve, em português.
    pub message: String,
}

impl ConfError {
    fn na_linha(line: usize, message: impl Into<String>) -> Self {
        ConfError {
            line: Some(line),
            message: message.into(),
        }
    }

    fn geral(message: impl Into<String>) -> Self {
        ConfError {
            line: None,
            message: message.into(),
        }
    }
}

impl fmt::Display for ConfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(l) => write!(f, "linha {l}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for ConfError {}

/// Algo do `.conf` que foi ignorado (não é erro).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Aviso {
    /// A linha (a partir de 1); 0 quando o aviso é do arquivo todo.
    pub line: usize,
    /// O que houve, em português.
    pub message: String,
}

impl fmt::Display for Aviso {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line == 0 {
            f.write_str(&self.message)
        } else {
            write!(f, "linha {}: {}", self.line, self.message)
        }
    }
}

/// Um endereço da interface, com o prefixo (`10.66.0.2/24`): o endereço do nó é `ip`, e o
/// prefixo diz a rede.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterfaceAddress {
    /// O endereço do nó.
    pub ip: IpAddr,
    /// O tamanho do prefixo.
    pub prefix: u8,
}

impl fmt::Display for InterfaceAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.ip, self.prefix)
    }
}

/// Para onde mandar os datagramas de um par.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    /// Um IP e uma porta.
    Addr(SocketAddr),
    /// Um nome e uma porta; o nome é resolvido na partida e de novo quando o handshake falha.
    Host {
        /// O nome.
        host: String,
        /// A porta.
        port: u16,
    },
}

impl Endpoint {
    /// Lê `host:porta`, `a.b.c.d:porta` ou `[v6]:porta`.
    pub fn parse(texto: &str) -> Option<Endpoint> {
        let texto = texto.trim();
        if let Ok(addr) = texto.parse::<SocketAddr>() {
            return Some(Endpoint::Addr(addr));
        }
        let (host, porta) = texto.rsplit_once(':')?;
        let porta: u16 = porta.trim().parse().ok()?;
        let host = host.trim();
        if host.is_empty() || host.contains(char::is_whitespace) || host.contains(':') {
            return None;
        }
        Some(Endpoint::Host {
            host: host.to_string(),
            port: porta,
        })
    }
}

impl fmt::Display for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Endpoint::Addr(a) => write!(f, "{a}"),
            Endpoint::Host { host, port } => write!(f, "{host}:{port}"),
        }
    }
}

/// A seção `[Interface]`.
#[derive(Clone)]
pub struct InterfaceConfig {
    /// A chave privada.
    pub private_key: PrivateKey,
    /// Os endereços do nó dentro da rede.
    pub addresses: Vec<InterfaceAddress>,
    /// Os servidores de nomes e domínios de busca (guardados; ainda sem uso).
    pub dns: Vec<String>,
    /// O MTU do túnel.
    pub mtu: u16,
}

impl fmt::Debug for InterfaceConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InterfaceConfig")
            .field("private_key", &"<oculta>")
            .field("addresses", &self.addresses)
            .field("dns", &self.dns)
            .field("mtu", &self.mtu)
            .finish()
    }
}

/// Uma seção `[Peer]`.
#[derive(Clone)]
pub struct PeerSection {
    /// A chave pública do par.
    pub public_key: PublicKey,
    /// A chave pré-compartilhada.
    pub preshared_key: Option<PresharedKey>,
    /// Para onde mandar.
    pub endpoint: Option<Endpoint>,
    /// As faixas que o par pode usar (`AllowedIPs`).
    pub allowed_ips: Vec<Cidr>,
    /// O keepalive persistente, em segundos.
    pub persistent_keepalive: Option<u16>,
}

impl PeerSection {
    /// Um par com só a chave pública.
    pub fn new(public_key: PublicKey) -> Self {
        PeerSection {
            public_key,
            preshared_key: None,
            endpoint: None,
            allowed_ips: Vec::new(),
            persistent_keepalive: None,
        }
    }
}

impl fmt::Debug for PeerSection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PeerSection")
            .field("public_key", &self.public_key)
            .field(
                "preshared_key",
                &self.preshared_key.as_ref().map(|_| "<oculta>"),
            )
            .field("endpoint", &self.endpoint)
            .field("allowed_ips", &self.allowed_ips)
            .field("persistent_keepalive", &self.persistent_keepalive)
            .finish()
    }
}

/// A configuração de um túnel WireGuard.
#[derive(Clone)]
pub struct WgConfig {
    /// A seção `[Interface]`.
    pub interface: InterfaceConfig,
    /// As seções `[Peer]`.
    pub peers: Vec<PeerSection>,
    /// O texto original, para [`WgConfig::without_secrets`] manter os comentários.
    source: Option<String>,
}

impl fmt::Debug for WgConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // O texto original tem as chaves: não aparece.
        f.debug_struct("WgConfig")
            .field("interface", &self.interface)
            .field("peers", &self.peers)
            .finish_non_exhaustive()
    }
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Secao {
    Fora,
    Interface,
    Par,
}

/// Tira o comentário (`#` até o fim da linha) e os espaços.
fn sem_comentario(linha: &str) -> &str {
    linha.split('#').next().unwrap_or("").trim()
}

/// Separa `Chave = valor` (o valor pode ter `=`, como o preenchimento do base64).
fn chave_valor(linha: &str) -> Option<(&str, &str)> {
    let (k, v) = linha.split_once('=')?;
    Some((k.trim(), v.trim()))
}

/// As chaves de `[Interface]` que não se aplicam a um app que não mexe no sistema.
const IGNORADAS: [&str; 7] = [
    "table",
    "preup",
    "postup",
    "predown",
    "postdown",
    "saveconfig",
    "fwmark",
];

impl WgConfig {
    /// Monta uma configuração à mão (nos testes e em quem já tem as chaves).
    pub fn new(private_key: PrivateKey) -> Self {
        WgConfig {
            interface: InterfaceConfig {
                private_key,
                addresses: Vec::new(),
                dns: Vec::new(),
                mtu: u16::try_from(MTU_PADRAO).unwrap_or(1420),
            },
            peers: Vec::new(),
            source: None,
        }
    }

    /// Lê um `.conf` no formato do `wg-quick`. Devolve a configuração e os avisos do que foi
    /// ignorado.
    ///
    /// # Errors
    ///
    /// Se uma linha não tem a forma `Chave = valor`, uma chave ou um valor é inválido, uma
    /// chave é desconhecida ou falta a `PrivateKey` ou a `PublicKey` de um par.
    pub fn from_conf(texto: &str) -> Result<(WgConfig, Vec<Aviso>), ConfError> {
        let mut avisos = Vec::new();
        let mut secao = Secao::Fora;
        let mut chave_privada: Option<PrivateKey> = None;
        let mut enderecos = Vec::new();
        let mut dns = Vec::new();
        let mut mtu = u16::try_from(MTU_PADRAO).unwrap_or(1420);
        let mut pares: Vec<(usize, PeerSection)> = Vec::new();
        let mut par_atual: Option<(usize, Option<PublicKey>, PeerSection)> = None;
        let mut viu_interface = false;

        fn fechar(
            atual: &mut Option<(usize, Option<PublicKey>, PeerSection)>,
            pares: &mut Vec<(usize, PeerSection)>,
        ) -> Result<(), ConfError> {
            if let Some((linha, chave, mut par)) = atual.take() {
                match chave {
                    Some(k) => {
                        par.public_key = k;
                        pares.push((linha, par));
                    }
                    None => {
                        return Err(ConfError::na_linha(
                            linha,
                            "falta a PublicKey na seção [Peer]",
                        ));
                    }
                }
            }
            Ok(())
        }

        for (indice, bruta) in texto.lines().enumerate() {
            let n = indice + 1;
            let linha = sem_comentario(bruta);
            if linha.is_empty() {
                continue;
            }
            if let Some(nome) = linha.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
                fechar(&mut par_atual, &mut pares)?;
                match nome.trim().to_ascii_lowercase().as_str() {
                    "interface" => {
                        if viu_interface {
                            return Err(ConfError::na_linha(n, "há mais de uma seção [Interface]"));
                        }
                        viu_interface = true;
                        secao = Secao::Interface;
                    }
                    "peer" => {
                        secao = Secao::Par;
                        // A chave pública fictícia é trocada ao fechar a seção.
                        par_atual =
                            Some((n, None, PeerSection::new(PublicKey::from_bytes([0; 32]))));
                    }
                    outra => {
                        return Err(ConfError::na_linha(
                            n,
                            format!(
                                "seção desconhecida [{outra}]; esperado: [Interface] ou [Peer]"
                            ),
                        ));
                    }
                }
                continue;
            }
            let Some((chave, valor)) = chave_valor(linha) else {
                return Err(ConfError::na_linha(
                    n,
                    format!("esperado \"Chave = valor\", achei \"{linha}\""),
                ));
            };
            let minuscula = chave.to_ascii_lowercase();
            match secao {
                Secao::Fora => {
                    return Err(ConfError::na_linha(
                        n,
                        format!(
                            "{chave} está fora de uma seção; esperado antes: [Interface] ou [Peer]"
                        ),
                    ));
                }
                Secao::Interface => match minuscula.as_str() {
                    "privatekey" => {
                        chave_privada = Some(PrivateKey::from_base64(valor).map_err(|_| {
                            ConfError::na_linha(
                                n,
                                "a chave privada do .conf não é válida (esperado: 32 bytes em base64)",
                            )
                        })?);
                    }
                    "address" => {
                        for item in valor.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                            let (ip, prefixo) = match item.split_once('/') {
                                Some((ip, p)) => (ip.trim(), Some(p.trim())),
                                None => (item, None),
                            };
                            let ip: IpAddr = ip.parse().map_err(|_| {
                                ConfError::na_linha(
                                    n,
                                    format!("Address inválido: \"{item}\"; esperado: 10.0.0.2/24"),
                                )
                            })?;
                            let maximo = if ip.is_ipv4() { 32 } else { 128 };
                            let prefix = match prefixo {
                                Some(p) => {
                                    p.parse::<u8>().ok().filter(|p| *p <= maximo).ok_or_else(
                                        || {
                                            ConfError::na_linha(
                                                n,
                                                format!("o prefixo de \"{item}\" não é válido"),
                                            )
                                        },
                                    )?
                                }
                                None => maximo,
                            };
                            enderecos.push(InterfaceAddress { ip, prefix });
                        }
                    }
                    "dns" => {
                        dns.extend(
                            valor
                                .split(',')
                                .map(str::trim)
                                .filter(|s| !s.is_empty())
                                .map(str::to_string),
                        );
                    }
                    "mtu" => {
                        mtu = valor.parse::<u16>().ok().filter(|m| *m >= 576).ok_or_else(|| {
                            ConfError::na_linha(
                                n,
                                format!("MTU inválido: \"{valor}\"; esperado um número de 576 a 65535"),
                            )
                        })?;
                    }
                    "listenport" => avisos.push(Aviso {
                        line: n,
                        message: "ListenPort ignorado: o app usa uma porta aleatória".into(),
                    }),
                    k if IGNORADAS.contains(&k) => avisos.push(Aviso {
                        line: n,
                        message: format!(
                            "{chave} não se aplica (o app não mexe no sistema); ignorada"
                        ),
                    }),
                    _ => {
                        return Err(ConfError::na_linha(
                            n,
                            format!("chave desconhecida em [Interface]: {chave}"),
                        ));
                    }
                },
                Secao::Par => {
                    let Some((_, chave_publica, par)) = par_atual.as_mut() else {
                        continue;
                    };
                    match minuscula.as_str() {
                        "publickey" => {
                            *chave_publica = Some(PublicKey::from_base64(valor).map_err(|_| {
                                ConfError::na_linha(
                                    n,
                                    "a PublicKey do par não é válida (esperado: 32 bytes em base64)",
                                )
                            })?);
                        }
                        "presharedkey" => {
                            par.preshared_key = Some(PresharedKey::from_base64(valor).map_err(
                                |_| {
                                    ConfError::na_linha(
                                        n,
                                        "a PresharedKey do par não é válida (esperado: 32 bytes em base64)",
                                    )
                                },
                            )?);
                        }
                        "endpoint" => {
                            par.endpoint = Some(Endpoint::parse(valor).ok_or_else(|| {
                                ConfError::na_linha(
                                    n,
                                    format!(
                                        "Endpoint inválido: \"{valor}\"; esperado: host:porta ou [v6]:porta"
                                    ),
                                )
                            })?);
                        }
                        "allowedips" => {
                            for item in valor.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                                let faixa: Cidr = item
                                    .parse()
                                    .map_err(|e| ConfError::na_linha(n, format!("{e}")))?;
                                par.allowed_ips.push(faixa);
                            }
                        }
                        "persistentkeepalive" => {
                            par.persistent_keepalive = match valor {
                                "off" | "0" => None,
                                v => Some(v.parse::<u16>().map_err(|_| {
                                    ConfError::na_linha(
                                        n,
                                        format!(
                                            "PersistentKeepalive inválido: \"{v}\"; esperado: segundos (0 a 65535) ou off"
                                        ),
                                    )
                                })?),
                            };
                        }
                        _ => {
                            return Err(ConfError::na_linha(
                                n,
                                format!("chave desconhecida em [Peer]: {chave}"),
                            ));
                        }
                    }
                }
            }
        }
        fechar(&mut par_atual, &mut pares)?;

        let private_key = chave_privada
            .ok_or_else(|| ConfError::geral("falta a PrivateKey na seção [Interface]"))?;
        for (linha, par) in &pares {
            if par.endpoint.is_none() {
                avisos.push(Aviso {
                    line: *linha,
                    message: format!(
                        "o par {} não tem Endpoint; só funciona se o outro lado iniciar",
                        par.public_key.short()
                    ),
                });
            }
        }
        Ok((
            WgConfig {
                interface: InterfaceConfig {
                    private_key,
                    addresses: enderecos,
                    dns,
                    mtu,
                },
                peers: pares.into_iter().map(|(_, p)| p).collect(),
                source: Some(texto.to_string()),
            },
            avisos,
        ))
    }

    /// Confere se a configuração serve para subir um túnel: pelo menos um endereço IPv4 (a
    /// pilha do app ainda é IPv4), pelo menos um par com `Endpoint`, nenhum par repetido e
    /// nenhuma faixa igual em dois pares (faixas que se sobrepõem são permitidas: vale a mais
    /// longa).
    ///
    /// # Errors
    ///
    /// Com todos os problemas achados, separados por "; ".
    pub fn check(&self) -> Result<(), ConfError> {
        let mut problemas: Vec<String> = Vec::new();
        if !self.interface.addresses.iter().any(|a| a.ip.is_ipv4()) {
            problemas.push(
                "o .conf não tem um Address IPv4 em [Interface]; o app ainda só fala IPv4".into(),
            );
        }
        if self.peers.is_empty() {
            problemas.push("o .conf não tem nenhum [Peer]".into());
        } else if self.peers.iter().all(|p| p.endpoint.is_none()) {
            problemas.push(
                "nenhum par tem Endpoint; o app só inicia o handshake com quem tem endereço".into(),
            );
        }
        for (i, par) in self.peers.iter().enumerate() {
            if self.peers[..i]
                .iter()
                .any(|outro| outro.public_key == par.public_key)
            {
                problemas.push(format!(
                    "o par {} aparece mais de uma vez",
                    par.public_key.short()
                ));
            }
            for faixa in &par.allowed_ips {
                if let Some(outro) = self.peers[..i]
                    .iter()
                    .find(|o| o.allowed_ips.contains(faixa))
                {
                    problemas.push(format!(
                        "a faixa {faixa} está nos AllowedIPs dos pares {} e {}",
                        outro.public_key.short(),
                        par.public_key.short()
                    ));
                }
            }
        }
        if problemas.is_empty() {
            Ok(())
        } else {
            Err(ConfError::geral(problemas.join("; ")))
        }
    }

    /// O texto do `.conf` sem `PrivateKey` e sem `PresharedKey`, mantendo o resto (e os
    /// comentários), para o app guardar fora do cofre. As chaves ficam no cofre.
    pub fn without_secrets(&self) -> String {
        match &self.source {
            Some(texto) => {
                let mut saida = String::with_capacity(texto.len());
                for linha in texto.lines() {
                    let chave = chave_valor(sem_comentario(linha))
                        .map(|(k, _)| k.to_ascii_lowercase())
                        .unwrap_or_default();
                    if chave == "privatekey" || chave == "presharedkey" {
                        continue;
                    }
                    saida.push_str(linha);
                    saida.push('\n');
                }
                saida
            }
            None => self.render(),
        }
    }

    /// Escreve a configuração (sem as chaves secretas) no formato do `wg-quick`.
    fn render(&self) -> String {
        let mut s = String::from("[Interface]\n");
        if !self.interface.addresses.is_empty() {
            let lista: Vec<String> = self
                .interface
                .addresses
                .iter()
                .map(ToString::to_string)
                .collect();
            s.push_str(&format!("Address = {}\n", lista.join(", ")));
        }
        if !self.interface.dns.is_empty() {
            s.push_str(&format!("DNS = {}\n", self.interface.dns.join(", ")));
        }
        s.push_str(&format!("MTU = {}\n", self.interface.mtu));
        for par in &self.peers {
            s.push_str(&format!("\n[Peer]\nPublicKey = {}\n", par.public_key));
            if let Some(e) = &par.endpoint {
                s.push_str(&format!("Endpoint = {e}\n"));
            }
            if !par.allowed_ips.is_empty() {
                let lista: Vec<String> = par.allowed_ips.iter().map(ToString::to_string).collect();
                s.push_str(&format!("AllowedIPs = {}\n", lista.join(", ")));
            }
            if let Some(k) = par.persistent_keepalive {
                s.push_str(&format!("PersistentKeepalive = {k}\n"));
            }
        }
        s
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    const PRIV: &str = "yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk=";
    const PUB: &str = "xTIBA5rboUvnH4htodjb6e697QjLERt1NAB4mZqp8Dg=";
    const PSK: &str = "FpCyhws9cxwWoV4xELtfJvjJN+zQVRPISllRWgeopVE=";

    fn conf_completo() -> String {
        format!(
            "# meu túnel\n[Interface]\nPrivateKey = {PRIV}\nAddress = 10.66.0.2/32, fd00::2/128\n\
             DNS = 10.66.0.1, exemplo.lan\nMTU = 1380\nListenPort = 51820\nPostUp = iptables -A x\n\
             Table = off\n\n[Peer]\n# o servidor\nPublicKey = {PUB}\nPresharedKey = {PSK}\n\
             Endpoint = vpn.exemplo.com:51820\nAllowedIPs = 10.66.0.0/24, 192.168.1.0/24\n\
             PersistentKeepalive = 25\n"
        )
    }

    #[test]
    fn le_um_conf_completo() {
        let (c, avisos) = WgConfig::from_conf(&conf_completo()).unwrap();
        assert_eq!(c.interface.mtu, 1380);
        assert_eq!(c.interface.addresses.len(), 2);
        assert_eq!(c.interface.addresses[0].to_string(), "10.66.0.2/32");
        assert_eq!(c.interface.dns, ["10.66.0.1", "exemplo.lan"]);
        assert_eq!(c.peers.len(), 1);
        let par = &c.peers[0];
        assert_eq!(par.public_key.to_base64(), PUB);
        assert!(par.preshared_key.is_some());
        assert_eq!(
            par.endpoint,
            Some(Endpoint::Host {
                host: "vpn.exemplo.com".into(),
                port: 51820
            })
        );
        assert_eq!(par.allowed_ips.len(), 2);
        assert_eq!(par.persistent_keepalive, Some(25));
        assert!(c.check().is_ok());
        let mensagens: Vec<String> = avisos.iter().map(ToString::to_string).collect();
        assert!(
            mensagens
                .iter()
                .any(|m| m.contains("ListenPort ignorado: o app usa uma porta aleatória"))
        );
        assert!(
            mensagens
                .iter()
                .any(|m| m.contains("PostUp não se aplica (o app não mexe no sistema); ignorada"))
        );
        assert!(mensagens.iter().any(|m| m.contains("Table não se aplica")));
    }

    #[test]
    fn o_mtu_padrao_e_1420() {
        let texto = format!("[Interface]\nPrivateKey = {PRIV}\nAddress = 10.0.0.2/32\n");
        let (c, _) = WgConfig::from_conf(&texto).unwrap();
        assert_eq!(c.interface.mtu, 1420);
    }

    #[test]
    fn a_chave_privada_invalida_diz_o_formato() {
        let e = WgConfig::from_conf("[Interface]\nPrivateKey = curta\n").unwrap_err();
        assert_eq!(e.line, Some(2));
        assert!(
            e.message
                .contains("a chave privada do .conf não é válida (esperado: 32 bytes em base64)")
        );
    }

    #[test]
    fn falta_a_chave_privada() {
        let e = WgConfig::from_conf("[Interface]\nAddress = 10.0.0.2/32\n").unwrap_err();
        assert!(e.message.contains("falta a PrivateKey"));
    }

    #[test]
    fn par_sem_chave_publica_e_erro_com_a_linha_da_secao() {
        let texto = format!("[Interface]\nPrivateKey = {PRIV}\n\n[Peer]\nEndpoint = 1.2.3.4:5\n");
        let e = WgConfig::from_conf(&texto).unwrap_err();
        assert_eq!(e.line, Some(4));
        assert!(e.message.contains("PublicKey"));
    }

    #[test]
    fn chave_desconhecida_e_erro() {
        let texto = format!("[Interface]\nPrivateKey = {PRIV}\nJc = 4\n");
        let e = WgConfig::from_conf(&texto).unwrap_err();
        assert!(e.message.contains("chave desconhecida em [Interface]: Jc"));
    }

    #[test]
    fn par_sem_endpoint_avisa() {
        let texto = format!(
            "[Interface]\nPrivateKey = {PRIV}\nAddress = 10.0.0.2/32\n[Peer]\nPublicKey = {PUB}\nAllowedIPs = 10.0.0.1/32\n"
        );
        let (c, avisos) = WgConfig::from_conf(&texto).unwrap();
        assert!(
            avisos[0]
                .message
                .contains("não tem Endpoint; só funciona se o outro lado iniciar")
        );
        let e = c.check().unwrap_err();
        assert!(e.message.contains("nenhum par tem Endpoint"));
    }

    #[test]
    fn allowed_ips_aceita_o_tudo_pelo_tunel_e_ip_sem_prefixo() {
        let texto = format!(
            "[Interface]\nPrivateKey = {PRIV}\nAddress = 10.0.0.2\n[Peer]\nPublicKey = {PUB}\nEndpoint = 1.2.3.4:51820\nAllowedIPs = 0.0.0.0/0, 10.0.0.1\n"
        );
        let (c, _) = WgConfig::from_conf(&texto).unwrap();
        assert_eq!(c.peers[0].allowed_ips[0].to_string(), "0.0.0.0/0");
        assert_eq!(c.peers[0].allowed_ips[1].to_string(), "10.0.0.1/32");
        assert_eq!(c.interface.addresses[0].prefix, 32);
        assert!(c.check().is_ok());
    }

    #[test]
    fn endpoints_ipv4_ipv6_e_nome() {
        assert_eq!(
            Endpoint::parse("1.2.3.4:51820"),
            Some(Endpoint::Addr("1.2.3.4:51820".parse().unwrap()))
        );
        assert_eq!(
            Endpoint::parse("[2001:db8::1]:51820"),
            Some(Endpoint::Addr("[2001:db8::1]:51820".parse().unwrap()))
        );
        assert_eq!(
            Endpoint::parse("vpn.exemplo.com:1"),
            Some(Endpoint::Host {
                host: "vpn.exemplo.com".into(),
                port: 1
            })
        );
        assert_eq!(Endpoint::parse("sem-porta"), None);
        assert_eq!(Endpoint::parse("a:b:1"), None);
        assert_eq!(Endpoint::parse("host:99999"), None);
    }

    #[test]
    fn check_recusa_par_repetido_e_faixa_igual() {
        let (mut c, _) = WgConfig::from_conf(&conf_completo()).unwrap();
        let mut outro = c.peers[0].clone();
        outro.public_key = PublicKey::from_bytes([7; 32]);
        c.peers.push(outro);
        let e = c.check().unwrap_err();
        assert!(e.message.contains("está nos AllowedIPs dos pares"));
        let mut c2 = c.clone();
        c2.peers[1] = c2.peers[0].clone();
        let e = c2.check().unwrap_err();
        assert!(e.message.contains("aparece mais de uma vez"));
    }

    #[test]
    fn faixas_sobrepostas_sao_permitidas() {
        let (mut c, _) = WgConfig::from_conf(&conf_completo()).unwrap();
        let mut outro = PeerSection::new(PublicKey::from_bytes([7; 32]));
        outro.allowed_ips = vec!["10.66.0.0/16".parse().unwrap()];
        c.peers.push(outro);
        assert!(c.check().is_ok());
    }

    #[test]
    fn sem_endereco_ipv4_o_check_recusa() {
        let texto = format!(
            "[Interface]\nPrivateKey = {PRIV}\nAddress = fd00::2/128\n[Peer]\nPublicKey = {PUB}\nEndpoint = 1.2.3.4:5\n"
        );
        let (c, _) = WgConfig::from_conf(&texto).unwrap();
        assert!(c.check().unwrap_err().message.contains("Address IPv4"));
    }

    #[test]
    fn sem_segredos_mantem_os_comentarios_e_tira_as_chaves() {
        let (c, _) = WgConfig::from_conf(&conf_completo()).unwrap();
        let limpo = c.without_secrets();
        assert!(limpo.contains("# meu túnel"));
        assert!(limpo.contains("# o servidor"));
        assert!(!limpo.contains("PrivateKey"));
        assert!(!limpo.contains("PresharedKey"));
        assert!(!limpo.contains(PRIV));
        assert!(!limpo.contains(PSK));
        assert!(limpo.contains(&format!("PublicKey = {PUB}")));
        assert!(limpo.contains("Endpoint = vpn.exemplo.com:51820"));
    }

    #[test]
    fn sem_segredos_de_uma_configuracao_montada_a_mao() {
        let mut c = WgConfig::new(PrivateKey::from_base64(PRIV).unwrap());
        c.interface.addresses.push(InterfaceAddress {
            ip: "10.0.0.2".parse().unwrap(),
            prefix: 32,
        });
        let mut par = PeerSection::new(PublicKey::from_base64(PUB).unwrap());
        par.preshared_key = Some(PresharedKey::from_base64(PSK).unwrap());
        par.endpoint = Endpoint::parse("1.2.3.4:51820");
        par.allowed_ips = vec!["10.0.0.1/32".parse().unwrap()];
        c.peers.push(par);
        let texto = c.without_secrets();
        assert!(!texto.contains(PRIV) && !texto.contains(PSK));
        assert!(texto.contains("Address = 10.0.0.2/32"));
    }

    #[test]
    fn o_debug_nao_mostra_as_chaves() {
        let (c, _) = WgConfig::from_conf(&conf_completo()).unwrap();
        let d = format!("{c:?}");
        assert!(!d.contains(PRIV) && !d.contains(PSK));
    }

    #[test]
    fn comentario_no_fim_da_linha_e_chave_em_minusculas() {
        let texto = format!(
            "[interface]\nprivatekey = {PRIV}   # segredo\naddress=10.0.0.2/24\n[PEER]\npublickey={PUB}\nendpoint = 1.2.3.4:5\npersistentkeepalive = off\n"
        );
        let (c, _) = WgConfig::from_conf(&texto).unwrap();
        assert_eq!(c.interface.addresses[0].prefix, 24);
        assert_eq!(c.peers[0].persistent_keepalive, None);
    }

    #[test]
    fn linha_sem_igual_e_erro() {
        let e = WgConfig::from_conf("[Interface]\nsolta\n").unwrap_err();
        assert_eq!(e.line, Some(2));
    }
}
