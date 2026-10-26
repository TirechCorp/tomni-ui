//! O S3 (AWS e os compatíveis) como um [`Fs`] (M32).
//!
//! [`S3::connect`] fala com um servidor S3 (Amazon, MinIO, Cloudflare R2, Wasabi, Backblaze B2,
//! DigitalOcean Spaces, Ceph RGW…) por HTTP: o `rusty-s3` assina cada pedido (SigV4) e o
//! `ureq` os envia, com o `rustls` e as raízes do sistema. Com `bucket`, a raiz `/` é a raiz do
//! bucket; sem ele, a raiz lista os buckets como pastas e `/bucket/chave` é um objeto. Uma
//! pasta é um prefixo terminado em `/`; os envios grandes vão em partes e **retomam** pelo
//! próprio servidor ([`Fs::resume_offset`]).
//!
//! ```no_run
//! use tomni_ui_files::{Fs, s3::{S3, S3Config, S3UrlStyle}};
//!
//! let mut s3 = S3::connect(S3Config {
//!     bucket: Some("meu-bucket".into()),
//!     url_style: S3UrlStyle::Path,
//!     ..S3Config::new("http://127.0.0.1:9000", "minioadmin", "minioadmin")
//! })?;
//! for entrada in s3.list("/")? {
//!     println!("{}", entrada.name);
//! }
//! # Ok::<(), std::io::Error>(())
//! ```

mod client;
mod upload;
mod xml;

use std::io::{self, Read};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use rusty_s3::actions::{
    DeleteObject, DeleteObjects, GetObject, HeadObject, ListObjectsV2, ObjectIdentifier, PutObject,
    S3Action, UploadPart,
};
use rusty_s3::signing::sign;
use rusty_s3::{Credentials, Method, UrlStyle};
use tomni_ui_net::Proxy;
use url::Url;

use crate::entry::{Caps, Entry, EntryKind};
use crate::fs::{Download, Fs, Upload, describe, unsupported};
use crate::path;
use crate::remote::Opener;

use client::{Client, Prepared, VALIDADE, header, text};
use upload::{S3Upload, check_ok_body, contiguous_bytes, resposta_invalida};

/// O tamanho padrão de cada parte de um envio grande.
pub const PART_SIZE_PADRAO: u64 = 8 << 20;
/// O menor tamanho de parte que o S3 aceita (menos a última).
pub const PART_SIZE_MINIMO: u64 = 5 << 20;
/// A partir de quantos bytes copiar (renomear) é feito em partes: o limite da AWS.
pub const COPY_LIMIT_PADRAO: u64 = 5 << 30;
/// O máximo de partes de um envio.
pub(crate) const PARTES_MAXIMAS: usize = 10_000;
/// O máximo de validade de um link pré-assinado (o limite do SigV4).
pub const VALIDADE_MAXIMA_LINK: Duration = Duration::from_secs(7 * 24 * 3600);
/// O cabeçalho em que a data de modificação vai (o mesmo que o rclone usa).
pub const CABECALHO_MTIME: &str = "x-amz-meta-mtime";
/// As classes de armazenamento da AWS.
pub const CLASSES_DE_ARMAZENAMENTO: [&str; 8] = [
    "STANDARD",
    "REDUCED_REDUNDANCY",
    "STANDARD_IA",
    "ONEZONE_IA",
    "INTELLIGENT_TIERING",
    "GLACIER_IR",
    "GLACIER",
    "DEEP_ARCHIVE",
];

/// Os bytes que o `x-amz-copy-source` escapa (tudo menos o que a AWS deixa passar).
const COPY_SOURCE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'/')
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

/// Como o certificado do servidor é conferido.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum TlsMode {
    /// Pelas raízes do sistema.
    #[default]
    Verify,
    /// Pelas raízes do sistema e mais as autoridades deste arquivo PEM (um MinIO com CA própria).
    CustomCa(PathBuf),
    /// Aceita qualquer certificado (um MinIO de laboratório).
    NoVerify,
}

/// Como o bucket entra na URL.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum S3UrlStyle {
    /// `https://endpoint/bucket/chave`: o dos servidores próprios (MinIO, Ceph).
    #[default]
    Path,
    /// `https://bucket.endpoint/chave`: o da AWS e de quase todos os provedores.
    VirtualHost,
}

