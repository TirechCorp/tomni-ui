//! O HTTP: monta o agente e o pedido, lê a resposta (SSE ou JSON inteiro) numa thread e traduz
//! os erros para o português.

use std::io::{self, Read};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use serde_json::Value;
use tomni_ui_net::http::{HttpOptions, HttpTls, agent};
use ureq::Agent;
use ureq::http::Response;

use crate::dialect::{StreamParser, error_text};
use crate::locality::host_port;
use crate::sse::SseParser;
use crate::types::{AiError, AiErrorKind, Dialect, Endpoint, Event, Request, Tls};
use crate::{anthropic, openai};

/// O tamanho de cada leitura do corpo.
const BLOCK: usize = 8 * 1024;
/// O quanto do corpo de um erro se lê.
const ERROR_BODY_LIMIT: u64 = 64 * 1024;
/// O maior corpo inteiro que se aceita (um servidor que ignora o `stream`, a lista de modelos).
const WHOLE_BODY_LIMIT: u64 = 32 * 1024 * 1024;

/// O pedido em andamento.
///
/// Parar o pedido com [`StreamHandle::cancel`] é **imediato para quem usa**: nenhum evento sai
/// depois dele. A thread termina na próxima leitura que voltar (os dois dialetos mandam `ping`
/// ou comentários enquanto esperam) ou quando o servidor fechar a conexão; a conexão não é
/// reaproveitada. O `ureq` não tem tempo de espera entre leituras do corpo, e esta camada não
/// inventa um.
#[derive(Clone, Debug)]
pub struct StreamHandle {
    cancelled: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
}

impl StreamHandle {
    /// Para o pedido: `on_event` não é mais chamado.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// A thread do pedido já terminou (depois do `Done`, do `Error` ou do cancelamento).
    pub fn finished(&self) -> bool {
        self.finished.load(Ordering::SeqCst)
    }
}

/// Abre o pedido numa thread e chama `on_event` com cada evento, na ordem, **na thread do
/// pedido**: quem desenha a tela guarda os eventos e acorda a interface (`wake()`). O último
/// evento é sempre um [`Event::Done`] ou um [`Event::Error`], exceto quando a pessoa cancelou.
///
/// A biblioteca não redige nem limpa o que vai no [`Request`]: isso é do app.
///
/// ```no_run
/// use std::sync::{Arc, Mutex};
/// use tomni_ui_ai::{Dialect, Endpoint, Event, Message, Request, stream};
///
/// let ponto = Endpoint::new(Dialect::OpenAi, "http://localhost:11434/v1", "llama3");
/// let pedido = Request {
///     messages: vec![Message::user("Diga oi")],
///     ..Request::default()
/// };
/// let eventos = Arc::new(Mutex::new(Vec::new()));
/// let fila = eventos.clone();
/// let pedido = stream(ponto, pedido, move |e: Event| fila.lock().unwrap().push(e));
/// pedido.cancel();
/// ```
pub fn stream(
    endpoint: Endpoint,
    request: Request,
    mut on_event: impl FnMut(Event) + Send + 'static,
) -> StreamHandle {
    let handle = StreamHandle {
        cancelled: Arc::new(AtomicBool::new(false)),
        finished: Arc::new(AtomicBool::new(false)),
    };
    let cancelled = handle.cancelled.clone();
    let finished = handle.finished.clone();
    let spawned = thread::Builder::new().name("tomni-ai".into()).spawn({
        let finished = finished.clone();
        move || {
            let mut emit = |event: Event| {
                if !cancelled.load(Ordering::SeqCst) {
                    on_event(event);
                }
            };
            run(&endpoint, &request, &cancelled, &mut emit);
            finished.store(true, Ordering::SeqCst);
        }
    });
    if spawned.is_err() {
        finished.store(true, Ordering::SeqCst);
    }
    handle
}

