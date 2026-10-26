//! Chaves SSH: gerar, ler, trocar a passphrase e mostrar a impressão digital e o randomart, no
//! formato do OpenSSH (`aes256-ctr` com `bcrypt-pbkdf`), sem programa externo.
//!
//! É o equivalente do `ssh-keygen`, também com o `.ppk`. Quem usa decide a segurança: a chave pode ficar
//! sem passphrase, e as chaves antigas (RSA menor, DSA) continuam legíveis. Só a **geração**
//! recusa o que o OpenSSH atual já recusa: [`KeyType`] não tem DSA nem RSA com menos de 3072 bits.
//!
//! ```no_run
//! use tomni_ui_terminal::ssh::keys::{KeyGen, KeyType};
//!
//! let chave = KeyGen::new(KeyType::Ed25519)
//!     .comment("ana@notebook")
//!     .passphrase("uma frase longa")
//!     .generate()
//!     .expect("gerar");
//! println!("{}", chave.fingerprint());
//! println!("{}", chave.randomart());
//! chave.write("/home/ana/.ssh/id_ed25519".as_ref(), false).expect("gravar");
//! ```

use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use rand::Rng;
use ssh_key::private::{KeypairData, RsaKeypair};
use ssh_key::{Algorithm, Cipher, Fingerprint, HashAlg, Kdf, KdfAlg, LineEnding, PrivateKey};
use ssh_key::{PublicKey, public};
use zeroize::Zeroizing;

pub use ssh_key::EcdsaCurve;

mod convert;
mod ppk;

pub use convert::{
    PrivateFormat, PublicFormat, export_private, export_public, parse_public, write_key_text,
};

/// O tipo da chave gerada.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyType {
    /// Ed25519, o padrão do OpenSSH.
    Ed25519,
    /// ECDSA numa das curvas NIST.
    Ecdsa(EcdsaCurve),
    /// RSA de 3072 ou 4096 bits.
    Rsa(RsaBits),
}

/// O tamanho de uma chave RSA nova: o OpenSSH atual recusa menos de 3072 bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RsaBits {
    /// 3072 bits.
    B3072,
    /// 4096 bits.
    B4096,
}

impl RsaBits {
    /// O tamanho em bits.
    pub fn bits(self) -> u32 {
        match self {
            Self::B3072 => 3072,
            Self::B4096 => 4096,
        }
    }
}

impl KeyType {
    /// Todos os tipos que se pode gerar, na ordem em que o app os lista.
    pub const ALL: [KeyType; 6] = [
        KeyType::Ed25519,
        KeyType::Ecdsa(EcdsaCurve::NistP256),
        KeyType::Ecdsa(EcdsaCurve::NistP384),
        KeyType::Ecdsa(EcdsaCurve::NistP521),
        KeyType::Rsa(RsaBits::B3072),
        KeyType::Rsa(RsaBits::B4096),
    ];

    /// O nome para mostrar: `Ed25519`, `ECDSA P-256`, `RSA 3072`.
    pub fn label(self) -> String {
        match self {
            Self::Ed25519 => "Ed25519".to_owned(),
            Self::Ecdsa(curve) => format!("ECDSA P-{}", curve_bits(curve)),
            Self::Rsa(bits) => format!("RSA {}", bits.bits()),
        }
    }

