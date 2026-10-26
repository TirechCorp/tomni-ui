//! Um programa local num pseudoterminal, pelo `portable-pty`: o `openpty` no Linux e no macOS
//! e o ConPTY no Windows.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use portable_pty::{
    ChildKiller, CommandBuilder, ExitStatus, MasterPty, PtySize, native_pty_system,
};

use crate::transport::{Inbox, Status, Transport, closed_error, error_message};

/// O `TERM` do processo: o terminal entende o que o xterm entende, com 256 cores.
const TERM: &str = "xterm-256color";

type Master = Arc<Mutex<Option<Box<dyn MasterPty + Send>>>>;

/// Um programa local num pseudoterminal: o shell da pessoa, um `htop`, um REPL. O terminal
/// mostra a saída e manda as teclas (no Windows, pelo ConPTY).
///
/// O processo recebe `TERM=xterm-256color` e `COLORTERM=truecolor`. Quando ele sai, o
/// terminal fecha; um código diferente de zero aparece no fim da tela ("o processo saiu com o
/// código 3"). Soltar o `Process` (fechar a aba) manda o `SIGHUP` ao processo, como fechar a
/// janela de um terminal.
///
/// Só existe com a feature `pty` e fora do Android.
///
/// ```no_run
/// use tomni_ui_terminal::{Process, Terminal};
///
/// let shell = Terminal::new(Process::shell()).id("shell");
/// let htop = Terminal::new(Process::builder("htop").arg("-d").arg("10").spawn()).id("htop");
/// ```
pub struct Process {
    inbox: Inbox,
    commands: Sender<Command>,
    master: Master,
    killer: Option<Box<dyn ChildKiller + Send + Sync>>,
    /// O pid do filho, enquanto há um ([`Transport::process_id`]).
    pid: Option<u32>,
    /// Como iniciar de novo: o [`Transport::reopen`] e o [`Transport::duplicate`].
    builder: ProcessBuilder,
}

impl fmt::Debug for Process {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Process")
            .field("inbox", &self.inbox)
            .finish_non_exhaustive()
    }
}

enum Command {
    Data(Vec<u8>),
    Close,
}

/// O programa, os argumentos, o diretório e o ambiente de um [`Process`], antes de iniciar.
#[derive(Clone, Debug)]
pub struct ProcessBuilder {
    command: CommandBuilder,
    program: OsString,
}

impl ProcessBuilder {
    /// `program` com os argumentos que vierem depois. Sem caminho, é procurado no `PATH`.
    pub fn new(program: impl AsRef<OsStr>) -> Self {
        Self {
            command: CommandBuilder::new(program.as_ref()),
            program: program.as_ref().to_owned(),
        }
    }

    /// O shell da pessoa: o `$SHELL` (ou o do usuário no sistema) no Linux e no macOS, o
    /// `%ComSpec%` no Windows. Começa na pasta pessoal.
    pub fn shell() -> Self {
        Self {
            command: CommandBuilder::new_default_prog(),
            program: OsString::from("o shell"),
        }
    }

    /// Mais um argumento.
    pub fn arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        self.command.arg(arg);
        self
    }

    /// Mais argumentos.
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.command.args(args);
        self
    }

    /// A pasta onde o processo começa (a pasta pessoal, se não for dada).
    pub fn cwd(mut self, dir: impl AsRef<Path>) -> Self {
        self.command.cwd(dir.as_ref());
        self
    }

    /// Uma variável de ambiente, além das que o app já tem.
    pub fn env(mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Self {
        self.command.env(key, value);
        self
    }

    /// Inicia o processo num pseudoterminal de 80 × 24 (o terminal manda o tamanho certo no
    /// primeiro layout). Não falha: um erro ("não foi possível iniciar htop") aparece no
    /// terminal, como o da conexão do `Telnet`.
    pub fn spawn(self) -> Process {
        let (commands, receiver) = mpsc::channel();
        let mut process = Process {
            inbox: Inbox::new(),
            commands,
            master: Arc::new(Mutex::new(None)),
            killer: None,
            pid: None,
            builder: self,
        };
        process.launch(receiver);
        process
    }

    fn start(&self, process: &mut Process, commands: Receiver<Command>) -> Result<(), String> {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| error.to_string())?;
        let mut command = self.command.clone();
        command.env("TERM", TERM);
        command.env("COLORTERM", "truecolor");
        let mut child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| error.to_string())?;
        // Só o processo fica com o lado escravo: quando ele sai, a leitura recebe o fim.
        drop(pair.slave);
        process.killer = Some(child.clone_killer());
        process.pid = child.process_id();

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| error.to_string())?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|error| error.to_string())?;
        *process
            .master
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(pair.master);

        let (status_tx, status_rx) = mpsc::channel();
        let wait_master = Arc::clone(&process.master);
        spawn("tomni-ui-pty-espera", move || {
            let status = child.wait();
            // O ConPTY só termina a leitura quando o pseudoterminal fecha.
            if cfg!(windows) {
                wait_master
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take();
            }
            let _ = status_tx.send(status);
        })?;
        let read_inbox = process.inbox.clone();
        spawn("tomni-ui-pty-leitura", move || {
            read_loop(reader, &read_inbox, &status_rx);
        })?;
        let write_inbox = process.inbox.clone();
        spawn("tomni-ui-pty-escrita", move || {
            write_loop(writer, &commands, &write_inbox);
        })?;
        Ok(())
    }
}

