//! O dialeto da Anthropic: `POST {base}/messages` com `stream`.

use serde_json::{Value, json};

use crate::dialect::{StreamParser, finish_reason, inline_error, merge_usage, tokens};
use crate::sse::SseEvent;
use crate::types::{
    AiError, AiErrorKind, Endpoint, Event, Finish, FinishReason, Request, Role, Usage,
};

/// A versão estável da API.
pub(crate) const VERSION: &str = "2023-06-01";
/// O `max_tokens` quando o pedido não diz (a API exige o campo).
const MAX_TOKENS_PADRAO: u32 = 4096;

/// As mensagens como a API aceita: papéis seguidos juntados com uma linha em branco e a primeira
/// sempre da pessoa.
pub(crate) fn messages(request: &Request) -> Vec<(&'static str, String)> {
    let mut out: Vec<(&'static str, String)> = Vec::new();
    for message in &request.messages {
        let role = match message.role {
            Role::User => "user",
            Role::Assistant => "assistant",
        };
        match out.last_mut() {
            Some((last, text)) if *last == role => {
                text.push_str("\n\n");
                text.push_str(&message.content);
            }
            _ => out.push((role, message.content.clone())),
        }
    }
    if out.first().is_none_or(|(role, _)| *role != "user") {
        out.insert(0, ("user", "(continuação)".to_owned()));
    }
    out
}

pub(crate) fn body(endpoint: &Endpoint, request: &Request) -> String {
    let messages: Vec<Value> = messages(request)
        .into_iter()
        .map(|(role, content)| json!({"role": role, "content": content}))
        .collect();
    let mut body = json!({
        "model": endpoint.model,
        "max_tokens": request.max_tokens.unwrap_or(MAX_TOKENS_PADRAO),
        "messages": messages,
        "stream": true,
    });
    if !request.system.is_empty() {
        body["system"] = json!(request.system);
    }
    if let Some(temperature) = request.temperature {
        body["temperature"] = json!(temperature);
    }
    body.to_string()
}

pub(crate) fn headers(endpoint: &Endpoint) -> Vec<(String, String)> {
    let mut headers = vec![
        ("anthropic-version".to_owned(), VERSION.to_owned()),
        ("Accept".to_owned(), "text/event-stream".to_owned()),
    ];
    if let Some(key) = &endpoint.api_key {
        headers.push(("x-api-key".to_owned(), key.as_str().to_owned()));
    }
    headers
}

#[derive(Default)]
pub(crate) struct Parser {
    reason: Option<FinishReason>,
    usage: Option<Usage>,
    saw_text: bool,
}

impl Parser {
    fn done(&mut self) -> Event {
        Event::Done(Finish {
            reason: self.reason.take().unwrap_or(FinishReason::Stop),
            usage: self.usage.take(),
        })
    }
}

impl StreamParser for Parser {
    fn event(&mut self, event: &SseEvent) -> Vec<Event> {
        let value: Value = match serde_json::from_str(event.data.trim()) {
            Ok(value) => value,
            Err(error) => return vec![Event::Error(AiError::protocol(error))],
        };
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .or(event.name.as_deref())
            .unwrap_or_default();
        match kind {
            "message_start" => {
                merge_usage(
                    &mut self.usage,
                    tokens(value.pointer("/message/usage/input_tokens")),
                    None,
                );
                Vec::new()
            }
            "content_block_delta" => {
                let delta = value.get("delta");
                let delta_type = delta
                    .and_then(|d| d.get("type"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                match delta_type {
                    "text_delta" => {
                        let text = delta
                            .and_then(|d| d.get("text"))
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        if text.is_empty() {
                            return Vec::new();
                        }
                        self.saw_text = true;
                        vec![Event::Text(text.to_owned())]
                    }
                    "thinking_delta" => {
                        let text = delta
                            .and_then(|d| d.get("thinking"))
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        if text.is_empty() {
                            Vec::new()
                        } else {
                            vec![Event::Reasoning(text.to_owned())]
                        }
                    }
                    _ => Vec::new(),
                }
            }
            "message_delta" => {
                if let Some(reason) = value.pointer("/delta/stop_reason").and_then(Value::as_str) {
                    self.reason = Some(finish_reason(reason));
                }
                merge_usage(
                    &mut self.usage,
                    tokens(value.pointer("/usage/input_tokens")),
                    tokens(value.pointer("/usage/output_tokens")),
                );
                Vec::new()
            }
            "message_stop" => vec![self.done()],
            "error" => {
                let hint = value
                    .pointer("/error/type")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                vec![Event::Error(inline_error(&value, hint))]
            }
            // `ping`, `content_block_start`, `content_block_stop` e os tipos que a API
            // acrescentar depois.
            _ => Vec::new(),
        }
    }

    fn end(&mut self) -> Event {
        if self.saw_text {
            if self.reason.is_none() {
                self.reason = Some(FinishReason::Other("fim sem aviso".into()));
            }
            return self.done();
        }
        Event::Error(AiError::new(
            AiErrorKind::Protocol,
            None,
            "O provedor encerrou a conexão sem mandar nenhuma resposta.",
        ))
    }

    fn whole(&mut self, body: &str) -> Vec<Event> {
        let value: Value = match serde_json::from_str(body) {
            Ok(value) => value,
            Err(error) => return vec![Event::Error(AiError::protocol(error))],
        };
        if value.get("type").and_then(Value::as_str) == Some("error") {
            let hint = value
                .pointer("/error/type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            return vec![Event::Error(inline_error(&value, hint))];
        }
        let Some(blocks) = value.get("content").and_then(Value::as_array) else {
            return vec![Event::Error(AiError::protocol("faltou o campo content"))];
        };
        let mut out = Vec::new();
        for block in blocks {
            let text = match block.get("type").and_then(Value::as_str) {
                Some("text") => block.get("text").and_then(Value::as_str),
                _ => None,
            };
            if let Some(text) = text.filter(|t| !t.is_empty()) {
                out.push(Event::Text(text.to_owned()));
            }
        }
        if let Some(reason) = value.get("stop_reason").and_then(Value::as_str) {
            self.reason = Some(finish_reason(reason));
        }
        merge_usage(
            &mut self.usage,
            tokens(value.pointer("/usage/input_tokens")),
            tokens(value.pointer("/usage/output_tokens")),
        );
        out.push(self.done());
        out
    }
}
