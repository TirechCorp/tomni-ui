//! O agente HTTP que o S3 e a IA dividem: o `ureq` com o `rustls` (provedor `ring`), as raízes do
//! sistema (mais uma CA própria, se pedida) e o [`Proxy`] da conexão.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use ureq::Agent;
use ureq::tls::{Certificate, RootCerts, TlsConfig, TlsProvider};

use crate::Proxy;

/// Como o certificado do servidor é conferido.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum HttpTls {
    /// Pelas raízes do sistema.
    #[default]
    System,
    /// Pelas raízes do sistema e mais as autoridades deste arquivo PEM.
    CustomCa(PathBuf),
    /// Aceita qualquer certificado (um servidor de laboratório).
    NoVerify,
}

/// As opções de um [`agent`].
///
/// ```
/// use std::time::Duration;
/// use tomni_ui_net::http::{HttpOptions, agent};
///
/// let opcoes = HttpOptions {
///     user_agent: "meu-app".into(),
///     timeout_connect: Some(Duration::from_secs(5)),
///     ..HttpOptions::default()
/// };
/// assert!(agent(&opcoes).is_ok());
/// ```
#[derive(Clone, Debug)]
pub struct HttpOptions {
    /// Como o certificado é conferido.
    pub tls: HttpTls,
    /// O proxy por onde a conexão passa.
    pub proxy: Option<Proxy>,
    /// Quantos redirecionamentos o agente segue; `0` não segue nenhum (e a resposta 3xx volta
    /// como está, sem erro).
    pub max_redirects: u32,
    /// Quanto se espera pelo DNS e pelo TCP; `None` espera sem limite.
    pub timeout_connect: Option<Duration>,
    /// Quanto se espera pelos cabeçalhos da resposta; `None` espera sem limite. O corpo, lido
    /// aos poucos, nunca tem limite de tempo.
    pub timeout_recv_response: Option<Duration>,
    /// O cabeçalho `User-Agent`.
    pub user_agent: String,
}

impl Default for HttpOptions {
    fn default() -> Self {
        Self {
            tls: HttpTls::System,
            proxy: None,
            max_redirects: 0,
            timeout_connect: Some(crate::CONNECT_TIMEOUT),
            timeout_recv_response: Some(Duration::from_secs(60)),
            user_agent: "tomni-ui".into(),
        }
    }
}

/// Monta o agente: `rustls` com o `ring`, as raízes do sistema (mais a CA própria), o proxy, sem
/// tratar o status HTTP como erro e sem compressão (o corpo chega como o servidor mandou, o que
/// importa para o streaming).
pub fn agent(options: &HttpOptions) -> io::Result<Agent> {
    let mut roots: Vec<Certificate<'static>> = rustls_native_certs::load_native_certs()
        .certs
        .iter()
        .map(|der| Certificate::from_der(der.as_ref()).to_owned())
        .collect();
    if let HttpTls::CustomCa(path) = &options.tls {
        roots.extend(read_ca(path)?);
    }
    let tls_config = TlsConfig::builder()
        .provider(TlsProvider::Rustls)
        .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .root_certs(RootCerts::new_with_certs(&roots))
        .disable_verification(matches!(options.tls, HttpTls::NoVerify))
        .build();
    let proxy = options.proxy.as_ref().map(to_ureq_proxy).transpose()?;
    let config = Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(options.max_redirects)
        .max_redirects_will_error(false)
        .https_only(false)
        .tls_config(tls_config)
        .proxy(proxy)
        .user_agent(options.user_agent.as_str())
        .accept_encoding("identity")
        .timeout_resolve(options.timeout_connect)
        .timeout_connect(options.timeout_connect)
        .timeout_recv_response(options.timeout_recv_response)
        .build();
    Ok(Agent::new_with_config(config))
}

fn read_ca(path: &Path) -> io::Result<Vec<Certificate<'static>>> {
    let bytes = std::fs::read(path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "Não foi possível ler o arquivo da CA ({}): {}",
                path.display(),
                crate::error_message(&error)
            ),
        )
    })?;
    let mut certificates = Vec::new();
    for item in ureq::tls::parse_pem(&bytes) {
        if let Ok(ureq::tls::PemItem::Certificate(certificate)) = item {
            certificates.push(certificate);
        }
    }
    if certificates.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "O arquivo da CA não tem nenhum certificado em PEM: {}",
                path.display()
            ),
        ));
    }
    Ok(certificates)
}

/// O [`Proxy`] no formato do `ureq`. O SOCKS5 resolve o nome no proxy (`socks5h`).
pub fn to_ureq_proxy(proxy: &Proxy) -> io::Result<ureq::Proxy> {
    let (protocol, host, port, user, password) = match proxy {
        Proxy::Socks5 {
            host,
            port,
            user,
            password,
        } => (ureq::ProxyProtocol::Socks5h, host, port, user, password),
        Proxy::Http {
            host,
            port,
            user,
            password,
        } => (ureq::ProxyProtocol::Http, host, port, user, password),
    };
    let mut builder = ureq::Proxy::builder(protocol).host(host).port(*port);
    if let Some(user) = user {
        builder = builder.username(user);
    }
    if let Some(password) = password {
        builder = builder.password(password);
    }
    builder.build().map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("O proxy não serve ({error})"),
        )
    })
}