fn spawn(name: &str, f: impl FnOnce() + Send + 'static) -> Result<(), String> {
    thread::Builder::new()
        .name(name.into())
        .spawn(f)
        .map(drop)
        .map_err(|error| error_message(&error))
}

impl Process {
    /// Inicia o processo do `builder` e fecha a fila com o erro, se não deu.
    fn launch(&mut self, commands: Receiver<Command>) {
        let builder = self.builder.clone();
        if let Err(message) = builder.start(self, commands) {
            let program = builder.program.to_string_lossy();
            self.inbox.close(Some(format!(
                "não foi possível iniciar {program}: {message}"
            )));
        }
    }

    /// O shell da pessoa ([`ProcessBuilder::shell`]), já iniciado.
    pub fn shell() -> Self {
        ProcessBuilder::shell().spawn()
    }

    /// Um programa com argumentos, iniciado com [`ProcessBuilder::spawn`].
    pub fn builder(program: impl AsRef<OsStr>) -> ProcessBuilder {
        ProcessBuilder::new(program)
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        if let Some(killer) = &mut self.killer {
            let _ = killer.kill();
        }
        let _ = self.commands.send(Command::Close);
    }
}

impl Transport for Process {
    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.commands
            .send(Command::Data(bytes.to_vec()))
            .map_err(|_| closed_error())
    }

    fn resize(&mut self, cols: u16, rows: u16) {
        let master = self.master.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(master) = master.as_ref() {
            let _ = master.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
    }

    fn read(&mut self, buf: &mut Vec<u8>, max: usize) -> Status {
        self.inbox.read(buf, max)
    }

    fn pending(&self) -> bool {
        self.inbox.pending()
    }

    fn reopen(&mut self) -> bool {
        if let Some(killer) = &mut self.killer {
            let _ = killer.kill();
        }
        let _ = self.commands.send(Command::Close);
        // O pseudoterminal novo não divide o `master` com a thread de espera do antigo.
        self.master = Arc::new(Mutex::new(None));
        self.killer = None;
        self.pid = None;
        self.inbox.reopen();
        let (commands, receiver) = mpsc::channel();
        self.commands = commands;
        self.launch(receiver);
        true
    }

    fn duplicate(&self) -> Option<Box<dyn Transport>> {
        self.duplicate_in(None)
    }

    fn duplicate_in(&self, dir: Option<&Path>) -> Option<Box<dyn Transport>> {
        let mut builder = self.builder.clone();
        if let Some(dir) = dir.filter(|dir| dir.is_dir()) {
            builder = builder.cwd(dir);
        }
        Some(Box::new(builder.spawn()))
    }

    fn process_id(&self) -> Option<u32> {
        self.pid
    }
}

/// Lê a saída até o fim (no Linux, o fim chega como `EIO` quando o processo sai) e fecha com
/// o resultado do processo.
fn read_loop(
    mut reader: Box<dyn Read + Send>,
    inbox: &Inbox,
    status: &Receiver<io::Result<ExitStatus>>,
) {
    let mut buf = vec![0; 16 * 1024];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => inbox.push(&buf[..n]),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    let reason = match status.recv() {
        Ok(Ok(status)) => exit_message(&status),
        Ok(Err(error)) => Some(error_message(&error)),
        Err(_) => None,
    };
    inbox.close(reason);
}

fn write_loop(mut writer: Box<dyn Write + Send>, commands: &Receiver<Command>, inbox: &Inbox) {
    for command in commands {
        match command {
            Command::Data(bytes) => {
                if let Err(error) = writer.write_all(&bytes).and_then(|()| writer.flush()) {
                    inbox.close(Some(error_message(&error)));
                    return;
                }
            }
            Command::Close => return,
        }
    }
}

