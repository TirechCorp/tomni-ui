//! A pasta em que o shell de uma aba SSH está agora, descoberta por um comando num canal `exec`
//! da mesma conexão, sem configurar nada no servidor.
//!
//! O OpenSSH (e o Dropbear) atende todos os canais de uma conexão por um único processo do
//! servidor (o `sshd: ana@pts/3`). O shell da aba e o comando da sonda são, então, filhos do
//! mesmo processo. O script sobe do próprio pid até esse processo, olha os filhos dele que têm
//! terminal e lê a pasta do processo em primeiro plano do terminal do shell (ou, se não puder
//! ler, a do próprio shell). Só funciona em servidores com `/proc` (Linux); em qualquer outro
//! (Windows, BSD, macOS) o resultado é "não sei".

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use super::exec::ExecOutput;
use super::link::SshChannelOpener;
use crate::quote::{ShellQuote, shell_quote};

/// Os nomes (`comm`) do processo do servidor que atende a conexão: o `sshd` (e o `sshd-session`
/// do OpenSSH 9.8 ou mais novo) e o `dropbear`.
pub(super) const SERVER_NAMES: [&str; 2] = ["sshd*", "dropbear*"];

/// O maior tamanho aceito para a saída da sonda (e para o caminho).
const MAX_OUTPUT: usize = 4096;

/// O script `sh` da sonda. Recebe como argumentos os padrões do `comm` do processo do
/// servidor. Sem aspas simples nem barras duplas, para sobreviver ao shell de login de quem
/// conecta (`bash`, `zsh`, `fish`...). Imprime a pasta numa linha, ou nada.
const SCRIPT: &str = r#"
fields() {
  { read -r l; } 2>/dev/null < "/proc/$1/stat" || return 1
  set -- ${l##*) }
  F_PPID=$2
  F_TTY=$5
  F_TPGID=$6
}
p=$$
n=0
srv=
while [ "$n" -lt 32 ] && [ -n "$p" ] && [ "$p" -gt 1 ]; do
  { read -r c; } 2>/dev/null < "/proc/$p/comm" || exit 0
  for q in "$@"; do
    case "$c" in $q) srv=$p ;; esac
  done
  [ -n "$srv" ] && break
  fields "$p" || exit 0
  p=$F_PPID
  n=$((n + 1))
