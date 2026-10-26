//! Cliente das ferramentas para agentes, compartilhado pela CLI `tomni-ui` e pelo servidor
//! MCP `tomni-ui-mcp`.
//!
//! O app do usuário é compilado com o `cargo` ([`cargo::build`]) e roda em modo de ferramentas
//! (`TOMNI_UI_TOOLS=1`): em vez de abrir a janela, atende pedidos em JSON pela entrada padrão
//! ([`Session`]). Assim a tela é a do app de verdade, com os signals e handlers do usuário,
//! rasterizada no backend CPU determinístico.

use std::fmt;

pub mod cargo;
pub mod dist;
pub mod format;
pub mod init;
mod session;

pub use session::{Hello, Screen, Session};

/// Versão do protocolo com o app (`tomni_ui::tools::PROTOCOL`).
pub const PROTOCOL: u64 = 19;

/// Variável de ambiente que põe o app em modo de ferramentas (`tomni_ui::tools::ENV`).
pub const TOOLS_ENV: &str = "TOMNI_UI_TOOLS";

/// Começo das linhas de resposta do app (`tomni_ui::tools::PREFIX`).
pub const PREFIX: &str = "@tomni-ui ";

/// Regras do lint (`tomni_ui::lint::Rule`), para validar `--allow`.
pub const RULES: [&str; 10] = [
    "contrast",
    "touch-target",
    "clipped-text",
    "offscreen",
    "missing-id",
    "missing-glyph",
    "missing-label",
    "missing-alt",
    "webview-in-scroll",
    "duplicate-accelerator",
];

/// Erro das ferramentas, com a mensagem pronta para mostrar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// O `cargo` falhou: a compilação, com os diagnósticos quando capturados.
    Build(String),
    /// O app recusou o pedido (tela ou id que não existe, tamanho inválido).
    App(String),
    /// Uso errado da CLI ou dos parâmetros do MCP.
    Usage(String),
    /// Processo, entrada e saída ou protocolo.
    Other(String),
}

impl Error {
    /// Mensagem, sem o tipo.
    pub fn message(&self) -> &str {
        match self {
            Self::Build(m) | Self::App(m) | Self::Usage(m) | Self::Other(m) => m,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for Error {}

/// Tema pelo nome da CLI e do MCP: `light` ou `dark`.
pub fn parse_theme(name: &str) -> Result<&'static str, Error> {
    match name {
        "light" | "claro" => Ok("light"),
        "dark" | "escuro" => Ok("dark"),
        _ => Err(Error::Usage(format!(
            "tema {name:?} desconhecido: use light ou dark"
        ))),
    }
}

/// Tamanho no formato `390x844`.
pub fn parse_size(text: &str) -> Result<(u32, u32), Error> {
    let parsed = text
        .split_once(['x', 'X', '×'])
        .and_then(|(w, h)| Some((w.trim().parse().ok()?, h.trim().parse().ok()?)));
    match parsed {
        Some((w, h)) if w > 0 && h > 0 => Ok((w, h)),
        _ => Err(Error::Usage(format!(
            "tamanho {text:?} inválido: use LARGURAxALTURA, como 390x844"
        ))),
    }
}

/// Confere os nomes de regras de `--allow`.
pub fn check_rules(rules: &[String]) -> Result<(), Error> {
    for rule in rules {
        if !RULES.contains(&rule.as_str()) {
            return Err(Error::Usage(format!(
                "regra {rule:?} desconhecida; regras: {}",
                RULES.join(", ")
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tamanhos_e_temas() {
        assert_eq!(parse_size("390x844"), Ok((390, 844)));
        assert_eq!(parse_size("390×844"), Ok((390, 844)));
        assert!(parse_size("0x10").is_err());
        assert!(parse_size("390").is_err());
        assert_eq!(parse_theme("escuro"), Ok("dark"));
        assert!(parse_theme("azul").is_err());
    }

    #[test]
    fn constantes_batem_com_a_lib() {
        assert_eq!(PROTOCOL, u64::from(tomni_ui::tools::PROTOCOL));
        assert_eq!(TOOLS_ENV, tomni_ui::tools::ENV);
        assert_eq!(PREFIX, tomni_ui::tools::PREFIX);
        let rules: Vec<&str> = tomni_ui::lint::Rule::ALL.iter().map(|r| r.name()).collect();
        assert_eq!(RULES.as_slice(), rules);
    }

    #[test]
    fn guia_lista_todos_os_icones() {
        let guia = include_str!("../templates/AGENTS.md");
        let (_, resto) = guia
            .split_once("<!-- ícones: início")
            .expect("o guia tem o marcador de início da lista de ícones");
        let (lista, _) = resto
            .split_once("<!-- ícones: fim -->")
            .expect("o guia tem o marcador de fim da lista de ícones");
        let nomes: Vec<&str> = lista.split('`').skip(1).step_by(2).collect();
        let symbols: Vec<&str> = tomni_ui::prelude::Symbol::ALL
            .iter()
            .map(|s| s.name())
            .collect();
        assert_eq!(
            nomes, symbols,
            "rode `python3 -I crates/theme/icons/gerar.py CACHE` para refazer a lista"
        );
    }

    #[test]
    fn templates_do_cargo_about_iguais_aos_da_raiz() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for (name, embedded) in [
            ("about.hbs", dist::ABOUT_TEMPLATE),
            ("about.toml", dist::ABOUT_CONFIG),
        ] {
            let original = std::fs::read_to_string(root.join(name)).unwrap();
            assert_eq!(
                embedded, original,
                "crates/cli/templates/{name} difere do {name} da raiz: copie de novo"
            );
        }
    }
}
