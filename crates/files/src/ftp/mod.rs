//! O FTP e o FTPS: um [`Fs`] sobre um cliente próprio e bloqueante (M31).
//!
//! [`FtpBuilder::connect`] abre a conexão de controle (RFC 959, 2228, 2389, 4217), entra e
//! prepara a sessão. Cada listagem e cada transferência abre uma conexão de dados (passiva com
//! `EPSV`/`PASV` ou ativa com `EPRT`/`PORT`); no FTPS, ela **retoma a sessão TLS do
//! controle**, que é o que o vsftpd e o proftpd exigem. O FTP não faz duas transferências na
//! mesma conexão: as [`Transfers`](crate::Transfers) abrem uma por trabalhador
//! ([`FtpBuilder::opener`]).
//!
//! ```no_run
//! use tomni_ui_files::{Fs, ftp::{FtpBuilder, FtpSecurity}};
//!
//! let mut ftp = FtpBuilder::new("ftp.exemplo.com", 0)
//!     .login("ana", "segredo")
//!     .security(FtpSecurity::Explicit)
//!     .check_certificate(|certificado| certificado.sha256 == "AB:CD:…")
//!     .connect()?;
//! let casa = ftp.home()?;
//! for entrada in ftp.list(&casa)? {
//!     println!("{}", entrada.name);
//! }
//! # Ok::<(), std::io::Error>(())
//! ```

mod control;
mod data;
mod encoding;
mod list;
mod tls;

use std::collections::HashSet;
use std::io;
use std::net::IpAddr;
use std::ops::RangeInclusive;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use tomni_ui_net::{CONNECT_TIMEOUT, Proxy};

use crate::entry::{Caps, Entry, EntryKind};
use crate::fs::{
    Download, Fs, Upload, connection_limit, describe, is_connection_error, unsupported,
};
use crate::path;
use crate::remote::Opener;

use control::{Control, Reply, reply_code, reply_error};
use data::{FtpDownload, FtpUpload};
use encoding::Charset;
use tls::{CertificateCheck, Tls};
pub use tls::{CertificateInfo, TlsVersion};

/// A porta padrão do FTP.
pub const PORTA_FTP: u16 = 21;
/// A porta padrão do FTPS implícito.
pub const PORTA_FTPS_IMPLICITO: u16 = 990;

/// Como a conexão é protegida.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FtpSecurity {
    /// FTP simples, sem criptografia.
    None,
    /// FTP sobre TLS explícito (`AUTH TLS`): é recusado se o servidor não aceitar.
    #[default]
    Explicit,
    /// TLS explícito se o servidor aceitar; senão segue em claro.
    ExplicitIfAvailable,
    /// FTPS implícito: o TLS já começa na conexão (porta 990).
    Implicit,
}

/// Quem abre a conexão de dados.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FtpMode {
    /// O cliente conecta ao servidor (`EPSV`, `PASV`).
    #[default]
    Passive,
    /// O servidor conecta ao cliente (`EPRT`, `PORT`).
    Active,
}

/// A codificação dos nomes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FtpEncoding {
    /// UTF-8 se o servidor disser (`UTF8` no `FEAT`); se um nome não for UTF-8, passa a Latin-1.
    #[default]
    Auto,
    /// Sempre UTF-8.
    Utf8,
    /// Sempre Latin-1 (ISO 8859-1).
    Latin1,
}

/// O aviso de uma conexão "TLS explícito, se disponível" cujo servidor não aceitou o TLS e
/// seguiu em claro (`Ftp::take_notice`).
pub const NOTICE_NO_TLS: &str = "O servidor não oferece TLS; a conexão segue sem criptografia.";

/// A segurança padrão de uma conexão nova (o app a oferece nas configurações).
pub const SEGURANCA_FTP_PADRAO: FtpSecurity = FtpSecurity::Explicit;
/// O modo padrão de uma conexão nova.
pub const MODO_FTP_PADRAO: FtpMode = FtpMode::Passive;

/// Quanto se espera cada resposta do servidor.
const TIMEOUT_PADRAO: Duration = Duration::from_secs(30);

