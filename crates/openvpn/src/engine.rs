//! O motor: um laço assíncrono fino sobre a [`Session`] (sem E/S). Abre o enlace, entrega os
//! datagramas e o relógio à sessão, leva os pacotes IP de e para a pilha de rede e cuida do que
//! é do app: os `remote`, as esperas entre tentativas, as perguntas à pessoa e a troca da pilha.
//!
//! Tudo roda numa thread só, com um runtime `current_thread` (ver [`crate::vpn`]).

use std::future::{Future, pending, poll_fn};
use std::io;
use std::net::{IpAddr, Ipv4Addr};
use std::pin::Pin;
use std::sync::{Arc, PoisonError, RwLock};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tokio::time::{sleep, sleep_until, timeout};
use tomni_ui_net::Proxy;
use tomni_ui_netstack::{
    Answer, Cidr, DnsOverTcp, DnsScope, NetStack, PacketTunnel, Resolver, SocksBridge,
    SocksListener, Unreachable,
};
use zeroize::Zeroizing;

use crate::auth::{AuthFailure, AuthState, Credentials};
use crate::channel::Clock;
use crate::config::{Config, Family, Proto, Remote};
use crate::control::{ControlError, ControlOptions};
use crate::session::{Connected, Failure, Session, SessionEvent, SessionSettings};
use crate::transport::{Connector, Link, Target};
use crate::vpn::{Command, Prompt, PromptKind, Shared, State};

/// A espera padrão entre tentativas (`connect-retry`).
const DEFAULT_CONNECT_RETRY: Duration = Duration::from_secs(5);
/// O teto padrão da espera crescente.
const DEFAULT_CONNECT_RETRY_MAX: Duration = Duration::from_secs(300);
/// A espera para recomeçar depois de uma sessão que tinha conectado (queda, `RESTART`).
const QUICK_RETRY: Duration = Duration::from_secs(1);
/// O tempo padrão para o enlace (TCP e proxy) chegar ao servidor.
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// O máximo que a saída espera para escrever o aviso de saída e fechar.
const CLOSE_FLUSH: Duration = Duration::from_secs(1);
/// Quantos pacotes da pilha o motor lê por volta do laço antes de olhar o resto.
const BATCH: usize = 64;
/// O tamanho da fila de pacotes entre a pilha e o motor (o que passa disso é descartado).
const QUEUE: usize = 1024;

/// O que o `start` entrega ao motor.
pub(crate) struct Params {
    pub config: Config,
    pub credentials: Option<Credentials>,
    pub proxy: Option<Proxy>,
    pub connector: Box<dyn Connector>,
    pub clock: Clock,
}

/// O resolvedor de DNS da ponte, trocado a cada conexão (os servidores vêm do `PUSH_REPLY`).
#[derive(Default)]
struct SwitchResolver {
    current: RwLock<Option<Arc<DnsOverTcp>>>,
}

impl SwitchResolver {
    fn set(&self, resolver: Option<Arc<DnsOverTcp>>) {
        *self.current.write().unwrap_or_else(PoisonError::into_inner) = resolver;
    }
}

impl Resolver for SwitchResolver {
    fn resolve<'a>(
        &'a self,
        stack: &'a NetStack,
        name: &'a str,
    ) -> Pin<Box<dyn Future<Output = Answer> + Send + 'a>> {
        Box::pin(async move {
            let current = self
                .current
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            match current {
                Some(resolver) => resolver.resolve(stack, name).await,
                None => Answer::Skip,
            }
        })
    }
}

/// O túnel de pacotes IP que a pilha de rede usa: duas filas para o motor.
struct OvpnTunnel {
    mtu: usize,
    addresses: Vec<Cidr>,
    routes: Vec<Cidr>,
    to_engine: mpsc::Sender<Vec<u8>>,
    from_engine: mpsc::Receiver<Vec<u8>>,
    unreachable: mpsc::UnboundedReceiver<Unreachable>,
}

