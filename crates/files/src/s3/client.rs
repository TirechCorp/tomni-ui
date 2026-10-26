//! O HTTP do S3: monta o `ureq::Agent`, assina cada pedido, repete o que vale repetir, troca a
//! região quando o servidor aponta a certa e traduz os erros (o XML `<Error>`) para o português.

use std::collections::HashMap;
use std::io;
use std::sync::Mutex;
use std::time::Duration;

use rusty_s3::actions::S3Action;
use rusty_s3::{Bucket, Credentials, UrlStyle};
use tomni_ui_net::Proxy;
use tomni_ui_net::http::{HttpOptions, HttpTls};
use ureq::http::{Request, Response};
use ureq::{Agent, Body, SendBody};
use url::Url;

use super::TlsMode;
use super::xml;

/// Quanto tempo vale a assinatura de cada pedido.
pub(crate) const VALIDADE: Duration = Duration::from_secs(300);
/// Quantas vezes se repete um pedido que falhou por motivo passageiro.
const TENTATIVAS: u32 = 3;

/// Um pedido pronto para sair: o método, a URL assinada, os cabeçalhos que entraram na
/// assinatura e o corpo.
pub(crate) struct Prepared<'a> {
    pub method: &'static str,
    pub url: Url,
    pub headers: Vec<(String, String)>,
    pub body: &'a [u8],
}

impl<'a> Prepared<'a> {
    /// Assina `action` com os cabeçalhos extras (que vão no pedido e na assinatura).
    pub fn sign<'x, A: S3Action<'x>>(
        mut action: A,
        headers: &[(&str, String)],
        body: &'a [u8],
    ) -> Prepared<'a> {
        let mut sent = Vec::new();
        for (name, value) in headers {
            action
                .headers_mut()
                .insert((*name).to_owned(), value.clone());
            sent.push(((*name).to_owned(), value.clone()));
        }
        Prepared {
            method: A::METHOD.to_str(),
            url: action.sign(VALIDADE),
            headers: sent,
            body,
        }
    }
}

/// O que se leu do `<Error>` de uma resposta.
#[derive(Debug, Default)]
pub(crate) struct ErrorBody {
    pub code: String,
    pub message: String,
    pub region: Option<String>,
}

impl ErrorBody {
    pub fn parse(text: &str) -> ErrorBody {
        let Some(root) = xml::parse(text) else {
            return ErrorBody::default();
        };
        if root.name != "Error" {
            return ErrorBody::default();
        }
        ErrorBody {
            code: root.text_of("Code").unwrap_or_default().trim().to_owned(),
            message: root
                .text_of("Message")
                .unwrap_or_default()
                .trim()
                .to_owned(),
            region: root
                .text_of("Region")
                .map(|region| region.trim().to_owned())
                .filter(|region| !region.is_empty()),
        }
    }
}

/// A conexão HTTP com o S3: um agente, as credenciais e as regiões de cada bucket.
pub(crate) struct Client {
    agent: Agent,
    credentials: Credentials,
    endpoint: Url,
    style: UrlStyle,
    region: String,
    regions: Mutex<HashMap<String, String>>,
    retry_base: Duration,
}

