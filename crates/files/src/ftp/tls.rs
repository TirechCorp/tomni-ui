//! O TLS do FTPS: o `ClientConfig` de uma conexão, a retomada da sessão do controle nas
//! conexões de dados e a verificação do certificado (com a pergunta da impressão).
//!
//! Um [`Tls`] vale para **uma** conexão FTP: o controle e todas as conexões de dados usam o mesmo
//! `ClientConfig` (com o armazenamento de sessões próprio) e o mesmo nome do servidor, e é isso
//! que faz o rustls oferecer a retomada da sessão do controle. O vsftpd (`require_ssl_reuse`)
//! e o proftpd recusam a conexão de dados que não a retoma.

use std::fmt;
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::{ClientSessionMemoryCache, Resumption, WebPkiServerVerifier};
use rustls::crypto::{CryptoProvider, ring};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{
    CertificateError, ClientConfig, ClientConnection, DigitallySignedStruct, Error, RootCertStore,
    SignatureScheme, StreamOwned, version,
};
use sha2::{Digest, Sha256};
use x509_cert::Certificate;
use x509_cert::der::Decode;

/// A versão do TLS que a conexão aceita.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TlsVersion {
    /// A mais nova que o servidor aceitar (1.3, e 1.2 como alternativa); se o servidor não
    /// retomar a sessão no 1.3, a conexão passa ao 1.2 sozinha.
    #[default]
    Auto,
    /// Só o TLS 1.2.
    Only12,
    /// Só o TLS 1.3.
    Only13,
}

/// O que se sabe de um certificado que o sistema não confia, para perguntar a quem usa.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CertificateInfo {
    /// O servidor a que a conexão se destina.
    pub host: String,
    /// A impressão SHA-256 do certificado, em hexadecimal com `:` (`AB:CD:…`).
    pub sha256: String,
    /// O assunto (quem o certificado diz ser).
    pub subject: String,
    /// Quem assinou.
    pub issuer: String,
    /// Início da validade.
    pub not_before: SystemTime,
    /// Fim da validade.
    pub not_after: SystemTime,
    /// Por que a verificação do sistema falhou, em português.
    pub error: String,
}

/// A função que decide se um certificado não confiável vale para esta conexão.
pub(crate) type CertificateCheck = Arc<dyn Fn(&CertificateInfo) -> bool + Send + Sync>;

/// A falha do handshake TLS (o fallback para o TLS 1.2 reconhece por ela).
#[derive(Debug)]
pub(crate) struct HandshakeError(pub String);

impl fmt::Display for HandshakeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for HandshakeError {}

/// `true` no erro do handshake TLS.
pub(crate) fn is_handshake_error(error: &io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|inner| inner.is::<HandshakeError>())
}

fn roots() -> Arc<RootCertStore> {
    static ROOTS: OnceLock<Arc<RootCertStore>> = OnceLock::new();
    ROOTS
        .get_or_init(|| {
            let mut store = RootCertStore::empty();
            store.add_parsable_certificates(rustls_native_certs::load_native_certs().certs);
            Arc::new(store)
        })
        .clone()
}

/// A impressão SHA-256 de um certificado (`AB:CD:…`).
pub(crate) fn fingerprint(der: &[u8]) -> String {
    Sha256::digest(der)
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

fn describe_error(error: &Error) -> String {
    match error {
        Error::InvalidCertificate(CertificateError::UnknownIssuer) => {
            "o certificado não é de uma autoridade conhecida (autoassinado ou cadeia incompleta)"
                .to_owned()
        }
        Error::InvalidCertificate(CertificateError::Expired) => "o certificado venceu".to_owned(),
        Error::InvalidCertificate(CertificateError::NotValidYet) => {
            "o certificado ainda não começou a valer".to_owned()
        }
        Error::InvalidCertificate(
            CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. },
        ) => "o certificado não vale para o endereço deste servidor".to_owned(),
        Error::InvalidCertificate(CertificateError::BadSignature) => {
            "a assinatura do certificado não confere".to_owned()
        }
        Error::InvalidCertificate(other) => format!("certificado inválido ({other:?})"),
        other => other.to_string(),
    }
}

/// Confere o certificado: o do sistema (raízes + nome) e, se falhar, a decisão de quem usa.
struct Verifier {
    webpki: Option<Arc<WebPkiServerVerifier>>,
    provider: Arc<CryptoProvider>,
    host: String,
    check: Option<CertificateCheck>,
    /// A impressão do certificado que valeu na primeira conexão (o controle): as conexões
    /// de dados que não retomam a sessão têm de apresentar o mesmo.
    accepted: Mutex<Option<String>>,
    /// Por que o último certificado foi recusado (a mensagem que o usuário vê).
    failure: Mutex<Option<String>>,
}

impl fmt::Debug for Verifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Verifier")
            .field("host", &self.host)
            .finish()
    }
}