impl PacketTunnel for OvpnTunnel {
    fn mtu(&self) -> usize {
        self.mtu
    }

    fn addresses(&self) -> Vec<Cidr> {
        self.addresses.clone()
    }

    fn routes(&self) -> Vec<Cidr> {
        self.routes.clone()
    }

    fn send(&mut self, packet: &[u8]) {
        // Fila cheia: descarta, o TCP de dentro retransmite.
        let _ = self.to_engine.try_send(packet.to_vec());
    }

    fn poll_recv(&mut self, cx: &mut Context<'_>, buf: &mut Vec<u8>) -> Poll<io::Result<()>> {
        match self.from_engine.poll_recv(cx) {
            Poll::Ready(Some(packet)) => {
                *buf = packet;
                Poll::Ready(Ok(()))
            }
            Poll::Ready(None) => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "a VPN parou",
            ))),
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_unreachable(&mut self, cx: &mut Context<'_>) -> Poll<Unreachable> {
        match self.unreachable.poll_recv(cx) {
            Poll::Ready(Some(notice)) => Poll::Ready(notice),
            Poll::Ready(None) | Poll::Pending => Poll::Pending,
        }
    }
}

/// A pilha de rede montada sobre uma sessão e as pontas do motor das filas.
struct Net {
    stack: NetStack,
    to_stack: mpsc::Sender<Vec<u8>>,
    from_stack: mpsc::Receiver<Vec<u8>>,
    _unreachable: mpsc::UnboundedSender<Unreachable>,
    /// O que a identifica: se mudar numa reconexão, a pilha é trocada.
    identity: (Ipv4Addr, u8, usize, Vec<Cidr>),
}

/// Como uma sessão terminou.
enum End {
    /// A pessoa pediu para parar.
    Stopped,
    /// Recomeçar depois da espera normal.
    Retry(String),
    /// Parar de vez, com o motivo.
    Fatal(String),
    /// O servidor recusou a autenticação.
    Auth(AuthFailure),
}

/// O que acordou o laço da sessão.
enum Wake {
    Datagram(io::Result<()>),
    Packet(Option<Vec<u8>>),
    Timer,
    Command(Option<Command>),
}

/// O que a pessoa respondeu a uma pergunta.
enum Asked {
    Answer(Zeroizing<String>),
    Cancelled,
    Stop,
}

struct Engine {
    config: Config,
    credentials: Option<Credentials>,
    proxy: Option<Proxy>,
    connector: Box<dyn Connector>,
    clock: Clock,
    shared: Arc<Shared>,
    commands: mpsc::UnboundedReceiver<Command>,
    bridge: SocksBridge,
    resolver: Arc<SwitchResolver>,
    remotes: Vec<Remote>,
    next_remote: usize,
    /// O `auth-token` e o usuário que vai com ele (só em memória).
    token: Option<(Option<String>, Zeroizing<String>)>,
    /// A resposta a um desafio `CRV1`, para a próxima conexão.
    challenge: Option<Credentials>,
    net: Option<Net>,
    prompts: u64,
}

pub(crate) async fn run(
    params: Params,
    shared: Arc<Shared>,
    commands: mpsc::UnboundedReceiver<Command>,
    listener: SocksListener,
    ready: std::sync::mpsc::Sender<Result<(), String>>,
) {
    let resolver = Arc::new(SwitchResolver::default());
    let bridge = match listener.resolver(resolver.clone()).serve_later() {
        Ok(bridge) => bridge,
        Err(error) => {
            let _ = ready.send(Err(format!(
                "não foi possível abrir a ponte local da VPN: {error}"
            )));
            return;
        }
    };
    let _ = ready.send(Ok(()));
    let mut remotes = params.config.remotes.clone();
    if params.config.remote_random {
        shuffle(&mut remotes);
    }
    let mut engine = Engine {
        config: params.config,
        credentials: params.credentials,
        proxy: params.proxy,
        connector: params.connector,
        clock: params.clock,
        shared,
        commands,
        bridge,
        resolver,
        remotes,
        next_remote: 0,
        token: None,
        challenge: None,
        net: None,
        prompts: 0,
    };
    engine.main_loop().await;
    engine.finish().await;
}

