//! O `.ovpn`: ler o perfil de cliente, separar os segredos e conferir o que falta.
//!
//! [`Config::from_ovpn`] lê o texto (com os blocos `<ca>`, `<cert>`… ou com os arquivos ao
//! lado do perfil) e devolve a [`Config`] mais os [`Warning`] das opções que o motor ignora.
//! Opções que o app não consegue honrar (`dev tap`, `secret`, `mode server`, `fragment`…) são
//! erro, com a mensagem de quem importou. A [`Config`] não tem `Serialize` e o `Debug` dela não
//! mostra chaves nem senhas: o app guarda o texto com [`Config::without_secrets`] e entrega os
//! segredos do cofre por [`Config::set_pki`].

mod lexer;

use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use zeroize::Zeroizing;

use crate::pki::{
    Pki, PkiError, PrivateKey, Problem, Wrap, parse_certificates, parse_tls_crypt_v2_client,
};
use crate::static_key::StaticKey;
use lexer::{Item, LexError, lex};

/// A porta padrão do OpenVPN (IANA), quando nem o `remote` nem o `port` dizem outra.
pub const DEFAULT_PORT: u16 = 1194;

/// As cifras do canal de dados que o app tem, na ordem de preferência do `IV_CIPHERS`.
pub const SUPPORTED_DATA_CIPHERS: [&str; 3] = ["AES-256-GCM", "AES-128-GCM", "CHACHA20-POLY1305"];

/// A lista de `data-ciphers` que o OpenVPN 2.6 usa quando o `.ovpn` não diz.
pub const DEFAULT_DATA_CIPHERS: [&str; 3] = ["AES-256-GCM", "AES-128-GCM", "CHACHA20-POLY1305"];

/// O transporte de um `remote`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Proto {
    /// UDP.
    Udp,
    /// TCP (cliente).
    Tcp,
}

/// A família de endereços pedida por `proto udp4`, `udp6`, `tcp4-client`…
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// Qualquer uma.
    Any,
    /// Só IPv4.
    V4,
    /// Só IPv6.
    V6,
}

/// Um servidor a tentar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remote {
    /// O nome ou o IP.
    pub host: String,
    /// A porta.
    pub port: u16,
    /// UDP ou TCP.
    pub proto: Proto,
    /// A família de endereços.
    pub family: Family,
}

/// O algoritmo do HMAC do `tls-auth` (e do CBC).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthDigest {
    /// `auth none` (só com AEAD).
    None,
    /// SHA-1.
    Sha1,
    /// SHA-256.
    Sha256,
    /// SHA-384.
    Sha384,
    /// SHA-512.
    Sha512,
}

impl AuthDigest {
    /// O tamanho da saída em bytes.
    pub fn len(self) -> usize {
        match self {
            AuthDigest::None => 0,
            AuthDigest::Sha1 => 20,
            AuthDigest::Sha256 => 32,
            AuthDigest::Sha384 => 48,
            AuthDigest::Sha512 => 64,
        }
    }

    /// Se é `none`.
    pub fn is_empty(self) -> bool {
        self == AuthDigest::None
    }
}

/// As versões de TLS que o app fala.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TlsVersion {
    /// TLS 1.2.
    V1_2,
    /// TLS 1.3.
    V1_3,
}

/// Como o `verify-x509-name` compara.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameMode {
    /// O sujeito inteiro, no formato do OpenVPN (`C=BR, ST=SP, CN=servidor`).
    Subject,
    /// O CN igual ao texto.
    Name,
    /// O CN começa com o texto.
    NamePrefix,
}

/// O `verify-x509-name`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct X509Name {
    /// O texto a comparar.
    pub value: String,
    /// O modo.
    pub mode: NameMode,
}

/// O que o `.ovpn` pede para conferir no certificado do servidor (seção 4.5 do plano).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServerVerify {
    /// `remote-cert-tls server` (ou o `ns-cert-type server`, que vira o mesmo).
    pub remote_cert_tls: bool,
    /// `remote-cert-ku`: os valores aceitos (cada um é uma máscara de bits do `keyUsage`).
    pub remote_cert_ku: Vec<u16>,
    /// `remote-cert-eku`: o nome ou o OID exigido.
    pub remote_cert_eku: Option<String>,
    /// O `verify-x509-name`.
    pub x509_name: Option<X509Name>,
    /// `peer-fingerprint`: as impressões SHA-256 aceitas.
    pub peer_fingerprints: Vec<[u8; 32]>,
    /// `verify-hash`: as impressões aceitas (SHA-1 ou SHA-256, pelo tamanho).
    pub verify_hash: Vec<Vec<u8>>,
}

impl ServerVerify {
    /// Se nada além da cadeia é conferido (o aviso da seção 4.5, item 5).
    pub fn checks_nothing(&self) -> bool {
        !self.remote_cert_tls
            && self.remote_cert_ku.is_empty()
            && self.remote_cert_eku.is_none()
            && self.x509_name.is_none()
            && self.peer_fingerprints.is_empty()
            && self.verify_hash.is_empty()
    }
}

/// O que o `.ovpn` diz da senha de usuário.
#[derive(Clone, Default, PartialEq, Eq)]
pub enum AuthUserPass {
    /// Sem `auth-user-pass`.
    #[default]
    None,
    /// `auth-user-pass` sem arquivo: o app pergunta.
    Ask,
    /// `auth-user-pass arquivo` (ou bloco): usuário e, se houver, senha.
    Credentials {
        /// O usuário (primeira linha).
        user: String,
        /// A senha (segunda linha), se o arquivo tiver.
        password: Option<Zeroizing<String>>,
    },
}

impl fmt::Debug for AuthUserPass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuthUserPass::None => f.write_str("AuthUserPass::None"),
            AuthUserPass::Ask => f.write_str("AuthUserPass::Ask"),
            AuthUserPass::Credentials { user, password } => write!(
                f,
                "AuthUserPass::Credentials {{ user: {user:?}, password: {} }}",
                if password.is_some() {
                    "(oculta)"
                } else {
                    "(nenhuma)"
                }
            ),
        }
    }
}

/// O `static-challenge "texto" echo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticChallenge {
    /// O texto que a pessoa vê.
    pub text: String,
    /// Se a resposta aparece enquanto digita.
    pub echo: bool,
}

