//! O que um [`Fs`](crate::Fs) conta sobre um arquivo, e o que ele sabe fazer.

use std::time::SystemTime;

/// O tipo de uma entrada.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EntryKind {
    /// Um arquivo comum.
    File,
    /// Uma pasta.
    Dir,
    /// Um link simbólico (só no `lstat` e na listagem; o `stat` segue o link).
    Symlink,
    /// Qualquer outra coisa (dispositivo, socket, fila).
    Other,
}

/// Uma entrada de uma pasta.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// O nome, sem o caminho.
    pub name: String,
    /// O tipo.
    pub kind: EntryKind,
    /// O tamanho em bytes (0 numa pasta que o protocolo não mede).
    pub size: u64,
    /// A data da última modificação, se o protocolo diz.
    pub modified: Option<SystemTime>,
    /// Os 12 bits do modo (`0o7777`); `None` se o protocolo não diz.
    pub permissions: Option<u32>,
    /// O dono: o nome, ou o número em texto se só houver o número.
    pub owner: Option<String>,
    /// O grupo, como o dono.
    pub group: Option<String>,
    /// Para onde um link aponta, se for um link e o protocolo diz.
    pub link_target: Option<String>,
}

impl Entry {
    /// Uma entrada sem dados além do nome e do tipo.
    pub fn new(name: impl Into<String>, kind: EntryKind) -> Self {
        Self {
            name: name.into(),
            kind,
            size: 0,
            modified: None,
            permissions: None,
            owner: None,
            group: None,
            link_target: None,
        }
    }

    /// `true` numa pasta.
    pub fn is_dir(&self) -> bool {
        self.kind == EntryKind::Dir
    }

    /// `true` num arquivo comum.
    pub fn is_file(&self) -> bool {
        self.kind == EntryKind::File
    }
}

/// O que o backend sabe fazer. A tela esconde ou desabilita o que não existe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caps {
    /// Lê e muda as permissões.
    pub permissions: bool,
    /// Muda a data de modificação.
    pub set_modified: bool,
    /// Entende links simbólicos.
    pub symlinks: bool,
    /// Grava a partir de um deslocamento (retomar um envio).
    pub resume_upload: bool,
    /// Informa o dono e o grupo.
    pub owner: bool,
}

impl Caps {
    /// Tudo permitido.
    pub const ALL: Caps = Caps {
        permissions: true,
        set_modified: true,
        symlinks: true,
        resume_upload: true,
        owner: true,
    };
}