/// Embaralha com números do sistema (Fisher-Yates).
fn shuffle<T>(list: &mut [T]) {
    for i in (1..list.len()).rev() {
        let mut raw = [0u8; 4];
        if getrandom::fill(&mut raw).is_err() {
            return;
        }
        let j = (u32::from_be_bytes(raw) as usize) % (i + 1);
        list.swap(i, j);
    }
}

fn describe(remote: &Remote) -> String {
    let proto = match remote.proto {
        Proto::Udp => "udp",
        Proto::Tcp => "tcp",
    };
    format!("{}:{}/{proto}", remote.host, remote.port)
}

impl Engine {
    fn state(&self, state: State) {
        self.shared.set(state);
    }

    async fn finish(&mut self) {
        if let Some(net) = self.net.take() {
            net.stack.shutdown().await;
        }
        self.resolver.set(None);
        // `stop` só consome a ponte; o motor acaba junto, então basta largá-la.
    }

    /// Espera `limit` ou o pedido de parada. `true` se pediram para parar.
    async fn sleep_or_stop(&mut self, limit: Duration) -> bool {
        let sleeper = sleep(limit);
        tokio::pin!(sleeper);
        loop {
            tokio::select! {
                () = &mut sleeper => return false,
                command = self.commands.recv() => match command {
                    Some(Command::Answer { .. }) => {}
                    Some(Command::Stop) | None => return true,
                },
            }
        }
    }

    /// Pergunta à pessoa e espera a resposta (sem limite de tempo).
    async fn ask(&mut self, kind: PromptKind, text: String, echo: bool) -> Asked {
        self.prompts += 1;
        let id = self.prompts;
        self.state(State::Asking(Prompt {
            id,
            text,
            echo,
            kind,
        }));
        loop {
            match self.commands.recv().await {
                Some(Command::Answer {
                    id: answered,
                    answer,
                }) if answered == id => {
                    return match answer {
                        Some(answer) => Asked::Answer(answer),
                        None => Asked::Cancelled,
                    };
                }
                Some(Command::Answer { .. }) => {}
                Some(Command::Stop) | None => return Asked::Stop,
            }
        }
    }

    async fn main_loop(&mut self) {
        // A senha que falta no cofre (`auth-user-pass` com o usuário e sem a senha).
        if self.credentials.is_none()
            && let crate::config::AuthUserPass::Credentials {
                user,
                password: None,
            } = &self.config.auth_user_pass
        {
            let user = user.clone();
            let text = format!("Senha de {user}");
            match self.ask(PromptKind::Password, text, false).await {
                Asked::Answer(password) => {
                    self.credentials = Some(Credentials { user, password });
                }
                Asked::Cancelled => {
                    self.state(State::Failed("a senha não foi informada".into()));
                    return;
                }
                Asked::Stop => {
                    self.state(State::Stopped);
                    return;
                }
            }
        }
        let retry_base = self
            .config
            .connect_retry
            .map_or(DEFAULT_CONNECT_RETRY, |s| Duration::from_secs(u64::from(s)));
        let retry_cap = self
            .config
            .connect_retry_max
            .map_or(DEFAULT_CONNECT_RETRY_MAX, |s| {
                Duration::from_secs(u64::from(s))
            })
            .max(retry_base);
        let mut failures: u32 = 0;
        loop {
            let auth = match self.prepare_auth().await {
                Ok(auth) => auth,
                Err(end) => {
                    self.conclude(end);
                    return;
                }
            };
            let remote = self.remotes[self.next_remote % self.remotes.len()].clone();
            self.next_remote = (self.next_remote + 1) % self.remotes.len();
            let label = describe(&remote);
            self.state(State::Connecting {
                remote: label.clone(),
            });
            let used_token = auth.has_token();
            let (end, reached) = self.attempt(&remote, &label, auth).await;
            if reached {
                failures = 0;
            }
            let wait = match end {
                End::Stopped => {
                    self.state(State::Stopped);
                    return;
                }
                End::Fatal(message) => {
                    self.state(State::Failed(message));
                    return;
                }
                End::Retry(message) => {
                    self.state(State::Retrying(message));
                    failures += 1;
                    if reached {
                        QUICK_RETRY
                    } else {
                        backoff(retry_base, retry_cap, failures)
                    }
                }
                End::Auth(failure) => match self.on_auth_failure(failure, used_token).await {
                    Ok(Some(wait)) => wait,
                    Ok(None) => Duration::ZERO,
                    Err(end) => {
                        self.conclude(end);
                        return;
                    }
                },
            };
            if !wait.is_zero() && self.sleep_or_stop(wait).await {
                self.state(State::Stopped);
                return;
            }
        }
    }

