//! O trait [`Fs`]: o que um sistema de arquivos, local ou remoto, sabe fazer. É bloqueante e
//! roda numa thread de trabalho (a [`Remote`](crate::Remote) ou as [`Transfers`](crate::Transfers)).

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::SystemTime;

use crate::entry::{Caps, Entry};

/// Um sistema de arquivos. Os caminhos são absolutos, com `/` (veja [`path`](crate::path)).
///
/// Os erros são `io::Error` com o `ErrorKind` certo (`NotFound`, `PermissionDenied`,
/// `AlreadyExists`, `Unsupported`, `ConnectionReset`…) e a mensagem em português, com o caminho.
pub trait Fs: Send {
    /// O que este backend sabe fazer.
    fn caps(&self) -> Caps;
    /// A pasta inicial (a casa de quem entrou).
    fn home(&mut self) -> io::Result<String>;
    /// O conteúdo de uma pasta, sem `.` e `..`.
    fn list(&mut self, dir: &str) -> io::Result<Vec<Entry>>;
    /// Os dados de um caminho, seguindo o link.
    fn stat(&mut self, path: &str) -> io::Result<Entry>;
    /// Os dados de um caminho, sem seguir o link.
    fn lstat(&mut self, path: &str) -> io::Result<Entry>;
    /// Abre um arquivo para ler a partir de `offset`.
    fn open_read(&mut self, path: &str, offset: u64) -> io::Result<Box<dyn Download + '_>>;
    /// Abre um arquivo para gravar. Com `offset` 0, cria ou esvazia; com `offset > 0`, grava a
    /// partir dali (retomar). O backend que não sabe devolve `Unsupported`
    /// ([`Caps::resume_upload`] é `false`).
    fn open_write(&mut self, path: &str, offset: u64) -> io::Result<Box<dyn Upload + '_>>;
    /// Como [`open_write`](Fs::open_write), dizendo a data de modificação que o arquivo deve ter
    /// ao fim. Os backends que não mudam a data depois (o S3) a gravam no envio; os outros
    /// ignoram `modified`, e a [`Transfers`](crate::Transfers) chama [`set_modified`](Fs::set_modified).
    fn open_write_with(
        &mut self,
        path: &str,
        offset: u64,
        modified: Option<SystemTime>,
    ) -> io::Result<Box<dyn Upload + '_>> {
        let _ = modified;
        self.open_write(path, offset)
    }
    /// Quantos bytes de `path` o servidor já guarda de um envio interrompido, ou 0. Os backends
    /// que retomam pelo tamanho do destino (SFTP, FTP) não precisam dele: a
    /// [`Transfers`](crate::Transfers) usa o `lstat`. O S3 guarda as partes à parte.
    fn resume_offset(&mut self, path: &str) -> io::Result<u64> {
        let _ = path;
        Ok(0)
    }
    /// Cria uma pasta.
    fn mkdir(&mut self, path: &str) -> io::Result<()>;
    /// Apaga um arquivo (ou link).
    fn remove_file(&mut self, path: &str) -> io::Result<()>;
    /// Apaga uma pasta **vazia**; a recursiva é das [`Transfers`](crate::Transfers).
    fn remove_dir(&mut self, path: &str) -> io::Result<()>;
    /// Renomeia ou move. Com `overwrite`, troca o destino se existir.
    fn rename(&mut self, from: &str, to: &str, overwrite: bool) -> io::Result<()>;
    /// Muda as permissões (os 12 bits do modo).
    fn set_permissions(&mut self, path: &str, mode: u32) -> io::Result<()>;
    /// Muda a data de modificação.
    fn set_modified(&mut self, path: &str, time: SystemTime) -> io::Result<()>;
    /// Cria um link simbólico `link` que aponta para `target`.
    fn symlink(&mut self, target: &str, link: &str) -> io::Result<()>;
    /// Para onde um link aponta.
    fn read_link(&mut self, path: &str) -> io::Result<String>;
    /// O caminho absoluto e canônico de `path`, com os links simbólicos resolvidos (o
    /// `realpath` do SFTP). `Unsupported` onde o protocolo não tem isso. Um caminho que não
    /// existe dá `NotFound`; quem precisa do caminho de algo que ainda não existe resolve o pai.
    fn real_path(&mut self, path: &str) -> io::Result<String> {
        let _ = path;
        Err(unsupported("resolver caminhos"))
    }
    /// Mantém a conexão viva quando está parada (o FTP manda `NOOP`).
    fn keepalive(&mut self) -> io::Result<()> {
        Ok(())
    }
    /// Um aviso para mostrar uma vez, se a conexão tiver um (o FTPS que passou ao TLS 1.2).
    fn take_notice(&mut self) -> Option<String> {
        None
    }
    /// Quem pode copiar sem passar os bytes pelo app: o grupo (o mesmo `rcd` do rclone, por
    /// exemplo) e o nome deste lado dentro dele. Dois backends do mesmo grupo copiam entre si
    /// por [`delegated_copy`](Fs::delegated_copy).
    fn delegation(&self) -> Option<Delegation> {
        None
    }
    /// Copia um arquivo sem passar os bytes pelo app, se este backend sabe (o rclone copia
    /// sozinho entre o disco e a nuvem). `progress` recebe `(feito, total)`; se `cancel` virar
    /// `true`, a cópia para e devolve um erro. `None`: a [`Transfers`](crate::Transfers) copia em
    /// fluxo (`open_read` + `open_write`), como sempre.
    fn delegated_copy(
        &mut self,
        from: &Location,
        to: &Location,
        progress: &dyn Fn(u64, u64),
        cancel: &AtomicBool,
    ) -> Option<io::Result<()>> {
        let _ = (from, to, progress, cancel);
        None
    }
    /// Envia uma pasta do disco inteira para `to`, que ainda não existe, com várias coisas em
    /// voo ao mesmo tempo: o SFTP percorre o disco uma vez, cria as pastas e manda até
    /// [`TreeOptions::files_in_flight`] arquivos juntos, com várias escritas em voo cada um.
    /// `progress` recebe `(feito, total)` em bytes. Se `cancel` virar `true`, para e devolve
    /// `Interrupted` (o que já foi enviado fica). No primeiro erro para tudo e devolve o erro,
    /// com o caminho. `None`: o backend não sabe, e a [`Transfers`](crate::Transfers) envia item
    /// a item, como sempre.
    fn upload_tree(
        &mut self,
        from: &Path,
        to: &str,
        options: &TreeOptions,
        progress: &dyn Fn(u64, u64),
        cancel: &AtomicBool,
    ) -> Option<io::Result<()>> {
        let _ = (from, to, options, progress, cancel);
        None
    }
}

