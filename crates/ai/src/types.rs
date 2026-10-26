//! Os tipos públicos: o endereço do provedor, o pedido, os eventos da resposta e os erros.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use tomni_ui_net::Proxy;
use zeroize::Zeroizing;

/// O formato do pedido e do streaming.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialect {
    /// `POST {base}/chat/completions`, `GET {base}/models`, `Authorization: Bearer`.
    /// Ollama, LM Studio, OpenRouter, OpenAI, Groq, vLLM, llama.cpp e a maioria dos outros.
    OpenAi,
    /// `POST {base}/messages`, `x-api-key`, `anthropic-version: 2023-06-01`.
    Anthropic,
}

/// Como o certificado do provedor é conferido.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Tls {
    /// Pelas raízes do sistema.
    #[default]
    System,
    /// Pelas raízes do sistema e mais as autoridades deste arquivo PEM.
    CustomCa(PathBuf),
    /// Aceita qualquer certificado (um servidor de laboratório).
    NoVerify,
}

/// Para onde vai o pedido e como.
///
/// ```
/// use tomni_ui_ai::{Dialect, Endpoint};
///
/// let ponto = Endpoint::new(Dialect::OpenAi, "http://localhost:11434/v1", "llama3");
/// assert!(ponto.api_key.is_none());
/// ```
#[derive(Clone)]
pub struct Endpoint {
    /// O formato do provedor.
    pub dialect: Dialect,
    /// Até o `/v1`, sem a barra do fim: `http://localhost:11434/v1`.
    pub base_url: String,
    /// A chave de API. Fica na memória apagável e nunca aparece no `Debug`.
    pub api_key: Option<Zeroizing<String>>,
    /// O nome do modelo.
    pub model: String,
    /// Cabeçalhos a mais, na ordem (o OpenRouter aceita `HTTP-Referer` e `X-Title`).
    pub headers: Vec<(String, String)>,
    /// O proxy por onde o pedido passa.
    pub proxy: Option<Proxy>,
    /// Como o certificado é conferido.
    pub tls: Tls,
    /// Quanto se espera pelo DNS e pelo TCP. Padrão 10 s.
    pub connect_timeout: Duration,
    /// Quanto se espera pelos cabeçalhos da resposta. `None` espera sem limite (um modelo local
    /// carregando pode levar minutos). Padrão `None`.
    pub response_timeout: Option<Duration>,
}

impl Endpoint {
    /// Um endereço com os padrões: sem chave, sem cabeçalhos extras, sem proxy, TLS do sistema.
    pub fn new(dialect: Dialect, base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            dialect,
            base_url: base_url.into(),
            api_key: None,
            model: model.into(),
            headers: Vec::new(),
            proxy: None,
            tls: Tls::System,
            connect_timeout: Duration::from_secs(10),
            response_timeout: None,
        }
    }
}

impl fmt::Debug for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Sem a chave, sem o valor dos cabeçalhos (podem levar tokens) e sem a senha do proxy.
        f.debug_struct("Endpoint")
            .field("dialect", &self.dialect)
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "…"))
            .field("model", &self.model)
            .field(
                "headers",
                &self.headers.iter().map(|(k, _)| k).collect::<Vec<_>>(),
            )
            .field("proxy", &self.proxy)
            .field("tls", &self.tls)
            .finish_non_exhaustive()
    }
}

/// Quem escreveu a mensagem.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// A pessoa.
    User,
    /// O modelo.
    Assistant,
}

/// Uma mensagem da conversa.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    /// Quem escreveu.
    pub role: Role,
    /// O texto.
    pub content: String,
}

impl Message {
    /// Uma mensagem da pessoa.
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }

    /// Uma mensagem do modelo.
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}

/// O que se pede ao modelo.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Request {
    /// O prompt de sistema; vazio não vai.
    pub system: String,
    /// A conversa, da mais antiga para a mais nova.
    pub messages: Vec<Message>,
    /// Obrigatório no dialeto da Anthropic: `None` lá vira 4096. No compatível com OpenAI,
    /// `None` não manda o campo (alguns modelos recusam `max_tokens`).
    pub max_tokens: Option<u32>,
    /// A temperatura; `None` deixa a do provedor.
    pub temperature: Option<f32>,
}

/// O que chega da resposta, na ordem.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// Um pedaço da resposta.
    Text(String),
    /// Um pedaço do "raciocínio" que alguns modelos mandam à parte (`reasoning`,
    /// `reasoning_content`, `thinking_delta`). O app mostra "pensando…" e não guarda.
    Reasoning(String),
    /// O fim normal; nenhum evento depois dele.
    Done(Finish),
    /// O fim com erro; nenhum evento depois dele.
    Error(AiError),
}

/// Como a resposta terminou.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finish {
    /// O motivo.
    pub reason: FinishReason,
    /// Os tokens gastos, quando o provedor conta.
    pub usage: Option<Usage>,
}

/// Por que o modelo parou.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FinishReason {
    /// Terminou o que tinha a dizer.
    Stop,
    /// Bateu no limite de tokens da resposta: o texto está cortado.
    Length,
    /// Qualquer outro motivo, como o provedor o chamou.
    Other(String),
}

/// Os tokens gastos.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    /// Tokens do pedido.
    pub input: Option<u64>,
    /// Tokens da resposta.
    pub output: Option<u64>,
}

/// Um erro, com a mensagem pronta para a tela.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AiError {
    /// A classe do erro.
    pub kind: AiErrorKind,
    /// O status HTTP, quando houve resposta.
    pub status: Option<u16>,
    /// A mensagem em português.
    pub message: String,
}

impl fmt::Display for AiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for AiError {}

impl AiError {
    pub(crate) fn new(kind: AiErrorKind, status: Option<u16>, message: impl Into<String>) -> Self {
        Self {
            kind,
            status,
            message: message.into(),
        }
    }

    pub(crate) fn protocol(detail: impl fmt::Display) -> Self {
        Self::new(
            AiErrorKind::Protocol,
            None,
            format!("A resposta do provedor veio num formato inesperado: {detail}."),
        )
    }
}

/// A classe de um [`AiError`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AiErrorKind {
    /// Nada respondeu: conexão recusada, nome não achado, tempo esgotado ou conexão caída.
    Connect,
    /// A conexão segura falhou (certificado).
    Tls,
    /// O provedor recusou a chave (401, 403).
    Auth,
    /// O endereço ou o modelo não existe (404).
    NotFound,
    /// Pedidos demais (429).
    RateLimit,
    /// O provedor está sobrecarregado (503, 529).
    Overloaded,
    /// Qualquer outro erro do provedor.
    Server,
    /// A resposta não tem o formato esperado.
    Protocol,
}

/// Um modelo da lista do provedor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelInfo {
    /// O identificador, o que vai no campo `model`.
    pub id: String,
    /// O nome para mostrar, se o provedor der.
    pub name: Option<String>,
    /// O tamanho da janela de contexto, se o provedor disser.
    pub context: Option<u64>,
}
