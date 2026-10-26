//! Servidor MCP com `ui_screenshot`, `ui_tree`, `ui_interact` e `ui_lint`.
//!
//! JSON-RPC 2.0 pela entrada e saída padrão, uma mensagem por linha. Cada chamada recompila o
//! app (`cargo build`, rápido quando nada mudou) e fala com ele em modo de ferramentas. A
//! sessão continua entre chamadas, com o estado da tela (texto digitado, cliques), até o app
//! ser recompilado, a tela mudar ou `reset: true`.
//!
//! Os argumentos do servidor são opções do `cargo` (`--example login`, `--manifest-path`),
//! usadas em toda chamada; `bin`, `example` e `package` nos parâmetros passam por cima.

use std::io;
use std::path::PathBuf;

use serde_json::{Value, json};
use tomni_ui::mcp::{self, McpError, McpTools};
use tomni_ui_cli::cargo::{self, Target};
use tomni_ui_cli::{Error, RULES, Screen, Session, format};

const INSTRUCTIONS: &str = "Ferramentas para ver e verificar a UI de um app Tomni UI. Cada \
chamada recompila o app e usa a tela de verdade, sem janela. Fluxo: depois de mudar uma tela, \
rode ui_lint (precisa de zero erros), olhe com ui_screenshot e teste interações com \
ui_interact (ações por id: click, type, key...). O estado da tela continua entre chamadas até \
o app ser recompilado, a tela mudar ou reset=true. O guia da API está no AGENTS.md do projeto.";

const HELP: &str = "\
tomni-ui-mcp: servidor MCP do Tomni UI (ui_screenshot, ui_tree, ui_interact e ui_lint)

Uso: tomni-ui-mcp [opções do cargo]

Fala JSON-RPC pela entrada e saída padrão; configure no cliente MCP (`tomni-ui init` grava o
.mcp.json). Opções do cargo, usadas para compilar o app em toda chamada:
  -p, --package P, --bin NOME, --example NOME, -F, --features F,
  --no-default-features, --manifest-path CAMINHO, --release
";

fn main() {
    let mut target = Target::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "-h" || arg == "--help" {
            print!("{HELP}");
            return;
        }
        let applied = target.apply(&arg, || {
            args.next()
                .ok_or_else(|| Error::Usage(format!("{arg} precisa de um valor")))
        });
        match applied {
            Ok(true) => {}
            Ok(false) => {
                eprintln!("tomni-ui-mcp: opção {arg} desconhecida; veja --help");
                std::process::exit(2);
            }
            Err(e) => {
                eprintln!("tomni-ui-mcp: {e}");
                std::process::exit(2);
            }
        }
    }
    let mut server = Server {
        target,
        session: None,
        last: None,
        out_dir: None,
    };
    mcp::serve(&mut server, io::stdin().lock(), io::stdout().lock());
}

struct Server {
    target: Target,
    session: Option<Session>,
    /// A última tela aberta, reaberta quando o app é recompilado.
    last: Option<Screen>,
    out_dir: Option<PathBuf>,
}

impl McpTools for Server {
    fn server_info(&self) -> (String, String) {
        ("tomni-ui".to_owned(), env!("CARGO_PKG_VERSION").to_owned())
    }

    fn instructions(&self) -> String {
        INSTRUCTIONS.to_owned()
    }

    fn tools(&self) -> Value {
        tools()
    }

    fn call(&mut self, name: &str, args: &Value, _id: &Value) -> Result<Value, McpError> {
        Ok(match self.run(name, args) {
            Ok(content) => json!({ "content": content, "isError": false }),
            Err(e) => mcp::error_result(e.message()),
        })
    }
}

