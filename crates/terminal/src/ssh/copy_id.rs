//! O `ssh-copy-id`: instala uma chave pública no `authorized_keys` do servidor.
//!
//! Os comandos são os do `ssh-copy-id` do OpenSSH 9.x. A diferença está na verificação: o
//! original tenta entrar com a chave, e aqui se lê o arquivo, que é mais confiável e não
//! autentica uma vez a mais.

use ssh_key::PublicKey;

use super::exec::on_thread;
use super::link::SshLink;
use super::{SshBuilder, keys};

/// O caminho do `authorized_keys` num servidor Unix, a partir da pasta pessoal.
const UNIX_PATH: &str = ".ssh/authorized_keys";

/// O comando do `ssh-copy-id` 9.x que anexa a linha lida do `stdin` ao `authorized_keys`.
const UNIX_INSTALL: &str = "sh -c 'cd ; umask 077 ; AUTH_KEY_FILE=\".ssh/authorized_keys\" ; \
mkdir -p \"$(dirname \"$AUTH_KEY_FILE\")\" && { [ -z \"$(tail -1c \"$AUTH_KEY_FILE\" 2>/dev/null)\" ] \
|| echo >> \"$AUTH_KEY_FILE\" || exit 1 ; } && cat >> \"$AUTH_KEY_FILE\" || exit 1 ; \
if type restorecon >/dev/null 2>&1 ; then restorecon -F .ssh \"$AUTH_KEY_FILE\" ; fi'";

/// O SID do grupo de administradores do Windows.
const ADMINISTRATORS: &str = "S-1-5-32-544";

/// As opções de [`copy_id`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CopyIdOptions {
    /// Instala mesmo se a chave já estiver no servidor (`-f` do `ssh-copy-id`).
    pub force: bool,
    /// Só confere o que seria feito, sem mudar nada no servidor (`-n`).
    pub dry_run: bool,
}

/// O que o [`copy_id`] fez.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CopyIdOutcome {
    /// A chave foi instalada em `path` (no servidor).
    Installed {
        /// `.ssh/authorized_keys`, ou o caminho completo no Windows.
        path: String,
    },
    /// A chave já estava no servidor e nada mudou.
    AlreadyThere,
    /// Com `dry_run`: a chave seria instalada em `path`.
    WouldInstall {
        /// O mesmo caminho que o `Installed` traria.
        path: String,
    },
}

/// Instala `key` no `authorized_keys` do servidor de `builder`, que autentica como nas outras
/// conexões (senha, chaves, agente). Funciona em servidores Unix e no OpenSSH do Windows.
///
/// ```no_run
/// use tomni_ui_terminal::ssh::{CopyIdOptions, SshBuilder, copy_id};
/// use ssh_key::PublicKey;
///
/// let chave = PublicKey::from_openssh("ssh-ed25519 AAAA… ana@notebook")?;
/// let servidor = SshBuilder::new("servidor", 22, "ana").password("segredo");
/// let resultado = copy_id(&servidor, &chave, CopyIdOptions::default())?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn copy_id(
    builder: &SshBuilder,
    key: &PublicKey,
    options: CopyIdOptions,
) -> Result<CopyIdOutcome, String> {
    let mut line = keys::public_openssh(key).trim_end().to_owned();
    if line.is_empty() {
        return Err("a chave pública está vazia".to_owned());
    }
    line.push('\n');
    on_thread(builder, |remote| match detect(remote)? {
        System::Unix => unix(remote, &line, options),
        System::Windows => windows(remote, &line, options),
    })
}

enum System {
    Unix,
    Windows,
}

