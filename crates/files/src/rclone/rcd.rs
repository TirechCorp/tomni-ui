//! O `rclone rcd`: o processo do rclone que atende a API de controle remoto, e o cliente HTTP
//! que fala com ele.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::io::{self, BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use serde_json::Value;
use ureq::http::{Request, Response};
use ureq::{Agent, Body, SendBody};

/// A versão mais antiga do rclone que o app aceita (a do Ubuntu 24.04).
pub const VERSAO_MINIMA: (u32, u32, u32) = (1, 60, 0);
/// Onde se instala o rclone (para a mensagem de binário ausente).
pub const ENDERECO_DE_INSTALACAO: &str = "https://rclone.org/install/";
/// Quanto o `spawn` espera o rclone dizer em que porta escuta.
const ESPERA_DA_PORTA: Duration = Duration::from_secs(10);
/// Quanto o `shutdown` espera o processo sair antes de matá-lo.
const ESPERA_DO_ENCERRAMENTO: Duration = Duration::from_secs(2);
/// Quantas linhas do stderr do rclone ficam guardadas para as mensagens de erro.
const LINHAS_GUARDADAS: usize = 50;
/// O limite de uma chamada comum.
const LIMITE_DA_CHAMADA: Duration = Duration::from_secs(300);
/// O limite de uma chamada que espera o login no navegador.
pub const LIMITE_DO_LOGIN: Duration = Duration::from_secs(600);

/// O que dá errado ao falar com o rclone.
#[derive(Debug)]
pub enum RcloneError {
    /// O binário não foi encontrado.
    NotFound,
    /// O rclone é mais antigo que o mínimo.
    TooOld((u32, u32, u32)),
    /// O processo não subiu (com o que ele escreveu no stderr).
    Start(String),
    /// A configuração do rclone está cifrada e a senha falta ou está errada.
    ConfigPassword,
    /// O rclone recusou o pedido.
    Api {
        /// O status HTTP.
        status: u16,
        /// O texto do erro do rclone.
        message: String,
    },
    /// A conversa com o processo falhou (rede, resposta que não é JSON…).
    Io(io::Error),
}

impl std::fmt::Display for RcloneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RcloneError::NotFound => write!(
                f,
                "O rclone não foi encontrado. Instale ({ENDERECO_DE_INSTALACAO}) ou indique o caminho nas configurações."
            ),
            RcloneError::TooOld((a, b, c)) => write!(
                f,
                "O rclone {a}.{b}.{c} é antigo; é preciso a versão {}.{} ou mais nova.",
                VERSAO_MINIMA.0, VERSAO_MINIMA.1
            ),
            RcloneError::Start(texto) if texto.is_empty() => {
                f.write_str("Não foi possível iniciar o rclone.")
            }
            RcloneError::Start(texto) => {
                write!(f, "Não foi possível iniciar o rclone: \"{texto}\"")
            }
            RcloneError::ConfigPassword => f.write_str(
                "A configuração do rclone é cifrada e a senha não confere ou não foi informada.",
            ),
            RcloneError::Api { status: 401, .. } => f.write_str(
                "O rclone recusou a autenticação da conversa (usuário ou senha errados).",
            ),
            RcloneError::Api { message, .. } => write!(f, "O rclone recusou: \"{message}\""),
            RcloneError::Io(error) => write!(
                f,
                "Falha na conversa com o rclone: {}",
                tomni_ui_net::error_message(error)
            ),
        }
    }
}

impl std::error::Error for RcloneError {}

impl From<io::Error> for RcloneError {
    fn from(error: io::Error) -> Self {
        RcloneError::Io(error)
    }
}