    fn conclude(&self, end: End) {
        match end {
            End::Stopped => self.state(State::Stopped),
            End::Fatal(message) => self.state(State::Failed(message)),
            End::Retry(message) => self.state(State::Failed(message)),
            End::Auth(failure) => self.state(State::Failed(failure.to_string())),
        }
    }

    /// O que fazer com a recusa de autenticação. `Ok(Some(espera))` recomeça depois da espera;
    /// `Ok(None)` recomeça já; `Err` termina.
    async fn on_auth_failure(
        &mut self,
        failure: AuthFailure,
        used_token: bool,
    ) -> Result<Option<Duration>, End> {
        match failure {
            // O token não vale mais: tenta uma vez com a senha de verdade.
            AuthFailure::Rejected { .. } | AuthFailure::Session { .. } if used_token => {
                self.token = None;
                Ok(None)
            }
            AuthFailure::Session { .. } => {
                self.token = None;
                Ok(None)
            }
            AuthFailure::Temporary {
                reason, backoff, ..
            } => {
                let wait = backoff.unwrap_or(DEFAULT_CONNECT_RETRY);
                let text = AuthFailure::Temporary {
                    reason,
                    backoff,
                    advance: None,
                }
                .to_string();
                self.state(State::Retrying(text));
                Ok(Some(wait))
            }
            AuthFailure::Challenge(challenge) => {
                match self
                    .ask(
                        PromptKind::Challenge,
                        challenge.text.clone(),
                        challenge.echo,
                    )
                    .await
                {
                    Asked::Answer(answer) => {
                        self.challenge = Some(challenge.answer(&answer));
                        Ok(None)
                    }
                    Asked::Cancelled => Err(End::Fatal(
                        "o desafio do servidor não foi respondido".into(),
                    )),
                    Asked::Stop => Err(End::Stopped),
                }
            }
            rejected @ AuthFailure::Rejected { .. } => {
                let Some(user) = self.credentials.as_ref().map(|c| c.user.clone()) else {
                    return Err(End::Fatal(rejected.to_string()));
                };
                match self
                    .ask(PromptKind::Password, rejected.to_string(), false)
                    .await
                {
                    Asked::Answer(password) => {
                        self.credentials = Some(Credentials { user, password });
                        Ok(None)
                    }
                    Asked::Cancelled => Err(End::Fatal(rejected.to_string())),
                    Asked::Stop => Err(End::Stopped),
                }
            }
        }
    }

    /// O que vai nos campos de usuário e senha desta tentativa.
    async fn prepare_auth(&mut self) -> Result<AuthState, End> {
        let mut auth = if let Some(challenge) = self.challenge.take() {
            AuthState::new(challenge)
        } else {
            match self.credentials.clone() {
                None => AuthState::none(),
                Some(credentials) => match self.config.static_challenge.clone() {
                    // Cada conexão nova do zero precisa de um código novo. Com o `auth-token` o
                    // servidor não pergunta de novo.
                    Some(challenge) if self.token.is_none() => {
                        match self
                            .ask(PromptKind::StaticChallenge, challenge.text, challenge.echo)
                            .await
                        {
                            Asked::Answer(answer) => {
                                AuthState::with_static_challenge(&credentials, &answer)
                            }
                            Asked::Cancelled => {
                                return Err(End::Fatal(
                                    "o código de verificação não foi informado".into(),
                                ));
                            }
                            Asked::Stop => return Err(End::Stopped),
                        }
                    }
                    _ => AuthState::new(credentials),
                },
            }
        };
        if let Some((user, token)) = &self.token {
            auth.set_token(token.clone(), user.clone());
        }
        Ok(auth)
    }

