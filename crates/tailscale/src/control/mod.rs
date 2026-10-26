//! O cliente do plano de controle: busca a chave do servidor (`/key`), abre o canal Noise
//! (`/ts2021`), fala HTTP/2 por cima dele e registra o nó (`/machine/register`).
//!
//! Por padrão nada sai do app além desta conexão com o servidor que você escolheu.

pub mod map;
pub mod noise;

use std::fmt;
use std::time::Duration;

use bytes::Bytes;
use h2::client::SendRequest;
use jiff::Timestamp;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::task::JoinHandle;
use zeroize::Zeroizing;

use crate::key::{MachineKey, MachinePublic, NodePublic};
use crate::tailcfg::{
    CAPABILITY_VERSION, Hostinfo, MapRequest, RegisterAuth, RegisterRequest, RegisterResponse,
    tempo_zero,
};
use crate::upgrade::{self, Roots, Target, UpgradeError};
use noise::{EarlyHandle, NoiseError, NoiseStream};

/// O protocolo do pedido de upgrade.
const PROTOCOLO: &str = "tailscale-control-protocol";
/// O cabeçalho que leva a iniciação do handshake.
const CABECALHO_HANDSHAKE: &str = "X-Tailscale-Handshake";
/// O maior corpo de resposta de um pedido comum.
const MAIOR_RESPOSTA: usize = 1024 * 1024;
/// O tempo de um pedido de registro comum.
const TEMPO_PEDIDO: Duration = Duration::from_secs(60);
/// O tempo de cada espera pelo login no navegador antes de repetir o pedido.
const TEMPO_ESPERA_LOGIN: Duration = Duration::from_secs(5 * 60);

/// O que pode dar errado falando com o controle.
#[derive(Debug)]
pub enum ControlError {
    /// A URL do servidor não serve.
    Url(String),
    /// Não foi possível falar com o servidor.
    Unreachable {
        /// A URL do servidor.
        url: String,
        /// O motivo.
        source: UpgradeError,
    },
    /// O servidor respondeu, mas não fala o protocolo do Tailscale.
    NotTailscale {
        /// A URL do servidor.
        url: String,
        /// O status que o `/key` deu.
        status: u16,
    },
    /// O servidor recusou a versão do cliente.
    VersionRejected {
        /// A versão de capacidade declarada.
        version: u32,
    },
    /// O canal Noise falhou.
    Noise(NoiseError),
    /// O HTTP/2 por cima do canal falhou.
    Http2(String),
    /// O servidor respondeu HTTP com erro.
    Http {
        /// O código.
        status: u16,
        /// O começo do corpo.
        body: String,
    },
    /// Muitos pedidos: o servidor pediu para esperar.
    RateLimited {
        /// Quantos segundos esperar, se ele disse.
        retry_after: Option<u64>,
    },
    /// O servidor mandou um JSON que não foi entendido.
    Json(String),
    /// A chave de autenticação foi recusada.
    AuthKeyRejected(String),
    /// O servidor recusou o registro por outro motivo.
    Rejected(String),
    /// A tailnet usa o Tailnet Lock.
    TailnetLock,
    /// O pedido demorou demais.
    Timeout,
    /// Um quadro do mapa veio corrompido (tamanho absurdo, `zstd` ou JSON inválido).
    Frame(String),
    /// O servidor ficou em silêncio além do que o sinal de vida permite.
    Silent,
}

