//! O rclone como um [`Fs`] (M33).
//!
//! O [`Rcd`] sobe o `rclone rcd` em `127.0.0.1` e conversa com ele pela API de controle remoto;
//! o [`Rclone`] é um [`Fs`] sobre um remoto do rclone (`gdrive:`, `gdrive:Backup`,
//! `:memory:`…), então os mais de 70 serviços do rclone (Google Drive, OneDrive, Dropbox, Azure
//! Blob, WebDAV, SMB…) abrem na mesma tela dos outros. As cópias entre o disco e a nuvem podem
//! ser feitas pelo próprio rclone ([`Fs::delegated_copy`]), sem passar os bytes pelo app. A
//! configuração dos remotos (provedores, criar, perguntas do OAuth) está em [`config`].
//!
//! ```no_run
//! use tomni_ui_files::Fs;
//! use tomni_ui_files::rclone::{Rcd, RcdConfig, Rclone, find_rclone};
//!
//! let binario = find_rclone(None).expect("rclone instalado");
//! let rcd = Rcd::spawn(RcdConfig::new(binario))?;
//! let mut drive = Rclone::new(rcd, "gdrive:");
//! for entrada in drive.list("/")? {
//!     println!("{}", entrada.name);
//! }
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod config;
mod rcd;

use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Deserialize;
use serde_json::{Value, json};
use ureq::SendBody;
use ureq::http::StatusCode;

use crate::entry::{Caps, Entry, EntryKind};
use crate::fs::{Delegation, Download, Fs, Location, Upload, describe, unsupported};
use crate::path;

pub use rcd::{
    ENDERECO_DE_INSTALACAO, LIMITE_DO_LOGIN, Rcd, RcdConfig, RcloneError, VERSAO_MINIMA,
    find_rclone,
};

/// Os bytes que o rclone espera escapados na URL (tudo menos o que não precisa).
const URL: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'/')
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');
/// Os bytes escapados num valor de consulta (a barra também).
const CONSULTA: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');
/// O tamanho de cada pedaço do envio.
const PEDACO: usize = 128 * 1024;
/// De quanto em quanto tempo o progresso de uma cópia delegada é lido.
const INTERVALO_DO_PROGRESSO: Duration = Duration::from_millis(250);

/// O que um remoto do rclone sabe fazer (`operations/fsinfo`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Features {
    /// Copia no servidor.
    pub copy: bool,
    /// Move no servidor.
    pub move_: bool,
    /// Move pastas inteiras no servidor.
    pub dir_move: bool,
    /// Faz links públicos.
    pub public_link: bool,
    /// Diz o espaço usado.
    pub about: bool,
    /// Aceita pastas vazias.
    pub can_have_empty_directories: bool,
}

/// O espaço de um remoto (`operations/about`); o que o serviço não diz fica `None`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct About {
    /// O total em bytes.
    pub total: Option<u64>,
    /// O usado em bytes.
    pub used: Option<u64>,
    /// O livre em bytes.
    pub free: Option<u64>,
}

/// Um remoto do rclone como [`Fs`].
pub struct Rclone {
    rcd: Arc<Rcd>,
    spec: String,
    features: OnceLock<Features>,
}

impl std::fmt::Debug for Rclone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Rclone")
            .field("remote", &self.spec)
            .finish()
    }
}

impl Rclone {
    /// O remoto `remote` (`gdrive:` ou `gdrive:Backup`) do `rcd`. Os caminhos do [`Fs`] são
    /// relativos à raiz dele (`/` é a raiz).
    pub fn new(rcd: Arc<Rcd>, remote: &str) -> Rclone {
        Rclone {
            rcd,
            spec: remote.to_owned(),
            features: OnceLock::new(),
        }
    }

    /// O `rcd` que atende este remoto.
    pub fn rcd(&self) -> &Arc<Rcd> {
        &self.rcd
    }

    /// O nome do remoto, como foi dado a [`Rclone::new`].
    pub fn remote(&self) -> &str {
        &self.spec
    }