/// Quantos arquivos de uma pasta seguem juntos num [`Fs::upload_tree`], se ninguém disser outro
/// número.
pub const ARQUIVOS_EM_VOO: usize = 32;

/// Como [`Fs::upload_tree`] envia a pasta.
///
/// ```
/// use tomni_ui_files::TreeOptions;
///
/// let opcoes = TreeOptions::default();
/// assert_eq!(opcoes.files_in_flight, tomni_ui_files::ARQUIVOS_EM_VOO);
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeOptions {
    /// Quantos arquivos seguem juntos. O servidor que recusa abrir tantos (o limite de handles
    /// do SFTP) faz o envio cair à metade até caber.
    pub files_in_flight: usize,
    /// Dar a cada arquivo a data de modificação do original.
    pub preserve_date: bool,
    /// Dar a cada arquivo as permissões do original.
    pub preserve_permissions: bool,
}

impl Default for TreeOptions {
    fn default() -> Self {
        Self {
            files_in_flight: ARQUIVOS_EM_VOO,
            preserve_date: crate::PRESERVAR_DATA_PADRAO,
            preserve_permissions: crate::PRESERVAR_PERMISSOES_PADRAO,
        }
    }
}

/// O grupo de backends que copiam entre si sem passar os bytes pelo app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Delegation {
    /// Identifica o grupo (dois backends só copiam entre si se o grupo for o mesmo).
    pub group: usize,
    /// O nome deste lado no grupo (`gdrive:` ou `gdrive:Backup`).
    pub remote: String,
}