fn run(
    endpoint: &Endpoint,
    request: &Request,
    cancelled: &AtomicBool,
    emit: &mut dyn FnMut(Event),
) {
    let (host, port) = host_port(&endpoint.base_url);
    let agent = match build_agent(endpoint) {
        Ok(agent) => agent,
        Err(error) => return emit(Event::Error(from_io(&error, &host, port))),
    };
    let (path, body, extra) = match endpoint.dialect {
        Dialect::OpenAi => (
            "chat/completions",
            openai::body(endpoint, request),
            openai::headers(endpoint),
        ),
        Dialect::Anthropic => (
            "messages",
            anthropic::body(endpoint, request),
            anthropic::headers(endpoint),
        ),
    };
    let url = format!("{}/{path}", endpoint.base_url.trim_end_matches('/'));
    let mut builder = agent.post(&url).header("Content-Type", "application/json");
    for (name, value) in extra.iter().chain(endpoint.headers.iter()) {
        builder = builder.header(name.as_str(), value.as_str());
    }
    let response = match builder.send(body.as_str()) {
        Ok(response) => response,
        Err(error) => return emit(Event::Error(from_ureq(error, &host, port))),
    };
    if cancelled.load(Ordering::SeqCst) {
        return;
    }
    let status = response.status().as_u16();
    if status >= 400 {
        return emit(Event::Error(status_error(response, &host)));
    }
    let is_json = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.to_ascii_lowercase().contains("application/json"));
    let mut parser: Box<dyn StreamParser> = match endpoint.dialect {
        Dialect::OpenAi => Box::new(openai::Parser::default()),
        Dialect::Anthropic => Box::new(anthropic::Parser::default()),
    };
    let mut body = response.into_body();
    if is_json {
        let text = body
            .with_config()
            .limit(WHOLE_BODY_LIMIT)
            .read_to_string()
            .map_err(|e| from_ureq(e, &host, port));
        match text {
            Ok(text) => {
                for event in parser.whole(&text) {
                    emit(event);
                }
            }
            Err(error) => emit(Event::Error(error)),
        }
        return;
    }
    let mut reader = body.as_reader();
    let mut sse = SseParser::default();
    let mut block = [0u8; BLOCK];
    loop {
        if cancelled.load(Ordering::SeqCst) {
            return;
        }
        match reader.read(&mut block) {
            Ok(0) => break,
            Ok(n) => {
                if feed(&mut *parser, sse.push(&block[..n]), emit) {
                    return;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => {
                if !cancelled.load(Ordering::SeqCst) {
                    emit(Event::Error(AiError::new(
                        AiErrorKind::Connect,
                        None,
                        format!(
                            "A conexão com {host} caiu no meio da resposta: {}.",
                            tomni_ui_net::error_message(&error)
                        ),
                    )));
                }
                return;
            }
        }
    }
    if feed(&mut *parser, sse.finish(), emit) {
        return;
    }
    emit(parser.end());
}

/// Passa os eventos SSE pelo dialeto; `true` quando a resposta terminou (`Done` ou `Error`).
fn feed(
    parser: &mut dyn StreamParser,
    events: Vec<crate::sse::SseEvent>,
    emit: &mut dyn FnMut(Event),
) -> bool {
    for sse in events {
        for event in parser.event(&sse) {
            let last = matches!(event, Event::Done(_) | Event::Error(_));
            emit(event);
            if last {
                return true;
            }
        }
    }
    false
}

pub(crate) fn build_agent(endpoint: &Endpoint) -> io::Result<Agent> {
    agent(&HttpOptions {
        tls: match &endpoint.tls {
            Tls::System => HttpTls::System,
            Tls::CustomCa(path) => HttpTls::CustomCa(path.clone()),
            Tls::NoVerify => HttpTls::NoVerify,
        },
        proxy: endpoint.proxy.clone(),
        max_redirects: 0,
        timeout_connect: Some(endpoint.connect_timeout),
        timeout_recv_response: endpoint.response_timeout,
        user_agent: "tomni-ui".into(),
    })
}

/// Um erro de rede ou de TLS do `ureq`.
pub(crate) fn from_ureq(error: ureq::Error, host: &str, port: u16) -> AiError {
    use ureq::Error;
    match error {
        Error::HostNotFound => AiError::new(
            AiErrorKind::Connect,
            None,
            format!("Não achei o endereço {host}."),
        ),
        Error::ConnectionFailed | Error::Timeout(_) => connect_error(host, port),
        Error::Io(error) => from_io(&error, host, port),
        Error::Tls(_) | Error::Rustls(_) | Error::Pem(_) => tls_error(host, &error.to_string()),
        other => {
            let text = other.to_string();
            if looks_like_tls(&text) {
                tls_error(host, &text)
            } else {
                AiError::new(
                    AiErrorKind::Connect,
                    None,
                    format!("Não consegui falar com {host}:{port}: {text}."),
                )
            }
        }
    }
}

pub(crate) fn from_io(error: &io::Error, host: &str, port: u16) -> AiError {
    let text = tomni_ui_net::error_message(error);
    if looks_like_tls(&text) || looks_like_tls(&error.to_string()) {
        return tls_error(host, &text);
    }
    match error.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::InvalidInput | io::ErrorKind::InvalidData => {
            AiError::new(
                AiErrorKind::Connect,
                None,
                format!("Não consegui falar com {host}:{port}: {text}."),
            )
        }
        _ => connect_error(host, port),
    }
}