    /// O que o remoto sabe fazer.
    pub fn features(&self) -> io::Result<Features> {
        if let Some(features) = self.features.get() {
            return Ok(*features);
        }
        let resposta = self.call("operations/fsinfo", json!({ "fs": self.spec }), "")?;
        let marca = |nome: &str| {
            resposta
                .get("Features")
                .and_then(|f| f.get(nome))
                .and_then(Value::as_bool)
                .unwrap_or(false)
        };
        let features = Features {
            copy: marca("Copy"),
            move_: marca("Move"),
            dir_move: marca("DirMove"),
            public_link: marca("PublicLink"),
            about: marca("About"),
            can_have_empty_directories: marca("CanHaveEmptyDirectories"),
        };
        Ok(*self.features.get_or_init(|| features))
    }

    /// O espaço do remoto, se o serviço diz.
    pub fn about(&self) -> io::Result<Option<About>> {
        if !self.features()?.about {
            return Ok(None);
        }
        let resposta = self.call("operations/about", json!({ "fs": self.spec }), "")?;
        let numero = |nome: &str| resposta.get(nome).and_then(Value::as_u64);
        Ok(Some(About {
            total: numero("total"),
            used: numero("used"),
            free: numero("free"),
        }))
    }

    /// O link público de `target`, válido por `expire` (`1h`, `7d`…) se o serviço aceita.
    pub fn public_link(&self, target: &str, expire: Option<&str>) -> io::Result<String> {
        let mut pedido = json!({ "fs": self.spec, "remote": relativo(target), "unlink": false });
        if let Some(expire) = expire {
            pedido["expire"] = json!(expire);
        }
        let resposta = self.call("operations/publiclink", pedido, target)?;
        resposta
            .get("url")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "o rclone não devolveu o link público",
                )
            })
    }

    /// Chama o `rcd` e traduz o erro para `io::Error` (com `target` na mensagem, se houver).
    fn call(&self, method: &str, input: Value, target: &str) -> io::Result<Value> {
        self.rcd
            .call(method, input)
            .map_err(|erro| traduzir(erro, target))
    }

    /// O remoto com uma subpasta (`gdrive:` + `a/b` → `gdrive:a/b`).
    fn sub(&self, rel: &str) -> String {
        juntar_remoto(&self.spec, rel)
    }

    /// A URL do `--rc-serve` para ler `rel`.
    fn url_de_leitura(&self, rel: &str) -> String {
        let colchetes = format!("[{}]", self.spec);
        let remoto = utf8_percent_encode(&colchetes, URL);
        format!("/{remoto}/{}", utf8_percent_encode(rel, URL))
    }
}

/// O caminho do [`Fs`] sem a barra do começo.
fn relativo(target: &str) -> String {
    target.trim_matches('/').to_owned()
}

fn juntar_remoto(spec: &str, rel: &str) -> String {
    if rel.is_empty() {
        spec.to_owned()
    } else if spec.ends_with(':') || spec.ends_with('/') {
        format!("{spec}{rel}")
    } else {
        format!("{spec}/{rel}")
    }
}

/// O erro do rclone como `io::Error` em português; `target` entra nas mensagens de
/// "não encontrado", "sem permissão" e "já existe".
fn traduzir(erro: RcloneError, target: &str) -> io::Error {
    let tipo = erro.kind();
    let conhecido = matches!(
        tipo,
        io::ErrorKind::NotFound
            | io::ErrorKind::PermissionDenied
            | io::ErrorKind::AlreadyExists
            | io::ErrorKind::DirectoryNotEmpty
            | io::ErrorKind::NotADirectory
    );
    let api = matches!(erro, RcloneError::Api { .. });
    let base: io::Error = erro.into();
    if conhecido && api && !target.is_empty() {
        // Mantém o texto do rclone entre aspas ao lado da mensagem curta.
        let curta = describe(io::Error::from(tipo), "acessar", target);
        return io::Error::new(tipo, format!("{curta} ({})", base));
    }
    base
}

/// Um item de `operations/list` e `operations/stat`.
#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
struct Item {
    name: String,
    size: i64,
    mod_time: Option<String>,
    is_dir: bool,
}

