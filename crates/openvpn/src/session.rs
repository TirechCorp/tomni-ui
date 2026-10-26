//! A sessão do cliente: o canal de controle e o de dados juntos, mais os relógios do protocolo
//! (ping, `ping-restart`, renegociação, aviso de saída). Sem E/S: quem usa entrega os datagramas
//! e o relógio e retira o que enviar e os pacotes IP que chegaram.
//!
//! O motor ([`crate::vpn`]) coloca isto sobre um UDP ou TCP e a pilha de rede por cima.

use std::collections::VecDeque;
use std::fmt;
use std::net::{Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use crate::auth::{AuthFailure, AuthState};
use crate::channel::Datagram;
use crate::config::{AllowCompression, Compress, Config, MssFix, Proto};
use crate::control::{Control, ControlError, ControlOptions, DEFAULT_HAND_WINDOW, Event};
use crate::data::{DataChannel, DataCipher, DataError, DataOptions, Opened};
use crate::prf::DataKeys;
use crate::push::{IpRoute, Pushed, parse_route};

/// O `tun-mtu` quando nem o perfil nem o servidor dizem.
pub const DEFAULT_TUN_MTU: u32 = 1500;
/// O `mssfix` do app quando o perfil não diz (o OpenVPN usa 1492; o app é mais conservador para
/// caber em redes com PPPoE e outros túneis).
pub const DEFAULT_MSSFIX: u32 = 1450;
/// O `reneg-sec` do OpenVPN.
pub const DEFAULT_RENEG_SEC: u32 = 3600;
/// O menor MTU que o app aceita para a pilha.
pub const MIN_MTU: usize = 576;

/// O que a sessão precisa do perfil, fora do canal de controle.
#[derive(Debug, Clone)]
pub struct SessionSettings {
    /// UDP ou TCP.
    pub transport: Proto,
    /// O `ping` do perfil, em segundos.
    pub ping: Option<u32>,
    /// O `ping-restart` do perfil.
    pub ping_restart: Option<u32>,
    /// O `ping-exit` do perfil.
    pub ping_exit: Option<u32>,
    /// O `reneg-sec` (0 desliga).
    pub reneg_sec: Option<u32>,
    /// O `reneg-bytes`.
    pub reneg_bytes: Option<u64>,
    /// O `reneg-pkts`.
    pub reneg_pkts: Option<u64>,
    /// O `replay-window`, em pacotes.
    pub replay_window: u32,
    /// O `tran-window`.
    pub tran_window: Duration,
    /// O `hand-window`.
    pub hand_window: Duration,
    /// A cifra do perfil (vale quando o servidor não empurra `cipher`).
    pub cipher: Option<String>,
    /// O `data-ciphers-fallback`.
    pub cipher_fallback: Option<String>,
    /// O enquadramento da compressão do perfil.
    pub compress: Option<Compress>,
    /// O `allow-compression`.
    pub allow_compression: Option<AllowCompression>,
    /// O `explicit-exit-notify` do perfil.
    pub explicit_exit_notify: Option<u32>,
    /// O `tun-mtu`.
    pub tun_mtu: u32,
    /// O `mssfix`.
    pub mssfix: Option<MssFix>,
    /// As rotas do `.ovpn` (valem mesmo com `route-nopull`).
    pub routes: Vec<IpRoute>,
    /// Os servidores de DNS do `.ovpn` (`dhcp-option DNS`).
    pub dns_servers: Vec<SocketAddr>,
    /// Os domínios de busca do `.ovpn` (`dhcp-option DOMAIN`).
    pub search_domains: Vec<String>,
    /// O `redirect-gateway` do `.ovpn`.
    pub redirect_gateway: bool,
}

impl SessionSettings {
    /// Lê os ajustes do perfil.
    pub fn from_config(config: &Config, transport: Proto) -> SessionSettings {
        let mut dns_servers = Vec::new();
        let mut search_domains = Vec::new();
        for args in &config.dhcp_options {
            let (Some(tipo), Some(valor)) = (args.first(), args.get(1)) else {
                continue;
            };
            match tipo.to_ascii_uppercase().as_str() {
                "DNS" => {
                    if let Ok(ip) = valor.parse::<std::net::IpAddr>() {
                        dns_servers.push(SocketAddr::new(ip, 53));
                    }
                }
                "DOMAIN" | "DOMAIN-SEARCH" => search_domains.push(valor.clone()),
                _ => {}
            }
        }
        let routes = config
            .routes
            .iter()
            .filter_map(|r| parse_route(r).ok().flatten())
            .collect();
        let secs = |v: Option<u32>, default: Duration| {
            v.map_or(default, |s| Duration::from_secs(u64::from(s)))
        };
        SessionSettings {
            transport,
            ping: config.ping,
            ping_restart: config.ping_restart,
            ping_exit: config.ping_exit,
            reneg_sec: config.reneg_sec,
            reneg_bytes: config.reneg_bytes,
            reneg_pkts: config.reneg_pkts,
            replay_window: config.replay_window.map_or(64, |(p, _)| p),
            tran_window: secs(config.tran_window, Duration::from_secs(60)),
            hand_window: secs(config.hand_window, DEFAULT_HAND_WINDOW),
            cipher: config.cipher.clone(),
            cipher_fallback: config.data_ciphers_fallback.clone(),
            compress: config.compress,
            allow_compression: config.allow_compression,
            explicit_exit_notify: config.explicit_exit_notify,
            tun_mtu: config.tun_mtu.map_or(DEFAULT_TUN_MTU, u32::from),
            mssfix: config.mssfix,
            routes,
            dns_servers,
            search_domains,
            redirect_gateway: config.redirect_gateway.is_some(),
        }
    }
}

/// A rede que a sessão montou: o que a pilha precisa saber.
#[derive(Debug, Clone)]
pub struct Connected {
    /// A configuração empurrada (já filtrada pelos `pull-filter`).
    pub pushed: Pushed,
    /// O endereço do cliente dentro da VPN.
    pub address: Ipv4Addr,
    /// O prefixo do endereço.
    pub prefix_len: u8,
    /// A outra ponta (`net30`/`p2p`).
    pub peer: Option<Ipv4Addr>,
    /// As redes alcançáveis pelo túnel: as empurradas e as do `.ovpn`, mais a rota padrão com
    /// `redirect-gateway`. A rede do próprio endereço entra por conta da pilha.
    pub routes: Vec<IpRoute>,
    /// Os servidores de DNS da VPN, na ordem de prioridade.
    pub dns_servers: Vec<SocketAddr>,
    /// Os domínios de busca.
    pub search_domains: Vec<String>,
    /// Se todo o tráfego deve ir pela VPN.
    pub redirect_gateway: bool,
    /// O maior pacote IP que o túnel leva.
    pub mtu: usize,
    /// A cifra do canal de dados.
    pub cipher: DataCipher,
}

/// Por que a sessão acabou sem o servidor mandar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// A negociação do canal de controle falhou.
    Control(ControlError),
    /// O servidor pede algo que o app não faz (cifra CBC, compressão...). A mensagem orienta.
    Unsupported(String),
    /// Nada chegou do servidor no prazo do `ping-restart` (em segundos).
    PingRestart(u32),
    /// Nada chegou no prazo do `ping-exit`: a pessoa pediu para parar.
    PingExit(u32),
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::Control(e) => write!(f, "{e}"),
            Failure::Unsupported(m) => f.write_str(m),
            Failure::PingRestart(s) => write!(
                f,
                "o servidor ficou {s} s sem responder (ping-restart); reconectando"
            ),
            Failure::PingExit(s) => write!(f, "o servidor ficou {s} s sem responder (ping-exit)"),
        }
    }
}

