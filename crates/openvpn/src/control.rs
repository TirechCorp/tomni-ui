//! O canal de controle do cliente OpenVPN.
//!
//! Faz o que vem depois do `.ovpn` e antes do túnel:
//!
//! 1. o reset da sessão (`HARD_RESET`) e a confiabilidade das mensagens;
//! 2. o TLS por dentro das mensagens de controle (`rustls`), com o certificado do servidor
//!    conferido pelo [`crate::verify`];
//! 3. a troca de chaves (key-method 2): usuário, senha, `peer info` e os aleatórios;
//! 4. o `PUSH_REQUEST` e o `PUSH_REPLY`, com os `pull-filter`;
//! 5. as chaves do canal de dados, pelo exportador do TLS ou pela PRF do protocolo;
//! 6. as renegociações (`SOFT_RESET`) e o `auth-token` que as autentica.
//!
//! O [`Control`] não faz E/S nem lê o relógio sozinho: o chamador entrega os datagramas que
//! chegam ([`Control::handle_datagram`]), retira os que devem sair ([`Control::poll_transmit`]) e
//! avisa quando o tempo passa ([`Control::handle_timeout`], no instante de
//! [`Control::next_timeout`]). O que a negociação descobre sai como [`Event`].
//!
//! Sobre o TCP, quem chama cuida do prefixo de 2 bytes ([`crate::packet::tcp_frame`] e
//! [`crate::packet::TcpDeframer`]); aqui cada datagrama é um pacote inteiro.

use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rustls::client::Resumption;
use rustls::crypto::CryptoProvider;
use rustls::pki_types::ServerName;
use rustls::sign::CertifiedKey;
use rustls::{AlertDescription, ClientConfig, SupportedProtocolVersion, version};
use zeroize::Zeroizing;

use crate::auth::{AuthFailure, AuthState, ServerMessage, parse_server_message};
pub use crate::channel::{Clock, Datagram};
use crate::channel::{Core, CoreError, TlsSide};
use crate::config::{
    AuthDigest, AuthUserPass, Config, Proto, PullFilter, ServerVerify, TlsVersion,
};
use crate::keymethod::{
    ClientKeyExchange, KeyMethodError, OptionsParams, PeerInfo, ServerKeyExchange, options_string,
    proto_flags,
};
use crate::packet::{MAX_KEY_ID, Opcode, SessionId};
use crate::pki::{CertInfo, Pki, Wrap};
use crate::prf::{DataKeys, EKM_LABEL, KEY_BLOCK_LEN, PrfInputs, derive_key_block};
use crate::push::{PushReader, Pushed};
use crate::reliable::ReliableConfig;
use crate::static_key::{Direction, StaticKey};
use crate::verify::{ServerVerifier, VerifyError};
use crate::wrap::{Role, Wrapper};

/// O `tls-mtu` padrão: quanto cabe num pacote de controle, com o cabeçalho e a proteção.
///
/// Confira: é o valor que a documentação do OpenVPN dá como padrão (1250).
pub const DEFAULT_TLS_MTU: usize = 1250;

/// O `hand-window` padrão: quanto dura a negociação de uma geração de chaves. Vem da
/// documentação do OpenVPN (60 segundos).
pub const DEFAULT_HAND_WINDOW: Duration = Duration::from_secs(60);

/// O `tran-window` padrão: quanto a chave antiga ainda vale depois de uma renegociação. Vem da
/// documentação do OpenVPN (60 segundos).
pub const DEFAULT_TRAN_WINDOW: Duration = Duration::from_secs(60);

/// O `replay-window` padrão, em pacotes (64, da documentação do OpenVPN).
pub const DEFAULT_REPLAY_WINDOW: u32 = 64;

/// O intervalo do `PUSH_REQUEST`.
///
/// Um servidor que recebe `IV_PROTO` com `REQUEST_PUSH` manda o `PUSH_REPLY` sozinho; o pedido só
/// serve de rede de segurança para um servidor que não o faça. O valor não vem do protocolo (é
/// uma escolha deste motor, em aberto): um segundo dá tempo ao servidor sem atrasar a conexão.
pub const PUSH_REQUEST_INTERVAL: Duration = Duration::from_secs(1);