/// As opções de uma conexão S3.
#[derive(Clone)]
pub struct S3Config {
    /// O endereço do serviço (`https://s3.amazonaws.com`, `http://127.0.0.1:9000`…).
    pub endpoint: String,
    /// A região (`us-east-1`; o R2 usa `auto`).
    pub region: String,
    /// O bucket. Sem ele, a raiz lista os buckets como pastas.
    pub bucket: Option<String>,
    /// Como o bucket entra na URL.
    pub url_style: S3UrlStyle,
    /// O Access Key ID.
    pub access_key: String,
    /// O Secret Access Key.
    pub secret_key: String,
    /// O token de sessão, se as chaves são temporárias.
    pub session_token: Option<String>,
    /// A classe de armazenamento dos envios (`STANDARD_IA`…); `None`: a do bucket.
    pub storage_class: Option<String>,
    /// Como o certificado é conferido.
    pub tls: TlsMode,
    /// O proxy por onde a conexão passa.
    pub proxy: Option<Proxy>,
    /// O tamanho de cada parte de um envio grande (no mínimo 5 MiB).
    pub part_size: u64,
    /// Acima de quantos bytes uma cópia (renomear) vai em partes.
    pub copy_limit: u64,
    /// A espera antes da primeira repetição de um pedido que falhou (dobra a cada tentativa).
    pub retry_base: Duration,
}

impl std::fmt::Debug for S3Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Sem as chaves.
        f.debug_struct("S3Config")
            .field("endpoint", &self.endpoint)
            .field("region", &self.region)
            .field("bucket", &self.bucket)
            .field("url_style", &self.url_style)
            .finish_non_exhaustive()
    }
}

impl S3Config {
    /// Uma conexão a `endpoint` com estas chaves, na região `us-east-1` e com os padrões.
    pub fn new(
        endpoint: impl Into<String>,
        access_key: impl Into<String>,
        secret_key: impl Into<String>,
    ) -> Self {
        Self {
            endpoint: endpoint.into(),
            region: "us-east-1".to_owned(),
            bucket: None,
            url_style: S3UrlStyle::Path,
            access_key: access_key.into(),
            secret_key: secret_key.into(),
            session_token: None,
            storage_class: None,
            tls: TlsMode::Verify,
            proxy: None,
            part_size: PART_SIZE_PADRAO,
            copy_limit: COPY_LIMIT_PADRAO,
            retry_base: Duration::from_millis(500),
        }
    }

    /// Um [`Opener`] que abre uma conexão nova a cada chamada (as
    /// [`Transfers`](crate::Transfers) abrem uma por trabalhador).
    pub fn opener(self) -> Opener {
        Arc::new(move || {
            let s3 = S3::connect(self.clone())?;
            Ok(Box::new(s3) as Box<dyn Fs>)
        })
    }
}

/// O que o servidor conta sobre um objeto (o `HeadObject`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectInfo {
    /// O tamanho em bytes.
    pub size: u64,
    /// O `ETag`, sem as aspas.
    pub etag: Option<String>,
    /// O tipo do conteúdo.
    pub content_type: Option<String>,
    /// A classe de armazenamento (`STANDARD` quando o servidor não diz).
    pub storage_class: String,
    /// A data que o servidor registrou (`Last-Modified`).
    pub last_modified: Option<SystemTime>,
    /// A data do arquivo de origem (`x-amz-meta-mtime`), ou a `last_modified`.
    pub modified: Option<SystemTime>,
    /// Os metadados (`x-amz-meta-*`), sem o prefixo.
    pub metadata: Vec<(String, String)>,
}

/// Um envio em partes que não terminou.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingUpload {
    /// O caminho do objeto, no formato do [`Fs`].
    pub path: String,
    /// O identificador do envio.
    pub upload_id: String,
    /// Quando começou.
    pub initiated: Option<SystemTime>,
}

/// Um bucket (ou o serviço todo) como sistema de arquivos.
pub struct S3 {
    client: Arc<Client>,
    fixed: Option<String>,
    storage_class: Option<String>,
    part_size: u64,
    copy_limit: u64,
}

impl std::fmt::Debug for S3 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3")
            .field("endpoint", &self.client.endpoint().as_str())
            .field("bucket", &self.fixed)
            .finish_non_exhaustive()
    }
}

/// Um caminho do [`Fs`] separado em bucket e chave.
#[derive(Clone, Debug)]
struct Loc {
    bucket: String,
    /// Sem `/` no começo nem no fim; vazia na raiz do bucket.
    key: String,
}

struct Obj {
    key: String,
    size: u64,
    modified: Option<SystemTime>,
}

#[derive(Default)]
struct Page {
    objects: Vec<Obj>,
    prefixes: Vec<String>,
    next: Option<String>,
}

