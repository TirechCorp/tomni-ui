//! O [`MemFs`]: um sistema de arquivos em memória, para os testes das telas e da fila (sem
//! rede nem disco). Todas as cópias (`connect`) enxergam a mesma árvore, como duas conexões
//! no mesmo servidor.

use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::entry::{Caps, Entry, EntryKind};
use crate::fs::{Download, Fs, Upload, describe, unsupported};
use crate::path;
use crate::remote::Opener;

/// A data de tudo o que o `MemFs` cria, para os testes não dependerem do relógio (2023-11-14).
pub fn mem_time() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_700_000_000)
}

#[derive(Clone)]
struct Node {
    kind: EntryKind,
    data: Vec<u8>,
    modified: Option<SystemTime>,
    permissions: u32,
    link: Option<String>,
}

struct Tree {
    nodes: BTreeMap<String, Node>,
    generation: u64,
    caps: Caps,
    chunk: usize,
    delay: Duration,
    clock: SystemTime,
    denied: Vec<String>,
    connections: usize,
    lists: usize,
    open_files: usize,
    max_open_files: usize,
}

struct Shared {
    tree: Mutex<Tree>,
}

/// Um sistema de arquivos em memória. É um `Clone`/`connect` barato: todas as cópias dividem
/// a árvore.
pub struct MemFs {
    shared: Arc<Shared>,
    generation: u64,
}

impl Default for MemFs {
    fn default() -> Self {
        Self::new()
    }
}

impl MemFs {
    /// Um servidor novo, só com a raiz e a pasta `/home/ana` (a [`home`](Fs::home)).
    pub fn new() -> Self {
        let mut nodes = BTreeMap::new();
        for dir in ["/", "/home", "/home/ana"] {
            nodes.insert(dir.to_owned(), dir_node());
        }
        Self {
            shared: Arc::new(Shared {
                tree: Mutex::new(Tree {
                    nodes,
                    generation: 0,
                    caps: Caps::ALL,
                    chunk: 64 * 1024,
                    delay: Duration::ZERO,
                    clock: mem_time(),
                    denied: Vec::new(),
                    connections: 0,
                    lists: 0,
                    open_files: 0,
                    max_open_files: 0,
                }),
            }),
            generation: 0,
        }
    }

    fn tree(&self) -> MutexGuard<'_, Tree> {
        self.shared
            .tree
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Uma conexão nova no mesmo servidor.
    pub fn connect(&self) -> MemFs {
        let mut tree = self.tree();
        tree.connections += 1;
        MemFs {
            shared: Arc::clone(&self.shared),
            generation: tree.generation,
        }
    }

    /// Um [`Opener`] que devolve uma conexão nova a cada chamada.
    pub fn opener(&self) -> Opener {
        let shared = Arc::clone(&self.shared);
        Arc::new(move || {
            let fs = MemFs {
                shared: Arc::clone(&shared),
                generation: 0,
            };
            Ok(Box::new(fs.connect()) as Box<dyn Fs>)
        })
    }

    /// Quantas conexões já foram abertas (`connect` e o `opener`).
    pub fn connections(&self) -> usize {
        self.tree().connections
    }

    /// Quantas listagens de pasta ([`Fs::list`]) o servidor já fez, de todas as conexões (para
    /// provar que uma navegação lista a pasta uma vez só).
    pub fn lists(&self) -> usize {
        self.tree().lists
    }

    /// O maior número de arquivos abertos ao mesmo tempo (para provar as paralelas).
    pub fn max_open_files(&self) -> usize {
        self.tree().max_open_files
    }

    /// Derruba todas as conexões abertas: a próxima operação de cada uma falha com
    /// `ConnectionReset`. Quem abrir de novo (pelo `opener`) funciona.
    pub fn drop_connections(&self) {
        self.tree().generation += 1;
    }

    /// Limita cada leitura e gravação a `bytes` (para ver o progresso andar).
    pub fn set_chunk(&self, bytes: usize) {
        self.tree().chunk = bytes.max(1);
    }

    /// Espera `delay` a cada bloco lido ou gravado.
    pub fn set_delay(&self, delay: Duration) {
        self.tree().delay = delay;
    }

    /// Troca o que o backend diz saber fazer.
    pub fn set_caps(&self, caps: Caps) {
        self.tree().caps = caps;
    }

    /// A data que as gravações e pastas novas recebem.
    pub fn set_clock(&self, time: SystemTime) {
        self.tree().clock = time;
    }

    /// Passa a negar (`PermissionDenied`) qualquer acesso a `path` e ao que há dentro.
    pub fn deny(&self, path: &str) {
        self.tree().denied.push(path::normalize(path));
    }