/// O que a sessão conta a quem a usa.
#[derive(Debug)]
pub enum SessionEvent {
    /// A primeira chave do canal de dados ficou pronta: a pilha pode subir.
    Connected(Box<Connected>),
    /// Uma renegociação trocou as chaves.
    Rekeyed,
    /// O servidor espera uma confirmação de fora (login pela web, segundo fator...).
    AuthPending {
        /// Quanto o servidor espera, se disse.
        timeout: Option<Duration>,
    },
    /// O servidor recusou a autenticação. A sessão acabou.
    AuthFailed(AuthFailure),
    /// O servidor pede que se abra esta página.
    OpenUrl(String),
    /// O servidor mandou reiniciar. A sessão acabou.
    Restart(String),
    /// O servidor mandou parar. A sessão acabou.
    Halt(String),
    /// Uma informação do servidor.
    Info(String),
    /// Algo que a pessoa deve saber, sem ser erro.
    Warning(String),
    /// A sessão acabou por uma falha.
    Failed(Failure),
}

/// Uma sessão de cliente.
pub struct Session {
    control: Control,
    settings: SessionSettings,
    data: Option<DataChannel>,
    pushed: Option<Pushed>,
    events: VecDeque<SessionEvent>,
    out: VecDeque<Vec<u8>>,
    ip_in: VecDeque<Vec<u8>>,
    connected: bool,
    finished: bool,
    closing: bool,
    ping: Option<Duration>,
    ping_restart: Option<Duration>,
    ping_exit: Option<Duration>,
    last_tx: Instant,
    last_rx: Instant,
    ping_at: Option<Instant>,
    reneg_at: Option<Instant>,
    reneg_asked: bool,
    mtu: usize,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("connected", &self.connected)
            .field("finished", &self.finished)
            .finish_non_exhaustive()
    }
}

