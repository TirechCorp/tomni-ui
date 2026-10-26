//! Um servidor S3 de teste, em processo, que imita o MinIO e a AWS.
//!
//! Sobe em `127.0.0.1:0`, uma thread por conexão (com *keep-alive*), HTTP/1.1 escrito à mão, e
//! guarda tudo na memória. Fala o estilo de caminho (`/bucket/chave`). Confere cada pedido
//! como o S3 de verdade: **refaz a assinatura** SigV4 por query (a mesma conta do cliente, com
//! o `rusty-s3`) e recusa a chave errada (`InvalidAccessKeyId`), o segredo errado
//! (`SignatureDoesNotMatch`), o prazo vencido (`AccessDenied`) e a região trocada
//! (`AuthorizationHeaderMalformed`, com o `x-amz-bucket-region`).
//!
//! Entende `ListBuckets`, `ListObjectsV2` (com `delimiter`, `encoding-type=url` e páginas de
//! mil), `HeadObject`, `GetObject` (com `Range`), `PutObject`, `CopyObject`, `DeleteObject`,
//! `DeleteObjects` (com `Content-MD5` obrigatório e no máximo mil chaves), o envio em partes
//! (`CreateMultipartUpload`, `UploadPart`, `UploadPartCopy`, `ListParts`, `ListMultipartUploads`,
//! `CompleteMultipartUpload` com a regra das partes de 5 MiB, `AbortMultipartUpload`) e os erros
//! `<Error>`. Os `ETag`s não são MD5 (basta que sejam estáveis). Com `tls`, usa um certificado
//! autoassinado do rcgen.

#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime};

use percent_encoding::percent_decode_str;
use rustls::crypto::ring;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned, version};
use rusty_s3::Method;
use rusty_s3::signing::sign;
use tomni_ui_files::s3::{S3Config, S3UrlStyle, TlsMode};
use url::Url;

/// A chave de acesso do servidor.
pub const ACESSO: &str = "AKIATESTE0123456789";
/// O segredo (com `/` e `+`, que dão trabalho na assinatura).
pub const SEGREDO: &str = "segredo/de+teste";
/// A menor parte que não é a última.
const PARTE_MINIMA: usize = 5 << 20;

/// O que o servidor faz de diferente.
#[derive(Clone)]
pub struct Opcoes {
    /// Fala HTTPS com um certificado autoassinado.
    pub tls: bool,
    /// A região em que as chaves valem; outra dá `AuthorizationHeaderMalformed`.
    pub regiao: &'static str,
    /// Ignora o cabeçalho `Range` (devolve 200 com o objeto inteiro).
    pub ignorar_range: bool,
}

impl Default for Opcoes {
    fn default() -> Self {
        Opcoes {
            tls: false,
            regiao: "us-east-1",
            ignorar_range: false,
        }
    }
}

struct Objeto {
    dados: Vec<u8>,
    tipo: String,
    meta: BTreeMap<String, String>,
    classe: String,
    modificado: SystemTime,
    etag: String,
}

struct Envio {
    bucket: String,
    chave: String,
    tipo: String,
    meta: BTreeMap<String, String>,
    classe: String,
    iniciado: SystemTime,
    partes: BTreeMap<u32, (Vec<u8>, String)>,
}

#[derive(Default)]
struct Estado {
    buckets: BTreeMap<String, BTreeMap<String, Objeto>>,
    envios: BTreeMap<String, Envio>,
    proximo_id: u64,
    pedidos: Vec<String>,
}

struct Compartilhado {
    estado: Mutex<Estado>,
    opcoes: Opcoes,
    falhas: AtomicUsize,
    esquema: &'static str,
}

/// O servidor. Para ao ser solto.
pub struct S3d {
    endereco: SocketAddr,
    compartilhado: Arc<Compartilhado>,
    parar: Arc<AtomicBool>,
}

struct Pedido {
    metodo: String,
    caminho: String,
    consulta: Vec<(String, String)>,
    cabecalhos: HashMap<String, String>,
    corpo: Vec<u8>,
    esquema: &'static str,
}

impl Pedido {
    fn q(&self, nome: &str) -> Option<&str> {
        self.consulta
            .iter()
            .find(|(k, _)| k == nome)
            .map(|(_, v)| v.as_str())
    }

    fn tem(&self, nome: &str) -> bool {
        self.consulta.iter().any(|(k, _)| k == nome)
    }

    fn h(&self, nome: &str) -> Option<&str> {
        self.cabecalhos.get(nome).map(String::as_str)
    }
}

struct Resposta {
    status: u16,
    cabecalhos: Vec<(String, String)>,
    corpo: Vec<u8>,
}

type R = Result<Resposta, Resposta>;

impl Resposta {
    fn vazia(status: u16) -> Resposta {
        Resposta {
            status,
            cabecalhos: Vec::new(),
            corpo: Vec::new(),
        }
    }

    fn xml(corpo: String) -> Resposta {
        Resposta {
            status: 200,
            cabecalhos: vec![("content-type".into(), "application/xml".into())],
            corpo: format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n{corpo}").into_bytes(),
        }
    }

    fn erro(status: u16, codigo: &str, mensagem: &str) -> Resposta {
        let mut resposta = Resposta::xml(format!(
            "<Error><Code>{codigo}</Code><Message>{}</Message><RequestId>teste</RequestId></Error>",
            esc(mensagem)
        ));
        resposta.status = status;
        resposta
    }

    fn com(mut self, nome: &str, valor: impl Into<String>) -> Resposta {
        self.cabecalhos.push((nome.to_owned(), valor.into()));
        self
    }
}