/// As cifras que o cliente anuncia (`IV_CIPHERS`) quando o `.ovpn` não tem `data-ciphers`: o
/// padrão do OpenVPN 2.6.
pub const DEFAULT_DATA_CIPHERS: [&str; 3] = ["AES-256-GCM", "AES-128-GCM", "CHACHA20-POLY1305"];

/// O nome que se dá ao `rustls` quando o `.ovpn` não tem um endereço utilizável. Ele não vai
/// pela rede (o SNI fica desligado) e não é conferido.
const FALLBACK_SERVER_NAME: &str = "openvpn.server";

/// O `tun-mtu` padrão do OpenVPN (1500).
const DEFAULT_TUN_MTU: u32 = 1500;

/// O que o `link-mtu` soma ao `tun-mtu` no texto de opções para AES-GCM sobre UDP. O servidor só
/// compara o texto com o dele e avisa; não recusa. Confira com um servidor real.
const LINK_MTU_OVERHEAD: u32 = 49;

/// A identidade que o cliente declara no `peer info`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// O nível de recursos (`IV_VER`). Servidores comparam com versões do OpenVPN para ligar
    /// recursos, então este é o do OpenVPN 2.6, que o motor implementa.
    pub version: String,
    /// O sistema (`IV_PLAT`): `linux`, `win`, `mac`...
    pub platform: String,
    /// O programa e a versão (`IV_GUI_VER`).
    pub gui: Option<String>,
}

impl Default for Identity {
    fn default() -> Self {
        Identity {
            version: "2.6.0".to_owned(),
            platform: match std::env::consts::OS {
                "linux" => "linux",
                "windows" => "win",
                "macos" => "mac",
                "android" => "android",
                _ => "other",
            }
            .to_owned(),
            gui: Some(format!("tomni-ui-openvpn_{}", env!("CARGO_PKG_VERSION"))),
        }
    }
}

/// Tudo o que o canal de controle precisa do perfil.
#[derive(Debug, Clone)]
pub struct ControlOptions {
    /// UDP ou TCP.
    pub transport: Proto,
    /// Se o endereço do servidor é IPv6 (só muda o nome no texto de opções).
    pub ipv6: bool,
    /// Os certificados, a chave e a proteção do canal de controle.
    pub pki: Pki,
    /// O que conferir no certificado do servidor.
    pub verify: ServerVerify,
    /// O usuário, a senha e o `auth-token`.
    pub auth: AuthState,
    /// O digest do `tls-auth` e do HMAC dos dados (`auth`).
    pub auth_digest: AuthDigest,
    /// O `tls-version-min`.
    pub tls_min: Option<TlsVersion>,
    /// O `tls-version-max`.
    pub tls_max: Option<TlsVersion>,
    /// O `tls-mtu`.
    pub tls_mtu: usize,
    /// O `hand-window`.
    pub hand_window: Duration,
    /// O `tran-window`.
    pub tran_window: Duration,
    /// O `replay-window` do `tls-auth`/`tls-crypt`, em pacotes.
    pub replay_window: u32,
    /// O `cipher` do `.ovpn` (para o texto de opções).
    pub cipher: Option<String>,
    /// As cifras anunciadas (`IV_CIPHERS`).
    pub data_ciphers: Vec<String>,
    /// O `tun-mtu`.
    pub tun_mtu: u32,
    /// O `link-mtu`.
    pub link_mtu: u32,
    /// Os `pull-filter`.
    pub pull_filters: Vec<PullFilter>,
    /// O `route-nopull`.
    pub route_nopull: bool,
    /// Quem somos para o servidor.
    pub identity: Identity,
    /// O relógio.
    pub clock: Clock,
    /// O endereço do servidor (só dá nome ao `rustls`; não é enviado nem conferido).
    pub server_name: String,
}