impl RcloneError {
    /// O `ErrorKind` que a tela entende para este erro.
    pub fn kind(&self) -> io::ErrorKind {
        use io::ErrorKind::*;
        match self {
            RcloneError::NotFound => NotFound,
            RcloneError::TooOld(_) => Unsupported,
            RcloneError::Start(_) => ConnectionRefused,
            RcloneError::ConfigPassword => PermissionDenied,
            RcloneError::Io(error) => error.kind(),
            RcloneError::Api { status, message } => {
                let texto = message.to_lowercase();
                if *status == 404
                    || texto.contains("not found")
                    || texto.contains("didn't find")
                    || texto.contains("doesn't exist")
                    || texto.contains("does not exist")
                {
                    NotFound
                } else if *status == 401
                    || *status == 403
                    || texto.contains("permission denied")
                    || texto.contains("forbidden")
                    || texto.contains("unauthorized")
                {
                    PermissionDenied
                } else if texto.contains("already exists") {
                    AlreadyExists
                } else if texto.contains("directory not empty") {
                    DirectoryNotEmpty
                } else if texto.contains("not a directory") || texto.contains("is a file") {
                    NotADirectory
                } else {
                    Other
                }
            }
        }
    }
}

impl From<RcloneError> for io::Error {
    fn from(error: RcloneError) -> Self {
        match error {
            RcloneError::Io(inner) => inner,
            other => io::Error::new(other.kind(), other.to_string()),
        }
    }
}

/// Como subir o `rclone rcd`.
#[derive(Clone, Debug)]
pub struct RcdConfig {
    /// O binário (veja [`find_rclone`]).
    pub binary: PathBuf,
    /// O arquivo de configuração; `None`: o padrão do rclone (o do `rclone config`).
    pub config_file: Option<PathBuf>,
    /// A senha de uma configuração cifrada (vai em `RCLONE_CONFIG_PASS`).
    pub config_password: Option<String>,
    /// O proxy do rclone (`socks5h://127.0.0.1:1081`, `http://…`); vai nas variáveis `HTTP_PROXY`,
    /// `HTTPS_PROXY` e `ALL_PROXY`.
    pub proxy: Option<String>,
}

impl RcdConfig {
    /// A configuração padrão para `binary`.
    pub fn new(binary: impl Into<PathBuf>) -> Self {
        RcdConfig {
            binary: binary.into(),
            config_file: None,
            config_password: None,
            proxy: None,
        }
    }
}

/// Procura o rclone: o `configured`, se for um arquivo, ou o primeiro `rclone` do `PATH`.
pub fn find_rclone(configured: Option<&Path>) -> Option<PathBuf> {
    if let Some(caminho) = configured
        && !caminho.as_os_str().is_empty()
    {
        return caminho.is_file().then(|| caminho.to_path_buf());
    }
    let path = std::env::var_os("PATH")?;
    let nomes: &[&str] = if cfg!(windows) {
        &["rclone.exe", "rclone.cmd", "rclone.bat"]
    } else {
        &["rclone"]
    };
    std::env::split_paths(&path)
        .flat_map(|pasta| nomes.iter().map(move |nome| pasta.join(nome)))
        .find(|candidato| executavel(candidato))
}

fn executavel(caminho: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(caminho) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Um `rclone rcd` em execução (ou um já existente, ligado por [`Rcd::attach`]).
///
/// Soltar a última cópia do `Arc` encerra o processo.
pub struct Rcd {
    base: String,
    authorization: String,
    agent: Agent,
    child: Mutex<Option<Child>>,
    stderr: Arc<Mutex<VecDeque<String>>>,
}

impl std::fmt::Debug for Rcd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Sem o usuário e a senha.
        f.debug_struct("Rcd").field("base", &self.base).finish()
    }
}

fn agent(limite: Duration) -> Agent {
    let config = Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .max_redirects_will_error(false)
        // O `rcd` é local: nenhum proxy do ambiente entra no caminho.
        .proxy(None)
        .user_agent("tomni-ui")
        .accept_encoding("identity")
        .timeout_connect(Some(Duration::from_secs(5)))
        .timeout_recv_response(Some(limite))
        .build();
    Agent::new_with_config(config)
}

fn sorteio(bytes: usize) -> io::Result<Vec<u8>> {
    let mut buffer = vec![0u8; bytes];
    getrandom::fill(&mut buffer)
        .map_err(|erro| io::Error::other(format!("sem números aleatórios: {erro}")))?;
    Ok(buffer)
}