impl fmt::Display for ControlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ControlError::Url(m) => write!(f, "{m}"),
            ControlError::Unreachable { url, source } => {
                write!(
                    f,
                    "não foi possível falar com o servidor de controle {url}: {source}"
                )
            }
            ControlError::NotTailscale { url, status } => write!(
                f,
                "o servidor {url} não fala o protocolo do Tailscale (/key respondeu {status})"
            ),
            ControlError::VersionRejected { version } => write!(
                f,
                "o servidor recusou esta versão do cliente (capacidade {version}); atualize o \
                 Tirech Omni"
            ),
            ControlError::Noise(e) => write!(f, "{e}"),
            ControlError::Http2(m) => write!(f, "a conversa HTTP/2 com o servidor falhou: {m}"),
            ControlError::Http { status, body } => {
                write!(f, "o servidor de controle respondeu HTTP {status}")?;
                if !body.is_empty() {
                    write!(f, ": {body}")?;
                }
                Ok(())
            }
            ControlError::RateLimited { retry_after } => {
                f.write_str("o servidor pediu para esperar um pouco antes de tentar de novo")?;
                if let Some(s) = retry_after {
                    write!(f, " ({s} s)")?;
                }
                Ok(())
            }
            ControlError::Json(m) => write!(f, "a resposta do servidor não foi entendida: {m}"),
            ControlError::AuthKeyRejected(m) => {
                write!(f, "a chave de autenticação foi recusada: {m}")
            }
            ControlError::Rejected(m) => write!(f, "o servidor recusou o registro: {m}"),
            ControlError::TailnetLock => {
                f.write_str("esta tailnet usa o Tailnet Lock, que o Tirech Omni ainda não suporta")
            }
            ControlError::Timeout => {
                f.write_str("o servidor de controle demorou demais a responder")
            }
            ControlError::Frame(m) => write!(f, "o mapa que o servidor mandou veio estragado: {m}"),
            ControlError::Silent => {
                f.write_str("o servidor de controle parou de mandar o mapa e os sinais de vida")
            }
        }
    }
}

impl std::error::Error for ControlError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ControlError::Unreachable { source, .. } => Some(source),
            ControlError::Noise(e) => Some(e),
            _ => None,
        }
    }
}

impl ControlError {
    /// Se vale tentar de novo depois de um tempo (rede, servidor ocupado) ou se o erro é
    /// definitivo (chave recusada, versão recusada).
    pub fn is_transient(&self) -> bool {
        match self {
            ControlError::Unreachable { source, .. } => !matches!(
                source,
                UpgradeError::Url(_) | UpgradeError::Certificate { .. }
            ),
            ControlError::Noise(NoiseError::Io(_)) => true,
            ControlError::Http2(_)
            | ControlError::Timeout
            | ControlError::Frame(_)
            | ControlError::Silent
            | ControlError::RateLimited { .. } => true,
            ControlError::Http { status, .. } => *status >= 500,
            _ => false,
        }
    }
}

/// A espera crescente entre tentativas: 1 s, 2 s, 4 s… até 60 s.
#[derive(Debug, Clone)]
pub struct Backoff {
    base: Duration,
    teto: Duration,
    atual: Duration,
}

impl Backoff {
    /// Começa em `base` e dobra até `teto`.
    pub fn new(base: Duration, teto: Duration) -> Self {
        Backoff {
            base,
            teto,
            atual: base,
        }
    }

    /// A espera da próxima tentativa (e já dobra a seguinte).
    pub fn next_delay(&mut self) -> Duration {
        let espera = self.atual;
        self.atual = (self.atual * 2).min(self.teto);
        espera
    }

    /// Volta ao começo, depois de uma conexão que deu certo.
    pub fn reset(&mut self) {
        self.atual = self.base;
    }
}

impl Default for Backoff {
    fn default() -> Self {
        // O mesmo ritmo do cliente oficial: dobra a cada falha, até um minuto.
        Backoff::new(Duration::from_secs(1), Duration::from_secs(60))
    }
}

/// O que se configura para falar com o controle.
#[derive(Debug, Clone)]
pub struct ControlConfig {
    /// A URL do servidor (`https://controlplane.tailscale.com` ou a do Headscale).
    pub url: String,
    /// Em que CAs confiar, além das do sistema.
    pub roots: Roots,
    /// A espera entre tentativas.
    pub backoff: Backoff,
}

impl ControlConfig {
    /// A configuração para uma URL, com as raízes do sistema.
    pub fn new(url: impl Into<String>) -> Self {
        ControlConfig {
            url: url.into(),
            roots: Roots::system(),
            backoff: Backoff::default(),
        }
    }
}

/// A chave pública do servidor, dada por `GET /key?v=`.
#[derive(Debug, Clone, serde::Deserialize)]
struct RespostaChave {
    #[serde(rename = "publicKey")]
    public_key: MachinePublic,
}

