//! Encaminhamento X11 (o `ssh -X` e o `ssh -Y`): os programas gráficos do servidor abrem a
//! janela no servidor X desta máquina.
//!
//! Como no OpenSSH, o servidor nunca vê o cookie de verdade: o cliente pede o encaminhamento
//! com um cookie falso e, a cada canal X11 que o servidor abre, confere o falso no pacote de
//! setup do X11 e o troca pelo real antes de falar com o display local.

use std::io;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use rand::Rng;
use russh::Channel;
use russh::client::{ChannelOpenHandle, Msg};
use subtle::ConstantTimeEq;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::link::ChannelGuard;
use super::{Notice, Wire};

/// O protocolo de autenticação do X11 que o encaminhamento usa.
const PROTOCOL: &str = "MIT-MAGIC-COOKIE-1";
/// Quanto esperar o servidor X local aceitar a conexão TCP.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Quanto esperar o pacote de setup depois de aceitar o canal.
const SETUP_TIMEOUT: Duration = Duration::from_secs(10);
/// Quanto esperar a resposta do servidor ao pedido de encaminhamento.
pub(super) const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Se o programa remoto é confiável para o servidor X desta máquina.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum X11Mode {
    /// O `ssh -Y`: o programa remoto tem o acesso total do cookie.
    Trusted,
    /// O `ssh -X`: um cookie criado com a extensão SECURITY do servidor X (`xauth generate`),
    /// que limita o que o programa remoto pode fazer (e vale por uma hora desde o último
    /// programa que o usou).
    Untrusted,
}

/// Encaminhamento X11 de uma conexão SSH: os programas gráficos do servidor abrem a janela no
/// servidor X desta máquina (o `ssh -X` e o `ssh -Y`). Passe a [`SshBuilder::x11`].
///
/// O servidor precisa de `X11Forwarding yes` no `sshd_config` e do programa `xauth`. Aqui, o
/// display vem da variável `DISPLAY` (no Windows e no macOS, `localhost:0` sem ela) e o cookie
/// do `Xauthority`; sem entrada nele, a conexão com o display segue sem autenticação, o que o
/// VcXsrv com o controle de acesso desligado, o XQuartz e o Xming aceitam.
///
/// ```no_run
/// use tomni_ui_terminal::Ssh;
/// use tomni_ui_terminal::ssh::X11;
///
/// let ssh = Ssh::builder("servidor.exemplo", 22, "ana")
///     .agent_auth()
///     .x11(X11::trusted())
///     .connect();
/// ```
///
/// [`SshBuilder::x11`]: super::SshBuilder::x11
#[derive(Clone, Debug)]
pub struct X11 {
    mode: X11Mode,
    display: Option<String>,
    auth_file: Option<PathBuf>,
    xauth: Option<PathBuf>,
}

impl X11 {
    fn new(mode: X11Mode) -> Self {
        Self {
            mode,
            display: None,
            auth_file: None,
            xauth: None,
        }
    }

    /// Confiável (o `ssh -Y`).
    pub fn trusted() -> Self {
        Self::new(X11Mode::Trusted)
    }

    /// Não confiável (o `ssh -X`): roda `xauth generate` para criar um cookie limitado. Se o
    /// `xauth` não existe ou o servidor X não tem a extensão SECURITY (o XWayland de várias
    /// distribuições), usa o confiável e avisa no terminal.
    pub fn untrusted() -> Self {
        Self::new(X11Mode::Untrusted)
    }

    /// O display local, no formato da variável `DISPLAY` (`:0`, `localhost:10.0`, o caminho do
    /// XQuartz). O padrão é o `DISPLAY` do ambiente, lido ao conectar.
    pub fn display(mut self, display: impl Into<String>) -> Self {
        self.display = Some(display.into());
        self
    }

    /// O arquivo de cookies. O padrão é o `XAUTHORITY` do ambiente, ou `~/.Xauthority`.
    pub fn auth_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.auth_file = Some(path.into());
        self
    }

    /// O programa `xauth` do modo não confiável. O padrão é o do `PATH`.
    pub fn xauth_program(mut self, path: impl Into<PathBuf>) -> Self {
        self.xauth = Some(path.into());
        self
    }

    /// O modo pedido.
    pub fn mode(&self) -> X11Mode {
        self.mode
    }
}

/// O estado do X11 de uma conexão, para a tela de túneis ativos.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct X11Status {
    /// O modo em uso (um não confiável que caiu para o confiável aparece como confiável).
    pub mode: X11Mode,
    /// O display local, como foi usado (`:0`).
    pub display: String,
    /// Quantas janelas (canais X11) estão abertas agora.
    pub active: usize,
    /// Quantos canais X11 já foram abertos nesta conexão.
    pub total: u64,
    /// O último erro (o display local que recusou a conexão, por exemplo).
    pub error: Option<String>,
}