/// Descobre o sistema do servidor: um `sh` responde ao `uname`; senão, se há PowerShell, é o
/// OpenSSH do Windows.
fn detect(remote: &SshLink) -> Result<System, String> {
    let probe = remote.exec("echo $0 ; uname -s", b"")?;
    if probe.success() {
        let text = probe.stdout_text();
        let mut lines = text.lines().map(str::trim);
        let shell = lines.next().unwrap_or_default();
        let uname = lines.next().unwrap_or_default();
        if !shell.is_empty() && !uname.is_empty() && !uname.contains(char::is_whitespace) {
            return Ok(System::Unix);
        }
    }
    let probe = remote.exec(
        "powershell -NoProfile -Command \"[Environment]::OSVersion.Platform\"",
        b"",
    )?;
    if probe.success() && probe.stdout_text().contains("Win32NT") {
        return Ok(System::Windows);
    }
    Err(
        "não deu para identificar o sistema do servidor (nem um shell Unix nem o PowerShell)"
            .to_owned(),
    )
}

fn unix(remote: &SshLink, line: &str, options: CopyIdOptions) -> Result<CopyIdOutcome, String> {
    if !options.force {
        let current = remote.exec("cat .ssh/authorized_keys 2>/dev/null", b"")?;
        if has_key(&current.stdout_text(), line) {
            return Ok(CopyIdOutcome::AlreadyThere);
        }
    }
    if options.dry_run {
        return Ok(CopyIdOutcome::WouldInstall {
            path: UNIX_PATH.to_owned(),
        });
    }
    let output = remote.exec(UNIX_INSTALL, line.as_bytes())?;
    if !output.success() {
        return Err(failure(&output));
    }
    Ok(CopyIdOutcome::Installed {
        path: UNIX_PATH.to_owned(),
    })
}

fn windows(remote: &SshLink, line: &str, options: CopyIdOptions) -> Result<CopyIdOutcome, String> {
    let info = remote.exec(&powershell(&windows_info_script()), b"")?;
    if !info.success() {
        return Err(failure(&info));
    }
    let text = info.stdout_text();
    let mut lines = text.lines();
    let admin = lines.next().is_some_and(|kind| kind.trim() == "admin");
    let path = lines.next().unwrap_or_default().trim().to_owned();
    if path.is_empty() {
        return Err("o servidor não informou onde fica o authorized_keys".to_owned());
    }
    let current = lines.collect::<Vec<_>>().join("\n");
    if !options.force && has_key(&current, line) {
        return Ok(CopyIdOutcome::AlreadyThere);
    }
    if options.dry_run {
        return Ok(CopyIdOutcome::WouldInstall { path });
    }
    let output = remote.exec(
        &powershell(&windows_install_script(&path, admin)),
        line.as_bytes(),
    )?;
    if !output.success() {
        return Err(failure(&output));
    }
    Ok(CopyIdOutcome::Installed { path })
}

/// A mensagem de um comando remoto que falhou.
fn failure(output: &super::exec::ExecOutput) -> String {
    let detail = output.stderr_text();
    let detail = detail.trim();
    match (output.status, detail.is_empty()) {
        (Some(status), true) => format!("o servidor recusou o comando (código {status})"),
        (Some(status), false) => {
            format!("o servidor recusou o comando (código {status}): {detail}")
        }
        (None, true) => "o servidor terminou o comando sem dar o resultado".to_owned(),
        (None, false) => detail.to_owned(),
    }
}

/// Se algum item de `authorized_keys` tem o mesmo tipo e o mesmo blob de `line` (uma linha
/// `.pub`). As opções no começo (`from=`, `command=`) e o comentário não contam.
fn has_key(authorized_keys: &str, line: &str) -> bool {
    let mut wanted = line.split_whitespace();
    let (Some(kind), Some(blob)) = (wanted.next(), wanted.next()) else {
        return false;
    };
    authorized_keys
        .lines()
        .map(str::trim)
        .filter(|entry| !entry.is_empty() && !entry.starts_with('#'))
        .any(|entry| {
            let tokens: Vec<&str> = entry.split_whitespace().collect();
            tokens
                .windows(2)
                .any(|pair| pair[0] == kind && pair[1] == blob)
        })
}