    /// Cria (ou troca) um arquivo, e as pastas que faltam até ele.
    pub fn put(&self, path: &str, data: impl AsRef<[u8]>) -> &Self {
        let path = path::normalize(path);
        let mut tree = self.tree();
        make_parents(&mut tree, &path);
        let modified = Some(tree.clock);
        tree.nodes.insert(
            path,
            Node {
                kind: EntryKind::File,
                data: data.as_ref().to_vec(),
                modified,
                permissions: 0o644,
                link: None,
            },
        );
        self
    }

    /// Cria uma pasta (e as que faltam).
    pub fn dir(&self, path: &str) -> &Self {
        let path = path::normalize(path);
        let mut tree = self.tree();
        make_parents(&mut tree, &path);
        let modified = Some(tree.clock);
        tree.nodes.entry(path).or_insert_with(|| Node {
            modified,
            ..dir_node()
        });
        self
    }

    /// Cria um link simbólico.
    pub fn link(&self, path: &str, target: &str) -> &Self {
        let path = path::normalize(path);
        let mut tree = self.tree();
        make_parents(&mut tree, &path);
        let modified = Some(tree.clock);
        tree.nodes.insert(
            path,
            Node {
                kind: EntryKind::Symlink,
                data: Vec::new(),
                modified,
                permissions: 0o777,
                link: Some(target.to_owned()),
            },
        );
        self
    }

    /// Muda a data de um item.
    pub fn touch(&self, path: &str, time: SystemTime) -> &Self {
        if let Some(node) = self.tree().nodes.get_mut(&path::normalize(path)) {
            node.modified = Some(time);
        }
        self
    }

    /// O conteúdo de um arquivo, se existir.
    pub fn contents(&self, path: &str) -> Option<Vec<u8>> {
        self.tree()
            .nodes
            .get(&path::normalize(path))
            .filter(|n| n.kind == EntryKind::File)
            .map(|n| n.data.clone())
    }

    /// `true` se o caminho existe.
    pub fn exists(&self, path: &str) -> bool {
        self.tree().nodes.contains_key(&path::normalize(path))
    }

    /// Os caminhos de tudo o que há, em ordem.
    pub fn paths(&self) -> Vec<String> {
        self.tree().nodes.keys().cloned().collect()
    }

    fn check(&self) -> io::Result<MutexGuard<'_, Tree>> {
        let tree = self.tree();
        if tree.generation != self.generation {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "conexão interrompida pelo outro lado",
            ));
        }
        Ok(tree)
    }
}

fn dir_node() -> Node {
    Node {
        kind: EntryKind::Dir,
        data: Vec::new(),
        modified: Some(mem_time()),
        permissions: 0o755,
        link: None,
    }
}

fn make_parents(tree: &mut Tree, path: &str) {
    let mut missing = Vec::new();
    let mut current = path::parent(path);
    while !tree.nodes.contains_key(&current) {
        missing.push(current.clone());
        let up = path::parent(&current);
        if up == current {
            break;
        }
        current = up;
    }
    for dir in missing {
        let modified = Some(tree.clock);
        tree.nodes.insert(
            dir,
            Node {
                modified,
                ..dir_node()
            },
        );
    }
}

fn entry(name: &str, node: &Node) -> Entry {
    Entry {
        name: name.to_owned(),
        kind: node.kind,
        size: node.data.len() as u64,
        modified: node.modified,
        permissions: Some(node.permissions),
        owner: Some("ana".to_owned()),
        group: Some("ana".to_owned()),
        link_target: node.link.clone(),
    }
}

fn denied(tree: &Tree, path: &str, action: &str) -> io::Result<()> {
    let blocked = tree
        .denied
        .iter()
        .any(|d| path == d || path.starts_with(&format!("{}/", d.trim_end_matches('/'))));
    if blocked {
        Err(describe(
            io::ErrorKind::PermissionDenied.into(),
            action,
            path,
        ))
    } else {
        Ok(())
    }
}