/// As opções de uma conexão FTP.
#[derive(Clone)]
pub struct FtpBuilder {
    host: String,
    port: u16,
    user: Option<String>,
    password: Option<String>,
    account: Option<String>,
    security: FtpSecurity,
    mode: FtpMode,
    tls_version: TlsVersion,
    encoding: FtpEncoding,
    proxy: Option<Proxy>,
    active_ports: Option<RangeInclusive<u16>>,
    active_address: Option<IpAddr>,
    timezone_offset_min: i32,
    check: Option<CertificateCheck>,
    timeout: Duration,
}

impl std::fmt::Debug for FtpBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Sem a senha.
        f.debug_struct("FtpBuilder")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("security", &self.security)
            .field("mode", &self.mode)
            .field("proxy", &self.proxy)
            .finish_non_exhaustive()
    }
}

impl FtpBuilder {
    /// Uma conexão a `host:port`. Com a porta 0, vale a padrão da segurança (21, ou 990 no
    /// FTPS implícito).
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
            user: None,
            password: None,
            account: None,
            security: SEGURANCA_FTP_PADRAO,
            mode: MODO_FTP_PADRAO,
            tls_version: TlsVersion::Auto,
            encoding: FtpEncoding::Auto,
            proxy: None,
            active_ports: None,
            active_address: None,
            timezone_offset_min: 0,
            check: None,
            timeout: TIMEOUT_PADRAO,
        }
    }

    /// O usuário e a senha. Sem isto, entra como `anonymous`.
    pub fn login(mut self, user: impl Into<String>, password: impl Into<String>) -> Self {
        self.user = Some(user.into());
        self.password = Some(password.into());
        self
    }

    /// A conta (`ACCT`), se o servidor pedir.
    pub fn account(mut self, account: impl Into<String>) -> Self {
        self.account = Some(account.into());
        self
    }

    /// Como a conexão é protegida (padrão: TLS explícito).
    pub fn security(mut self, security: FtpSecurity) -> Self {
        self.security = security;
        self
    }

    /// Passivo (padrão) ou ativo.
    pub fn mode(mut self, mode: FtpMode) -> Self {
        self.mode = mode;
        self
    }

    /// A versão do TLS.
    pub fn tls_version(mut self, version: TlsVersion) -> Self {
        self.tls_version = version;
        self
    }

    /// A codificação dos nomes.
    pub fn encoding(mut self, encoding: FtpEncoding) -> Self {
        self.encoding = encoding;
        self
    }

    /// Conecta por um proxy (só o modo passivo funciona).
    pub fn proxy(mut self, proxy: Proxy) -> Self {
        self.proxy = Some(proxy);
        self
    }

    /// A faixa de portas em que o modo ativo escuta (padrão: qualquer uma).
    pub fn active_ports(mut self, range: RangeInclusive<u16>) -> Self {
        self.active_ports = Some(range);
        self
    }

    /// O endereço (externo) que o modo ativo anuncia no `PORT`/`EPRT`; padrão: o local do
    /// socket de controle.
    pub fn active_address(mut self, address: IpAddr) -> Self {
        self.active_address = Some(address);
        self
    }

    /// Quantos minutos o relógio do servidor está a leste do UTC. Só vale para o `LIST`; o
    /// MLSD e o MDTM vêm em UTC.
    pub fn timezone_offset(mut self, minutes: i32) -> Self {
        self.timezone_offset_min = minutes;
        self
    }

    /// Decide o que fazer com um certificado que o sistema não confia: `true` aceita **esta
    /// conexão**. Sem isto, o certificado não confiável é um erro.
    pub fn check_certificate(
        mut self,
        check: impl Fn(&CertificateInfo) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.check = Some(Arc::new(check));
        self
    }

    /// Quanto esperar cada resposta (padrão: 30 s).
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// A porta, com o padrão aplicado.
    fn port(&self) -> u16 {
        match (self.port, self.security) {
            (0, FtpSecurity::Implicit) => PORTA_FTPS_IMPLICITO,
            (0, _) => PORTA_FTP,
            (port, _) => port,
        }
    }

    /// Conecta, entra e prepara a sessão. Bloqueia até terminar.
    pub fn connect(self) -> io::Result<Ftp> {
        let charset = match self.encoding {
            FtpEncoding::Latin1 => Charset::Latin1,
            _ => Charset::Utf8,
        };
        let mut ftp = Ftp {
            state: State::new(self.tls_version, charset),
            cfg: self,
            ctrl: None,
            tls: None,
        };
        ftp.open()?;
        Ok(ftp)
    }

    /// Um [`Opener`] para as [`Transfers`](crate::Transfers): cada chamada abre **uma conexão
    /// nova** (o FTP não divide a conexão entre transferências).
    pub fn opener(self) -> Opener {
        Arc::new(move || {
            let ftp = self.clone().connect()?;
            Ok(Box::new(ftp) as Box<dyn Fs>)
        })
    }
}