/// Quem faz pedidos numa conexão ts2021: pode ser copiado e usado ao mesmo tempo que um
/// pedido longo (o do mapa) está aberto na mesma conexão.
#[derive(Clone)]
pub struct Requester {
    envio: SendRequest<Bytes>,
    autoridade: String,
}

impl Requester {
    /// Faz um `POST` com corpo JSON e lê a resposta JSON.
    pub async fn post_json<Q: Serialize, R: DeserializeOwned>(
        &self,
        caminho: &str,
        pedido: &Q,
        extras: &[(&str, String)],
        limite: Duration,
    ) -> Result<R, ControlError> {
        let corpo = serde_json::to_vec(pedido).map_err(|e| ControlError::Json(e.to_string()))?;
        tokio::time::timeout(limite, self.post_cru(caminho, corpo, extras))
            .await
            .map_err(|_| ControlError::Timeout)?
            .and_then(|bytes| {
                serde_json::from_slice(&bytes).map_err(|e| ControlError::Json(e.to_string()))
            })
    }

    /// Manda o pedido (cabeçalho e corpo) e espera o cabeçalho da resposta.
    async fn enviar(
        &self,
        caminho: &str,
        corpo: Vec<u8>,
        extras: &[(&str, String)],
    ) -> Result<http::Response<h2::RecvStream>, ControlError> {
        let h2 = |e: h2::Error| ControlError::Http2(e.to_string());
        let mut req = http::Request::builder()
            .method("POST")
            .uri(format!("https://{}{caminho}", self.autoridade))
            .header("content-type", "application/json");
        for (n, v) in extras {
            req = req.header(*n, v.as_str());
        }
        let req = req
            .body(())
            .map_err(|e| ControlError::Http2(e.to_string()))?;
        let mut envio = self.envio.clone().ready().await.map_err(h2)?;
        let (resposta, mut corpo_envio) = envio.send_request(req, false).map_err(h2)?;
        corpo_envio
            .send_data(Bytes::from(corpo), true)
            .map_err(h2)?;
        resposta.await.map_err(h2)
    }

    /// Faz um `POST` com corpo JSON e só confere que o servidor respondeu 200 (a resposta, se
    /// houver, é descartada).
    pub async fn post_ignored<Q: Serialize>(
        &self,
        caminho: &str,
        pedido: &Q,
        limite: Duration,
    ) -> Result<(), ControlError> {
        let corpo = serde_json::to_vec(pedido).map_err(|e| ControlError::Json(e.to_string()))?;
        tokio::time::timeout(limite, self.post_cru(caminho, corpo, &[]))
            .await
            .map_err(|_| ControlError::Timeout)?
            .map(|_| ())
    }

    async fn post_cru(
        &self,
        caminho: &str,
        corpo: Vec<u8>,
        extras: &[(&str, String)],
    ) -> Result<Vec<u8>, ControlError> {
        let h2 = |e: h2::Error| ControlError::Http2(e.to_string());
        let resposta = self.enviar(caminho, corpo, extras).await?;
        let status = resposta.status().as_u16();
        let retry_after = retry_after(&resposta);
        let mut recebido = resposta.into_body();
        let mut bytes = Vec::new();
        while let Some(pedaco) = recebido.data().await {
            let pedaco = pedaco.map_err(h2)?;
            let _ = recebido.flow_control().release_capacity(pedaco.len());
            if bytes.len() + pedaco.len() > MAIOR_RESPOSTA {
                return Err(ControlError::Http2("a resposta é grande demais".into()));
            }
            bytes.extend_from_slice(&pedaco);
        }
        resultado_http(status, retry_after, bytes)
    }

    /// Abre o pedido longo do mapa (`POST /machine/map` com `Stream`) e devolve o fluxo de
    /// respostas, quadro a quadro.
    pub async fn open_map(
        &self,
        pedido: &MapRequest,
        limite: Duration,
    ) -> Result<map::MapStream, ControlError> {
        let corpo = serde_json::to_vec(pedido).map_err(|e| ControlError::Json(e.to_string()))?;
        let resposta = tokio::time::timeout(limite, self.enviar("/machine/map", corpo, &[]))
            .await
            .map_err(|_| ControlError::Timeout)??;
        let status = resposta.status().as_u16();
        if status != 200 {
            let retry_after = retry_after(&resposta);
            let mut recebido = resposta.into_body();
            let mut bytes = Vec::new();
            while let Some(Ok(pedaco)) = recebido.data().await {
                let _ = recebido.flow_control().release_capacity(pedaco.len());
                bytes.extend_from_slice(&pedaco);
                if bytes.len() > 4096 {
                    break;
                }
            }
            return Err(erro_http(status, retry_after, bytes));
        }
        Ok(map::MapStream::new(resposta.into_body()))
    }
}