impl Rcd {
    /// Sobe o `rclone rcd` em `127.0.0.1`, numa porta que o rclone escolhe, com usuário e senha
    /// sorteados, e confere a versão.
    pub fn spawn(config: RcdConfig) -> Result<Arc<Rcd>, RcloneError> {
        if !config.binary.is_file() && find_rclone(Some(&config.binary)).is_none() {
            return Err(RcloneError::NotFound);
        }
        let usuario = URL_SAFE_NO_PAD.encode(sorteio(12)?);
        let senha = STANDARD.encode(sorteio(24)?);

        let mut comando = Command::new(&config.binary);
        comando.args(["rcd", "--rc-addr", "127.0.0.1:0", "--rc-serve"]);
        if let Some(arquivo) = &config.config_file {
            let mut argumento = OsString::from("--config=");
            argumento.push(arquivo);
            comando.arg(argumento);
        }
        // Pelo ambiente, não pela linha de comando: o `ps` não mostra a senha.
        comando.env("RCLONE_RC_USER", &usuario);
        comando.env("RCLONE_RC_PASS", &senha);
        if let Some(senha) = &config.config_password {
            comando.env("RCLONE_CONFIG_PASS", senha);
        }
        if let Some(proxy) = &config.proxy {
            for nome in [
                "HTTP_PROXY",
                "HTTPS_PROXY",
                "ALL_PROXY",
                "http_proxy",
                "https_proxy",
                "all_proxy",
            ] {
                comando.env(nome, proxy);
            }
            for nome in ["NO_PROXY", "no_proxy"] {
                comando.env(nome, "127.0.0.1,localhost");
            }
        }
        // No Windows, sem a janela de console que o rclone abriria ao lado do app (M65).
        tomni_ui_core::process::no_window(&mut comando)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut filho = comando.spawn().map_err(|erro| match erro.kind() {
            io::ErrorKind::NotFound => RcloneError::NotFound,
            _ => RcloneError::Start(tomni_ui_net::error_message(&erro)),
        })?;

        let stderr = Arc::new(Mutex::new(VecDeque::new()));
        let (porta_tx, porta_rx) = mpsc::channel::<u16>();
        if let Some(saida) = filho.stderr.take() {
            let guardadas = Arc::clone(&stderr);
            std::thread::spawn(move || ler_stderr(saida, &guardadas, &porta_tx));
        }
        let porta = match porta_rx.recv_timeout(ESPERA_DA_PORTA) {
            Ok(porta) => porta,
            Err(motivo) => {
                // A thread de leitura termina quando o processo fecha o stderr.
                let _ = filho.kill();
                let _ = filho.wait();
                std::thread::sleep(Duration::from_millis(50));
                let texto = ultimas_linhas(&stderr);
                if pede_senha_da_configuracao(&texto) {
                    return Err(RcloneError::ConfigPassword);
                }
                return Err(RcloneError::Start(match motivo {
                    mpsc::RecvTimeoutError::Timeout if texto.is_empty() => {
                        "o rclone não disse em que porta escuta".to_owned()
                    }
                    _ => texto,
                }));
            }
        };
        let rcd = Rcd {
            base: format!("http://127.0.0.1:{porta}"),
            authorization: autorizacao(&usuario, &senha),
            agent: agent(LIMITE_DA_CHAMADA),
            child: Mutex::new(Some(filho)),
            stderr,
        };
        let rcd = Arc::new(rcd);
        // Se a versão não serve, o `Drop` do `rcd` encerra o processo.
        rcd.check_version()?;
        // O rclone só lê a configuração no primeiro uso. Com a configuração cifrada e a senha
        // faltando ou errada, ele pede a senha no terminal, não encontra e sai: aqui se vê.
        if let Err(erro) = rcd.call("config/listremotes", Value::Object(Default::default())) {
            std::thread::sleep(Duration::from_millis(150));
            let texto = rcd.log();
            if pede_senha_da_configuracao(&texto) {
                return Err(RcloneError::ConfigPassword);
            }
            return Err(match erro {
                RcloneError::Io(_) => RcloneError::Start(texto),
                outro => outro,
            });
        }
        Ok(rcd)
    }