fn esc(texto: &str) -> String {
    texto
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn desfazer_esc(texto: &str) -> String {
    texto
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// O `encoding-type=url` do S3: o espaço vira `+`.
fn codifica_url(texto: &str) -> String {
    let mut saida = String::new();
    for byte in texto.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                saida.push(byte as char);
            }
            b' ' => saida.push('+'),
            outro => saida.push_str(&format!("%{outro:02X}")),
        }
    }
    saida
}

fn etag_de(dados: &[u8]) -> String {
    let mut a = std::collections::hash_map::DefaultHasher::new();
    dados.hash(&mut a);
    let mut b = std::collections::hash_map::DefaultHasher::new();
    (dados.len(), 7u8).hash(&mut b);
    dados.hash(&mut b);
    format!("{:016x}{:016x}", a.finish(), b.finish())
}

fn iso(tempo: SystemTime) -> String {
    jiff::Timestamp::try_from(tempo)
        .unwrap()
        .strftime("%Y-%m-%dT%H:%M:%S.000Z")
        .to_string()
}

fn data_http(tempo: SystemTime) -> String {
    jiff::Timestamp::try_from(tempo)
        .unwrap()
        .strftime("%a, %d %b %Y %H:%M:%S GMT")
        .to_string()
}

/// O certificado do servidor, a chave e o PEM da CA que o assinou, uma vez por processo. Uma CA de
/// verdade: um certificado autoassinado de folha não vale como âncora de confiança.
fn certificado() -> &'static (Vec<CertificateDer<'static>>, Vec<u8>, String) {
    static CERT: OnceLock<(Vec<CertificateDer<'static>>, Vec<u8>, String)> = OnceLock::new();
    CERT.get_or_init(|| {
        let mut ca_params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        ca_params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "CA do servidor S3 de teste");
        let ca_chave = rcgen::KeyPair::generate().unwrap();
        let ca = ca_params.self_signed(&ca_chave).unwrap();
        let emissor = rcgen::Issuer::new(ca_params, ca_chave);
        let folha_chave = rcgen::KeyPair::generate().unwrap();
        let folha =
            rcgen::CertificateParams::new(vec!["localhost".to_owned(), "127.0.0.1".to_owned()])
                .unwrap()
                .signed_by(&folha_chave, &emissor)
                .unwrap();
        (
            vec![folha.der().clone()],
            folha_chave.serialize_der(),
            ca.pem(),
        )
    })
}

/// O PEM da CA do servidor (para o `TlsMode::CustomCa`).
pub fn certificado_pem() -> String {
    certificado().2.clone()
}

fn config_tls() -> Arc<ServerConfig> {
    let (cadeia, chave, _) = certificado();
    let provider = Arc::new(ring::default_provider());
    Arc::new(
        ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&version::TLS13, &version::TLS12])
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                cadeia.clone(),
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(chave.clone())),
            )
            .unwrap(),
    )
}

trait Conexao: Read + Write + Send {}
impl<T: Read + Write + Send> Conexao for T {}

impl S3d {
    pub fn novo() -> S3d {
        S3d::com_opcoes(Opcoes::default())
    }

    pub fn com_opcoes(opcoes: Opcoes) -> S3d {
        let esquema = if opcoes.tls { "https" } else { "http" };
        let tls = opcoes.tls.then(config_tls);
        let compartilhado = Arc::new(Compartilhado {
            estado: Mutex::new(Estado::default()),
            opcoes,
            falhas: AtomicUsize::new(0),
            esquema,
        });
        let escuta = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let endereco = escuta.local_addr().unwrap();
        let parar = Arc::new(AtomicBool::new(false));
        {
            let compartilhado = compartilhado.clone();
            let parar = parar.clone();
            thread::spawn(move || {
                for conexao in escuta.incoming() {
                    if parar.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(conexao) = conexao else { continue };
                    let compartilhado = compartilhado.clone();
                    let tls = tls.clone();
                    thread::spawn(move || atender_conexao(&compartilhado, conexao, tls));
                }
            });
        }
        S3d {
            endereco,
            compartilhado,
            parar,
        }
    }

    /// `http://127.0.0.1:porta` (ou `https`).
    pub fn endpoint(&self) -> String {
        format!("{}://{}", self.compartilhado.esquema, self.endereco)
    }

    /// A configuração de um cliente para este servidor, com as repetições rápidas.
    pub fn config(&self, bucket: Option<&str>) -> S3Config {
        S3Config {
            bucket: bucket.map(str::to_owned),
            url_style: S3UrlStyle::Path,
            region: self.compartilhado.opcoes.regiao.to_owned(),
            tls: if self.compartilhado.opcoes.tls {
                TlsMode::NoVerify
            } else {
                TlsMode::Verify
            },
            retry_base: Duration::from_millis(10),
            ..S3Config::new(self.endpoint(), ACESSO, SEGREDO)
        }
    }

    pub fn criar_bucket(&self, nome: &str) {
        self.estado().buckets.entry(nome.to_owned()).or_default();
    }

    /// Grava um objeto direto no estado (sem passar pelo HTTP).
    pub fn colocar(&self, bucket: &str, chave: &str, dados: &[u8]) {
        let objeto = novo_objeto(dados.to_vec(), "application/octet-stream", BTreeMap::new());
        self.estado()
            .buckets
            .entry(bucket.to_owned())
            .or_default()
            .insert(chave.to_owned(), objeto);
    }

    pub fn objeto(&self, bucket: &str, chave: &str) -> Option<Vec<u8>> {
        self.estado()
            .buckets
            .get(bucket)?
            .get(chave)
            .map(|o| o.dados.clone())
    }

    pub fn chaves(&self, bucket: &str) -> Vec<String> {
        self.estado()
            .buckets
            .get(bucket)
            .map(|b| b.keys().cloned().collect())
            .unwrap_or_default()
    }

    pub fn classe(&self, bucket: &str, chave: &str) -> Option<String> {
        self.estado()
            .buckets
            .get(bucket)?
            .get(chave)
            .map(|o| o.classe.clone())
    }