/// Como mudar a data de um arquivo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DateMethod {
    Mfmt,
    Mdtm,
    Utime,
}

/// O que se aprendeu do servidor, e que vale na reconexão.
struct State {
    feat: HashSet<String>,
    charset: Charset,
    /// O endereço para abrir as conexões de dados: o IP do servidor ou, com proxy, o nome.
    server: String,
    server_ip: Option<IpAddr>,
    use_epsv: bool,
    use_eprt: bool,
    dash_a: bool,
    tls_version: TlsVersion,
    fell_back: bool,
    notice: Option<String>,
    date_method: Option<DateMethod>,
    caps: Caps,
    home: String,
    /// `true` se o controle está protegido (e os dados vão com `PROT P`).
    secured: bool,
}

impl State {
    fn new(tls_version: TlsVersion, charset: Charset) -> Self {
        Self {
            feat: HashSet::new(),
            charset,
            server: String::new(),
            server_ip: None,
            use_epsv: true,
            use_eprt: true,
            dash_a: true,
            tls_version,
            fell_back: false,
            notice: None,
            date_method: None,
            caps: Caps {
                permissions: true,
                set_modified: true,
                symlinks: false,
                resume_upload: true,
                owner: true,
            },
            home: "/".to_owned(),
            secured: false,
        }
    }
}

/// O `421` ao abrir a conexão: o servidor não aceita mais conexões deste cliente.
fn limit_error(reply: &Reply) -> io::Error {
    connection_limit(format!("o servidor recusou a conexão: {}", reply.text()))
}

/// Uma conexão FTP. É um [`Fs`].
pub struct Ftp {
    cfg: FtpBuilder,
    ctrl: Option<Control>,
    tls: Option<Tls>,
    state: State,
}

impl Ftp {
    /// `true` se a conexão está protegida por TLS.
    pub fn is_secure(&self) -> bool {
        self.state.secured
    }

    /// Um aviso para mostrar uma vez (por exemplo, o fallback para o TLS 1.2), se houver.
    pub fn take_notice(&mut self) -> Option<String> {
        self.state.notice.take()
    }

    /// `true` se o servidor anunciou `feature` no `FEAT` (em maiúsculas: `MLST`, `MFMT`…).
    pub fn has_feature(&self, feature: &str) -> bool {
        self.state.feat.contains(&feature.to_ascii_uppercase())
    }

    fn io_timeout(&self) -> Option<Duration> {
        Some(self.cfg.timeout)
    }

