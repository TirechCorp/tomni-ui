//! O HTTP/1.1 mínimo que o plano de controle pede, escrito à mão: um `GET` simples (o `/key`)
//! e o pedido de `Upgrade` (o `/ts2021`), com TLS quando a URL é `https://`.
//!
//! Não há cliente HTTP completo aqui de propósito: são dois pedidos fixos, e o resto da
//! conversa (HTTP/2) roda por cima do canal já aberto.

use std::fmt;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName};
use rustls::{CertificateError, ClientConfig, RootCertStore};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

/// O tempo para abrir a conexão (TCP e TLS).
const TEMPO_CONEXAO: Duration = Duration::from_secs(20);
/// O tempo para o servidor responder ao pedido.
const TEMPO_RESPOSTA: Duration = Duration::from_secs(30);
/// O maior cabeçalho de resposta aceito.
const MAIOR_CABECALHO: usize = 16 * 1024;
/// O maior corpo aceito (a resposta do `/key` tem uns 150 bytes).
const MAIOR_CORPO: usize = 1024 * 1024;

/// O que pode dar errado falando com o controle por HTTP.
#[derive(Debug)]
pub enum UpgradeError {
    /// A URL do controle não serve.
    Url(String),
    /// Não foi possível abrir a conexão.
    Connect {
        /// O `host:porta` tentado.
        host: String,
        /// O que o sistema disse.
        source: io::Error,
    },
    /// O certificado do servidor não é confiável.
    Certificate {
        /// O nome do servidor.
        host: String,
        /// O que o TLS disse.
        detail: String,
    },
    /// O servidor respondeu com um status que não é o esperado.
    Status {
        /// O código HTTP.
        status: u16,
        /// O começo do corpo, para ajudar a entender.
        body: String,
    },
    /// A resposta não parece HTTP, ou não é a esperada.
    Protocol(String),
    /// A conexão caiu ou demorou demais.
    Io(io::Error),
}

impl fmt::Display for UpgradeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UpgradeError::Url(m) => write!(f, "a URL do servidor de controle não serve: {m}"),
            UpgradeError::Connect { host, source } => {
                write!(f, "não foi possível conectar a {host}: {source}")
            }
            UpgradeError::Certificate { host, detail } => write!(
                f,
                "o certificado de {host} não é confiável ({detail}); se o servidor usa uma CA \
                 própria, informe-a no perfil"
            ),
            UpgradeError::Status { status, body } => {
                write!(f, "o servidor respondeu HTTP {status}")?;
                if !body.is_empty() {
                    write!(f, ": {body}")?;
                }
                Ok(())
            }
            UpgradeError::Protocol(m) => write!(f, "resposta inesperada do servidor: {m}"),
            UpgradeError::Io(e) => write!(f, "a conexão com o servidor falhou: {e}"),
        }
    }
}

impl std::error::Error for UpgradeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            UpgradeError::Connect { source, .. } | UpgradeError::Io(source) => Some(source),
            _ => None,
        }
    }
}

impl From<io::Error> for UpgradeError {
    fn from(e: io::Error) -> Self {
        UpgradeError::Io(e)
    }
}

/// O servidor de controle: a URL já separada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// Se é `https`.
    pub https: bool,
    /// O nome ou IP do servidor.
    pub host: String,
    /// A porta (443 e 80 se a URL não disser).
    pub port: u16,
}