impl S3 {
    /// Abre a conexão e confere as chaves: com `bucket`, lista uma chave; sem ele, lista os
    /// buckets.
    pub fn connect(config: S3Config) -> io::Result<S3> {
        let endpoint = Url::parse(config.endpoint.trim()).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("O endereço do servidor S3 não é válido ({error}): use https://…"),
            )
        })?;
        if config.access_key.is_empty() || config.secret_key.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Faltam a chave de acesso e o segredo do S3",
            ));
        }
        let credentials = match config.session_token.as_deref().filter(|t| !t.is_empty()) {
            Some(token) => Credentials::new_with_token(
                config.access_key.clone(),
                config.secret_key.clone(),
                token,
            ),
            None => Credentials::new(config.access_key.clone(), config.secret_key.clone()),
        };
        let style = match config.url_style {
            S3UrlStyle::Path => UrlStyle::Path,
            S3UrlStyle::VirtualHost => UrlStyle::VirtualHost,
        };
        let region = if config.region.trim().is_empty() {
            "us-east-1".to_owned()
        } else {
            config.region.trim().to_owned()
        };
        let client = Client::new(
            endpoint,
            style,
            region,
            credentials,
            &config.tls,
            config.proxy.as_ref(),
            config.retry_base,
        )?;
        let s3 = S3 {
            client: Arc::new(client),
            fixed: config.bucket.filter(|b| !b.trim().is_empty()),
            storage_class: config.storage_class.filter(|c| !c.is_empty()),
            part_size: config.part_size.max(PART_SIZE_MINIMO),
            copy_limit: config.copy_limit.max(PART_SIZE_MINIMO),
        };
        match s3.fixed.clone() {
            Some(bucket) => {
                s3.page(&bucket, "", true, None, Some(1))?;
            }
            None => {
                s3.list_buckets()?;
            }
        }
        Ok(s3)
    }

    /// O link pré-assinado de um objeto, que baixa sem credenciais por até `valid_for` (no
    /// máximo 7 dias).
    pub fn presign_get(&self, path: &str, valid_for: Duration) -> io::Result<String> {
        if valid_for.is_zero() || valid_for > VALIDADE_MAXIMA_LINK {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "O link vale de 1 segundo a 7 dias",
            ));
        }
        let loc = self.object_loc(path, "gerar o link de")?;
        let bucket = self.client.bucket(&loc.bucket)?;
        let url =
            GetObject::new(&bucket, Some(self.client.credentials()), &loc.key).sign(valid_for);
        Ok(url.to_string())
    }

    /// Os dados do objeto (`HeadObject`).
    pub fn head(&mut self, path: &str) -> io::Result<ObjectInfo> {
        let loc = self.object_loc(path, "ler")?;
        self.head_loc(&loc)
            .map_err(|error| describe(error, "ler", path))
    }

    /// Troca a classe de armazenamento de um objeto (uma cópia sobre ele mesmo).
    pub fn set_storage_class(&mut self, path: &str, class: &str) -> io::Result<()> {
        let loc = self.object_loc(path, "mudar a classe de")?;
        let info = self
            .head_loc(&loc)
            .map_err(|error| describe(error, "ler", path))?;
        self.copy_one(
            &loc,
            &loc,
            info.size,
            &[
                ("x-amz-storage-class", class.to_owned()),
                ("x-amz-metadata-directive", "COPY".to_owned()),
            ],
        )
        .map_err(|error| describe(error, "mudar a classe de", path))
    }

    /// Os envios em partes que ficaram pendentes sob `prefix` (um caminho do [`Fs`]; `/` é tudo).
    pub fn pending_uploads(&mut self, prefix: &str) -> io::Result<Vec<PendingUpload>> {
        let mut found = Vec::new();
        match self.locate(prefix) {
            None => {
                for (name, _) in self.list_buckets()? {
                    // Um bucket sem permissão de listar os envios não impede os outros.
                    if let Ok(uploads) = upload::list_uploads(&self.client, &name, "") {
                        for (key, id, initiated) in uploads {
                            found.push(PendingUpload::new(
                                self.path_of(&name, &key),
                                id,
                                initiated,
                            ));
                        }
                    }
                }
            }
            Some(loc) => {
                let key_prefix = if loc.key.is_empty() {
                    String::new()
                } else {
                    format!("{}/", loc.key)
                };
                // O filtro `prefix` do servidor falha com pasta em alguns (o MinIO não devolve nada
                // para `pasta/`); por isso lista tudo e filtra aqui.
                for (key, id, initiated) in upload::list_uploads(&self.client, &loc.bucket, "")?
                    .into_iter()
                    .filter(|(key, _, _)| key.starts_with(&key_prefix))
                {
                    found.push(PendingUpload::new(
                        self.path_of(&loc.bucket, &key),
                        id,
                        initiated,
                    ));
                }
            }
        }
        Ok(found)
    }

    /// Desiste de um envio pendente e apaga as partes que ele guardava.
    pub fn abort_upload(&mut self, path: &str, upload_id: &str) -> io::Result<()> {
        let loc = self.object_loc(path, "abortar o envio de")?;
        upload::abort(&self.client, &loc.bucket, &loc.key, upload_id)
    }

    // ---- caminhos ----

    /// `None` na raiz do serviço (a lista de buckets).
    fn locate(&self, path: &str) -> Option<Loc> {
        let trimmed = path.trim_matches('/');
        match &self.fixed {
            Some(bucket) => Some(Loc {
                bucket: bucket.clone(),
                key: trimmed.to_owned(),
            }),
            None if trimmed.is_empty() => None,
            None => {
                let (bucket, key) = trimmed.split_once('/').unwrap_or((trimmed, ""));
                Some(Loc {
                    bucket: bucket.to_owned(),
                    key: key.trim_matches('/').to_owned(),
                })
            }
        }
    }

    /// Um caminho que tem de ser um objeto (com chave).
    fn object_loc(&self, path: &str, action: &str) -> io::Result<Loc> {
        match self.locate(path) {
            Some(loc) if !loc.key.is_empty() => Ok(loc),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Escolha um objeto dentro de um bucket para {action}: {path}"),
            )),
        }
    }

    fn path_of(&self, bucket: &str, key: &str) -> String {
        match &self.fixed {
            Some(_) => format!("/{key}"),
            None if key.is_empty() => format!("/{bucket}"),
            None => format!("/{bucket}/{key}"),
        }
    }

    // ---- consultas ----

    fn list_buckets(&self) -> io::Result<Vec<(String, Option<SystemTime>)>> {
        let mut response = self.client.call_service(|endpoint, region, credentials| {
            let url = sign(
                &jiff::Timestamp::now(),
                Method::Get,
                endpoint.clone(),
                credentials.key(),
                credentials.secret(),
                credentials.token(),
                region,
                VALIDADE.as_secs(),
                std::iter::empty(),
                std::iter::empty(),
            );
            Prepared {
                method: "GET",
                url,
                headers: Vec::new(),
                body: &[],
            }
        })?;
        let root = xml::parse(&text(&mut response)?)
            .ok_or_else(|| resposta_invalida("a lista de buckets"))?;
        let mut buckets = Vec::new();
        if let Some(list) = root.child("Buckets") {
            for bucket in list.all("Bucket") {
                if let Some(name) = bucket.text_of("Name") {
                    buckets.push((
                        name.trim().to_owned(),
                        bucket.text_of("CreationDate").and_then(parse_iso),
                    ));
                }
            }
        }
        Ok(buckets)
    }

    fn page(
        &self,
        bucket: &str,
        prefix: &str,
        delimiter: bool,
        token: Option<&str>,
        max_keys: Option<usize>,
    ) -> io::Result<Page> {
        let mut response = self.client.call(bucket, |bucket, credentials| {
            let mut action = ListObjectsV2::new(bucket, Some(credentials));
            if !prefix.is_empty() {
                action.with_prefix(prefix);
            }
            if delimiter {
                action.with_delimiter("/");
            }
            if let Some(token) = token {
                action.with_continuation_token(token);
            }
            if let Some(max) = max_keys {
                action.with_max_keys(max);
            }
            Prepared::sign(action, &[], &[])
        })?;
        let root = xml::parse(&text(&mut response)?)
            .ok_or_else(|| resposta_invalida("a lista de objetos"))?;
        let encoded = root.text_of("EncodingType").map(str::trim) == Some("url");
        let decode = |value: &str| {
            if encoded {
                // O `encoding-type=url` do S3 escreve o espaço como `+` (e o `+` como `%2B`).
                percent_decode_str(&value.replace('+', " "))
                    .decode_utf8_lossy()
                    .into_owned()
            } else {
                value.to_owned()
            }
        };
        let mut page = Page::default();
        for content in root.all("Contents") {
            page.objects.push(Obj {
                key: decode(content.text_of("Key").unwrap_or_default()),
                size: content
                    .text_of("Size")
                    .and_then(|size| size.trim().parse().ok())
                    .unwrap_or(0),
                modified: content.text_of("LastModified").and_then(parse_iso),
            });
        }
        for common in root.all("CommonPrefixes") {
            if let Some(prefix) = common.text_of("Prefix") {
                page.prefixes.push(decode(prefix));
            }
        }
        let truncated = root.text_of("IsTruncated").map(str::trim) != Some("false");
        page.next = root
            .text_of("NextContinuationToken")
            .map(|token| token.trim().to_owned())
            .filter(|token| !token.is_empty() && truncated);
        Ok(page)
    }

    /// Todos os objetos sob `prefix`, sem delimitador (a pasta inteira).
    fn all_under(&self, bucket: &str, prefix: &str) -> io::Result<Vec<Obj>> {
        let mut all = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let page = self.page(bucket, prefix, false, token.as_deref(), None)?;
            all.extend(page.objects);
            match page.next {
                Some(next) if token.as_deref() != Some(next.as_str()) => token = Some(next),
                _ => break,
            }
        }
        Ok(all)
    }

    fn head_loc(&self, loc: &Loc) -> io::Result<ObjectInfo> {
        let response = self.client.call(&loc.bucket, |bucket, credentials| {
            Prepared::sign(
                HeadObject::new(bucket, Some(credentials), &loc.key),
                &[],
                &[],
            )
        })?;
        Ok(object_info(&response))
    }

    fn entry_of_dir(&self, name: &str) -> Entry {
        Entry::new(name, EntryKind::Dir)
    }

    fn stat_loc(&self, path: &str) -> io::Result<Entry> {
        let name = path::file_name(path).to_owned();
        let Some(loc) = self.locate(path) else {
            return Ok(self.entry_of_dir("/"));
        };
        if loc.key.is_empty() {
            // A raiz de um bucket: confere que existe.
            if self.fixed.is_none() {
                self.page(&loc.bucket, "", true, None, Some(1))?;
            }
            return Ok(self.entry_of_dir(&name));
        }
        match self.head_loc(&loc) {
            Ok(info) => {
                let mut entry = Entry::new(name, EntryKind::File);
                entry.size = info.size;
                entry.modified = info.modified;
                Ok(entry)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let prefix = format!("{}/", loc.key);
                let page = self.page(&loc.bucket, &prefix, false, None, Some(1))?;
                if page.objects.is_empty() && page.prefixes.is_empty() {
                    Err(error)
                } else {
                    Ok(self.entry_of_dir(&name))
                }
            }
            Err(error) => Err(error),
        }
    }

    // ---- cópia e remoção ----

    /// Copia `from` para `to` dentro do servidor, em partes se passar do limite.
    fn copy_one(
        &self,
        from: &Loc,
        to: &Loc,
        size: u64,
        extra: &[(&'static str, String)],
    ) -> io::Result<()> {
        let source = format!(
            "/{}/{}",
            from.bucket,
            utf8_percent_encode(&from.key, COPY_SOURCE)
        );
        if size <= self.copy_limit {
            let mut headers = vec![("x-amz-copy-source", source)];
            headers.extend(extra.iter().cloned());
            let mut response = self.client.call(&to.bucket, |bucket, credentials| {
                Prepared::sign(
                    PutObject::new(bucket, Some(credentials), &to.key),
                    &headers,
                    &[],
                )
            })?;
            return check_ok_body(&text(&mut response)?);
        }
        // Acima do limite: UploadPartCopy, parte por parte.
        let info = self.head_loc(from)?;
        let mut headers: Vec<(&str, String)> = Vec::new();
        if let Some(content_type) = &info.content_type {
            headers.push(("content-type", content_type.clone()));
        }
        let class = extra
            .iter()
            .find(|(name, _)| *name == "x-amz-storage-class")
            .map(|(_, value)| value.clone())
            .or_else(|| (info.storage_class != "STANDARD").then(|| info.storage_class.clone()));
        if let Some(class) = class {
            headers.push(("x-amz-storage-class", class));
        }
        let metadata: Vec<(String, String)> = info
            .metadata
            .iter()
            .map(|(name, value)| (format!("x-amz-meta-{name}"), value.clone()))
            .collect();
        for (name, value) in &metadata {
            headers.push((name.as_str(), value.clone()));
        }
        let id = upload::create(&self.client, &to.bucket, &to.key, &headers)?;
        let result = (|| {
            let mut etags = Vec::new();
            let mut start = 0u64;
            while start < size {
                let end = (start + self.copy_limit).min(size) - 1;
                let number = u16::try_from(etags.len() + 1)
                    .ok()
                    .filter(|number| usize::from(*number) <= PARTES_MAXIMAS)
                    .ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidInput, "O objeto tem partes demais")
                    })?;
                let part_headers = vec![
                    ("x-amz-copy-source", source.clone()),
                    ("x-amz-copy-source-range", format!("bytes={start}-{end}")),
                ];
                let mut response = self.client.call(&to.bucket, |bucket, credentials| {
                    Prepared::sign(
                        UploadPart::new(bucket, Some(credentials), &to.key, number, &id),
                        &part_headers,
                        &[],
                    )
                })?;
                let body = text(&mut response)?;
                check_ok_body(&body)?;
                let etag = xml::parse(&body)
                    .and_then(|root| root.text_of("ETag").map(|etag| etag.trim().to_owned()))
                    .or_else(|| header(&response, "etag"))
                    .ok_or_else(|| resposta_invalida("a cópia de uma parte"))?;
                etags.push(etag);
                start = end + 1;
            }
            upload::complete(&self.client, &to.bucket, &to.key, &id, &etags)
        })();
        if result.is_err() {
            let _ = upload::abort(&self.client, &to.bucket, &to.key, &id);
        }
        result
    }

    fn delete_keys(&self, bucket: &str, keys: &[String]) -> io::Result<()> {
        for batch in keys.chunks(1000) {
            let identifiers: Vec<ObjectIdentifier> = batch
                .iter()
                .map(|key| ObjectIdentifier::new(key.clone()))
                .collect();
            let (body, md5) = {
                let bucket = self.client.bucket(bucket)?;
                let mut action = DeleteObjects::new(&bucket, None, identifiers.iter());
                action.set_quiet(true);
                action.body_with_md5()
            };
            let result = self.client.call(bucket, |bucket, credentials| {
                Prepared::sign(
                    DeleteObjects::new(bucket, Some(credentials), std::iter::empty()),
                    &[("content-md5", md5.clone())],
                    body.as_bytes(),
                )
            });
            match result {
                Ok(mut response) => {
                    let answer = text(&mut response)?;
                    if let Some(root) = xml::parse(&answer)
                        && let Some(error) = root.child("Error")
                    {
                        return Err(io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            format!(
                                "Não foi possível apagar {}: {} ({})",
                                error.text_of("Key").unwrap_or_default(),
                                error.text_of("Message").unwrap_or_default(),
                                error.text_of("Code").unwrap_or_default()
                            ),
                        ));
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Unsupported => {
                    // O servidor não tem o apagar em lote: um por um.
                    for key in batch {
                        self.delete_key(bucket, key)?;
                    }
                }
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn delete_key(&self, bucket: &str, key: &str) -> io::Result<()> {
        self.client.call(bucket, |bucket, credentials| {
            Prepared::sign(DeleteObject::new(bucket, Some(credentials), key), &[], &[])
        })?;
        Ok(())
    }

    // ---- envio ----

    fn upload_headers(
        &self,
        key: &str,
        modified: Option<SystemTime>,
    ) -> Vec<(&'static str, String)> {
        let mut headers = vec![(
            "content-type",
            mime_guess::from_path(key)
                .first_or_octet_stream()
                .essence_str()
                .to_owned(),
        )];
        if let Some(class) = &self.storage_class {
            headers.push(("x-amz-storage-class", class.clone()));
        }
        if let Some(time) = modified {
            headers.push((CABECALHO_MTIME, format_mtime(time)));
        }
        headers
    }

    /// O envio pendente de `key` cujas partes seguidas somam `bytes` (ou, com `None`, o mais
    /// novo), com as partes.
    fn find_pending(
        &self,
        loc: &Loc,
        bytes: Option<u64>,
    ) -> io::Result<Option<(String, Vec<upload::Part>)>> {
        let uploads = upload::list_uploads(&self.client, &loc.bucket, &loc.key)?;
        for (key, id, _) in uploads {
            if key != loc.key {
                continue;
            }
            let parts = upload::list_parts(&self.client, &loc.bucket, &loc.key, &id)?;
            if bytes.is_none_or(|bytes| contiguous_bytes(&parts) == bytes) {
                return Ok(Some((id, parts)));
            }
        }
        Ok(None)
    }
}

impl Fs for S3 {
    fn caps(&self) -> Caps {
        Caps {
            permissions: false,
            set_modified: false,
            symlinks: false,
            resume_upload: true,
            owner: false,
        }
    }

    fn home(&mut self) -> io::Result<String> {
        Ok("/".to_owned())
    }

    fn list(&mut self, dir: &str) -> io::Result<Vec<Entry>> {
        let Some(loc) = self.locate(dir) else {
            return Ok(self
                .list_buckets()?
                .into_iter()
                .map(|(name, created)| {
                    let mut entry = Entry::new(name, EntryKind::Dir);
                    entry.modified = created;
                    entry
                })
                .collect());
        };
        let prefix = if loc.key.is_empty() {
            String::new()
        } else {
            format!("{}/", loc.key)
        };
        let mut entries = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let page = self
                .page(&loc.bucket, &prefix, true, token.as_deref(), None)
                .map_err(|error| describe(error, "listar", dir))?;
            for common in &page.prefixes {
                let name = common[prefix.len().min(common.len())..].trim_end_matches('/');
                if !name.is_empty() {
                    entries.push(Entry::new(name, EntryKind::Dir));
                }
            }
            for object in &page.objects {
                // O marcador de pasta (a chave é o próprio prefixo) não é um arquivo.
                let name = &object.key[prefix.len().min(object.key.len())..];
                if name.is_empty() || name.ends_with('/') {
                    continue;
                }
                let mut entry = Entry::new(name, EntryKind::File);
                entry.size = object.size;
                entry.modified = object.modified;
                entries.push(entry);
            }
            match page.next {
                Some(next) if token.as_deref() != Some(next.as_str()) => token = Some(next),
                _ => break,
            }
        }
        Ok(entries)
    }

    fn stat(&mut self, target: &str) -> io::Result<Entry> {
        self.stat_loc(target)
            .map_err(|error| describe(error, "ler", target))
    }

    fn lstat(&mut self, target: &str) -> io::Result<Entry> {
        self.stat(target)
    }

    fn open_read(&mut self, target: &str, offset: u64) -> io::Result<Box<dyn Download + '_>> {
        let loc = self.object_loc(target, "ler")?;
        let range = (offset > 0).then(|| ("range", format!("bytes={offset}-")));
        let headers: Vec<(&str, String)> = range.into_iter().collect();
        let response = self
            .client
            .call(&loc.bucket, |bucket, credentials| {
                Prepared::sign(
                    GetObject::new(bucket, Some(credentials), &loc.key),
                    &headers,
                    &[],
                )
            })
            .map_err(|error| describe(error, "ler", target))?;
        let status = response.status().as_u16();
        let mut reader = response.into_body().into_reader();
        if offset > 0 && status == 200 {
            // O servidor ignorou o `Range`: pula o que já foi lido.
            io::copy(&mut (&mut reader).take(offset), &mut io::sink())?;
        }
        Ok(Box::new(S3Download {
            reader: Box::new(reader),
        }))
    }

    fn open_write(&mut self, target: &str, offset: u64) -> io::Result<Box<dyn Upload + '_>> {
        self.open_write_with(target, offset, None)
    }

    fn open_write_with(
        &mut self,
        target: &str,
        offset: u64,
        modified: Option<SystemTime>,
    ) -> io::Result<Box<dyn Upload + '_>> {
        let loc = self.object_loc(target, "gravar em")?;
        let mut multipart = None;
        if offset > 0 {
            let (id, parts) = self
                .find_pending(&loc, Some(offset))
                .map_err(|error| describe(error, "gravar em", target))?
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "Não há um envio pendente de {offset} bytes para retomar: {target}"
                        ),
                    )
                })?;
            multipart = Some((id, parts.into_iter().map(|part| part.etag).collect()));
        }
        Ok(Box::new(S3Upload {
            client: self.client.clone(),
            bucket: loc.bucket,
            headers: self.upload_headers(&loc.key, modified),
            key: loc.key,
            part_size: self.part_size,
            buffer: Vec::new(),
            multipart,
        }))
    }

    fn resume_offset(&mut self, target: &str) -> io::Result<u64> {
        let Some(loc) = self.locate(target).filter(|loc| !loc.key.is_empty()) else {
            return Ok(0);
        };
        Ok(self
            .find_pending(&loc, None)?
            .map_or(0, |(_, parts)| contiguous_bytes(&parts)))
    }

    fn mkdir(&mut self, target: &str) -> io::Result<()> {
        let loc = match self.locate(target) {
            Some(loc) if !loc.key.is_empty() => loc,
            _ => return Err(unsupported("criar buckets (use o console do provedor)")),
        };
        let key = format!("{}/", loc.key);
        self.client
            .call(&loc.bucket, |bucket, credentials| {
                Prepared::sign(PutObject::new(bucket, Some(credentials), &key), &[], &[])
            })
            .map(|_| ())
            .map_err(|error| describe(error, "criar", target))
    }

    fn remove_file(&mut self, target: &str) -> io::Result<()> {
        let loc = self.object_loc(target, "apagar")?;
        self.delete_key(&loc.bucket, &loc.key)
            .map_err(|error| describe(error, "apagar", target))
    }

    fn remove_dir(&mut self, target: &str) -> io::Result<()> {
        let loc = match self.locate(target) {
            Some(loc) if !loc.key.is_empty() => loc,
            _ => return Err(unsupported("apagar buckets (use o console do provedor)")),
        };
        let prefix = format!("{}/", loc.key);
        let keys: Vec<String> = self
            .all_under(&loc.bucket, &prefix)
            .map_err(|error| describe(error, "apagar", target))?
            .into_iter()
            .map(|object| object.key)
            .collect();
        self.delete_keys(&loc.bucket, &keys)
            .map_err(|error| describe(error, "apagar", target))
    }

    fn rename(&mut self, from: &str, to: &str, overwrite: bool) -> io::Result<()> {
        let source = self.object_or_dir(from)?;
        let destination = self.object_loc(to, "renomear para")?;
        if !overwrite && self.stat_loc(to).is_ok() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("já existe: {to}"),
            ));
        }
        match source {
            (loc, Some(size)) => {
                self.copy_one(&loc, &destination, size, &[])
                    .map_err(|error| describe(error, "renomear", from))?;
                self.delete_key(&loc.bucket, &loc.key)
                    .map_err(|error| describe(error, "renomear", from))
            }
            (loc, None) => {
                let from_prefix = format!("{}/", loc.key);
                let to_prefix = format!("{}/", destination.key);
                let objects = self
                    .all_under(&loc.bucket, &from_prefix)
                    .map_err(|error| describe(error, "renomear", from))?;
                let mut keys = Vec::with_capacity(objects.len());
                for object in objects {
                    let rest = &object.key[from_prefix.len().min(object.key.len())..];
                    let target = Loc {
                        bucket: destination.bucket.clone(),
                        key: format!("{to_prefix}{rest}"),
                    };
                    let origin = Loc {
                        bucket: loc.bucket.clone(),
                        key: object.key.clone(),
                    };
                    self.copy_one(&origin, &target, object.size, &[])
                        .map_err(|error| describe(error, "renomear", from))?;
                    keys.push(object.key);
                }
                self.delete_keys(&loc.bucket, &keys)
                    .map_err(|error| describe(error, "renomear", from))
            }
        }
    }

    fn set_permissions(&mut self, _path: &str, _mode: u32) -> io::Result<()> {
        Err(unsupported("mudar as permissões"))
    }

    fn set_modified(&mut self, _path: &str, _time: SystemTime) -> io::Result<()> {
        // O S3 não muda a data: ela vai no envio (`x-amz-meta-mtime`, por `open_write_with`).
        Err(unsupported("mudar a data"))
    }

    fn symlink(&mut self, _target: &str, _link: &str) -> io::Result<()> {
        Err(unsupported("criar links simbólicos"))
    }

    fn read_link(&mut self, _path: &str) -> io::Result<String> {
        Err(unsupported("ler links simbólicos"))
    }
}

