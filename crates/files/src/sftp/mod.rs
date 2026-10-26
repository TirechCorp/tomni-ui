//! O SFTP: um [`Fs`] sobre uma conexão SSH, com o `russh-sftp`.
//!
//! [`Sftp::connect`] abre uma conexão própria (com os saltos, o proxy e a autenticação do
//! `SshBuilder`). Cada operação bloqueia na thread de quem chama (a [`Remote`](crate::Remote)
//! ou um trabalhador das [`Transfers`](crate::Transfers)), com o runtime do tokio da conexão.

use std::future::Future;
use std::io::{self, Read, SeekFrom, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use russh_sftp::client::error::Error as SftpError;
use russh_sftp::client::fs::{File, Metadata};
use russh_sftp::client::{Config, SftpSession};
use russh_sftp::protocol::{FileAttributes, FileType, OpenFlags, StatusCode};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::runtime::{Builder, Runtime};
use tomni_ui_terminal::ssh::{SshBuilder, SshChannelOpener, SshLink};

use crate::entry::{Caps, Entry, EntryKind};
use crate::fs::{Download, Fs, TreeOptions, Upload, describe};
use crate::path;

mod tree;

/// Quanto esperar a resposta de cada pedido antes de dar a conexão por perdida.
const REQUEST_TIMEOUT_SECS: u64 = 30;

/// Quantas escritas de um arquivo seguem sem esperar a resposta da anterior.
const ESCRITAS_EM_VOO: usize = 64;

/// Quantas leituras de um arquivo seguem sem esperar a resposta da anterior.
const LEITURAS_EM_VOO: usize = 64;

/// A configuração da sessão do `russh-sftp`: mais escritas e leituras em voo que o padrão dele
/// (16), que numa conexão com latência deixa o canal parado esperando respostas.
fn config() -> Config {
    Config {
        max_concurrent_writes: ESCRITAS_EM_VOO,
        max_concurrent_reads: LEITURAS_EM_VOO,
        request_timeout_secs: REQUEST_TIMEOUT_SECS,
        ..Config::default()
    }
}

/// Quanto esperar o servidor aceitar o canal do SFTP numa sessão aberta.
const CHANNEL_TIMEOUT: Duration = Duration::from_secs(20);

/// Quanto esperar o servidor confirmar o fechamento do SFTP ao soltar o [`Sftp`].
const CLOSE_TIMEOUT: Duration = Duration::from_secs(2);

/// De onde vem o runtime que move a conexão.
enum Driver {
    /// Uma conexão própria.
    Link(SshLink),
    /// Um canal na conexão de um terminal; só o runtime que lê e escreve nele é nosso.
    Runtime(Runtime),
}

impl Driver {
    fn block_on<F: Future>(&self, future: F) -> F::Output {
        match self {
            Driver::Link(link) => link.block_on(future),
            Driver::Runtime(runtime) => runtime.block_on(future),
        }
    }

    fn is_open(&self) -> bool {
        match self {
            Driver::Link(link) => link.is_open(),
            // Sem o `Handle`, a queda da conexão aparece como erro do próprio pedido.
            Driver::Runtime(_) => true,
        }
    }

    fn disconnect(&mut self) {
        match self {
            Driver::Link(link) => link.disconnect(),
            Driver::Runtime(_) => {}
        }
    }
}

/// Um sistema de arquivos remoto pelo SFTP.
///
/// ```no_run
/// use tomni_ui_files::{Fs, sftp::Sftp};
/// use tomni_ui_terminal::ssh::SshBuilder;
///
/// let mut sftp = Sftp::connect(&SshBuilder::new("servidor", 22, "ana").password("segredo"))?;
/// let casa = sftp.home()?;
/// for entrada in sftp.list(&casa)? {
///     println!("{}", entrada.name);
/// }
/// # Ok::<(), std::io::Error>(())
/// ```
pub struct Sftp {
    // A sessão vem antes: solta primeiro, com o runtime ainda vivo.
    session: Arc<SftpSession>,
    driver: Driver,
}

impl Sftp {
    /// Conecta, autentica e abre o subsistema `sftp`. Bloqueia até terminar; o erro traz a
    /// mensagem em português (servidor recusou, senha errada, chave do servidor mudou…).
    pub fn connect(builder: &SshBuilder) -> io::Result<Self> {
        let link = builder.connect_link().map_err(io::Error::other)?;
        let channel = link.open_subsystem("sftp").map_err(io::Error::other)?;
        let session = link
            .block_on(SftpSession::new_with_config(channel, config()))
            .map_err(|error| fail(&error, "abrir o SFTP em", "o servidor"))?;
        Ok(Self {
            session: Arc::new(session),
            driver: Driver::Link(link),
        })
    }

    /// Abre o SFTP num canal novo da conexão SSH de um terminal
    /// ([`Session::ssh_channels`](tomni_ui_terminal::Session::ssh_channels)): não autentica de
    /// novo nem abre outra conexão TCP. Se a sessão acabou, o erro é `ConnectionReset`.
    pub fn from_channel(opener: &SshChannelOpener) -> io::Result<Self> {
        let channel = opener.open_subsystem("sftp", CHANNEL_TIMEOUT)?;
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(io::Error::other)?;
        let session = runtime
            .block_on(SftpSession::new_with_config(channel, config()))
            .map_err(|error| fail(&error, "abrir o SFTP em", "o servidor"))?;
        Ok(Self {
            session: Arc::new(session),
            driver: Driver::Runtime(runtime),
        })
    }

    fn run<T>(
        &self,
        action: &str,
        path: &str,
        future: impl Future<Output = Result<T, SftpError>>,
    ) -> io::Result<T> {
        if !self.driver.is_open() {
            return Err(describe(
                io::ErrorKind::ConnectionReset.into(),
                action,
                path,
            ));
        }
        self.driver
            .block_on(future)
            .map_err(|error| fail(&error, action, path))
    }

    /// Se o caminho existe (sem seguir o link).
    fn exists(&self, path: &str) -> bool {
        self.driver
            .block_on(self.session.symlink_metadata(path))
            .is_ok()
    }

    /// Lê o alvo de um link, se o servidor disser.
    fn link_target(&self, path: &str) -> Option<String> {
        self.driver.block_on(self.session.read_link(path)).ok()
    }
}

impl Drop for Sftp {
    fn drop(&mut self) {
        // Fecha o subsistema e desconecta com o runtime ainda vivo.
        let _ = self
            .driver
            .block_on(async { tokio::time::timeout(CLOSE_TIMEOUT, self.session.close()).await });
        self.driver.disconnect();
    }
}

/// Traduz um erro do `russh-sftp` para um `io::Error` com o `ErrorKind` certo e a mensagem em
/// português (`action` e `path` dizem o que se tentava).
fn fail(error: &SftpError, action: &str, path: &str) -> io::Error {
    let kind = match error {
        SftpError::Status(status) => match status.status_code {
            StatusCode::NoSuchFile => io::ErrorKind::NotFound,
            StatusCode::PermissionDenied => io::ErrorKind::PermissionDenied,
            StatusCode::Eof => io::ErrorKind::UnexpectedEof,
            StatusCode::BadMessage => io::ErrorKind::InvalidData,
            StatusCode::NoConnection | StatusCode::ConnectionLost => io::ErrorKind::ConnectionReset,
            StatusCode::OpUnsupported => io::ErrorKind::Unsupported,
            StatusCode::Ok | StatusCode::Failure => io::ErrorKind::Other,
        },
        SftpError::Timeout => io::ErrorKind::TimedOut,
        SftpError::Limited(_) => io::ErrorKind::InvalidInput,
        // O canal fechou, o servidor caiu ou o pedido nem saiu.
        SftpError::IO(_) | SftpError::UnexpectedPacket | SftpError::UnexpectedBehavior(_) => {
            io::ErrorKind::ConnectionReset
        }
    };
    let detail = match error {
        SftpError::Status(status) if kind == io::ErrorKind::Other => {
            let text = status.error_message.trim();
            if text.is_empty() || text == "Failure" {
                "o servidor recusou".to_owned()
            } else {
                format!("o servidor recusou ({text})")
            }
        }
        _ => error.to_string(),
    };
    describe(io::Error::new(kind, detail), action, path)
}

/// O mesmo de [`fail`] para o erro de uma leitura ou gravação, que o `russh-sftp` entrega
/// como `io::Error` com o erro original dentro.
fn fail_io(error: io::Error, action: &str, path: &str) -> io::Error {
    match error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<SftpError>())
    {
        Some(inner) => fail(inner, action, path),
        None if error.kind() == io::ErrorKind::Other => {
            // Um status de gravação sem o erro original (só a mensagem do servidor).
            let text = error.to_string();
            describe(
                io::Error::new(
                    if text.to_lowercase().contains("permission") {
                        io::ErrorKind::PermissionDenied
                    } else {
                        io::ErrorKind::Other
                    },
                    format!("o servidor recusou ({text})"),
                ),
                action,
                path,
            )
        }
        None => describe(error, action, path),
    }
}