impl Verifier {
    fn info(&self, der: &CertificateDer<'_>, error: String) -> CertificateInfo {
        let (subject, issuer, not_before, not_after) = match Certificate::from_der(der) {
            Ok(cert) => {
                let tbs = &cert.tbs_certificate;
                (
                    tbs.subject.to_string(),
                    tbs.issuer.to_string(),
                    tbs.validity.not_before.to_system_time(),
                    tbs.validity.not_after.to_system_time(),
                )
            }
            Err(_) => (
                "(ilegível)".to_owned(),
                "(ilegível)".to_owned(),
                SystemTime::UNIX_EPOCH,
                SystemTime::UNIX_EPOCH,
            ),
        };
        CertificateInfo {
            host: self.host.clone(),
            sha256: fingerprint(der),
            subject,
            issuer,
            not_before,
            not_after,
            error,
        }
    }

    fn reject(&self, message: String, error: Error) -> Error {
        *self.failure.lock().unwrap() = Some(message);
        error
    }
}

impl ServerCertVerifier for Verifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        let print = fingerprint(end_entity);
        let mut accepted = self.accepted.lock().unwrap();
        if let Some(previous) = accepted.as_ref() {
            return if *previous == print {
                Ok(ServerCertVerified::assertion())
            } else {
                Err(self.reject(
                    "o certificado da conexão de dados é diferente do certificado do controle"
                        .to_owned(),
                    Error::General("certificado diferente".to_owned()),
                ))
            };
        }
        let normal = match &self.webpki {
            Some(webpki) => webpki.verify_server_cert(
                end_entity,
                intermediates,
                server_name,
                ocsp_response,
                now,
            ),
            None => Err(Error::InvalidCertificate(CertificateError::UnknownIssuer)),
        };
        let error = match normal {
            Ok(verified) => {
                *accepted = Some(print);
                return Ok(verified);
            }
            Err(error) => error,
        };
        let message = describe_error(&error);
        match &self.check {
            Some(check) if check(&self.info(end_entity, message.clone())) => {
                *accepted = Some(print);
                Ok(ServerCertVerified::assertion())
            }
            Some(_) => Err(self.reject("você não confiou neste certificado".to_owned(), error)),
            None => Err(self.reject(message, error)),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// O TLS de uma conexão FTP: o controle e os dados dividem isto.
pub(crate) struct Tls {
    config: Arc<ClientConfig>,
    name: ServerName<'static>,
    verifier: Arc<Verifier>,
}

impl Tls {
    pub(crate) fn new(
        host: &str,
        version: TlsVersion,
        check: Option<CertificateCheck>,
    ) -> io::Result<Tls> {
        let provider = Arc::new(ring::default_provider());
        let store = roots();
        let webpki = if store.is_empty() {
            None
        } else {
            WebPkiServerVerifier::builder_with_provider(store, provider.clone())
                .build()
                .ok()
        };
        let verifier = Arc::new(Verifier {
            webpki,
            provider: provider.clone(),
            host: host.to_owned(),
            check,
            accepted: Mutex::new(None),
            failure: Mutex::new(None),
        });
        let versions: &[&'static rustls::SupportedProtocolVersion] = match version {
            TlsVersion::Auto => &[&version::TLS13, &version::TLS12],
            TlsVersion::Only12 => &[&version::TLS12],
            TlsVersion::Only13 => &[&version::TLS13],
        };
        let mut config = ClientConfig::builder_with_provider(provider)
            .with_protocol_versions(versions)
            .map_err(io::Error::other)?
            .dangerous()
            .with_custom_certificate_verifier(verifier.clone())
            .with_no_client_auth();
        config.resumption = Resumption::store(Arc::new(ClientSessionMemoryCache::new(32)));
        let name = ServerName::try_from(host.to_owned()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("endereço inválido para o TLS: {host}"),
            )
        })?;
        Ok(Tls {
            config: Arc::new(config),
            name,
            verifier,
        })
    }

    /// Faz o handshake TLS (como cliente) sobre `tcp`.
    pub(crate) fn connect(&self, mut tcp: TcpStream) -> io::Result<Stream> {
        let mut conn = ClientConnection::new(self.config.clone(), self.name.clone())
            .map_err(|error| io::Error::other(HandshakeError(error.to_string())))?;
        *self.verifier.failure.lock().unwrap() = None;
        while conn.is_handshaking() {
            if let Err(error) = conn.complete_io(&mut tcp) {
                return Err(self.handshake_failure(error));
            }
        }
        Ok(Stream::Tls(Box::new(StreamOwned::new(conn, tcp))))
    }

    fn handshake_failure(&self, error: io::Error) -> io::Error {
        let message = if let Some(reason) = self.verifier.failure.lock().unwrap().take() {
            // Um certificado recusado não é "o servidor não retoma": sem fallback para o 1.2.
            return io::Error::other(format!("certificado não confiável: {reason}"));
        } else {
            match error
                .get_ref()
                .and_then(|inner| inner.downcast_ref::<Error>())
            {
                Some(Error::AlertReceived(alert)) => {
                    format!("o servidor recusou a conexão TLS ({alert:?})")
                }
                Some(Error::PeerIncompatible(_)) => {
                    "o servidor não oferece uma versão ou cifra TLS compatível".to_owned()
                }
                Some(other) => format!("falha no TLS: {other}"),
                None if error.kind() == io::ErrorKind::UnexpectedEof => {
                    "o servidor encerrou a conexão durante o handshake TLS".to_owned()
                }
                None => format!("falha no TLS: {}", tomni_ui_net::error_message(&error)),
            }
        };
        io::Error::other(HandshakeError(message))
    }
}

