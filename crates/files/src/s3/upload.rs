//! O envio em partes (multipart) e a conversa com o servidor sobre os envios pendentes.

use std::io::{self, Write};
use std::sync::Arc;
use std::time::SystemTime;

use rusty_s3::actions::{
    AbortMultipartUpload, CompleteMultipartUpload, CreateMultipartUpload, ListParts, PutObject,
    UploadPart,
};
use rusty_s3::signing::sign;
use rusty_s3::{Map, Method};

use super::client::{Client, Prepared, VALIDADE, header, text};
use super::{PARTES_MAXIMAS, PendingUpload, parse_iso, xml};
use crate::fs::Upload;

/// Uma parte enviada.
#[derive(Clone, Debug)]
pub(crate) struct Part {
    pub number: u16,
    pub size: u64,
    pub etag: String,
}

/// Cria o envio em partes e devolve o `upload_id`.
pub(crate) fn create(
    client: &Client,
    bucket: &str,
    key: &str,
    headers: &[(&str, String)],
) -> io::Result<String> {
    let mut response = client.call(bucket, |bucket, credentials| {
        Prepared::sign(
            CreateMultipartUpload::new(bucket, Some(credentials), key),
            headers,
            &[],
        )
    })?;
    let body = text(&mut response)?;
    xml::parse(&body)
        .and_then(|root| root.text_of("UploadId").map(|id| id.trim().to_owned()))
        .filter(|id| !id.is_empty())
        .ok_or_else(|| resposta_invalida("o início do envio em partes"))
}

/// Envia uma parte e devolve o `ETag`.
pub(crate) fn put_part(
    client: &Client,
    bucket: &str,
    key: &str,
    upload_id: &str,
    number: u16,
    data: &[u8],
) -> io::Result<String> {
    let response = client.call(bucket, |bucket, credentials| {
        Prepared::sign(
            UploadPart::new(bucket, Some(credentials), key, number, upload_id),
            &[],
            data,
        )
    })?;
    header(&response, "etag").ok_or_else(|| resposta_invalida("o envio de uma parte (sem ETag)"))
}

/// Junta as partes. Confere o `<Error>` que alguns servidores põem num 200.
pub(crate) fn complete(
    client: &Client,
    bucket: &str,
    key: &str,
    upload_id: &str,
    etags: &[String],
) -> io::Result<()> {
    let body = CompleteMultipartUpload::new(
        &client.bucket(bucket)?,
        None,
        key,
        upload_id,
        etags.iter().map(String::as_str),
    )
    .body();
    let mut response = client.call(bucket, |bucket, credentials| {
        Prepared::sign(
            CompleteMultipartUpload::new(
                bucket,
                Some(credentials),
                key,
                upload_id,
                std::iter::empty::<&str>(),
            ),
            &[],
            body.as_bytes(),
        )
    })?;
    check_ok_body(&text(&mut response)?)
}

/// Desiste de um envio pendente.
pub(crate) fn abort(client: &Client, bucket: &str, key: &str, upload_id: &str) -> io::Result<()> {
    client.call(bucket, |bucket, credentials| {
        Prepared::sign(
            AbortMultipartUpload::new(bucket, Some(credentials), key, upload_id),
            &[],
            &[],
        )
    })?;
    Ok(())
}

/// As partes já enviadas de um envio pendente, em ordem.
pub(crate) fn list_parts(
    client: &Client,
    bucket: &str,
    key: &str,
    upload_id: &str,
) -> io::Result<Vec<Part>> {
    let mut parts = Vec::new();
    let mut marker: Option<u16> = None;
    loop {
        let mut response = client.call(bucket, |bucket, credentials| {
            let mut action = ListParts::new(bucket, Some(credentials), key, upload_id);
            if let Some(marker) = marker {
                action.set_part_number_marker(marker);
            }
            Prepared::sign(action, &[], &[])
        })?;
        let root = xml::parse(&text(&mut response)?)
            .ok_or_else(|| resposta_invalida("a lista de partes"))?;
        for part in root.all("Part") {
            parts.push(Part {
                number: part
                    .text_of("PartNumber")
                    .and_then(|n| n.trim().parse().ok())
                    .unwrap_or(0),
                size: part
                    .text_of("Size")
                    .and_then(|n| n.trim().parse().ok())
                    .unwrap_or(0),
                etag: part.text_of("ETag").unwrap_or_default().trim().to_owned(),
            });
        }
        let truncated = root.text_of("IsTruncated").map(str::trim) == Some("true");
        let next = root
            .text_of("NextPartNumberMarker")
            .and_then(|n| n.trim().parse::<u16>().ok());
        match next {
            Some(next) if truncated && Some(next) != marker => marker = Some(next),
            _ => break,
        }
    }
    parts.sort_by_key(|part| part.number);
    Ok(parts)
}

/// Quantos bytes das partes seguidas desde a 1.
pub(crate) fn contiguous_bytes(parts: &[Part]) -> u64 {
    let mut total = 0;
    for (index, part) in parts.iter().enumerate() {
        if usize::from(part.number) != index + 1 {
            break;
        }
        total += part.size;
    }
    total
}