    pub fn metadado(&self, bucket: &str, chave: &str, nome: &str) -> Option<String> {
        self.estado()
            .buckets
            .get(bucket)?
            .get(chave)?
            .meta
            .get(nome)
            .cloned()
    }

    pub fn tipo(&self, bucket: &str, chave: &str) -> Option<String> {
        self.estado()
            .buckets
            .get(bucket)?
            .get(chave)
            .map(|o| o.tipo.clone())
    }

    /// Quantos envios em partes estão abertos.
    pub fn envios_abertos(&self) -> usize {
        self.estado().envios.len()
    }

    /// Os pedidos recebidos (`MÉTODO /caminho?consulta`, sem os `X-Amz-*`).
    pub fn pedidos(&self) -> Vec<String> {
        self.estado().pedidos.clone()
    }

    pub fn limpar_pedidos(&self) {
        self.estado().pedidos.clear();
    }

    /// Quantos pedidos casam com o método e o trecho.
    pub fn contar(&self, metodo: &str, trecho: &str) -> usize {
        self.pedidos()
            .iter()
            .filter(|p| p.starts_with(&format!("{metodo} ")) && p.contains(trecho))
            .count()
    }

    /// Os `n` próximos pedidos (de qualquer tipo) recebem `503 SlowDown`.
    pub fn falhar_proximos(&self, n: usize) {
        self.compartilhado.falhas.store(n, Ordering::SeqCst);
    }

    fn estado(&self) -> std::sync::MutexGuard<'_, Estado> {
        self.compartilhado.estado.lock().unwrap()
    }
}

impl Drop for S3d {
    fn drop(&mut self) {
        self.parar.store(true, Ordering::SeqCst);
        // Acorda o `accept`.
        let _ = TcpStream::connect(self.endereco);
    }
}

fn novo_objeto(dados: Vec<u8>, tipo: &str, meta: BTreeMap<String, String>) -> Objeto {
    Objeto {
        etag: etag_de(&dados),
        dados,
        tipo: tipo.to_owned(),
        meta,
        classe: "STANDARD".to_owned(),
        modificado: SystemTime::now(),
    }
}

// ---- HTTP ----

fn atender_conexao(c: &Compartilhado, tcp: TcpStream, tls: Option<Arc<ServerConfig>>) {
    let _ = tcp.set_read_timeout(Some(Duration::from_secs(30)));
    let _ = tcp.set_nodelay(true);
    let fluxo: Box<dyn Conexao> = match tls {
        Some(config) => Box::new(StreamOwned::new(
            ServerConnection::new(config).unwrap(),
            tcp,
        )),
        None => Box::new(tcp),
    };
    let mut leitor = BufReader::new(fluxo);
    while let Some(pedido) = ler_pedido(&mut leitor, c.esquema) {
        let resposta = atender(c, &pedido);
        if escrever(leitor.get_mut(), &pedido, resposta).is_err() {
            break;
        }
    }
}

fn ler_pedido(leitor: &mut BufReader<Box<dyn Conexao>>, esquema: &'static str) -> Option<Pedido> {
    let mut linha = String::new();
    if leitor.read_line(&mut linha).ok()? == 0 {
        return None;
    }
    let mut partes = linha.split_whitespace();
    let metodo = partes.next()?.to_owned();
    let alvo = partes.next()?.to_owned();
    let mut cabecalhos = HashMap::new();
    loop {
        let mut linha = String::new();
        leitor.read_line(&mut linha).ok()?;
        let linha = linha.trim_end();
        if linha.is_empty() {
            break;
        }
        let (nome, valor) = linha.split_once(':')?;
        cabecalhos.insert(nome.trim().to_ascii_lowercase(), valor.trim().to_owned());
    }
    if cabecalhos
        .get("expect")
        .is_some_and(|v| v.eq_ignore_ascii_case("100-continue"))
    {
        leitor
            .get_mut()
            .write_all(b"HTTP/1.1 100 Continue\r\n\r\n")
            .ok()?;
    }
    let tamanho: usize = cabecalhos
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut corpo = vec![0; tamanho];
    leitor.read_exact(&mut corpo).ok()?;
    let (caminho, consulta_bruta) = alvo.split_once('?').unwrap_or((&alvo, ""));
    let consulta = consulta_bruta
        .split('&')
        .filter(|par| !par.is_empty())
        .map(|par| {
            let (k, v) = par.split_once('=').unwrap_or((par, ""));
            (
                percent_decode_str(k).decode_utf8_lossy().into_owned(),
                percent_decode_str(v).decode_utf8_lossy().into_owned(),
            )
        })
        .collect();
    Some(Pedido {
        metodo,
        caminho: caminho.to_owned(),
        consulta,
        cabecalhos,
        corpo,
        esquema,
    })
}

fn razao(status: u16) -> &'static str {
    match status {
        100 => "Continue",
        200 => "OK",
        204 => "No Content",
        206 => "Partial Content",
        301 => "Moved Permanently",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        416 => "Range Not Satisfiable",
        501 => "Not Implemented",
        503 => "Slow Down",
        _ => "Error",
    }
}

fn escrever(fluxo: &mut dyn Conexao, pedido: &Pedido, resposta: Resposta) -> std::io::Result<()> {
    let mut cabeca = format!(
        "HTTP/1.1 {} {}\r\nServer: AmazonS3\r\nx-amz-request-id: teste\r\n",
        resposta.status,
        razao(resposta.status)
    );
    let mut tem_tamanho = false;
    for (nome, valor) in &resposta.cabecalhos {
        tem_tamanho |= nome.eq_ignore_ascii_case("content-length");
        cabeca.push_str(&format!("{nome}: {valor}\r\n"));
    }
    if !tem_tamanho && resposta.status != 204 {
        cabeca.push_str(&format!("Content-Length: {}\r\n", resposta.corpo.len()));
    }
    cabeca.push_str("\r\n");
    fluxo.write_all(cabeca.as_bytes())?;
    if pedido.metodo != "HEAD" {
        fluxo.write_all(&resposta.corpo)?;
    }
    fluxo.flush()
}