impl Session {
    /// Começa a sessão: o primeiro datagrama (o reset) sai em [`Session::poll_transmit`].
    ///
    /// # Errors
    ///
    /// Os erros de [`Control::new`].
    pub fn new(
        control: ControlOptions,
        settings: SessionSettings,
        now: Instant,
    ) -> Result<Session, ControlError> {
        let control = Control::new(control, now)?;
        Ok(Session {
            control,
            settings,
            data: None,
            pushed: None,
            events: VecDeque::new(),
            out: VecDeque::new(),
            ip_in: VecDeque::new(),
            connected: false,
            finished: false,
            closing: false,
            ping: None,
            ping_restart: None,
            ping_exit: None,
            last_tx: now,
            last_rx: now,
            ping_at: None,
            reneg_at: None,
            reneg_asked: false,
            mtu: DEFAULT_TUN_MTU as usize,
        })
    }

    /// Entrega um datagrama que chegou do servidor (já sem o prefixo do TCP).
    pub fn handle_datagram(&mut self, raw: &[u8], now: Instant) {
        if self.finished {
            return;
        }
        match self.control.handle_datagram(raw, now) {
            Datagram::Control => {
                self.last_rx = now;
            }
            Datagram::Data(packet) => self.handle_data(&packet, now),
            Datagram::Dropped => {}
        }
        self.pump(now);
    }

    fn handle_data(&mut self, packet: &[u8], now: Instant) {
        let Some(data) = self.data.as_mut() else {
            return;
        };
        match data.open(packet, now) {
            Ok(opened) => {
                self.last_rx = now;
                if let Opened::Ip(ip) = opened {
                    self.ip_in.push_back(ip);
                }
            }
            Err(DataError::Compressed(algorithm)) => {
                self.fail(Failure::Unsupported(
                    DataError::Compressed(algorithm).to_string(),
                ));
            }
            Err(_) => {}
        }
    }

    /// Cifra um pacote IP para o servidor. Sem chave ainda, o pacote é descartado (o TCP
    /// retransmite).
    pub fn send_ip(&mut self, packet: &[u8], now: Instant) {
        if self.finished || self.closing {
            return;
        }
        let Some(data) = self.data.as_mut() else {
            return;
        };
        if let Ok(datagram) = data.seal(packet) {
            self.out.push_back(datagram);
            if data.needs_renegotiation() && !self.reneg_asked {
                self.reneg_asked = true;
                self.control.renegotiate(now);
            }
        }
    }