impl S3 {
    /// Para o `rename`: o objeto com o tamanho, ou a pasta (`None`).
    fn object_or_dir(&self, path: &str) -> io::Result<(Loc, Option<u64>)> {
        let loc = self.object_loc(path, "renomear")?;
        let entry = self
            .stat_loc(path)
            .map_err(|error| describe(error, "renomear", path))?;
        Ok(if entry.is_dir() {
            (loc, None)
        } else {
            (loc, Some(entry.size))
        })
    }
}

struct S3Download {
    reader: Box<dyn Read + Send>,
}

impl Read for S3Download {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.reader.read(buffer)
    }
}

impl Download for S3Download {
    fn finish(self: Box<Self>) -> io::Result<()> {
        Ok(())
    }
}

fn object_info(response: &ureq::http::Response<ureq::Body>) -> ObjectInfo {
    let get = |name: &str| header(response, name);
    let mut metadata = Vec::new();
    for (name, value) in response.headers() {
        if let (Some(name), Ok(value)) = (name.as_str().strip_prefix("x-amz-meta-"), value.to_str())
        {
            metadata.push((name.to_owned(), value.to_owned()));
        }
    }
    let last_modified = get("last-modified").and_then(|date| parse_http_date(&date));
    let mtime = metadata
        .iter()
        .find(|(name, _)| name == "mtime")
        .and_then(|(_, value)| parse_mtime(value));
    ObjectInfo {
        size: get("content-length")
            .and_then(|size| size.trim().parse().ok())
            .unwrap_or(0),
        etag: get("etag").map(|etag| etag.trim_matches('"').to_owned()),
        content_type: get("content-type"),
        storage_class: get("x-amz-storage-class").unwrap_or_else(|| "STANDARD".to_owned()),
        last_modified,
        modified: mtime.or(last_modified),
        metadata,
    }
}