    /// Liga a um `rcd` que já existe (os testes, ou um rclone que a pessoa subiu). Não é dono
    /// do processo: `shutdown` não faz nada.
    pub fn attach(url: &str, user: &str, password: &str) -> Arc<Rcd> {
        Arc::new(Rcd {
            base: url.trim_end_matches('/').to_owned(),
            authorization: autorizacao(user, password),
            agent: agent(LIMITE_DA_CHAMADA),
            child: Mutex::new(None),
            stderr: Arc::new(Mutex::new(VecDeque::new())),
        })
    }

    /// O endereço do `rcd` (`http://127.0.0.1:PORTA`).
    pub fn url(&self) -> &str {
        &self.base
    }

    /// As últimas linhas que o rclone escreveu no stderr.
    pub fn log(&self) -> String {
        ultimas_linhas(&self.stderr)
    }

    /// `POST /{method}` com `input` em JSON; devolve o JSON da resposta.
    pub fn call(&self, method: &str, input: Value) -> Result<Value, RcloneError> {
        self.call_with(&self.agent, method, input)
    }

    /// Como [`call`](Rcd::call), com o limite de [`LIMITE_DO_LOGIN`]: para a chamada que espera
    /// o login no navegador.
    pub fn call_login(&self, method: &str, input: Value) -> Result<Value, RcloneError> {
        self.call_with(&agent(LIMITE_DO_LOGIN), method, input)
    }

    fn call_with(&self, agent: &Agent, method: &str, input: Value) -> Result<Value, RcloneError> {
        let corpo = serde_json::to_vec(&input)
            .map_err(|erro| RcloneError::Io(io::Error::other(erro.to_string())))?;
        let mut resposta = self.send(
            agent,
            "POST",
            &format!("/{method}"),
            &[("Content-Type", "application/json")],
            SendBody::from_owned_reader(io::Cursor::new(corpo)),
        )?;
        let texto = ler_texto(&mut resposta)?;
        if !resposta.status().is_success() {
            return Err(erro_da_api(resposta.status().as_u16(), &texto));
        }
        if texto.trim().is_empty() {
            return Ok(Value::Object(Default::default()));
        }
        serde_json::from_str(&texto).map_err(|erro| {
            RcloneError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("resposta inválida: {erro}"),
            ))
        })
    }

    /// Um pedido cru ao `rcd` (o envio em fluxo e a leitura com faixa usam).
    pub(crate) fn send(
        &self,
        agent: &Agent,
        method: &str,
        path_and_query: &str,
        headers: &[(&str, &str)],
        body: SendBody<'_>,
    ) -> Result<Response<Body>, RcloneError> {
        let mut builder = Request::builder()
            .method(method)
            .uri(format!("{}{path_and_query}", self.base))
            .header("Authorization", self.authorization.as_str());
        for (nome, valor) in headers {
            builder = builder.header(*nome, *valor);
        }
        let pedido = builder
            .body(body)
            .map_err(|erro| RcloneError::Io(io::Error::other(erro.to_string())))?;
        agent.run(pedido).map_err(erro_de_transporte)
    }

    /// O agente comum (as chamadas de fluxo usam o mesmo).
    pub(crate) fn agent(&self) -> &Agent {
        &self.agent
    }

    /// O erro de uma resposta que não deu certo.
    pub(crate) fn erro_da_resposta(&self, resposta: &mut Response<Body>) -> RcloneError {
        let status = resposta.status().as_u16();
        let texto = ler_texto(resposta).unwrap_or_default();
        erro_da_api(status, &texto)
    }

    /// A versão do rclone.
    pub fn version(&self) -> Result<(u32, u32, u32), RcloneError> {
        let resposta = self.call("core/version", Value::Object(Default::default()))?;
        let partes = resposta
            .get("decomposed")
            .and_then(Value::as_array)
            .map(|v| {
                v.iter()
                    .map(|n| n.as_u64().unwrap_or(0) as u32)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        match partes.as_slice() {
            [a, b, c, ..] => Ok((*a, *b, *c)),
            _ => Err(RcloneError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                "o rclone não disse a versão",
            ))),
        }
    }

    /// A versão, ou [`RcloneError::TooOld`] se for mais antiga que [`VERSAO_MINIMA`].
    pub fn check_version(&self) -> Result<(u32, u32, u32), RcloneError> {
        let versao = self.version()?;
        if versao < VERSAO_MINIMA {
            return Err(RcloneError::TooOld(versao));
        }
        Ok(versao)
    }

    /// Encerra o processo: `core/quit`, espera 2 s e depois mata. Pode ser chamado de novo.
    pub fn shutdown(&self) {
        let filho = self
            .child
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let Some(mut filho) = filho else {
            return;
        };
        let curto = agent(Duration::from_secs(1));
        let _ = self.call_with(&curto, "core/quit", Value::Object(Default::default()));
        let limite = Instant::now() + ESPERA_DO_ENCERRAMENTO;
        loop {
            match filho.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) if Instant::now() < limite => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                _ => break,
            }
        }
        let _ = filho.kill();
        let _ = filho.wait();
    }
}