    /// O próximo datagrama a enviar (sem o prefixo do TCP). Chame até devolver `None`.
    pub fn poll_transmit(&mut self, now: Instant) -> Option<Vec<u8>> {
        let datagram = self
            .control
            .poll_transmit(now)
            .or_else(|| self.out.pop_front());
        if datagram.is_some() {
            self.last_tx = now;
            if let Some(p) = self.ping {
                self.ping_at = Some(now + p);
            }
        }
        datagram
    }

    /// O próximo pacote IP que chegou do servidor.
    pub fn poll_ip(&mut self) -> Option<Vec<u8>> {
        self.ip_in.pop_front()
    }

    /// O próximo evento.
    pub fn poll_event(&mut self) -> Option<SessionEvent> {
        self.events.pop_front()
    }

    /// Quando chamar [`Session::handle_timeout`] e [`Session::poll_transmit`] de novo.
    pub fn next_timeout(&self) -> Option<Instant> {
        if self.finished {
            return None;
        }
        let ping_restart = self.ping_restart.map(|d| self.last_rx + d);
        let ping_exit = self.ping_exit.map(|d| self.last_rx + d);
        [
            self.control.next_timeout(),
            self.ping_at,
            ping_restart.filter(|_| self.connected),
            ping_exit.filter(|_| self.connected),
            self.reneg_at,
            self.data.as_ref().and_then(DataChannel::next_expiry),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// Avisa que o tempo passou.
    pub fn handle_timeout(&mut self, now: Instant) {
        if self.finished {
            return;
        }
        self.control.handle_timeout(now);
        if let Some(data) = self.data.as_mut() {
            data.expire(now);
        }
        if self.connected && !self.closing {
            if self.ping_at.is_some_and(|t| now >= t) {
                if let Some(data) = self.data.as_mut()
                    && let Ok(ping) = data.seal_ping()
                {
                    self.out.push_back(ping);
                }
                self.ping_at = self.ping.map(|p| now + p);
            }
            if let Some(d) = self.ping_exit
                && now >= self.last_rx + d
            {
                self.fail(Failure::PingExit(d.as_secs() as u32));
                return;
            }
            if let Some(d) = self.ping_restart
                && now >= self.last_rx + d
            {
                self.fail(Failure::PingRestart(d.as_secs() as u32));
                return;
            }
            if self.reneg_at.is_some_and(|t| now >= t) {
                self.reneg_at = Some(now + self.settings.hand_window);
                self.reneg_asked = true;
                self.control.renegotiate(now);
            }
        }
        self.pump(now);
    }

    /// Avisa o servidor de que o cliente vai sair (`explicit-exit-notify`): no UDP, o `EXIT`
    /// pelo canal de controle quando o servidor o anunciou, ou o `OCC_EXIT` no canal de dados.
    /// No TCP, fechar a conexão basta. Os datagramas saem em [`Session::poll_transmit`].
    pub fn close(&mut self, now: Instant) {
        if self.finished || self.closing || !self.connected {
            self.closing = true;
            return;
        }
        self.closing = true;
        if self.settings.transport != Proto::Udp {
            return;
        }
        let cc_exit = self
            .pushed
            .as_ref()
            .is_some_and(|p| p.protocol_flags.iter().any(|f| f == "cc-exit"));
        if cc_exit {
            self.control.send_message("EXIT");
        } else if let Some(data) = self.data.as_mut() {
            let times = self.settings.explicit_exit_notify.unwrap_or(1).clamp(1, 10);
            for _ in 0..times {
                if let Ok(d) = data.seal_exit() {
                    self.out.push_back(d);
                }
            }
        }
        let _ = now;
    }

    /// Se a sessão acabou (falha, recusa, `HALT` ou `RESTART`).
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Se as chaves de dados já existem.
    pub fn is_connected(&self) -> bool {
        self.connected
    }

    /// A senha e o `auth-token` de agora, para a próxima conexão.
    pub fn auth_state(&self) -> &AuthState {
        self.control.auth_state()
    }

    /// A configuração empurrada.
    pub fn pushed(&self) -> Option<&Pushed> {
        self.pushed.as_ref()
    }

    /// O MTU do túnel (vale depois de [`SessionEvent::Connected`]).
    pub fn mtu(&self) -> usize {
        self.mtu
    }

    /// Quantos pacotes de dados foram recusados.
    pub fn rejected_packets(&self) -> u64 {
        self.data.as_ref().map_or(0, DataChannel::rejected)
    }

    /// O canal de controle (para os testes).
    pub fn control(&self) -> &Control {
        &self.control
    }

    fn fail(&mut self, failure: Failure) {
        if self.finished {
            return;
        }
        self.finished = true;
        self.events.push_back(SessionEvent::Failed(failure));
    }

    /// Passa os eventos do canal de controle adiante e reage a eles.
    fn pump(&mut self, now: Instant) {
        while let Some(event) = self.control.poll_event() {
            if self.finished {
                continue;
            }
            match event {
                Event::Pushed(p) => self.on_pushed(*p),
                Event::DataKeys { key_id, keys } => self.on_keys(key_id, &keys, now),
                Event::AuthPending { timeout } => {
                    self.events.push_back(SessionEvent::AuthPending { timeout });
                }
                Event::AuthFailed(f) => {
                    self.finished = true;
                    self.events.push_back(SessionEvent::AuthFailed(f));
                }
                Event::OpenUrl(u) => self.events.push_back(SessionEvent::OpenUrl(u)),
                Event::Restart(t) => {
                    self.finished = true;
                    self.events.push_back(SessionEvent::Restart(t));
                }
                Event::Halt(t) => {
                    self.finished = true;
                    self.events.push_back(SessionEvent::Halt(t));
                }
                Event::Info(t) => self.events.push_back(SessionEvent::Info(t)),
                Event::Warning(t) => self.events.push_back(SessionEvent::Warning(t)),
                Event::Failed(e) => self.fail(Failure::Control(e)),
            }
        }
    }

    fn on_pushed(&mut self, pushed: Pushed) {
        let settings = &self.settings;
        let name = pushed
            .cipher
            .clone()
            .or_else(|| settings.cipher.clone())
            .or_else(|| settings.cipher_fallback.clone());
        let Some(name) = name else {
            self.fail(Failure::Unsupported(
                "o servidor não escolheu a cifra do canal de dados e o perfil não tem cipher; \
                 o app só fala AES-GCM e ChaCha20-Poly1305"
                    .to_owned(),
            ));
            return;
        };
        let Some(cipher) = DataCipher::from_name(&name) else {
            self.fail(Failure::Unsupported(format!(
                "a cifra {name} do canal de dados não é suportada; peça ao administrador \
                 data-ciphers AES-256-GCM:AES-128-GCM:CHACHA20-POLY1305"
            )));
            return;
        };
        let compress = pushed.compress.or(settings.compress);
        if let Some(c) = compress
            && matches!(c, Compress::Lzo | Compress::Lz4 | Compress::Lz4V2)
        {
            // O app não comprime; o servidor ainda pode mandar pacotes não comprimidos com o
            // enquadramento certo, e os comprimidos viram um erro claro.
            if settings.allow_compression == Some(AllowCompression::No) {
                self.fail(Failure::Unsupported(
                    "o servidor exige compressão e o perfil tem allow-compression no".to_owned(),
                ));
                return;
            }
        }
        let tran = settings.tran_window;
        let options = DataOptions {
            cipher,
            peer_id: pushed.peer_id,
            compress,
            replay_window: settings.replay_window,
            tran_window: tran,
            reneg_bytes: settings.reneg_bytes,
            reneg_pkts: settings.reneg_pkts,
        };
        let seconds = |v: Option<u32>| {
            v.filter(|s| *s > 0)
                .map(|s| Duration::from_secs(u64::from(s)))
        };
        self.ping = seconds(pushed.ping.or(settings.ping));
        self.ping_restart = seconds(pushed.ping_restart.or(settings.ping_restart));
        self.ping_exit = seconds(pushed.ping_exit.or(settings.ping_exit));
        self.mtu = self.compute_mtu(&options, &pushed);
        self.data = Some(DataChannel::new(options));
        self.pushed = Some(pushed);
    }

    fn compute_mtu(&self, options: &DataOptions, pushed: &Pushed) -> usize {
        let tun = pushed.tun_mtu.unwrap_or(self.settings.tun_mtu) as usize;
        let overhead = {
            let probe = DataChannel::new(options.clone());
            probe.overhead()
        };
        let limit = if self.settings.transport == Proto::Udp {
            match self.settings.mssfix {
                Some(MssFix::Off) => None,
                Some(MssFix::Value(v)) => Some(usize::from(v)),
                Some(MssFix::Auto) | None => Some(DEFAULT_MSSFIX as usize),
            }
        } else {
            None
        };
        let mut mtu = tun;
        if let Some(l) = limit {
            mtu = mtu.min(l.saturating_sub(overhead));
        }
        mtu.max(MIN_MTU)
    }

    fn on_keys(&mut self, key_id: u8, keys: &DataKeys, now: Instant) {
        let Some(data) = self.data.as_mut() else {
            self.fail(Failure::Unsupported(
                "chegaram chaves do canal de dados antes da configuração do servidor".to_owned(),
            ));
            return;
        };
        if data.install(key_id, keys, now).is_err() {
            self.fail(Failure::Unsupported(
                "as chaves do canal de dados não servem para a cifra escolhida".to_owned(),
            ));
            return;
        }
        self.reneg_asked = false;
        self.reneg_at = self.reneg_deadline(now);
        if !self.connected {
            self.connected = true;
            self.last_rx = now;
            self.ping_at = self.ping.map(|p| now + p);
            if let Some(connected) = self.build_connected() {
                self.events
                    .push_back(SessionEvent::Connected(Box::new(connected)));
            } else {
                self.fail(Failure::Unsupported(
                    "o servidor não empurrou o endereço do cliente (ifconfig); o app não \
                     consegue usar a VPN sem ele"
                        .to_owned(),
                ));
            }
        } else {
            self.events.push_back(SessionEvent::Rekeyed);
        }
    }

    /// O próximo `reneg-sec`, de 90 % a 100 % do prazo, para os dois lados não começarem juntos.
    fn reneg_deadline(&self, now: Instant) -> Option<Instant> {
        let secs = self.settings.reneg_sec.unwrap_or(DEFAULT_RENEG_SEC);
        if secs == 0 {
            return None;
        }
        let mut byte = [0u8; 1];
        let _ = getrandom::fill(&mut byte);
        let permille = 900 + u64::from(byte[0]) * 100 / 255;
        Some(now + Duration::from_millis(u64::from(secs) * permille))
    }

    fn build_connected(&self) -> Option<Connected> {
        let pushed = self.pushed.as_ref()?;
        let ifconfig = pushed.ifconfig?;
        let mut routes = pushed.effective_routes();
        for r in &self.settings.routes {
            if !routes.contains(r) {
                routes.push(*r);
            }
        }
        let redirect = pushed.redirect_gateway || self.settings.redirect_gateway;
        if self.settings.redirect_gateway && !pushed.redirect_gateway {
            let default = IpRoute {
                network: Ipv4Addr::UNSPECIFIED,
                prefix_len: 0,
            };
            if !routes.contains(&default) {
                routes.push(default);
            }
        }
        let mut dns = pushed.dns_servers.clone();
        for d in &self.settings.dns_servers {
            if !dns.contains(d) {
                dns.push(*d);
            }
        }
        let mut search = pushed.search_domains.clone();
        for d in &self.settings.search_domains {
            if !search.contains(d) {
                search.push(d.clone());
            }
        }
        Some(Connected {
            pushed: pushed.clone(),
            address: ifconfig.address,
            prefix_len: ifconfig.prefix_len,
            peer: ifconfig.peer,
            routes,
            dns_servers: dns,
            search_domains: search,
            redirect_gateway: redirect,
            mtu: self.mtu,
            cipher: self.data.as_ref()?.options().cipher,
        })
    }
}