done
[ -n "$srv" ] || exit 0
count=0
shell=
tpgid=
for d in /proc/[0-9]*; do
  pid=${d#/proc/}
  fields "$pid" || continue
  [ "$F_PPID" = "$srv" ] || continue
  [ "$F_TTY" != 0 ] || continue
  count=$((count + 1))
  shell=$pid
  tpgid=$F_TPGID
done
[ "$count" = 1 ] || exit 0
dir=
if [ "$tpgid" -gt 0 ] 2>/dev/null; then
  dir=$(readlink "/proc/$tpgid/cwd" 2>/dev/null)
fi
[ -n "$dir" ] || dir=$(readlink "/proc/$shell/cwd" 2>/dev/null)
[ -n "$dir" ] && printf "%s\n" "$dir"
exit 0
"#;

/// O comando que o `exec` manda: `sh -c '<script>' sh <padrões>`. Embrulhado em `sh -c` porque o
/// `exec` roda no shell de login da pessoa, que pode não ser POSIX.
pub(super) fn probe_command(names: &[&str]) -> String {
    let mut command = format!("sh -c {} sh", shell_quote(SCRIPT, ShellQuote::Posix));
    for name in names {
        command.push(' ');
        command.push_str(&shell_quote(name, ShellQuote::Posix));
    }
    command
}

/// Lê a saída da sonda: uma pasta absoluta numa linha. Vazio, lixo, caminho que não começa com
/// `/`, mais de uma linha ou mais de 4 KiB dão `None`.
pub(super) fn parse_output(output: &ExecOutput) -> Option<String> {
    if output.status != Some(0) || output.stdout.len() > MAX_OUTPUT {
        return None;
    }
    let text = std::str::from_utf8(&output.stdout).ok()?;
    let line = text.strip_suffix('\n').unwrap_or(text);
    let line = line.strip_suffix('\r').unwrap_or(line);
    if !line.starts_with('/') || line.contains(['\n', '\r', '\0']) {
        return None;
    }
    Some(line.to_owned())
}

impl SshChannelOpener {
    /// A pasta em que o shell da aba desta conexão está agora, lida do servidor por um comando
    /// num canal `exec` novo da mesma conexão (sem configurar nada nele). `None` quando o
    /// servidor não é Linux (sem `/proc`), quando não há exatamente um shell com terminal na
    /// conexão, quando a leitura passa de `timeout` ou quando a resposta não é uma pasta. Erro
    /// só se o canal não abre (a conexão caiu). A pasta é a do processo em primeiro plano do
    /// terminal do shell (um `vim`, um `ssh` aninhado) ou, se o servidor não deixa ler esse
    /// (um `sudo -i`), a do próprio shell.
    ///
    /// Bloqueia até `timeout`; chame de uma thread. A saída é limitada a 4 KiB.
    ///
    /// ```no_run
    /// use std::time::Duration;
    /// use tomni_ui_terminal::Session;
    ///
    /// # fn exemplo(sessao: Session) -> std::io::Result<()> {
    /// let canais = sessao.ssh_channels().expect("a sessão é SSH");
    /// if let Some(pasta) = canais.shell_dir(Duration::from_secs(2))? {
    ///     println!("a aba está em {pasta}");
    /// }
    /// # Ok(()) }
    /// ```
    pub fn shell_dir(&self, timeout: Duration) -> io::Result<Option<String>> {
        let cancel = Arc::new(AtomicBool::new(false));
        // O relógio do tempo todo: a bandeira de cancelamento fecha o canal se o servidor
        // demora. Quando a leitura acaba, o remetente cai e o vigia sai na hora.
        let (done, wait) = mpsc::channel::<()>();
        let watcher = {
            let cancel = Arc::clone(&cancel);
            thread::Builder::new()
                .name("tomni-ui-ssh-pasta".into())
                .spawn(move || {
                    if wait.recv_timeout(timeout) == Err(mpsc::RecvTimeoutError::Timeout) {
                        cancel.store(true, Ordering::Relaxed);
                    }
                })?
        };
        let result = self.exec_cancelable(
            &probe_command(&SERVER_NAMES),
            &mut io::empty(),
            &mut |_| {},
            timeout,
            &cancel,
        );
        drop(done);
        let _ = watcher.join();
        match result {
            Ok(output) => Ok(parse_output(&output)),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => Ok(None),
            Err(error) if error.kind() == io::ErrorKind::TimedOut => Ok(None),
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saida(status: Option<u32>, stdout: &[u8]) -> ExecOutput {
        ExecOutput {
            status,
            stdout: stdout.to_vec(),
            stderr: Vec::new(),
        }
    }

    #[test]
    fn uma_pasta_numa_linha() {
        assert_eq!(
            parse_output(&saida(Some(0), b"/srv/app\n")),
            Some("/srv/app".into())
        );
        assert_eq!(
            parse_output(&saida(Some(0), b"/srv/meu app\r\n")),
            Some("/srv/meu app".into())
        );
        assert_eq!(
            parse_output(&saida(Some(0), "/srv/pasta é".as_bytes())),
            Some("/srv/pasta é".into())
        );
    }

    #[test]
    fn vazio_lixo_e_relativo_dao_none() {
        assert_eq!(parse_output(&saida(Some(0), b"")), None);
        assert_eq!(parse_output(&saida(Some(0), b"\n")), None);
        assert_eq!(
            parse_output(&saida(Some(0), b"sh: 1: readlink: not found\n")),
            None
        );
        assert_eq!(parse_output(&saida(Some(0), b"app/pasta\n")), None);
        assert_eq!(parse_output(&saida(Some(0), b"/a\n/b\n")), None);
        assert_eq!(
            parse_output(&saida(Some(0), &[b'/', 0xff, 0xfe, b'\n'])),
            None
        );
        assert_eq!(parse_output(&saida(Some(0), b"/a\0b\n")), None);
    }

    #[test]
    fn falha_do_comando_da_none() {
        assert_eq!(parse_output(&saida(Some(127), b"/srv/app\n")), None);
        assert_eq!(parse_output(&saida(None, b"/srv/app\n")), None);
    }

    #[test]
    fn mais_de_4_kib_da_none() {
        let mut grande = vec![b'/'];
        grande.extend(std::iter::repeat_n(b'a', MAX_OUTPUT));
        grande.push(b'\n');
        assert_eq!(parse_output(&saida(Some(0), &grande)), None);
        let mut cabe = vec![b'/'];
        cabe.extend(std::iter::repeat_n(b'a', MAX_OUTPUT - 2));
        cabe.push(b'\n');
        assert!(parse_output(&saida(Some(0), &cabe)).is_some());
    }

    #[test]
    fn o_comando_embrulha_o_script_num_sh_c() {
        let comando = probe_command(&SERVER_NAMES);
        assert!(comando.starts_with("sh -c '"), "{comando}");
        assert!(comando.ends_with(" sh 'sshd*' 'dropbear*'"), "{comando}");
        // O script sobrevive ao `fish`: dentro de aspas simples ele só tira o sentido de `\\` e
        // de `\'`, então o script não tem aspa simples nem barra dupla (o `\n` do printf passa).
        assert!(!SCRIPT.contains("\\\\"));
        assert!(!SCRIPT.contains('\''));
    }
}

/// O script contra processos de verdade, com um `sshd` de mentira: um `sh` com outro nome
/// (`comm`) faz o papel do processo do servidor, e o `setsid` tira o terminal da sonda, como o
/// `sshd` faz com os comandos do `exec`.
#[cfg(all(test, target_os = "linux", feature = "pty"))]
mod com_processos {
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    use super::*;
    use crate::process::Process;
    use crate::transport::{Status, Transport};

    fn sh_real() -> PathBuf {
        ["/bin/sh", "/usr/bin/sh"]
            .iter()
            .map(Path::new)
            .find(|p| p.exists())
            .expect("há um sh")
            .canonicalize()
            .unwrap()
    }

    /// Um diretório de teste novo, com o `sh` copiado por ligação simbólica com o nome do
    /// servidor de mentira.
    fn preparar(nome: &str) -> (PathBuf, PathBuf) {
        let base =
            std::env::temp_dir().join(format!("tomni-ui-pasta-{nome}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("onde-o-servidor-esta")).unwrap();
        std::fs::create_dir_all(base.join("onde-o-shell-esta")).unwrap();
        let falso = base.join("sshd-de-mentira");
        std::os::unix::fs::symlink(sh_real(), &falso).unwrap();
        (base.canonicalize().unwrap(), falso)
    }

    /// Roda `externo` num "servidor" (o `sh` com o nome `sshd-de-mentira`, num pseudoterminal) e
    /// devolve o que a sonda imprimiu entre as marcas.
    fn rodar(falso: &Path, base: &Path, externo: &str) -> String {
        let sonda = probe_command(&["sshd-de-mentira*"]);
        let externo = externo.replace("SONDA", &format!("setsid {sonda}"));
        let mut servidor = Process::builder(falso)
            .arg("-c")
            .arg(format!("echo INI; {externo}; echo FIM"))
            .cwd(base.join("onde-o-servidor-esta"))
            .spawn();
        let limite = Instant::now() + Duration::from_secs(15);
        let mut lido = Vec::new();
        loop {
            let status = servidor.read(&mut lido, usize::MAX);
            let texto = String::from_utf8_lossy(&lido).replace('\r', "");
            if (texto.contains("\nFIM\n") && texto.contains("INI\n"))
                || status != Status::Open
                || Instant::now() > limite
            {
                let depois = texto.split("INI\n").nth(1).unwrap_or_default().to_owned();
                return depois.split("FIM\n").next().unwrap_or_default().to_owned();
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn acha_o_shell_com_terminal_e_le_a_pasta_do_primeiro_plano() {
        let (base, falso) = preparar("um");
        let shell = base.join("onde-o-shell-esta");
        // Um shell filho do "servidor", com o terminal dele. O processo em primeiro plano do
        // terminal é o próprio "servidor" (sem controle de jobs), e é a pasta dele que sai.
        let saida = rodar(
            &falso,
            &base,
            &format!(
                "(cd {} && exec sleep 5) & a=$!; sleep 0.3; SONDA; kill $a",
                shell.display()
            ),
        );
        assert_eq!(
            saida.trim(),
            base.join("onde-o-servidor-esta").display().to_string()
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn sem_nenhum_shell_com_terminal_nao_imprime_nada() {
        let (base, falso) = preparar("zero");
        let saida = rodar(&falso, &base, "SONDA");
        assert_eq!(saida.trim(), "");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn com_mais_de_um_shell_nao_imprime_nada() {
        let (base, falso) = preparar("dois");
        let saida = rodar(
            &falso,
            &base,
            "(exec sleep 5) & a=$!; (exec sleep 5) & b=$!; sleep 0.3; SONDA; kill $a $b",
        );
        assert_eq!(saida.trim(), "");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn sem_o_servidor_na_cadeia_nao_imprime_nada() {
        let (base, falso) = preparar("sem");
        // O padrão não casa com nenhum antepassado.
        let sonda = probe_command(&["nada-casa-com-isto*"]);
        let mut servidor = Process::builder(&falso)
            .arg("-c")
            .arg(format!("echo INI; setsid {sonda}; echo FIM"))
            .spawn();
        let limite = Instant::now() + Duration::from_secs(15);
        let mut lido = Vec::new();
        while Instant::now() < limite {
            let status = servidor.read(&mut lido, usize::MAX);
            let texto = String::from_utf8_lossy(&lido).replace('\r', "");
            if texto.contains("\nFIM\n") || status != Status::Open {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let texto = String::from_utf8_lossy(&lido).replace('\r', "");
        assert_eq!(texto.trim(), "INI\nFIM");
        let _ = std::fs::remove_dir_all(&base);
    }
}
