//! A API do app: [`OpenVpnBuilder`], [`OpenVpn`], [`State`] e [`Prompt`].
//!
//! O motor roda numa thread própria (`openvpn-<perfil>`), com um runtime do `tokio`. O único
//! acesso à rede da VPN é o [`OpenVpn::proxy`]: um SOCKS5 em `127.0.0.1` com porta e credencial
//! aleatórias, que só os clientes do próprio app recebem. Não existe placa de rede virtual, nem
//! rota no sistema, nem porta que outro programa alcance.

use std::fmt;
use std::net::IpAddr;
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tomni_ui_net::Proxy;
use tomni_ui_netstack::{SocksAccess, SocksListener};
use zeroize::Zeroizing;

use crate::auth::Credentials;
use crate::channel::Clock;
use crate::config::{AuthUserPass, Config};
use crate::engine::{self, Params};
use crate::transport::{Connector, SystemConnector};

/// O que o motor pergunta à pessoa.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    /// O número que [`OpenVpn::answer`] devolve.
    pub id: u64,
    /// O texto da pergunta (do servidor ou do `.ovpn`; no caso da senha, o motivo da recusa).
    pub text: String,
    /// Se a resposta aparece enquanto se digita (um código de uso único costuma aparecer; uma
    /// senha, não).
    pub echo: bool,
    /// De onde vem a pergunta.
    pub kind: PromptKind,
}

/// De onde vem uma [`Prompt`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    /// O `static-challenge` do `.ovpn`: pergunta antes de conectar, a cada conexão nova.
    StaticChallenge,
    /// O desafio do servidor (`CRV1`) depois de uma recusa.
    Challenge,
    /// A senha: o `.ovpn` pede e o cofre não tem, ou o servidor a recusou.
    Password,
}

/// Em que pé está a VPN.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// Abrindo o enlace e o TLS com `remote`.
    Connecting {
        /// O servidor, como `nome:porta/protocolo`.
        remote: String,
    },
    /// O motor precisa de uma resposta da pessoa ([`OpenVpn::answer`]).
    Asking(Prompt),
    /// O servidor espera uma confirmação de fora (`AUTH_PENDING`: um aviso no celular, um login
    /// pelo navegador).
    Waiting {
        /// O que mostrar.
        message: String,
        /// A página a abrir, se o servidor pediu.
        url: Option<String>,
    },
    /// Conectada: a rede da VPN está disponível pelo [`OpenVpn::proxy`].
    Connected {
        /// O endereço do app dentro da VPN.
        address: IpAddr,
        /// O servidor, como `nome:porta/protocolo`.
        remote: String,
        /// A cifra do canal de dados.
        cipher: String,
        /// Avisos para a pessoa (opções do servidor que o app ignorou…).
        warnings: Vec<String>,
    },
    /// Caiu e está recomeçando (`ping-restart`, `RESTART`, rede fora); o motivo vem em português.
    Retrying(String),
    /// Parou por algo que não se resolve sozinho (senha recusada e cancelada, `HALT`, certificado
    /// vencido). O motivo vem em português.
    Failed(String),
    /// Parada a pedido.
    Stopped,
}

/// Por que a VPN não pôde começar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// O ouvinte das mudanças de estado.
type Callback = Box<dyn Fn(&State) + Send>;

/// O estado compartilhado entre o motor e a [`OpenVpn`].
pub(crate) struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    callback: Mutex<Option<Callback>>,
}

impl Shared {
    pub(crate) fn set(&self, state: State) {
        if let Some(callback) = &*self.callback.lock().unwrap_or_else(PoisonError::into_inner) {
            callback(&state);
        }
        *self.state.lock().unwrap_or_else(PoisonError::into_inner) = state;
        self.changed.notify_all();
    }