    /// O nome do arquivo que o OpenSSH usa por padrão: `id_ed25519`, `id_ecdsa`, `id_rsa`.
    pub fn file_name(self) -> &'static str {
        match self {
            Self::Ed25519 => "id_ed25519",
            Self::Ecdsa(_) => "id_ecdsa",
            Self::Rsa(_) => "id_rsa",
        }
    }

    /// Lê o `-t` e o `-b` do `ssh-keygen`: `ed25519`, `ecdsa` (256, 384 ou 521; o padrão é 256) e
    /// `rsa` (3072 ou 4096; o padrão é 3072). Outro tipo ou tamanho é um erro em português.
    pub fn parse(name: &str, bits: Option<u32>) -> Result<Self, String> {
        match name.to_ascii_lowercase().as_str() {
            "ed25519" => match bits {
                None | Some(256) => Ok(Self::Ed25519),
                Some(_) => Err("o tamanho de uma chave ed25519 é fixo (256)".to_owned()),
            },
            "ecdsa" => match bits {
                None | Some(256) => Ok(Self::Ecdsa(EcdsaCurve::NistP256)),
                Some(384) => Ok(Self::Ecdsa(EcdsaCurve::NistP384)),
                Some(521) => Ok(Self::Ecdsa(EcdsaCurve::NistP521)),
                Some(_) => Err("o tamanho de uma chave ecdsa é 256, 384 ou 521".to_owned()),
            },
            "rsa" => match bits {
                None | Some(3072) => Ok(Self::Rsa(RsaBits::B3072)),
                Some(4096) => Ok(Self::Rsa(RsaBits::B4096)),
                Some(_) => Err("o tamanho de uma chave rsa é 3072 ou 4096".to_owned()),
            },
            "dsa" => Err("chaves DSA não são geradas: o OpenSSH atual as recusa".to_owned()),
            other => Err(format!("tipo de chave desconhecido: {other}")),
        }
    }
}

fn curve_bits(curve: EcdsaCurve) -> u32 {
    match curve {
        EcdsaCurve::NistP256 => 256,
        EcdsaCurve::NistP384 => 384,
        EcdsaCurve::NistP521 => 521,
    }
}

/// Quantas rodadas o `bcrypt-pbkdf` faz para derivar a chave da passphrase (o `-a` do
/// `ssh-keygen`). Mais rodadas deixam o ataque por força bruta mais lento, e o desbloqueio também.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KdfLevel {
    /// 16 rodadas, o padrão do OpenSSH.
    Standard,
    /// 64 rodadas.
    High,
    /// 100 rodadas.
    VeryHigh,
    /// Outro número de rodadas (pelo menos 1).
    Custom(u32),
}

impl KdfLevel {
    /// O número de rodadas.
    pub fn rounds(self) -> u32 {
        match self {
            Self::Standard => 16,
            Self::High => 64,
            Self::VeryHigh => 100,
            Self::Custom(rounds) => rounds,
        }
    }
}

/// Por que uma chave não foi gerada, lida ou gravada.
#[derive(Debug)]
pub enum KeyError {
    /// A passphrase não abre a chave.
    WrongPassphrase,
    /// A chave é cifrada e nenhuma passphrase foi dada.
    NeedsPassphrase,
    /// O formato ou o algoritmo não é suportado.
    Unsupported(String),
    /// O arquivo não é uma chave legível.
    Invalid(String),
    /// O arquivo já existe e não pode ser sobrescrito.
    Exists(PathBuf),
    /// Erro ao ler ou gravar.
    Io(io::Error),
    /// O texto é uma chave pública: falta a privada.
    OnlyPublic,
    /// O formato não existe para este tipo de chave (por exemplo, o PEM tradicional do Ed25519).
    FormatNotSupported {
        /// O formato pedido.
        format: &'static str,
        /// O tipo da chave (`ED25519`, `RSA`…).
        kind: String,
    },
    /// O formato não guarda passphrase (o PEM tradicional).
    PassphraseNotSupported,
}

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongPassphrase => f.write_str("A passphrase não confere."),
            Self::NeedsPassphrase => f.write_str("A chave é protegida por uma passphrase."),
            Self::Unsupported(what) => write!(f, "Não há suporte para {what}."),
            Self::Invalid(why) => write!(f, "A chave não pôde ser lida: {why}."),
            Self::Exists(path) => write!(f, "O arquivo já existe: {}.", path.display()),
            Self::Io(error) => write!(f, "{}", crate::transport::error_message(error)),
            Self::OnlyPublic => f.write_str("O texto é uma chave pública: falta a privada."),
            Self::FormatNotSupported { format, kind } => {
                write!(f, "O formato {format} não existe para chaves {kind}.")
            }
            Self::PassphraseNotSupported => f.write_str(
                "O PEM tradicional não guarda passphrase: use PKCS#8, OpenSSH ou PuTTY.",
            ),
        }
    }
}

impl std::error::Error for KeyError {}

