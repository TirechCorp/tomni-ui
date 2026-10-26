//! Modelos de linguagem para os apps do Tomni UI (M52): o pedido em streaming e a lista de
//! modelos de um provedor, pelos dois dialetos que quase todos falam.
//!
//! - [`Dialect::OpenAi`]: `POST {base}/chat/completions`. Ollama, LM Studio, OpenRouter, OpenAI,
//!   Groq, vLLM, llama.cpp e a maioria dos outros.
//! - [`Dialect::Anthropic`]: `POST {base}/messages`.
//!
//! O [`stream`] roda numa thread e entrega cada [`Event`] em ordem, com cancelamento
//! ([`StreamHandle::cancel`]). O HTTP é o `ureq` (síncrono, com o `rustls` e as raízes do
//! sistema) e o SSE é lido à mão: não há `tokio` nem `reqwest`. [`models`] lista os modelos e
//! [`locality`] diz se o endereço fica na máquina, na rede local ou na Internet.
//!
//! **Esta camada só fala com o provedor.** Ela não redige segredos, não limpa o texto e não
//! executa nada do que o modelo responde: isso é do app.
//!
//! Com a feature `test-util`, [`test_util::FakeAi`] é um servidor falso dos dois dialetos para
//! os testes dos apps.
//!
//! ```no_run
//! use std::sync::{Arc, Mutex};
//! use tomni_ui_ai::{Dialect, Endpoint, Event, Message, Request, stream};
//!
//! let ponto = Endpoint::new(Dialect::OpenAi, "http://localhost:11434/v1", "llama3");
//! let pedido = Request {
//!     system: "Responda em português.".into(),
//!     messages: vec![Message::user("Como listo as portas abertas?")],
//!     ..Request::default()
//! };
//! let recebido = Arc::new(Mutex::new(String::new()));
//! let fila = recebido.clone();
//! let _pedido = stream(ponto, pedido, move |evento| {
//!     if let Event::Text(pedaco) = evento {
//!         fila.lock().unwrap().push_str(&pedaco);
//!     }
//! });
//! ```

mod anthropic;
mod dialect;
mod http;
mod locality;
mod models;
mod openai;
mod sse;
#[cfg(any(test, feature = "test-util"))]
pub mod test_util;
mod types;

pub use http::{StreamHandle, stream};
pub use locality::{Locality, locality};
pub use models::models;
pub use types::{
    AiError, AiErrorKind, Dialect, Endpoint, Event, Finish, FinishReason, Message, ModelInfo,
    Request, Role, Tls, Usage,
};
pub use zeroize::Zeroizing;