// ----- o display local -----

/// Um jeito de chegar ao servidor X local.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Endpoint {
    /// Um socket abstrato do Linux (`@/tmp/.X11-unix/X0`), sem o `@`.
    #[cfg_attr(not(any(target_os = "linux", target_os = "android")), allow(dead_code))]
    Abstract(String),
    /// Um socket Unix num arquivo.
    Path(PathBuf),
    /// TCP.
    Tcp(String, u16),
}

/// De que máquina é o display, para escolher o cookie.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Where {
    /// Esta máquina (`:0`, o caminho do XQuartz).
    Local,
    /// `localhost`, `127.0.0.1` ou `[::1]`.
    Loopback,
    /// Outra máquina.
    Host(String),
}

/// Um display decifrado.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Display {
    pub(super) endpoints: Vec<Endpoint>,
    pub(super) number: u32,
    pub(super) screen: u32,
    pub(super) place: Where,
}

/// Lê `N` ou `N.S`.
fn number_and_screen(text: &str) -> Result<(u32, u32), String> {
    let (number, screen) = match text.split_once('.') {
        Some((number, screen)) => (number, screen),
        None => (text, "0"),
    };
    let number = number
        .parse::<u32>()
        .map_err(|_| format!("DISPLAY inválido: {text:?}"))?;
    let screen = screen
        .parse::<u32>()
        .map_err(|_| format!("DISPLAY inválido: {text:?}"))?;
    Ok((number, screen))
}

fn tcp_port(number: u32) -> Result<u16, String> {
    u16::try_from(6000 + u64::from(number))
        .map_err(|_| format!("o display :{number} não tem porta TCP"))
}

/// Decifra o valor de um `DISPLAY` nos jeitos de chegar a ele, na ordem em que se tenta.
pub(super) fn parse_display(text: &str) -> Result<Display, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("DISPLAY vazio".to_owned());
    }
    // O XQuartz: o caminho do socket do launchd até o último `:`, e o número depois.
    if text.starts_with('/') {
        let (path, rest) = text
            .rsplit_once(':')
            .ok_or_else(|| format!("DISPLAY inválido: {text:?}"))?;
        let (number, screen) = number_and_screen(rest)?;
        return Ok(Display {
            endpoints: vec![Endpoint::Path(PathBuf::from(path))],
            number,
            screen,
            place: Where::Local,
        });
    }
    let (host, rest) = text
        .rsplit_once(':')
        .ok_or_else(|| format!("DISPLAY inválido: {text:?}"))?;
    let (number, screen) = number_and_screen(rest)?;
    // Fora do Unix só sobra o TCP e o `Vec` nasce com um único `push`.
    #[cfg_attr(not(unix), allow(clippy::vec_init_then_push))]
    if host.is_empty() || host == "unix" {
        let mut endpoints = Vec::new();
        #[cfg(any(target_os = "linux", target_os = "android"))]
        endpoints.push(Endpoint::Abstract(format!("/tmp/.X11-unix/X{number}")));
        #[cfg(unix)]
        endpoints.push(Endpoint::Path(PathBuf::from(format!(
            "/tmp/.X11-unix/X{number}"
        ))));
        endpoints.push(Endpoint::Tcp("127.0.0.1".to_owned(), tcp_port(number)?));
        return Ok(Display {
            endpoints,
            number,
            screen,
            place: Where::Local,
        });
    }
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let loopback = host.eq_ignore_ascii_case("localhost")
        || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback());
    let connect = if host.eq_ignore_ascii_case("localhost") {
        "127.0.0.1"
    } else {
        host
    };
    Ok(Display {
        endpoints: vec![Endpoint::Tcp(connect.to_owned(), tcp_port(number)?)],
        number,
        screen,
        place: if loopback {
            Where::Loopback
        } else {
            Where::Host(host.to_owned())
        },
    })
}

// ----- o cookie real -----

/// Uma entrada do arquivo `Xauthority`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct XauthEntry {
    pub(super) family: u16,
    pub(super) address: Vec<u8>,
    pub(super) number: Vec<u8>,
    pub(super) name: Vec<u8>,
    pub(super) data: Vec<u8>,
}

/// Lê um campo com prefixo de comprimento (`u16` big-endian).
fn read_field<'a>(bytes: &mut &'a [u8]) -> Option<&'a [u8]> {
    let (head, rest) = bytes.split_first_chunk::<2>()?;
    let len = usize::from(u16::from_be_bytes(*head));
    let field = rest.get(..len)?;
    *bytes = &rest[len..];
    Some(field)
}

