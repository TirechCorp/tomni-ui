//! O enquadramento de um servidor MCP: JSON-RPC 2.0 pela entrada e saída padrão, **uma mensagem
//! por linha** e sem lotes.
//!
//! O app implementa [`McpTools`] (o nome do servidor, a lista de ferramentas e as chamadas) e
//! chama [`serve`]; o `initialize` (com a escolha da versão do protocolo), o `ping`, o
//! `tools/list`, o `tools/call`, o `notifications/cancelled` e os erros do JSON-RPC ficam por
//! conta do módulo. Nada aqui depende da interface: dá para usar num processo sem janela.
//!
//! ```
//! use std::io::Cursor;
//! use serde_json::{Value, json};
//! use tomni_ui::mcp::{McpError, McpTools, serve, text_result};
//!
//! struct Eco;
//!
//! impl McpTools for Eco {
//!     fn server_info(&self) -> (String, String) {
//!         ("eco".into(), "1.0".into())
//!     }
//!     fn instructions(&self) -> String {
//!         String::new()
//!     }
//!     fn tools(&self) -> Value {
//!         json!([{ "name": "eco", "description": "Devolve o texto.",
//!                  "inputSchema": { "type": "object" } }])
//!     }
//!     fn call(&mut self, name: &str, args: &Value, _id: &Value) -> Result<Value, McpError> {
//!         match name {
//!             "eco" => Ok(text_result(args["texto"].as_str().unwrap_or_default())),
//!             _ => Err(McpError::new(-32602, format!("ferramenta {name} desconhecida"))),
//!         }
//!     }
//! }
//!
//! let entrada = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
//! let mut saida = Vec::new();
//! serve(&mut Eco, Cursor::new(entrada), &mut saida);
//! assert!(String::from_utf8(saida).unwrap().contains(r#""result":{}"#));
//! ```

use std::io::{BufRead, Write};

use serde_json::{Map, Value, json};

/// As versões do protocolo MCP que o servidor aceita, da mais antiga à mais nova.
pub const PROTOCOL_VERSIONS: [&str; 3] = ["2024-11-05", "2025-03-26", "2025-06-18"];

/// Quem atende as ferramentas de um servidor MCP.
pub trait McpTools {
    /// O nome e a versão que vão em `serverInfo`.
    fn server_info(&self) -> (String, String);

    /// O texto de `instructions` do `initialize` (vazio: o campo não é enviado).
    fn instructions(&self) -> String;

    /// A lista de `tools/list`, já no formato do MCP (`name`, `description`, `inputSchema`).
    fn tools(&self) -> Value;

    /// Uma chamada de `tools/call`; `args` é `{}` quando o cliente não manda argumentos e `id`
    /// é o do pedido, para poder cancelar o que roda por ele. `Ok` é o `result` (`content`,
    /// `isError`; veja [`text_result`] e [`error_result`]); `Err` vira um erro JSON-RPC
    /// (ferramenta desconhecida, argumentos tortos).
    fn call(&mut self, name: &str, args: &Value, id: &Value) -> Result<Value, McpError>;

    /// O cliente cancelou o pedido `id` (`notifications/cancelled`). O padrão não faz nada.
    fn cancelled(&mut self, _id: &Value) {}
}

/// Um erro JSON-RPC: o código e a mensagem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpError {
    /// O código JSON-RPC (`-32602` para argumentos tortos, `-32601` para método desconhecido).
    pub code: i64,
    /// O texto que o cliente mostra.
    pub message: String,
}