impl ControlOptions {
    /// Monta as opções a partir do perfil. `auth` traz o que a pessoa digitou (ou o que o perfil
    /// já tem); com `auth-user-pass` no perfil e `auth` vazio, é um erro.
    pub fn from_config(
        config: &Config,
        transport: Proto,
        ipv6: bool,
        auth: AuthState,
        clock: Clock,
    ) -> Result<ControlOptions, ControlError> {
        if config.auth_user_pass != AuthUserPass::None && auth.is_empty() {
            return Err(ControlError::Setup(
                "o perfil pede usuário e senha (auth-user-pass); informe-os antes de conectar"
                    .into(),
            ));
        }
        if config.pki.cert.is_some() != config.pki.key.is_some() {
            return Err(ControlError::Setup(
                "o certificado e a chave do cliente precisam vir juntos (cert e key)".into(),
            ));
        }
        let tun_mtu = config.tun_mtu.map_or(DEFAULT_TUN_MTU, u32::from);
        let link_mtu = config
            .link_mtu
            .map_or(tun_mtu + LINK_MTU_OVERHEAD, u32::from);
        let data_ciphers = match &config.data_ciphers {
            Some(l) if !l.is_empty() => l.clone(),
            _ => DEFAULT_DATA_CIPHERS
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
        };
        let seconds = |v: Option<u32>, default: Duration| {
            v.map_or(default, |s| Duration::from_secs(u64::from(s)))
        };
        Ok(ControlOptions {
            transport,
            ipv6,
            pki: config.pki.clone(),
            verify: config.verify.clone(),
            auth,
            auth_digest: config.auth_digest.unwrap_or(AuthDigest::Sha1),
            tls_min: config.tls_version_min,
            tls_max: config.tls_version_max,
            tls_mtu: config.tls_mtu.map_or(DEFAULT_TLS_MTU, usize::from),
            hand_window: seconds(config.hand_window, DEFAULT_HAND_WINDOW),
            tran_window: seconds(config.tran_window, DEFAULT_TRAN_WINDOW),
            replay_window: config
                .replay_window
                .map_or(DEFAULT_REPLAY_WINDOW, |(packets, _)| packets),
            cipher: config.cipher.clone(),
            data_ciphers,
            tun_mtu,
            link_mtu,
            pull_filters: config.pull_filters.clone(),
            route_nopull: config.route_nopull,
            identity: Identity::default(),
            clock,
            server_name: config
                .remotes
                .first()
                .map_or_else(|| FALLBACK_SERVER_NAME.to_owned(), |r| r.host.clone()),
        })
    }

    fn proto_name(&self) -> &'static str {
        match (self.transport, self.ipv6) {
            (Proto::Udp, false) => "UDPv4",
            (Proto::Udp, true) => "UDPv6",
            (Proto::Tcp, false) => "TCPv4_CLIENT",
            (Proto::Tcp, true) => "TCPv6_CLIENT",
        }
    }
}

/// O que dá errado no canal de controle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlError {
    /// As opções não servem (falta a chave, o perfil pede senha...).
    Setup(String),
    /// O certificado do servidor foi recusado.
    Verify(VerifyError),
    /// O servidor recusou o certificado do cliente.
    ClientCertificateRejected(String),
    /// O TLS falhou.
    Tls(String),
    /// A troca de chaves veio malformada.
    KeyMethod(KeyMethodError),
    /// O `PUSH_REPLY` veio malformado ou foi recusado por um `pull-filter reject`.
    Push(String),
    /// A negociação não terminou no `hand-window`.
    HandshakeTimeout {
        /// A duração da janela.
        seconds: u64,
        /// Se algum pacote válido do servidor chegou.
        heard_server: bool,
    },
    /// O servidor fechou o TLS.
    Closed,
}

impl fmt::Display for ControlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Setup(m) => f.write_str(m),
            Self::Verify(e) => e.fmt(f),
            Self::ClientCertificateRejected(alert) => write!(
                f,
                "o servidor não aceitou o certificado do cliente ({alert}); confira se ele foi emitido pela CA do servidor e se não venceu"
            ),
            Self::Tls(m) => write!(f, "falha no TLS do canal de controle: {m}"),
            Self::KeyMethod(e) => e.fmt(f),
            Self::Push(m) => write!(f, "a configuração empurrada pelo servidor não serve: {m}"),
            Self::HandshakeTimeout {
                seconds,
                heard_server: false,
            } => write!(
                f,
                "o servidor não respondeu em {seconds} s; confira o endereço, a porta, o protocolo (UDP ou TCP), o firewall e, se o .ovpn tem tls-auth, tls-crypt ou tls-crypt-v2, a chave"
            ),
            Self::HandshakeTimeout {
                seconds,
                heard_server: true,
            } => write!(f, "a negociação com o servidor não terminou em {seconds} s"),
            Self::Closed => f.write_str("o servidor encerrou o canal de controle"),
        }
    }
}

impl std::error::Error for ControlError {}