impl Target {
    /// Lê `https://host[:porta]` ou `http://host[:porta]` (uma barra final é aceita).
    pub fn parse(url: &str) -> Result<Self, UpgradeError> {
        let url = url.trim();
        let (https, resto) = if let Some(r) = url.strip_prefix("https://") {
            (true, r)
        } else if let Some(r) = url.strip_prefix("http://") {
            (false, r)
        } else {
            return Err(UpgradeError::Url(format!(
                "\"{url}\" precisa começar com https:// ou http://"
            )));
        };
        let autoridade = resto.split(['/', '?', '#']).next().unwrap_or("");
        if autoridade.is_empty() {
            return Err(UpgradeError::Url(format!("\"{url}\" não tem servidor")));
        }
        if autoridade.contains('@') {
            return Err(UpgradeError::Url(
                "a URL não pode ter usuário e senha".to_string(),
            ));
        }
        let padrao = if https { 443 } else { 80 };
        // IPv6 entre colchetes: `[::1]:8080`.
        let (host, porta) = if let Some(r) = autoridade.strip_prefix('[') {
            let (h, depois) = r
                .split_once(']')
                .ok_or_else(|| UpgradeError::Url(format!("\"{autoridade}\" está incompleto")))?;
            (h.to_string(), depois.strip_prefix(':'))
        } else if let Some((h, p)) = autoridade.rsplit_once(':') {
            (h.to_string(), Some(p))
        } else {
            (autoridade.to_string(), None)
        };
        let port = match porta {
            None | Some("") => padrao,
            Some(p) => p
                .parse()
                .map_err(|_| UpgradeError::Url(format!("a porta \"{p}\" não é um número")))?,
        };
        if host.is_empty() {
            return Err(UpgradeError::Url(format!("\"{url}\" não tem servidor")));
        }
        Ok(Target { https, host, port })
    }

    /// O valor do cabeçalho `Host`.
    fn cabecalho_host(&self) -> String {
        let padrao = if self.https { 443 } else { 80 };
        let nome = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        if self.port == padrao {
            nome
        } else {
            format!("{nome}:{}", self.port)
        }
    }

    /// O `host:porta` para as mensagens.
    pub fn authority(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// Em que raízes o TLS confia: as do sistema, mais as CAs extras do perfil.
#[derive(Clone, Default)]
pub struct Roots {
    /// As CAs extras, em PEM (um Headscale com CA própria).
    extras: Vec<CertificateDer<'static>>,
}

impl fmt::Debug for Roots {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Roots")
            .field("extras", &self.extras.len())
            .finish()
    }
}

impl Roots {
    /// Só as raízes do sistema.
    pub fn system() -> Self {
        Self::default()
    }

    /// Soma as CAs de um texto PEM.
    pub fn with_pem(mut self, pem: &str) -> Result<Self, UpgradeError> {
        for item in CertificateDer::pem_slice_iter(pem.as_bytes()) {
            let der = item
                .map_err(|e| UpgradeError::Url(format!("a CA extra não é um PEM válido: {e}")))?;
            self.extras.push(der);
        }
        if self.extras.is_empty() {
            return Err(UpgradeError::Url(
                "a CA extra não tem nenhum certificado".to_string(),
            ));
        }
        Ok(self)
    }

    fn config(&self) -> Arc<ClientConfig> {
        let mut raizes = RootCertStore::empty();
        raizes.add_parsable_certificates(rustls_native_certs::load_native_certs().certs);
        raizes.add_parsable_certificates(self.extras.iter().cloned());
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("as versões padrão do TLS valem para o ring")
            .with_root_certificates(raizes)
            .with_no_client_auth();
        Arc::new(config)
    }
}

/// Uma conexão qualquer, de leitura e escrita.
pub trait Duplex: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Duplex for T {}

/// A conexão aberta (TCP, ou TLS por cima), já sem o cabeçalho HTTP.
pub type Stream = Box<dyn Duplex>;

/// Uma conexão com alguns bytes já lidos na frente (o que veio junto com o cabeçalho).
pub struct Prefixed {
    antes: Vec<u8>,
    pos: usize,
    dentro: Stream,
}

impl Prefixed {
    pub(crate) fn new(antes: Vec<u8>, dentro: Stream) -> Self {
        Prefixed {
            antes,
            pos: 0,
            dentro,
        }
    }
}

impl AsyncRead for Prefixed {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.pos < self.antes.len() {
            let n = buf.remaining().min(self.antes.len() - self.pos);
            let (ini, fim) = (self.pos, self.pos + n);
            buf.put_slice(&self.antes[ini..fim]);
            self.pos = fim;
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut self.dentro).poll_read(cx, buf)
    }
}

