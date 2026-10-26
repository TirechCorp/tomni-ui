//! `tomni-ui build --target windows|linux|macos|android`: o binário (ou APK) de distribuição,
//! com os avisos de terceiros que as licenças MIT, BSD e OFL exigem.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::SystemTime;

use crate::Error;
use crate::cargo::{self, Metadata, Target};

/// Template do `cargo about` (o `about.hbs` da raiz do repositório).
pub const ABOUT_TEMPLATE: &str = include_str!("../templates/about.hbs");

/// Configuração do `cargo about` (o `about.toml` da raiz do repositório).
pub const ABOUT_CONFIG: &str = include_str!("../templates/about.toml");

/// Nome do arquivo de avisos ao lado do binário.
pub const NOTICES: &str = "THIRD-PARTY-NOTICES.txt";

/// Sistema de destino.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    /// Windows.
    Windows,
    /// Linux.
    Linux,
    /// macOS.
    Macos,
    /// Android, como APK.
    Android,
}

impl Os {
    /// Pelo nome da CLI.
    pub fn from_name(name: &str) -> Result<Self, Error> {
        Ok(match name {
            "windows" => Self::Windows,
            "linux" => Self::Linux,
            "macos" => Self::Macos,
            "android" => Self::Android,
            _ => {
                return Err(Error::Usage(format!(
                    "alvo {name:?} desconhecido: use windows, linux, macos ou android"
                )));
            }
        })
    }

    /// Nome da CLI e da pasta em `target/tomni-ui/dist`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Linux => "linux",
            Self::Macos => "macos",
            Self::Android => "android",
        }
    }

    /// O sistema em que a CLI roda.
    pub fn host() -> Option<Self> {
        match std::env::consts::OS {
            "windows" => Some(Self::Windows),
            "linux" => Some(Self::Linux),
            "macos" => Some(Self::Macos),
            _ => None,
        }
    }

    /// Triple padrão na compilação cruzada, na arquitetura da máquina quando dá.
    fn default_triple(self) -> &'static str {
        let arm = std::env::consts::ARCH == "aarch64";
        match self {
            Self::Windows if cfg!(windows) => {
                if arm {
                    "aarch64-pc-windows-msvc"
                } else {
                    "x86_64-pc-windows-msvc"
                }
            }
            // Fora do Windows, o MSVC não existe; o MinGW sim (`x86_64-w64-mingw32-gcc`).
            Self::Windows => "x86_64-pc-windows-gnu",
            Self::Linux if arm => "aarch64-unknown-linux-gnu",
            Self::Linux => "x86_64-unknown-linux-gnu",
            Self::Macos if arm => "aarch64-apple-darwin",
            Self::Macos => "x86_64-apple-darwin",
            Self::Android => "aarch64-linux-android",
        }
    }
}

/// O que foi gerado.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dist {
    /// Pasta da distribuição.
    pub dir: PathBuf,
    /// Binário ou APK.
    pub artifact: PathBuf,
    /// Avisos de terceiros.
    pub notices: PathBuf,
}

/// Compila em release para `os` e junta o binário e os avisos em `target/tomni-ui/dist/<os>/`.
///
/// No sistema da própria máquina, sem `target.triple`, compila nativo. Para outro sistema, usa
/// o triple padrão (ou `target.triple`), que precisa do `rustup target add` e de um linker.
/// No Android, roda o `cargo apk build`.
pub fn build(target: &Target, os: Os) -> Result<Dist, Error> {
    let metadata = cargo::metadata(target)?;
    let mut target = target.clone();
    target.release = true;
    if target.triple.is_none() && Some(os) != Os::host() {
        target.triple = Some(os.default_triple().to_owned());
    }
    let dir = cargo::output_dir(&metadata).join("dist").join(os.name());
    cargo::create_dir(&dir)?;

    let built = if os == Os::Android {
        apk(&target, &metadata)?
    } else {
        cargo::build(&target, false)?.path
    };
    let notices = dir.join(NOTICES);
    third_party_notices(&target, os, &metadata, &notices)?;
    let artifact = dir.join(built.file_name().expect("executável tem nome"));
    std::fs::copy(&built, &artifact).map_err(|e| {
        Error::Other(format!(
            "não deu para copiar {} para {}: {e}",
            built.display(),
            artifact.display()
        ))
    })?;
    Ok(Dist {
        dir,
        artifact,
        notices,
    })
}

