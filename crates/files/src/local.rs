//! O [`Local`]: o sistema de arquivos da máquina, pelo `std::fs`.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::entry::{Caps, Entry, EntryKind};
#[cfg(not(unix))]
use crate::fs::unsupported;
use crate::fs::{Download, Fs, Upload, describe};
use crate::path;

/// O sistema de arquivos local. Os caminhos do [`Fs`] têm `/` (no Windows, `C:/pasta`).
#[derive(Clone, Copy, Debug, Default)]
pub struct Local;

impl Local {
    /// O sistema de arquivos local.
    pub fn new() -> Self {
        Self
    }
}

fn native(path: &str) -> PathBuf {
    #[cfg(windows)]
    {
        PathBuf::from(path.replace('/', "\\"))
    }
    #[cfg(not(windows))]
    {
        PathBuf::from(path)
    }
}

fn text(path: &Path) -> String {
    path::normalize(&path.to_string_lossy())
}

/// O alvo de um link, como está (um alvo relativo continua relativo).
fn link_text(target: &Path) -> String {
    target.to_string_lossy().replace('\\', "/")
}

#[cfg(unix)]
fn mode_of(meta: &fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(meta.permissions().mode() & 0o7777)
}

#[cfg(not(unix))]
fn mode_of(_: &fs::Metadata) -> Option<u32> {
    None
}

#[cfg(unix)]
fn owner_of(meta: &fs::Metadata) -> (Option<String>, Option<String>) {
    use std::os::unix::fs::MetadataExt;
    (Some(meta.uid().to_string()), Some(meta.gid().to_string()))
}

#[cfg(not(unix))]
fn owner_of(_: &fs::Metadata) -> (Option<String>, Option<String>) {
    (None, None)
}

fn entry_of(name: &str, meta: &fs::Metadata, link: Option<String>) -> Entry {
    let file_type = meta.file_type();
    let kind = if file_type.is_symlink() {
        EntryKind::Symlink
    } else if file_type.is_dir() {
        EntryKind::Dir
    } else if file_type.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    };
    let (owner, group) = owner_of(meta);
    Entry {
        name: name.to_owned(),
        kind,
        size: if kind == EntryKind::Dir {
            0
        } else {
            meta.len()
        },
        modified: meta.modified().ok(),
        permissions: mode_of(meta),
        owner,
        group,
        link_target: link,
    }
}

impl Fs for Local {
    fn caps(&self) -> Caps {
        Caps {
            permissions: cfg!(unix),
            set_modified: true,
            symlinks: cfg!(unix),
            resume_upload: true,
            owner: cfg!(unix),
        }
    }

    fn home(&mut self) -> io::Result<String> {
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .filter(|p| p.is_dir());
        Ok(match home {
            Some(home) => text(&home),
            None => text(&std::env::current_dir()?),
        })
    }

    fn list(&mut self, dir: &str) -> io::Result<Vec<Entry>> {
        let mut entries = Vec::new();
        for item in fs::read_dir(native(dir)).map_err(|e| describe(e, "ler", dir))? {
            let item = item.map_err(|e| describe(e, "ler", dir))?;
            let name = item.file_name().to_string_lossy().into_owned();
            // Um arquivo que sumiu entre a listagem e o `stat` é pulado.
            let Ok(meta) = fs::symlink_metadata(item.path()) else {
                continue;
            };
            let link = meta
                .file_type()
                .is_symlink()
                .then(|| fs::read_link(item.path()).ok().map(|p| link_text(&p)))
                .flatten();
            entries.push(entry_of(&name, &meta, link));
        }
        Ok(entries)
    }

    fn stat(&mut self, path: &str) -> io::Result<Entry> {
        let meta = fs::metadata(native(path)).map_err(|e| describe(e, "ler", path))?;
        Ok(entry_of(path::file_name(path), &meta, None))
    }

    fn lstat(&mut self, path: &str) -> io::Result<Entry> {
        let p = native(path);
        let meta = fs::symlink_metadata(&p).map_err(|e| describe(e, "ler", path))?;
        let link = meta
            .file_type()
            .is_symlink()
            .then(|| fs::read_link(&p).ok().map(|t| link_text(&t)))
            .flatten();
        Ok(entry_of(path::file_name(path), &meta, link))
    }