/// A mensagem do fim da tela: nenhuma quando o processo saiu com zero.
fn exit_message(status: &ExitStatus) -> Option<String> {
    if status.success() {
        None
    } else if let Some(signal) = status.signal() {
        Some(format!("o processo terminou com o sinal {signal}"))
    } else {
        Some(format!(
            "o processo saiu com o código {}",
            status.exit_code()
        ))
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Lê até `fim` aparecer na saída ou o processo fechar, com um limite de 10 s.
    fn ler_ate(process: &mut impl Transport, fim: &str) -> (String, Status) {
        let limite = Instant::now() + Duration::from_secs(10);
        let mut lido = Vec::new();
        loop {
            let status = process.read(&mut lido, usize::MAX);
            let texto = String::from_utf8_lossy(&lido).into_owned();
            if texto.contains(fim) || status != Status::Open || Instant::now() > limite {
                return (texto, status);
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn sh() -> Process {
        Process::builder("sh").env("PS1", "$ ").spawn()
    }

    #[test]
    fn echo_num_shell_de_verdade() {
        let mut shell = sh();
        shell.write(b"echo ol\xc3\xa1-$((1+1))\r").unwrap();
        let (texto, status) = ler_ate(&mut shell, "olá-2\r\n");
        assert_eq!(status, Status::Open, "{texto}");
        assert!(texto.contains("olá-2\r\n"), "{texto:?}");
    }

    #[test]
    fn o_ambiente_diz_que_e_um_xterm() {
        let mut shell = sh();
        shell.write(b"echo \"[$TERM $COLORTERM]\"\r").unwrap();
        let (texto, _) = ler_ate(&mut shell, "[xterm-256color truecolor]");
        assert!(texto.contains("[xterm-256color truecolor]"), "{texto:?}");
    }

    #[test]
    fn redimensionar_avisa_o_pseudoterminal() {
        let mut shell = sh();
        shell.resize(100, 30);
        shell.write(b"stty size\r").unwrap();
        let (texto, _) = ler_ate(&mut shell, "30 100");
        assert!(texto.contains("30 100"), "{texto:?}");
    }

    #[test]
    fn sair_com_zero_fecha_sem_mensagem() {
        let mut shell = sh();
        shell.write(b"exit\r").unwrap();
        let (texto, status) = ler_ate(&mut shell, "\u{0}");
        assert_eq!(status, Status::Closed(None), "{texto:?}");
    }

    #[test]
    fn sair_com_erro_mostra_o_codigo() {
        let mut shell = sh();
        shell.write(b"exit 3\r").unwrap();
        let (_, status) = ler_ate(&mut shell, "\u{0}");
        assert_eq!(
            status,
            Status::Closed(Some("o processo saiu com o código 3".into()))
        );
    }

    #[test]
    fn programa_que_nao_existe() {
        let mut process = Process::builder("tomni-ui-programa-que-nao-existe").spawn();
        let (_, status) = ler_ate(&mut process, "\u{0}");
        let Status::Closed(Some(mensagem)) = status else {
            panic!("devia fechar com erro: {status:?}");
        };
        assert!(
            mensagem.starts_with("não foi possível iniciar tomni-ui-programa-que-nao-existe: "),
            "{mensagem}"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn o_pid_do_filho_existe_em_proc() {
        let shell = sh();
        let pid = shell.process_id().expect("o filho tem pid");
        assert!(Path::new(&format!("/proc/{pid}")).exists());
        // Reabrir dá outro filho, com outro pid.
        let mut shell = shell;
        assert!(shell.reopen());
        let novo = shell.process_id().expect("o filho novo tem pid");
        assert_ne!(novo, pid);
        assert!(Path::new(&format!("/proc/{novo}")).exists());
    }

    #[test]
    fn duplicate_in_comeca_na_pasta() {
        let pasta = std::env::temp_dir().join("tomni-ui-duplicate-in");
        std::fs::create_dir_all(&pasta).unwrap();
        let pasta = pasta.canonicalize().unwrap();
        let shell = sh();
        let mut copia = shell.duplicate_in(Some(&pasta)).expect("o Process duplica");
        copia.write(b"pwd\r").unwrap();
        let (texto, _) = ler_ate(&mut copia, &format!("{}\r\n", pasta.display()));
        assert!(
            texto.contains(&format!("{}\r\n", pasta.display())),
            "{texto:?}"
        );
    }

    #[test]
    fn duplicate_in_com_pasta_que_nao_existe_abre_como_o_duplicate() {
        let shell = sh();
        let mut copia = shell
            .duplicate_in(Some(Path::new("/tomni-ui/pasta/que/nao/existe")))
            .expect("o Process duplica");
        copia.write(b"echo vivo-$((1+1))\r").unwrap();
        let (texto, status) = ler_ate(&mut copia, "vivo-2");
        assert_eq!(status, Status::Open, "{texto:?}");
        assert!(texto.contains("vivo-2"), "{texto:?}");
    }

    #[test]
    fn a_sessao_duplica_na_pasta_e_diz_o_pid() {
        let pasta = std::env::temp_dir().join("tomni-ui-duplicate-in-sessao");
        std::fs::create_dir_all(&pasta).unwrap();
        let pasta = pasta.canonicalize().unwrap();
        let origem = crate::Session::new(sh());
        assert!(origem.process_id().is_some());
        let copia = origem.duplicate_in(Some(&pasta)).expect("a sessão duplica");
        assert_ne!(copia.process_id(), origem.process_id());
        copia.send(b"pwd\r");
        let limite = Instant::now() + Duration::from_secs(10);
        let esperado = pasta.display().to_string();
        while !copia.text().contains(&esperado) && Instant::now() < limite {
            crate::Session::poll_all();
            thread::sleep(Duration::from_millis(10));
        }
        assert!(copia.text().contains(&esperado), "{}", copia.text());
        origem.dispose();
        copia.dispose();
    }
}