impl From<VerifyError> for ControlError {
    fn from(e: VerifyError) -> Self {
        ControlError::Verify(e)
    }
}

impl From<KeyMethodError> for ControlError {
    fn from(e: KeyMethodError) -> Self {
        ControlError::KeyMethod(e)
    }
}

/// Converte um erro do `rustls` na mensagem que a pessoa entende.
pub(crate) fn map_tls_error(error: &rustls::Error) -> ControlError {
    if let Some(v) = VerifyError::from_tls(error) {
        return ControlError::Verify(v);
    }
    match error {
        rustls::Error::AlertReceived(
            a @ (AlertDescription::BadCertificate
            | AlertDescription::UnsupportedCertificate
            | AlertDescription::CertificateRevoked
            | AlertDescription::CertificateExpired
            | AlertDescription::CertificateUnknown
            | AlertDescription::UnknownCA
            | AlertDescription::CertificateRequired
            | AlertDescription::AccessDenied),
        ) => ControlError::ClientCertificateRejected(format!("{a:?}")),
        e => ControlError::Tls(e.to_string()),
    }
}

/// O que a negociação descobre.
#[derive(Debug)]
pub enum Event {
    /// O servidor empurrou a configuração (endereço, rotas, DNS...).
    Pushed(Box<Pushed>),
    /// As chaves do canal de dados de uma geração, prontas para uso.
    DataKeys {
        /// O `key_id` da geração.
        key_id: u8,
        /// As chaves.
        keys: DataKeys,
    },
    /// O servidor espera uma confirmação de fora (login pela web, segundo fator...).
    AuthPending {
        /// Quanto o servidor espera, se disse.
        timeout: Option<Duration>,
    },
    /// O servidor recusou a autenticação. A sessão acabou.
    AuthFailed(AuthFailure),
    /// O servidor pede que se abra esta página.
    OpenUrl(String),
    /// O servidor mandou reiniciar a conexão. A sessão acabou.
    Restart(String),
    /// O servidor mandou parar. A sessão acabou.
    Halt(String),
    /// Uma informação do servidor.
    Info(String),
    /// Algo que a pessoa deve saber, sem ser erro.
    Warning(String),
    /// A negociação falhou. A sessão acabou.
    Failed(ControlError),
}

/// O canal de controle de um cliente.
pub struct Control {
    core: Core,
    seen: Arc<Mutex<Option<CertInfo>>>,
    auth: AuthState,
    filters: Vec<PullFilter>,
    route_nopull: bool,
    push_reader: PushReader,
    pushed: Option<Pushed>,
    ekm: bool,
    events: VecDeque<Event>,
    push_timer: Option<Instant>,
    current_key: Option<u8>,
    finished: bool,
    tran_window: Duration,
    options_text: String,
    peer_info_text: String,
}

impl fmt::Debug for Control {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Control")
            .field("slots", &self.core.slots.len())
            .field("finished", &self.finished)
            .finish_non_exhaustive()
    }
}