    pub(crate) fn current(&self) -> State {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// O que a [`OpenVpn`] manda ao motor.
pub(crate) enum Command {
    Answer {
        id: u64,
        answer: Option<Zeroizing<String>>,
    },
    Stop,
}

/// Monta e liga uma [`OpenVpn`].
pub struct OpenVpnBuilder {
    config: Config,
    name: String,
    credentials: Option<Credentials>,
    proxy: Option<Proxy>,
    callback: Option<Callback>,
    connector: Option<Box<dyn Connector>>,
    clock: Option<Clock>,
}

impl OpenVpnBuilder {
    /// Um motor para o perfil `config`.
    pub fn new(config: Config) -> OpenVpnBuilder {
        OpenVpnBuilder {
            config,
            name: "vpn".to_owned(),
            credentials: None,
            proxy: None,
            callback: None,
            connector: None,
            clock: None,
        }
    }

    /// O nome do perfil: dá nome à thread (`openvpn-<nome>`).
    pub fn name(mut self, name: &str) -> Self {
        self.name = name.to_owned();
        self
    }

    /// O usuário e a senha do `auth-user-pass` (do cofre). Valem no lugar dos do `.ovpn`.
    pub fn credentials(mut self, user: String, password: Zeroizing<String>) -> Self {
        self.credentials = Some(Credentials { user, password });
        self
    }

    /// O proxy do transporte TCP (o "Salto/Proxy" do perfil). O UDP não passa por proxy.
    pub fn proxy(mut self, proxy: Proxy) -> Self {
        self.proxy = Some(proxy);
        self
    }

    /// Recebe cada mudança de estado, na thread do motor: não demore dentro dela.
    pub fn on_state(mut self, callback: impl Fn(&State) + Send + 'static) -> Self {
        self.callback = Some(Box::new(callback));
        self
    }

    /// Quem abre o enlace com o servidor (os testes ligam um transporte em memória).
    #[cfg(feature = "test-util")]
    pub fn connector(mut self, connector: impl Connector) -> Self {
        self.connector = Some(Box::new(connector));
        self
    }

    /// A hora que vale para a validade dos certificados (os testes usam uma fixa).
    #[cfg(feature = "test-util")]
    pub fn clock(mut self, clock: Clock) -> Self {
        self.clock = Some(clock);
        self
    }

    /// Liga o motor numa thread própria e devolve na hora, com a ponte SOCKS interna já aberta.
    /// A conexão com o servidor acontece depois, e o resultado chega pelo [`State`].
    ///
    /// # Errors
    ///
    /// O perfil que não serve (sem `remote`, pede senha e não há), ou a ponte que não abriu.
    pub fn start(self) -> Result<OpenVpn, Error> {
        let config = self.config;
        if !config.is_client {
            return Err(Error(
                "o perfil não é de cliente (falta a linha `client`)".into(),
            ));
        }
        if config.remotes.is_empty() {
            return Err(Error("o perfil não tem nenhuma linha `remote`".into()));
        }
        let credentials = self
            .credentials
            .or_else(|| Credentials::from_config(&config.auth_user_pass));
        if credentials.is_none() && config.auth_user_pass == AuthUserPass::Ask {
            return Err(Error(
                "o perfil pede usuário e senha (auth-user-pass); informe-os antes de conectar"
                    .into(),
            ));
        }
        let listener = SocksListener::bind().map_err(|e| {
            Error(format!(
                "não foi possível abrir a ponte local da VPN: {}",
                tomni_ui_net::error_message(&e)
            ))
        })?;
        let access = listener.access().clone();
        let shared = Arc::new(Shared {
            state: Mutex::new(State::Connecting {
                remote: "…".to_owned(),
            }),
            changed: Condvar::new(),
            callback: Mutex::new(self.callback),
        });
        let (commands, receiver) = mpsc::unbounded_channel();
        let (ready_tx, ready_rx) = std_mpsc::channel();
        let (done_tx, done_rx) = std_mpsc::channel();
        let params = Params {
            config,
            credentials,
            proxy: self.proxy,
            connector: self.connector.unwrap_or_else(|| Box::new(SystemConnector)),
            clock: self.clock.unwrap_or_else(Clock::system),
        };
        let engine_shared = shared.clone();
        std::thread::Builder::new()
            .name(format!("openvpn-{}", self.name))
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                match runtime {
                    Ok(runtime) => runtime.block_on(engine::run(
                        params,
                        engine_shared,
                        receiver,
                        listener,
                        ready_tx,
                    )),
                    Err(error) => {
                        let _ = ready_tx.send(Err(format!("sem runtime para a VPN: {error}")));
                    }
                }
                let _ = done_tx.send(());
            })
            .map_err(|e| Error(format!("não foi possível iniciar a thread da VPN: {e}")))?;
        match ready_rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(message)) => return Err(Error(message)),
            Err(_) => return Err(Error("o motor da VPN não iniciou".into())),
        }
        Ok(OpenVpn {
            access,
            shared,
            commands,
            done: Mutex::new(done_rx),
        })
    }
}

