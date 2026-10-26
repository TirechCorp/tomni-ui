//! O servidor de verdade, por JSON-RPC, contra o exemplo `login` do `tomni-ui`.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

#[test]
fn sessao_com_o_login() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
    let mut server = Command::new(env!("CARGO_BIN_EXE_tomni-ui-mcp"))
        .args(["--manifest-path", manifest.to_str().unwrap()])
        .args(["-p", "tomni-ui", "--example", "login"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = server.stdin.take().unwrap();
    let mut stdout = BufReader::new(server.stdout.take().unwrap());
    let mut id = 0;
    let mut call = |method: &str, params: Value| -> Value {
        id += 1;
        let request = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        writeln!(stdin, "{request}").unwrap();
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        let reply: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(reply["id"], id);
        reply["result"].clone()
    };
    let text = |result: &Value| -> String {
        result["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|c| c["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n")
    };

    let init = call("initialize", json!({ "protocolVersion": "2025-06-18" }));
    assert_eq!(init["serverInfo"]["name"], "tomni-ui");

    let lint = call(
        "tools/call",
        json!({ "name": "ui_lint", "arguments": { "all": true } }),
    );
    assert_eq!(lint["isError"], false);
    assert!(
        text(&lint).contains("0 erro(s), 0 aviso(s)"),
        "{}",
        text(&lint)
    );

    // O estado continua entre chamadas: digitar, depois clicar.
    let digitar = call(
        "tools/call",
        json!({ "name": "ui_interact", "arguments": { "theme": "light", "reset": true, "actions": [
            { "action": "type", "id": "email", "text": "ana@x.com" },
            { "action": "type", "id": "senha", "text": "123456" }
        ] } }),
    );
    assert_eq!(digitar["isError"], false, "{}", text(&digitar));
    let image = &digitar["content"][2];
    assert_eq!(image["type"], "image");
    assert!(image["data"].as_str().unwrap().starts_with("iVBORw0KGgo"));
    let clicar = call(
        "tools/call",
        json!({ "name": "ui_interact", "arguments": { "actions": [{ "action": "click", "id": "entrar" }] } }),
    );
    assert!(
        text(&clicar).contains(r#"button #entrar "Bem-vindo!""#),
        "{}",
        text(&clicar)
    );

    let arvore = call(
        "tools/call",
        json!({ "name": "ui_tree", "arguments": { "format": "json", "width": 320 } }),
    );
    assert!(
        text(&arvore).contains(r#""text": "ana@x.com""#),
        "{}",
        text(&arvore)
    );
    assert!(text(&arvore).contains("tela login (tema claro, 320×844)"));

    let erro = call(
        "tools/call",
        json!({ "name": "ui_interact", "arguments": { "actions": [{ "action": "click", "id": "x" }] } }),
    );
    assert_eq!(erro["isError"], true);

    drop(stdin);
    assert!(server.wait().unwrap().success());
}