    /// Abre (ou reabre) a conexão de controle e prepara a sessão.
    fn open(&mut self) -> io::Result<()> {
        self.ctrl = None;
        let cfg = self.cfg.clone();
        let tcp = tomni_ui_net::connect(&cfg.host, cfg.port(), cfg.proxy.as_ref(), CONNECT_TIMEOUT)
            .map_err(|error| {
                io::Error::new(error.kind(), tomni_ui_net::connect_error_message(&error))
            })?;
        let _ = tcp.set_nodelay(true);
        tcp.set_read_timeout(self.io_timeout())?;
        tcp.set_write_timeout(self.io_timeout())?;
        self.state.server_ip = if cfg.proxy.is_some() {
            None
        } else {
            Some(tcp.peer_addr()?.ip())
        };
        self.state.server = match self.state.server_ip {
            Some(ip) => ip.to_string(),
            None => cfg.host.clone(),
        };
        if cfg.security != FtpSecurity::None && self.tls.is_none() {
            self.tls = Some(Tls::new(
                &cfg.host,
                self.state.tls_version,
                cfg.check.clone(),
            )?);
        }
        let charset = self.state.charset;
        let mut secured = false;
        let mut ctrl = if cfg.security == FtpSecurity::Implicit {
            secured = true;
            Control::new(self.tls()?.connect(tcp)?, charset)
        } else {
            Control::new(tls::Stream::Plain(tcp), charset)
        };

        let greeting = loop {
            let reply = ctrl.read_reply()?;
            if reply.code != 120 {
                break reply;
            }
        };
        if greeting.code == 421 {
            return Err(limit_error(&greeting));
        }
        if greeting.code != 220 {
            return Err(reply_error(&greeting, "conectar a", ""));
        }

        if matches!(
            cfg.security,
            FtpSecurity::Explicit | FtpSecurity::ExplicitIfAvailable
        ) {
            let mut accepted = false;
            for method in ["AUTH TLS", "AUTH SSL"] {
                let reply = ctrl.command(method)?;
                if reply.code == 234 || reply.code == 334 {
                    accepted = true;
                    break;
                }
                if reply.code == 421 {
                    return Err(reply_error(&reply, "conectar a", ""));
                }
            }
            if accepted {
                let Control {
                    stream, charset, ..
                } = ctrl;
                let tcp = match stream {
                    tls::Stream::Plain(tcp) => tcp,
                    tls::Stream::Tls(_) => unreachable!("o controle ainda está em claro"),
                };
                ctrl = Control::new(self.tls()?.connect(tcp)?, charset);
                secured = true;
            } else if cfg.security == FtpSecurity::ExplicitIfAvailable {
                self.state.notice = Some(NOTICE_NO_TLS.to_owned());
            } else if cfg.security == FtpSecurity::Explicit {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "o servidor não aceita FTPS (recusou o AUTH TLS); escolha o FTP sem criptografia ou o TLS \"se disponível\"",
                ));
            }
        }
        self.state.secured = secured;

        self.login(&mut ctrl)?;
        self.prepare(&mut ctrl)?;
        self.ctrl = Some(ctrl);
        Ok(())
    }

    fn tls(&self) -> io::Result<&Tls> {
        self.tls
            .as_ref()
            .ok_or_else(|| io::Error::other("o TLS não foi preparado"))
    }

    fn login(&self, ctrl: &mut Control) -> io::Result<()> {
        let user = self.cfg.user.as_deref().unwrap_or("anonymous");
        let password = self.cfg.password.as_deref().unwrap_or("anonymous@");
        let mut reply = ctrl.command(&format!("USER {user}"))?;
        if reply.code == 331 || reply.code == 332 {
            if reply.code == 331 {
                reply = ctrl.command(&format!("PASS {password}"))?;
            }
            if reply.code == 332 {
                let Some(account) = self.cfg.account.as_deref() else {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "o servidor pediu uma conta (ACCT) e nenhuma foi informada",
                    ));
                };
                reply = ctrl.command(&format!("ACCT {account}"))?;
            }
        }
        if reply.code == 230 || reply.code == 202 {
            return Ok(());
        }
        if reply.code == 421 {
            return Err(limit_error(&reply));
        }
        // Sem o texto do comando: a senha nunca aparece na mensagem.
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("o servidor recusou o login: {}", reply.text()),
        ))
    }

    /// `FEAT`, `OPTS UTF8`, `PBSZ`/`PROT`, `TYPE I` e `PWD`.
    fn prepare(&mut self, ctrl: &mut Control) -> io::Result<()> {
        self.state.feat.clear();
        let reply = ctrl.command("FEAT")?;
        if reply.code == 211 {
            for line in reply.body() {
                let line = line.trim().to_ascii_uppercase();
                if let Some(word) = line.split_whitespace().next() {
                    self.state.feat.insert(word.to_owned());
                }
                self.state.feat.insert(line);
            }
        }
        if self.state.feat.contains("UTF8") && self.cfg.encoding != FtpEncoding::Latin1 {
            let reply = ctrl.command("OPTS UTF8 ON")?;
            if reply.ok() {
                self.state.charset = Charset::Utf8;
                ctrl.charset = Charset::Utf8;
            }
        }
        if self.state.secured {
            let reply = ctrl.command("PBSZ 0")?;
            if !reply.ok() {
                return Err(reply_error(&reply, "proteger os dados em", ""));
            }
            let reply = ctrl.command("PROT P")?;
            if !reply.ok() {
                return Err(reply_error(&reply, "proteger os dados em", ""));
            }
        }
        let reply = ctrl.command("TYPE I")?;
        if !reply.ok() {
            return Err(reply_error(&reply, "usar o modo binário em", ""));
        }
        let reply = ctrl.command("PWD")?;
        if reply.code == 257 {
            self.state.home = parse_pwd(&reply.text()).unwrap_or_else(|| "/".to_owned());
        }
        // Os caminhos são absolutos: o servidor que lista os comandos do SITE diz se faz links.
        if let Ok(help) = ctrl.command("SITE HELP")
            && help.code == 214
        {
            let text = help.lines.join(" ").to_ascii_uppercase();
            self.state.caps.symlinks = text.contains("SYMLINK");
        }
        Ok(())
    }

    /// Manda um comando pela conexão de controle e lê a resposta. Um erro de E/S ou o `421`
    /// derrubam a conexão (a próxima operação reabre).
    fn cmd(&mut self, line: &str) -> io::Result<Reply> {
        let ctrl = self
            .ctrl
            .as_mut()
            .ok_or_else(|| io::Error::from(io::ErrorKind::ConnectionReset))?;
        match ctrl.command(line) {
            Ok(reply) if reply.code == 421 => {
                self.ctrl = None;
                Err(reply_error(&reply, "falar com", ""))
            }
            Ok(reply) => Ok(reply),
            Err(error) => {
                self.ctrl = None;
                Err(error)
            }
        }
    }

    /// Lê a próxima resposta da conexão de controle (a final de uma transferência).
    fn read_reply(&mut self) -> io::Result<Reply> {
        let ctrl = self
            .ctrl
            .as_mut()
            .ok_or_else(|| io::Error::from(io::ErrorKind::ConnectionReset))?;
        match ctrl.read_reply() {
            Ok(reply) => Ok(reply),
            Err(error) => {
                self.ctrl = None;
                Err(error)
            }
        }
    }

    /// Roda `f` com a conexão aberta; se ela caiu, reabre e repete uma vez.
    fn run<T>(&mut self, mut f: impl FnMut(&mut Ftp) -> io::Result<T>) -> io::Result<T> {
        if self.ctrl.is_none() {
            self.open()?;
        }
        match f(self) {
            Err(error) if is_connection_error(&error) => {
                self.ctrl = None;
                self.open()?;
                f(self)
            }
            other => other,
        }
    }

    /// Lê os nomes de um texto de listagem. No modo automático, bytes que não são UTF-8
    /// mudam a conexão para Latin-1 (e ela lembra).
    fn decode_names(&mut self, bytes: &[u8]) -> String {
        if self.cfg.encoding == FtpEncoding::Auto
            && self.state.charset == Charset::Utf8
            && std::str::from_utf8(bytes).is_err()
        {
            self.state.charset = Charset::Latin1;
            if let Some(ctrl) = self.ctrl.as_mut() {
                ctrl.charset = Charset::Latin1;
            }
        }
        self.state.charset.decode(bytes)
    }

    fn now(&self) -> SystemTime {
        SystemTime::now()
    }

    fn list_once(&mut self, dir: &str) -> io::Result<Vec<Entry>> {
        let mlsd = self.has_feature("MLST");
        loop {
            let command = if mlsd {
                format!("MLSD {dir}")
            } else if self.state.dash_a {
                format!("LIST -a {dir}")
            } else {
                format!("LIST {dir}")
            };
            let dash_a = !mlsd && self.state.dash_a;
            let bytes = match self.fetch(&command, "listar", dir) {
                Err(error)
                    if dash_a && matches!(reply_code(&error), Some(500 | 501 | 502 | 504)) =>
                {
                    self.state.dash_a = false;
                    continue;
                }
                other => other?,
            };
            let text = self.decode_names(&bytes);
            let entries = if mlsd {
                list::parse_mlsd(&text)
            } else {
                list::parse_list(&text, self.now(), self.cfg.timezone_offset_min)
            };
            if dash_a && entries.iter().any(|entry| entry.name == "-a") {
                // O servidor tratou o `-a` como um caminho.
                self.state.dash_a = false;
                continue;
            }
            return Ok(entries);
        }
    }

    fn exists(&mut self, target: &str) -> bool {
        self.stat_once(target, false).is_ok()
    }

    fn stat_once(&mut self, target: &str, follow: bool) -> io::Result<Entry> {
        let target = path::normalize(target);
        if target == "/" {
            let mut root = Entry::new("/", EntryKind::Dir);
            root.permissions = None;
            return Ok(root);
        }
        let name = path::file_name(&target).to_owned();
        let mut entry = None;
        if self.has_feature("MLST") {
            let reply = self.cmd(&format!("MLST {target}"))?;
            if reply.code == 250 {
                entry = reply
                    .body()
                    .iter()
                    .find_map(|line| list::parse_mlsd_line(line))
                    .map(|mut entry| {
                        entry.name = name.clone();
                        entry
                    });
            } else if reply.code == 550 {
                return Err(describe(io::ErrorKind::NotFound.into(), "ler", &target));
            } else if reply.code >= 400 && !matches!(reply.code, 500 | 501 | 502 | 504) {
                return Err(reply_error(&reply, "ler", &target));
            }
        }
        if entry.is_none() {
            let reply = self.cmd(&format!("SIZE {target}"))?;
            if reply.code == 213 {
                let mut file = Entry::new(name.clone(), EntryKind::File);
                file.size = reply.text().trim().parse().unwrap_or(0);
                let mdtm = self.cmd(&format!("MDTM {target}"))?;
                if mdtm.code == 213 {
                    file.modified = list::parse_timestamp(&mdtm.text());
                }
                entry = Some(file);
            } else {
                // Uma pasta (ou um servidor sem SIZE): procura o nome na pasta de cima.
                let found = self
                    .list_once(&path::parent(&target))?
                    .into_iter()
                    .find(|candidate| candidate.name == name);
                entry = found;
            }
        }
        let mut entry =
            entry.ok_or_else(|| describe(io::ErrorKind::NotFound.into(), "ler", &target))?;
        if follow && entry.kind == EntryKind::Symlink {
            entry = self.resolve_link(entry, &target)?;
        }
        Ok(entry)
    }

    /// O que um link é: arquivo (se tem tamanho) ou pasta (se dá para entrar nela).
    fn resolve_link(&mut self, mut link: Entry, target: &str) -> io::Result<Entry> {
        let reply = self.cmd(&format!("SIZE {target}"))?;
        if reply.code == 213 {
            link.kind = EntryKind::File;
            link.size = reply.text().trim().parse().unwrap_or(0);
        } else if self.cmd(&format!("CWD {target}"))?.ok() {
            link.kind = EntryKind::Dir;
        } else {
            return Err(describe(io::ErrorKind::NotFound.into(), "ler", target));
        }
        Ok(link)
    }

    /// Troca um "sem permissão" de um arquivo que não existe por "não encontrado" (o vsftpd diz
    /// `550 Failed to open file` nos dois casos).
    fn refine_missing(&mut self, error: io::Error, target: &str) -> io::Error {
        if error.kind() == io::ErrorKind::PermissionDenied
            && self.ctrl.is_some()
            && !self.exists(target)
        {
            return describe(io::ErrorKind::NotFound.into(), "ler", target);
        }
        error
    }

    fn simple(&mut self, line: &str, action: &str, target: &str) -> io::Result<Reply> {
        let reply = self.cmd(line)?;
        if reply.ok() {
            Ok(reply)
        } else {
            Err(reply_error(&reply, action, target))
        }
    }
}