impl Item {
    fn entry(self) -> Entry {
        let tipo = if self.is_dir {
            EntryKind::Dir
        } else {
            EntryKind::File
        };
        let mut entry = Entry::new(self.name, tipo);
        entry.size = if self.is_dir {
            0
        } else {
            self.size.max(0) as u64
        };
        entry.modified = self.mod_time.as_deref().and_then(data);
        entry
    }
}

/// Lê uma data RFC 3339.
fn data(texto: &str) -> Option<SystemTime> {
    let momento: jiff::Timestamp = texto.parse().ok()?;
    let nanos = momento.as_nanosecond();
    if nanos >= 0 {
        UNIX_EPOCH.checked_add(Duration::from_nanos(u64::try_from(nanos).ok()?))
    } else {
        UNIX_EPOCH.checked_sub(Duration::from_nanos(u64::try_from(-nanos).ok()?))
    }
}

impl Fs for Rclone {
    fn caps(&self) -> Caps {
        Caps {
            permissions: false,
            set_modified: false,
            symlinks: false,
            resume_upload: false,
            owner: false,
        }
    }

    fn home(&mut self) -> io::Result<String> {
        Ok("/".to_owned())
    }

    fn list(&mut self, dir: &str) -> io::Result<Vec<Entry>> {
        let resposta = self.call(
            "operations/list",
            json!({ "fs": self.spec, "remote": relativo(dir), "opt": {} }),
            dir,
        )?;
        let itens: Vec<Item> = resposta
            .get("list")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|erro| io::Error::new(io::ErrorKind::InvalidData, erro.to_string()))?
            .unwrap_or_default();
        Ok(itens.into_iter().map(Item::entry).collect())
    }

    fn stat(&mut self, target: &str) -> io::Result<Entry> {
        let rel = relativo(target);
        if rel.is_empty() {
            return Ok(Entry::new("", EntryKind::Dir));
        }
        let resposta = self.call(
            "operations/stat",
            json!({ "fs": self.spec, "remote": rel }),
            target,
        )?;
        match resposta.get("item") {
            Some(item) if !item.is_null() => {
                let item: Item = serde_json::from_value(item.clone())
                    .map_err(|erro| io::Error::new(io::ErrorKind::InvalidData, erro.to_string()))?;
                Ok(item.entry())
            }
            _ => Err(describe(io::ErrorKind::NotFound.into(), "ler", target)),
        }
    }

    fn lstat(&mut self, target: &str) -> io::Result<Entry> {
        self.stat(target)
    }

    fn open_read(&mut self, target: &str, offset: u64) -> io::Result<Box<dyn Download + '_>> {
        let rel = relativo(target);
        let faixa = format!("bytes={offset}-");
        let cabecalhos: Vec<(&str, &str)> = if offset > 0 {
            vec![("Range", faixa.as_str())]
        } else {
            Vec::new()
        };
        let mut resposta = self
            .rcd
            .send(
                self.rcd.agent(),
                "GET",
                &self.url_de_leitura(&rel),
                &cabecalhos,
                SendBody::none(),
            )
            .map_err(|erro| traduzir(erro, target))?;
        let status = resposta.status();
        if status == StatusCode::NOT_FOUND {
            return Err(describe(io::ErrorKind::NotFound.into(), "ler", target));
        }
        if status == StatusCode::RANGE_NOT_SATISFIABLE {
            // O arquivo já está inteiro no destino: não há mais nada a ler.
            return Ok(Box::new(RcloneDownload {
                leitor: Box::new(io::empty()),
            }));
        }
        if !status.is_success() {
            let erro = self.rcd.erro_da_resposta(&mut resposta);
            return Err(traduzir(erro, target));
        }
        let mut leitor = resposta.into_body().into_reader();
        if offset > 0 && status == StatusCode::OK {
            // O servidor ignorou a faixa: pula o que já foi lido.
            io::copy(&mut (&mut leitor).take(offset), &mut io::sink())?;
        }
        Ok(Box::new(RcloneDownload {
            leitor: Box::new(leitor),
        }))
    }

    fn open_write(&mut self, target: &str, offset: u64) -> io::Result<Box<dyn Upload + '_>> {
        if offset > 0 {
            return Err(unsupported("retomar um envio"));
        }
        let rel = relativo(target);
        let pasta = path::parent(&format!("/{rel}"));
        let nome = path::file_name(&rel).to_owned();
        if nome.is_empty() {
            return Err(describe(
                io::ErrorKind::InvalidInput.into(),
                "gravar em",
                target,
            ));
        }
        Ok(Box::new(RcloneUpload::iniciar(
            Arc::clone(&self.rcd),
            &self.spec,
            &relativo(&pasta),
            &nome,
            target,
        )))
    }

    fn mkdir(&mut self, target: &str) -> io::Result<()> {
        self.call(
            "operations/mkdir",
            json!({ "fs": self.spec, "remote": relativo(target) }),
            target,
        )
        .map(drop)
    }

    fn remove_file(&mut self, target: &str) -> io::Result<()> {
        self.call(
            "operations/deletefile",
            json!({ "fs": self.spec, "remote": relativo(target) }),
            target,
        )
        .map(drop)
    }

    fn remove_dir(&mut self, target: &str) -> io::Result<()> {
        self.call(
            "operations/rmdir",
            json!({ "fs": self.spec, "remote": relativo(target) }),
            target,
        )
        .map(drop)
    }

    fn rename(&mut self, from: &str, to: &str, overwrite: bool) -> io::Result<()> {
        let origem = self.stat(from)?;
        if !overwrite && self.stat(to).is_ok() {
            return Err(describe(io::ErrorKind::AlreadyExists.into(), "mover", to));
        }
        if origem.is_dir() {
            self.call(
                "sync/move",
                json!({
                    "srcFs": self.sub(&relativo(from)),
                    "dstFs": self.sub(&relativo(to)),
                    "deleteEmptySrcDirs": true,
                }),
                from,
            )?;
            // O `deleteEmptySrcDirs` tira as subpastas vazias, mas deixa a própria pasta.
            let _ = self.call(
                "operations/rmdir",
                json!({ "fs": self.spec, "remote": relativo(from) }),
                from,
            );
            Ok(())
        } else {
            self.call(
                "operations/movefile",
                json!({
                    "srcFs": self.spec,
                    "srcRemote": relativo(from),
                    "dstFs": self.spec,
                    "dstRemote": relativo(to),
                }),
                from,
            )
            .map(drop)
        }
    }

    fn set_permissions(&mut self, _target: &str, _mode: u32) -> io::Result<()> {
        Err(unsupported("mudar as permissões"))
    }

    fn set_modified(&mut self, _target: &str, _time: SystemTime) -> io::Result<()> {
        Err(unsupported("mudar a data"))
    }

    fn symlink(&mut self, _target: &str, _link: &str) -> io::Result<()> {
        Err(unsupported("criar links simbólicos"))
    }

    fn read_link(&mut self, _target: &str) -> io::Result<String> {
        Err(unsupported("ler links simbólicos"))
    }

    fn delegation(&self) -> Option<Delegation> {
        Some(Delegation {
            group: Arc::as_ptr(&self.rcd) as usize,
            remote: self.spec.clone(),
        })
    }

    fn delegated_copy(
        &mut self,
        from: &Location,
        to: &Location,
        progress: &dyn Fn(u64, u64),
        cancel: &AtomicBool,
    ) -> Option<io::Result<()>> {
        let (origem, destino) = (self.ponta(from)?, self.ponta(to)?);
        Some(self.copiar(origem, destino, progress, cancel))
    }
}