/// Lê o `Xauthority`: uma sequência de entradas, cada uma com a família (`u16` big-endian) e
/// quatro campos com prefixo de comprimento. Um arquivo cortado no meio vale até a última
/// entrada inteira.
pub(super) fn read_xauthority(mut bytes: &[u8]) -> Vec<XauthEntry> {
    let mut entries = Vec::new();
    while let Some((family, rest)) = bytes.split_first_chunk::<2>() {
        let family = u16::from_be_bytes(*family);
        bytes = rest;
        let (Some(address), Some(number), Some(name), Some(data)) = (
            read_field(&mut bytes),
            read_field(&mut bytes),
            read_field(&mut bytes),
            read_field(&mut bytes),
        ) else {
            break;
        };
        entries.push(XauthEntry {
            family,
            address: address.to_vec(),
            number: number.to_vec(),
            name: name.to_vec(),
            data: data.to_vec(),
        });
    }
    entries
}

const FAMILY_INTERNET: u16 = 0;
const FAMILY_INTERNET6: u16 = 6;
const FAMILY_LOCAL: u16 = 256;
const FAMILY_WILD: u16 = 65535;

/// O cookie do display: a primeira entrada `MIT-MAGIC-COOKIE-1` do número dele cuja família
/// serve a este display.
pub(super) fn find_cookie(
    entries: &[XauthEntry],
    display: &Display,
    hostname: &str,
) -> Option<Vec<u8>> {
    let number = display.number.to_string();
    let ip = match &display.place {
        Where::Host(host) => host.parse::<IpAddr>().ok(),
        Where::Loopback => None,
        Where::Local => None,
    };
    entries
        .iter()
        .filter(|entry| entry.name == PROTOCOL.as_bytes() && entry.number == number.as_bytes())
        .find(|entry| match entry.family {
            FAMILY_WILD => true,
            FAMILY_LOCAL => {
                matches!(display.place, Where::Local | Where::Loopback)
                    && entry.address == hostname.as_bytes()
            }
            FAMILY_INTERNET => match (ip, display.place.clone()) {
                (Some(IpAddr::V4(ip)), _) => entry.address == ip.octets(),
                (_, Where::Loopback) => entry.address == [127, 0, 0, 1],
                _ => false,
            },
            FAMILY_INTERNET6 => match ip {
                Some(IpAddr::V6(ip)) => entry.address == ip.octets(),
                _ => false,
            },
            _ => false,
        })
        .map(|entry| entry.data.clone())
}

/// O nome desta máquina, sem `unsafe` e sem dependência: o arquivo do kernel, o `/etc/hostname`
/// ou o comando `hostname`.
fn hostname() -> String {
    for file in ["/proc/sys/kernel/hostname", "/etc/hostname"] {
        if let Ok(text) = std::fs::read_to_string(file) {
            let text = text.trim();
            if !text.is_empty() {
                return text.to_owned();
            }
        }
    }
    if let Ok(output) =
        tomni_ui_core::process::no_window(&mut std::process::Command::new("hostname")).output()
        && output.status.success()
    {
        let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if !text.is_empty() {
            return text;
        }
    }
    std::env::var("COMPUTERNAME").unwrap_or_default()
}

/// O arquivo de cookies: o pedido, o `XAUTHORITY` ou `~/.Xauthority`.
fn auth_path(x11: &X11) -> Option<PathBuf> {
    x11.auth_file
        .clone()
        .or_else(|| std::env::var_os("XAUTHORITY").map(PathBuf::from))
        .or_else(|| std::env::home_dir().map(|home| home.join(".Xauthority")))
}

fn read_cookie(path: &Path, display: &Display) -> Option<Vec<u8>> {
    let bytes = std::fs::read(path).ok()?;
    find_cookie(&read_xauthority(&bytes), display, &hostname())
}