/// O `realpath`: percorre o caminho componente a componente, troca cada link pelo alvo e deixa
/// o `..` subir sobre o que já foi resolvido (como o servidor faz). Um ciclo de links dá
/// `InvalidInput`: os saltos são contados até o número de entradas da árvore, que é finito.
fn real_path(tree: &Tree, path: &str) -> io::Result<String> {
    let original = path::normalize(path);
    let max_hops = tree.nodes.len() + 1;
    let mut hops = 0;
    let mut pending: Vec<String> = path
        .replace('\\', "/")
        .split('/')
        .rev()
        .map(str::to_owned)
        .collect();
    let mut done: Vec<String> = Vec::new();
    while let Some(part) = pending.pop() {
        match part.as_str() {
            "" | "." => {}
            ".." => {
                done.pop();
            }
            name => {
                let here = format!("/{}", done.join("/"));
                let here = path::normalize(&here);
                if tree.nodes.get(&here).map(|n| n.kind) != Some(EntryKind::Dir) {
                    return Err(describe(
                        io::ErrorKind::NotADirectory.into(),
                        "ler",
                        &original,
                    ));
                }
                denied(tree, &path::join(&here, name), "ler")?;
                let candidate = path::join(&here, name);
                let node = tree
                    .nodes
                    .get(&candidate)
                    .ok_or_else(|| describe(io::ErrorKind::NotFound.into(), "ler", &original))?;
                match (&node.kind, &node.link) {
                    (EntryKind::Symlink, Some(target)) => {
                        hops += 1;
                        if hops > max_hops {
                            return Err(describe(
                                io::ErrorKind::InvalidInput.into(),
                                "ler",
                                &format!("links demais em {original}"),
                            ));
                        }
                        if target.starts_with('/') {
                            done.clear();
                        }
                        pending.extend(target.split('/').rev().map(str::to_owned));
                    }
                    _ => done.push(name.to_owned()),
                }
            }
        }
    }
    Ok(path::normalize(&format!("/{}", done.join("/"))))
}

/// Segue links até um item que não é link (no máximo 16 saltos).
fn resolve(tree: &Tree, path: &str) -> Option<String> {
    let mut current = path.to_owned();
    for _ in 0..16 {
        let node = tree.nodes.get(&current)?;
        match (&node.kind, &node.link) {
            (EntryKind::Symlink, Some(target)) => {
                current = path::join(&path::parent(&current), target);
            }
            _ => return Some(current),
        }
    }
    None
}

impl Fs for MemFs {
    fn caps(&self) -> Caps {
        self.tree().caps
    }

    fn home(&mut self) -> io::Result<String> {
        self.check().map(drop)?;
        Ok("/home/ana".to_owned())
    }

    fn list(&mut self, dir: &str) -> io::Result<Vec<Entry>> {
        let mut tree = self.check()?;
        tree.lists += 1;
        let dir = path::normalize(dir);
        denied(&tree, &dir, "ler")?;
        let target = resolve(&tree, &dir)
            .ok_or_else(|| describe(io::ErrorKind::NotFound.into(), "ler", &dir))?;
        if tree.nodes[&target].kind != EntryKind::Dir {
            return Err(describe(io::ErrorKind::NotADirectory.into(), "ler", &dir));
        }
        Ok(tree
            .nodes
            .iter()
            .filter(|(p, _)| p.as_str() != "/" && path::parent(p) == target)
            .map(|(p, n)| entry(path::file_name(p), n))
            .collect())
    }

    fn stat(&mut self, path: &str) -> io::Result<Entry> {
        let tree = self.check()?;
        let path = path::normalize(path);
        denied(&tree, &path, "ler")?;
        let real = resolve(&tree, &path)
            .ok_or_else(|| describe(io::ErrorKind::NotFound.into(), "ler", &path))?;
        Ok(entry(path::file_name(&path), &tree.nodes[&real]))
    }

    fn lstat(&mut self, path: &str) -> io::Result<Entry> {
        let tree = self.check()?;
        let path = path::normalize(path);
        denied(&tree, &path, "ler")?;
        let node = tree
            .nodes
            .get(&path)
            .ok_or_else(|| describe(io::ErrorKind::NotFound.into(), "ler", &path))?;
        Ok(entry(path::file_name(&path), node))
    }