    fn open_read(&mut self, path: &str, offset: u64) -> io::Result<Box<dyn Download + '_>> {
        let mut file = File::open(native(path)).map_err(|e| describe(e, "ler", path))?;
        if file
            .metadata()
            .map_err(|e| describe(e, "ler", path))?
            .is_dir()
        {
            return Err(describe(io::ErrorKind::IsADirectory.into(), "ler", path));
        }
        if offset > 0 {
            file.seek(SeekFrom::Start(offset))?;
        }
        Ok(Box::new(LocalFile { file }))
    }

    fn open_write(&mut self, path: &str, offset: u64) -> io::Result<Box<dyn Upload + '_>> {
        let mut options = OpenOptions::new();
        options.write(true);
        if offset == 0 {
            options.create(true).truncate(true);
        }
        let mut file = options
            .open(native(path))
            .map_err(|e| describe(e, "gravar em", path))?;
        if offset > 0 {
            file.set_len(offset)
                .and_then(|()| file.seek(SeekFrom::Start(offset)).map(drop))
                .map_err(|e| describe(e, "gravar em", path))?;
        }
        Ok(Box::new(LocalFile { file }))
    }

    fn mkdir(&mut self, path: &str) -> io::Result<()> {
        fs::create_dir(native(path)).map_err(|e| describe(e, "criar", path))
    }

    fn remove_file(&mut self, path: &str) -> io::Result<()> {
        fs::remove_file(native(path)).map_err(|e| describe(e, "apagar", path))
    }

    fn remove_dir(&mut self, path: &str) -> io::Result<()> {
        fs::remove_dir(native(path)).map_err(|e| describe(e, "apagar", path))
    }

    fn rename(&mut self, from: &str, to: &str, overwrite: bool) -> io::Result<()> {
        if !overwrite && fs::symlink_metadata(native(to)).is_ok() {
            return Err(describe(
                io::ErrorKind::AlreadyExists.into(),
                "renomear",
                to,
            ));
        }
        fs::rename(native(from), native(to)).map_err(|e| describe(e, "renomear", from))
    }

    #[cfg(unix)]
    fn set_permissions(&mut self, path: &str, mode: u32) -> io::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(native(path), fs::Permissions::from_mode(mode & 0o7777))
            .map_err(|e| describe(e, "mudar as permissões de", path))
    }

    #[cfg(not(unix))]
    fn set_permissions(&mut self, path: &str, mode: u32) -> io::Result<()> {
        // No Windows só existe o "somente leitura".
        let p = native(path);
        let mut permissions = fs::metadata(&p)
            .map_err(|e| describe(e, "mudar as permissões de", path))?
            .permissions();
        permissions.set_readonly(mode & 0o200 == 0);
        fs::set_permissions(&p, permissions)
            .map_err(|e| describe(e, "mudar as permissões de", path))
    }

    fn set_modified(&mut self, path: &str, time: SystemTime) -> io::Result<()> {
        let p = native(path);
        let file = if fs::metadata(&p).map(|m| m.is_dir()).unwrap_or(false) {
            File::open(&p)
        } else {
            OpenOptions::new().write(true).open(&p)
        }
        .map_err(|e| describe(e, "mudar a data de", path))?;
        file.set_modified(time)
            .map_err(|e| describe(e, "mudar a data de", path))
    }

    #[cfg(unix)]
    fn symlink(&mut self, target: &str, link: &str) -> io::Result<()> {
        std::os::unix::fs::symlink(target, native(link)).map_err(|e| describe(e, "criar", link))
    }

    #[cfg(not(unix))]
    fn symlink(&mut self, _target: &str, _link: &str) -> io::Result<()> {
        Err(unsupported("criar links simbólicos"))
    }

    fn read_link(&mut self, path: &str) -> io::Result<String> {
        fs::read_link(native(path))
            .map(|p| link_text(&p))
            .map_err(|e| describe(e, "ler", path))
    }

    fn real_path(&mut self, path: &str) -> io::Result<String> {
        let real = fs::canonicalize(native(path)).map_err(|e| describe(e, "ler", path))?;
        // No Windows o `canonicalize` devolve o prefixo `\\?\`, que o resto do app não usa.
        let real = real.to_string_lossy();
        let real = real.strip_prefix(r"\\?\").unwrap_or(&real);
        Ok(path::normalize(real))
    }
}

/// Um arquivo local aberto, para leitura ou gravação.
struct LocalFile {
    file: File,
}

impl Read for LocalFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.file.read(buf)
    }
}

impl Write for LocalFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.file.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl Download for LocalFile {
    fn finish(self: Box<Self>) -> io::Result<()> {
        Ok(())
    }
}

impl Upload for LocalFile {
    fn finish(mut self: Box<Self>) -> io::Result<()> {
        self.file.flush()
    }
}