impl McpError {
    /// Um erro com o `code` e a `message`.
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Um `result` de `tools/call` com um texto.
pub fn text_result(text: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": false })
}

/// Um `result` de `tools/call` que diz que a ferramenta falhou (o assistente lê o texto).
pub fn error_result(text: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": true })
}

fn error_reply(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// Atende uma mensagem já lida como JSON. `None` quando não há resposta: uma notificação, ou a
/// resposta do cliente a um pedido nosso (o servidor não faz pedidos).
pub fn handle(tools: &mut dyn McpTools, message: &Value) -> Option<Value> {
    let Some(method) = message["method"].as_str() else {
        if message.is_array() {
            return Some(error_reply(
                &Value::Null,
                -32600,
                "lotes de mensagens não são aceitos",
            ));
        }
        return None;
    };
    let params = &message["params"];
    if method == "notifications/cancelled" {
        if let Some(id) = params.get("requestId") {
            tools.cancelled(id);
        }
        return None;
    }
    // As outras notificações não têm `id` nem resposta.
    let id = message.get("id")?;
    let result = match method {
        "initialize" => {
            let asked = params["protocolVersion"].as_str().unwrap_or_default();
            let version = PROTOCOL_VERSIONS
                .into_iter()
                .find(|v| *v == asked)
                .unwrap_or(PROTOCOL_VERSIONS[PROTOCOL_VERSIONS.len() - 1]);
            let (name, server_version) = tools.server_info();
            let mut result = json!({
                "protocolVersion": version,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": name, "version": server_version },
            });
            let instructions = tools.instructions();
            if !instructions.is_empty() {
                result["instructions"] = Value::String(instructions);
            }
            result
        }
        "ping" => json!({}),
        "tools/list" => json!({ "tools": tools.tools() }),
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or_default();
            let empty = Value::Object(Map::new());
            let args = match &params["arguments"] {
                Value::Null => &empty,
                args => args,
            };
            match tools.call(name, args, id) {
                Ok(result) => result,
                Err(e) => return Some(error_reply(id, e.code, &e.message)),
            }
        }
        _ => {
            return Some(error_reply(
                id,
                -32601,
                &format!("método {method} desconhecido"),
            ));
        }
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

/// O laço inteiro: lê uma linha, chama [`handle`], escreve a resposta (uma linha) e esvazia o
/// buffer. As linhas vazias são ignoradas e um JSON inválido volta como `-32700`. Termina
/// quando a entrada fecha ou a saída falha.
pub fn serve(tools: &mut dyn McpTools, input: impl BufRead, mut output: impl Write) {
    for line in input.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(message) => handle(tools, &message),
            Err(e) => Some(error_reply(
                &Value::Null,
                -32700,
                &format!("JSON inválido: {e}"),
            )),
        };
        if let Some(reply) = reply
            && writeln!(output, "{reply}")
                .and_then(|()| output.flush())
                .is_err()
        {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    /// Um atendente de brinquedo: `soma` soma `a` e `b`, o resto é desconhecido.
    #[derive(Default)]
    struct Brinquedo {
        canceladas: Vec<Value>,
        ultimos_args: Option<Value>,
        instrucoes: String,
    }

    impl McpTools for Brinquedo {
        fn server_info(&self) -> (String, String) {
            ("brinquedo".into(), "0.3".into())
        }
        fn instructions(&self) -> String {
            self.instrucoes.clone()
        }
        fn tools(&self) -> Value {
            json!([{ "name": "soma", "description": "Soma.", "inputSchema": { "type": "object" } }])
        }
        fn call(&mut self, name: &str, args: &Value, _id: &Value) -> Result<Value, McpError> {
            self.ultimos_args = Some(args.clone());
            match name {
                "soma" => Ok(text_result(
                    &(args["a"].as_i64().unwrap_or(0) + args["b"].as_i64().unwrap_or(0))
                        .to_string(),
                )),
                _ => Err(McpError::new(
                    -32602,
                    format!("ferramenta {name} desconhecida"),
                )),
            }
        }
        fn cancelled(&mut self, id: &Value) {
            self.canceladas.push(id.clone());
        }
    }

    fn pedido(id: i64, method: &str, params: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
    }

    #[test]
    fn initialize_escolhe_a_versao_pedida() {
        let mut b = Brinquedo::default();
        let r = handle(
            &mut b,
            &pedido(1, "initialize", json!({ "protocolVersion": "2025-03-26" })),
        )
        .unwrap();
        assert_eq!(r["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(r["result"]["serverInfo"]["name"], "brinquedo");
        assert_eq!(r["result"]["serverInfo"]["version"], "0.3");
        assert_eq!(r["id"], 1);
        // Sem `instructions`, o campo não vai.
        assert!(r["result"].get("instructions").is_none());
    }

    #[test]
    fn initialize_cai_na_mais_nova_quando_nao_conhece() {
        let mut b = Brinquedo {
            instrucoes: "use com cuidado".into(),
            ..Default::default()
        };
        let r = handle(
            &mut b,
            &pedido(1, "initialize", json!({ "protocolVersion": "1999-01-01" })),
        )
        .unwrap();
        assert_eq!(r["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(r["result"]["instructions"], "use com cuidado");
        let sem = handle(&mut b, &pedido(2, "initialize", Value::Null)).unwrap();
        assert_eq!(sem["result"]["protocolVersion"], "2025-06-18");
    }

    #[test]
    fn tools_list_devolve_o_que_o_atendente_da() {
        let mut b = Brinquedo::default();
        let r = handle(&mut b, &pedido(3, "tools/list", Value::Null)).unwrap();
        assert_eq!(r["result"]["tools"][0]["name"], "soma");
    }

    #[test]
    fn tools_call_sem_argumentos_vira_objeto_vazio() {
        let mut b = Brinquedo::default();
        let r = handle(&mut b, &pedido(4, "tools/call", json!({ "name": "soma" }))).unwrap();
        assert_eq!(b.ultimos_args, Some(json!({})));
        assert_eq!(r["result"]["content"][0]["text"], "0");
        let r = handle(
            &mut b,
            &pedido(
                5,
                "tools/call",
                json!({ "name": "soma", "arguments": { "a": 2, "b": 3 } }),
            ),
        )
        .unwrap();
        assert_eq!(r["result"]["content"][0]["text"], "5");
        assert_eq!(r["result"]["isError"], false);
    }

    #[test]
    fn erro_do_atendente_vira_erro_json_rpc() {
        let mut b = Brinquedo::default();
        let r = handle(&mut b, &pedido(6, "tools/call", json!({ "name": "nada" }))).unwrap();
        assert_eq!(r["error"]["code"], -32602);
        assert_eq!(r["error"]["message"], "ferramenta nada desconhecida");
        assert_eq!(r["id"], 6);
    }

    #[test]
    fn notificacao_nao_tem_resposta() {
        let mut b = Brinquedo::default();
        let n = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        assert_eq!(handle(&mut b, &n), None);
        // A resposta do cliente a um pedido nosso também não tem resposta.
        let resposta = json!({ "jsonrpc": "2.0", "id": 9, "result": {} });
        assert_eq!(handle(&mut b, &resposta), None);
    }

    #[test]
    fn cancelamento_chama_cancelled() {
        let mut b = Brinquedo::default();
        let n = json!({ "jsonrpc": "2.0", "method": "notifications/cancelled",
                        "params": { "requestId": 7, "reason": "desisti" } });
        assert_eq!(handle(&mut b, &n), None);
        assert_eq!(b.canceladas, vec![json!(7)]);
        // O id pode ser texto.
        let n = json!({ "method": "notifications/cancelled", "params": { "requestId": "a-1" } });
        handle(&mut b, &n);
        assert_eq!(b.canceladas, vec![json!(7), json!("a-1")]);
        // Sem `requestId`, não há o que cancelar.
        handle(
            &mut b,
            &json!({ "method": "notifications/cancelled", "params": {} }),
        );
        assert_eq!(b.canceladas.len(), 2);
    }

    #[test]
    fn lote_e_recusado() {
        let mut b = Brinquedo::default();
        let lote = json!([
            pedido(1, "ping", Value::Null),
            pedido(2, "ping", Value::Null)
        ]);
        let r = handle(&mut b, &lote).unwrap();
        assert_eq!(r["error"]["code"], -32600);
        assert_eq!(r["id"], Value::Null);
    }

    #[test]
    fn metodo_desconhecido_da_32601() {
        let mut b = Brinquedo::default();
        let r = handle(&mut b, &pedido(8, "x/y", Value::Null)).unwrap();
        assert_eq!(r["error"]["code"], -32601);
        assert_eq!(r["error"]["message"], "método x/y desconhecido");
    }

    #[test]
    fn ping_responde_vazio() {
        let mut b = Brinquedo::default();
        let r = handle(&mut b, &pedido(1, "ping", Value::Null)).unwrap();
        assert_eq!(r["result"], json!({}));
    }

    #[test]
    fn serve_responde_linha_a_linha_e_ignora_linhas_vazias() {
        let mut b = Brinquedo::default();
        let entrada = format!(
            "{}\n\n   \n{}\nisso não é json\n{}\n",
            pedido(1, "ping", Value::Null),
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            pedido(2, "tools/list", Value::Null),
        );
        let mut saida = Vec::new();
        serve(&mut b, Cursor::new(entrada), &mut saida);
        let saida = String::from_utf8(saida).unwrap();
        let linhas: Vec<Value> = saida
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        // Uma resposta por pedido, a do JSON inválido e nada para a notificação.
        assert_eq!(linhas.len(), 3);
        assert_eq!(linhas[0]["id"], 1);
        assert_eq!(linhas[1]["error"]["code"], -32700);
        assert_eq!(linhas[1]["id"], Value::Null);
        assert_eq!(linhas[2]["result"]["tools"][0]["name"], "soma");
    }

    #[test]
    fn serve_para_quando_a_saida_falha() {
        struct Quebrada;
        impl Write for Quebrada {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut b = Brinquedo::default();
        let entrada = format!(
            "{}\n{}\n",
            pedido(1, "tools/call", json!({ "name": "soma" })),
            pedido(2, "tools/call", json!({ "name": "soma" })),
        );
        serve(&mut b, Cursor::new(entrada), Quebrada);
        // A segunda linha nem chega a ser lida: a primeira resposta já falhou.
        assert_eq!(b.ultimos_args, Some(json!({})));
    }

    #[test]
    fn os_ajudantes_marcam_o_erro() {
        assert_eq!(text_result("ok")["isError"], false);
        let e = error_result("falhou");
        assert_eq!(e["isError"], true);
        assert_eq!(e["content"][0]["text"], "falhou");
    }
}