/// O cookie não confiável: roda `xauth generate` num arquivo temporário e lê o cookie dele.
fn generate_untrusted(x11: &X11, display_text: &str, display: &Display) -> Result<Vec<u8>, String> {
    let mut suffix = [0u8; 8];
    rand::rng().fill_bytes(&mut suffix);
    let file = std::env::temp_dir().join(format!("tomni-ui-xauth-{}", hex(&suffix)));
    let program = x11.xauth.clone().unwrap_or_else(|| PathBuf::from("xauth"));
    let result = (|| {
        let mut command = std::process::Command::new(&program);
        let output = tomni_ui_core::process::no_window(&mut command)
            .arg("-f")
            .arg(&file)
            .args([
                "generate",
                display_text,
                PROTOCOL,
                "untrusted",
                "timeout",
                "3600",
            ])
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|error| match error.kind() {
                io::ErrorKind::NotFound => "o programa xauth não está instalado".to_owned(),
                _ => error.to_string(),
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let line = stderr.lines().next().unwrap_or("o xauth falhou").trim();
            return Err(if line.is_empty() {
                "o xauth falhou".to_owned()
            } else {
                line.to_owned()
            });
        }
        let bytes = std::fs::read(&file).map_err(|_| "o xauth não gravou o cookie".to_owned())?;
        let entries = read_xauthority(&bytes);
        entries
            .iter()
            .find(|entry| entry.name == PROTOCOL.as_bytes())
            .or_else(|| entries.first())
            .map(|entry| entry.data.clone())
            .or_else(|| find_cookie(&entries, display, &hostname()))
            .ok_or_else(|| "o xauth não gravou o cookie".to_owned())
    })();
    let _ = std::fs::remove_file(&file);
    result
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

// ----- o setup do X11 -----

/// Por que o pacote de setup não pôde ser trocado.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum SetupError {
    /// O pacote ainda não chegou inteiro.
    Incomplete,
    /// O nome ou o dado de autenticação não são o cookie falso.
    WrongCookie,
    /// O primeiro byte não é `B` nem `l`.
    BadByteOrder,
}

/// O pacote de setup refeito.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Rewrite {
    pub(super) packet: Vec<u8>,
    /// Quantos bytes de `head` o setup original ocupava (os que vierem depois são do protocolo).
    pub(super) consumed: usize,
}

fn padded(len: usize) -> usize {
    len.div_ceil(4) * 4
}

/// Confere o cookie falso no pacote de setup do cliente X11 que está em `head` e o refaz com o
/// cookie real (ou com nome e dado vazios, se `real` é `None`). Os comprimentos mudam, então o
/// pacote é reconstruído, não trocado no lugar.
pub(super) fn rewrite_setup(
    head: &[u8],
    fake: &[u8],
    real: Option<&[u8]>,
) -> Result<Rewrite, SetupError> {
    const HEADER: usize = 12;
    if head.is_empty() {
        return Err(SetupError::Incomplete);
    }
    let big = match head[0] {
        b'B' => true,
        b'l' => false,
        _ => return Err(SetupError::BadByteOrder),
    };
    if head.len() < HEADER {
        return Err(SetupError::Incomplete);
    }
    let word = |at: usize| {
        let pair = [head[at], head[at + 1]];
        usize::from(if big {
            u16::from_be_bytes(pair)
        } else {
            u16::from_le_bytes(pair)
        })
    };
    let (name_len, data_len) = (word(6), word(8));
    let total = HEADER + padded(name_len) + padded(data_len);
    if head.len() < total {
        return Err(SetupError::Incomplete);
    }
    let name = &head[HEADER..HEADER + name_len];
    let data_at = HEADER + padded(name_len);
    let data = &head[data_at..data_at + data_len];
    let same_name = name.ct_eq(PROTOCOL.as_bytes());
    let same_data = data.ct_eq(fake);
    // Os dois resultados, sem parar no primeiro (o tempo não conta onde a diferença está).
    if !bool::from(same_name & same_data) {
        return Err(SetupError::WrongCookie);
    }
    let (new_name, new_data): (&[u8], &[u8]) = match real {
        Some(cookie) => (PROTOCOL.as_bytes(), cookie),
        None => (&[], &[]),
    };
    let put = |value: usize| {
        let value = u16::try_from(value).unwrap_or(u16::MAX);
        if big {
            value.to_be_bytes()
        } else {
            value.to_le_bytes()
        }
    };
    let mut packet = Vec::with_capacity(HEADER + padded(new_name.len()) + padded(new_data.len()));
    packet.extend_from_slice(&head[..6]);
    packet.extend_from_slice(&put(new_name.len()));
    packet.extend_from_slice(&put(new_data.len()));
    packet.extend_from_slice(&[0, 0]);
    packet.extend_from_slice(new_name);
    packet.resize(HEADER + padded(new_name.len()), 0);
    packet.extend_from_slice(new_data);
    packet.resize(HEADER + padded(new_name.len()) + padded(new_data.len()), 0);
    Ok(Rewrite {
        packet,
        consumed: total,
    })
}

// ----- o estado da conexão -----

/// O X11 de uma conexão: o display, os cookies e os contadores.
pub(super) struct X11State {
    mode: X11Mode,
    display_text: String,
    display: Display,
    real: Option<Vec<u8>>,
    fake: [u8; 16],
    /// O endpoint que conectou por último, tentado primeiro.
    preferred: AtomicUsize,
    active: AtomicUsize,
    total: AtomicU64,
    error: Mutex<Option<String>>,
    warned: AtomicBool,
}

impl std::fmt::Debug for X11State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("X11State")
            .field("mode", &self.mode)
            .field("display", &self.display_text)
            .finish_non_exhaustive()
    }
}