    fn open_read(&mut self, path: &str, offset: u64) -> io::Result<Box<dyn Download + '_>> {
        let mut tree = self.check()?;
        let path = path::normalize(path);
        denied(&tree, &path, "ler")?;
        let real = resolve(&tree, &path)
            .ok_or_else(|| describe(io::ErrorKind::NotFound.into(), "ler", &path))?;
        if tree.nodes[&real].kind != EntryKind::File {
            return Err(describe(io::ErrorKind::IsADirectory.into(), "ler", &path));
        }
        open_file(&mut tree);
        Ok(Box::new(MemFile {
            shared: Arc::clone(&self.shared),
            generation: self.generation,
            path: real,
            pos: offset as usize,
        }))
    }

    fn open_write(&mut self, path: &str, offset: u64) -> io::Result<Box<dyn Upload + '_>> {
        let mut tree = self.check()?;
        let path = path::normalize(path);
        denied(&tree, &path, "gravar em")?;
        if offset > 0 && !tree.caps.resume_upload {
            return Err(unsupported("retomar o envio"));
        }
        let parent = path::parent(&path);
        if tree.nodes.get(&parent).map(|n| n.kind) != Some(EntryKind::Dir) {
            return Err(describe(
                io::ErrorKind::NotFound.into(),
                "gravar em",
                &parent,
            ));
        }
        let modified = Some(tree.clock);
        let node = tree.nodes.entry(path.clone()).or_insert_with(|| Node {
            kind: EntryKind::File,
            data: Vec::new(),
            modified,
            permissions: 0o644,
            link: None,
        });
        if node.kind != EntryKind::File {
            return Err(describe(
                io::ErrorKind::IsADirectory.into(),
                "gravar em",
                &path,
            ));
        }
        node.data.truncate(offset as usize);
        node.data.resize(offset as usize, 0);
        node.modified = modified;
        open_file(&mut tree);
        Ok(Box::new(MemFile {
            shared: Arc::clone(&self.shared),
            generation: self.generation,
            path,
            pos: offset as usize,
        }))
    }

    fn mkdir(&mut self, path: &str) -> io::Result<()> {
        let mut tree = self.check()?;
        let path = path::normalize(path);
        denied(&tree, &path, "criar")?;
        if tree.nodes.contains_key(&path) {
            return Err(describe(
                io::ErrorKind::AlreadyExists.into(),
                "criar",
                &path,
            ));
        }
        if tree.nodes.get(&path::parent(&path)).map(|n| n.kind) != Some(EntryKind::Dir) {
            return Err(describe(io::ErrorKind::NotFound.into(), "criar", &path));
        }
        let modified = Some(tree.clock);
        tree.nodes.insert(
            path,
            Node {
                modified,
                ..dir_node()
            },
        );
        Ok(())
    }

    fn remove_file(&mut self, path: &str) -> io::Result<()> {
        let mut tree = self.check()?;
        let path = path::normalize(path);
        denied(&tree, &path, "apagar")?;
        match tree.nodes.get(&path).map(|n| n.kind) {
            None => Err(describe(io::ErrorKind::NotFound.into(), "apagar", &path)),
            Some(EntryKind::Dir) => Err(describe(
                io::ErrorKind::IsADirectory.into(),
                "apagar",
                &path,
            )),
            Some(_) => {
                tree.nodes.remove(&path);
                Ok(())
            }
        }
    }

    fn remove_dir(&mut self, path: &str) -> io::Result<()> {
        let mut tree = self.check()?;
        let path = path::normalize(path);
        denied(&tree, &path, "apagar")?;
        match tree.nodes.get(&path).map(|n| n.kind) {
            None => return Err(describe(io::ErrorKind::NotFound.into(), "apagar", &path)),
            Some(EntryKind::Dir) => {}
            Some(_) => {
                return Err(describe(
                    io::ErrorKind::NotADirectory.into(),
                    "apagar",
                    &path,
                ));
            }
        }
        if tree
            .nodes
            .keys()
            .any(|p| p != &path && path::parent(p) == path)
        {
            return Err(describe(
                io::ErrorKind::DirectoryNotEmpty.into(),
                "apagar",
                &path,
            ));
        }
        tree.nodes.remove(&path);
        Ok(())
    }

    fn rename(&mut self, from: &str, to: &str, overwrite: bool) -> io::Result<()> {
        let mut tree = self.check()?;
        let (from, to) = (path::normalize(from), path::normalize(to));
        denied(&tree, &from, "renomear")?;
        denied(&tree, &to, "renomear")?;
        if !tree.nodes.contains_key(&from) {
            return Err(describe(io::ErrorKind::NotFound.into(), "renomear", &from));
        }
        if tree.nodes.contains_key(&to) && !overwrite {
            return Err(describe(
                io::ErrorKind::AlreadyExists.into(),
                "renomear",
                &to,
            ));
        }
        if tree.nodes.get(&path::parent(&to)).map(|n| n.kind) != Some(EntryKind::Dir) {
            return Err(describe(io::ErrorKind::NotFound.into(), "renomear", &to));
        }
        let prefix = format!("{}/", from.trim_end_matches('/'));
        let moved: Vec<String> = tree
            .nodes
            .keys()
            .filter(|p| **p == from || p.starts_with(&prefix))
            .cloned()
            .collect();
        for old in moved {
            let node = tree
                .nodes
                .remove(&old)
                .expect("o caminho acabou de ser listado");
            let new = format!("{to}{}", &old[from.len()..]);
            tree.nodes.insert(new, node);
        }
        Ok(())
    }

    fn set_permissions(&mut self, path: &str, mode: u32) -> io::Result<()> {
        let mut tree = self.check()?;
        if !tree.caps.permissions {
            return Err(unsupported("mudar as permissões"));
        }
        let path = path::normalize(path);
        denied(&tree, &path, "mudar as permissões de")?;
        match tree.nodes.get_mut(&path) {
            Some(node) => {
                node.permissions = mode & 0o7777;
                Ok(())
            }
            None => Err(describe(
                io::ErrorKind::NotFound.into(),
                "mudar as permissões de",
                &path,
            )),
        }
    }

    fn set_modified(&mut self, path: &str, time: SystemTime) -> io::Result<()> {
        let mut tree = self.check()?;
        if !tree.caps.set_modified {
            return Err(unsupported("mudar a data"));
        }
        let path = path::normalize(path);
        match tree.nodes.get_mut(&path) {
            Some(node) => {
                node.modified = Some(time);
                Ok(())
            }
            None => Err(describe(
                io::ErrorKind::NotFound.into(),
                "mudar a data de",
                &path,
            )),
        }
    }

    fn symlink(&mut self, target: &str, link: &str) -> io::Result<()> {
        let mut tree = self.check()?;
        if !tree.caps.symlinks {
            return Err(unsupported("criar links simbólicos"));
        }
        let link = path::normalize(link);
        if tree.nodes.contains_key(&link) {
            return Err(describe(
                io::ErrorKind::AlreadyExists.into(),
                "criar",
                &link,
            ));
        }
        let modified = Some(tree.clock);
        tree.nodes.insert(
            link,
            Node {
                kind: EntryKind::Symlink,
                data: Vec::new(),
                modified,
                permissions: 0o777,
                link: Some(target.to_owned()),
            },
        );
        Ok(())
    }

    fn read_link(&mut self, path: &str) -> io::Result<String> {
        let tree = self.check()?;
        let path = path::normalize(path);
        tree.nodes
            .get(&path)
            .and_then(|n| n.link.clone())
            .ok_or_else(|| describe(io::ErrorKind::NotFound.into(), "ler", &path))
    }

    fn real_path(&mut self, path: &str) -> io::Result<String> {
        let tree = self.check()?;
        real_path(&tree, path)
    }
}