impl Client {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        endpoint: Url,
        style: UrlStyle,
        region: String,
        credentials: Credentials,
        tls: &TlsMode,
        proxy: Option<&Proxy>,
        retry_base: Duration,
    ) -> io::Result<Client> {
        Ok(Client {
            agent: agent(tls, proxy)?,
            credentials,
            endpoint,
            style,
            region,
            regions: Mutex::new(HashMap::new()),
            retry_base,
        })
    }

    pub fn credentials(&self) -> &Credentials {
        &self.credentials
    }

    pub fn endpoint(&self) -> &Url {
        &self.endpoint
    }

    /// A região em uso (a do bucket, se o servidor corrigiu a configurada).
    pub fn region_of(&self, bucket: &str) -> String {
        self.regions
            .lock()
            .ok()
            .and_then(|regions| regions.get(bucket).cloned())
            .unwrap_or_else(|| self.region.clone())
    }

    /// O `Bucket` do `rusty-s3` com a região atual.
    pub fn bucket(&self, name: &str) -> io::Result<Bucket> {
        Bucket::new(
            self.endpoint.clone(),
            self.style,
            name.to_owned(),
            self.region_of(name),
        )
        .map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("O endereço do servidor S3 não serve para o bucket {name}: {error}"),
            )
        })
    }

    /// Um pedido a um bucket. `build` monta o pedido com o `Bucket` da região atual; é
    /// chamado de novo a cada tentativa e depois de uma troca de região.
    pub fn call<'a>(
        &self,
        bucket: &str,
        build: impl Fn(&Bucket, &Credentials) -> Prepared<'a>,
    ) -> io::Result<Response<Body>> {
        self.run(Some(bucket), &|| {
            let bucket = self.bucket(bucket)?;
            Ok(build(&bucket, &self.credentials))
        })
    }

    /// Um pedido ao serviço, sem bucket (`ListBuckets`).
    pub fn call_service<'a>(
        &self,
        build: impl Fn(&Url, &str, &Credentials) -> Prepared<'a>,
    ) -> io::Result<Response<Body>> {
        self.run(None, &|| {
            Ok(build(&self.endpoint, &self.region, &self.credentials))
        })
    }

    fn run<'a>(
        &self,
        bucket: Option<&str>,
        make: &dyn Fn() -> io::Result<Prepared<'a>>,
    ) -> io::Result<Response<Body>> {
        let mut attempt = 0u32;
        let mut switched = false;
        loop {
            let prepared = make()?;
            let mut response = match self.send(&prepared) {
                Ok(response) => response,
                Err(error) => {
                    let error = map_transport(error);
                    if attempt + 1 < TENTATIVAS && transient(&error) {
                        self.wait(attempt);
                        attempt += 1;
                        continue;
                    }
                    return Err(error);
                }
            };
            let status = response.status().as_u16();
            if (200..300).contains(&status) {
                return Ok(response);
            }
            let header_region = header(&response, "x-amz-bucket-region");
            let text = if prepared.method == "HEAD" {
                String::new()
            } else {
                response
                    .body_mut()
                    .with_config()
                    .limit(64 << 20)
                    .lossy_utf8(true)
                    .read_to_string()
                    .unwrap_or_default()
            };
            let body = ErrorBody::parse(&text);
            if !switched
                && let Some(name) = bucket
                && let Some(region) = header_region.clone().or_else(|| body.region.clone())
                && region != self.region_of(name)
            {
                if let Ok(mut regions) = self.regions.lock() {
                    regions.insert(name.to_owned(), region);
                }
                switched = true;
                continue;
            }
            let retry = status >= 500 || status == 429 || body.code == "SlowDown";
            if retry && attempt + 1 < TENTATIVAS {
                self.wait(attempt);
                attempt += 1;
                continue;
            }
            return Err(translate(status, &body, header_region.as_deref()));
        }
    }

    fn wait(&self, attempt: u32) {
        std::thread::sleep(self.retry_base * 2u32.pow(attempt));
    }

    fn send(&self, prepared: &Prepared<'_>) -> Result<Response<Body>, ureq::Error> {
        let mut builder = Request::builder()
            .method(prepared.method)
            .uri(prepared.url.as_str());
        for (name, value) in &prepared.headers {
            builder = builder.header(name.as_str(), value.as_str());
        }
        if prepared.body.is_empty() {
            let request = builder.body(SendBody::none()).map_err(ureq::Error::from)?;
            self.agent.run(request)
        } else {
            let request = builder.body(prepared.body).map_err(ureq::Error::from)?;
            self.agent.run(request)
        }
    }
}

/// O valor de um cabeçalho de resposta, em texto.
pub(crate) fn header(response: &Response<Body>, name: &str) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

/// Lê o corpo da resposta como texto.
pub(crate) fn text(response: &mut Response<Body>) -> io::Result<String> {
    response
        .body_mut()
        .with_config()
        .limit(256 << 20)
        .lossy_utf8(true)
        .read_to_string()
        .map_err(map_transport)
}

fn transient(error: &io::Error) -> bool {
    use io::ErrorKind::*;
    matches!(
        error.kind(),
        ConnectionReset | ConnectionAborted | BrokenPipe | UnexpectedEof | TimedOut
    )
}

/// Os erros de rede e de TLS do `ureq`, em português e com o `ErrorKind` que a tela entende.
pub(crate) fn map_transport(error: ureq::Error) -> io::Error {
    use ureq::Error;
    match error {
        Error::Io(error) => {
            let text = tomni_ui_net::error_message(&error);
            match certificate_hint(&text) {
                Some(hint) => {
                    io::Error::new(io::ErrorKind::InvalidData, format!("{hint} ({text})"))
                }
                None => io::Error::new(error.kind(), text),
            }
        }
        Error::Timeout(_) => io::Error::new(
            io::ErrorKind::TimedOut,
            "O servidor S3 não respondeu a tempo",
        ),
        Error::HostNotFound => io::Error::new(
            io::ErrorKind::ConnectionRefused,
            "Não foi possível encontrar o servidor S3: confira o endereço",
        ),
        Error::ConnectionFailed => io::Error::new(
            io::ErrorKind::ConnectionRefused,
            "Não foi possível conectar ao servidor S3",
        ),
        other => {
            let text = other.to_string();
            match certificate_hint(&text) {
                Some(hint) => io::Error::other(format!("{hint} ({text})")),
                None => io::Error::other(format!("Falha na conexão com o S3 ({text})")),
            }
        }
    }
}