// ---- autenticação ----

fn atender(c: &Compartilhado, pedido: &Pedido) -> Resposta {
    let consulta_visivel: Vec<String> = pedido
        .consulta
        .iter()
        .filter(|(k, _)| !k.starts_with("X-Amz-"))
        .map(|(k, v)| {
            if v.is_empty() {
                k.clone()
            } else {
                format!("{k}={v}")
            }
        })
        .collect();
    let mut registro = format!(
        "{} {}",
        pedido.metodo,
        percent_decode_str(&pedido.caminho).decode_utf8_lossy()
    );
    if !consulta_visivel.is_empty() {
        registro.push('?');
        registro.push_str(&consulta_visivel.join("&"));
    }
    c.estado.lock().unwrap().pedidos.push(registro);

    if c.falhas
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
        .is_ok()
    {
        return Resposta::erro(503, "SlowDown", "Please reduce your request rate.");
    }
    autenticar(c, pedido)
        .and_then(|()| rotear(c, pedido))
        .unwrap_or_else(|erro| erro)
}

fn autenticar(c: &Compartilhado, p: &Pedido) -> Result<(), Resposta> {
    let negado = |codigo: &str, mensagem: &str| Resposta::erro(403, codigo, mensagem);
    let credencial = p
        .q("X-Amz-Credential")
        .ok_or_else(|| negado("AccessDenied", "Falta a assinatura (X-Amz-Credential)."))?;
    let mut campos = credencial.split('/');
    let acesso = campos.next().unwrap_or_default();
    let _data = campos.next();
    let regiao = campos.next().unwrap_or_default();
    if acesso != ACESSO {
        return Err(negado(
            "InvalidAccessKeyId",
            "The AWS Access Key Id you provided does not exist in our records.",
        ));
    }
    if regiao != c.opcoes.regiao {
        let mut erro = Resposta::erro(
            400,
            "AuthorizationHeaderMalformed",
            &format!(
                "the region '{regiao}' is wrong; expecting '{}'",
                c.opcoes.regiao
            ),
        )
        .com("x-amz-bucket-region", c.opcoes.regiao);
        erro.corpo = String::from_utf8(erro.corpo)
            .unwrap()
            .replace(
                "</Error>",
                &format!("<Region>{}</Region></Error>", c.opcoes.regiao),
            )
            .into_bytes();
        return Err(erro);
    }
    let data_texto = p.q("X-Amz-Date").unwrap_or_default();
    let data = parse_data(data_texto)
        .ok_or_else(|| negado("AccessDenied", "X-Amz-Date não é uma data válida."))?;
    let validade: u64 = p
        .q("X-Amz-Expires")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let agora = jiff::Timestamp::now();
    if agora.as_second() > data.as_second() + i64::try_from(validade).unwrap_or(i64::MAX) {
        return Err(negado("AccessDenied", "Request has expired"));
    }
    let host = p
        .h("host")
        .ok_or_else(|| negado("AccessDenied", "Falta o cabeçalho host."))?;
    let url = Url::parse(&format!("{}://{host}{}", p.esquema, p.caminho))
        .map_err(|_| negado("AccessDenied", "URL inválida."))?;
    let mut consulta: Vec<(&str, &str)> = p
        .consulta
        .iter()
        .filter(|(k, _)| !k.starts_with("X-Amz-"))
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    consulta.sort();
    let assinados: Vec<String> = p
        .q("X-Amz-SignedHeaders")
        .unwrap_or("host")
        .split(';')
        .filter(|nome| *nome != "host")
        .map(str::to_owned)
        .collect();
    let valores: Vec<(&str, &str)> = assinados
        .iter()
        .map(|nome| (nome.as_str(), p.h(nome).unwrap_or_default()))
        .collect();
    let metodo = match p.metodo.as_str() {
        "GET" => Method::Get,
        "HEAD" => Method::Head,
        "PUT" => Method::Put,
        "POST" => Method::Post,
        "DELETE" => Method::Delete,
        _ => {
            return Err(Resposta::erro(
                405,
                "MethodNotAllowed",
                "Método desconhecido.",
            ));
        }
    };
    let token = p.q("X-Amz-Security-Token");
    let esperada = sign(
        &data,
        metodo,
        url,
        ACESSO,
        SEGREDO,
        token,
        regiao,
        validade,
        consulta.iter().copied(),
        valores.iter().copied(),
    );
    let esperada = esperada
        .query_pairs()
        .find(|(k, _)| k == "X-Amz-Signature")
        .map(|(_, v)| v.into_owned());
    if esperada.as_deref() != p.q("X-Amz-Signature") {
        return Err(negado(
            "SignatureDoesNotMatch",
            "The request signature we calculated does not match the signature you provided. Check your key and signing method.",
        ));
    }
    Ok(())
}

/// `20240102T030405Z`.
fn parse_data(texto: &str) -> Option<jiff::Timestamp> {
    if texto.len() != 16 || !texto.is_ascii() {
        return None;
    }
    let n = |de: usize, ate: usize| texto[de..ate].parse::<i16>().ok();
    let data = jiff::civil::DateTime::new(
        n(0, 4)?,
        i8::try_from(n(4, 6)?).ok()?,
        i8::try_from(n(6, 8)?).ok()?,
        i8::try_from(n(9, 11)?).ok()?,
        i8::try_from(n(11, 13)?).ok()?,
        i8::try_from(n(13, 15)?).ok()?,
        0,
    )
    .ok()?;
    data.to_zoned(jiff::tz::TimeZone::UTC)
        .ok()
        .map(|z| z.timestamp())
}