impl From<io::Error> for KeyError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<ssh_key::Error> for KeyError {
    fn from(error: ssh_key::Error) -> Self {
        Self::Invalid(error.to_string())
    }
}

/// O comentário que o `ssh-keygen` põe por padrão: `usuário@máquina`. O usuário vem de `USER` ou
/// `USERNAME` e a máquina de `HOSTNAME` ou de `/etc/hostname`; sem nenhum dos dois, fica vazio.
pub fn default_comment() -> String {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default();
    let host = std::env::var("HOSTNAME")
        .ok()
        .or_else(|| fs::read_to_string("/etc/hostname").ok())
        .map(|host| host.trim().to_owned())
        .unwrap_or_default();
    match (user.is_empty(), host.is_empty()) {
        (false, false) => format!("{user}@{host}"),
        (false, true) => user,
        (true, false) => host,
        (true, true) => String::new(),
    }
}

/// Gera uma chave nova.
#[derive(Clone, Debug)]
pub struct KeyGen {
    kind: KeyType,
    comment: Option<String>,
    passphrase: Zeroizing<String>,
    kdf: KdfLevel,
}

impl KeyGen {
    /// Uma chave do tipo `kind`, sem passphrase.
    pub fn new(kind: KeyType) -> Self {
        Self {
            kind,
            comment: None,
            passphrase: Zeroizing::new(String::new()),
            kdf: KdfLevel::Standard,
        }
    }

    /// O comentário da chave (o padrão é [`default_comment`]).
    pub fn comment(mut self, comment: impl Into<String>) -> Self {
        self.comment = Some(comment.into());
        self
    }

    /// A passphrase que cifra a chave privada; vazia, a chave fica sem passphrase.
    pub fn passphrase(mut self, passphrase: impl Into<String>) -> Self {
        self.passphrase = Zeroizing::new(passphrase.into());
        self
    }

    /// As rodadas do `bcrypt-pbkdf` (só contam com passphrase).
    pub fn kdf(mut self, level: KdfLevel) -> Self {
        self.kdf = level;
        self
    }

    /// Gera a chave. O RSA 4096 leva alguns segundos: chame numa thread.
    pub fn generate(self) -> Result<GeneratedKey, KeyError> {
        let mut rng = rand::rng();
        let comment = self.comment.unwrap_or_else(default_comment);
        let mut plain = match self.kind {
            KeyType::Ed25519 => PrivateKey::random(&mut rng, Algorithm::Ed25519)?,
            KeyType::Ecdsa(curve) => PrivateKey::random(&mut rng, Algorithm::Ecdsa { curve })?,
            KeyType::Rsa(bits) => {
                let pair = RsaKeypair::random(&mut rng, bits.bits() as usize)?;
                PrivateKey::new(KeypairData::from(pair), "")?
            }
        };
        plain.set_comment(comment);
        let stored = protect(&plain, &self.passphrase, self.kdf)?;
        Ok(GeneratedKey { plain, stored })
    }
}

/// Cifra `key` com `passphrase` (vazia, devolve a chave como está) no formato do `ssh-keygen`.
fn protect(key: &PrivateKey, passphrase: &str, level: KdfLevel) -> Result<PrivateKey, KeyError> {
    if passphrase.is_empty() {
        return Ok(key.clone());
    }
    if level.rounds() == 0 {
        return Err(KeyError::Unsupported("zero rodadas do bcrypt".to_owned()));
    }
    let mut rng = rand::rng();
    let Kdf::Bcrypt { salt, .. } = Kdf::new(KdfAlg::Bcrypt, &mut rng)? else {
        return Err(KeyError::Unsupported("este KDF".to_owned()));
    };
    let kdf = Kdf::Bcrypt {
        salt,
        rounds: level.rounds(),
    };
    Ok(key.encrypt_with(Cipher::Aes256Ctr, kdf, rng.next_u32(), passphrase)?)
}

/// Uma chave recém-gerada, com a privada cifrada ou não, pronta para gravar.
pub struct GeneratedKey {
    /// A chave aberta, para o app carregá-la no agente.
    plain: PrivateKey,
    /// A chave como vai para o arquivo.
    stored: PrivateKey,
}