/// A explicação de um erro de certificado do `rustls`, se o texto for de um.
fn certificate_hint(text: &str) -> Option<&'static str> {
    let lower = text.to_lowercase();
    (lower.contains("certificate") || lower.contains("unknownissuer")).then_some(
        "O certificado do servidor não é confiável: use uma CA própria ou desligue a verificação",
    )
}

/// O `<Error>` do S3 como `io::Error`: a mensagem em português e o código entre parênteses.
pub(crate) fn translate(status: u16, body: &ErrorBody, region: Option<&str>) -> io::Error {
    use io::ErrorKind::*;
    let code = body.code.as_str();
    let (kind, message): (io::ErrorKind, String) = match code {
        "AccessDenied" | "AllAccessDisabled" => (PermissionDenied, "Acesso negado".into()),
        "NoSuchBucket" => (NotFound, "O bucket não existe".into()),
        "NoSuchKey" | "NoSuchUpload" => (NotFound, "O objeto não existe".into()),
        "SignatureDoesNotMatch" => (
            PermissionDenied,
            "A assinatura não confere: confira o segredo e a região".into(),
        ),
        "InvalidAccessKeyId" => (
            PermissionDenied,
            "A chave de acesso não existe neste servidor".into(),
        ),
        "InvalidToken" | "ExpiredToken" | "TokenRefreshRequired" => (
            PermissionDenied,
            "O token de sessão venceu ou não vale".into(),
        ),
        "RequestTimeTooSkewed" => (
            PermissionDenied,
            "O relógio deste computador está muito diferente do servidor".into(),
        ),
        "AuthorizationHeaderMalformed" | "AuthorizationQueryParametersError" => (
            InvalidInput,
            match body.region.as_deref().or(region) {
                Some(region) => format!("Região errada: o bucket está em {region}"),
                None => "Região errada: confira a região da conexão".into(),
            },
        ),
        "PermanentRedirect" | "TemporaryRedirect" => (
            InvalidInput,
            match region {
                Some(region) => format!("Região errada: o bucket está em {region}"),
                None => "O bucket está em outro endereço: confira o endpoint e a região".into(),
            },
        ),
        "InvalidRange" => (
            InvalidInput,
            "O trecho pedido está além do fim do arquivo".into(),
        ),
        "BucketAlreadyExists" | "BucketAlreadyOwnedByYou" => {
            (AlreadyExists, "O bucket já existe".into())
        }
        "EntityTooLarge" => (InvalidInput, "O objeto é grande demais".into()),
        "EntityTooSmall" => (InvalidInput, "A parte do envio é pequena demais".into()),
        "SlowDown" | "ServiceUnavailable" | "InternalError" => (
            Other,
            "O servidor S3 está sobrecarregado; tente de novo daqui a pouco".into(),
        ),
        "NotImplemented" | "MethodNotAllowed" => {
            (Unsupported, "O servidor não aceita esta operação".into())
        }
        _ => {
            let kind = match status {
                404 => NotFound,
                401 | 403 => PermissionDenied,
                409 => AlreadyExists,
                501 => Unsupported,
                _ => Other,
            };
            let message = if !body.message.is_empty() {
                body.message.clone()
            } else {
                format!("O servidor S3 respondeu {status}")
            };
            (kind, message)
        }
    };
    let shown = if code.is_empty() {
        format!("{message} (HTTP {status})")
    } else {
        format!("{message} ({code})")
    };
    io::Error::new(kind, shown)
}

/// O agente HTTP: o do `tomni-ui-net`, sem redirecionamentos (a troca de região é nossa).
fn agent(tls: &TlsMode, proxy: Option<&Proxy>) -> io::Result<Agent> {
    let tls = match tls {
        TlsMode::Verify => HttpTls::System,
        TlsMode::CustomCa(path) => HttpTls::CustomCa(path.clone()),
        TlsMode::NoVerify => HttpTls::NoVerify,
    };
    tomni_ui_net::http::agent(&HttpOptions {
        tls,
        proxy: proxy.cloned(),
        ..HttpOptions::default()
    })
}