impl Drop for Rcd {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn autorizacao(usuario: &str, senha: &str) -> String {
    format!("Basic {}", STANDARD.encode(format!("{usuario}:{senha}")))
}

fn ler_stderr(saida: impl Read, guardadas: &Mutex<VecDeque<String>>, porta: &mpsc::Sender<u16>) {
    const MARCA: &str = "Serving remote control on http://";
    let mut enviou = false;
    for linha in BufReader::new(saida).lines().map_while(Result::ok) {
        if !enviou
            && let Some((_, resto)) = linha.split_once(MARCA)
            && let Some(numero) = resto
                .split(['/', ' '])
                .next()
                .and_then(|endereco| endereco.rsplit(':').next())
                .and_then(|numero| numero.parse::<u16>().ok())
        {
            enviou = true;
            let _ = porta.send(numero);
        }
        let mut fila = guardadas.lock().unwrap_or_else(PoisonError::into_inner);
        if fila.len() == LINHAS_GUARDADAS {
            fila.pop_front();
        }
        fila.push_back(linha);
    }
}

fn ultimas_linhas(guardadas: &Mutex<VecDeque<String>>) -> String {
    let fila = guardadas.lock().unwrap_or_else(PoisonError::into_inner);
    fila.iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("\n")
}

fn pede_senha_da_configuracao(texto: &str) -> bool {
    let texto = texto.to_lowercase();
    texto.contains("configuration password")
        || texto.contains("config file is encrypted")
        || texto.contains("unable to decrypt configuration")
        || texto.contains("most likely wrong password")
}

fn ler_texto(resposta: &mut Response<Body>) -> Result<String, RcloneError> {
    resposta
        .body_mut()
        .with_config()
        .limit(256 << 20)
        .lossy_utf8(true)
        .read_to_string()
        .map_err(erro_de_transporte)
}

fn erro_da_api(status: u16, texto: &str) -> RcloneError {
    let mensagem = serde_json::from_str::<Value>(texto)
        .ok()
        .and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_owned))
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| {
            if texto.trim().is_empty() {
                format!("HTTP {status}")
            } else {
                texto.trim().chars().take(300).collect()
            }
        });
    RcloneError::Api {
        status,
        message: mensagem,
    }
}

fn erro_de_transporte(erro: ureq::Error) -> RcloneError {
    use ureq::Error;
    RcloneError::Io(match erro {
        Error::Io(erro) => erro,
        Error::Timeout(_) => {
            io::Error::new(io::ErrorKind::TimedOut, "o rclone não respondeu a tempo")
        }
        Error::ConnectionFailed | Error::HostNotFound => io::Error::new(
            io::ErrorKind::ConnectionRefused,
            "não foi possível falar com o rclone",
        ),
        outro => io::Error::other(outro.to_string()),
    })
}