// ---- rotas ----

fn separar(caminho: &str) -> (String, String) {
    let decodificado = percent_decode_str(caminho).decode_utf8_lossy().into_owned();
    let sem = decodificado.trim_start_matches('/');
    let (bucket, chave) = sem.split_once('/').unwrap_or((sem, ""));
    (bucket.to_owned(), chave.to_owned())
}

fn rotear(c: &Compartilhado, p: &Pedido) -> R {
    let (bucket, chave) = separar(&p.caminho);
    let mut estado = c.estado.lock().unwrap();
    if bucket.is_empty() {
        return match p.metodo.as_str() {
            "GET" => Ok(listar_buckets(&estado)),
            _ => Err(Resposta::erro(405, "MethodNotAllowed", "Só o GET na raiz.")),
        };
    }
    if !estado.buckets.contains_key(&bucket) {
        return Err(Resposta::erro(
            404,
            "NoSuchBucket",
            "The specified bucket does not exist",
        ));
    }
    if chave.is_empty() {
        return match p.metodo.as_str() {
            "GET" if p.tem("uploads") => Ok(listar_envios(&estado, &bucket, p)),
            "GET" => Ok(listar_v2(&estado, &bucket, p)),
            "HEAD" => Ok(Resposta::vazia(200)),
            "POST" if p.tem("delete") => apagar_lote(&mut estado, &bucket, p),
            _ => Err(Resposta::erro(
                405,
                "MethodNotAllowed",
                "Operação não suportada.",
            )),
        };
    }
    match p.metodo.as_str() {
        "HEAD" => cabeca_objeto(&estado, &bucket, &chave),
        "GET" if p.tem("uploadId") => listar_partes(&estado, p),
        "GET" => pegar(c, &estado, &bucket, &chave, p),
        "PUT" if p.tem("partNumber") => enviar_parte(&mut estado, &bucket, &chave, p),
        "PUT" if p.h("x-amz-copy-source").is_some() => copiar(&mut estado, &bucket, &chave, p),
        "PUT" => colocar_objeto(&mut estado, &bucket, &chave, p),
        "POST" if p.tem("uploads") => criar_envio(&mut estado, &bucket, &chave, p),
        "POST" if p.tem("uploadId") => completar(&mut estado, &bucket, &chave, p),
        "DELETE" if p.tem("uploadId") => abortar(&mut estado, p),
        "DELETE" => {
            if let Some(objetos) = estado.buckets.get_mut(&bucket) {
                objetos.remove(&chave);
            }
            Ok(Resposta::vazia(204))
        }
        _ => Err(Resposta::erro(
            405,
            "MethodNotAllowed",
            "Operação não suportada.",
        )),
    }
}

fn listar_buckets(estado: &Estado) -> Resposta {
    let mut xml = String::from(
        "<ListAllMyBucketsResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Owner><ID>teste</ID></Owner><Buckets>",
    );
    for nome in estado.buckets.keys() {
        xml.push_str(&format!(
            "<Bucket><Name>{}</Name><CreationDate>{}</CreationDate></Bucket>",
            esc(nome),
            iso(SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000))
        ));
    }
    xml.push_str("</Buckets></ListAllMyBucketsResult>");
    Resposta::xml(xml)
}

fn listar_v2(estado: &Estado, bucket: &str, p: &Pedido) -> Resposta {
    let objetos = &estado.buckets[bucket];
    let prefixo = p.q("prefix").unwrap_or("");
    let delimitador = p.q("delimiter").filter(|d| !d.is_empty());
    let maximo: usize = p
        .q("max-keys")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1000)
        .min(1000);
    let depois = p
        .q("continuation-token")
        .map(|t| t.strip_prefix("tok:").unwrap_or(t))
        .or_else(|| p.q("start-after"));
    let codificar = p.q("encoding-type") == Some("url");
    let nome = |texto: &str| {
        if codificar {
            codifica_url(texto)
        } else {
            esc(texto)
        }
    };

    let mut conteudos: Vec<(&String, &Objeto)> = Vec::new();
    let mut prefixos: Vec<String> = Vec::new();
    let mut ultimo: Option<String> = None;
    let mut truncado = false;
    let mut itens = 0;
    for (chave, objeto) in objetos
        .iter()
        .filter(|(k, _)| k.starts_with(prefixo) && depois.is_none_or(|d| k.as_str() > d))
    {
        let resto = &chave[prefixo.len()..];
        let comum = delimitador.and_then(|d| {
            resto
                .find(d)
                .map(|i| format!("{prefixo}{}", &resto[..i + d.len()]))
        });
        match comum {
            Some(comum) => {
                if prefixos.last() == Some(&comum) {
                    ultimo = Some(chave.clone());
                    continue;
                }
                if itens >= maximo {
                    truncado = true;
                    break;
                }
                prefixos.push(comum);
            }
            None => {
                if itens >= maximo {
                    truncado = true;
                    break;
                }
                conteudos.push((chave, objeto));
            }
        }
        itens += 1;
        ultimo = Some(chave.clone());
    }

    let mut xml = format!(
        "<ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Name>{}</Name><Prefix>{}</Prefix><KeyCount>{itens}</KeyCount><MaxKeys>{maximo}</MaxKeys>",
        esc(bucket),
        nome(prefixo)
    );
    if let Some(d) = delimitador {
        xml.push_str(&format!("<Delimiter>{}</Delimiter>", nome(d)));
    }
    if codificar {
        xml.push_str("<EncodingType>url</EncodingType>");
    }
    xml.push_str(&format!("<IsTruncated>{truncado}</IsTruncated>"));
    if truncado && let Some(ultimo) = &ultimo {
        xml.push_str(&format!(
            "<NextContinuationToken>tok:{}</NextContinuationToken>",
            esc(ultimo)
        ));
    }
    for (chave, objeto) in conteudos {
        xml.push_str(&format!(
            "<Contents><Key>{}</Key><LastModified>{}</LastModified><ETag>&quot;{}&quot;</ETag><Size>{}</Size><StorageClass>{}</StorageClass></Contents>",
            nome(chave),
            iso(objeto.modificado),
            objeto.etag,
            objeto.dados.len(),
            objeto.classe
        ));
    }
    for comum in prefixos {
        xml.push_str(&format!(
            "<CommonPrefixes><Prefix>{}</Prefix></CommonPrefixes>",
            nome(&comum)
        ));
    }
    xml.push_str("</ListBucketResult>");
    Resposta::xml(xml)
}