impl AsyncWrite for Prefixed {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.dentro).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.dentro).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.dentro).poll_shutdown(cx)
    }
}

/// O que a resposta HTTP diz antes do corpo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    /// O código.
    pub status: u16,
    /// Os cabeçalhos, com o nome em minúsculas.
    pub headers: Vec<(String, String)>,
}

impl Head {
    /// O valor de um cabeçalho (o nome em minúsculas).
    pub fn header(&self, nome: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == nome)
            .map(|(_, v)| v.as_str())
    }
}

/// Abre a conexão com o servidor: TCP e, se `https`, o TLS por cima.
pub async fn connect(alvo: &Target, raizes: &Roots) -> Result<Stream, UpgradeError> {
    connect_to(alvo, raizes, None).await
}

/// Como [`connect`], mas discando para `ip` em vez de resolver o nome (o TLS continua
/// conferindo o nome do `alvo`). É o caso do DERP, que o mapa dá com o IP fixo.
pub async fn connect_to(
    alvo: &Target,
    raizes: &Roots,
    ip: Option<std::net::IpAddr>,
) -> Result<Stream, UpgradeError> {
    let abrir = async {
        let tcp = match ip {
            Some(ip) => TcpStream::connect((ip, alvo.port)).await,
            None => TcpStream::connect((alvo.host.as_str(), alvo.port)).await,
        }
        .map_err(|source| UpgradeError::Connect {
            host: alvo.authority(),
            source,
        })?;
        let _ = tcp.set_nodelay(true);
        if !alvo.https {
            return Ok(Box::new(tcp) as Stream);
        }
        let nome = ServerName::try_from(alvo.host.clone())
            .map_err(|_| UpgradeError::Url(format!("\"{}\" não é um nome válido", alvo.host)))?;
        let tls = TlsConnector::from(raizes.config())
            .connect(nome, tcp)
            .await
            .map_err(|e| erro_tls(alvo, e))?;
        Ok(Box::new(tls) as Stream)
    };
    tokio::time::timeout(TEMPO_CONEXAO, abrir)
        .await
        .map_err(|_| UpgradeError::Connect {
            host: alvo.authority(),
            source: io::Error::new(io::ErrorKind::TimedOut, "o servidor não respondeu a tempo"),
        })?
}

/// Separa o erro de certificado dos demais erros do TLS.
fn erro_tls(alvo: &Target, e: io::Error) -> UpgradeError {
    let interno = e
        .get_ref()
        .and_then(|i| i.downcast_ref::<rustls::Error>())
        .cloned();
    match interno {
        Some(rustls::Error::InvalidCertificate(c)) => UpgradeError::Certificate {
            host: alvo.host.clone(),
            detail: match c {
                CertificateError::UnknownIssuer => "emissor desconhecido".to_string(),
                CertificateError::Expired => "vencido".to_string(),
                CertificateError::NotValidYet => "ainda não vale".to_string(),
                CertificateError::NotValidForName => "não vale para este nome".to_string(),
                outro => format!("{outro:?}"),
            },
        },
        _ => UpgradeError::Connect {
            host: alvo.authority(),
            source: e,
        },
    }
}