/// A ação de um `pull-filter`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterAction {
    /// Aceita a linha e para de olhar os outros filtros.
    Accept,
    /// Descarta a linha.
    Ignore,
    /// Descarta a linha e derruba a conexão.
    Reject,
}

/// Um `pull-filter`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullFilter {
    /// A ação.
    pub action: FilterAction,
    /// O prefixo da linha empurrada.
    pub prefix: String,
}

/// A compressão pedida por `compress` ou `comp-lzo`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compress {
    /// `compress` sem algoritmo, ou `compress stub`: o enquadramento do LZ4 antigo (o byte de
    /// marca `0xFB` na frente, com o primeiro byte da carga levado para o fim), sem comprimir.
    Stub,
    /// `comp-lzo no`: o enquadramento do LZO desligado, só o byte `0xFA` na frente.
    LzoOff,
    /// `compress stub-v2`: o enquadramento novo, sem comprimir.
    StubV2,
    /// `comp-lzo` / `compress lzo`.
    Lzo,
    /// `compress lz4`.
    Lz4,
    /// `compress lz4-v2`.
    Lz4V2,
}

/// O `allow-compression`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllowCompression {
    /// Não aceita pacotes comprimidos.
    No,
    /// Aceita descomprimir, mas não comprime.
    Asym,
    /// Aceita e comprime.
    Yes,
}

/// O `mssfix`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MssFix {
    /// `mssfix 0`: desligado.
    Off,
    /// `mssfix n`.
    Value(u16),
    /// `mssfix` sem número: o motor calcula.
    Auto,
}

/// Um aviso da leitura (a opção segue, ou foi ignorada).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    /// A opção.
    pub option: String,
    /// A linha do `.ovpn` (de 1 em diante).
    pub line: usize,
    /// A mensagem.
    pub message: String,
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "linha {}: {}", self.line, self.message)
    }
}

/// O que deu errado ao ler o `.ovpn`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigErrorKind {
    /// Erro de sintaxe (aspas ou bloco sem fechar).
    Syntax(String),
    /// Uma opção que o app não suporta (a mensagem já diz o que fazer).
    Unsupported(String),
    /// Um arquivo citado e ausente.
    MissingFile {
        /// O nome do arquivo.
        file: String,
        /// A opção que o cita.
        option: String,
    },
    /// Um arquivo fora da pasta do perfil.
    OutsideFolder {
        /// O nome do arquivo.
        file: String,
        /// A opção que o cita.
        option: String,
    },
    /// Um arquivo que não pôde ser lido.
    Unreadable {
        /// O nome do arquivo.
        file: String,
        /// O motivo do sistema.
        reason: String,
    },
    /// Falta um argumento da opção.
    MissingArgument {
        /// A opção.
        option: String,
    },
    /// O valor de uma opção não vale.
    InvalidValue {
        /// A opção.
        option: String,
        /// O motivo.
        reason: String,
    },
    /// Um certificado ou chave não pôde ser lido.
    Pki(PkiError),
}

/// O erro de [`Config::from_ovpn`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    /// A linha do `.ovpn`, quando se sabe.
    pub line: Option<usize>,
    /// O que houve.
    pub kind: ConfigErrorKind,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(line) = self.line {
            write!(f, "linha {line}: ")?;
        }
        match &self.kind {
            ConfigErrorKind::Syntax(m) | ConfigErrorKind::Unsupported(m) => f.write_str(m),
            ConfigErrorKind::MissingFile { file, option } => {
                write!(f, "falta o arquivo {file}, citado em {option}")
            }
            ConfigErrorKind::OutsideFolder { file, option } => write!(
                f,
                "o arquivo {file}, citado em {option}, fica fora da pasta do perfil; ponha-o ao lado do .ovpn ou use o bloco <{option}>"
            ),
            ConfigErrorKind::Unreadable { file, reason } => {
                write!(f, "não foi possível ler o arquivo {file}: {reason}")
            }
            ConfigErrorKind::MissingArgument { option } => {
                write!(f, "falta o argumento da opção {option}")
            }
            ConfigErrorKind::InvalidValue { option, reason } => {
                write!(f, "valor inválido em {option}: {reason}")
            }
            ConfigErrorKind::Pki(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for ConfigError {}

/// O perfil de cliente.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    /// `client` (ou `pull` com `tls-client`): é um perfil de cliente.
    pub is_client: bool,
    /// Os servidores, na ordem do arquivo (o motor embaralha com `remote_random`).
    pub remotes: Vec<Remote>,
    /// `remote-random`.
    pub remote_random: bool,
    /// `lport`: a porta local do UDP.
    pub local_port: Option<u16>,
    /// `connect-retry`: espera entre tentativas, em segundos.
    pub connect_retry: Option<u32>,
    /// `connect-retry` (segundo argumento): o teto da espera, em segundos.
    pub connect_retry_max: Option<u32>,
    /// `connect-timeout`: segundos para o servidor responder a cada tentativa.
    pub connect_timeout: Option<u32>,
    /// `server-poll-timeout`: segundos de espera por servidor na rodada.
    pub server_poll_timeout: Option<u32>,
    /// Os certificados e as chaves.
    pub pki: Pki,
    /// `auth`: o HMAC (do `tls-auth` e do CBC). `None` no campo é o padrão do OpenVPN, SHA-1.
    pub auth_digest: Option<AuthDigest>,
    /// `cipher`.
    pub cipher: Option<String>,
    /// `data-ciphers` (ou `ncp-ciphers`), na ordem do arquivo.
    pub data_ciphers: Option<Vec<String>>,
    /// `data-ciphers-fallback`.
    pub data_ciphers_fallback: Option<String>,
    /// `tls-version-min`.
    pub tls_version_min: Option<TlsVersion>,
    /// `tls-version-max`.
    pub tls_version_max: Option<TlsVersion>,
    /// O que conferir no certificado do servidor.
    pub verify: ServerVerify,
    /// `auth-user-pass`.
    pub auth_user_pass: AuthUserPass,
    /// `static-challenge`.
    pub static_challenge: Option<StaticChallenge>,
    /// `tun-mtu`.
    pub tun_mtu: Option<u16>,
    /// `link-mtu`.
    pub link_mtu: Option<u16>,
    /// `tls-mtu`: o tamanho máximo do pacote de controle.
    pub tls_mtu: Option<u16>,
    /// `mssfix`.
    pub mssfix: Option<MssFix>,
    /// `reneg-sec`, em segundos (0 desliga).
    pub reneg_sec: Option<u32>,
    /// `reneg-bytes`.
    pub reneg_bytes: Option<u64>,
    /// `reneg-pkts`.
    pub reneg_pkts: Option<u64>,
    /// `hand-window`, em segundos.
    pub hand_window: Option<u32>,
    /// `tran-window`, em segundos.
    pub tran_window: Option<u32>,
    /// `replay-window`: pacotes e, se houver, segundos.
    pub replay_window: Option<(u32, Option<u32>)>,
    /// `ping` (ou o primeiro número do `keepalive`), em segundos.
    pub ping: Option<u32>,
    /// `ping-restart` (ou o segundo número do `keepalive`), em segundos.
    pub ping_restart: Option<u32>,
    /// `ping-exit`, em segundos.
    pub ping_exit: Option<u32>,
    /// `inactive`: segundos e, se houver, o mínimo de bytes.
    pub inactive: Option<(u32, Option<u64>)>,
    /// `explicit-exit-notify`: quantas vezes avisar (1 sem argumento).
    pub explicit_exit_notify: Option<u32>,
    /// `route`: os argumentos de cada linha.
    pub routes: Vec<Vec<String>>,
    /// `route-nopull`.
    pub route_nopull: bool,
    /// `pull-filter`, em ordem.
    pub pull_filters: Vec<PullFilter>,
    /// `redirect-gateway` e seus sinalizadores.
    pub redirect_gateway: Option<Vec<String>>,
    /// `dhcp-option`: os argumentos de cada linha.
    pub dhcp_options: Vec<Vec<String>>,
    /// `compress` / `comp-lzo`.
    pub compress: Option<Compress>,
    /// `allow-compression`.
    pub allow_compression: Option<AllowCompression>,
}

/// Quantos dias o texto de um segredo foi lido de um arquivo (para as mensagens).
struct Pending {
    line: usize,
    text: Zeroizing<String>,
}

#[derive(Default)]
struct RawRemote {
    host: String,
    port: Option<u16>,
    proto: Option<(Proto, Family)>,
}

/// Os valores do `.ovpn` que valem depois de ler tudo (a ordem das opções não importa).
#[derive(Default)]
struct Collected {
    ca: Vec<Pending>,
    cert: Option<Pending>,
    key: Option<Pending>,
    extra_certs: Vec<Pending>,
    tls_auth: Option<Pending>,
    tls_auth_direction: Option<u8>,
    tls_crypt: Option<Pending>,
    tls_crypt_v2: Option<Pending>,
    key_direction: Option<u8>,
    global_port: Option<u16>,
    global_rport: Option<u16>,
    global_proto: Option<(Proto, Family)>,
    remotes: Vec<RawRemote>,
    client: bool,
    pull: bool,
    tls_client: bool,
}

struct Reader<'a> {
    dir: Option<&'a Path>,
    config: Config,
    warnings: Vec<Warning>,
    got: Collected,
    blocks: Vec<(String, String, usize)>,
}