fn listar_envios(estado: &Estado, bucket: &str, p: &Pedido) -> Resposta {
    let prefixo = p.q("prefix").unwrap_or("");
    let mut xml = format!(
        "<ListMultipartUploadsResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Bucket>{}</Bucket><IsTruncated>false</IsTruncated>",
        esc(bucket)
    );
    for (id, envio) in &estado.envios {
        if envio.bucket == bucket && envio.chave.starts_with(prefixo) {
            xml.push_str(&format!(
                "<Upload><Key>{}</Key><UploadId>{}</UploadId><Initiated>{}</Initiated></Upload>",
                esc(&envio.chave),
                esc(id),
                iso(envio.iniciado)
            ));
        }
    }
    xml.push_str("</ListMultipartUploadsResult>");
    Resposta::xml(xml)
}

fn sem_objeto() -> Resposta {
    Resposta::erro(404, "NoSuchKey", "The specified key does not exist.")
}

fn cabecalhos_do_objeto(resposta: Resposta, objeto: &Objeto) -> Resposta {
    let mut resposta = resposta
        .com("etag", format!("\"{}\"", objeto.etag))
        .com("last-modified", data_http(objeto.modificado))
        .com("content-type", objeto.tipo.clone())
        .com("accept-ranges", "bytes");
    if objeto.classe != "STANDARD" {
        resposta = resposta.com("x-amz-storage-class", objeto.classe.clone());
    }
    for (nome, valor) in &objeto.meta {
        resposta = resposta.com(&format!("x-amz-meta-{nome}"), valor.clone());
    }
    resposta
}

fn cabeca_objeto(estado: &Estado, bucket: &str, chave: &str) -> R {
    let objeto = estado.buckets[bucket].get(chave).ok_or_else(sem_objeto)?;
    Ok(cabecalhos_do_objeto(Resposta::vazia(200), objeto)
        .com("content-length", objeto.dados.len().to_string()))
}

fn pegar(c: &Compartilhado, estado: &Estado, bucket: &str, chave: &str, p: &Pedido) -> R {
    let objeto = estado.buckets[bucket].get(chave).ok_or_else(sem_objeto)?;
    let total = objeto.dados.len();
    let faixa = if c.opcoes.ignorar_range {
        None
    } else {
        p.h("range").and_then(|r| r.strip_prefix("bytes="))
    };
    let Some(faixa) = faixa else {
        let mut resposta = cabecalhos_do_objeto(Resposta::vazia(200), objeto);
        resposta.corpo = objeto.dados.clone();
        return Ok(resposta);
    };
    let (de, ate) = faixa.split_once('-').unwrap_or((faixa, ""));
    let de: usize = de.parse().unwrap_or(0);
    let ate: usize = ate
        .parse::<usize>()
        .map_or(total.saturating_sub(1), |a| a.min(total.saturating_sub(1)));
    if de >= total {
        return Err(Resposta::erro(
            416,
            "InvalidRange",
            "The requested range is not satisfiable",
        ));
    }
    let mut resposta = cabecalhos_do_objeto(Resposta::vazia(206), objeto)
        .com("content-range", format!("bytes {de}-{ate}/{total}"));
    resposta.corpo = objeto.dados[de..=ate].to_vec();
    Ok(resposta)
}

fn meta_do_pedido(p: &Pedido) -> BTreeMap<String, String> {
    p.cabecalhos
        .iter()
        .filter_map(|(k, v)| {
            k.strip_prefix("x-amz-meta-")
                .map(|nome| (nome.to_owned(), v.clone()))
        })
        .collect()
}

fn colocar_objeto(estado: &mut Estado, bucket: &str, chave: &str, p: &Pedido) -> R {
    let mut objeto = novo_objeto(
        p.corpo.clone(),
        p.h("content-type").unwrap_or("binary/octet-stream"),
        meta_do_pedido(p),
    );
    if let Some(classe) = p.h("x-amz-storage-class") {
        objeto.classe = classe.to_owned();
    }
    let etag = format!("\"{}\"", objeto.etag);
    estado
        .buckets
        .get_mut(bucket)
        .unwrap()
        .insert(chave.to_owned(), objeto);
    Ok(Resposta::vazia(200).com("etag", etag))
}

/// O `/bucket/chave` do `x-amz-copy-source`.
fn origem_da_copia(p: &Pedido) -> Result<(String, String), Resposta> {
    let bruto = p.h("x-amz-copy-source").unwrap_or_default();
    let decodificado = percent_decode_str(bruto).decode_utf8_lossy().into_owned();
    let (bucket, chave) = decodificado
        .trim_start_matches('/')
        .split_once('/')
        .ok_or_else(|| Resposta::erro(400, "InvalidArgument", "Copy Source inválido"))?;
    Ok((bucket.to_owned(), chave.to_owned()))
}