impl Server {
    fn run(&mut self, name: &str, args: &Value) -> Result<Vec<Value>, Error> {
        if !matches!(
            name,
            "ui_screenshot" | "ui_tree" | "ui_interact" | "ui_lint"
        ) {
            return Err(Error::Usage(format!("ferramenta {name} desconhecida")));
        }
        if !args.is_object() {
            return Err(Error::Usage("os argumentos precisam ser um objeto".into()));
        }
        let lint_all = name == "ui_lint" && args["all"] == true;
        let mut notes = self.prepare(args, !lint_all)?;
        let result = match name {
            "ui_screenshot" => self.screenshot().map(|image| image.to_vec()),
            "ui_tree" => self.tree(args).map(|tree| vec![tree]),
            "ui_interact" => self.interact(args),
            _ => self.lint(args, lint_all),
        };
        let mut content = match result {
            Ok(content) => content,
            Err(e) => {
                // Um app que caiu não serve para a próxima chamada.
                if matches!(e, Error::Other(_)) {
                    self.session = None;
                }
                return Err(e);
            }
        };
        // No lint de todas as telas, cada uma já se descreve.
        if !lint_all && let Some(screen) = self.session.as_ref().and_then(|s| s.screen.as_ref()) {
            notes.insert(0, describe(screen));
        }
        if !notes.is_empty() {
            content.insert(0, text(&notes.join("\n")));
        }
        Ok(content)
    }

    /// Compila o app, reinicia a sessão se ele mudou e deixa a tela pedida aberta. Devolve
    /// avisos para o agente.
    fn prepare(&mut self, args: &Value, open: bool) -> Result<Vec<String>, Error> {
        let target = self.target_for(args)?;
        let exe = cargo::build(&target, true)?;
        let mut notes = Vec::new();
        if self.session.as_ref().is_some_and(|s| s.is_stale(&exe.path)) {
            self.session = None;
            notes.push(
                "o app foi recompilado: a sessão recomeçou e o estado anterior (texto digitado, \
                 cliques) se perdeu"
                    .to_owned(),
            );
        }
        let fresh = self.session.is_none();
        if fresh {
            self.session = Some(Session::start(&exe.path)?);
        }
        if !open {
            return Ok(notes);
        }
        let session = self.session.as_mut().expect("iniciada acima");
        let view = string(args, "view")?;
        let theme = string(args, "theme")?
            .map(|t| tomni_ui_cli::parse_theme(&t))
            .transpose()?;
        let width = number(args, "width")?;
        let height = number(args, "height")?;
        let reset = args["reset"] == true;

        let current = session.screen.clone().or_else(|| self.last.clone());
        let base_size = current.as_ref().map_or(session.hello.size, |s| s.size);
        let size = (width.is_some() || height.is_some())
            .then(|| (width.unwrap_or(base_size.0), height.unwrap_or(base_size.1)));
        let changed_view = view
            .as_deref()
            .is_some_and(|v| current.as_ref().is_none_or(|c| c.view != v));
        if fresh || reset || changed_view || session.screen.is_none() {
            // Tela nova, ou a mesma de antes num app recompilado.
            let view = view.or_else(|| current.as_ref().map(|c| c.view.clone()));
            let size = size.or_else(|| current.as_ref().map(|c| c.size));
            let theme = theme
                .map(str::to_owned)
                .or_else(|| current.as_ref().map(|c| c.theme.clone()));
            let result = session.open(view.as_deref(), size, theme.as_deref());
            if let Err(e) = result {
                // A tela de antes pode ter sido removida do app.
                if !changed_view && current.is_some() && matches!(e, Error::App(_)) {
                    self.last = None;
                }
                return Err(e);
            }
        } else {
            let screen = session.screen.clone().expect("aberta");
            if let Some((w, h)) = size
                && (w, h) != screen.size
            {
                session.resize(w, h)?;
            }
            if let Some(theme) = theme
                && theme != screen.theme
            {
                session.set_theme(theme)?;
            }
        }
        self.last.clone_from(&session.screen);
        Ok(notes)
    }

    fn target_for(&self, args: &Value) -> Result<Target, Error> {
        let mut target = self.target.clone();
        let bin = string(args, "bin")?;
        let example = string(args, "example")?;
        if bin.is_some() && example.is_some() {
            return Err(Error::Usage("passe `bin` ou `example`, não os dois".into()));
        }
        if bin.is_some() || example.is_some() {
            target.bin = bin;
            target.example = example;
        }
        if let Some(package) = string(args, "package")? {
            target.package = Some(package);
        }
        Ok(target)
    }

    fn session(&mut self) -> &mut Session {
        self.session.as_mut().expect("preparada antes")
    }