impl Config {
    /// Lê o texto de um `.ovpn`.
    ///
    /// `pasta` é onde procurar os arquivos citados (`ca ca.crt`…); sem ela, só valem os blocos.
    /// Um arquivo precisa ficar dentro da pasta: caminho absoluto ou com `..` é recusado, porque
    /// um perfil de fora não deve mandar o app ler (e enviar ao servidor) um arquivo qualquer.
    pub fn from_ovpn(
        texto: &str,
        pasta: Option<&Path>,
    ) -> Result<(Config, Vec<Warning>), ConfigError> {
        let items = lex(texto, 1).map_err(lex_error)?;
        let mut reader = Reader {
            dir: pasta,
            config: Config::default(),
            warnings: Vec::new(),
            got: Collected::default(),
            blocks: Vec::new(),
        };
        // Os blocos primeiro: um `ca [inline]` pode vir antes do `<ca>`.
        for item in &items {
            if let Item::Block {
                tag, body, line, ..
            } = item
            {
                reader.blocks.push((tag.clone(), body.clone(), *line));
            }
        }
        for item in &items {
            reader.item(item)?;
        }
        reader.finish()
    }

    /// Devolve o `.ovpn` sem os segredos: os blocos e as linhas de `ca`, `cert`, `key`,
    /// `extra-certs`, `tls-auth`, `tls-crypt`, `tls-crypt-v2` e `pkcs12`, e o conteúdo de
    /// `auth-user-pass` (que fica como `auth-user-pass` sem arquivo). O resto, inclusive os
    /// comentários, fica byte a byte. Texto que não se lê devolve-se inteiro.
    pub fn without_secrets(texto: &str) -> String {
        let Ok(items) = lex(texto, 1) else {
            return texto.to_string();
        };
        let mut out = String::with_capacity(texto.len());
        let mut cursor = 0usize;
        for item in &items {
            let span = item.span();
            let replacement = secret_replacement(item);
            if let Some(replacement) = replacement {
                out.push_str(&texto[cursor..span.start]);
                out.push_str(replacement);
                cursor = span.end;
            }
        }
        out.push_str(&texto[cursor..]);
        out
    }

    /// Troca o PKI (o app entrega os segredos que vêm do cofre).
    pub fn set_pki(&mut self, pki: Pki) {
        self.pki = pki;
    }

    /// Confere o perfil antes de salvar e devolve o que está errado ou merece aviso.
    pub fn check(&self, agora: SystemTime) -> Vec<Problem> {
        let mut out = Vec::new();
        if !self.is_client {
            out.push(Problem::error(
                "o .ovpn não é de um cliente (falta client, ou pull e tls-client)",
            ));
        }
        if self.remotes.is_empty() {
            out.push(Problem::error(
                "o .ovpn não tem nenhum remote (o endereço do servidor)",
            ));
        }
        out.extend(self.pki.check(agora));
        out.extend(self.check_ciphers());
        if self.verify.checks_nothing() {
            out.push(Problem::warning(
                "o .ovpn não confere o certificado do servidor (falta remote-cert-tls server)",
            ));
        }
        out
    }