/// `cargo apk build` e o APK mais novo em `target/<perfil>/apk`.
fn apk(target: &Target, metadata: &Metadata) -> Result<PathBuf, Error> {
    let started = SystemTime::now();
    let mut apk_target = target.clone();
    // O cargo-apk escolhe as ABIs pelo `[package.metadata.android]`.
    apk_target.triple = None;
    let status = cargo::cargo()
        .args(["apk", "build"])
        .args(apk_target.args())
        .stdin(Stdio::null())
        .status()
        .map_err(|e| Error::Other(format!("não deu para rodar o cargo: {e}")))?;
    if !status.success() {
        return Err(Error::Build(
            "o `cargo apk build` falhou. Ele precisa do cargo-apk (`cargo install cargo-apk`), \
             do Android SDK e do NDK (ANDROID_HOME e ANDROID_NDK_ROOT), e o app de um \
             `android_main` (veja o exemplo login)"
                .into(),
        ));
    }
    let apk_dir = metadata.target_directory.join("release").join("apk");
    newest_apk(&apk_dir, started).ok_or_else(|| {
        Error::Other(format!(
            "o cargo apk terminou, mas não há APK novo em {}",
            apk_dir.display()
        ))
    })
}

fn newest_apk(dir: &Path, since: SystemTime) -> Option<PathBuf> {
    let mut found: Option<(SystemTime, PathBuf)> = None;
    let mut dirs = vec![dir.to_owned()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).ok()?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "apk")
                && let Ok(modified) = entry.metadata().and_then(|m| m.modified())
                && modified >= since
                && found.as_ref().is_none_or(|(t, _)| modified > *t)
            {
                found = Some((modified, path));
            }
        }
    }
    found.map(|(_, path)| path)
}

/// Gera os avisos com o `cargo about`, filtrando as dependências do alvo. Usa o `about.toml`
/// e o `about.hbs` do projeto, se houver; senão os do Tomni UI.
fn third_party_notices(
    target: &Target,
    os: Os,
    metadata: &Metadata,
    output: &Path,
) -> Result<(), Error> {
    let own = |name: &str, embedded: &str| -> Result<PathBuf, Error> {
        let path = metadata.workspace_root.join(name);
        if path.is_file() {
            return Ok(path);
        }
        let dir = cargo::output_dir(metadata).join("about");
        cargo::create_dir(&dir)?;
        let path = dir.join(name);
        std::fs::write(&path, embedded)
            .map_err(|e| Error::Other(format!("não deu para gravar {}: {e}", path.display())))?;
        Ok(path)
    };
    let config = own("about.toml", ABOUT_CONFIG)?;
    let template = own("about.hbs", ABOUT_TEMPLATE)?;
    let triple = target
        .triple
        .clone()
        .unwrap_or_else(|| host_triple(os).to_owned());
    let mut command = cargo::cargo();
    command
        .arg("about")
        .arg("generate")
        .arg("-c")
        .arg(&config)
        .arg(&template)
        .arg("-o")
        .arg(output)
        .args(["--target", &triple])
        .stdin(Stdio::null());
    if let Some(path) = &target.manifest_path {
        command.arg("--manifest-path").arg(path);
    }
    if !target.features.is_empty() {
        command.args(["--features", &target.features.join(" ")]);
    }
    if target.no_default_features {
        command.arg("--no-default-features");
    }
    let output = command
        .output()
        .map_err(|e| Error::Other(format!("não deu para rodar o cargo: {e}")))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("no such command") || stderr.contains("no such subcommand") {
        return Err(Error::Build(
            "os avisos de terceiros saem do cargo-about: instale com `cargo install cargo-about \
             --locked`"
                .into(),
        ));
    }
    Err(Error::Build(format!(
        "o cargo about falhou. Se uma dependência usa uma licença fora da lista, revise e \
         acrescente ao `accepted` de um about.toml na raiz do projeto:\n{}",
        stderr.trim()
    )))
}

/// Triple da máquina, para filtrar as dependências no build nativo.
fn host_triple(os: Os) -> &'static str {
    let arm = std::env::consts::ARCH == "aarch64";
    match os {
        Os::Windows if arm => "aarch64-pc-windows-msvc",
        Os::Windows => "x86_64-pc-windows-msvc",
        Os::Linux if arm => "aarch64-unknown-linux-gnu",
        Os::Linux => "x86_64-unknown-linux-gnu",
        Os::Macos if arm => "aarch64-apple-darwin",
        Os::Macos => "x86_64-apple-darwin",
        Os::Android => "aarch64-linux-android",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alvos() {
        for name in ["windows", "linux", "macos", "android"] {
            assert_eq!(Os::from_name(name).unwrap().name(), name);
        }
        assert!(Os::from_name("ios").is_err());
        // Os triples padrão estão na lista do about.toml.
        for os in [Os::Windows, Os::Linux, Os::Macos, Os::Android] {
            assert!(
                ABOUT_CONFIG.contains(host_triple(os)),
                "{} fora do about.toml",
                host_triple(os)
            );
        }
    }
}