fn entry_kind(file_type: FileType) -> EntryKind {
    match file_type {
        FileType::Dir => EntryKind::Dir,
        FileType::File => EntryKind::File,
        FileType::Symlink => EntryKind::Symlink,
        FileType::Other => EntryKind::Other,
    }
}

fn entry_of(name: &str, attrs: &Metadata, link_target: Option<String>) -> Entry {
    let kind = entry_kind(attrs.file_type());
    Entry {
        name: name.to_owned(),
        kind,
        size: if kind == EntryKind::Dir {
            0
        } else {
            attrs.len()
        },
        modified: attrs.modified().ok(),
        permissions: attrs.permissions.map(|mode| mode & 0o7777),
        owner: attrs
            .user
            .clone()
            .or_else(|| attrs.uid.map(|uid| uid.to_string())),
        group: attrs
            .group
            .clone()
            .or_else(|| attrs.gid.map(|gid| gid.to_string())),
        link_target,
    }
}

pub(super) fn unix_seconds(time: SystemTime) -> u32 {
    time.duration_since(UNIX_EPOCH)
        .map_or(0, |d| u32::try_from(d.as_secs()).unwrap_or(u32::MAX))
}

impl Fs for Sftp {
    fn caps(&self) -> Caps {
        Caps::ALL
    }