fn connect_error(host: &str, port: u16) -> AiError {
    AiError::new(
        AiErrorKind::Connect,
        None,
        format!("Nada respondeu em {host}:{port}."),
    )
}

fn tls_error(host: &str, detail: &str) -> AiError {
    AiError::new(
        AiErrorKind::Tls,
        None,
        format!("A conexão segura com {host} falhou: {detail}."),
    )
}

fn looks_like_tls(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    [
        "certificate",
        "unknownissuer",
        "tls",
        "handshake",
        "invalid peer",
    ]
    .iter()
    .any(|word| lower.contains(word))
}

/// O erro de uma resposta 4xx ou 5xx, lendo o corpo para pôr o texto do provedor na mensagem.
fn status_error(response: Response<ureq::Body>, host: &str) -> AiError {
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let body = response
        .into_body()
        .with_config()
        .limit(ERROR_BODY_LIMIT)
        .read_to_string()
        .unwrap_or_default();
    let _ = host;
    http_error(status, retry_after.as_deref(), &body)
}

/// A tabela de erros HTTP: a classe e a mensagem em português para um status.
pub(crate) fn http_error(status: u16, retry_after: Option<&str>, body: &str) -> AiError {
    let text = provider_text(body);
    let with_text = |lead: String| match &text {
        Some(text) => format!("{lead}: {text}."),
        None => format!("{lead}."),
    };
    let (kind, message) = match status {
        401 | 403 => (
            AiErrorKind::Auth,
            with_text(format!("O provedor recusou a chave (HTTP {status})")),
        ),
        404 => (
            AiErrorKind::NotFound,
            with_text("O provedor não conhece este endereço ou este modelo (HTTP 404)".into()),
        ),
        429 => {
            let wait = retry_after
                .and_then(|v| v.trim().parse::<u64>().ok())
                .map_or_else(|| "em instantes".to_owned(), |s| format!("em {s} s"));
            (
                AiErrorKind::RateLimit,
                format!("O provedor limitou os pedidos; tente de novo {wait}."),
            )
        }
        503 | 529 => (
            AiErrorKind::Overloaded,
            format!("O provedor está sobrecarregado (HTTP {status}); tente de novo em instantes."),
        ),
        _ => (
            AiErrorKind::Server,
            with_text(format!("O provedor respondeu HTTP {status}")),
        ),
    };
    AiError::new(kind, Some(status), message)
}

/// O texto que o provedor deu no corpo do erro: `error.message`, `message` ou o corpo cru
/// (cortado, numa linha só).
fn provider_text(body: &str) -> Option<String> {
    let body = body.trim();
    if body.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<Value>(body) {
        return error_text(&value);
    }
    let one_line: String = body.split_whitespace().collect::<Vec<_>>().join(" ");
    let cut: String = one_line.chars().take(300).collect();
    Some(cut.trim_end_matches('.').to_owned())
}