/// Uma ponta de `operations/copyfile`: o `Fs` do rclone e o caminho dentro dele.
struct Ponta {
    fs: String,
    remote: String,
}

impl Rclone {
    fn ponta(&self, local: &Location) -> Option<Ponta> {
        match local {
            Location::Here(caminho) => Some(Ponta {
                fs: self.spec.clone(),
                remote: relativo(caminho),
            }),
            Location::Peer(grupo, caminho) => (grupo.group == Arc::as_ptr(&self.rcd) as usize)
                .then(|| Ponta {
                    fs: grupo.remote.clone(),
                    remote: relativo(caminho),
                }),
            Location::Local(caminho) => {
                let texto = caminho.to_string_lossy().replace('\\', "/");
                let pasta = path::parent(&texto);
                Some(Ponta {
                    fs: pasta,
                    remote: path::file_name(&texto).to_owned(),
                })
            }
        }
    }

    fn copiar(
        &self,
        origem: Ponta,
        destino: Ponta,
        progress: &dyn Fn(u64, u64),
        cancel: &AtomicBool,
    ) -> io::Result<()> {
        let destino_texto = format!("{}{}", destino.fs, destino.remote);
        let resposta = self.call(
            "operations/copyfile",
            json!({
                "srcFs": origem.fs,
                "srcRemote": origem.remote,
                "dstFs": destino.fs,
                "dstRemote": destino.remote,
                "_async": true,
            }),
            &destino_texto,
        )?;
        let job = resposta
            .get("jobid")
            .and_then(Value::as_i64)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "o rclone não devolveu o número da tarefa",
                )
            })?;
        let mut parando = false;
        loop {
            if cancel.load(Ordering::SeqCst) && !parando {
                parando = true;
                let _ = self.rcd.call("job/stop", json!({ "jobid": job }));
            }
            let estado = self.call("job/status", json!({ "jobid": job }), "")?;
            if estado.get("finished").and_then(Value::as_bool) == Some(true) {
                if parando {
                    return Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "cópia cancelada",
                    ));
                }
                let erro = estado.get("error").and_then(Value::as_str).unwrap_or("");
                if erro.is_empty() {
                    if let Some((feito, total)) = self.progresso(job) {
                        progress(total.max(feito), total.max(feito));
                    }
                    return Ok(());
                }
                return Err(traduzir(
                    RcloneError::Api {
                        status: 500,
                        message: erro.to_owned(),
                    },
                    &destino_texto,
                ));
            }
            if let Some((feito, total)) = self.progresso(job) {
                progress(feito, total);
            }
            std::thread::sleep(INTERVALO_DO_PROGRESSO);
        }
    }

    /// `(bytes feitos, bytes no total)` da tarefa, se o rclone já sabe.
    fn progresso(&self, job: i64) -> Option<(u64, u64)> {
        let estatisticas = self
            .rcd
            .call("core/stats", json!({ "group": format!("job/{job}") }))
            .ok()?;
        let feito = estatisticas.get("bytes").and_then(Value::as_u64)?;
        let total = estatisticas
            .get("totalBytes")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        Some((feito, total))
    }
}