/// O comando que roda `script` no PowerShell do servidor. O script vai codificado
/// (`-EncodedCommand`), sem depender das aspas do shell padrão (o `cmd.exe`).
fn powershell(script: &str) -> String {
    let utf16: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    format!(
        "powershell -NoProfile -NonInteractive -EncodedCommand {}",
        base64(&utf16)
    )
}

/// O script que diz quem é o usuário (`admin` ou `user`), onde fica o `authorized_keys` e o
/// que há nele.
fn windows_info_script() -> String {
    format!(
        "$ErrorActionPreference = 'Stop'\n\
$admin = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.SecurityIdentifier]'{ADMINISTRATORS}')\n\
if ($admin) {{ $file = Join-Path $env:ProgramData 'ssh\\administrators_authorized_keys' }} else {{ $file = Join-Path $env:USERPROFILE '.ssh\\authorized_keys' }}\n\
if ($admin) {{ 'admin' }} else {{ 'user' }}\n\
$file\n\
if (Test-Path $file) {{ Get-Content -Raw $file }}\n"
    )
}

/// O script que anexa ao `path` a linha que chega pelo `stdin` (`$input`), sem BOM e com as
/// permissões que o OpenSSH do Windows exige no arquivo dos administradores.
fn windows_install_script(path: &str, admin: bool) -> String {
    let path = path.replace('\'', "''");
    let admin_flag = if admin { "$true" } else { "$false" };
    format!(
        "$ErrorActionPreference = 'Stop'\n\
$file = '{path}'\n\
$admin = {admin_flag}\n\
$line = ($input | Out-String).Trim()\n\
$dir = Split-Path $file\n\
if (-not (Test-Path $dir)) {{ New-Item -ItemType Directory -Force -Path $dir | Out-Null }}\n\
$utf8 = New-Object System.Text.UTF8Encoding $false\n\
$prefix = ''\n\
if (Test-Path $file) {{ $raw = [IO.File]::ReadAllText($file); if ($raw.Length -gt 0 -and -not $raw.EndsWith(\"`n\")) {{ $prefix = \"`r`n\" }} }}\n\
[IO.File]::AppendAllText($file, $prefix + $line + \"`r`n\", $utf8)\n\
if ($admin) {{ icacls $file /inheritance:r /grant '*{ADMINISTRATORS}:F' /grant 'SYSTEM:F' | Out-Null; if ($LASTEXITCODE -ne 0) {{ exit 1 }} }}\n"
    )
}

/// Base64 padrão (a mesma do `-EncodedCommand`).
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let triple = (u32::from(chunk[0]) << 16)
            | (u32::from(chunk.get(1).copied().unwrap_or(0)) << 8)
            | u32::from(chunk.get(2).copied().unwrap_or(0));
        for position in 0..4 {
            if position <= chunk.len() {
                let index = (triple >> (18 - 6 * position)) & 0x3f;
                out.push(char::from(ALPHABET[index as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_confere_com_os_vetores_do_rfc_4648() {
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
    fn has_key_ignora_opcoes_comentario_e_linhas_vazias() {
        let linha = "ssh-ed25519 AAAAC3Nza ana@notebook\n";
        let arquivo = "# chaves\n\nssh-rsa AAAAB3 outra\nfrom=\"10.0.0.1\",command=\"ls\" ssh-ed25519 AAAAC3Nza velho\n";
        assert!(has_key(arquivo, linha));
        assert!(!has_key("ssh-ed25519 OUTRO ana@notebook\n", linha));
        assert!(!has_key("ssh-rsa AAAAC3Nza x\n", linha));
        assert!(!has_key("", linha));
        assert!(!has_key(arquivo, ""));
    }

    #[test]
    fn script_do_windows_nao_leva_a_chave_nem_aspas_soltas() {
        let script = windows_install_script("C:\\Users\\o'brien\\.ssh\\authorized_keys", false);
        assert!(script.contains("o''brien"));
        assert!(script.contains("$input"));
        assert!(!script.contains("ssh-ed25519"));
    }
}
