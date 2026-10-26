//! CLI para agentes: `screenshot`, `tree`, `lint`, `run`, `build` e `init`.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::{Value, json};
use tomni_ui_cli::cargo::{self, Target};
use tomni_ui_cli::{Error, Session, dist, format, init};

const HELP: &str = "\
tomni-ui: ferramentas do Tomni UI para agentes e gente

Uso: tomni-ui <comando> [opções]

Comandos:
  screenshot   PNG da tela (o caminho sai na saída padrão)
  tree         árvore de widgets com id, tipo, retângulo e estado
  lint         contraste AA, alvo de toque, texto cortado e widgets fora da janela
  run          abre o app numa janela (cargo run)
  build        binário de release (ou APK) com o THIRD-PARTY-NOTICES
  init         grava AGENTS.md, CLAUDE.md, a skill e o .mcp.json no projeto

Tela (screenshot, tree e lint):
  --view NOME          tela registrada no App (padrão: a do App::new)
  --size LxA           tamanho, como 390x844 (padrão: o do App::size)
  --theme light|dark   tema (padrão: o do App ou claro)

screenshot:  -o, --out ARQUIVO   onde gravar (padrão: target/tomni-ui/<tela>-<tema>.png)
tree:        --json              a árvore em JSON
lint:        --allow REGRA       ignora uma regra (repetível); --json para JSON
             Sem --view, confere todas as telas nos temas claro e escuro.
             Regras: contrast, touch-target, clipped-text, offscreen, missing-glyph,
             missing-alt, webview-in-scroll, duplicate-accelerator, missing-id e
             missing-label (avisos).
build:       --target windows|linux|macos|android   (obrigatório)
             --triple TRIPLE     triple do Rust na compilação cruzada
init:        [PASTA]             raiz do projeto (padrão: a do workspace atual)

Opções do cargo (todos os comandos):
  -p, --package P, --bin NOME, --example NOME, -F, --features F,
  --no-default-features, --manifest-path CAMINHO, --release

O app é compilado e roda com TOMNI_UI_TOOLS=1, sem janela: o `main` precisa terminar em
`tomni_ui::App::new(tela).run()`. Saída: 0 ok, 1 falha (ou erros no lint), 2 uso errado.
";

fn main() -> ExitCode {
    let args: VecDeque<String> = std::env::args().skip(1).collect();
    match run(args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("erro: {e}");
            if matches!(e, Error::Usage(_)) {
                eprintln!("Veja `tomni-ui --help`.");
                ExitCode::from(2)
            } else {
                ExitCode::FAILURE
            }
        }
    }
}

/// Opções já separadas.
#[derive(Debug, Default)]
struct Options {
    target: Target,
    view: Option<String>,
    size: Option<(u32, u32)>,
    theme: Option<&'static str>,
    out: Option<PathBuf>,
    json: bool,
    allow: Vec<String>,
    os: Option<String>,
    positional: Vec<String>,
    /// Depois de `--`: argumentos do app no `run`.
    rest: Vec<String>,
}

fn run(mut args: VecDeque<String>) -> Result<ExitCode, Error> {
    let Some(command) = args.pop_front() else {
        print!("{HELP}");
        return Ok(ExitCode::from(2));
    };
    match command.as_str() {
        "-h" | "--help" | "help" => {
            print!("{HELP}");
            return Ok(ExitCode::SUCCESS);
        }
        "-V" | "--version" => {
            println!("tomni-ui {}", env!("CARGO_PKG_VERSION"));
            return Ok(ExitCode::SUCCESS);
        }
        _ => {}
    }
    let options = parse(&command, args)?;
    match command.as_str() {
        "screenshot" => screenshot(options),
        "tree" => tree(options),
        "lint" => lint(options),
        "run" => run_app(options),
        "build" => build(options),
        "init" => init(options),
        _ => Err(Error::Usage(format!("comando {command:?} desconhecido"))),
    }
}