    fn check_ciphers(&self) -> Vec<Problem> {
        let mut out = Vec::new();
        if let Some(list) = &self.data_ciphers {
            let ours = |c: &String| {
                SUPPORTED_DATA_CIPHERS
                    .iter()
                    .any(|s| s.eq_ignore_ascii_case(c))
            };
            if !list.iter().any(ours) {
                out.push(Problem::error(format!(
                    "nenhuma cifra em comum com o servidor (o .ovpn oferece {}; o app tem {})",
                    list.join(":"),
                    SUPPORTED_DATA_CIPHERS.join(":")
                )));
            }
        } else if let Some(cipher) = &self.cipher
            && !SUPPORTED_DATA_CIPHERS
                .iter()
                .any(|s| s.eq_ignore_ascii_case(cipher))
        {
            out.push(Problem::warning(format!(
                "a cifra {cipher} é antiga e não é suportada; peça ao administrador para usar AES-GCM"
            )));
        }
        out
    }
}

/// O texto que substitui um item secreto, ou `None` se o item fica.
fn secret_replacement(item: &Item) -> Option<&'static str> {
    const SECRET: [&str; 8] = [
        "ca",
        "cert",
        "key",
        "extra-certs",
        "tls-auth",
        "tls-crypt",
        "tls-crypt-v2",
        "pkcs12",
    ];
    match item {
        Item::Block { tag, .. } if SECRET.contains(&tag.as_str()) => Some(""),
        Item::Block { tag, .. } if tag == "auth-user-pass" => Some("auth-user-pass\n"),
        Item::Directive { name, args, .. } => {
            if SECRET.contains(&name.as_str()) {
                Some("")
            } else if name == "auth-user-pass" && !args.is_empty() {
                Some("auth-user-pass\n")
            } else {
                None
            }
        }
        Item::Block { .. } => None,
    }
}

fn lex_error(e: LexError) -> ConfigError {
    ConfigError {
        line: Some(e.line()),
        kind: ConfigErrorKind::Syntax(e.to_string()),
    }
}

