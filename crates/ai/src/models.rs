//! A lista de modelos do provedor.

use serde_json::Value;

use crate::http::{build_agent, from_ureq, http_error};
use crate::locality::host_port;
use crate::types::{AiError, AiErrorKind, Dialect, Endpoint, ModelInfo};
use crate::{anthropic, openai};

/// O máximo de páginas que se segue na lista da Anthropic.
const MAX_PAGES: usize = 50;

/// `GET {base}/models`, bloqueante: chame numa thread. Segue a paginação da Anthropic
/// (`has_more` e `last_id`).
///
/// ```no_run
/// use tomni_ui_ai::{Dialect, Endpoint, models};
///
/// let ponto = Endpoint::new(Dialect::OpenAi, "http://localhost:11434/v1", "");
/// for modelo in models(&ponto).unwrap_or_default() {
///     println!("{}", modelo.id);
/// }
/// ```
pub fn models(endpoint: &Endpoint) -> Result<Vec<ModelInfo>, AiError> {
    let (host, port) = host_port(&endpoint.base_url);
    let agent = build_agent(endpoint).map_err(|e| crate::http::from_io(&e, &host, port))?;
    let base = endpoint.base_url.trim_end_matches('/');
    let headers = match endpoint.dialect {
        Dialect::OpenAi => openai::headers(endpoint),
        Dialect::Anthropic => anthropic::headers(endpoint),
    };
    let mut out = Vec::new();
    let mut after: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let mut url = format!("{base}/models");
        if endpoint.dialect == Dialect::Anthropic {
            url.push_str("?limit=1000");
            if let Some(after) = &after {
                url.push_str("&after_id=");
                url.push_str(after);
            }
        }
        let mut builder = agent.get(&url).header("Accept", "application/json");
        for (name, value) in headers.iter().chain(endpoint.headers.iter()) {
            if !name.eq_ignore_ascii_case("accept") {
                builder = builder.header(name.as_str(), value.as_str());
            }
        }
        let mut response = builder.call().map_err(|e| from_ureq(e, &host, port))?;
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let text = response
            .body_mut()
            .with_config()
            .limit(32 * 1024 * 1024)
            .read_to_string()
            .map_err(|e| from_ureq(e, &host, port))?;
        if status >= 400 {
            return Err(http_error(status, retry_after.as_deref(), &text));
        }
        let value: Value = serde_json::from_str(&text).map_err(AiError::protocol)?;
        out.extend(parse_page(&value)?);
        let more = value.get("has_more").and_then(Value::as_bool) == Some(true);
        match (
            endpoint.dialect,
            more,
            value.get("last_id").and_then(Value::as_str),
        ) {
            (Dialect::Anthropic, true, Some(last)) => after = Some(last.to_owned()),
            _ => break,
        }
    }
    Ok(out)
}

/// Os modelos de uma página: a lista em `data` (ou `models`, que alguns servidores usam).
fn parse_page(value: &Value) -> Result<Vec<ModelInfo>, AiError> {
    let list = value
        .get("data")
        .or_else(|| value.get("models"))
        .and_then(Value::as_array)
        .or_else(|| value.as_array())
        .ok_or_else(|| AiError::new(
            AiErrorKind::Protocol,
            None,
            "A resposta do provedor veio num formato inesperado: a lista de modelos não está em data.",
        ))?;
    let mut out = Vec::new();
    for item in list {
        let Some(id) = item
            .get("id")
            .or_else(|| item.get("name"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let name = item
            .get("display_name")
            .and_then(Value::as_str)
            .filter(|n| *n != id)
            .map(str::to_owned);
        let context = ["context_length", "context_window", "max_context_length"]
            .iter()
            .find_map(|key| item.get(key).and_then(Value::as_u64));
        out.push(ModelInfo {
            id: id.to_owned(),
            name,
            context,
        });
    }
    Ok(out)
}