    /// Abre o enlace e roda a sessão. O `bool` diz se a sessão chegou a conectar.
    async fn attempt(&mut self, remote: &Remote, label: &str, auth: AuthState) -> (End, bool) {
        let connect_timeout = self
            .config
            .connect_timeout
            .map_or(DEFAULT_CONNECT_TIMEOUT, |s| {
                Duration::from_secs(u64::from(s))
            });
        let target = Target {
            remote,
            local_port: self.config.local_port,
            timeout: connect_timeout,
            proxy: self.proxy.as_ref(),
        };
        let opened = tokio::select! {
            result = self.connector.connect(target) => result,
            () = stop_command(&mut self.commands) => return (End::Stopped, false),
        };
        let mut link = match opened {
            Ok(link) => link,
            Err(error) => {
                return (
                    End::Retry(format!(
                        "não foi possível conectar a {label}: {}",
                        tomni_ui_net::connect_error_message(&error)
                    )),
                    false,
                );
            }
        };
        let ipv6 = remote.family == Family::V6 || remote.host.parse::<std::net::Ipv6Addr>().is_ok();
        let mut control = match ControlOptions::from_config(
            &self.config,
            remote.proto,
            ipv6,
            auth,
            self.clock.clone(),
        ) {
            Ok(control) => control,
            Err(error) => return (End::Fatal(error.to_string()), false),
        };
        control.server_name = remote.host.clone();
        let settings = SessionSettings::from_config(&self.config, remote.proto);
        let session = match Session::new(control, settings, Instant::now()) {
            Ok(session) => session,
            Err(error) => return (End::Fatal(error.to_string()), false),
        };
        self.run_session(session, &mut link, label).await
    }