fn retry_after(resposta: &http::Response<h2::RecvStream>) -> Option<u64> {
    resposta
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
}

/// Transforma o status e o corpo da resposta no resultado do pedido comum.
fn resultado_http(
    status: u16,
    retry_after: Option<u64>,
    bytes: Vec<u8>,
) -> Result<Vec<u8>, ControlError> {
    match status {
        200 => Ok(bytes),
        _ => Err(erro_http(status, retry_after, bytes)),
    }
}

/// O erro de uma resposta que não foi 200.
fn erro_http(status: u16, retry_after: Option<u64>, bytes: Vec<u8>) -> ControlError {
    if status == 429 {
        return ControlError::RateLimited { retry_after };
    }
    ControlError::Http {
        status,
        body: String::from_utf8_lossy(&bytes)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(200)
            .collect(),
    }
}

/// Uma conexão ts2021 aberta, com o HTTP/2 por cima.
pub struct Connection {
    pedidos: Requester,
    early: EarlyHandle,
    motor: JoinHandle<()>,
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.motor.abort();
    }
}

impl Connection {
    /// A carga antecipada do servidor, se já chegou.
    pub fn early(&self) -> &EarlyHandle {
        &self.early
    }

    /// Quem faz pedidos nesta conexão (uma cópia barata).
    pub fn requester(&self) -> Requester {
        self.pedidos.clone()
    }

    /// Se a conexão ainda está de pé.
    pub fn is_open(&self) -> bool {
        !self.motor.is_finished()
    }

    /// Faz um `POST` com corpo JSON e lê a resposta JSON.
    pub async fn post_json<Q: Serialize, R: DeserializeOwned>(
        &mut self,
        caminho: &str,
        pedido: &Q,
        extras: &[(&str, String)],
        limite: Duration,
    ) -> Result<R, ControlError> {
        self.pedidos
            .post_json(caminho, pedido, extras, limite)
            .await
    }
}

/// O que o registro resultou.
#[derive(Debug, Clone, PartialEq)]
pub enum RegisterOutcome {
    /// A máquina entrou na tailnet.
    Authorized(RegisterResponse),
    /// O usuário precisa abrir esta URL no navegador para entrar.
    NeedsLogin {
        /// A URL de login.
        url: String,
    },
    /// O login foi feito, mas o administrador da tailnet ainda precisa aprovar a máquina.
    NeedsApproval(RegisterResponse),
    /// O servidor diz que a chave de nó venceu: gere outra e registre de novo.
    KeyExpired,
}

/// O que se pede no registro.
pub struct RegisterParams {
    /// A chave pública do nó que se registra.
    pub node_key: NodePublic,
    /// A chave pública anterior, ao trocar de chave de nó.
    pub old_node_key: Option<NodePublic>,
    /// A chave de autenticação, quando se tem uma.
    pub auth_key: Option<Zeroizing<String>>,
    /// O que a máquina conta de si.
    pub hostinfo: Hostinfo,
    /// Se a máquina é efêmera.
    pub ephemeral: bool,
}

impl fmt::Debug for RegisterParams {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RegisterParams")
            .field("node_key", &self.node_key)
            .field("old_node_key", &self.old_node_key)
            .field("auth_key", &self.auth_key.as_ref().map(|_| "***"))
            .field("hostinfo", &self.hostinfo)
            .field("ephemeral", &self.ephemeral)
            .finish()
    }
}

/// O cliente do controle: guarda a chave da máquina, a do servidor e a conexão atual.
pub struct Control {
    config: ControlConfig,
    alvo: Target,
    maquina: MachineKey,
    servidor: Option<MachinePublic>,
    conexao: Option<Connection>,
    backoff: Backoff,
}

