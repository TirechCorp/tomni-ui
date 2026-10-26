//! `tomni-ui init`: grava no projeto o guia para agentes, a skill e a configuração do MCP.
//!
//! Pode rodar de novo para atualizar: o trecho do Tomni UI no `AGENTS.md` fica entre
//! marcadores e é trocado; o resto do arquivo, o `CLAUDE.md` e o `.mcp.json` do usuário são
//! preservados.

use std::fmt;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::Error;

/// Guia da API e do fluxo de trabalho, para o `AGENTS.md`.
pub const AGENTS: &str = include_str!("../templates/AGENTS.md");

/// `CLAUDE.md`: importa o `AGENTS.md`.
pub const CLAUDE: &str = include_str!("../templates/CLAUDE.md");

/// Skill do Claude Code, em `.claude/skills/tomni-ui/SKILL.md`.
pub const SKILL: &str = include_str!("../templates/SKILL.md");

/// `.mcp.json` com o servidor `tomni-ui-mcp`.
pub const MCP: &str = include_str!("../templates/mcp.json");

const BEGIN: &str = "<!-- tomni-ui:begin";
const END: &str = "<!-- tomni-ui:end -->";

/// O que aconteceu com um arquivo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// Arquivo novo.
    Created,
    /// Arquivo existente, atualizado.
    Updated,
    /// Já estava em dia.
    Unchanged,
}

impl fmt::Display for Change {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Created => "criado",
            Self::Updated => "atualizado",
            Self::Unchanged => "sem mudança",
        })
    }
}

/// Conteúdo novo de um arquivo a partir do atual, se houver.
type Update = fn(Option<&str>) -> Result<String, Error>;

/// Grava os arquivos em `dir` (a raiz do projeto) e diz o que fez com cada um.
pub fn init(dir: &Path) -> Result<Vec<(PathBuf, Change)>, Error> {
    let files: [(&str, Update); 4] = [
        ("AGENTS.md", |old| Ok(agents(old))),
        ("CLAUDE.md", |old| Ok(claude(old))),
        (".claude/skills/tomni-ui/SKILL.md", |_| Ok(SKILL.to_owned())),
        (".mcp.json", mcp),
    ];
    let mut changes = Vec::new();
    for (name, update) in files {
        let path = dir.join(name);
        let old = match std::fs::read_to_string(&path) {
            Ok(text) => Some(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(Error::Other(format!("não deu para ler {name}: {e}"))),
        };
        let new = update(old.as_deref()).map_err(|e| Error::Other(format!("{name}: {e}")))?;
        let change = match &old {
            None => Change::Created,
            Some(old) if *old == new => Change::Unchanged,
            Some(_) => Change::Updated,
        };
        if change != Change::Unchanged {
            if let Some(parent) = path.parent() {
                crate::cargo::create_dir(parent)?;
            }
            std::fs::write(&path, new)
                .map_err(|e| Error::Other(format!("não deu para gravar {name}: {e}")))?;
        }
        changes.push((PathBuf::from(name), change));
    }
    Ok(changes)
}

/// `AGENTS.md` com o trecho do Tomni UI: troca o antigo ou acrescenta no fim.
fn agents(old: Option<&str>) -> String {
    let Some(old) = old else {
        return AGENTS.to_owned();
    };
    if let Some(start) = old.find(BEGIN)
        && let Some(end) = old[start..].find(END)
    {
        let end = start + end + END.len();
        let end = end + usize::from(old[end..].starts_with('\n'));
        return format!("{}{AGENTS}{}", &old[..start], &old[end..]);
    }
    let separator = if old.ends_with("\n\n") || old.is_empty() {
        ""
    } else if old.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    };
    format!("{old}{separator}{AGENTS}")
}

/// `CLAUDE.md` que importa o `AGENTS.md`.
fn claude(old: Option<&str>) -> String {
    match old {
        None => CLAUDE.to_owned(),
        Some(old) if old.lines().any(|line| line.trim() == "@AGENTS.md") => old.to_owned(),
        Some(old) => {
            let separator = if old.ends_with('\n') || old.is_empty() {
                ""
            } else {
                "\n"
            };
            format!("{old}{separator}\n@AGENTS.md\n")
        }
    }
}

/// `.mcp.json` com o servidor `tomni-ui`, sem mexer nos outros.
fn mcp(old: Option<&str>) -> Result<String, Error> {
    let Some(old) = old else {
        return Ok(MCP.to_owned());
    };
    let mut config: Value = serde_json::from_str(old)
        .map_err(|e| Error::Other(format!("JSON inválido, corrija antes ({e})")))?;
    let template: Value = serde_json::from_str(MCP).expect("template válido");
    let servers = config
        .as_object_mut()
        .ok_or_else(|| Error::Other("esperava um objeto JSON".into()))?
        .entry("mcpServers")
        .or_insert_with(|| json!({}));
    let servers = servers
        .as_object_mut()
        .ok_or_else(|| Error::Other("`mcpServers` não é um objeto".into()))?;
    if servers.contains_key("tomni-ui") {
        // Pode ter argumentos do usuário (`--example`): fica como está.
        return Ok(old.to_owned());
    }
    servers.insert(
        "tomni-ui".into(),
        template["mcpServers"]["tomni-ui"].clone(),
    );
    let mut text = serde_json::to_string_pretty(&config).expect("JSON serializável");
    text.push('\n');
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agents_troca_o_trecho_e_preserva_o_resto() {
        assert_eq!(agents(None), AGENTS);
        let proprio = "# Meu app\n\nRegras do time.\n";
        let com = agents(Some(proprio));
        assert!(com.starts_with(proprio) && com.ends_with(AGENTS), "{com}");
        let antigo = format!("# Meu app\n\n{BEGIN} velho -->\nx\n{END}\nDepois.\n");
        assert_eq!(
            agents(Some(&antigo)),
            format!("# Meu app\n\n{AGENTS}Depois.\n")
        );
        // Rodar de novo não muda nada.
        assert_eq!(agents(Some(&com)), com);
    }

    #[test]
    fn claude_importa_o_agents() {
        assert_eq!(claude(None), "@AGENTS.md\n");
        assert_eq!(claude(Some("Use pt-BR.")), "Use pt-BR.\n\n@AGENTS.md\n");
        assert_eq!(claude(Some("@AGENTS.md\n")), "@AGENTS.md\n");
    }

    #[test]
    fn mcp_acrescenta_o_servidor() {
        let outro = r#"{"mcpServers": {"db": {"command": "db-mcp"}}}"#;
        let novo: Value = serde_json::from_str(&mcp(Some(outro)).unwrap()).unwrap();
        assert_eq!(novo["mcpServers"]["db"]["command"], "db-mcp");
        assert_eq!(novo["mcpServers"]["tomni-ui"]["command"], "tomni-ui-mcp");
        let configurado =
            r#"{"mcpServers": {"tomni-ui": {"command": "x", "args": ["--example", "a"]}}}"#;
        assert_eq!(mcp(Some(configurado)).unwrap(), configurado);
        assert!(mcp(Some("{")).is_err());
    }

    #[test]
    fn grava_e_atualiza() {
        let dir = std::env::temp_dir().join(format!("tomni-ui-init-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let primeira = init(&dir).unwrap();
        assert!(primeira.iter().all(|(_, c)| *c == Change::Created));
        let segunda = init(&dir).unwrap();
        assert!(segunda.iter().all(|(_, c)| *c == Change::Unchanged));
        assert!(dir.join(".claude/skills/tomni-ui/SKILL.md").is_file());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