fn open_file(tree: &mut Tree) {
    tree.open_files += 1;
    tree.max_open_files = tree.max_open_files.max(tree.open_files);
}

/// Um arquivo aberto (leitura ou gravação) do `MemFs`.
struct MemFile {
    shared: Arc<Shared>,
    generation: u64,
    path: String,
    pos: usize,
}

impl MemFile {
    fn tree(&self) -> io::Result<MutexGuard<'_, Tree>> {
        let tree = self
            .shared
            .tree
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if tree.generation != self.generation {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "conexão interrompida pelo outro lado",
            ));
        }
        Ok(tree)
    }
}

impl Drop for MemFile {
    fn drop(&mut self) {
        let mut tree = self
            .shared
            .tree
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        tree.open_files = tree.open_files.saturating_sub(1);
    }
}

impl Read for MemFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let (n, delay) = {
            let tree = self.tree()?;
            let node = tree
                .nodes
                .get(&self.path)
                .ok_or_else(|| describe(io::ErrorKind::NotFound.into(), "ler", &self.path))?;
            let data = node.data.get(self.pos..).unwrap_or_default();
            let n = data.len().min(buf.len()).min(tree.chunk);
            buf[..n].copy_from_slice(&data[..n]);
            (n, tree.delay)
        };
        self.pos += n;
        if !delay.is_zero() && n > 0 {
            std::thread::sleep(delay);
        }
        Ok(n)
    }
}

impl Write for MemFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let (n, delay) = {
            let mut tree = self.tree()?;
            let n = buf.len().min(tree.chunk);
            let delay = tree.delay;
            let path = self.path.clone();
            let node = tree
                .nodes
                .get_mut(&path)
                .ok_or_else(|| describe(io::ErrorKind::NotFound.into(), "gravar em", &path))?;
            if node.data.len() < self.pos + n {
                node.data.resize(self.pos + n, 0);
            }
            node.data[self.pos..self.pos + n].copy_from_slice(&buf[..n]);
            (n, delay)
        };
        self.pos += n;
        if !delay.is_zero() {
            std::thread::sleep(delay);
        }
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Download for MemFile {
    fn finish(self: Box<Self>) -> io::Result<()> {
        self.tree().map(drop)
    }
}

impl Upload for MemFile {
    fn finish(self: Box<Self>) -> io::Result<()> {
        self.tree().map(drop)
    }
}