impl Drop for Ftp {
    fn drop(&mut self) {
        if let Some(mut ctrl) = self.ctrl.take() {
            let _ = ctrl.stream.set_timeout(Some(Duration::from_secs(1)));
            let _ = ctrl.send("QUIT");
            ctrl.stream.close(false);
        }
    }
}

/// O diretório do `257 "/caminho" é o diretório atual` (aspas dobradas viram uma).
fn parse_pwd(text: &str) -> Option<String> {
    let rest = text
        .strip_prefix('"')
        .or_else(|| text.split_once('"').map(|(_, rest)| rest))?;
    let mut out = String::new();
    let mut chars = rest.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' {
            if chars.peek() == Some(&'"') {
                chars.next();
                out.push('"');
            } else {
                return Some(out);
            }
        } else {
            out.push(c);
        }
    }
    None
}

impl Fs for Ftp {
    fn caps(&self) -> Caps {
        self.state.caps
    }

    fn home(&mut self) -> io::Result<String> {
        self.run(|ftp| Ok(ftp.state.home.clone()))
    }

    fn list(&mut self, dir: &str) -> io::Result<Vec<Entry>> {
        self.run(|ftp| ftp.list_once(dir))
    }

    fn stat(&mut self, target: &str) -> io::Result<Entry> {
        self.run(|ftp| ftp.stat_once(target, true))
    }

