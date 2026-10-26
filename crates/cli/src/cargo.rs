//! Chamadas ao `cargo`: compilar o app e achar o executável, e o `cargo metadata`.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

use crate::Error;

/// O que compilar: as opções do `cargo` que a CLI e o MCP repassam.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Target {
    /// `--manifest-path`.
    pub manifest_path: Option<PathBuf>,
    /// `-p`/`--package`.
    pub package: Option<String>,
    /// `--bin`.
    pub bin: Option<String>,
    /// `--example`.
    pub example: Option<String>,
    /// `--features`, uma por item ou separadas por vírgula.
    pub features: Vec<String>,
    /// `--no-default-features`.
    pub no_default_features: bool,
    /// `--release`.
    pub release: bool,
    /// `--target`: o triple, na compilação cruzada do `tomni-ui build`.
    pub triple: Option<String>,
}

impl Target {
    /// Opções para o `cargo build` e o `cargo run`.
    pub fn args(&self) -> Vec<String> {
        let mut args = Vec::new();
        let mut push = |flag: &str, value: Option<String>| {
            if let Some(value) = value {
                args.push(flag.to_owned());
                args.push(value);
            }
        };
        let path = self
            .manifest_path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned());
        push("--manifest-path", path);
        push("--package", self.package.clone());
        push("--bin", self.bin.clone());
        push("--example", self.example.clone());
        push("--target", self.triple.clone());
        if !self.features.is_empty() {
            args.push("--features".into());
            args.push(self.features.join(","));
        }
        if self.no_default_features {
            args.push("--no-default-features".into());
        }
        if self.release {
            args.push("--release".into());
        }
        args
    }

    /// Opções que valem para o `cargo metadata` e o `cargo about`.
    fn manifest_args(&self) -> Vec<String> {
        match &self.manifest_path {
            Some(path) => vec![
                "--manifest-path".into(),
                path.to_string_lossy().into_owned(),
            ],
            None => Vec::new(),
        }
    }

    /// Aplica uma opção do `cargo` vinda da linha de comando. Devolve `false` se `flag` não é
    /// do `cargo`; `value` só é consumido nas opções com valor.
    pub fn apply(
        &mut self,
        flag: &str,
        value: impl FnOnce() -> Result<String, Error>,
    ) -> Result<bool, Error> {
        match flag {
            "--manifest-path" => self.manifest_path = Some(value()?.into()),
            "-p" | "--package" => self.package = Some(value()?),
            "--bin" => self.bin = Some(value()?),
            "--example" => self.example = Some(value()?),
            "-F" | "--features" => self.features.push(value()?),
            "--no-default-features" => self.no_default_features = true,
            "--release" => self.release = true,
            _ => return Ok(false),
        }
        Ok(true)
    }
}

/// O `cargo` que rodou a CLI, se houver; senão o do `PATH`.
pub fn cargo() -> Command {
    Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
}

/// Um executável produzido pelo `cargo build`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Executable {
    /// Nome do binário ou do exemplo.
    pub name: String,
    /// `bin` ou `example`.
    pub kind: String,
    /// Caminho do arquivo.
    pub path: PathBuf,
}

/// Compila o app e devolve o executável.
///
/// Com `capture`, os diagnósticos do compilador voltam no [`Error::Build`] (o MCP); sem, saem
/// na saída de erro, com cores (a CLI). Sem `--bin` nem `--example`, o pacote precisa ter um
/// executável só.
pub fn build(target: &Target, capture: bool) -> Result<Executable, Error> {
    let mut command = cargo();
    command
        .arg("build")
        .arg("--message-format=json-render-diagnostics")
        .args(target.args())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(if capture {
            Stdio::piped()
        } else {
            Stdio::inherit()
        });
    let mut child = command
        .spawn()
        .map_err(|e| Error::Other(format!("não deu para rodar o cargo: {e}")))?;
    let stderr = child.stderr.take().map(|mut pipe| {
        std::thread::spawn(move || {
            let mut text = String::new();
            let _ = pipe.read_to_string(&mut text);
            text
        })
    });
    let mut executables: Vec<Executable> = Vec::new();
    let stdout = child.stdout.take().expect("stdout em pipe");
    for line in BufReader::new(stdout).lines() {
        let Ok(line) = line else { break };
        if let Some(exe) = executable(&line)
            && !executables.contains(&exe)
        {
            executables.push(exe);
        }
    }
    let status = child
        .wait()
        .map_err(|e| Error::Other(format!("falha esperando o cargo: {e}")))?;
    let diagnostics = stderr
        .and_then(|thread| thread.join().ok())
        .map(|text| compiler_output(&text));
    if !status.success() {
        return Err(Error::Build(match diagnostics {
            Some(text) if !text.is_empty() => format!("a compilação falhou:\n{text}"),
            _ => "a compilação falhou: veja os erros do cargo acima".into(),
        }));
    }
    choose(executables)
}

