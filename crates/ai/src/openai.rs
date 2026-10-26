//! O dialeto compatível com OpenAI: `POST {base}/chat/completions` com `stream`.

use serde_json::{Value, json};

use crate::dialect::{StreamParser, error_text, finish_reason, inline_error, merge_usage, tokens};
use crate::sse::SseEvent;
use crate::types::{
    AiError, AiErrorKind, Endpoint, Event, Finish, FinishReason, Request, Role, Usage,
};

/// O corpo do pedido. Sem `stream_options` (servidores antigos recusam o campo desconhecido) e
/// sem `max_tokens` quando não foi configurado (os modelos novos recusam o nome antigo).
pub(crate) fn body(endpoint: &Endpoint, request: &Request) -> String {
    let mut messages = Vec::new();
    if !request.system.is_empty() {
        messages.push(json!({"role": "system", "content": request.system}));
    }
    for message in &request.messages {
        let role = match message.role {
            Role::User => "user",
            Role::Assistant => "assistant",
        };
        messages.push(json!({"role": role, "content": message.content}));
    }
    let mut body = json!({"model": endpoint.model, "messages": messages, "stream": true});
    if let Some(max) = request.max_tokens {
        body["max_tokens"] = json!(max);
    }
    if let Some(temperature) = request.temperature {
        body["temperature"] = json!(temperature);
    }
    body.to_string()
}

/// Os cabeçalhos próprios do dialeto.
pub(crate) fn headers(endpoint: &Endpoint) -> Vec<(String, String)> {
    let mut headers = vec![("Accept".to_owned(), "text/event-stream".to_owned())];
    if let Some(key) = &endpoint.api_key {
        headers.push((
            "Authorization".to_owned(),
            format!("Bearer {}", key.as_str()),
        ));
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

    fn read_usage(&mut self, value: &Value) {
        if let Some(usage) = value.get("usage").filter(|u| u.is_object()) {
            merge_usage(
                &mut self.usage,
                tokens(usage.get("prompt_tokens")),
                tokens(usage.get("completion_tokens")),
            );
        }
    }
}

impl StreamParser for Parser {
    fn event(&mut self, event: &SseEvent) -> Vec<Event> {
        let data = event.data.trim();
        if data == "[DONE]" {
            return vec![self.done()];
        }
        let value: Value = match serde_json::from_str(data) {
            Ok(value) => value,
            Err(error) => return vec![Event::Error(AiError::protocol(error))],
        };
        if value.get("error").is_some_and(|e| !e.is_null()) {
            let hint = value
                .pointer("/error/type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            return vec![Event::Error(inline_error(&value, hint))];
        }
        self.read_usage(&value);
        let mut out = Vec::new();
        let Some(choice) = value.pointer("/choices/0") else {
            return out;
        };
        let delta = choice.get("delta");
        for key in ["reasoning", "reasoning_content"] {
            if let Some(text) = delta.and_then(|d| d.get(key)).and_then(Value::as_str)
                && !text.is_empty()
            {
                out.push(Event::Reasoning(text.to_owned()));
            }
        }
        if let Some(text) = delta.and_then(|d| d.get("content")).and_then(Value::as_str)
            && !text.is_empty()
        {
            self.saw_text = true;
            out.push(Event::Text(text.to_owned()));
        }
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.reason = Some(finish_reason(reason));
        }
        out
    }

    fn end(&mut self) -> Event {
        if self.reason.is_some() {
            return self.done();
        }
        if self.saw_text {
            self.reason = Some(FinishReason::Other("fim sem aviso".into()));
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
        if value.get("error").is_some_and(|e| !e.is_null()) {
            return vec![Event::Error(inline_error(&value, ""))];
        }
        let Some(message) = value.pointer("/choices/0/message") else {
            let detail = error_text(&value).unwrap_or_else(|| "faltou o campo choices".into());
            return vec![Event::Error(AiError::protocol(detail))];
        };
        let mut out = Vec::new();
        for key in ["reasoning", "reasoning_content"] {
            if let Some(text) = message.get(key).and_then(Value::as_str)
                && !text.is_empty()
            {
                out.push(Event::Reasoning(text.to_owned()));
            }
        }
        if let Some(text) = message.get("content").and_then(Value::as_str)
            && !text.is_empty()
        {
            out.push(Event::Text(text.to_owned()));
        }
        if let Some(reason) = value
            .pointer("/choices/0/finish_reason")
            .and_then(Value::as_str)
        {
            self.reason = Some(finish_reason(reason));
        }
        self.read_usage(&value);
        out.push(self.done());
        out
    }
}