impl Control {
    /// Começa a negociação: o primeiro datagrama (o reset) sai em [`Control::poll_transmit`].
    pub fn new(options: ControlOptions, now: Instant) -> Result<Control, ControlError> {
        let verifier = ServerVerifier::new(&options.pki.ca, &options.verify)?;
        let mut events = VecDeque::new();
        for w in verifier.warnings() {
            events.push_back(Event::Warning(w));
        }
        let seen = verifier.certificate_handle();
        let config = client_config(&options, verifier)?;
        let name = ServerName::try_from(options.server_name.clone())
            .or_else(|_| ServerName::try_from(FALLBACK_SERVER_NAME.to_owned()))
            .map_err(|e| ControlError::Setup(format!("nome do servidor inválido: {e}")))?;
        let wrapper = build_wrapper(
            &options.pki,
            Role::Client,
            options.auth_digest,
            &options.clock,
            options.replay_window,
        )?;
        let reliable = match options.transport {
            Proto::Udp => ReliableConfig::udp(),
            Proto::Tcp => ReliableConfig::tcp(),
        };
        let first = if matches!(options.pki.wrap, Wrap::TlsCryptV2 { .. }) {
            Opcode::HardResetClientV3
        } else {
            Opcode::HardResetClientV2
        };
        let mut core = Core::new(
            Role::Client,
            wrapper,
            reliable,
            TlsSide::Client(config, name),
            options.tls_mtu,
            options.hand_window,
        );
        core.open_slot(0, first, true, now)
            .map_err(|e| ControlError::Tls(e.to_string()))?;

        let options_text = options_string(&OptionsParams {
            dev_type: "tun",
            link_mtu: options.link_mtu,
            tun_mtu: options.tun_mtu,
            proto: options.proto_name(),
            cipher: options.cipher.as_deref(),
            auth: match options.auth_digest {
                AuthDigest::None => None,
                AuthDigest::Sha1 => Some("SHA1"),
                AuthDigest::Sha256 => Some("SHA256"),
                AuthDigest::Sha384 => Some("SHA384"),
                AuthDigest::Sha512 => Some("SHA512"),
            },
            keysize: None,
        });
        let peer_info_text = PeerInfo {
            version: options.identity.version.clone(),
            platform: options.identity.platform.clone(),
            proto: proto_flags::DATA_V2
                | proto_flags::REQUEST_PUSH
                | proto_flags::TLS_KEY_EXPORT
                | proto_flags::AUTH_PENDING_KW
                | proto_flags::CC_EXIT_NOTIFY
                | proto_flags::AUTH_FAIL_TEMP
                | proto_flags::DNS_OPTION_V2,
            ciphers: options.data_ciphers.clone(),
            gui_version: options.identity.gui.clone(),
            mtu: None,
        }
        .to_text();

        Ok(Control {
            core,
            seen,
            auth: options.auth.clone(),
            push_reader: PushReader::new(&options.pull_filters, options.route_nopull),
            filters: options.pull_filters.clone(),
            route_nopull: options.route_nopull,
            pushed: None,
            ekm: false,
            events,
            push_timer: None,
            current_key: None,
            finished: false,
            tran_window: options.tran_window,
            options_text,
            peer_info_text,
        })
    }

    /// Entrega um datagrama que chegou do servidor (já sem o prefixo do TCP).
    pub fn handle_datagram(&mut self, raw: &[u8], now: Instant) -> Datagram {
        if self.finished {
            return Datagram::Dropped;
        }
        match self.core.handle_datagram(raw, now) {
            Ok(d) => {
                self.advance(now);
                d
            }
            Err(CoreError::Tls(e)) => {
                self.fail(map_tls_error(&e));
                Datagram::Control
            }
        }
    }

    /// O próximo datagrama a enviar (sem o prefixo do TCP). Chame até devolver `None`.
    pub fn poll_transmit(&mut self, now: Instant) -> Option<Vec<u8>> {
        self.core.poll_transmit(now)
    }

    /// O próximo [`Event`].
    pub fn poll_event(&mut self) -> Option<Event> {
        self.events.pop_front()
    }

    /// Quando chamar [`Control::handle_timeout`] e [`Control::poll_transmit`] de novo.
    pub fn next_timeout(&self) -> Option<Instant> {
        if self.finished {
            return None;
        }
        [self.core.next_timeout(), self.push_timer]
            .into_iter()
            .flatten()
            .min()
    }

    /// Avisa que o tempo passou: vence a janela de negociação, a chave antiga se aposenta e o
    /// `PUSH_REQUEST` se repete.
    pub fn handle_timeout(&mut self, now: Instant) {
        if self.finished {
            return;
        }
        let seconds = self.core.hand_window.as_secs();
        let heard_server = self.core.heard;
        let mut failed = false;
        let mut lost = Vec::new();
        for slot in &mut self.core.slots {
            if slot.deadline.is_some_and(|t| now >= t) {
                slot.deadline = None;
                if slot.initial {
                    failed = true;
                } else {
                    lost.push(slot.key_id);
                }
            }
        }
        if failed {
            self.fail(ControlError::HandshakeTimeout {
                seconds,
                heard_server,
            });
            return;
        }
        for key_id in lost {
            self.core.slots.retain(|s| s.key_id != key_id);
            self.events.push_back(Event::Warning(format!(
                "a renegociação das chaves não terminou em {seconds} s; a conexão segue com a chave anterior"
            )));
        }
        self.core.slots.retain(|s| !s.is_retired(now));

        if self.push_timer.is_some_and(|t| now >= t)
            && let Some(i) = self
                .core
                .slots
                .iter()
                .position(|s| s.initial && s.server_kx.is_some() && !s.keys_done)
        {
            self.push_timer = Some(now + PUSH_REQUEST_INTERVAL);
            if let Err(e) = self.core.write_plain(i, b"PUSH_REQUEST\0") {
                self.fail(map_tls_error(&e));
            }
        }
    }