fn copiar(estado: &mut Estado, bucket: &str, chave: &str, p: &Pedido) -> R {
    let (b_origem, k_origem) = origem_da_copia(p)?;
    let origem = estado
        .buckets
        .get(&b_origem)
        .and_then(|b| b.get(&k_origem))
        .ok_or_else(sem_objeto)?;
    let substituir = p.h("x-amz-metadata-directive") == Some("REPLACE");
    let mesma = b_origem == bucket && k_origem == chave;
    if mesma && !substituir && p.h("x-amz-storage-class").is_none() {
        return Err(Resposta::erro(
            400,
            "InvalidRequest",
            "This copy request is illegal because it is trying to copy an object to itself without changing the object's metadata, storage class, website redirect location or encryption attributes.",
        ));
    }
    let mut novo = Objeto {
        dados: origem.dados.clone(),
        tipo: if substituir {
            p.h("content-type")
                .unwrap_or("binary/octet-stream")
                .to_owned()
        } else {
            origem.tipo.clone()
        },
        meta: if substituir {
            meta_do_pedido(p)
        } else {
            origem.meta.clone()
        },
        classe: p
            .h("x-amz-storage-class")
            .map_or_else(|| origem.classe.clone(), str::to_owned),
        modificado: SystemTime::now(),
        etag: origem.etag.clone(),
    };
    if mesma {
        novo.modificado = origem.modificado;
    }
    let resposta = Resposta::xml(format!(
        "<CopyObjectResult><ETag>&quot;{}&quot;</ETag><LastModified>{}</LastModified></CopyObjectResult>",
        novo.etag,
        iso(novo.modificado)
    ));
    estado
        .buckets
        .get_mut(bucket)
        .unwrap()
        .insert(chave.to_owned(), novo);
    Ok(resposta)
}

fn apagar_lote(estado: &mut Estado, bucket: &str, p: &Pedido) -> R {
    if p.h("content-md5").is_none() {
        return Err(Resposta::erro(
            400,
            "InvalidRequest",
            "Missing required header for this request: Content-MD5.",
        ));
    }
    let corpo = String::from_utf8_lossy(&p.corpo).into_owned();
    let silencioso = corpo.contains("<Quiet>true</Quiet>");
    let chaves: Vec<String> = corpo
        .split("<Object>")
        .skip(1)
        .filter_map(|trecho| entre(trecho, "<Key>", "</Key>"))
        .map(|k| desfazer_esc(&k))
        .collect();
    if chaves.is_empty() || chaves.len() > 1000 {
        return Err(Resposta::erro(
            400,
            "MalformedXML",
            "O lote tem de 1 a 1000 chaves.",
        ));
    }
    let objetos = estado.buckets.get_mut(bucket).unwrap();
    let mut xml = String::from("<DeleteResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">");
    for chave in &chaves {
        objetos.remove(chave);
        if !silencioso {
            xml.push_str(&format!("<Deleted><Key>{}</Key></Deleted>", esc(chave)));
        }
    }
    xml.push_str("</DeleteResult>");
    Ok(Resposta::xml(xml))
}

fn entre(texto: &str, inicio: &str, fim: &str) -> Option<String> {
    let de = texto.find(inicio)? + inicio.len();
    let ate = texto[de..].find(fim)? + de;
    Some(texto[de..ate].to_owned())
}

// ---- envio em partes ----

fn criar_envio(estado: &mut Estado, bucket: &str, chave: &str, p: &Pedido) -> R {
    estado.proximo_id += 1;
    let id = format!("envio-{}", estado.proximo_id);
    estado.envios.insert(
        id.clone(),
        Envio {
            bucket: bucket.to_owned(),
            chave: chave.to_owned(),
            tipo: p
                .h("content-type")
                .unwrap_or("binary/octet-stream")
                .to_owned(),
            meta: meta_do_pedido(p),
            classe: p.h("x-amz-storage-class").unwrap_or("STANDARD").to_owned(),
            iniciado: SystemTime::now(),
            partes: BTreeMap::new(),
        },
    );
    Ok(Resposta::xml(format!(
        "<InitiateMultipartUploadResult><Bucket>{}</Bucket><Key>{}</Key><UploadId>{id}</UploadId></InitiateMultipartUploadResult>",
        esc(bucket),
        esc(chave)
    )))
}

fn envio_do_pedido<'a>(estado: &'a mut Estado, p: &Pedido) -> Result<&'a mut Envio, Resposta> {
    estado
        .envios
        .get_mut(p.q("uploadId").unwrap_or_default())
        .ok_or_else(|| Resposta::erro(404, "NoSuchUpload", "The specified upload does not exist."))
}

fn enviar_parte(estado: &mut Estado, bucket: &str, chave: &str, p: &Pedido) -> R {
    let numero: u32 = p
        .q("partNumber")
        .and_then(|n| n.parse().ok())
        .filter(|n| (1..=10_000).contains(n))
        .ok_or_else(|| Resposta::erro(400, "InvalidArgument", "partNumber inválido"))?;
    let copiada = if p.h("x-amz-copy-source").is_some() {
        let (b_origem, k_origem) = origem_da_copia(p)?;
        let origem = estado
            .buckets
            .get(&b_origem)
            .and_then(|b| b.get(&k_origem))
            .ok_or_else(sem_objeto)?;
        let faixa = p
            .h("x-amz-copy-source-range")
            .and_then(|r| r.strip_prefix("bytes="))
            .and_then(|r| r.split_once('-'))
            .and_then(|(de, ate)| Some((de.parse::<usize>().ok()?, ate.parse::<usize>().ok()?)));
        Some(match faixa {
            Some((de, ate)) if de <= ate && ate < origem.dados.len() => {
                origem.dados[de..=ate].to_vec()
            }
            Some(_) => {
                return Err(Resposta::erro(
                    416,
                    "InvalidRange",
                    "The x-amz-copy-source-range value is invalid.",
                ));
            }
            None => origem.dados.clone(),
        })
    } else {
        None
    };
    let envio = envio_do_pedido(estado, p)?;
    if envio.bucket != bucket || envio.chave != chave {
        return Err(Resposta::erro(
            404,
            "NoSuchUpload",
            "O envio é de outro objeto.",
        ));
    }
    let por_copia = copiada.is_some();
    let dados = copiada.unwrap_or_else(|| p.corpo.clone());
    let etag = etag_de(&dados);
    envio.partes.insert(numero, (dados, etag.clone()));
    if por_copia {
        Ok(Resposta::xml(format!(
            "<CopyPartResult><ETag>&quot;{etag}&quot;</ETag><LastModified>{}</LastModified></CopyPartResult>",
            iso(SystemTime::now())
        )))
    } else {
        Ok(Resposta::vazia(200).com("etag", format!("\"{etag}\"")))
    }
}