impl X11State {
    /// Prepara o X11 de `x11`: acha o display, lê o cookie e, no não confiável, cria o
    /// limitado. Os avisos vão para `notice`. `None` quando não há como encaminhar (o terminal
    /// já mostrou por quê).
    pub(super) fn prepare(x11: &X11, notice: &mut dyn FnMut(&str)) -> Option<Arc<Self>> {
        let text = x11
            .display
            .clone()
            .filter(|display| !display.trim().is_empty())
            .or_else(|| {
                std::env::var("DISPLAY")
                    .ok()
                    .filter(|d| !d.trim().is_empty())
            })
            .or_else(|| cfg!(any(windows, target_os = "macos")).then(|| "localhost:0".to_owned()));
        let Some(text) = text else {
            notice(
                "Encaminhamento X11 desligado: a variável DISPLAY está vazia (não há servidor X nesta sessão).",
            );
            return None;
        };
        let display = match parse_display(&text) {
            Ok(display) => display,
            Err(message) => {
                notice(&format!("Encaminhamento X11 desligado: {message}."));
                return None;
            }
        };
        let mut mode = x11.mode;
        let mut real = auth_path(x11).and_then(|path| read_cookie(&path, &display));
        if mode == X11Mode::Untrusted {
            match generate_untrusted(x11, &text, &display) {
                Ok(cookie) => real = Some(cookie),
                Err(reason) => {
                    notice(&format!(
                        "Encaminhamento X11 não confiável indisponível ({reason}); usando o confiável."
                    ));
                    mode = X11Mode::Trusted;
                }
            }
        }
        let mut fake = [0u8; 16];
        rand::rng().fill_bytes(&mut fake);
        Some(Arc::new(Self {
            mode,
            display_text: text,
            display,
            real,
            fake,
            preferred: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
            total: AtomicU64::new(0),
            error: Mutex::new(None),
            warned: AtomicBool::new(false),
        }))
    }

    /// O cookie falso em hexadecimal, para o `x11-req`.
    pub(super) fn fake_hex(&self) -> String {
        hex(&self.fake)
    }

    pub(super) fn screen(&self) -> u32 {
        self.display.screen
    }

    pub(super) fn active(&self) -> usize {
        self.active.load(Ordering::Acquire)
    }

    pub(super) fn status(&self) -> X11Status {
        X11Status {
            mode: self.mode,
            display: self.display_text.clone(),
            active: self.active(),
            total: self.total.load(Ordering::Acquire),
            error: self
                .error
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        }
    }

    /// Conecta ao servidor X local, tentando primeiro o endpoint que funcionou da última vez.
    async fn connect(&self) -> io::Result<Box<dyn Wire>> {
        let endpoints = &self.display.endpoints;
        let first = self
            .preferred
            .load(Ordering::Acquire)
            .min(endpoints.len() - 1);
        let mut last = None;
        for index in std::iter::once(first).chain((0..endpoints.len()).filter(|i| *i != first)) {
            match connect_endpoint(&endpoints[index]).await {
                Ok(stream) => {
                    self.preferred.store(index, Ordering::Release);
                    return Ok(stream);
                }
                Err(error) => last = Some(error),
            }
        }
        Err(last.unwrap_or_else(|| io::Error::other("sem endpoint")))
    }

    /// Atende um canal X11 do servidor: conecta ao display local, aceita o canal, troca o
    /// cookie do setup e faz a ponte. Roda numa tarefa própria.
    pub(super) async fn serve(
        self: Arc<Self>,
        channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        guard: ChannelGuard,
        notice: Notice,
    ) {
        let _guard = guard;
        let mut local = match self.connect().await {
            Ok(stream) => stream,
            Err(error) => {
                let reason = crate::transport::error_message(&error);
                *self.error.lock().unwrap_or_else(PoisonError::into_inner) = Some(format!(
                    "não há servidor X em {} ({reason})",
                    self.display_text
                ));
                if !self.warned.swap(true, Ordering::AcqRel) {
                    notice(&format!(
                        "Um programa do servidor tentou abrir uma janela, mas não há servidor X em {} ({reason}). No Windows, abra o VcXsrv ou o Xming; no macOS, o XQuartz.",
                        self.display_text
                    ));
                }
                // `reply` cai aqui e o servidor recebe a recusa.
                return;
            }
        };
        reply.accept().await;
        let mut remote = channel.into_stream();
        let _count = Count::new(&self);
        let setup =
            tokio::time::timeout(SETUP_TIMEOUT, self.exchange_setup(&mut remote, &mut local));
        let Ok(Some(())) = setup.await else {
            return;
        };
        let _ = tokio::io::copy_bidirectional(&mut remote, &mut local).await;
    }