/// Lê até o fim do cabeçalho (`\r\n\r\n`). Devolve o cabeçalho e o que sobrou depois dele.
async fn ler_cabecalho<R: AsyncRead + Unpin>(r: &mut R) -> Result<(Head, Vec<u8>), UpgradeError> {
    let mut buf: Vec<u8> = Vec::with_capacity(512);
    let mut pedaco = [0u8; 1024];
    let fim = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        if buf.len() > MAIOR_CABECALHO {
            return Err(UpgradeError::Protocol(
                "o cabeçalho da resposta é grande demais".to_string(),
            ));
        }
        let n = r.read(&mut pedaco).await?;
        if n == 0 {
            return Err(UpgradeError::Protocol(
                "a conexão fechou antes do fim do cabeçalho".to_string(),
            ));
        }
        buf.extend_from_slice(&pedaco[..n]);
    };
    let resto = buf.split_off(fim + 4);
    let texto = std::str::from_utf8(&buf[..fim])
        .map_err(|_| UpgradeError::Protocol("o cabeçalho não é texto".to_string()))?;
    let mut linhas = texto.split("\r\n");
    let status_linha = linhas.next().unwrap_or("");
    let mut partes = status_linha.split_whitespace();
    let versao = partes.next().unwrap_or("");
    if !versao.starts_with("HTTP/1.") {
        return Err(UpgradeError::Protocol(format!(
            "\"{}\" não é uma resposta HTTP/1.x",
            status_linha.chars().take(40).collect::<String>()
        )));
    }
    let status: u16 = partes
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| UpgradeError::Protocol("a resposta não tem código de status".to_string()))?;
    let headers = linhas
        .filter_map(|l| l.split_once(':'))
        .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    Ok((Head { status, headers }, resto))
}

/// Lê o corpo: `chunked`, `Content-Length` ou até o fim da conexão.
async fn ler_corpo<R: AsyncRead + Unpin>(
    r: &mut R,
    head: &Head,
    mut ja_lido: Vec<u8>,
) -> Result<Vec<u8>, UpgradeError> {
    if head
        .header("transfer-encoding")
        .is_some_and(|v| v.to_ascii_lowercase().contains("chunked"))
    {
        return ler_em_pedacos(r, ja_lido).await;
    }
    let tamanho: Option<usize> = head.header("content-length").and_then(|v| v.parse().ok());
    if let Some(t) = tamanho
        && t > MAIOR_CORPO
    {
        return Err(UpgradeError::Protocol(
            "o corpo é grande demais".to_string(),
        ));
    }
    let mut pedaco = [0u8; 4096];
    loop {
        if let Some(t) = tamanho
            && ja_lido.len() >= t
        {
            ja_lido.truncate(t);
            return Ok(ja_lido);
        }
        if ja_lido.len() > MAIOR_CORPO {
            return Err(UpgradeError::Protocol(
                "o corpo é grande demais".to_string(),
            ));
        }
        let n = r.read(&mut pedaco).await?;
        if n == 0 {
            if tamanho.is_some() {
                return Err(UpgradeError::Protocol(
                    "a conexão fechou no meio do corpo".to_string(),
                ));
            }
            return Ok(ja_lido);
        }
        ja_lido.extend_from_slice(&pedaco[..n]);
    }
}

/// Lê um corpo `Transfer-Encoding: chunked`.
async fn ler_em_pedacos<R: AsyncRead + Unpin>(
    r: &mut R,
    mut buf: Vec<u8>,
) -> Result<Vec<u8>, UpgradeError> {
    let mut corpo = Vec::new();
    let mut pedaco = [0u8; 4096];
    let mut pos = 0usize;
    loop {
        // Precisa de uma linha com o tamanho em hexadecimal.
        let fim_linha = loop {
            if let Some(i) = buf[pos..].windows(2).position(|w| w == b"\r\n") {
                break pos + i;
            }
            let n = r.read(&mut pedaco).await?;
            if n == 0 {
                return Err(UpgradeError::Protocol(
                    "a conexão fechou no meio do corpo".to_string(),
                ));
            }
            buf.extend_from_slice(&pedaco[..n]);
        };
        let linha = std::str::from_utf8(&buf[pos..fim_linha])
            .map_err(|_| UpgradeError::Protocol("tamanho de pedaço inválido".to_string()))?;
        let hex = linha.split(';').next().unwrap_or("").trim();
        let tam = usize::from_str_radix(hex, 16)
            .map_err(|_| UpgradeError::Protocol("tamanho de pedaço inválido".to_string()))?;
        pos = fim_linha + 2;
        if tam == 0 {
            return Ok(corpo);
        }
        if corpo.len() + tam > MAIOR_CORPO {
            return Err(UpgradeError::Protocol(
                "o corpo é grande demais".to_string(),
            ));
        }
        while buf.len() < pos + tam + 2 {
            let n = r.read(&mut pedaco).await?;
            if n == 0 {
                return Err(UpgradeError::Protocol(
                    "a conexão fechou no meio do corpo".to_string(),
                ));
            }
            buf.extend_from_slice(&pedaco[..n]);
        }
        corpo.extend_from_slice(&buf[pos..pos + tam]);
        pos += tam + 2;
    }
}