/// A VPN em funcionamento. Largar o valor para a VPN (sem o aviso ao servidor que o
/// [`stop`](OpenVpn::stop) espera).
pub struct OpenVpn {
    access: SocksAccess,
    shared: Arc<Shared>,
    commands: mpsc::UnboundedSender<Command>,
    /// Num `Mutex` para o `OpenVpn` ser `Sync` e poder ficar num `Arc` compartilhado entre threads.
    done: Mutex<std_mpsc::Receiver<()>>,
}

impl OpenVpn {
    /// O endereço e a credencial da ponte SOCKS5 interna. O app nunca os mostra.
    pub fn socks(&self) -> &SocksAccess {
        &self.access
    }

    /// A ponte pronta para os clientes da lib (SSH, SFTP, Telnet, FTP, S3…).
    pub fn proxy(&self) -> Proxy {
        Proxy::Socks5 {
            host: self.access.addr.ip().to_string(),
            port: self.access.addr.port(),
            user: Some(self.access.user.clone()),
            password: Some(self.access.password.to_string()),
        }
    }

    /// O estado de agora.
    pub fn state(&self) -> State {
        self.shared.current()
    }

    /// Responde à pergunta do [`State::Asking`] (código de uso único, desafio, senha). `None`
    /// cancela: a VPN para com "Failed".
    pub fn answer(&self, id: u64, answer: Option<Zeroizing<String>>) {
        let _ = self.commands.send(Command::Answer { id, answer });
    }

    /// Espera conectar por até `limit` e devolve o endereço da VPN, ou o motivo de não ter
    /// dado. O tempo que a pessoa leva para responder às perguntas não conta.
    ///
    /// # Errors
    ///
    /// A mensagem em português do que impediu, ou "tempo esgotado".
    pub fn wait_connected(&self, limit: Duration) -> Result<IpAddr, String> {
        let mut deadline = Instant::now() + limit;
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut asked_since: Option<Instant> = None;
        loop {
            let now = Instant::now();
            // O tempo parado numa pergunta (ou numa confirmação de fora) é da pessoa, não nosso.
            let waiting_person = matches!(&*state, State::Asking(_) | State::Waiting { .. });
            match (waiting_person, asked_since) {
                (true, None) => asked_since = Some(now),
                (false, Some(since)) => {
                    deadline += now - since;
                    asked_since = None;
                }
                _ => {}
            }
            match &*state {
                State::Connected { address, .. } => return Ok(*address),
                State::Failed(message) => return Err(message.clone()),
                State::Stopped => return Err("a VPN foi parada".into()),
                _ => {}
            }
            let wait = if asked_since.is_some() {
                // Parado numa pergunta: acorda quando o estado mudar.
                Duration::from_secs(1)
            } else if now >= deadline {
                return Err(format!(
                    "a VPN não conectou em {} s",
                    limit.as_secs().max(1)
                ));
            } else {
                deadline - now
            };
            state = self
                .shared
                .changed
                .wait_timeout(state, wait)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// Avisa o servidor (`explicit-exit-notify`) e para; espera no máximo `wait`.
    pub fn stop(self, wait: Duration) {
        let _ = self.commands.send(Command::Stop);
        let receiver = self.done.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = receiver.recv_timeout(wait);
    }
}

impl Drop for OpenVpn {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Stop);
    }
}

impl fmt::Debug for OpenVpn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenVpn")
            .field("state", &self.state())
            .finish_non_exhaustive()
    }
}