    /// Lê o setup do canal, confere o cookie falso e manda o pacote refeito (e o que vier
    /// junto) ao display. `None` se o cookie estava errado ou o canal acabou.
    async fn exchange_setup(
        &self,
        remote: &mut (impl AsyncReadExt + Unpin),
        local: &mut Box<dyn Wire>,
    ) -> Option<()> {
        let mut head = Vec::with_capacity(64);
        let mut chunk = [0u8; 512];
        let rewrite = loop {
            match rewrite_setup(&head, &self.fake, self.real.as_deref()) {
                Ok(rewrite) => break rewrite,
                Err(SetupError::Incomplete) => {
                    let read = remote.read(&mut chunk).await.ok()?;
                    if read == 0 {
                        return None;
                    }
                    head.extend_from_slice(&chunk[..read]);
                }
                Err(_) => return None,
            }
        };
        local.write_all(&rewrite.packet).await.ok()?;
        if head.len() > rewrite.consumed {
            local.write_all(&head[rewrite.consumed..]).await.ok()?;
        }
        local.flush().await.ok()?;
        Some(())
    }
}

/// Conta um canal X11 aberto enquanto existir.
struct Count<'a>(&'a X11State);

impl<'a> Count<'a> {
    fn new(state: &'a X11State) -> Self {
        state.active.fetch_add(1, Ordering::AcqRel);
        state.total.fetch_add(1, Ordering::AcqRel);
        Self(state)
    }
}

impl Drop for Count<'_> {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::AcqRel);
    }
}

async fn connect_endpoint(endpoint: &Endpoint) -> io::Result<Box<dyn Wire>> {
    match endpoint {
        Endpoint::Abstract(name) => connect_abstract(name),
        #[cfg(unix)]
        Endpoint::Path(path) => Ok(Box::new(tokio::net::UnixStream::connect(path).await?)),
        #[cfg(not(unix))]
        Endpoint::Path(_) => Err(io::Error::other("sockets Unix não existem aqui")),
        Endpoint::Tcp(host, port) => {
            let connect = tokio::net::TcpStream::connect((host.as_str(), *port));
            let stream = tokio::time::timeout(CONNECT_TIMEOUT, connect)
                .await
                .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "tempo esgotado"))??;
            let _ = stream.set_nodelay(true);
            Ok(Box::new(stream))
        }
    }
}

#[cfg(target_os = "linux")]
fn connect_abstract(name: &str) -> io::Result<Box<dyn Wire>> {
    use std::os::linux::net::SocketAddrExt;
    use std::os::unix::net::{SocketAddr, UnixStream};
    let address = SocketAddr::from_abstract_name(name)?;
    let stream = UnixStream::connect_addr(&address)?;
    stream.set_nonblocking(true)?;
    Ok(Box::new(tokio::net::UnixStream::from_std(stream)?))
}