    fn screenshot(&mut self) -> Result<[Value; 2], Error> {
        let dir = match &self.out_dir {
            Some(dir) => dir.clone(),
            None => {
                let dir = cargo::output_dir(&cargo::metadata(&self.target)?).join("mcp");
                cargo::create_dir(&dir)?;
                self.out_dir.insert(dir).clone()
            }
        };
        let session = self.session();
        let screen = session.screen.clone().expect("aberta");
        let path = dir.join(format!("{}-{}.png", screen.view, screen.theme));
        session.screenshot(&path)?;
        let png = std::fs::read(&path)
            .map_err(|e| Error::Other(format!("não deu para ler {}: {e}", path.display())))?;
        Ok([
            json!({ "type": "image", "data": base64(&png), "mimeType": "image/png" }),
            text(&format!("PNG em {}", path.display())),
        ])
    }

    fn tree(&mut self, args: &Value) -> Result<Value, Error> {
        let json = match string(args, "format")?.as_deref() {
            None | Some("text") => false,
            Some("json") => true,
            Some(other) => {
                return Err(Error::Usage(format!(
                    "format {other:?} desconhecido: use text ou json"
                )));
            }
        };
        let tree = self.session().tree()?;
        Ok(text(&if json {
            serde_json::to_string_pretty(&tree).expect("JSON serializável")
        } else {
            format::tree(&tree)
        }))
    }

    fn interact(&mut self, args: &Value) -> Result<Vec<Value>, Error> {
        let actions = &args["actions"];
        if !actions.as_array().is_some_and(|a| !a.is_empty()) {
            return Err(Error::Usage(
                "`actions` precisa ser uma lista com pelo menos uma ação, como \
                 [{\"action\": \"click\", \"id\": \"entrar\"}]"
                    .into(),
            ));
        }
        let result = self.session().act(actions)?;
        let names = |key: &str| -> Vec<String> {
            result[key]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        };
        let repaints = names("repaints");
        let mut summary = format!(
            "{} ação(ões) feita(s). Repintados: {}",
            actions.as_array().map_or(0, Vec::len),
            if repaints.is_empty() {
                "nenhum widget".to_owned()
            } else {
                repaints.join(", ")
            }
        );
        for note in names("notes") {
            summary.push_str("\nnota: ");
            summary.push_str(&note);
        }
        let [image, path] = self.screenshot()?;
        let tree = self.tree(&json!({}))?;
        Ok(vec![text(&summary), image, path, tree])
    }

    fn lint(&mut self, args: &Value, all: bool) -> Result<Vec<Value>, Error> {
        let allow: Vec<String> = match &args["allow"] {
            Value::Null => Vec::new(),
            Value::Array(rules) => rules
                .iter()
                .map(|r| {
                    r.as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| Error::Usage("`allow` é uma lista de nomes".into()))
                })
                .collect::<Result<_, _>>()?,
            _ => return Err(Error::Usage("`allow` é uma lista de nomes".into())),
        };
        tomni_ui_cli::check_rules(&allow)?;
        let mut screens = Vec::new();
        if all {
            let size = match (number(args, "width")?, number(args, "height")?) {
                (Some(w), Some(h)) => Some((w, h)),
                (None, None) => None,
                _ => return Err(Error::Usage("passe `width` e `height` juntos".into())),
            };
            let session = self.session();
            for view in session.hello.views.clone() {
                for theme in ["light", "dark"] {
                    session.open(Some(&view), size, Some(theme))?;
                    let issues = session.lint(&allow)?;
                    screens.push((session.screen.clone().expect("aberta"), issues));
                }
            }
            self.last = self.session.as_ref().and_then(|s| s.screen.clone());
        } else {
            let session = self.session();
            let issues = session.lint(&allow)?;
            screens.push((session.screen.clone().expect("aberta"), issues));
        }
        let (mut errors, mut warnings) = (0, 0);
        let mut lines = Vec::new();
        for (screen, issues) in &screens {
            let e = issues.iter().filter(|i| format::is_error(i)).count();
            errors += e;
            warnings += issues.len() - e;
            if issues.is_empty() {
                lines.push(format!("{}: ok", describe(screen)));
            } else {
                lines.push(format!("{}:", describe(screen)));
                lines.extend(issues.iter().map(|i| format!("  {}", format::issue(i))));
            }
        }
        lines.push(format!("{errors} erro(s), {warnings} aviso(s)"));
        if all {
            lines.push("As telas foram reabertas: o estado anterior se perdeu.".into());
        }
        Ok(vec![text(&lines.join("\n"))])
    }
}