impl Control {
    /// Prepara o cliente; ainda não conecta.
    pub fn new(config: ControlConfig, maquina: MachineKey) -> Result<Self, ControlError> {
        let alvo = Target::parse(&config.url).map_err(|e| ControlError::Url(e.to_string()))?;
        let backoff = config.backoff.clone();
        Ok(Control {
            config,
            alvo,
            maquina,
            servidor: None,
            conexao: None,
            backoff,
        })
    }

    fn inalcancavel(&self, source: UpgradeError) -> ControlError {
        ControlError::Unreachable {
            url: self.config.url.clone(),
            source,
        }
    }

    /// A chave pública do servidor (`GET /key?v=`), guardada depois da primeira vez.
    pub async fn server_key(&mut self) -> Result<MachinePublic, ControlError> {
        if let Some(k) = self.servidor {
            return Ok(k);
        }
        let caminho = format!("/key?v={CAPABILITY_VERSION}");
        let corpo = match upgrade::get(&self.alvo, &self.config.roots, &caminho).await {
            Ok(c) => c,
            Err(UpgradeError::Status { status, .. }) => {
                return Err(ControlError::NotTailscale {
                    url: self.config.url.clone(),
                    status,
                });
            }
            Err(e) => return Err(self.inalcancavel(e)),
        };
        let r: RespostaChave = serde_json::from_slice(&corpo).map_err(|_| {
            // Responde 200, mas não é o `/key` do Tailscale (uma página qualquer).
            ControlError::NotTailscale {
                url: self.config.url.clone(),
                status: 200,
            }
        })?;
        self.servidor = Some(r.public_key);
        Ok(r.public_key)
    }

    /// Abre a conexão ts2021 se ainda não houver uma.
    pub async fn connect(&mut self) -> Result<(), ControlError> {
        if let Some(c) = &self.conexao
            && c.is_open()
        {
            return Ok(());
        }
        self.conexao = None;
        let servidor = self.server_key().await?;
        let versao = CAPABILITY_VERSION as u16;
        let (inic, msg) = noise::start_client(&self.maquina, &servidor, versao, None)
            .map_err(ControlError::Noise)?;
        let b64 = noise::iniciacao_em_base64(&msg);
        let mut fluxo = match upgrade::upgrade(
            &self.alvo,
            &self.config.roots,
            "/ts2021",
            PROTOCOLO,
            &[(CABECALHO_HANDSHAKE, b64.as_str())],
        )
        .await
        {
            Ok(f) => f,
            Err(UpgradeError::Status { status: 426, .. }) => {
                return Err(ControlError::VersionRejected {
                    version: CAPABILITY_VERSION,
                });
            }
            Err(e) => return Err(self.inalcancavel(e)),
        };
        let resposta = match noise::read_response(&mut fluxo).await {
            Ok(r) => r,
            Err(NoiseError::Server(t)) if t.to_ascii_lowercase().contains("version") => {
                return Err(ControlError::VersionRejected {
                    version: CAPABILITY_VERSION,
                });
            }
            Err(e) => return Err(ControlError::Noise(e)),
        };
        let canal = inic.finish(&resposta).map_err(ControlError::Noise)?;
        let noise = NoiseStream::client(fluxo, canal);
        let early = noise.early();
        let (envio, conexao) = h2::client::handshake(noise)
            .await
            .map_err(|e| ControlError::Http2(e.to_string()))?;
        let motor = tokio::spawn(async move {
            // O resultado não importa: quando a conexão cai, o `SendRequest` devolve o erro.
            let _ = conexao.await;
        });
        self.conexao = Some(Connection {
            pedidos: Requester {
                envio,
                autoridade: self.alvo.authority(),
            },
            early,
            motor,
        });
        Ok(())
    }

    /// A conexão aberta (abre se preciso).
    pub async fn connection(&mut self) -> Result<&mut Connection, ControlError> {
        self.connect().await?;
        Ok(self.conexao.as_mut().expect("acabou de conectar"))
    }

    /// Se há uma conexão de pé.
    pub fn is_connected(&self) -> bool {
        self.conexao.as_ref().is_some_and(Connection::is_open)
    }