/// Executável numa linha do `--message-format=json`.
fn executable(line: &str) -> Option<Executable> {
    let message: Value = serde_json::from_str(line).ok()?;
    if message["reason"] != "compiler-artifact" {
        return None;
    }
    let path = message["executable"].as_str()?;
    let kinds = message["target"]["kind"].as_array()?;
    let kind = ["bin", "example"]
        .into_iter()
        .find(|k| kinds.iter().any(|v| v == k))?;
    // Um build script também é executável, mas é `custom-build`.
    Some(Executable {
        name: message["target"]["name"].as_str()?.to_owned(),
        kind: kind.to_owned(),
        path: path.into(),
    })
}

fn choose(mut executables: Vec<Executable>) -> Result<Executable, Error> {
    match executables.len() {
        1 => Ok(executables.remove(0)),
        0 => Err(Error::Usage(
            "o cargo não produziu executável nenhum: o pacote precisa de um `src/main.rs` (ou \
             `[[bin]]`); para um exemplo, use --example NOME"
                .into(),
        )),
        _ => {
            let names: Vec<String> = executables
                .iter()
                .map(|e| format!("--{} {}", e.kind, e.name))
                .collect();
            Err(Error::Usage(format!(
                "o cargo produziu {} executáveis; escolha um: {}",
                names.len(),
                names.join(", ")
            )))
        }
    }
}

/// Saída do `cargo` sem as linhas de progresso.
fn compiler_output(text: &str) -> String {
    const PROGRESS: [&str; 8] = [
        "Compiling",
        "Checking",
        "Finished",
        "Blocking",
        "Downloading",
        "Downloaded",
        "Updating",
        "Locking",
    ];
    text.lines()
        .filter(|line| {
            let word = line.trim_start().split(' ').next().unwrap_or("");
            !PROGRESS.contains(&word)
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

/// O que interessa do `cargo metadata`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Metadata {
    /// Pasta `target` do workspace.
    pub target_directory: PathBuf,
    /// Raiz do workspace.
    pub workspace_root: PathBuf,
}

/// Roda o `cargo metadata` do projeto de `target`.
pub fn metadata(target: &Target) -> Result<Metadata, Error> {
    let output = cargo()
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .args(target.manifest_args())
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| Error::Other(format!("não deu para rodar o cargo: {e}")))?;
    if !output.status.success() {
        return Err(Error::Build(format!(
            "o cargo metadata falhou (a pasta tem um Cargo.toml?):\n{}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| Error::Other(format!("saída inesperada do cargo metadata: {e}")))?;
    let path = |key: &str| {
        value[key]
            .as_str()
            .map(PathBuf::from)
            .ok_or_else(|| Error::Other(format!("o cargo metadata não informou {key}")))
    };
    Ok(Metadata {
        target_directory: path("target_directory")?,
        workspace_root: path("workspace_root")?,
    })
}

/// Pasta das saídas das ferramentas: `target/tomni-ui`.
pub fn output_dir(metadata: &Metadata) -> PathBuf {
    metadata.target_directory.join("tomni-ui")
}

/// Cria a pasta `dir`, com mensagem de erro boa.
pub fn create_dir(dir: &Path) -> Result<(), Error> {
    std::fs::create_dir_all(dir)
        .map_err(|e| Error::Other(format!("não deu para criar {}: {e}", dir.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opcoes_do_cargo() {
        let mut target = Target::default();
        assert!(target.apply("--example", || Ok("login".into())).unwrap());
        assert!(target.apply("-F", || Ok("cjk".into())).unwrap());
        assert!(target.apply("--release", || unreachable!()).unwrap());
        assert!(!target.apply("--view", || unreachable!()).unwrap());
        assert_eq!(
            target.args(),
            ["--example", "login", "--features", "cjk", "--release"]
        );
    }

    #[test]
    fn executaveis_na_saida_json() {
        let bin = r#"{"reason":"compiler-artifact","target":{"kind":["bin"],"name":"app"},"executable":"/t/app"}"#;
        let lib = r#"{"reason":"compiler-artifact","target":{"kind":["lib"],"name":"x"},"executable":null}"#;
        let script = r#"{"reason":"compiler-artifact","target":{"kind":["custom-build"],"name":"build-script-build"},"executable":"/t/b"}"#;
        assert_eq!(
            executable(bin),
            Some(Executable {
                name: "app".into(),
                kind: "bin".into(),
                path: "/t/app".into()
            })
        );
        assert_eq!(executable(lib), None);
        assert_eq!(executable(script), None);
        assert_eq!(executable("Compiling"), None);

        let exe = |name: &str| Executable {
            name: name.into(),
            kind: "bin".into(),
            path: name.into(),
        };
        assert_eq!(choose(vec![exe("a")]).unwrap().name, "a");
        let erro = choose(vec![exe("a"), exe("b")]).unwrap_err();
        assert!(erro.message().contains("--bin a, --bin b"), "{erro}");
        assert!(choose(Vec::new()).is_err());
    }

    #[test]
    fn diagnosticos_sem_progresso() {
        let text = "   Compiling app v0.1.0\nerror[E0425]: x\n  --> src/main.rs:1:1\n";
        assert_eq!(
            compiler_output(text),
            "error[E0425]: x\n  --> src/main.rs:1:1"
        );
    }
}