/// O arquivo lido pelo `--rc-serve`.
struct RcloneDownload {
    leitor: Box<dyn Read + Send>,
}

impl Read for RcloneDownload {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.leitor.read(buffer)
    }
}

impl Download for RcloneDownload {
    fn finish(self: Box<Self>) -> io::Result<()> {
        Ok(())
    }
}

/// O corpo `multipart/form-data` de um envio: o começo, os pedaços que chegam pelo canal e o
/// fecho. Se o envio é abandonado, a leitura falha e o pedido aborta (sem fechar o arquivo
/// pela metade como se estivesse completo).
struct CorpoDoEnvio {
    inicio: io::Cursor<Vec<u8>>,
    pedacos: Receiver<Vec<u8>>,
    atual: io::Cursor<Vec<u8>>,
    fim: io::Cursor<Vec<u8>>,
    fechado: bool,
    abortado: Arc<AtomicBool>,
}

impl Read for CorpoDoEnvio {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let lidos = self.inicio.read(buffer)?;
        if lidos > 0 {
            return Ok(lidos);
        }
        while !self.fechado {
            let lidos = self.atual.read(buffer)?;
            if lidos > 0 {
                return Ok(lidos);
            }
            match self.pedacos.recv() {
                Ok(pedaco) => self.atual = io::Cursor::new(pedaco),
                Err(_) => self.fechado = true,
            }
        }
        if self.abortado.load(Ordering::SeqCst) {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "envio cancelado",
            ));
        }
        self.fim.read(buffer)
    }
}

/// O arquivo enviado pelo `operations/uploadfile`, num pedido que corre em outra thread.
struct RcloneUpload {
    envio: Option<SyncSender<Vec<u8>>>,
    pedido: Option<JoinHandle<io::Result<()>>>,
    abortado: Arc<AtomicBool>,
    pendente: Vec<u8>,
}