    /// Começa uma renegociação das chaves (`SOFT_RESET`). Sem efeito antes das primeiras chaves.
    pub fn renegotiate(&mut self, now: Instant) {
        let Some(current) = self.current_key else {
            return;
        };
        if self.finished {
            return;
        }
        let key_id = next_key_id(current);
        if self.core.slot_index(key_id).is_some() {
            return;
        }
        if let Err(e) = self
            .core
            .open_slot(key_id, Opcode::ControlSoftResetV1, false, now)
        {
            self.fail(map_tls_error(&e));
        }
    }

    /// Manda uma mensagem de texto ao servidor pelo canal de controle (o NUL final é posto aqui).
    pub fn send_message(&mut self, text: &str) {
        let Some(i) = self.core.slots.iter().rposition(|s| s.keys_done) else {
            return;
        };
        let mut bytes = text.as_bytes().to_vec();
        bytes.push(0);
        if let Err(e) = self.core.write_plain(i, &bytes) {
            self.fail(map_tls_error(&e));
        }
    }

    /// A senha e o `auth-token` de agora. Guarde-os para a próxima conexão: sem o token
    /// (descartado quando o servidor o recusa), vale a senha.
    pub fn auth_state(&self) -> &AuthState {
        &self.auth
    }

    /// A configuração que o servidor empurrou, depois do `PUSH_REPLY`.
    pub fn pushed(&self) -> Option<&Pushed> {
        self.pushed.as_ref()
    }

    /// O certificado do servidor que foi aceito.
    pub fn server_certificate(&self) -> Option<CertInfo> {
        self.seen.lock().ok().and_then(|g| g.clone())
    }

    /// A sessão do cliente.
    pub fn local_session_id(&self) -> SessionId {
        self.core.local_sid
    }

    /// A sessão do servidor, depois do primeiro pacote dele.
    pub fn remote_session_id(&self) -> Option<SessionId> {
        self.core.remote_sid
    }

    /// Quantos datagramas foram descartados (inválidos, repetidos, de outra sessão).
    pub fn dropped_datagrams(&self) -> u64 {
        self.core.dropped
    }

    /// Se a sessão acabou (falha, recusa, `HALT` ou `RESTART`).
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    fn fail(&mut self, error: ControlError) {
        self.events.push_back(Event::Failed(error));
        self.finished = true;
    }

    fn advance(&mut self, now: Instant) {
        let mut i = 0;
        while i < self.core.slots.len() && !self.finished {
            self.advance_slot(i, now);
            i += 1;
        }
    }

    fn advance_slot(&mut self, i: usize, now: Instant) {
        // 1. O TLS terminou: manda a nossa troca de chaves.
        {
            let slot = &self.core.slots[i];
            if slot.tls_enabled && !slot.kx_sent && !slot.tls.is_handshaking() {
                let (user, password) = self.auth.wire();
                let kx = ClientKeyExchange::random(
                    self.options_text.clone(),
                    user,
                    password,
                    self.peer_info_text.clone(),
                );
                let bytes = match kx.encode() {
                    Ok(b) => b,
                    Err(e) => return self.fail(e.into()),
                };
                if let Err(e) = self.core.write_plain(i, &bytes) {
                    return self.fail(map_tls_error(&e));
                }
                let slot = &mut self.core.slots[i];
                slot.client_kx = Some(kx);
                slot.kx_sent = true;
            }
        }
        // 2. A troca de chaves do servidor.
        if self.core.slots[i].kx_sent && self.core.slots[i].server_kx.is_none() {
            let slot = &mut self.core.slots[i];
            match ServerKeyExchange::parse(&slot.plain_in) {
                Ok((kx, used)) => {
                    slot.plain_in.drain(..used);
                    slot.server_kx = Some(kx);
                    if slot.initial {
                        if self.pushed.is_none() {
                            self.push_timer = Some(now + PUSH_REQUEST_INTERVAL);
                        }
                    } else if let Err(e) = self.finish_keys(i, now) {
                        return self.fail(e);
                    }
                }
                Err(KeyMethodError::Incomplete) => {}
                Err(e) => return self.fail(e.into()),
            }
        }
        // 3. As mensagens de texto, terminadas em NUL.
        while self.core.slots[i].server_kx.is_some() && !self.finished {
            let slot = &mut self.core.slots[i];
            let Some(pos) = slot.plain_in.iter().position(|b| *b == 0) else {
                break;
            };
            let raw: Vec<u8> = slot.plain_in.drain(..=pos).collect();
            let text = String::from_utf8_lossy(&raw[..pos]).into_owned();
            self.handle_message(i, &text, now);
        }
        if !self.finished && self.core.slots[i].peer_closed && self.core.slots[i].initial {
            self.fail(ControlError::Closed);
        }
    }