#[cfg(target_os = "android")]
fn connect_abstract(name: &str) -> io::Result<Box<dyn Wire>> {
    use std::os::android::net::SocketAddrExt;
    use std::os::unix::net::{SocketAddr, UnixStream};
    let address = SocketAddr::from_abstract_name(name)?;
    let stream = UnixStream::connect_addr(&address)?;
    stream.set_nonblocking(true)?;
    Ok(Box::new(tokio::net::UnixStream::from_std(stream)?))
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn connect_abstract(_: &str) -> io::Result<Box<dyn Wire>> {
    Err(io::Error::other("sockets abstratos só existem no Linux"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(family: u16, address: &[u8], number: &str, name: &str, data: &[u8]) -> XauthEntry {
        XauthEntry {
            family,
            address: address.to_vec(),
            number: number.as_bytes().to_vec(),
            name: name.as_bytes().to_vec(),
            data: data.to_vec(),
        }
    }

    /// O arquivo de `entries`, no formato do `Xauthority`.
    pub(crate) fn encode(entries: &[XauthEntry]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for e in entries {
            bytes.extend_from_slice(&e.family.to_be_bytes());
            for field in [&e.address, &e.number, &e.name, &e.data] {
                bytes.extend_from_slice(&(field.len() as u16).to_be_bytes());
                bytes.extend_from_slice(field);
            }
        }
        bytes
    }

    #[test]
    fn o_display_dois_pontos_zero_tenta_o_socket_e_o_tcp() {
        let d = parse_display(":0").unwrap();
        assert_eq!((d.number, d.screen, d.place), (0, 0, Where::Local));
        assert_eq!(
            d.endpoints.last(),
            Some(&Endpoint::Tcp("127.0.0.1".to_owned(), 6000))
        );
        #[cfg(target_os = "linux")]
        {
            assert_eq!(
                d.endpoints[0],
                Endpoint::Abstract("/tmp/.X11-unix/X0".to_owned())
            );
            assert_eq!(
                d.endpoints[1],
                Endpoint::Path(PathBuf::from("/tmp/.X11-unix/X0"))
            );
        }
    }

    #[test]
    fn o_display_com_tela_e_unix() {
        let d = parse_display(":1.2").unwrap();
        assert_eq!((d.number, d.screen), (1, 2));
        let u = parse_display("unix:0").unwrap();
        assert_eq!(u.endpoints, parse_display(":0").unwrap().endpoints);
    }

    #[test]
    fn o_display_com_maquina_vira_tcp() {
        let d = parse_display("localhost:10.0").unwrap();
        assert_eq!(
            d.endpoints,
            vec![Endpoint::Tcp("127.0.0.1".to_owned(), 6010)]
        );
        assert_eq!(d.place, Where::Loopback);
        let d = parse_display("192.168.0.5:0").unwrap();
        assert_eq!(
            d.endpoints,
            vec![Endpoint::Tcp("192.168.0.5".to_owned(), 6000)]
        );
        assert_eq!(d.place, Where::Host("192.168.0.5".to_owned()));
        let d = parse_display("host:2").unwrap();
        assert_eq!(d.endpoints, vec![Endpoint::Tcp("host".to_owned(), 6002)]);
        let d = parse_display("[::1]:0").unwrap();
        assert_eq!(d.endpoints, vec![Endpoint::Tcp("::1".to_owned(), 6000)]);
        assert_eq!(d.place, Where::Loopback);
    }

    #[test]
    fn o_display_do_xquartz_e_o_caminho() {
        let d = parse_display("/private/tmp/com.apple.launchd.xyz/org.xquartz:0").unwrap();
        assert_eq!(
            d.endpoints,
            vec![Endpoint::Path(PathBuf::from(
                "/private/tmp/com.apple.launchd.xyz/org.xquartz"
            ))]
        );
        assert_eq!((d.number, d.place), (0, Where::Local));
    }

    #[test]
    fn o_display_vazio_ou_torto_e_erro() {
        assert!(parse_display("").is_err());
        assert!(parse_display("abc").is_err());
        assert!(parse_display(":x").is_err());
        assert!(parse_display(":70000").is_err());
    }

    #[test]
    fn o_xauthority_lido_ida_e_volta_e_cortado() {
        let entries = vec![
            entry(FAMILY_LOCAL, b"maq", "0", PROTOCOL, &[1; 16]),
            entry(FAMILY_WILD, b"", "1", PROTOCOL, &[2; 16]),
        ];
        let bytes = encode(&entries);
        assert_eq!(read_xauthority(&bytes), entries);
        // Cortado no meio da segunda: vale a primeira.
        assert_eq!(read_xauthority(&bytes[..bytes.len() - 5]), entries[..1]);
        assert!(read_xauthority(&[]).is_empty());
        assert!(read_xauthority(&[0]).is_empty());
    }

    #[test]
    fn o_cookie_acha_pela_familia_e_pelo_numero() {
        let local = parse_display(":0").unwrap();
        let entries = [
            entry(FAMILY_LOCAL, b"outra", "0", PROTOCOL, &[9; 16]),
            entry(FAMILY_LOCAL, b"maq", "1", PROTOCOL, &[8; 16]),
            entry(FAMILY_LOCAL, b"maq", "0", "OUTRO-1", &[7; 16]),
            entry(FAMILY_LOCAL, b"maq", "0", PROTOCOL, &[1; 16]),
            entry(FAMILY_WILD, b"", "0", PROTOCOL, &[2; 16]),
        ];
        assert_eq!(find_cookie(&entries, &local, "maq"), Some(vec![1; 16]));
        // Sem a entrada local do nome da máquina, a Wild serve.
        assert_eq!(find_cookie(&entries, &local, "x"), Some(vec![2; 16]));
        assert_eq!(find_cookie(&entries[..4], &local, "x"), None);
        // Número errado.
        let outro = parse_display(":5").unwrap();
        assert_eq!(find_cookie(&entries[..4], &outro, "maq"), None);
    }

    #[test]
    fn o_cookie_de_um_display_tcp_usa_o_ip() {
        let v4 = parse_display("192.168.0.5:0").unwrap();
        let v6 = parse_display("[fe80::1]:0").unwrap();
        let entries = [
            entry(FAMILY_INTERNET, &[192, 168, 0, 5], "0", PROTOCOL, &[3; 16]),
            entry(
                FAMILY_INTERNET6,
                &std::net::Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1).octets(),
                "0",
                PROTOCOL,
                &[4; 16],
            ),
        ];
        assert_eq!(find_cookie(&entries, &v4, "maq"), Some(vec![3; 16]));
        assert_eq!(find_cookie(&entries, &v6, "maq"), Some(vec![4; 16]));
        // O local com o hostname vale para o localhost, mas não para outra máquina.
        let local = [entry(FAMILY_LOCAL, b"maq", "0", PROTOCOL, &[5; 16])];
        let loop_ = parse_display("localhost:0").unwrap();
        assert_eq!(find_cookie(&local, &loop_, "maq"), Some(vec![5; 16]));
        assert_eq!(find_cookie(&local, &v4, "maq"), None);
    }

    /// Um setup de cliente X11 na ordem `big`, com `name` e `data`.
    fn setup(big: bool, name: &[u8], data: &[u8]) -> Vec<u8> {
        let put = |v: usize| {
            if big {
                (v as u16).to_be_bytes()
            } else {
                (v as u16).to_le_bytes()
            }
        };
        let mut packet = vec![if big { b'B' } else { b'l' }, 0];
        packet.extend_from_slice(&put(11));
        packet.extend_from_slice(&put(0));
        packet.extend_from_slice(&put(name.len()));
        packet.extend_from_slice(&put(data.len()));
        packet.extend_from_slice(&[0, 0]);
        packet.extend_from_slice(name);
        packet.resize(12 + padded(name.len()), 0);
        packet.extend_from_slice(data);
        packet.resize(12 + padded(name.len()) + padded(data.len()), 0);
        packet
    }

    #[test]
    fn o_setup_troca_o_cookie_nas_duas_ordens() {
        let (fake, real) = ([0xAA; 16], [0xBB; 16]);
        for big in [true, false] {
            let original = setup(big, PROTOCOL.as_bytes(), &fake);
            let feito = rewrite_setup(&original, &fake, Some(&real)).unwrap();
            assert_eq!(feito.consumed, original.len());
            assert_eq!(feito.packet, setup(big, PROTOCOL.as_bytes(), &real));
        }
    }

    #[test]
    fn o_setup_sem_cookie_real_vai_com_nome_e_dado_vazios() {
        let fake = [0xAA; 16];
        let original = setup(false, PROTOCOL.as_bytes(), &fake);
        let feito = rewrite_setup(&original, &fake, None).unwrap();
        assert_eq!(feito.packet, setup(false, b"", b""));
        assert_eq!(feito.packet.len(), 12);
    }

    #[test]
    fn o_setup_com_cookies_de_qualquer_tamanho_alinha_em_quatro() {
        let fake = [0xAA; 16];
        for tamanho in 0..=7 {
            let real = vec![0xCC; tamanho];
            let original = setup(true, PROTOCOL.as_bytes(), &fake);
            let feito = rewrite_setup(&original, &fake, Some(&real)).unwrap();
            assert_eq!(feito.packet.len() % 4, 0, "tamanho {tamanho}");
            assert_eq!(feito.packet, setup(true, PROTOCOL.as_bytes(), &real));
        }
    }

    #[test]
    fn o_setup_preserva_os_bytes_a_mais() {
        let fake = [0xAA; 16];
        let mut original = setup(true, PROTOCOL.as_bytes(), &fake);
        let tamanho = original.len();
        original.extend_from_slice(b"resto");
        let feito = rewrite_setup(&original, &fake, Some(&[1; 16])).unwrap();
        assert_eq!(feito.consumed, tamanho);
        assert_eq!(&original[feito.consumed..], b"resto");
    }

    #[test]
    fn o_setup_incompleto_errado_ou_de_ordem_invalida() {
        let fake = [0xAA; 16];
        let original = setup(true, PROTOCOL.as_bytes(), &fake);
        assert_eq!(rewrite_setup(&[], &fake, None), Err(SetupError::Incomplete));
        assert_eq!(
            rewrite_setup(&original[..11], &fake, None),
            Err(SetupError::Incomplete)
        );
        assert_eq!(
            rewrite_setup(&original[..original.len() - 1], &fake, None),
            Err(SetupError::Incomplete)
        );
        assert_eq!(
            rewrite_setup(&setup(true, PROTOCOL.as_bytes(), &[0; 16]), &fake, None),
            Err(SetupError::WrongCookie)
        );
        assert_eq!(
            rewrite_setup(&setup(true, b"OUTRO", &fake), &fake, None),
            Err(SetupError::WrongCookie)
        );
        assert_eq!(
            rewrite_setup(&setup(true, b"", b""), &fake, None),
            Err(SetupError::WrongCookie)
        );
        let mut torto = original.clone();
        torto[0] = b'X';
        assert_eq!(
            rewrite_setup(&torto, &fake, None),
            Err(SetupError::BadByteOrder)
        );
    }

    #[test]
    fn o_cookie_falso_em_hexadecimal() {
        assert_eq!(hex(&[0, 15, 255]), "000fff");
    }
}