/// Uma ponta de uma [`delegated_copy`](Fs::delegated_copy).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Location {
    /// Um caminho do disco desta máquina.
    Local(PathBuf),
    /// Um caminho deste backend.
    Here(String),
    /// Um caminho de outro backend do mesmo grupo.
    Peer(Delegation, String),
}

/// Um arquivo aberto para leitura.
pub trait Download: Read + Send {
    /// Fecha e confere o fim. No FTP só se sabe se deu certo depois de ler a resposta final da
    /// conexão de dados; no SFTP, fecha o handle e confere o status. Soltar sem chamar é
    /// cancelar.
    fn finish(self: Box<Self>) -> io::Result<()>;
}

/// Um arquivo aberto para gravação.
pub trait Upload: Write + Send {
    /// Fecha e confere o fim. Soltar sem chamar é cancelar (o arquivo parcial fica).
    fn finish(self: Box<Self>) -> io::Result<()>;
}

/// O erro de "não pode, o backend não sabe".
pub fn unsupported(what: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        format!("não é possível {what} neste tipo de conexão"),
    )
}

/// `true` nos erros que indicam que a conexão caiu (e vale reabrir e repetir).
pub fn is_connection_error(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::BrokenPipe
            | io::ErrorKind::UnexpectedEof
            | io::ErrorKind::TimedOut
            | io::ErrorKind::NotConnected
    )
}

/// Refaz `error` com a mensagem em português que diz o que se tentava (`action`: "ler",
/// "gravar em", "apagar"…) e em qual `path`. O `ErrorKind` não muda.
///
/// ```
/// use std::io;
/// use tomni_ui_files::describe;
///
/// let erro = describe(io::ErrorKind::PermissionDenied.into(), "ler", "/etc/shadow");
/// assert_eq!(erro.kind(), io::ErrorKind::PermissionDenied);
/// assert_eq!(erro.to_string(), "sem permissão para ler /etc/shadow");
/// ```
pub fn describe(error: io::Error, action: &str, path: &str) -> io::Error {
    let kind = error.kind();
    let message = match kind {
        io::ErrorKind::NotFound => format!("não encontrado: {path}"),
        io::ErrorKind::PermissionDenied => format!("sem permissão para {action} {path}"),
        io::ErrorKind::AlreadyExists => format!("já existe: {path}"),
        io::ErrorKind::DirectoryNotEmpty => format!("a pasta não está vazia: {path}"),
        io::ErrorKind::NotADirectory => format!("não é uma pasta: {path}"),
        io::ErrorKind::IsADirectory => format!("é uma pasta: {path}"),
        _ => format!("{}: {path}", tomni_ui_net::error_message(&error)),
    };
    io::Error::new(kind, message)
}

/// O servidor recusou abrir mais uma conexão (o `421 Too many connections` do FTP): quem abre
/// conexões a mais do que ele deixa usa menos delas.
#[derive(Debug)]
struct ConnectionLimit(String);

impl std::fmt::Display for ConnectionLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ConnectionLimit {}

/// O erro de "o servidor não aceita mais conexões minhas"; `message` é o texto que o usuário vê.
pub fn connection_limit(message: impl Into<String>) -> io::Error {
    io::Error::new(
        io::ErrorKind::ConnectionRefused,
        ConnectionLimit(message.into()),
    )
}

/// `true` se `error` veio de [`connection_limit`].
pub fn is_connection_limit(error: &io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|inner| inner.is::<ConnectionLimit>())
}