impl fmt::Debug for GeneratedKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GeneratedKey")
            .field("fingerprint", &self.fingerprint())
            .finish_non_exhaustive()
    }
}

impl GeneratedKey {
    /// O arquivo da chave privada, no formato OpenSSH (cifrado se houver passphrase).
    pub fn private_openssh(&self) -> Zeroizing<String> {
        self.stored
            .to_openssh(LineEnding::LF)
            .unwrap_or_else(|_| Zeroizing::new(String::new()))
    }

    /// A linha do `.pub`: `ssh-ed25519 AAAA… comentário`.
    pub fn public_openssh(&self) -> String {
        public_openssh(self.plain.public_key())
    }

    /// A impressão digital SHA-256: `SHA256:…`.
    pub fn fingerprint(&self) -> String {
        fingerprint(self.plain.public_key())
    }

    /// O quadro do `ssh-keygen -lv`.
    pub fn randomart(&self) -> String {
        randomart(self.plain.public_key())
    }

    /// A chave aberta (sem a passphrase), por exemplo para pôr no agente.
    pub fn private_key(&self) -> &PrivateKey {
        &self.plain
    }

    /// A chave pública.
    pub fn public_key(&self) -> &PublicKey {
        self.plain.public_key()
    }

    /// O comentário.
    pub fn comment(&self) -> String {
        self.plain.comment().to_string()
    }

    /// Grava `path` (0600 no Unix) e `path.pub` (0644). Cria a pasta se faltar (0700 no Unix).
    /// Sem `overwrite`, um dos dois arquivos que já existe é um [`KeyError::Exists`].
    ///
    /// No Windows os arquivos herdam as permissões da pasta do usuário, como no `ssh-keygen` de
    /// lá.
    pub fn write(&self, path: &Path, overwrite: bool) -> Result<(), KeyError> {
        let public = public_path(path);
        if !overwrite {
            for existing in [path, public.as_path()] {
                if existing.exists() {
                    return Err(KeyError::Exists(existing.to_owned()));
                }
            }
        }
        write_key_text(path, &self.private_openssh(), true, overwrite)?;
        write_key_text(
            &public,
            &format!("{}\n", self.public_openssh()),
            false,
            overwrite,
        )?;
        Ok(())
    }
}

/// O caminho do `.pub` de uma chave privada: o mesmo nome mais `.pub`.
pub fn public_path(private: &Path) -> PathBuf {
    let mut name = private.as_os_str().to_owned();
    name.push(".pub");
    PathBuf::from(name)
}

/// A linha `.pub` de uma chave pública (com o comentário).
pub fn public_openssh(public: &PublicKey) -> String {
    public.to_openssh().unwrap_or_default()
}

/// A impressão digital SHA-256 de uma chave pública: `SHA256:…`.
pub fn fingerprint(public: &PublicKey) -> String {
    public.fingerprint(HashAlg::Sha256).to_string()
}

/// O tipo para mostrar (`ED25519`, `ECDSA`, `RSA`, `DSA`) e o tamanho em bits da chave.
pub fn kind_and_bits(public: &PublicKey) -> (String, u32) {
    match public.key_data() {
        public::KeyData::Ed25519(_) => ("ED25519".to_owned(), 256),
        public::KeyData::Ecdsa(key) => {
            let curve = match key {
                public::EcdsaPublicKey::NistP256(_) => EcdsaCurve::NistP256,
                public::EcdsaPublicKey::NistP384(_) => EcdsaCurve::NistP384,
                public::EcdsaPublicKey::NistP521(_) => EcdsaCurve::NistP521,
            };
            ("ECDSA".to_owned(), curve_bits(curve))
        }
        public::KeyData::Rsa(key) => ("RSA".to_owned(), key.key_size()),
        other => (
            other
                .algorithm()
                .as_str()
                .split('@')
                .next()
                .unwrap_or_default()
                .to_ascii_uppercase(),
            0,
        ),
    }
}

/// O quadro de uma chave pública, igual ao `ssh-keygen -lv`.
pub fn randomart(public: &PublicKey) -> String {
    let (kind, bits) = kind_and_bits(public);
    let print: Fingerprint = public.fingerprint(HashAlg::Sha256);
    print.to_randomart(&format!("[{kind} {bits}]"))
}