    /// Larga a conexão atual (a próxima chamada abre outra).
    pub fn disconnect(&mut self) {
        self.conexao = None;
    }

    /// Faz **um** pedido de registro.
    ///
    /// `followup` é a URL de login do primeiro pedido, quando se está esperando o usuário
    /// entrar pelo navegador; nesse caso a resposta só vem depois do login.
    pub async fn register(
        &mut self,
        params: &RegisterParams,
        followup: Option<&str>,
    ) -> Result<RegisterOutcome, ControlError> {
        self.register_com_validade(params, followup, tempo_zero())
            .await
    }

    /// Sai da tailnet: pede ao controle que a chave de nó venza agora (o `Expiry` no passado,
    /// como o cliente oficial faz no `logout`). A chave não serve mais; para entrar de novo
    /// é preciso uma chave de nó nova.
    pub async fn logout(&mut self, params: &RegisterParams) -> Result<(), ControlError> {
        let passado = Timestamp::from_second(123).expect("data válida");
        match self.register_com_validade(params, None, passado).await {
            Ok(_) | Err(ControlError::AuthKeyRejected(_) | ControlError::Rejected(_)) => Ok(()),
            Err(e) => Err(e),
        }
    }

    async fn register_com_validade(
        &mut self,
        params: &RegisterParams,
        followup: Option<&str>,
        validade: Timestamp,
    ) -> Result<RegisterOutcome, ControlError> {
        let pedido = RegisterRequest {
            version: CAPABILITY_VERSION,
            node_key: params.node_key,
            old_node_key: params.old_node_key.unwrap_or_default(),
            nl_key: Default::default(),
            auth: params.auth_key.as_ref().map(|k| RegisterAuth {
                auth_key: k.as_str().to_string(),
            }),
            expiry: validade,
            followup: followup.unwrap_or_default().to_string(),
            hostinfo: Some(params.hostinfo.clone()),
            ephemeral: params.ephemeral,
            node_key_signature: None,
            timestamp: Some(jiff::Timestamp::now()),
        };
        let mut lb = vec![("ts-lb", params.node_key.to_string())];
        if let Some(velha) = params.old_node_key {
            lb.push(("ts-lb", velha.to_string()));
        }
        let limite = if followup.is_some() {
            TEMPO_ESPERA_LOGIN
        } else {
            TEMPO_PEDIDO
        };
        let resposta: Result<RegisterResponse, ControlError> = {
            let conexao = self.connection().await?;
            conexao
                .post_json("/machine/register", &pedido, &lb[..1], limite)
                .await
        };
        let resposta = match resposta {
            Ok(r) => r,
            Err(e) => {
                if !matches!(e, ControlError::Json(_)) {
                    // Pode ter sido a conexão que caiu: a próxima tentativa abre outra.
                    self.disconnect();
                }
                return Err(e);
            }
        };
        interpretar(resposta, params.auth_key.is_some())
    }

    /// O registro inteiro: repete com espera crescente quando a rede falha e, se o servidor
    /// pedir o login no navegador, chama `ao_pedir_login` com a URL e espera o usuário
    /// entrar (repetindo o pedido com o mesmo `Followup` se ele cair por tempo).
    ///
    /// Só volta com um erro definitivo ou com o resultado; para desistir, largue o futuro.
    pub async fn register_flow(
        &mut self,
        params: &RegisterParams,
        ao_pedir_login: &mut (dyn FnMut(&str) + Send),
    ) -> Result<RegisterOutcome, ControlError> {
        let mut url_de_login: Option<String> = None;
        loop {
            let tentativa = self.register(params, url_de_login.as_deref()).await;
            match tentativa {
                Ok(RegisterOutcome::NeedsLogin { url }) => {
                    if url_de_login.as_deref() == Some(url.as_str()) {
                        // O servidor devolveu a mesma URL de novo: o login ainda não saiu.
                        tokio::time::sleep(self.backoff.next_delay()).await;
                    } else {
                        self.backoff.reset();
                        ao_pedir_login(&url);
                        url_de_login = Some(url);
                    }
                }
                Ok(outro) => {
                    self.backoff.reset();
                    return Ok(outro);
                }
                // A espera pelo login estourou o tempo: repete o mesmo pedido, sem espera.
                Err(ControlError::Timeout) if url_de_login.is_some() => {}
                Err(e) if e.is_transient() => {
                    let espera = self.backoff.next_delay();
                    tokio::time::sleep(espera).await;
                }
                Err(e) => return Err(e),
            }
        }
    }
}