fn parse(command: &str, mut args: VecDeque<String>) -> Result<Options, Error> {
    let mut options = Options::default();
    while let Some(arg) = args.pop_front() {
        if arg == "--" {
            options.rest = args.drain(..).collect();
            break;
        }
        if !arg.starts_with('-') || arg == "-" {
            options.positional.push(arg);
            continue;
        }
        // `--opcao=valor` vira `--opcao valor`.
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => {
                (flag.to_owned(), Some(value.to_owned()))
            }
            _ => (arg, None),
        };
        let mut inline = inline;
        let mut value = || -> Result<String, Error> {
            inline
                .take()
                .or_else(|| args.pop_front())
                .ok_or_else(|| Error::Usage(format!("{flag} precisa de um valor")))
        };
        if options.target.apply(&flag, &mut value)? {
            continue;
        }
        let allowed: &[&str] = match command {
            "screenshot" => &["--view", "--size", "--theme", "-o", "--out"],
            "tree" => &["--view", "--size", "--theme", "--json"],
            "lint" => &["--view", "--size", "--theme", "--json", "--allow"],
            "build" => &["--target", "--triple"],
            _ => &[],
        };
        if !allowed.contains(&flag.as_str()) {
            return Err(Error::Usage(format!(
                "opção {flag} desconhecida no `{command}`"
            )));
        }
        match flag.as_str() {
            "--view" => options.view = Some(value()?),
            "--size" => options.size = Some(tomni_ui_cli::parse_size(&value()?)?),
            "--theme" => options.theme = Some(tomni_ui_cli::parse_theme(&value()?)?),
            "-o" | "--out" => options.out = Some(value()?.into()),
            "--json" => options.json = true,
            "--allow" => options.allow.push(value()?),
            "--target" => options.os = Some(value()?),
            "--triple" => options.target.triple = Some(value()?),
            _ => unreachable!("lista de opções acima"),
        }
        if inline.is_some() {
            return Err(Error::Usage(format!("{flag} não recebe valor")));
        }
    }
    let positional_ok = match command {
        "init" => options.positional.len() <= 1,
        _ => options.positional.is_empty(),
    };
    if !positional_ok {
        return Err(Error::Usage(format!(
            "argumento inesperado: {}",
            options.positional.join(" ")
        )));
    }
    if !options.rest.is_empty() && command != "run" {
        return Err(Error::Usage("`--` só vale no `run`".into()));
    }
    tomni_ui_cli::check_rules(&options.allow)?;
    Ok(options)
}

/// Compila e abre a tela pedida.
fn open(options: &Options) -> Result<Session, Error> {
    let exe = cargo::build(&options.target, false)?;
    let mut session = Session::start(&exe.path)?;
    session.open(options.view.as_deref(), options.size, options.theme)?;
    Ok(session)
}

fn screenshot(options: Options) -> Result<ExitCode, Error> {
    let mut session = open(&options)?;
    let screen = session.screen.clone().expect("aberta em `open`");
    let path = match options.out {
        Some(path) => path,
        None => {
            let dir = cargo::output_dir(&cargo::metadata(&options.target)?);
            cargo::create_dir(&dir)?;
            dir.join(format!("{}-{}.png", screen.view, screen.theme))
        }
    };
    let path = std::path::absolute(&path).unwrap_or(path);
    session.screenshot(&path)?;
    println!("{}", path.display());
    Ok(ExitCode::SUCCESS)
}

fn tree(options: Options) -> Result<ExitCode, Error> {
    let mut session = open(&options)?;
    let tree = session.tree()?;
    if options.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&tree).expect("JSON serializável")
        );
    } else {
        print!("{}", format::tree(&tree));
    }
    Ok(ExitCode::SUCCESS)
}