/// Cria a pasta de `path` se faltar (0700 no Unix, como o `~/.ssh`).
fn ensure_parent(path: &Path) -> io::Result<()> {
    let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) else {
        return Ok(());
    };
    if parent.exists() {
        return Ok(());
    }
    fs::create_dir_all(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Grava `bytes` em `path` com a permissão `mode` (só no Unix). Sem `overwrite`, falha se o
/// arquivo existe (a criação é atômica, sem janela entre conferir e criar).
fn write_file(path: &Path, bytes: &[u8], mode: u32, overwrite: bool) -> Result<(), KeyError> {
    let mut options = fs::OpenOptions::new();
    options.write(true);
    if overwrite {
        options.create(true).truncate(true);
    } else {
        options.create_new(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(mode);
    }
    #[cfg(not(unix))]
    let _ = mode;
    let mut file = options.open(path).map_err(|error| {
        if error.kind() == io::ErrorKind::AlreadyExists {
            KeyError::Exists(path.to_owned())
        } else {
            KeyError::Io(error)
        }
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Um arquivo que já existia mantém a permissão antiga com o `mode` acima.
        file.set_permissions(fs::Permissions::from_mode(mode))?;
    }
    file.write_all(bytes)?;
    file.flush()?;
    Ok(())
}

/// O formato do arquivo de uma chave privada.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyFormat {
    /// `BEGIN OPENSSH PRIVATE KEY`.
    OpenSsh,
    /// PEM antigo: `BEGIN RSA PRIVATE KEY` (PKCS#1) ou `BEGIN EC PRIVATE KEY`.
    Pem,
    /// `BEGIN PRIVATE KEY` ou `BEGIN ENCRYPTED PRIVATE KEY` (PKCS#8).
    Pkcs8,
    /// `.ppk` do PuTTY, versão 2 (SHA-1).
    PpkV2,
    /// `.ppk` do PuTTY, versão 3 (Argon2).
    PpkV3,
}

impl KeyFormat {
    /// Se é um `.ppk`, de qualquer versão.
    pub fn is_ppk(self) -> bool {
        matches!(self, Self::PpkV2 | Self::PpkV3)
    }
}

/// O que se sabe de um arquivo de chave privada sem a passphrase.
#[derive(Clone, Debug)]
pub struct KeyInfo {
    /// O arquivo.
    pub path: PathBuf,
    /// O formato do arquivo.
    pub format: KeyFormat,
    /// O tipo: `ED25519`, `ECDSA`, `RSA`… (vazio se não deu para saber).
    pub kind: String,
    /// O tamanho em bits (0 se não deu para saber).
    pub bits: u32,
    /// O comentário.
    pub comment: String,
    /// Se há passphrase.
    pub encrypted: bool,
    /// A chave pública, quando o arquivo (ou o `.pub` ao lado) a traz.
    pub public: Option<PublicKey>,
}

impl KeyInfo {
    /// A impressão digital SHA-256, se a chave pública é conhecida.
    pub fn fingerprint(&self) -> Option<String> {
        self.public.as_ref().map(fingerprint)
    }
}

/// Uma chave privada lida do disco e aberta (OpenSSH, PEM, PKCS#8 ou `.ppk`).
pub struct KeyFile {
    key: PrivateKey,
    info: KeyInfo,
}

impl fmt::Debug for KeyFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyFile")
            .field("path", &self.info.path)
            .finish_non_exhaustive()
    }
}

impl KeyFile {
    /// Lê o tipo, o tamanho, o comentário e se há passphrase, sem pedi-la.
    pub fn inspect(path: &Path) -> Result<KeyInfo, KeyError> {
        let bytes = fs::read(path)?;
        let text = String::from_utf8(bytes)
            .map_err(|_| KeyError::Invalid("o arquivo não é texto".to_owned()))?;
        let mut info = Self::inspect_text(&text)?;
        info.path = path.to_owned();
        // Sem a pública dentro do arquivo (PEM cifrado), o `.pub` ao lado serve.
        let sibling = fs::read_to_string(public_path(path))
            .ok()
            .and_then(|line| PublicKey::from_openssh(line.trim()).ok());
        if info.public.is_none() {
            info.public = sibling.clone();
            let (kind, bits) = info.public.as_ref().map(kind_and_bits).unwrap_or_default();
            info.kind = kind;
            info.bits = bits;
        }
        if info.comment.is_empty()
            && let Some(sibling) = &sibling
        {
            info.comment = sibling.comment().to_string();
        }
        Ok(info)
    }