/// Faz um `GET` e devolve o corpo, se o status for 200.
pub async fn get(alvo: &Target, raizes: &Roots, caminho: &str) -> Result<Vec<u8>, UpgradeError> {
    let mut conexao = connect(alvo, raizes).await?;
    let pedido = format!(
        "GET {caminho} HTTP/1.1\r\nHost: {}\r\nUser-Agent: tomni-ui\r\nAccept: application/json\r\n\
         Connection: close\r\n\r\n",
        alvo.cabecalho_host()
    );
    let resposta = async {
        conexao.write_all(pedido.as_bytes()).await?;
        conexao.flush().await?;
        let (head, resto) = ler_cabecalho(&mut conexao).await?;
        let corpo = ler_corpo(&mut conexao, &head, resto).await?;
        Ok::<_, UpgradeError>((head, corpo))
    };
    let (head, corpo) = tokio::time::timeout(TEMPO_RESPOSTA, resposta)
        .await
        .map_err(|_| {
            UpgradeError::Io(io::Error::new(
                io::ErrorKind::TimedOut,
                "o servidor não respondeu a tempo",
            ))
        })??;
    if head.status != 200 {
        return Err(UpgradeError::Status {
            status: head.status,
            body: resumo_do_corpo(&corpo),
        });
    }
    Ok(corpo)
}

/// Os primeiros caracteres do corpo, em uma linha, para as mensagens de erro.
fn resumo_do_corpo(corpo: &[u8]) -> String {
    String::from_utf8_lossy(corpo)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(200)
        .collect()
}

/// Faz um pedido `POST` com `Upgrade` e devolve a conexão depois do `101`.
///
/// Os bytes que o servidor mandou junto com o cabeçalho (o começo da resposta Noise) já vêm
/// na frente da leitura.
pub async fn upgrade(
    alvo: &Target,
    raizes: &Roots,
    caminho: &str,
    protocolo: &str,
    extras: &[(&str, &str)],
) -> Result<Stream, UpgradeError> {
    upgrade_with(alvo, raizes, None, "POST", caminho, protocolo, extras).await
}