impl Reader<'_> {
    fn err(&self, line: usize, kind: ConfigErrorKind) -> ConfigError {
        ConfigError {
            line: Some(line),
            kind,
        }
    }

    fn unsupported(&self, line: usize, message: &str) -> ConfigError {
        self.err(line, ConfigErrorKind::Unsupported(message.to_string()))
    }

    fn warn(&mut self, option: &str, line: usize, message: impl Into<String>) {
        self.warnings.push(Warning {
            option: option.to_string(),
            line,
            message: message.into(),
        });
    }

    fn missing(&self, option: &str, line: usize) -> ConfigError {
        self.err(
            line,
            ConfigErrorKind::MissingArgument {
                option: option.to_string(),
            },
        )
    }

    fn invalid(&self, option: &str, line: usize, reason: impl Into<String>) -> ConfigError {
        self.err(
            line,
            ConfigErrorKind::InvalidValue {
                option: option.to_string(),
                reason: reason.into(),
            },
        )
    }

    fn number<T: std::str::FromStr>(
        &self,
        option: &str,
        args: &[String],
        index: usize,
        line: usize,
    ) -> Result<T, ConfigError> {
        let raw = args.get(index).ok_or_else(|| self.missing(option, line))?;
        raw.parse::<T>()
            .map_err(|_| self.invalid(option, line, format!("\"{raw}\" não é um número válido")))
    }

    /// Lê um arquivo citado, só dentro da pasta do perfil.
    fn read_file(&self, option: &str, file: &str, line: usize) -> Result<Pending, ConfigError> {
        let missing = || {
            self.err(
                line,
                ConfigErrorKind::MissingFile {
                    file: file.to_string(),
                    option: option.to_string(),
                },
            )
        };
        let Some(dir) = self.dir else {
            return Err(missing());
        };
        let relative = Path::new(file);
        let inside = relative
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
        if !inside {
            return Err(self.err(
                line,
                ConfigErrorKind::OutsideFolder {
                    file: file.to_string(),
                    option: option.to_string(),
                },
            ));
        }
        let path: PathBuf = dir.join(relative);
        match std::fs::read(&path) {
            Ok(bytes) => {
                let text = String::from_utf8(bytes).map_err(|_| {
                    self.err(
                        line,
                        ConfigErrorKind::Unreadable {
                            file: file.to_string(),
                            reason: "o arquivo não é texto UTF-8".into(),
                        },
                    )
                })?;
                Ok(Pending {
                    line,
                    text: Zeroizing::new(text),
                })
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(missing()),
            Err(e) => Err(self.err(
                line,
                ConfigErrorKind::Unreadable {
                    file: file.to_string(),
                    reason: e.to_string(),
                },
            )),
        }
    }

    /// O texto de uma opção que vem de arquivo; `None` quando é o bloco de mesmo nome (`[inline]`).
    fn source(
        &self,
        option: &str,
        args: &[String],
        line: usize,
    ) -> Result<Option<Pending>, ConfigError> {
        let file = args.first().ok_or_else(|| self.missing(option, line))?;
        if file == "[inline]" {
            // O bloco de mesmo nome, esteja antes ou depois da linha, é lido por `block`; aqui só
            // se confere que ele existe.
            return if self.blocks.iter().any(|(tag, _, _)| tag == option) {
                Ok(None)
            } else {
                Err(self.err(
                    line,
                    ConfigErrorKind::MissingFile {
                        file: format!("<{option}>"),
                        option: option.to_string(),
                    },
                ))
            };
        }
        self.read_file(option, file, line).map(Some)
    }

    fn item(&mut self, item: &Item) -> Result<(), ConfigError> {
        match item {
            Item::Block {
                tag, body, line, ..
            } => self.block(tag, body, *line),
            Item::Directive {
                name, args, line, ..
            } => self.directive(name, args, *line),
        }
    }

    fn block(&mut self, tag: &str, body: &str, line: usize) -> Result<(), ConfigError> {
        let pending = || Pending {
            line,
            text: Zeroizing::new(body.to_string()),
        };
        match tag {
            "ca" => self.got.ca.push(pending()),
            "cert" => self.got.cert = Some(pending()),
            "key" => self.got.key = Some(pending()),
            "extra-certs" => self.got.extra_certs.push(pending()),
            "tls-auth" => self.got.tls_auth = Some(pending()),
            "tls-crypt" => self.got.tls_crypt = Some(pending()),
            "tls-crypt-v2" => self.got.tls_crypt_v2 = Some(pending()),
            "pkcs12" => return Err(self.pkcs12_error(line)),
            "secret" => return Err(self.secret_error(line)),
            "auth-user-pass" => {
                let mut lines = body.lines().map(str::trim).filter(|l| !l.is_empty());
                let user = lines.next().unwrap_or_default().to_string();
                let password = lines.next().map(|p| Zeroizing::new(p.to_string()));
                self.config.auth_user_pass = AuthUserPass::Credentials { user, password };
            }
            "peer-fingerprint" => {
                for l in body
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty() && !l.starts_with('#'))
                {
                    self.add_fingerprint(l, line)?;
                }
            }
            "connection" => self.connection(body, line)?,
            "crl-verify" => self.warn(
                tag,
                line,
                "crl-verify ainda não é verificado: certificados revogados não serão recusados",
            ),
            "dh" => {}
            other => self.warn(other, line, format!("bloco desconhecido: <{other}>")),
        }
        Ok(())
    }

    fn pkcs12_error(&self, line: usize) -> ConfigError {
        self.unsupported(
            line,
            "o .ovpn usa pkcs12, que ainda não é suportado; converta para PEM (openssl pkcs12 -in perfil.p12 -nodes) ou use o bloco <cert> e <key>",
        )
    }

    fn secret_error(&self, line: usize) -> ConfigError {
        self.unsupported(
            line,
            "o .ovpn usa chave estática (secret), que não é mais suportada pelo OpenVPN; peça um .ovpn com certificados",
        )
    }

    fn add_fingerprint(&mut self, text: &str, line: usize) -> Result<(), ConfigError> {
        let bytes = parse_hex_fingerprint(text)
            .filter(|b| b.len() == 32)
            .ok_or_else(|| {
                self.invalid(
                    "peer-fingerprint",
                    line,
                    "esperada uma impressão SHA-256 (32 bytes em hexadecimal)",
                )
            })?;
        let mut array = [0u8; 32];
        array.copy_from_slice(&bytes);
        self.config.verify.peer_fingerprints.push(array);
        Ok(())
    }

    fn connection(&mut self, body: &str, line: usize) -> Result<(), ConfigError> {
        let items = lex(body, line + 1).map_err(lex_error)?;
        let mut port = None;
        let mut proto = None;
        let mut remotes = Vec::new();
        for item in &items {
            let Item::Directive {
                name, args, line, ..
            } = item
            else {
                self.warn(
                    "connection",
                    item.line(),
                    "blocos dentro de <connection> são ignorados",
                );
                continue;
            };
            let line = *line;
            match name.as_str() {
                "remote" => remotes.push(self.parse_remote(args, line)?),
                "proto" => proto = Some(self.parse_proto(args, line)?),
                "port" | "rport" => port = Some(self.number::<u16>(name, args, 0, line)?),
                other => self.warn(
                    other,
                    line,
                    format!("a opção {other} dentro de <connection> ainda não é usada"),
                ),
            }
        }
        if remotes.is_empty() {
            self.warn("connection", line, "o bloco <connection> não tem remote");
        }
        for mut remote in remotes {
            if remote.port.is_none() {
                remote.port = port;
            }
            if remote.proto.is_none() {
                remote.proto = proto;
            }
            self.got.remotes.push(remote);
        }
        Ok(())
    }

    fn parse_remote(&self, args: &[String], line: usize) -> Result<RawRemote, ConfigError> {
        let host = args
            .first()
            .ok_or_else(|| self.missing("remote", line))?
            .clone();
        let port = match args.get(1) {
            Some(p) => Some(
                p.parse::<u16>()
                    .map_err(|_| self.invalid("remote", line, format!("porta \"{p}\" inválida")))?,
            ),
            None => None,
        };
        let proto = match args.get(2) {
            Some(p) => Some(self.proto_value(p, line)?),
            None => None,
        };
        Ok(RawRemote { host, port, proto })
    }

    fn parse_proto(&self, args: &[String], line: usize) -> Result<(Proto, Family), ConfigError> {
        let value = args.first().ok_or_else(|| self.missing("proto", line))?;
        self.proto_value(value, line)
    }

    fn proto_value(&self, value: &str, line: usize) -> Result<(Proto, Family), ConfigError> {
        match value {
            "udp" => Ok((Proto::Udp, Family::Any)),
            "udp4" => Ok((Proto::Udp, Family::V4)),
            "udp6" => Ok((Proto::Udp, Family::V6)),
            "tcp" | "tcp-client" => Ok((Proto::Tcp, Family::Any)),
            "tcp4" | "tcp4-client" => Ok((Proto::Tcp, Family::V4)),
            "tcp6" | "tcp6-client" => Ok((Proto::Tcp, Family::V6)),
            "tcp-server" | "tcp4-server" | "tcp6-server" => Err(self.unsupported(
                line,
                "o .ovpn é de um servidor (proto tcp-server); use o .ovpn de um cliente",
            )),
            other => {
                Err(self.invalid("proto", line, format!("protocolo \"{other}\" desconhecido")))
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    fn directive(&mut self, name: &str, args: &[String], line: usize) -> Result<(), ConfigError> {
        match name {
            // Aceitas em silêncio: o motor não precisa delas (seção 4.3 do plano).
            "pull" => self.got.pull = true,
            "tls-client" => self.got.tls_client = true,
            "client" => self.got.client = true,
            "nobind" | "persist-key" | "persist-tun" | "resolv-retry" | "auth-nocache"
            | "auth-retry" | "mute" | "verb" | "mute-replay-warnings" | "float" | "dh"
            | "remote-random-hostname" | "ifconfig-nowarn" | "mlock" | "bind" | "tcp-nodelay"
            | "preresolve" | "ncp-disable" | "auth-token-user" | "key-method" | "ifconfig"
            | "route-gateway" | "route-metric" | "route-delay" | "route-method" | "topology"
            | "sndbuf" | "rcvbuf" | "fast-io" | "block-outside-dns" | "register-dns"
            | "ip-win32" | "tap-sleep" | "win-sys" | "dhcp-renew" | "dhcp-release" | "daemon"
            | "log" | "log-append" | "syslog" | "status" | "status-version" | "writepid"
            | "user" | "group" | "chroot" | "cd" | "setenv" | "setenv-safe" | "echo" | "suppress-timestamps"
            | "machine-readable-output" | "passtos" | "txqueuelen" | "nice" | "rdns-internal" => {}
            n if n.starts_with("management") && n != "management-external-key" => {}
            "dev" | "dev-type" => {
                let value = args.first().ok_or_else(|| self.missing(name, line))?;
                if value.starts_with("tap") {
                    return Err(self.unsupported(
                        line,
                        "o .ovpn usa dev tap (ponte de rede), que o app não suporta; use o contêiner da pasta redes/openvpn",
                    ));
                }
                if !value.starts_with("tun") && name == "dev" {
                    self.warn(name, line, format!("dev {value} não é um dispositivo tun conhecido; o app usa a pilha em software"));
                }
            }
            "remote" => {
                let remote = self.parse_remote(args, line)?;
                self.got.remotes.push(remote);
            }
            "remote-random" => self.config.remote_random = true,
            "port" => self.got.global_port = Some(self.number(name, args, 0, line)?),
            "rport" => self.got.global_rport = Some(self.number(name, args, 0, line)?),
            "lport" => self.config.local_port = Some(self.number(name, args, 0, line)?),
            "local" => self.warn(name, line, "local (endereço de origem) não é usado pelo app"),
            "proto" => self.got.global_proto = Some(self.parse_proto(args, line)?),
            "connect-retry" => {
                self.config.connect_retry = Some(self.number(name, args, 0, line)?);
                if args.len() > 1 {
                    self.config.connect_retry_max = Some(self.number(name, args, 1, line)?);
                }
            }
            "connect-retry-max" => {}
            "connect-timeout" => self.config.connect_timeout = Some(self.number(name, args, 0, line)?),
            "server-poll-timeout" => {
                self.config.server_poll_timeout = Some(self.number(name, args, 0, line)?);
            }
            "ca" => {
                if let Some(p) = self.source(name, args, line)? {
                    self.got.ca.push(p);
                }
            }
            "cert" => {
                if let Some(p) = self.source(name, args, line)? {
                    self.got.cert = Some(p);
                }
            }
            "key" => {
                if let Some(p) = self.source(name, args, line)? {
                    self.got.key = Some(p);
                }
            }
            "extra-certs" => {
                if let Some(p) = self.source(name, args, line)? {
                    self.got.extra_certs.push(p);
                }
            }
            "pkcs12" => return Err(self.pkcs12_error(line)),
            "askpass" => self.warn(
                name,
                line,
                "askpass ainda não é usado: a chave privada com senha não é suportada",
            ),
            "tls-auth" => {
                if let Some(p) = self.source(name, args, line)? {
                    self.got.tls_auth = Some(p);
                }
                if let Some(d) = args.get(1) {
                    self.got.tls_auth_direction = Some(self.direction(name, d, line)?);
                }
            }
            "tls-crypt" => {
                if let Some(p) = self.source(name, args, line)? {
                    self.got.tls_crypt = Some(p);
                }
            }
            "tls-crypt-v2" => {
                if let Some(p) = self.source(name, args, line)? {
                    self.got.tls_crypt_v2 = Some(p);
                }
            }
            "key-direction" => {
                let d = args.first().ok_or_else(|| self.missing(name, line))?;
                self.got.key_direction = Some(self.direction(name, d, line)?);
            }
            "auth" => {
                let value = args.first().ok_or_else(|| self.missing(name, line))?;
                self.config.auth_digest = Some(match value.to_ascii_uppercase().as_str() {
                    "NONE" => AuthDigest::None,
                    "SHA1" => AuthDigest::Sha1,
                    "SHA256" => AuthDigest::Sha256,
                    "SHA384" => AuthDigest::Sha384,
                    "SHA512" => AuthDigest::Sha512,
                    other => {
                        return Err(self.invalid(
                            name,
                            line,
                            format!("o digest {other} não é suportado (use SHA1, SHA256, SHA384 ou SHA512)"),
                        ))
                    }
                });
            }
            "cipher" => self.config.cipher = args.first().cloned(),
            "data-ciphers" | "ncp-ciphers" => {
                let value = args.first().ok_or_else(|| self.missing(name, line))?;
                self.config.data_ciphers =
                    Some(value.split(':').filter(|c| !c.is_empty()).map(str::to_string).collect());
            }
            "data-ciphers-fallback" => self.config.data_ciphers_fallback = args.first().cloned(),
            "tls-version-min" => {
                self.config.tls_version_min = Some(self.tls_version(name, args, line)?);
            }
            "tls-version-max" => {
                self.config.tls_version_max = Some(self.tls_version(name, args, line)?);
            }
            "tls-cipher" | "tls-ciphersuites" | "tls-groups" | "tls-cert-profile" | "ecdh-curve" => {
                self.warn(name, line, format!("{name} é ignorado: o app usa as suítes TLS próprias, todas seguras"));
            }
            "remote-cert-tls" => match args.first().map(String::as_str) {
                Some("server") => self.config.verify.remote_cert_tls = true,
                Some("client") => {
                    return Err(self.invalid(
                        name,
                        line,
                        "remote-cert-tls client é de servidor; num cliente use server",
                    ))
                }
                _ => return Err(self.missing(name, line)),
            },
            "ns-cert-type" => match args.first().map(String::as_str) {
                Some("server") => {
                    self.config.verify.remote_cert_tls = true;
                    self.warn(name, line, "ns-cert-type é antigo; o app usa remote-cert-tls server no lugar");
                }
                _ => return Err(self.invalid(name, line, "só ns-cert-type server vale num cliente")),
            },
            "remote-cert-ku" => {
                for v in args {
                    let ku = u16::from_str_radix(v.trim_start_matches("0x"), 16)
                        .map_err(|_| self.invalid(name, line, format!("\"{v}\" não é hexadecimal")))?;
                    self.config.verify.remote_cert_ku.push(ku);
                }
            }
            "remote-cert-eku" => {
                self.config.verify.remote_cert_eku =
                    Some(args.first().ok_or_else(|| self.missing(name, line))?.clone());
            }
            "verify-x509-name" => {
                let value = args.first().ok_or_else(|| self.missing(name, line))?.clone();
                let mode = match args.get(1).map(String::as_str) {
                    None | Some("subject") => NameMode::Subject,
                    Some("name") => NameMode::Name,
                    Some("name-prefix") => NameMode::NamePrefix,
                    Some(other) => {
                        return Err(self.invalid(
                            name,
                            line,
                            format!("o tipo \"{other}\" não existe (use subject, name ou name-prefix)"),
                        ))
                    }
                };
                self.config.verify.x509_name = Some(X509Name { value, mode });
            }
            "peer-fingerprint" => {
                let value = args.first().ok_or_else(|| self.missing(name, line))?.clone();
                self.add_fingerprint(&value, line)?;
            }
            "verify-hash" => {
                let value = args.first().ok_or_else(|| self.missing(name, line))?;
                let bytes = parse_hex_fingerprint(value)
                    .filter(|b| b.len() == 20 || b.len() == 32)
                    .ok_or_else(|| {
                        self.invalid(name, line, "esperada uma impressão SHA-1 ou SHA-256 em hexadecimal")
                    })?;
                self.config.verify.verify_hash.push(bytes);
            }
            "crl-verify" => self.warn(
                name,
                line,
                "crl-verify ainda não é verificado: certificados revogados não serão recusados",
            ),
            "auth-user-pass" => match args.first() {
                None => self.config.auth_user_pass = AuthUserPass::Ask,
                Some(file) => {
                    let pending = self.read_file(name, file, line)?;
                    let mut lines = pending.text.lines().map(str::trim_end);
                    let user = lines.next().unwrap_or_default().to_string();
                    let password = lines.next().map(|p| Zeroizing::new(p.to_string()));
                    self.config.auth_user_pass = AuthUserPass::Credentials { user, password };
                }
            },
            "static-challenge" => {
                let text = args.first().ok_or_else(|| self.missing(name, line))?.clone();
                let echo = self.number::<u8>(name, args, 1, line)? != 0;
                self.config.static_challenge = Some(StaticChallenge { text, echo });
            }
            "tun-mtu" => self.config.tun_mtu = Some(self.number(name, args, 0, line)?),
            "link-mtu" => self.config.link_mtu = Some(self.number(name, args, 0, line)?),
            "tls-mtu" => self.config.tls_mtu = Some(self.number(name, args, 0, line)?),
            "mssfix" => {
                self.config.mssfix = Some(match args.first() {
                    None => MssFix::Auto,
                    Some(_) => match self.number::<u16>(name, args, 0, line)? {
                        0 => MssFix::Off,
                        n => MssFix::Value(n),
                    },
                });
            }
            "fragment" => {
                return Err(self.unsupported(
                    line,
                    "fragment não é suportado (o servidor também precisaria dele); tire a opção do .ovpn e do servidor",
                ))
            }
            "reneg-sec" => self.config.reneg_sec = Some(self.number(name, args, 0, line)?),
            "reneg-bytes" => self.config.reneg_bytes = Some(self.number(name, args, 0, line)?),
            "reneg-pkts" => self.config.reneg_pkts = Some(self.number(name, args, 0, line)?),
            "hand-window" => self.config.hand_window = Some(self.number(name, args, 0, line)?),
            "tran-window" => self.config.tran_window = Some(self.number(name, args, 0, line)?),
            "replay-window" => {
                let n = self.number(name, args, 0, line)?;
                let t = if args.len() > 1 { Some(self.number(name, args, 1, line)?) } else { None };
                self.config.replay_window = Some((n, t));
            }
            "ping" => self.config.ping = Some(self.number(name, args, 0, line)?),
            "ping-restart" => self.config.ping_restart = Some(self.number(name, args, 0, line)?),
            "ping-exit" => self.config.ping_exit = Some(self.number(name, args, 0, line)?),
            "keepalive" => {
                self.config.ping = Some(self.number(name, args, 0, line)?);
                self.config.ping_restart = Some(self.number(name, args, 1, line)?);
            }
            "inactive" => {
                let secs = self.number(name, args, 0, line)?;
                let bytes = if args.len() > 1 { Some(self.number(name, args, 1, line)?) } else { None };
                self.config.inactive = Some((secs, bytes));
            }
            "explicit-exit-notify" => {
                self.config.explicit_exit_notify =
                    Some(if args.is_empty() { 1 } else { self.number(name, args, 0, line)? });
            }
            "route" => self.config.routes.push(args.to_vec()),
            "route-nopull" => self.config.route_nopull = true,
            "pull-filter" => {
                let action = match args.first().map(String::as_str) {
                    Some("accept") => FilterAction::Accept,
                    Some("ignore") => FilterAction::Ignore,
                    Some("reject") => FilterAction::Reject,
                    _ => return Err(self.invalid(name, line, "esperado accept, ignore ou reject")),
                };
                let prefix = args.get(1).ok_or_else(|| self.missing(name, line))?.clone();
                self.config.pull_filters.push(PullFilter { action, prefix });
            }
            "redirect-gateway" => {
                self.config.redirect_gateway.get_or_insert_with(Vec::new).extend(args.iter().cloned());
            }
            "dhcp-option" => self.config.dhcp_options.push(args.to_vec()),
            "comp-lzo" => {
                self.config.compress = Some(match args.first().map(String::as_str) {
                    Some("no") => Compress::LzoOff,
                    _ => Compress::Lzo,
                });
                self.warn(
                    name,
                    line,
                    "comp-lzo é antigo e o app não comprime; pacotes comprimidos do servidor serão recusados",
                );
            }
            "compress" => {
                self.config.compress = Some(match args.first().map(String::as_str) {
                    None | Some("stub") => Compress::Stub,
                    Some("stub-v2") => Compress::StubV2,
                    Some("lzo") => Compress::Lzo,
                    Some("lz4") => Compress::Lz4,
                    Some("lz4-v2") => Compress::Lz4V2,
                    Some(other) => {
                        return Err(self.invalid(name, line, format!("algoritmo \"{other}\" desconhecido")))
                    }
                });
            }
            "allow-compression" => {
                self.config.allow_compression = Some(match args.first().map(String::as_str) {
                    Some("no") => AllowCompression::No,
                    Some("asym") => AllowCompression::Asym,
                    Some("yes") => AllowCompression::Yes,
                    _ => return Err(self.invalid(name, line, "esperado no, asym ou yes")),
                });
            }
            "http-proxy" | "socks-proxy" | "http-proxy-option" | "http-proxy-retry" | "http-proxy-timeout" | "socks-proxy-retry" => {
                self.warn(name, line, format!("{name} do .ovpn ainda não é usado; o app tem a configuração de proxy própria"));
            }
            "up" | "down" | "route-up" | "route-pre-down" | "ipchange" | "tls-verify" | "plugin"
            | "script-security" | "up-restart" | "learn-address" | "client-connect"
            | "client-disconnect" | "auth-user-pass-verify" | "down-pre" => {
                self.warn(name, line, "o app não executa scripts nem plugins do .ovpn");
            }
            "secret" => return Err(self.secret_error(line)),
            "mode" => {
                if args.first().map(String::as_str) != Some("p2p") {
                    return Err(self.unsupported(
                        line,
                        "o .ovpn é de um servidor (mode server); use o .ovpn de um cliente",
                    ));
                }
            }
            "server" | "tls-server" | "server-bridge" | "multihome" => {
                return Err(self.unsupported(
                    line,
                    "o .ovpn é de um servidor (mode server); use o .ovpn de um cliente",
                ))
            }
            "cryptoapicert" => {
                return Err(self.unsupported(
                    line,
                    "o .ovpn usa cryptoapicert, que o app não suporta; use o contêiner da pasta redes/openvpn",
                ))
            }
            "management-external-key" => {
                return Err(self.unsupported(
                    line,
                    "o .ovpn usa management-external-key, que o app não suporta; use o contêiner da pasta redes/openvpn",
                ))
            }
            n if n.starts_with("pkcs11-") => {
                return Err(self.unsupported(
                    line,
                    "o .ovpn usa pkcs11, que o app não suporta; use o contêiner da pasta redes/openvpn",
                ))
            }
            other => self.warn(other, line, format!("opção desconhecida: {other}")),
        }
        Ok(())
    }

    fn direction(&self, option: &str, value: &str, line: usize) -> Result<u8, ConfigError> {
        match value {
            "0" => Ok(0),
            "1" => Ok(1),
            other => Err(self.invalid(
                option,
                line,
                format!("a direção deve ser 0 ou 1, veio \"{other}\""),
            )),
        }
    }

    fn tls_version(
        &self,
        option: &str,
        args: &[String],
        line: usize,
    ) -> Result<TlsVersion, ConfigError> {
        match args.first().map(String::as_str) {
            Some("1.2") => Ok(TlsVersion::V1_2),
            Some("1.3") => Ok(TlsVersion::V1_3),
            Some("1.0" | "1.1") => Err(self.unsupported(
                line,
                &format!("{option} abaixo de 1.2 não é suportado; o app só fala TLS 1.2 e 1.3"),
            )),
            Some(other) => {
                Err(self.invalid(option, line, format!("versão \"{other}\" desconhecida")))
            }
            None => Err(self.missing(option, line)),
        }
    }

    /// Fecha a leitura: aplica os padrões e monta o PKI.
    fn finish(mut self) -> Result<(Config, Vec<Warning>), ConfigError> {
        let got = std::mem::take(&mut self.got);
        self.config.is_client = got.client || (got.pull && got.tls_client);

        // Os remotes: a porta e o protocolo do `remote`, depois os do bloco/globais, depois os
        // padrões do OpenVPN (porta 1194, UDP).
        for raw in got.remotes {
            let (proto, family) = raw
                .proto
                .or(got.global_proto)
                .unwrap_or((Proto::Udp, Family::Any));
            let port = raw
                .port
                .or(got.global_rport)
                .or(got.global_port)
                .unwrap_or(DEFAULT_PORT);
            self.config.remotes.push(Remote {
                host: raw.host,
                port,
                proto,
                family,
            });
        }

        let pki_error = |line: usize, e: PkiError| ConfigError {
            line: Some(line),
            kind: ConfigErrorKind::Pki(e),
        };
        let mut pki = Pki::default();
        for pending in &got.ca {
            let certs =
                parse_certificates(&pending.text).map_err(|e| pki_error(pending.line, e))?;
            pki.ca.extend(certs);
        }
        if let Some(pending) = &got.cert {
            let mut certs =
                parse_certificates(&pending.text).map_err(|e| pki_error(pending.line, e))?;
            for extra in &got.extra_certs {
                certs
                    .extend(parse_certificates(&extra.text).map_err(|e| pki_error(extra.line, e))?);
            }
            pki.cert = Some(certs);
        }
        if let Some(pending) = &got.key {
            pki.key =
                Some(PrivateKey::from_pem(&pending.text).map_err(|e| pki_error(pending.line, e))?);
        }
        pki.key_direction = got.key_direction.or(got.tls_auth_direction);
        let wraps = [
            got.tls_auth.is_some(),
            got.tls_crypt.is_some(),
            got.tls_crypt_v2.is_some(),
        ];
        if wraps.iter().filter(|w| **w).count() > 1 {
            let line = got
                .tls_crypt
                .as_ref()
                .or(got.tls_crypt_v2.as_ref())
                .or(got.tls_auth.as_ref())
                .map_or(1, |p| p.line);
            return Err(self.invalid(
                "tls-auth",
                line,
                "tls-auth, tls-crypt e tls-crypt-v2 não se combinam; deixe só um",
            ));
        }
        if let Some(pending) = &got.tls_auth {
            pki.static_key = Some(
                StaticKey::from_pem(&pending.text)
                    .map_err(|e| pki_error(pending.line, e.into()))?,
            );
            pki.wrap = Wrap::TlsAuth;
        } else if let Some(pending) = &got.tls_crypt {
            pki.static_key = Some(
                StaticKey::from_pem(&pending.text)
                    .map_err(|e| pki_error(pending.line, e.into()))?,
            );
            pki.wrap = Wrap::TlsCrypt;
        } else if let Some(pending) = &got.tls_crypt_v2 {
            pki.wrap =
                parse_tls_crypt_v2_client(&pending.text).map_err(|e| pki_error(pending.line, e))?;
        }
        self.config.pki = pki;
        Ok((self.config, self.warnings))
    }
}

/// Lê `AA:BB:…` ou `aabb…` em bytes.
fn parse_hex_fingerprint(text: &str) -> Option<Vec<u8>> {
    let digits: Vec<u8> = text.bytes().filter(|b| *b != b':').collect();
    if digits.is_empty() || digits.len() & 1 == 1 {
        return None;
    }
    digits
        .chunks(2)
        .map(|pair| {
            let h = char::from(pair[0]).to_digit(16)?;
            let l = char::from(pair[1]).to_digit(16)?;
            u8::try_from(h * 16 + l).ok()
        })
        .collect()
}