impl RcloneUpload {
    fn iniciar(rcd: Arc<Rcd>, spec: &str, pasta: &str, nome: &str, target: &str) -> RcloneUpload {
        let (envio, pedacos) = mpsc::sync_channel::<Vec<u8>>(4);
        let abortado = Arc::new(AtomicBool::new(false));
        let fronteira = format!("tomni-ui-{:016x}", fronteira_unica());
        let nome_seguro = nome.replace('\\', "\\\\").replace('"', "\\\"");
        let inicio = format!(
            "--{fronteira}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{nome_seguro}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
        );
        let corpo = CorpoDoEnvio {
            inicio: io::Cursor::new(inicio.into_bytes()),
            pedacos,
            atual: io::Cursor::new(Vec::new()),
            fim: io::Cursor::new(format!("\r\n--{fronteira}--\r\n").into_bytes()),
            fechado: false,
            abortado: Arc::clone(&abortado),
        };
        let caminho = format!(
            "/operations/uploadfile?fs={}&remote={}",
            utf8_percent_encode(spec, CONSULTA),
            utf8_percent_encode(pasta, CONSULTA)
        );
        let tipo = format!("multipart/form-data; boundary={fronteira}");
        let target = target.to_owned();
        let pedido = std::thread::spawn(move || {
            let mut resposta = rcd
                .send(
                    rcd.agent(),
                    "POST",
                    &caminho,
                    &[("Content-Type", tipo.as_str())],
                    SendBody::from_owned_reader(corpo),
                )
                .map_err(|erro| traduzir(erro, &target))?;
            if resposta.status().is_success() {
                Ok(())
            } else {
                let erro = rcd.erro_da_resposta(&mut resposta);
                Err(traduzir(erro, &target))
            }
        });
        RcloneUpload {
            envio: Some(envio),
            pedido: Some(pedido),
            abortado,
            pendente: Vec::new(),
        }
    }

    fn enviar_pendente(&mut self) -> io::Result<()> {
        if self.pendente.is_empty() {
            return Ok(());
        }
        let pedaco = std::mem::take(&mut self.pendente);
        let enviado = match &self.envio {
            Some(envio) => envio.send(pedaco).is_ok(),
            None => false,
        };
        if enviado {
            return Ok(());
        }
        // O canal fecha quando o pedido terminou antes da hora: o erro de verdade é dele.
        self.envio = None;
        let erro = match self.pedido.take() {
            Some(pedido) => match pedido.join() {
                Ok(Err(erro)) => erro,
                _ => io::Error::new(io::ErrorKind::BrokenPipe, "o rclone interrompeu o envio"),
            },
            None => io::Error::new(io::ErrorKind::BrokenPipe, "o envio terminou"),
        };
        Err(erro)
    }
}

impl Write for RcloneUpload {
    fn write(&mut self, dados: &[u8]) -> io::Result<usize> {
        self.pendente.extend_from_slice(dados);
        if self.pendente.len() >= PEDACO {
            self.enviar_pendente()?;
        }
        Ok(dados.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Upload for RcloneUpload {
    fn finish(mut self: Box<Self>) -> io::Result<()> {
        let enviado = self.enviar_pendente();
        // Fechar o canal termina o corpo; o pedido responde logo depois.
        self.envio = None;
        let resultado = match self.pedido.take() {
            Some(pedido) => pedido.join().unwrap_or_else(|_| {
                Err(io::Error::other("o envio para o rclone parou de repente"))
            }),
            None => Ok(()),
        };
        resultado.and(enviado)
    }
}

impl Drop for RcloneUpload {
    fn drop(&mut self) {
        // Soltar sem `finish` cancela: o corpo falha e o rclone descarta o que recebeu.
        if self.pedido.is_some() {
            self.abortado.store(true, Ordering::SeqCst);
            self.envio = None;
            if let Some(pedido) = self.pedido.take() {
                let _ = pedido.join();
            }
        }
    }
}

fn fronteira_unica() -> u64 {
    let mut bytes = [0u8; 8];
    let _ = getrandom::fill(&mut bytes);
    u64::from_le_bytes(bytes)
}
