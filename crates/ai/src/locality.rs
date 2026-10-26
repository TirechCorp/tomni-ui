//! Onde fica o endereço do provedor, sem resolver DNS.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Onde fica o endereço de um provedor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Locality {
    /// Este computador (`localhost`, `127.0.0.0/8`, `::1`).
    Machine,
    /// A rede local (faixas privadas, nomes sem ponto, `.local`, `.lan`).
    Lan,
    /// O resto: a Internet.
    Internet,
}

/// Onde fica o endereço, pelo host da URL, **sem resolver DNS**: um nome que aponta para a rede
/// local conta como [`Locality::Internet`].
///
/// ```
/// use tomni_ui_ai::{Locality, locality};
///
/// assert_eq!(locality("http://localhost:11434/v1"), Locality::Machine);
/// assert_eq!(locality("http://192.168.0.7:1234/v1"), Locality::Lan);
/// assert_eq!(locality("https://api.openai.com/v1"), Locality::Internet);
/// ```
pub fn locality(base_url: &str) -> Locality {
    let Some(host) = host_of(base_url) else {
        return Locality::Internet;
    };
    if host.is_empty() {
        return Locality::Internet;
    }
    if let Ok(ip) = host.parse::<Ipv4Addr>() {
        return of_v4(ip);
    }
    if let Ok(ip) = host.parse::<Ipv6Addr>() {
        return of_ip(IpAddr::V6(ip));
    }
    if host == "localhost" || host.ends_with(".localhost") {
        return Locality::Machine;
    }
    let local_suffix = [".local", ".lan", ".home.arpa", ".internal"]
        .iter()
        .any(|s| host.ends_with(s));
    if !host.contains('.') || local_suffix {
        return Locality::Lan;
    }
    Locality::Internet
}

fn of_ip(ip: IpAddr) -> Locality {
    match ip {
        IpAddr::V4(ip) => of_v4(ip),
        IpAddr::V6(ip) => {
            if let Some(v4) = ip.to_ipv4_mapped() {
                return of_v4(v4);
            }
            let first = ip.segments()[0];
            if ip.is_loopback() || ip.is_unspecified() {
                Locality::Machine
            } else if first & 0xfe00 == 0xfc00 || first & 0xffc0 == 0xfe80 {
                Locality::Lan
            } else {
                Locality::Internet
            }
        }
    }
}

fn of_v4(ip: Ipv4Addr) -> Locality {
    let [a, b, ..] = ip.octets();
    if ip.is_loopback() || ip.is_unspecified() {
        Locality::Machine
    } else if ip.is_private() || ip.is_link_local() || (a == 100 && (64..=127).contains(&b)) {
        Locality::Lan
    } else {
        Locality::Internet
    }
}

/// O host da URL, em minúsculas e sem colchetes, usuário nem porta.
pub(crate) fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority.rsplit_once('@').map_or(authority, |(_, a)| a);
    let host = if let Some(inner) = authority.strip_prefix('[') {
        inner.split(']').next()?
    } else {
        authority.split(':').next()?
    };
    Some(host.trim_end_matches('.').to_ascii_lowercase())
}

/// O host e a porta da URL (a padrão do esquema se faltar), para as mensagens de erro.
pub(crate) fn host_port(url: &str) -> (String, u16) {
    let host = host_of(url).unwrap_or_default();
    let https = url
        .trim_start()
        .to_ascii_lowercase()
        .starts_with("https://");
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority.rsplit_once('@').map_or(authority, |(_, a)| a);
    let after_host = if authority.starts_with('[') {
        authority.split_once(']').map_or("", |(_, r)| r)
    } else {
        authority.split_once(':').map_or("", |(_, r)| r)
    };
    let port = after_host
        .trim_start_matches(':')
        .parse::<u16>()
        .unwrap_or(if https { 443 } else { 80 });
    (host, port)
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn localidade() {
        let casos = [
            ("http://localhost:11434/v1", Locality::Machine),
            ("http://LOCALHOST/v1", Locality::Machine),
            ("http://127.0.0.1:1234/v1", Locality::Machine),
            ("http://127.5.5.5/v1", Locality::Machine),
            ("http://[::1]:8080/v1", Locality::Machine),
            ("http://0.0.0.0:8000", Locality::Machine),
            ("http://[::ffff:127.0.0.1]/v1", Locality::Machine),
            ("http://10.0.0.5/v1", Locality::Lan),
            ("http://172.16.0.1/v1", Locality::Lan),
            ("http://172.31.255.1/v1", Locality::Lan),
            ("http://172.32.0.1/v1", Locality::Internet),
            ("http://192.168.1.10:11434/v1", Locality::Lan),
            ("http://169.254.1.1/v1", Locality::Lan),
            ("http://100.64.0.1/v1", Locality::Lan),
            ("http://100.127.255.1/v1", Locality::Lan),
            ("http://100.128.0.1/v1", Locality::Internet),
            ("http://[fd00::1]/v1", Locality::Lan),
            ("http://[fe80::1]/v1", Locality::Lan),
            ("http://[2001:db8::1]/v1", Locality::Internet),
            ("http://servidor/v1", Locality::Lan),
            ("http://nas.local:11434/v1", Locality::Lan),
            ("http://gpu.lan/v1", Locality::Lan),
            ("http://gpu.home.arpa/v1", Locality::Lan),
            ("http://gpu.empresa.internal/v1", Locality::Lan),
            ("https://api.openai.com/v1", Locality::Internet),
            ("https://openrouter.ai/api/v1", Locality::Internet),
            (
                "https://usuario:senha@api.anthropic.com/v1",
                Locality::Internet,
            ),
            ("8.8.8.8", Locality::Internet),
            ("", Locality::Internet),
        ];
        for (url, esperado) in casos {
            assert_eq!(locality(url), esperado, "{url}");
        }
    }

    #[test]
    fn host_e_porta() {
        assert_eq!(
            host_port("http://localhost:11434/v1"),
            ("localhost".into(), 11434)
        );
        assert_eq!(
            host_port("https://api.openai.com/v1"),
            ("api.openai.com".into(), 443)
        );
        assert_eq!(host_port("http://a.b/v1"), ("a.b".into(), 80));
        assert_eq!(host_port("http://[::1]:99/v1"), ("::1".into(), 99));
    }
}