    fn home(&mut self) -> io::Result<String> {
        self.run("ler", ".", self.session.canonicalize("."))
    }

    fn list(&mut self, dir: &str) -> io::Result<Vec<Entry>> {
        let items = self.run("ler", dir, self.session.read_dir(dir))?;
        let mut entries = Vec::new();
        for item in items {
            let name = item.file_name();
            let attrs = item.metadata();
            let link = (attrs.file_type() == FileType::Symlink)
                .then(|| self.link_target(&path::join(dir, &name)))
                .flatten();
            entries.push(entry_of(&name, &attrs, link));
        }
        Ok(entries)
    }

    fn stat(&mut self, path: &str) -> io::Result<Entry> {
        let attrs = self.run("ler", path, self.session.metadata(path))?;
        Ok(entry_of(path::file_name(path), &attrs, None))
    }

    fn lstat(&mut self, path: &str) -> io::Result<Entry> {
        let attrs = self.run("ler", path, self.session.symlink_metadata(path))?;
        let link = (attrs.file_type() == FileType::Symlink)
            .then(|| self.link_target(path))
            .flatten();
        Ok(entry_of(path::file_name(path), &attrs, link))
    }

    fn open_read(&mut self, path: &str, offset: u64) -> io::Result<Box<dyn Download + '_>> {
        let attrs = self.run("ler", path, self.session.metadata(path))?;
        if attrs.file_type() == FileType::Dir {
            return Err(describe(io::ErrorKind::IsADirectory.into(), "ler", path));
        }
        let mut file = self.run("ler", path, self.session.open(path))?;
        if offset > 0 {
            self.driver
                .block_on(file.seek(SeekFrom::Start(offset)))
                .map_err(|e| fail_io(e, "ler", path))?;
        }
        Ok(Box::new(RemoteFile {
            driver: &self.driver,
            file,
            path: path.to_owned(),
        }))
    }

    fn open_write(&mut self, path: &str, offset: u64) -> io::Result<Box<dyn Upload + '_>> {
        let flags = if offset == 0 {
            OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE
        } else {
            OpenFlags::WRITE
        };
        let mut file = self.run("gravar em", path, self.session.open_with_flags(path, flags))?;
        if offset > 0 {
            self.driver
                .block_on(file.seek(SeekFrom::Start(offset)))
                .map_err(|e| fail_io(e, "gravar em", path))?;
        }
        Ok(Box::new(RemoteFile {
            driver: &self.driver,
            file,
            path: path.to_owned(),
        }))
    }

    fn mkdir(&mut self, path: &str) -> io::Result<()> {
        match self.run("criar", path, self.session.create_dir(path)) {
            // O OpenSSH responde "Failure" para uma pasta que já existe.
            Err(error) if error.kind() == io::ErrorKind::Other && self.exists(path) => {
                Err(describe(io::ErrorKind::AlreadyExists.into(), "criar", path))
            }
            other => other,
        }
    }

    fn remove_file(&mut self, path: &str) -> io::Result<()> {
        self.run("apagar", path, self.session.remove_file(path))
    }

    fn remove_dir(&mut self, path: &str) -> io::Result<()> {
        match self.run("apagar", path, self.session.remove_dir(path)) {
            // O OpenSSH responde "Failure" para uma pasta com coisas dentro.
            Err(error) if error.kind() == io::ErrorKind::Other => {
                let filled = self
                    .driver
                    .block_on(self.session.read_dir(path))
                    .is_ok_and(|mut items| items.next().is_some());
                if filled {
                    Err(describe(
                        io::ErrorKind::DirectoryNotEmpty.into(),
                        "apagar",
                        path,
                    ))
                } else {
                    Err(error)
                }
            }
            other => other,
        }
    }

    fn rename(&mut self, from: &str, to: &str, overwrite: bool) -> io::Result<()> {
        if self.exists(to) {
            if !overwrite {
                return Err(describe(
                    io::ErrorKind::AlreadyExists.into(),
                    "renomear",
                    to,
                ));
            }
            // O SFTP (versão 3) não troca o destino: apaga e renomeia.
            let target = self.lstat(to)?;
            if target.is_dir() {
                self.remove_dir(to)?;
            } else {
                self.remove_file(to)?;
            }
        }
        self.run("renomear", from, self.session.rename(from, to))
    }

    fn set_permissions(&mut self, path: &str, mode: u32) -> io::Result<()> {
        let attrs = FileAttributes {
            permissions: Some(mode & 0o7777),
            ..FileAttributes::empty()
        };
        self.run(
            "mudar as permissões de",
            path,
            self.session.set_metadata(path, attrs),
        )
    }

    fn set_modified(&mut self, path: &str, time: SystemTime) -> io::Result<()> {
        let seconds = unix_seconds(time);
        let attrs = FileAttributes {
            atime: Some(seconds),
            mtime: Some(seconds),
            ..FileAttributes::empty()
        };
        self.run(
            "mudar a data de",
            path,
            self.session.set_metadata(path, attrs),
        )
    }

    fn symlink(&mut self, target: &str, link: &str) -> io::Result<()> {
        // O servidor (OpenSSH) lê o alvo primeiro, ao contrário do rascunho do protocolo: é
        // o que o `sftp` do OpenSSH manda. O primeiro argumento do `russh-sftp` vai na frente.
        self.run("criar", link, self.session.symlink(target, link))
    }

    fn read_link(&mut self, path: &str) -> io::Result<String> {
        self.run("ler", path, self.session.read_link(path))
    }

    fn real_path(&mut self, path: &str) -> io::Result<String> {
        self.run("ler", path, self.session.canonicalize(path))
    }

    fn upload_tree(
        &mut self,
        from: &Path,
        to: &str,
        options: &TreeOptions,
        progress: &dyn Fn(u64, u64),
        cancel: &AtomicBool,
    ) -> Option<io::Result<()>> {
        Some(tree::enviar(
            &self.session,
            &self.driver,
            from,
            to,
            options,
            progress,
            cancel,
        ))
    }

    fn keepalive(&mut self) -> io::Result<()> {
        self.run(
            "manter a conexão com",
            "o servidor",
            self.session.canonicalize("."),
        )
        .map(drop)
    }
}

/// Um arquivo remoto aberto, para ler ou gravar.
struct RemoteFile<'a> {
    driver: &'a Driver,
    file: File,
    path: String,
}

impl Read for RemoteFile<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.driver
            .block_on(self.file.read(buf))
            .map_err(|e| fail_io(e, "ler", &self.path))
    }
}

impl Download for RemoteFile<'_> {
    fn finish(self: Box<Self>) -> io::Result<()> {
        let Self {
            driver, file, path, ..
        } = *self;
        driver
            .block_on(file.close())
            .map_err(|e| fail_io(e, "ler", &path))
    }
}

impl Write for RemoteFile<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.driver
            .block_on(self.file.write(buf))
            .map_err(|e| fail_io(e, "gravar em", &self.path))
    }

    fn flush(&mut self) -> io::Result<()> {
        self.driver
            .block_on(self.file.flush())
            .map_err(|e| fail_io(e, "gravar em", &self.path))
    }
}

impl Upload for RemoteFile<'_> {
    fn finish(self: Box<Self>) -> io::Result<()> {
        let Self {
            driver, file, path, ..
        } = *self;
        driver
            .block_on(file.close())
            .map_err(|e| fail_io(e, "gravar em", &path))
    }
}