fn listar_partes(estado: &Estado, p: &Pedido) -> R {
    let envio = estado
        .envios
        .get(p.q("uploadId").unwrap_or_default())
        .ok_or_else(|| {
            Resposta::erro(404, "NoSuchUpload", "The specified upload does not exist.")
        })?;
    let marcador: u32 = p
        .q("part-number-marker")
        .and_then(|m| m.parse().ok())
        .unwrap_or(0);
    let maximo: usize = p
        .q("max-parts")
        .and_then(|m| m.parse().ok())
        .unwrap_or(1000)
        .min(1000);
    let restantes: Vec<_> = envio
        .partes
        .iter()
        .filter(|(numero, _)| **numero > marcador)
        .collect();
    let truncado = restantes.len() > maximo;
    let pagina = &restantes[..restantes.len().min(maximo)];
    let mut xml = format!(
        "<ListPartsResult><Bucket>{}</Bucket><Key>{}</Key><UploadId>{}</UploadId><IsTruncated>{truncado}</IsTruncated>",
        esc(&envio.bucket),
        esc(&envio.chave),
        esc(p.q("uploadId").unwrap_or_default())
    );
    if truncado && let Some((ultimo, _)) = pagina.last() {
        xml.push_str(&format!(
            "<NextPartNumberMarker>{ultimo}</NextPartNumberMarker>"
        ));
    }
    for (numero, (dados, etag)) in pagina {
        xml.push_str(&format!(
            "<Part><PartNumber>{numero}</PartNumber><LastModified>{}</LastModified><ETag>&quot;{etag}&quot;</ETag><Size>{}</Size></Part>",
            iso(envio.iniciado),
            dados.len()
        ));
    }
    xml.push_str("</ListPartsResult>");
    Ok(Resposta::xml(xml))
}

fn completar(estado: &mut Estado, bucket: &str, chave: &str, p: &Pedido) -> R {
    let id = p.q("uploadId").unwrap_or_default().to_owned();
    let corpo = String::from_utf8_lossy(&p.corpo).into_owned();
    let mut pedidas: Vec<(u32, String)> = Vec::new();
    for trecho in corpo.split("<Part>").skip(1) {
        let numero = entre(trecho, "<PartNumber>", "</PartNumber>").and_then(|n| n.parse().ok());
        let etag = entre(trecho, "<ETag>", "</ETag>")
            .map(|e| desfazer_esc(&e).trim_matches('"').to_owned());
        match (numero, etag) {
            (Some(numero), Some(etag)) => pedidas.push((numero, etag)),
            _ => return Err(Resposta::erro(400, "MalformedXML", "Parte mal formada.")),
        }
    }
    if pedidas.is_empty() {
        return Err(Resposta::erro(400, "MalformedXML", "Nenhuma parte."));
    }
    if pedidas.windows(2).any(|par| par[0].0 >= par[1].0) {
        return Err(Resposta::erro(
            400,
            "InvalidPartOrder",
            "The list of parts was not in ascending order.",
        ));
    }
    let envio = envio_do_pedido(estado, p)?;
    let mut dados = Vec::new();
    for (indice, (numero, etag)) in pedidas.iter().enumerate() {
        let Some((parte, etag_da_parte)) = envio.partes.get(numero) else {
            return Err(Resposta::erro(
                400,
                "InvalidPart",
                "One or more of the specified parts could not be found.",
            ));
        };
        if etag_da_parte != etag {
            return Err(Resposta::erro(
                400,
                "InvalidPart",
                "One or more of the specified parts could not be found (ETag).",
            ));
        }
        if indice + 1 < pedidas.len() && parte.len() < PARTE_MINIMA {
            return Err(Resposta::erro(
                400,
                "EntityTooSmall",
                "Your proposed upload is smaller than the minimum allowed size",
            ));
        }
        dados.extend_from_slice(parte);
    }
    let envio = estado.envios.remove(&id).unwrap();
    let mut objeto = novo_objeto(dados, &envio.tipo, envio.meta);
    objeto.classe = envio.classe;
    objeto.etag = format!("{}-{}", objeto.etag, pedidas.len());
    let resposta = Resposta::xml(format!(
        "<CompleteMultipartUploadResult><Bucket>{}</Bucket><Key>{}</Key><ETag>&quot;{}&quot;</ETag></CompleteMultipartUploadResult>",
        esc(bucket),
        esc(chave),
        objeto.etag
    ));
    estado
        .buckets
        .get_mut(bucket)
        .unwrap()
        .insert(chave.to_owned(), objeto);
    Ok(resposta)
}

fn abortar(estado: &mut Estado, p: &Pedido) -> R {
    let id = p.q("uploadId").unwrap_or_default();
    if estado.envios.remove(id).is_none() {
        return Err(Resposta::erro(
            404,
            "NoSuchUpload",
            "The specified upload does not exist.",
        ));
    }
    Ok(Resposta::vazia(204))
}