/// Como [`upgrade`], com o método HTTP (o DERP usa `GET`) e, se houver, o IP para discar.
pub async fn upgrade_with(
    alvo: &Target,
    raizes: &Roots,
    ip: Option<std::net::IpAddr>,
    metodo: &str,
    caminho: &str,
    protocolo: &str,
    extras: &[(&str, &str)],
) -> Result<Stream, UpgradeError> {
    let mut conexao = connect_to(alvo, raizes, ip).await?;
    let mut pedido = format!(
        "{metodo} {caminho} HTTP/1.1\r\nHost: {}\r\nUser-Agent: tomni-ui\r\nUpgrade: {protocolo}\r\n\
         Connection: upgrade\r\n",
        alvo.cabecalho_host()
    );
    for (nome, valor) in extras {
        pedido.push_str(&format!("{nome}: {valor}\r\n"));
    }
    pedido.push_str("\r\n");
    let resposta = async {
        conexao.write_all(pedido.as_bytes()).await?;
        conexao.flush().await?;
        let (head, resto) = ler_cabecalho(&mut conexao).await?;
        if head.status != 101 {
            let corpo = ler_corpo(&mut conexao, &head, resto)
                .await
                .unwrap_or_default();
            return Err(UpgradeError::Status {
                status: head.status,
                body: resumo_do_corpo(&corpo),
            });
        }
        let promete = head.header("upgrade").unwrap_or("");
        if !promete.eq_ignore_ascii_case(protocolo) {
            return Err(UpgradeError::Protocol(format!(
                "o servidor aceitou trocar para \"{promete}\" em vez de \"{protocolo}\""
            )));
        }
        Ok(resto)
    };
    let resto = tokio::time::timeout(TEMPO_RESPOSTA, resposta)
        .await
        .map_err(|_| {
            UpgradeError::Io(io::Error::new(
                io::ErrorKind::TimedOut,
                "o servidor não respondeu a tempo",
            ))
        })??;
    Ok(Box::new(Prefixed::new(resto, conexao)))
}

#[cfg(test)]
mod testes {
    use super::*;
    use tokio::net::TcpListener;

    #[test]
    fn le_a_url() {
        let t = Target::parse("https://controlplane.tailscale.com").unwrap();
        assert_eq!(
            t,
            Target {
                https: true,
                host: "controlplane.tailscale.com".into(),
                port: 443
            }
        );
        let t = Target::parse("http://127.0.0.1:8080/").unwrap();
        assert_eq!(
            (t.https, t.host.as_str(), t.port),
            (false, "127.0.0.1", 8080)
        );
        let t = Target::parse("https://[::1]:9443/x").unwrap();
        assert_eq!((t.host.as_str(), t.port), ("::1", 9443));
        assert_eq!(t.cabecalho_host(), "[::1]:9443");
        assert_eq!(
            Target::parse("http://hs.exemplo.com")
                .unwrap()
                .cabecalho_host(),
            "hs.exemplo.com"
        );
    }

    #[test]
    fn recusa_a_url_ruim() {
        assert!(Target::parse("ftp://x").is_err());
        assert!(Target::parse("https://").is_err());
        assert!(Target::parse("https://a:b").is_err());
        assert!(Target::parse("https://u:p@host").is_err());
        assert!(Target::parse("host.com").is_err());
    }