fn lint(options: Options) -> Result<ExitCode, Error> {
    let exe = cargo::build(&options.target, false)?;
    let mut session = Session::start(&exe.path)?;
    // Sem `--view`, todas as telas; sem `--theme`, os dois temas.
    let views = match &options.view {
        Some(view) => vec![view.clone()],
        None => session.hello.views.clone(),
    };
    let themes = match options.theme {
        Some(theme) => vec![theme],
        None => vec!["light", "dark"],
    };
    let (mut errors, mut warnings) = (0, 0);
    let mut report = Vec::new();
    for view in &views {
        for theme in &themes {
            let screen = session.open(Some(view), options.size, Some(theme))?.clone();
            let issues = session.lint(&options.allow)?;
            let e = issues.iter().filter(|i| format::is_error(i)).count();
            errors += e;
            warnings += issues.len() - e;
            if options.json {
                report.push(json!({
                    "view": screen.view,
                    "theme": screen.theme,
                    "size": [screen.size.0, screen.size.1],
                    "issues": issues,
                }));
                continue;
            }
            let (w, h) = screen.size;
            let title = format!(
                "{} ({}, {w}×{h})",
                screen.view,
                format::theme(&screen.theme)
            );
            if issues.is_empty() {
                println!("{title}: ok");
            } else {
                println!("{title}:");
                for issue in &issues {
                    println!("  {}", format::issue(issue));
                }
            }
        }
    }
    if options.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&Value::Array(report)).expect("JSON serializável")
        );
    } else {
        println!("{}", summary(errors, warnings));
    }
    Ok(if errors == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn summary(errors: usize, warnings: usize) -> String {
    let plural =
        |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    format!(
        "{}, {}",
        plural(errors, "erro", "erros"),
        plural(warnings, "aviso", "avisos")
    )
}

fn run_app(options: Options) -> Result<ExitCode, Error> {
    let status = cargo::cargo()
        .arg("run")
        .args(options.target.args())
        .arg("--")
        .args(&options.rest)
        .status()
        .map_err(|e| Error::Other(format!("não deu para rodar o cargo: {e}")))?;
    Ok(match status.code() {
        Some(0) => ExitCode::SUCCESS,
        Some(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
        None => ExitCode::FAILURE,
    })
}

fn build(options: Options) -> Result<ExitCode, Error> {
    let os = options.os.as_deref().ok_or_else(|| {
        Error::Usage("o `build` precisa de --target windows|linux|macos|android".into())
    })?;
    let os = dist::Os::from_name(os)?;
    if os == dist::Os::Android {
        eprintln!(
            "aviso: o build de Android usa o cargo-apk, com o Android SDK e o NDK; não é \
             conferido no CI do Tomni UI"
        );
    }
    let dist = dist::build(&options.target, os)?;
    println!("{}", dist.artifact.display());
    println!("{}", dist.notices.display());
    Ok(ExitCode::SUCCESS)
}

fn init(options: Options) -> Result<ExitCode, Error> {
    let dir = match options.positional.first() {
        Some(dir) => PathBuf::from(dir),
        None => cargo::metadata(&options.target)?.workspace_root,
    };
    for (path, change) in init::init(&dir)? {
        println!("{}: {change}", dir.join(path).display());
    }
    if !on_path("tomni-ui-mcp") {
        // Os crates ainda não estão no crates.io.
        println!(
            "O .mcp.json chama o `tomni-ui-mcp`, que não está no PATH. Instale a partir do \
             código do Tomni UI: `cargo install --path CAMINHO/crates/mcp` (e \
             `CAMINHO/crates/cli` para o `tomni-ui`)."
        );
    }
    Ok(ExitCode::SUCCESS)
}

/// Se há um executável `name` em alguma pasta do `PATH`.
fn on_path(name: &str) -> bool {
    let file = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(&file).is_file()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(text: &str) -> VecDeque<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn opcoes() {
        let o = parse(
            "screenshot",
            args("--view login --size=390x844 --theme dark --example login -o a.png"),
        )
        .unwrap();
        assert_eq!(o.view.as_deref(), Some("login"));
        assert_eq!(o.size, Some((390, 844)));
        assert_eq!(o.theme, Some("dark"));
        assert_eq!(o.target.example.as_deref(), Some("login"));
        assert_eq!(o.out, Some(PathBuf::from("a.png")));

        let o = parse("run", args("--release -- --flag x")).unwrap();
        assert!(o.target.release);
        assert_eq!(o.rest, ["--flag", "x"]);

        assert!(matches!(
            parse("tree", args("--allow contrast")),
            Err(Error::Usage(_))
        ));
        assert!(matches!(
            parse("lint", args("--allow contraste")),
            Err(Error::Usage(_))
        ));
        assert!(matches!(
            parse("lint", args("--view")),
            Err(Error::Usage(_))
        ));
        assert!(matches!(
            parse("lint", args("--json=1")),
            Err(Error::Usage(_))
        ));
        assert!(parse("init", args("pasta")).is_ok());
        assert!(parse("tree", args("pasta")).is_err());
    }

    #[test]
    fn resumo() {
        assert_eq!(summary(0, 1), "0 erros, 1 aviso");
        assert_eq!(summary(1, 2), "1 erro, 2 avisos");
    }
}
