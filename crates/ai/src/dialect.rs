//! O que os dois dialetos têm em comum: o leitor de eventos que cada um implementa.

use serde_json::Value;

use crate::sse::SseEvent;
use crate::types::{AiError, AiErrorKind, Event, FinishReason, Usage};

/// Interpreta a resposta de um dialeto. Depois de um `Done` ou `Error` o driver para de chamar.
pub(crate) trait StreamParser: Send {
    /// Um evento SSE.
    fn event(&mut self, event: &SseEvent) -> Vec<Event>;
    /// O servidor fechou a conexão sem terminar a resposta.
    fn end(&mut self) -> Event;
    /// A resposta inteira, de um servidor que ignorou o `stream`.
    fn whole(&mut self, body: &str) -> Vec<Event>;
}

/// O texto de um erro de provedor dentro de um JSON: `error.message`, `message` ou `error`.
pub(crate) fn error_text(value: &Value) -> Option<String> {
    let found = value
        .pointer("/error/message")
        .or_else(|| value.get("message"))
        .or_else(|| value.get("error"))
        .and_then(Value::as_str)?;
    let found = found.trim();
    (!found.is_empty()).then(|| found.to_owned())
}

/// O erro que chega no meio do streaming: a classe vem do tipo ou do código que o provedor deu.
pub(crate) fn inline_error(value: &Value, hint: &str) -> AiError {
    let text = error_text(value).unwrap_or_else(|| "erro sem descrição".into());
    let lower = hint.to_ascii_lowercase();
    let code = value.pointer("/error/code").and_then(Value::as_u64);
    let kind = if lower.contains("overload") || matches!(code, Some(503 | 529)) {
        AiErrorKind::Overloaded
    } else if lower.contains("rate") || code == Some(429) {
        AiErrorKind::RateLimit
    } else if lower.contains("auth")
        || lower.contains("permission")
        || matches!(code, Some(401 | 403))
    {
        AiErrorKind::Auth
    } else if lower.contains("not_found") || code == Some(404) {
        AiErrorKind::NotFound
    } else {
        AiErrorKind::Server
    };
    AiError::new(
        kind,
        code.and_then(|c| u16::try_from(c).ok()),
        format!("O provedor mandou um erro durante a resposta: {text}."),
    )
}

pub(crate) fn finish_reason(raw: &str) -> FinishReason {
    match raw {
        "stop" | "end_turn" | "stop_sequence" => FinishReason::Stop,
        "length" | "max_tokens" => FinishReason::Length,
        other => FinishReason::Other(other.to_owned()),
    }
}

pub(crate) fn tokens(value: Option<&Value>) -> Option<u64> {
    value.and_then(Value::as_u64)
}

/// Junta o uso de duas fontes (o que veio no começo e o que veio no fim).
pub(crate) fn merge_usage(into: &mut Option<Usage>, input: Option<u64>, output: Option<u64>) {
    if input.is_none() && output.is_none() {
        return;
    }
    let usage = into.get_or_insert_with(Usage::default);
    usage.input = input.or(usage.input);
    usage.output = output.or(usage.output);
}