/// Os envios pendentes de `bucket` com este prefixo de chave, do mais novo ao mais velho.
pub(crate) fn list_uploads(
    client: &Client,
    bucket: &str,
    prefix: &str,
) -> io::Result<Vec<(String, String, Option<SystemTime>)>> {
    let mut found = Vec::new();
    let mut key_marker = String::new();
    let mut id_marker = String::new();
    loop {
        let mut response = client.call(bucket, |bucket, credentials| {
            let mut query = Map::new();
            query.insert("uploads", "");
            if !prefix.is_empty() {
                query.insert("prefix", prefix);
            }
            if !key_marker.is_empty() {
                query.insert("key-marker", key_marker.as_str());
            }
            if !id_marker.is_empty() {
                query.insert("upload-id-marker", id_marker.as_str());
            }
            let url = sign(
                &jiff::Timestamp::now(),
                Method::Get,
                bucket.base_url().clone(),
                credentials.key(),
                credentials.secret(),
                credentials.token(),
                bucket.region(),
                VALIDADE.as_secs(),
                query.iter(),
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
            .ok_or_else(|| resposta_invalida("a lista de envios pendentes"))?;
        for upload in root.all("Upload") {
            let key = upload.text_of("Key").unwrap_or_default().to_owned();
            let id = upload.text_of("UploadId").unwrap_or_default().to_owned();
            let initiated = upload.text_of("Initiated").and_then(parse_iso);
            if !key.is_empty() && !id.is_empty() {
                found.push((key, id, initiated));
            }
        }
        let truncated = root.text_of("IsTruncated").map(str::trim) == Some("true");
        let next_key = root.text_of("NextKeyMarker").unwrap_or_default().to_owned();
        let next_id = root
            .text_of("NextUploadIdMarker")
            .unwrap_or_default()
            .to_owned();
        if truncated && (next_key != key_marker || next_id != id_marker) {
            key_marker = next_key;
            id_marker = next_id;
        } else {
            break;
        }
    }
    found.sort_by_key(|(_, _, initiated)| std::cmp::Reverse(*initiated));
    Ok(found)
}

/// Um 200 com `<Error>` no corpo (a cópia e o `Complete` fazem isso) é erro.
pub(crate) fn check_ok_body(body: &str) -> io::Result<()> {
    if body.contains("<Error>") || body.contains("<Error ") {
        let parsed = super::client::ErrorBody::parse(body);
        return Err(super::client::translate(500, &parsed, None));
    }
    Ok(())
}

pub(crate) fn resposta_invalida(what: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("O servidor S3 respondeu algo que não se entende em {what}"),
    )
}

/// O tamanho da parte de número `number`: cresce a cada mil partes para caber no limite de
/// dez mil (até 32 vezes o tamanho base).
pub(crate) fn part_len(base: u64, number: u16) -> u64 {
    let tier = (u32::from(number).saturating_sub(1) / 1000).min(5);
    base << tier
}

/// O arquivo sendo enviado: junta os bytes até uma parte e a manda; no fim, um `PutObject` se
/// coube numa parte só, ou o `Complete` das partes.
pub(crate) struct S3Upload {
    pub client: Arc<Client>,
    pub bucket: String,
    pub key: String,
    pub part_size: u64,
    pub headers: Vec<(&'static str, String)>,
    pub buffer: Vec<u8>,
    /// O envio em partes, se já começou (ou se foi retomado).
    pub multipart: Option<(String, Vec<String>)>,
}

impl S3Upload {
    fn next_number(&self) -> u16 {
        let sent = self.multipart.as_ref().map_or(0, |(_, etags)| etags.len());
        u16::try_from(sent + 1).unwrap_or(u16::MAX)
    }

    fn send_part(&mut self, size: usize) -> io::Result<()> {
        if self.multipart.is_none() {
            let id = create(&self.client, &self.bucket, &self.key, &self.headers)?;
            self.multipart = Some((id, Vec::new()));
        }
        let number = self.next_number();
        if usize::from(number) > PARTES_MAXIMAS {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "O envio passou do limite de dez mil partes do S3",
            ));
        }
        let rest = self.buffer.split_off(size);
        let chunk = std::mem::replace(&mut self.buffer, rest);
        let (id, _) = self.multipart.as_ref().expect("começou acima");
        let etag = put_part(&self.client, &self.bucket, &self.key, id, number, &chunk)?;
        if let Some((_, etags)) = self.multipart.as_mut() {
            etags.push(etag);
        }
        Ok(())
    }
}

impl Write for S3Upload {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.buffer.extend_from_slice(data);
        loop {
            let size =
                usize::try_from(part_len(self.part_size, self.next_number())).unwrap_or(usize::MAX);
            if self.buffer.len() < size {
                break;
            }
            self.send_part(size)?;
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Upload for S3Upload {
    fn finish(mut self: Box<Self>) -> io::Result<()> {
        if self.multipart.is_none() {
            let body = std::mem::take(&mut self.buffer);
            let key = self.key.clone();
            let headers = self.headers.clone();
            self.client.call(&self.bucket, |bucket, credentials| {
                Prepared::sign(
                    PutObject::new(bucket, Some(credentials), &key),
                    &headers,
                    &body,
                )
            })?;
            return Ok(());
        }
        if !self.buffer.is_empty() {
            let size = self.buffer.len();
            self.send_part(size)?;
        }
        let (id, etags) = self.multipart.take().expect("começou acima");
        complete(&self.client, &self.bucket, &self.key, &id, &etags)
    }
}

impl PendingUpload {
    pub(crate) fn new(path: String, upload_id: String, initiated: Option<SystemTime>) -> Self {
        PendingUpload {
            path,
            upload_id,
            initiated,
        }
    }
}