    /// Abre a chave, com a passphrase se ela for cifrada.
    pub fn open(path: &Path, passphrase: Option<&str>) -> Result<Self, KeyError> {
        let info = Self::inspect(path)?;
        let passphrase = passphrase.filter(|p| !p.is_empty());
        if info.encrypted && passphrase.is_none() {
            return Err(KeyError::NeedsPassphrase);
        }
        let text = Zeroizing::new(fs::read_to_string(path)?);
        let mut file = Self::from_text(&text, passphrase)?;
        // O `.pub` ao lado completa o comentário e o tipo que o arquivo não trazia.
        file.info = info;
        if file.key.comment().is_empty() && !file.info.comment.is_empty() {
            file.key.set_comment(file.info.comment.clone());
        }
        Ok(file)
    }

    /// A chave aberta.
    pub fn private_key(&self) -> &PrivateKey {
        &self.key
    }

    /// O que o arquivo dizia (o formato, o tipo, o tamanho).
    pub fn info(&self) -> &KeyInfo {
        &self.info
    }

    /// Troca, põe ou tira a passphrase (`None` ou vazia tira) e regrava em OpenSSH, com o `.pub`
    /// ao lado. Um `.ppk` vira o mesmo nome sem a extensão, e o original fica; se o novo nome já
    /// existe, é um [`KeyError::Exists`].
    pub fn rewrite(
        &self,
        path: &Path,
        passphrase: Option<&str>,
        kdf: KdfLevel,
    ) -> Result<PathBuf, KeyError> {
        let target =
            if self.info.format.is_ppk() && path.extension().is_some_and(|ext| ext == "ppk") {
                let target = path.with_extension("");
                for existing in [target.clone(), public_path(&target)] {
                    if existing.exists() {
                        return Err(KeyError::Exists(existing));
                    }
                }
                target
            } else {
                path.to_owned()
            };
        let text = export_private(&self.key, PrivateFormat::OpenSsh, passphrase, kdf)?;
        let public = format!("{}\n", public_openssh(self.key.public_key()));
        // O arquivo novo nasce ao lado e troca o antigo de uma vez: um erro no meio não perde
        // a chave.
        write_key_text(&target, &text, true, true)?;
        write_key_text(&public_path(&target), &public, false, true)?;
        Ok(target)
    }
}

/// A pública, se há passphrase e o comentário de um `.ppk`, sem abri-lo.
fn inspect_ppk(text: &str) -> Result<(PublicKey, bool, String), KeyError> {
    let invalid = || KeyError::Invalid("o .ppk está incompleto".to_owned());
    let mut lines = text.lines();
    let header = lines.next().ok_or_else(invalid)?;
    let algorithm = header
        .split_once(':')
        .map(|(_, algorithm)| algorithm.trim())
        .ok_or_else(invalid)?;
    let (mut encrypted, mut comment, mut blob) = (false, String::new(), String::new());
    while let Some(line) = lines.next() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match name {
            "Encryption" => encrypted = value != "none",
            "Comment" => comment = value.to_owned(),
            "Public-Lines" => {
                let count: usize = value.parse().map_err(|_| invalid())?;
                for _ in 0..count {
                    blob.push_str(lines.next().ok_or_else(invalid)?.trim());
                }
                break;
            }
            _ => {}
        }
    }
    // O `Comment` pode vir antes ou depois das linhas públicas.
    for line in lines {
        if let Some(("Comment", value)) = line.split_once(':') {
            comment = value.trim().to_owned();
        }
    }
    let public = PublicKey::from_openssh(&format!("{algorithm} {blob}"))
        .map_err(|error| KeyError::Invalid(error.to_string()))?;
    Ok((public, encrypted, comment))
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tests_formatos;
