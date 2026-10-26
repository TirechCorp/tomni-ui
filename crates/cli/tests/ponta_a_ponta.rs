//! A CLI de verdade contra o exemplo `login` do `tomni-ui`: compila com o `cargo`, roda o app
//! em modo de ferramentas e confere a saída.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// `tomni-ui <args> -p tomni-ui --example login`.
fn tomni_ui(args: &[&str]) -> Output {
    let manifest = workspace().join("Cargo.toml");
    let output = Command::new(env!("CARGO_BIN_EXE_tomni-ui"))
        .args(args)
        .args(["--manifest-path", manifest.to_str().unwrap()])
        .args(["-p", "tomni-ui", "--example", "login"])
        .output()
        .unwrap();
    eprintln!("{}", String::from_utf8_lossy(&output.stderr));
    output
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

#[test]
fn lint_tree_e_screenshot() {
    let lint = tomni_ui(&["lint"]);
    assert!(lint.status.success());
    assert_eq!(
        stdout(&lint),
        "login (claro, 390×844): ok\nlogin (escuro, 390×844): ok\n0 erros, 0 avisos\n"
    );

    let tree = tomni_ui(&["tree", "--size", "320x480"]);
    assert!(tree.status.success());
    assert!(
        stdout(&tree).contains(r#"button #entrar "Entrar" accent (24, 136, 272×44) disabled"#),
        "{}",
        stdout(&tree)
    );

    let png = std::env::temp_dir().join(format!("tomni-ui-cli-{}.png", std::process::id()));
    let shot = tomni_ui(&[
        "screenshot",
        "--theme",
        "dark",
        "--size",
        "200x300",
        "-o",
        png.to_str().unwrap(),
    ]);
    assert!(shot.status.success());
    let bytes = std::fs::read(&png).unwrap();
    std::fs::remove_file(&png).unwrap();
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    // Largura e altura no cabeçalho IHDR.
    assert_eq!(&bytes[16..24], &[0, 0, 0, 200, 0, 0, 1, 44]);
}

#[test]
fn erros_de_uso() {
    let tela = tomni_ui(&["tree", "--view", "nada"]);
    assert_eq!(tela.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&tela.stderr).contains(r#"nenhuma tela "nada"; telas: login"#));

    let regra = tomni_ui(&["lint", "--allow", "cor"]);
    assert_eq!(regra.status.code(), Some(2));
}