    fn lstat(&mut self, target: &str) -> io::Result<Entry> {
        self.run(|ftp| ftp.stat_once(target, false))
    }

    fn open_read(&mut self, target: &str, offset: u64) -> io::Result<Box<dyn Download + '_>> {
        let command = format!("RETR {target}");
        let stream = self
            .run(|ftp| ftp.open_stream(&command, offset, "ler", target))
            .map_err(|error| self.refine_missing(error, target))?;
        Ok(Box::new(FtpDownload::new(self, stream)))
    }

    fn open_write(&mut self, target: &str, offset: u64) -> io::Result<Box<dyn Upload + '_>> {
        // Sem `REST STREAM`, o envio retomado vai por `APPE`, que já anexa no fim.
        let (command, offset) = if offset > 0 && !self.has_feature("REST STREAM") {
            (format!("APPE {target}"), 0)
        } else {
            (format!("STOR {target}"), offset)
        };
        let stream = self.run(|ftp| ftp.open_stream(&command, offset, "gravar em", target))?;
        Ok(Box::new(FtpUpload::new(self, stream)))
    }

    fn mkdir(&mut self, target: &str) -> io::Result<()> {
        self.run(|ftp| {
            let reply = ftp.cmd(&format!("MKD {target}"))?;
            if reply.ok() {
                return Ok(());
            }
            if reply.code == 550 && ftp.exists(target) {
                return Err(describe(
                    io::ErrorKind::AlreadyExists.into(),
                    "criar",
                    target,
                ));
            }
            Err(reply_error(&reply, "criar", target))
        })
    }

    fn remove_file(&mut self, target: &str) -> io::Result<()> {
        self.run(|ftp| ftp.simple(&format!("DELE {target}"), "apagar", target))
            .map(drop)
            .map_err(|error| self.refine_missing(error, target))
    }

    fn remove_dir(&mut self, target: &str) -> io::Result<()> {
        self.run(|ftp| {
            let reply = ftp.cmd(&format!("RMD {target}"))?;
            if reply.ok() {
                return Ok(());
            }
            if reply.code == 550 {
                if !ftp.exists(target) {
                    return Err(describe(io::ErrorKind::NotFound.into(), "apagar", target));
                }
                if ftp
                    .list_once(target)
                    .is_ok_and(|entries| !entries.is_empty())
                {
                    return Err(describe(
                        io::ErrorKind::DirectoryNotEmpty.into(),
                        "apagar",
                        target,
                    ));
                }
            }
            Err(reply_error(&reply, "apagar", target))
        })
    }

    fn rename(&mut self, from: &str, to: &str, overwrite: bool) -> io::Result<()> {
        self.run(|ftp| {
            if !overwrite && ftp.exists(to) {
                return Err(describe(
                    io::ErrorKind::AlreadyExists.into(),
                    "renomear",
                    to,
                ));
            }
            let reply = ftp.cmd(&format!("RNFR {from}"))?;
            if reply.code != 350 {
                return Err(reply_error(&reply, "renomear", from));
            }
            ftp.simple(&format!("RNTO {to}"), "renomear", to).map(drop)
        })
    }

    fn set_permissions(&mut self, target: &str, mode: u32) -> io::Result<()> {
        self.run(|ftp| {
            let reply = ftp.cmd(&format!("SITE CHMOD {:o} {target}", mode & 0o7777))?;
            if reply.ok() {
                return Ok(());
            }
            if matches!(reply.code, 500 | 501 | 502 | 504) {
                ftp.state.caps.permissions = false;
                return Err(unsupported("mudar as permissões"));
            }
            Err(reply_error(&reply, "mudar as permissões de", target))
        })
    }

    fn set_modified(&mut self, target: &str, time: SystemTime) -> io::Result<()> {
        let stamp = list::format_timestamp(time);
        self.run(|ftp| {
            let methods: Vec<DateMethod> = match ftp.state.date_method {
                Some(method) => vec![method],
                None if ftp.has_feature("MFMT") => {
                    vec![DateMethod::Mfmt, DateMethod::Mdtm, DateMethod::Utime]
                }
                None => vec![DateMethod::Mdtm, DateMethod::Utime],
            };
            let mut last = None;
            for method in methods {
                let line = match method {
                    DateMethod::Mfmt => format!("MFMT {stamp} {target}"),
                    DateMethod::Mdtm => format!("MDTM {stamp} {target}"),
                    DateMethod::Utime => format!("SITE UTIME {stamp} {target}"),
                };
                let reply = ftp.cmd(&line)?;
                if reply.ok() {
                    ftp.state.date_method = Some(method);
                    return Ok(());
                }
                if reply.code < 500 {
                    return Err(reply_error(&reply, "mudar a data de", target));
                }
                last = Some(reply);
            }
            if last.is_some() {
                ftp.state.caps.set_modified = false;
            }
            Err(unsupported("mudar a data"))
        })
    }

    fn symlink(&mut self, link_target: &str, link: &str) -> io::Result<()> {
        self.run(|ftp| {
            let reply = ftp.cmd(&format!("SITE SYMLINK {link_target} {link}"))?;
            if reply.ok() {
                return Ok(());
            }
            if reply.code >= 500 {
                ftp.state.caps.symlinks = false;
                return Err(unsupported("criar links simbólicos"));
            }
            Err(reply_error(&reply, "criar o link em", link))
        })
    }

    fn read_link(&mut self, target: &str) -> io::Result<String> {
        let entry = self.lstat(target)?;
        match (entry.kind, entry.link_target) {
            (EntryKind::Symlink, Some(to)) => Ok(to),
            (EntryKind::Symlink, None) => Err(unsupported("ler o destino do link")),
            _ => Err(describe(io::ErrorKind::InvalidInput.into(), "ler", target)),
        }
    }

    fn keepalive(&mut self) -> io::Result<()> {
        self.run(|ftp| ftp.simple("NOOP", "falar com", "").map(drop))
    }

    fn take_notice(&mut self) -> Option<String> {
        Ftp::take_notice(self)
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn pwd_com_aspas_dobradas() {
        assert_eq!(
            parse_pwd("\"/home/ana\" is the current directory").as_deref(),
            Some("/home/ana")
        );
        assert_eq!(parse_pwd("\"/a\"\"b\" ok").as_deref(), Some("/a\"b"));
        assert_eq!(parse_pwd("sem aspas"), None);
    }

    #[test]
    fn porta_padrao_depende_da_seguranca() {
        assert_eq!(FtpBuilder::new("h", 0).port(), 21);
        assert_eq!(
            FtpBuilder::new("h", 0)
                .security(FtpSecurity::Implicit)
                .port(),
            990
        );
        assert_eq!(FtpBuilder::new("h", 2121).port(), 2121);
    }

    #[test]
    fn a_senha_nao_aparece_no_debug() {
        let texto = format!("{:?}", FtpBuilder::new("h", 21).login("ana", "segredo123"));
        assert!(!texto.contains("segredo123"));
    }
}