pub(crate) fn parse_iso(text: &str) -> Option<SystemTime> {
    text.trim()
        .parse::<jiff::Timestamp>()
        .ok()
        .map(SystemTime::from)
}

fn parse_http_date(text: &str) -> Option<SystemTime> {
    jiff::fmt::rfc2822::parse(text.trim())
        .ok()
        .map(|zoned| SystemTime::from(zoned.timestamp()))
}

/// A data como o rclone guarda: segundos com fração.
fn format_mtime(time: SystemTime) -> String {
    let since = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    format!("{}.{:09}", since.as_secs(), since.subsec_nanos())
}

fn parse_mtime(text: &str) -> Option<SystemTime> {
    let seconds: f64 = text.trim().parse().ok()?;
    (seconds.is_finite() && seconds >= 0.0).then(|| UNIX_EPOCH + Duration::from_secs_f64(seconds))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_data_do_rclone_vai_e_volta() {
        let tempo = UNIX_EPOCH + Duration::new(1_700_000_000, 123_456_789);
        let texto = format_mtime(tempo);
        assert_eq!(texto, "1700000000.123456789");
        let volta = parse_mtime(&texto).unwrap();
        let diferenca = volta
            .duration_since(tempo)
            .unwrap_or_else(|erro| erro.duration());
        assert!(diferenca < Duration::from_micros(1));
        assert!(parse_mtime("abc").is_none());
    }

    #[test]
    fn as_datas_do_servidor() {
        assert_eq!(
            parse_iso("2024-01-02T03:04:05.000Z"),
            Some(UNIX_EPOCH + Duration::from_secs(1_704_164_645))
        );
        assert_eq!(
            parse_http_date("Tue, 02 Jan 2024 03:04:05 GMT"),
            Some(UNIX_EPOCH + Duration::from_secs(1_704_164_645))
        );
    }

    #[test]
    fn a_parte_cresce_a_cada_mil() {
        assert_eq!(upload::part_len(8, 1), 8);
        assert_eq!(upload::part_len(8, 1000), 8);
        assert_eq!(upload::part_len(8, 1001), 16);
        assert_eq!(upload::part_len(8, 10_000), 8 << 5);
    }
}