fn describe(screen: &Screen) -> String {
    let (w, h) = screen.size;
    format!(
        "tela {} (tema {}, {w}×{h})",
        screen.view,
        format::theme(&screen.theme)
    )
}

fn text(text: &str) -> Value {
    json!({ "type": "text", "text": text })
}

fn string(args: &Value, key: &str) -> Result<Option<String>, Error> {
    match &args[key] {
        Value::Null => Ok(None),
        Value::String(s) => Ok(Some(s.clone())),
        _ => Err(Error::Usage(format!("`{key}` precisa ser texto"))),
    }
}

fn number(args: &Value, key: &str) -> Result<Option<u32>, Error> {
    match &args[key] {
        Value::Null => Ok(None),
        value => value
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .filter(|v| *v > 0)
            .map(Some)
            .ok_or_else(|| Error::Usage(format!("`{key}` precisa ser um inteiro positivo"))),
    }
}

/// Base64 padrão (RFC 4648), com `=`.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[(n >> (18 - 6 * i) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Esquemas das ferramentas.
fn tools() -> Value {
    let screen = json!({
        "view": { "type": "string", "description": "Tela registrada no App (nome da função). Padrão: a atual, ou a do App::new." },
        "width": { "type": "integer", "minimum": 1, "description": "Largura em pixels lógicos." },
        "height": { "type": "integer", "minimum": 1, "description": "Altura em pixels lógicos." },
        "theme": { "type": "string", "enum": ["light", "dark"], "description": "Tema." },
        "reset": { "type": "boolean", "description": "Reabre a tela do zero, sem o estado das interações." },
        "bin": { "type": "string", "description": "Executável do pacote (cargo --bin)." },
        "example": { "type": "string", "description": "Exemplo do pacote (cargo --example)." },
        "package": { "type": "string", "description": "Pacote do workspace (cargo -p)." },
    });
    let with = |extra: Value, required: &[&str]| {
        let mut properties = screen.as_object().expect("objeto").clone();
        properties.extend(extra.as_object().expect("objeto").clone());
        json!({ "type": "object", "properties": properties, "required": required })
    };
    json!([
        {
            "name": "ui_screenshot",
            "title": "Screenshot da tela",
            "description": "Compila o app e devolve um PNG da tela, com o tamanho e o tema pedidos. Mantém o estado das interações anteriores.",
            "inputSchema": with(json!({}), &[]),
        },
        {
            "name": "ui_tree",
            "title": "Árvore de widgets",
            "description": "Árvore de widgets da tela: tipo, id, retângulo (x, y, largura×altura), texto e estado (rest, hover, pressed, focused, disabled). Num scroll_viewer, a rolagem e o tamanho do conteúdo; um widget rolado para fora da área visível vem marcado. Popups (content_dialog, flyout, menu_flyout, combo_box_dropdown, tooltip) aparecem dentro do widget que os abre, como open ou closed (sem os filhos); o item escolhido de um combo_box vem como selected.",
            "inputSchema": with(json!({
                "format": { "type": "string", "enum": ["text", "json"], "description": "Texto indentado (padrão) ou JSON." }
            }), &[]),
        },
        {
            "name": "ui_interact",
            "title": "Interagir com a tela",
            "description": "Roda ações na tela, em ordem, e devolve os widgets repintados, um screenshot e a árvore. Ações: {\"action\":\"click\",\"id\":\"entrar\"}, double_click, middle_click (fecha uma aba do TabView) e hover (id), {\"action\":\"click_at\",\"x\":10,\"y\":20}, {\"action\":\"drag\",\"id\":\"logs-app\",\"to\":\"logs-erro\"} (arrasta com o mouse do centro de id até o centro de to, ou até o ponto x, y: o divisor do Split, o Slider, as abas de um TabView com on_reorder), {\"action\":\"context\",\"id\":\"linha\"} (abre o menu de contexto, o context_flyout, com o botão direito), {\"action\":\"type\",\"id\":\"email\",\"text\":\"ana@x.com\"} (sem id, digita no foco; num Terminal, escreve na conexão: \\n é o Enter, e o eco que chegou entra na resposta; para um processo, espere com wait), {\"action\":\"key\",\"key\":\"enter\",\"modifiers\":[\"shift\"]} (teclas: enter, space, tab, escape, backspace, delete, left, right, up, down, home, end, pageup, pagedown, f1 a f12 ou uma letra; modificadores: shift, ctrl, alt, meta, cmd), {\"action\":\"ime_preedit\",\"text\":\"か\"}, {\"action\":\"ime_commit\",\"text\":\"漢字\"} e {\"action\":\"scroll\",\"id\":\"lista\",\"dy\":200} (rola um ScrollViewer; dy positivo desce), {\"action\":\"select\",\"id\":\"uf\",\"option\":\"SP\"} (abre o ComboBox e clica na opção; num RadioButtons, clica na opção; num TabView, na aba com esse título; num NavigationView, no item com esse rótulo, abrindo antes o painel ou o grupo), {\"action\":\"set_value\",\"id\":\"volume\",\"value\":70} (clica no trilho do Slider, no ponto do valor; num NumberBox, digita o número e confirma com Enter) e {\"action\":\"wait\",\"ms\":500} (passa o tempo: a dica de um hover abre em 500 ms). click, type e hover rolam sozinhos até o widget. Com um popup aberto, só ele recebe o clique: um clique fora fecha o flyout, o menu ou a lista do ComboBox, e não chega a nada atrás.",
            "inputSchema": with(json!({
                "actions": {
                    "type": "array",
                    "minItems": 1,
                    "description": "Ações, em ordem.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "action": { "type": "string", "enum": ["click", "double_click", "middle_click", "hover", "click_at", "drag", "context", "type", "key", "ime_preedit", "ime_commit", "scroll", "select", "set_value", "wait"] },
                            "id": { "type": "string" },
                            "text": { "type": "string" },
                            "to": { "type": "string", "description": "drag: id do widget onde soltar." },
                            "x": { "type": "number" },
                            "y": { "type": "number" },
                            "dx": { "type": "number", "description": "scroll: pixels na horizontal." },
                            "dy": { "type": "number", "description": "scroll: pixels na vertical." },
                            "key": { "type": "string" },
                            "option": { "type": "string", "description": "select: a opção, como aparece na lista, ou o título da aba." },
                            "value": { "type": "number", "description": "set_value: o valor do Slider ou do NumberBox." },
                            "ms": { "type": "integer", "minimum": 0, "description": "wait: milissegundos." },
                            "modifiers": { "type": "array", "items": { "type": "string", "enum": ["shift", "ctrl", "alt", "meta", "cmd"] } }
                        },
                        "required": ["action"]
                    }
                }
            }), &["actions"]),
        },
        {
            "name": "ui_lint",
            "title": "Lint da tela",
            "description": "Confere contraste AA, alvo de toque de 44 px, texto cortado, widgets fora da janela e widgets interativos sem id (aviso). Cada problema diz o widget e como corrigir. A tarefa só está pronta com zero erros.",
            "inputSchema": with(json!({
                "allow": { "type": "array", "items": { "type": "string", "enum": RULES }, "description": "Regras ignoradas." },
                "all": { "type": "boolean", "description": "Todas as telas, nos temas claro e escuro (reabre as telas)." }
            }), &[]),
        },
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_da_rfc() {
        for (entrada, saida) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(entrada.as_bytes()), saida);
        }
    }

    #[test]
    fn json_rpc_sem_app() {
        let mut server = Server {
            target: Target::default(),
            session: None,
            last: None,
            out_dir: None,
        };
        let init = mcp::handle(&mut server, &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26"}}))
            .unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(
            mcp::handle(
                &mut server,
                &json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
            ),
            None
        );
        let list = mcp::handle(
            &mut server,
            &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        )
        .unwrap();
        let names: Vec<&str> = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            ["ui_screenshot", "ui_tree", "ui_interact", "ui_lint"]
        );
        let erro = mcp::handle(
            &mut server,
            &json!({"jsonrpc": "2.0", "id": 3, "method": "x"}),
        )
        .unwrap();
        assert_eq!(erro["error"]["code"], -32601);
        let call = mcp::handle(&mut server, &json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "ui_nada"}}))
            .unwrap();
        assert_eq!(call["result"]["isError"], true);
    }
}