    /// Um servidor que responde `resposta` (bytes crus) a qualquer pedido e devolve o pedido.
    async fn servidor(resposta: Vec<u8>) -> (Target, tokio::task::JoinHandle<Vec<u8>>) {
        let ouvinte = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let porta = ouvinte.local_addr().unwrap().port();
        let tarefa = tokio::spawn(async move {
            let (mut s, _) = ouvinte.accept().await.unwrap();
            let mut lido = Vec::new();
            let mut b = [0u8; 1024];
            while !lido.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = s.read(&mut b).await.unwrap();
                if n == 0 {
                    break;
                }
                lido.extend_from_slice(&b[..n]);
            }
            s.write_all(&resposta).await.unwrap();
            s.flush().await.unwrap();
            let _ = s.shutdown().await;
            lido
        });
        (
            Target {
                https: false,
                host: "127.0.0.1".into(),
                port: porta,
            },
            tarefa,
        )
    }

    #[tokio::test]
    async fn get_com_content_length() {
        let (alvo, pedido) = servidor(
            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 7\r\n\r\n{\"a\":1}"
                .to_vec(),
        )
        .await;
        let corpo = get(&alvo, &Roots::system(), "/key?v=113").await.unwrap();
        assert_eq!(corpo, b"{\"a\":1}");
        let texto = String::from_utf8(pedido.await.unwrap()).unwrap();
        assert!(texto.starts_with("GET /key?v=113 HTTP/1.1\r\n"));
        assert!(texto.contains(&format!("Host: 127.0.0.1:{}\r\n", alvo.port)));
    }

    #[tokio::test]
    async fn get_com_pedacos() {
        let (alvo, _) = servidor(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n4;x=1\r\ndefg\r\n0\r\n\r\n"
                .to_vec(),
        )
        .await;
        let corpo = get(&alvo, &Roots::system(), "/").await.unwrap();
        assert_eq!(corpo, b"abcdefg");
    }

    #[tokio::test]
    async fn get_sem_tamanho_le_ate_o_fim() {
        let (alvo, _) = servidor(b"HTTP/1.0 200 OK\r\n\r\ntudo".to_vec()).await;
        assert_eq!(get(&alvo, &Roots::system(), "/").await.unwrap(), b"tudo");
    }

    #[tokio::test]
    async fn status_diferente_de_200_da_erro_com_o_corpo() {
        let (alvo, _) =
            servidor(b"HTTP/1.1 404 Not Found\r\nContent-Length: 10\r\n\r\nnada aqui\n".to_vec())
                .await;
        match get(&alvo, &Roots::system(), "/key").await {
            Err(UpgradeError::Status { status: 404, body }) => assert_eq!(body, "nada aqui"),
            outro => panic!("esperava 404, veio {outro:?}"),
        }
    }

    #[tokio::test]
    async fn resposta_que_nao_e_http_da_erro() {
        let (alvo, _) = servidor(b"SSH-2.0-OpenSSH_9.6\r\n\r\n".to_vec()).await;
        assert!(matches!(
            get(&alvo, &Roots::system(), "/").await,
            Err(UpgradeError::Protocol(_))
        ));
    }

    #[tokio::test]
    async fn upgrade_devolve_os_bytes_que_vieram_junto() {
        let (alvo, pedido) = servidor(
            b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: tailscale-control-protocol\r\n\
              Connection: upgrade\r\n\r\nRESTO"
                .to_vec(),
        )
        .await;
        let mut s = upgrade(
            &alvo,
            &Roots::system(),
            "/ts2021",
            "tailscale-control-protocol",
            &[("X-Tailscale-Handshake", "AAAA")],
        )
        .await
        .unwrap();
        let mut resto = Vec::new();
        s.read_to_end(&mut resto).await.unwrap();
        assert_eq!(resto, b"RESTO");
        let texto = String::from_utf8(pedido.await.unwrap()).unwrap();
        assert!(texto.starts_with("POST /ts2021 HTTP/1.1\r\n"));
        assert!(texto.contains("Upgrade: tailscale-control-protocol\r\n"));
        assert!(texto.contains("Connection: upgrade\r\n"));
        assert!(texto.contains("X-Tailscale-Handshake: AAAA\r\n"));
    }

    #[tokio::test]
    async fn upgrade_recusado_mostra_o_status() {
        let (alvo, _) =
            servidor(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 11\r\n\r\nversao ruim".to_vec())
                .await;
        match upgrade(&alvo, &Roots::system(), "/ts2021", "x", &[]).await {
            Err(UpgradeError::Status { status: 400, body }) => assert_eq!(body, "versao ruim"),
            outro => panic!("esperava 400, veio {:?}", outro.err()),
        }
    }

    #[tokio::test]
    async fn conexao_recusada_diz_o_endereco() {
        // Uma porta que acabou de ser liberada.
        let porta = {
            let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
            l.local_addr().unwrap().port()
        };
        let alvo = Target {
            https: false,
            host: "127.0.0.1".into(),
            port: porta,
        };
        let e = get(&alvo, &Roots::system(), "/").await.unwrap_err();
        assert!(e.to_string().contains(&format!("127.0.0.1:{porta}")), "{e}");
    }

    #[test]
    fn a_ca_extra_precisa_ser_pem() {
        assert!(Roots::system().with_pem("isto não é PEM").is_err());
    }
}