/// Uma conexão, em claro ou com TLS.
pub(crate) enum Stream {
    Plain(TcpStream),
    Tls(Box<StreamOwned<ClientConnection, TcpStream>>),
}

impl Stream {
    pub(crate) fn tcp(&self) -> &TcpStream {
        match self {
            Stream::Plain(tcp) => tcp,
            Stream::Tls(tls) => &tls.sock,
        }
    }

    pub(crate) fn local_addr(&self) -> io::Result<SocketAddr> {
        self.tcp().local_addr()
    }

    pub(crate) fn set_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        self.tcp().set_read_timeout(timeout)?;
        self.tcp().set_write_timeout(timeout)
    }

    pub(crate) fn is_tls(&self) -> bool {
        matches!(self, Stream::Tls(_))
    }

    /// `true` se esta conexão TLS retomou uma sessão.
    pub(crate) fn resumed(&self) -> bool {
        match self {
            Stream::Tls(tls) => tls.conn.handshake_kind() == Some(rustls::HandshakeKind::Resumed),
            Stream::Plain(_) => false,
        }
    }

    /// `true` se o servidor recusou ou fechou a conexão em até `wait`. Lê para dentro do rustls
    /// (sem perder nada): os dados e os tickets do TLS 1.3 ficam guardados para a leitura
    /// seguinte, e um alerta ou o fim da conexão aparecem aqui.
    pub(crate) fn closed_within(&mut self, wait: Duration) -> bool {
        let Stream::Tls(tls) = self else {
            return false;
        };
        let previous = tls.sock.read_timeout().ok().flatten();
        let deadline = std::time::Instant::now() + wait;
        let closed = loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            // O rustls só aceita mais bytes se couber; o que já veio basta para dizer que vive.
            if left.is_zero() || !tls.conn.wants_read() {
                break false;
            }
            let _ = tls.sock.set_read_timeout(Some(left));
            match tls.conn.read_tls(&mut tls.sock) {
                Ok(0) => break true,
                Ok(_) => match tls.conn.process_new_packets() {
                    Err(_) => break true,
                    Ok(state) if state.peer_has_closed() => break true,
                    // Um registro inteiro (ticket, dados): segue esperando um alerta ou o fim.
                    Ok(_) => {}
                },
                Err(error) => {
                    break matches!(
                        error.kind(),
                        io::ErrorKind::ConnectionReset
                            | io::ErrorKind::ConnectionAborted
                            | io::ErrorKind::BrokenPipe
                    );
                }
            }
        };
        let _ = tls.sock.set_read_timeout(previous);
        closed
    }

    /// `true` se negociou o TLS 1.3.
    pub(crate) fn is_tls13(&self) -> bool {
        match self {
            Stream::Tls(tls) => {
                tls.conn.protocol_version() == Some(rustls::ProtocolVersion::TLSv1_3)
            }
            Stream::Plain(_) => false,
        }
    }

    /// Fecha com educação: `close_notify` no TLS. Com `drain`, lê o que o servidor ainda
    /// manda (no TLS 1.3, os tickets da próxima retomada) até o `close_notify` dele, por até 2 s.
    pub(crate) fn close(mut self, drain: bool) {
        if let Stream::Tls(tls) = &mut self {
            tls.conn.send_close_notify();
            let _ = tls.flush();
        }
        let _ = self.tcp().shutdown(Shutdown::Write);
        if drain && self.is_tls() {
            let _ = self.set_timeout(Some(Duration::from_secs(2)));
            let mut sink = [0u8; 1024];
            while matches!(self.read(&mut sink), Ok(n) if n > 0) {}
        }
        let _ = self.tcp().shutdown(Shutdown::Both);
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Stream::Plain(tcp) => tcp.read(buf),
            // Muitos servidores fecham o TCP sem o `close_notify`: é o fim dos dados.
            Stream::Tls(tls) => match tls.read(buf) {
                Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(0),
                other => other,
            },
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Stream::Plain(tcp) => tcp.write(buf),
            Stream::Tls(tls) => tls.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Stream::Plain(tcp) => tcp.flush(),
            Stream::Tls(tls) => tls.flush(),
        }
    }
}