    fn handle_message(&mut self, i: usize, text: &str, now: Instant) {
        match parse_server_message(text) {
            ServerMessage::PushReply(reply) => match self.push_reader.feed(&reply) {
                Ok(Some(pushed)) => {
                    self.push_reader = PushReader::new(&self.filters, self.route_nopull);
                    if let Err(e) = self.on_pushed(i, pushed, now) {
                        self.fail(e);
                    }
                }
                Ok(None) => {}
                Err(e) => self.fail(ControlError::Push(e.to_string())),
            },
            ServerMessage::AuthFailed(failure) => {
                if matches!(failure, AuthFailure::Session { .. }) {
                    self.auth.drop_token();
                }
                self.events.push_back(Event::AuthFailed(failure));
                self.finished = true;
            }
            ServerMessage::AuthPending { timeout } => {
                self.push_timer = None;
                let wait = timeout.unwrap_or(self.core.hand_window);
                self.core.slots[i].deadline = Some(now + wait);
                self.events.push_back(Event::AuthPending { timeout });
            }
            ServerMessage::Restart(t) => {
                self.events.push_back(Event::Restart(t));
                self.finished = true;
            }
            ServerMessage::Halt(t) => {
                self.events.push_back(Event::Halt(t));
                self.finished = true;
            }
            ServerMessage::OpenUrl(url) => self.events.push_back(Event::OpenUrl(url)),
            ServerMessage::Info(t) => self.events.push_back(Event::Info(t)),
            ServerMessage::Other(_) => {}
        }
    }

    fn on_pushed(&mut self, i: usize, pushed: Pushed, now: Instant) -> Result<(), ControlError> {
        self.ekm =
            pushed.key_derivation_ekm || pushed.protocol_flags.iter().any(|f| f == "tls-ekm");
        if let Some(token) = &pushed.auth_token {
            self.auth
                .set_token(token.clone(), pushed.auth_token_user.clone());
        }
        self.push_timer = None;
        self.events
            .push_back(Event::Pushed(Box::new(pushed.clone())));
        self.pushed = Some(pushed);
        if !self.core.slots[i].keys_done {
            self.finish_keys(i, now)?;
        }
        Ok(())
    }

    /// Deriva as chaves de dados da geração e entrega.
    fn finish_keys(&mut self, i: usize, now: Instant) -> Result<(), ControlError> {
        let keys = self.derive_keys(i)?;
        let key_id = self.core.slots[i].key_id;
        {
            let slot = &mut self.core.slots[i];
            slot.keys_done = true;
            slot.deadline = None;
        }
        // As gerações anteriores valem só pelo `tran-window`.
        for slot in &mut self.core.slots {
            if slot.key_id != key_id && slot.retire_at.is_none() {
                slot.retire_at = Some(now + self.tran_window);
            }
        }
        self.current_key = Some(key_id);
        self.events.push_back(Event::DataKeys { key_id, keys });
        Ok(())
    }

    fn derive_keys(&self, i: usize) -> Result<DataKeys, ControlError> {
        let slot = &self.core.slots[i];
        let block: Zeroizing<[u8; KEY_BLOCK_LEN]> = if self.ekm {
            let mut out = Zeroizing::new([0u8; KEY_BLOCK_LEN]);
            slot.tls
                .export_keying_material(&mut out[..], EKM_LABEL.as_bytes(), None)
                .map_err(|e| map_tls_error(&e))?;
            out
        } else {
            let (Some(client), Some(server), Some(remote)) =
                (&slot.client_kx, &slot.server_kx, self.core.remote_sid)
            else {
                return Err(ControlError::Tls(
                    "faltou a troca de chaves para derivar as chaves de dados".into(),
                ));
            };
            derive_key_block(&PrfInputs {
                pre_master: &client.pre_master,
                client_random1: &client.random1,
                server_random1: &server.random1,
                client_random2: &client.random2,
                server_random2: &server.random2,
                client_session: self.core.local_sid,
                server_session: remote,
            })
        };
        Ok(DataKeys::from_block(&block, Role::Client))
    }
}