    /// O laço da sessão.
    async fn run_session(
        &mut self,
        mut session: Session,
        link: &mut Box<dyn Link>,
        label: &str,
    ) -> (End, bool) {
        let mut reached = false;
        let mut warnings: Vec<String> = Vec::new();
        let mut received = Vec::new();
        let mut stopping = false;
        loop {
            let now = Instant::now();
            // 1. O que a sessão tem a enviar, a entregar e a contar.
            while let Some(datagram) = session.poll_transmit(now) {
                if let Err(error) = link.send(&datagram) {
                    self.capture_token(&session);
                    return (End::Retry(link_error(&error)), reached);
                }
            }
            while let Some(packet) = session.poll_ip() {
                if let Some(net) = &self.net {
                    let _ = net.to_stack.try_send(packet);
                }
            }
            while let Some(event) = session.poll_event() {
                match event {
                    SessionEvent::Connected(connected) => {
                        reached = true;
                        self.capture_token(&session);
                        let cipher = connected.cipher.name().to_owned();
                        let address = IpAddr::V4(connected.address);
                        self.on_connected(&connected).await;
                        self.state(State::Connected {
                            address,
                            remote: label.to_owned(),
                            cipher,
                            warnings: warnings.clone(),
                        });
                    }
                    SessionEvent::Rekeyed => self.capture_token(&session),
                    SessionEvent::AuthPending { .. } => {
                        self.state(State::Waiting {
                            message: "Aguardando confirmação do servidor…".into(),
                            url: None,
                        });
                    }
                    SessionEvent::OpenUrl(url) => {
                        self.state(State::Waiting {
                            message: "Aguardando o login pelo navegador…".into(),
                            url: Some(url),
                        });
                    }
                    SessionEvent::Info(_) => {}
                    SessionEvent::Warning(message) => {
                        warnings.push(message);
                        if reached {
                            self.refresh_warnings(label, &warnings);
                        }
                    }
                    SessionEvent::AuthFailed(failure) => {
                        self.capture_token(&session);
                        return (End::Auth(failure), reached);
                    }
                    SessionEvent::Restart(text) => {
                        self.capture_token(&session);
                        let message = if text.is_empty() {
                            "o servidor mandou reiniciar".to_owned()
                        } else {
                            format!("o servidor mandou reiniciar: {text}")
                        };
                        return (End::Retry(message), reached);
                    }
                    SessionEvent::Halt(text) => {
                        let message = if text.is_empty() {
                            "o servidor mandou parar".to_owned()
                        } else {
                            format!("o servidor mandou parar: {text}")
                        };
                        return (End::Fatal(message), reached);
                    }
                    SessionEvent::Failed(failure) => {
                        self.capture_token(&session);
                        return (classify(failure), reached);
                    }
                }
            }
            if stopping {
                let _ = timeout(CLOSE_FLUSH, poll_fn(|cx| link.poll_flush(cx))).await;
                self.capture_token(&session);
                return (End::Stopped, reached);
            }
            if session.is_finished() {
                self.capture_token(&session);
                return (End::Retry("a sessão acabou".into()), reached);
            }

            // 2. Espera o que vier primeiro.
            let deadline = session.next_timeout();
            let wake = {
                let net = &mut self.net;
                let commands = &mut self.commands;
                tokio::select! {
                    result = poll_fn(|cx| link.poll_recv(cx, &mut received)) => Wake::Datagram(result),
                    packet = async {
                        match net {
                            Some(net) => net.from_stack.recv().await,
                            None => pending().await,
                        }
                    } => Wake::Packet(packet),
                    () = async {
                        match deadline {
                            Some(at) => sleep_until(tokio::time::Instant::from_std(at)).await,
                            None => pending().await,
                        }
                    } => Wake::Timer,
                    command = commands.recv() => Wake::Command(command),
                }
            };
            let now = Instant::now();
            match wake {
                Wake::Datagram(Ok(())) => session.handle_datagram(&received, now),
                Wake::Datagram(Err(error)) => {
                    self.capture_token(&session);
                    return (End::Retry(link_error(&error)), reached);
                }
                Wake::Packet(Some(packet)) => {
                    session.send_ip(&packet, now);
                    if let Some(net) = &mut self.net {
                        for _ in 0..BATCH {
                            match net.from_stack.try_recv() {
                                Ok(packet) => session.send_ip(&packet, now),
                                Err(_) => break,
                            }
                        }
                    }
                }
                // A pilha foi largada sem querer: segue sem ela.
                Wake::Packet(None) => {
                    self.net = None;
                }
                Wake::Timer => session.handle_timeout(now),
                Wake::Command(Some(Command::Answer { .. })) => {}
                Wake::Command(Some(Command::Stop) | None) => {
                    session.close(now);
                    stopping = true;
                }
            }
        }
    }

    fn refresh_warnings(&self, label: &str, warnings: &[String]) {
        if let State::Connected {
            address, cipher, ..
        } = self.shared_state()
        {
            self.state(State::Connected {
                address,
                remote: label.to_owned(),
                cipher,
                warnings: warnings.to_vec(),
            });
        }
    }

    fn shared_state(&self) -> State {
        self.shared.current()
    }

    /// Guarda o `auth-token` que o servidor empurrou, para a reconexão.
    fn capture_token(&mut self, session: &Session) {
        if let Some((user, token)) = session.auth_state().token() {
            self.token = Some((user.map(str::to_owned), Zeroizing::new(token.to_owned())));
        }
    }