/// Traduz a resposta do registro no que o motor precisa saber.
fn interpretar(r: RegisterResponse, com_chave: bool) -> Result<RegisterOutcome, ControlError> {
    if !r.error.is_empty() {
        return Err(if com_chave {
            ControlError::AuthKeyRejected(r.error)
        } else {
            ControlError::Rejected(r.error)
        });
    }
    if r.node_key_signature
        .as_deref()
        .is_some_and(|s| !s.is_empty())
    {
        return Err(ControlError::TailnetLock);
    }
    if r.node_key_expired {
        return Ok(RegisterOutcome::KeyExpired);
    }
    if !r.auth_url.is_empty() {
        return Ok(RegisterOutcome::NeedsLogin { url: r.auth_url });
    }
    if r.machine_authorized {
        Ok(RegisterOutcome::Authorized(r))
    } else {
        Ok(RegisterOutcome::NeedsApproval(r))
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn a_espera_dobra_ate_o_teto_e_volta() {
        let mut b = Backoff::default();
        let esperas: Vec<u64> = (0..9).map(|_| b.next_delay().as_secs()).collect();
        assert_eq!(esperas, [1, 2, 4, 8, 16, 32, 60, 60, 60]);
        b.reset();
        assert_eq!(b.next_delay().as_secs(), 1);
    }

    fn resposta(f: impl FnOnce(&mut RegisterResponse)) -> RegisterResponse {
        let mut r = RegisterResponse::default();
        f(&mut r);
        r
    }

    #[test]
    fn interpreta_as_respostas() {
        assert!(matches!(
            interpretar(resposta(|r| r.machine_authorized = true), false),
            Ok(RegisterOutcome::Authorized(_))
        ));
        assert!(matches!(
            interpretar(resposta(|r| r.auth_url = "https://x/a/1".into()), false),
            Ok(RegisterOutcome::NeedsLogin { url }) if url == "https://x/a/1"
        ));
        assert!(matches!(
            interpretar(resposta(|_| {}), false),
            Ok(RegisterOutcome::NeedsApproval(_))
        ));
        assert!(matches!(
            interpretar(resposta(|r| r.node_key_expired = true), false),
            Ok(RegisterOutcome::KeyExpired)
        ));
        assert!(matches!(
            interpretar(resposta(|r| r.error = "ruim".into()), true),
            Err(ControlError::AuthKeyRejected(m)) if m == "ruim"
        ));
        assert!(matches!(
            interpretar(resposta(|r| r.error = "ruim".into()), false),
            Err(ControlError::Rejected(_))
        ));
        assert!(matches!(
            interpretar(
                resposta(|r| r.node_key_signature = Some("AAAA".into())),
                false
            ),
            Err(ControlError::TailnetLock)
        ));
    }

    #[test]
    fn erros_definitivos_nao_repetem() {
        assert!(!ControlError::AuthKeyRejected("x".into()).is_transient());
        assert!(!ControlError::VersionRejected { version: 1 }.is_transient());
        assert!(!ControlError::TailnetLock.is_transient());
        assert!(ControlError::Timeout.is_transient());
        assert!(
            ControlError::Http {
                status: 503,
                body: String::new()
            }
            .is_transient()
        );
        assert!(
            !ControlError::Http {
                status: 403,
                body: String::new()
            }
            .is_transient()
        );
    }

    #[test]
    fn o_debug_do_pedido_nao_mostra_a_chave() {
        let p = RegisterParams {
            node_key: NodePublic::default(),
            old_node_key: None,
            auth_key: Some(Zeroizing::new("tskey-auth-segredo".into())),
            hostinfo: Hostinfo::default(),
            ephemeral: false,
        };
        assert!(!format!("{p:?}").contains("segredo"));
    }
}