/// O `key_id` seguinte: de 7 volta para 1 (o 0 só vale na primeira geração da sessão).
pub(crate) fn next_key_id(current: u8) -> u8 {
    if current >= MAX_KEY_ID {
        1
    } else {
        current + 1
    }
}

/// As versões do TLS que o `.ovpn` permite.
pub(crate) fn tls_versions(
    min: Option<TlsVersion>,
    max: Option<TlsVersion>,
) -> Result<Vec<&'static SupportedProtocolVersion>, ControlError> {
    let min = min.unwrap_or(TlsVersion::V1_2);
    let max = max.unwrap_or(TlsVersion::V1_3);
    let mut out = Vec::new();
    if min == TlsVersion::V1_2 {
        out.push(&version::TLS12);
    }
    if max == TlsVersion::V1_3 {
        out.push(&version::TLS13);
    }
    if out.is_empty() {
        return Err(ControlError::Setup(
            "tls-version-min e tls-version-max não deixam nenhuma versão do TLS".into(),
        ));
    }
    Ok(out)
}

fn client_config(
    options: &ControlOptions,
    verifier: ServerVerifier,
) -> Result<Arc<ClientConfig>, ControlError> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let tls = |e: rustls::Error| ControlError::Tls(e.to_string());
    let builder = ClientConfig::builder_with_details(
        provider as Arc<CryptoProvider>,
        Arc::new(options.clock.clone()),
    )
    .with_protocol_versions(&tls_versions(options.tls_min, options.tls_max)?)
    .map_err(tls)?
    .dangerous()
    .with_custom_certificate_verifier(Arc::new(verifier));
    let mut config = match (&options.pki.cert, &options.pki.key) {
        (Some(chain), Some(key)) => {
            let signer = key
                .signing_key()
                .map_err(|e| ControlError::Setup(e.to_string()))?;
            let certified = CertifiedKey::new(chain.clone(), signer);
            builder.with_client_cert_resolver(Arc::new(rustls::sign::SingleCertAndKey::from(
                certified,
            )))
        }
        _ => builder.with_no_client_auth(),
    };
    config.enable_sni = false;
    // A sessão do TLS não é retomada: cada conexão do OpenVPN negocia do zero.
    config.resumption = Resumption::disabled();
    Ok(Arc::new(config))
}

/// A proteção do canal de controle que o `.ovpn` pede.
pub(crate) fn build_wrapper(
    pki: &Pki,
    role: Role,
    digest: AuthDigest,
    clock: &Clock,
    replay_window: u32,
) -> Result<Wrapper, ControlError> {
    let start = clock.unix_secs();
    let missing = |what: &str| ControlError::Setup(format!("falta a chave estática do {what}"));
    let wrap_err = |e: crate::wrap::WrapError| ControlError::Setup(e.to_string());
    match &pki.wrap {
        Wrap::None => Ok(Wrapper::none()),
        Wrap::TlsAuth => {
            let key = pki.static_key.as_ref().ok_or_else(|| missing("tls-auth"))?;
            Wrapper::tls_auth(
                key,
                Direction::from_key_direction(pki.key_direction),
                digest,
                role,
                start,
                replay_window,
            )
            .map_err(wrap_err)
        }
        Wrap::TlsCrypt => {
            let key = pki
                .static_key
                .as_ref()
                .ok_or_else(|| missing("tls-crypt"))?;
            Ok(Wrapper::tls_crypt(key, role, start, replay_window))
        }
        Wrap::TlsCryptV2 { kc, wkc } => {
            let key = StaticKey::from_bytes(**kc);
            Wrapper::tls_crypt_v2(&key, wkc.clone(), start, replay_window).map_err(wrap_err)
        }
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn o_key_id_volta_do_7_para_o_1_e_nunca_para_o_0() {
        assert_eq!(next_key_id(0), 1);
        assert_eq!(next_key_id(6), 7);
        assert_eq!(next_key_id(7), 1);
    }
}