    /// A rede da sessão ficou pronta: monta a pilha (ou reaproveita a que já existe, se o
    /// endereço e as rotas são os mesmos) e liga o DNS da VPN.
    async fn on_connected(&mut self, connected: &Connected) {
        let mut routes: Vec<Cidr> = Vec::new();
        let mut push_route = |network: Ipv4Addr, prefix: u8| {
            if let Some(cidr) = Cidr::new(IpAddr::V4(network), prefix)
                && !routes.contains(&cidr)
            {
                routes.push(cidr);
            }
        };
        push_route(connected.address, connected.prefix_len);
        for route in &connected.routes {
            push_route(route.network, route.prefix_len);
        }
        if let Some(peer) = connected.peer {
            push_route(peer, 32);
        }
        let identity = (
            connected.address,
            connected.prefix_len,
            connected.mtu,
            routes.clone(),
        );
        let reuse = self
            .net
            .as_ref()
            .is_some_and(|net| net.identity == identity);
        if !reuse {
            if let Some(old) = self.net.take() {
                // O endereço ou as rotas mudaram: as conexões da pilha antiga caem.
                self.bridge.detach();
                old.stack.shutdown().await;
            }
            let Some(address) = Cidr::new(IpAddr::V4(connected.address), connected.prefix_len)
            else {
                return;
            };
            let (to_engine, from_tunnel) = mpsc::channel(QUEUE);
            let (to_tunnel, from_engine) = mpsc::channel(QUEUE);
            let (unreachable_tx, unreachable_rx) = mpsc::unbounded_channel();
            let tunnel = OvpnTunnel {
                mtu: connected.mtu,
                addresses: vec![address],
                routes,
                to_engine,
                from_engine,
                unreachable: unreachable_rx,
            };
            let stack = NetStack::spawn(tunnel);
            self.bridge.attach(stack.clone());
            self.net = Some(Net {
                stack,
                to_stack: to_tunnel,
                from_stack: from_tunnel,
                _unreachable: unreachable_tx,
                identity,
            });
        }
        self.resolver.set(dns_for(connected).map(Arc::new));
    }
}

/// Espera o pedido de parada (as respostas a perguntas, aqui, não valem).
async fn stop_command(commands: &mut mpsc::UnboundedReceiver<Command>) {
    loop {
        match commands.recv().await {
            Some(Command::Answer { .. }) => {}
            Some(Command::Stop) | None => return,
        }
    }
}

fn link_error(error: &io::Error) -> String {
    format!(
        "a conexão com o servidor caiu: {}",
        tomni_ui_net::error_message(error)
    )
}

/// A espera antes da tentativa `failures`: o `connect-retry` dobrando até o teto.
fn backoff(base: Duration, cap: Duration, failures: u32) -> Duration {
    let doublings = failures.saturating_sub(1).min(16);
    base.saturating_mul(1u32 << doublings).min(cap)
}

fn classify(failure: Failure) -> End {
    match &failure {
        Failure::Control(error) => match error {
            ControlError::HandshakeTimeout { .. } | ControlError::Closed | ControlError::Tls(_) => {
                End::Retry(failure.to_string())
            }
            _ => End::Fatal(failure.to_string()),
        },
        Failure::Unsupported(_) | Failure::PingExit(_) => End::Fatal(failure.to_string()),
        Failure::PingRestart(_) => End::Retry(failure.to_string()),
    }
}

/// O DNS da VPN, se o servidor empurrou algum servidor IPv4. Com `redirect-gateway`, todos os
/// nomes vão para ele; sem, só os dos domínios empurrados e os nomes sem ponto.
fn dns_for(connected: &Connected) -> Option<DnsOverTcp> {
    let servers: Vec<Ipv4Addr> = connected
        .dns_servers
        .iter()
        .filter_map(|server| match server.ip() {
            IpAddr::V4(ip) => Some(ip),
            IpAddr::V6(_) => None,
        })
        .collect();
    if servers.is_empty() {
        return None;
    }
    let scope = if connected.redirect_gateway {
        DnsScope::All
    } else {
        DnsScope::Domains(connected.search_domains.clone())
    };
    Some(DnsOverTcp::new(
        servers,
        connected.search_domains.clone(),
        scope,
    ))
}
