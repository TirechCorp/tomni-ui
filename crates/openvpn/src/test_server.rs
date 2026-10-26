//! O lado servidor do canal de controle, só para os testes (feature `test-util`).
//!
//! É um servidor mínimo, em memória e sem E/S, que fala o mesmo protocolo que o [`crate::control`]
//! e reaproveita o mesmo canal ([`crate::channel`]). Serve para os testes do cliente sem rede e
//! sem o programa `openvpn`: responde ao reset, faz o TLS (`rustls`, pedindo o certificado do
//! cliente), lê a troca de chaves, decide a autenticação por uma função do teste, empurra a
//! configuração e deriva as mesmas chaves de dados do cliente.
//!
//! O app nunca o usa. Não implementa o `tls-crypt-v2` do lado servidor.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rustls::pki_types::CertificateDer;
use rustls::server::WebPkiClientVerifier;
use rustls::sign::CertifiedKey;
use rustls::{RootCertStore, ServerConfig};
use zeroize::Zeroizing;

use crate::channel::{Clock, Core, CoreError, Datagram, TlsSide};
use crate::config::{AuthDigest, Proto, TlsVersion};
use crate::control::{
    DEFAULT_HAND_WINDOW, DEFAULT_REPLAY_WINDOW, DEFAULT_TLS_MTU, DEFAULT_TRAN_WINDOW,
    build_wrapper, tls_versions,
};
use crate::data::{DataChannel, DataCipher, DataOptions, Opened};
use crate::keymethod::{
    ClientKeyExchange, KeyMethodError, ServerKeyExchange, parse_peer_info, proto_flags,
};
use crate::pki::{Pki, PrivateKey};
use crate::prf::{DataKeys, EKM_LABEL, KEY_BLOCK_LEN, PrfInputs, derive_key_block};
use crate::push::PushReader;
use crate::reliable::ReliableConfig;
use crate::wrap::Role;

/// O que o cliente declarou na troca de chaves.
#[derive(Debug, Clone)]
pub struct Attempt {
    /// O `key_id` da geração.
    pub key_id: u8,
    /// Se é a primeira geração da sessão.
    pub initial: bool,
    /// O usuário.
    pub username: Option<String>,
    /// A senha (ou `SCRV1:...`, `CRV1::...`, o `auth-token`).
    pub password: Option<String>,
    /// O texto de opções.
    pub options: String,
    /// O `peer info`, em pares chave/valor.
    pub peer_info: Vec<(String, String)>,
}

impl Attempt {
    /// O valor de uma chave do `peer info`.
    pub fn peer(&self, key: &str) -> Option<&str> {
        self.peer_info
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

/// A decisão do servidor sobre uma tentativa.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Aceita. Com `token`, empurra um `auth-token` junto da configuração.
    Accept {
        /// O `auth-token` a empurrar.
        token: Option<String>,
    },
    /// Recusa com esta mensagem de controle (`AUTH_FAILED`, `AUTH_FAILED,TEMP:...`...).
    Fail(String),
    /// Segura: manda `AUTH_PENDING` e espera [`ServerControl::resolve_pending`].
    Pending {
        /// O `timeout` anunciado, em segundos.
        timeout: u64,
    },
}

/// O que o servidor de teste conta ao teste.
#[derive(Debug)]
pub enum ServerEvent {
    /// A tentativa foi aceita.
    Authenticated {
        /// O `key_id`.
        key_id: u8,
        /// O usuário.
        username: Option<String>,
    },
    /// As chaves de dados da geração.
    DataKeys {
        /// O `key_id`.
        key_id: u8,
        /// As chaves, do ponto de vista do servidor.
        keys: Box<DataKeys>,
    },
    /// A tentativa foi recusada com este texto.
    Rejected {
        /// O `key_id`.
        key_id: u8,
        /// A mensagem enviada.
        text: String,
    },
    /// Uma mensagem de texto do cliente (menos o `PUSH_REQUEST`, que o servidor responde).
    Message(String),
    /// A negociação falhou do lado do servidor (por exemplo, certificado do cliente recusado).
    Failed(String),
    /// O cliente avisou que vai sair (`OCC_EXIT` no canal de dados). Só vem de uma
    /// [`ServerSession`].
    Exit,
}

/// As opções do servidor de teste.
pub struct ServerOptions {
    /// UDP ou TCP.
    pub transport: Proto,
    /// O certificado do servidor (e as intermediárias).
    pub chain: Vec<CertificateDer<'static>>,
    /// A chave do servidor.
    pub key: PrivateKey,
    /// As CAs que emitiram os clientes aceitos.
    pub client_ca: Vec<CertificateDer<'static>>,
    /// Se o certificado do cliente é obrigatório.
    pub require_client_cert: bool,
    /// A proteção do canal de controle (usa `wrap`, `static_key` e `key_direction`). O
    /// `key_direction` é o mesmo do cliente: o servidor de teste inverte as metades pelo papel.
    pub wrap: Pki,
    /// O digest do `tls-auth`.
    pub auth_digest: AuthDigest,
    /// O `tls-version-min`.
    pub tls_min: Option<TlsVersion>,
    /// O `tls-version-max`.
    pub tls_max: Option<TlsVersion>,
    /// O `tls-mtu`.
    pub tls_mtu: usize,
    /// O `hand-window`.
    pub hand_window: Duration,
    /// O `replay-window`.
    pub replay_window: u32,
    /// Se as chaves de dados saem do exportador do TLS.
    pub ekm: bool,
    /// As mensagens `PUSH_REPLY` a enviar (várias, com `push-continuation`, se o teste quiser).
    pub push: Vec<String>,
    /// Se empurra a configuração sozinho quando o cliente anuncia `REQUEST_PUSH`. Desligado, só
    /// responde ao `PUSH_REQUEST`.
    pub auto_push: bool,
    /// O texto de opções do servidor.
    pub options_text: String,
    /// O `peer info` que o servidor manda, se mandar.
    pub peer_info: Option<String>,
    /// O relógio.
    pub clock: Clock,
}

impl ServerOptions {
    /// Opções comuns: UDP, sem proteção extra, chaves pela PRF, e uma configuração empurrada
    /// simples.
    pub fn new(
        chain: Vec<CertificateDer<'static>>,
        key: PrivateKey,
        client_ca: Vec<CertificateDer<'static>>,
        clock: Clock,
    ) -> ServerOptions {
        ServerOptions {
            transport: Proto::Udp,
            chain,
            key,
            client_ca,
            require_client_cert: true,
            wrap: Pki::default(),
            auth_digest: AuthDigest::Sha1,
            tls_min: None,
            tls_max: None,
            tls_mtu: DEFAULT_TLS_MTU,
            hand_window: DEFAULT_HAND_WINDOW,
            replay_window: DEFAULT_REPLAY_WINDOW,
            ekm: false,
            push: vec![Self::default_push(false)],
            auto_push: true,
            options_text: "V4,dev-type tun,link-mtu 1549,tun-mtu 1500,proto UDPv4,cipher AES-256-GCM,auth SHA1,keysize 256,key-method 2,tls-server".to_owned(),
            peer_info: None,
            clock,
        }
    }

    /// Um `PUSH_REPLY` simples.
    pub fn default_push(ekm: bool) -> String {
        let mut s = "PUSH_REPLY,route 10.9.0.0 255.255.255.0,route-gateway 10.8.0.1,topology subnet,ifconfig 10.8.0.2 255.255.255.0,peer-id 0,cipher AES-256-GCM,ping 10,ping-restart 60".to_owned();
        if ekm {
            s.push_str(",key-derivation tls-ekm");
        }
        s
    }
}

/// O estado de uma tentativa que o servidor segurou (`AUTH_PENDING`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// Esperando a troca de chaves do cliente.
    Waiting,
    /// Segurou a decisão.
    Held,
    /// Aceitou.
    Accepted,
    /// Recusou.
    Rejected,
}

/// O servidor de teste.
pub struct ServerControl {
    core: Core,
    authenticate: Box<dyn FnMut(&Attempt) -> Decision>,
    ekm: bool,
    push: Vec<String>,
    auto_push: bool,
    options_text: String,
    peer_info: Option<String>,
    events: VecDeque<ServerEvent>,
    stages: Vec<(u8, Stage)>,
    token: Option<String>,
    wants_push: bool,
    /// O último instante visto (as gerações antigas se aposentam a partir dele).
    last_now: Instant,
}

impl ServerControl {
    /// Cria o servidor. `authenticate` decide cada tentativa.
    pub fn new(
        options: ServerOptions,
        authenticate: impl FnMut(&Attempt) -> Decision + 'static,
    ) -> Result<ServerControl, String> {
        let config = server_config(&options)?;
        let wrapper = build_wrapper(
            &options.wrap,
            Role::Server,
            options.auth_digest,
            &options.clock,
            options.replay_window,
        )
        .map_err(|e| e.to_string())?;
        let reliable = match options.transport {
            Proto::Udp => ReliableConfig::udp(),
            Proto::Tcp => ReliableConfig::tcp(),
        };
        let core = Core::new(
            Role::Server,
            wrapper,
            reliable,
            TlsSide::Server(config),
            options.tls_mtu,
            options.hand_window,
        );
        Ok(ServerControl {
            core,
            authenticate: Box::new(authenticate),
            ekm: options.ekm,
            push: options.push,
            auto_push: options.auto_push,
            options_text: options.options_text,
            peer_info: options.peer_info,
            events: VecDeque::new(),
            stages: Vec::new(),
            token: None,
            wants_push: false,
            last_now: Instant::now(),
        })
    }

    /// Entrega um datagrama do cliente.
    pub fn handle_datagram(&mut self, raw: &[u8], now: Instant) -> Datagram {
        self.last_now = now;
        match self.core.handle_datagram(raw, now) {
            Ok(d) => {
                self.advance();
                d
            }
            Err(CoreError::Tls(e)) => {
                self.events.push_back(ServerEvent::Failed(e.to_string()));
                Datagram::Control
            }
        }
    }

    /// O próximo datagrama a enviar.
    pub fn poll_transmit(&mut self, now: Instant) -> Option<Vec<u8>> {
        self.last_now = now;
        // As gerações antigas saem depois do `tran-window`, como no cliente.
        self.core.slots.retain(|s| !s.is_retired(now));
        let vivas: Vec<u8> = self.core.slots.iter().map(|s| s.key_id).collect();
        self.stages.retain(|(k, _)| vivas.contains(k));
        self.core.poll_transmit(now)
    }

    /// O próximo evento.
    pub fn poll_event(&mut self) -> Option<ServerEvent> {
        self.events.pop_front()
    }

    /// Quando chamar [`ServerControl::poll_transmit`] de novo por causa do relógio (os reenvios;
    /// o servidor de teste não impõe prazos à negociação).
    pub fn next_timeout(&self) -> Option<Instant> {
        self.core.next_retransmit()
    }

    /// Manda uma mensagem de texto ao cliente.
    pub fn send_message(&mut self, text: &str) {
        let Some(i) = self.core.slots.iter().rposition(|s| s.server_kx.is_some()) else {
            return;
        };
        let mut bytes = text.as_bytes().to_vec();
        bytes.push(0);
        let _ = self.core.write_plain(i, &bytes);
    }

    /// Resolve uma tentativa segurada por [`Decision::Pending`].
    pub fn resolve_pending(&mut self, decision: Decision) {
        let Some(&(key_id, _)) = self.stages.iter().find(|(_, s)| *s == Stage::Held) else {
            return;
        };
        if let Some(i) = self.core.slot_index(key_id) {
            self.apply(i, decision);
        }
    }

    /// Quantos datagramas foram descartados.
    pub fn dropped_datagrams(&self) -> u64 {
        self.core.dropped
    }

    fn stage(&self, key_id: u8) -> Stage {
        self.stages
            .iter()
            .find(|(k, _)| *k == key_id)
            .map_or(Stage::Waiting, |(_, s)| *s)
    }

    fn set_stage(&mut self, key_id: u8, stage: Stage) {
        match self.stages.iter_mut().find(|(k, _)| *k == key_id) {
            Some(entry) => entry.1 = stage,
            None => self.stages.push((key_id, stage)),
        }
    }

    fn advance(&mut self) {
        for i in 0..self.core.slots.len() {
            self.advance_slot(i);
        }
    }

    fn advance_slot(&mut self, i: usize) {
        let key_id = self.core.slots[i].key_id;
        if self.core.slots[i].tls.is_handshaking() {
            return;
        }
        // A troca de chaves do cliente.
        if self.core.slots[i].client_kx.is_none() {
            let slot = &mut self.core.slots[i];
            match ClientKeyExchange::parse(&slot.plain_in) {
                Ok((kx, used)) => {
                    slot.plain_in.drain(..used);
                    slot.client_kx = Some(kx);
                }
                Err(KeyMethodError::Incomplete) => return,
                Err(e) => {
                    self.events.push_back(ServerEvent::Failed(e.to_string()));
                    return;
                }
            }
            let kx = self.core.slots[i]
                .client_kx
                .as_ref()
                .expect("acabou de ser lida");
            let peer_info = parse_peer_info(&kx.peer_info);
            let attempt = Attempt {
                key_id,
                initial: self.core.slots[i].initial,
                username: kx.username.as_ref().map(|s| s.to_string()),
                password: kx.password.as_ref().map(|s| s.to_string()),
                options: kx.options.clone(),
                peer_info,
            };
            self.wants_push = self.auto_push
                && attempt
                    .peer("IV_PROTO")
                    .and_then(|v| v.parse::<u32>().ok())
                    .is_some_and(|p| p & proto_flags::REQUEST_PUSH != 0);
            let skx = ServerKeyExchange::random(self.options_text.clone(), self.peer_info.clone());
            let bytes = skx.encode().expect("as opções do servidor de teste cabem");
            self.core.slots[i].server_kx = Some(skx);
            self.core.slots[i].kx_sent = true;
            let _ = self.core.write_plain(i, &bytes);
            let decision = (self.authenticate)(&attempt);
            self.apply(i, decision);
        }
        // As mensagens de texto depois da troca de chaves.
        if self.core.slots[i].server_kx.is_some() {
            loop {
                let slot = &mut self.core.slots[i];
                let Some(pos) = slot.plain_in.iter().position(|b| *b == 0) else {
                    break;
                };
                let raw: Vec<u8> = slot.plain_in.drain(..=pos).collect();
                let text = String::from_utf8_lossy(&raw[..pos]).into_owned();
                if text == "PUSH_REQUEST" {
                    if self.stage(key_id) == Stage::Accepted {
                        self.send_push(i);
                    }
                } else {
                    self.events.push_back(ServerEvent::Message(text));
                }
            }
        }
    }

    fn apply(&mut self, i: usize, decision: Decision) {
        let key_id = self.core.slots[i].key_id;
        match decision {
            Decision::Accept { token } => {
                self.set_stage(key_id, Stage::Accepted);
                self.core.slots[i].keys_done = true;
                self.core.slots[i].deadline = None;
                let aposentar = self.last_now + DEFAULT_TRAN_WINDOW;
                for outro in &mut self.core.slots {
                    if outro.key_id != key_id && outro.retire_at.is_none() {
                        outro.retire_at = Some(aposentar);
                    }
                }
                let username = self.core.slots[i]
                    .client_kx
                    .as_ref()
                    .and_then(|kx| kx.username.as_ref().map(|u| u.to_string()));
                self.events
                    .push_back(ServerEvent::Authenticated { key_id, username });
                match self.derive(i) {
                    Ok(keys) => self.events.push_back(ServerEvent::DataKeys {
                        key_id,
                        keys: Box::new(keys),
                    }),
                    Err(e) => self.events.push_back(ServerEvent::Failed(e)),
                }
                if token.is_some() {
                    self.token = token;
                }
                if self.core.slots[i].initial && self.wants_push {
                    self.send_push(i);
                }
            }
            Decision::Fail(text) => {
                self.set_stage(key_id, Stage::Rejected);
                self.send_text(i, &text);
                self.events
                    .push_back(ServerEvent::Rejected { key_id, text });
            }
            Decision::Pending { timeout } => {
                self.set_stage(key_id, Stage::Held);
                self.send_text(i, &format!("AUTH_PENDING,timeout {timeout}"));
            }
        }
    }

    fn send_text(&mut self, i: usize, text: &str) {
        let mut bytes = text.as_bytes().to_vec();
        bytes.push(0);
        let _ = self.core.write_plain(i, &bytes);
    }

    fn send_push(&mut self, i: usize) {
        for (n, message) in self.push.clone().iter().enumerate() {
            let mut text = message.clone();
            if n == 0
                && let Some(token) = &self.token
            {
                text = text.replacen("PUSH_REPLY", &format!("PUSH_REPLY,auth-token {token}"), 1);
            }
            self.send_text(i, &text);
        }
    }

    fn derive(&self, i: usize) -> Result<DataKeys, String> {
        let slot = &self.core.slots[i];
        let block: Zeroizing<[u8; KEY_BLOCK_LEN]> = if self.ekm {
            let mut out = Zeroizing::new([0u8; KEY_BLOCK_LEN]);
            slot.tls
                .export_keying_material(&mut out[..], EKM_LABEL.as_bytes(), None)
                .map_err(|e| e.to_string())?;
            out
        } else {
            let (Some(client), Some(server), Some(remote)) =
                (&slot.client_kx, &slot.server_kx, self.core.remote_sid)
            else {
                return Err("faltou a troca de chaves".into());
            };
            derive_key_block(&PrfInputs {
                pre_master: &client.pre_master,
                client_random1: &client.random1,
                server_random1: &server.random1,
                client_random2: &client.random2,
                server_random2: &server.random2,
                client_session: remote,
                server_session: self.core.local_sid,
            })
        };
        Ok(DataKeys::from_block(&block, Role::Server))
    }
}

fn server_config(options: &ServerOptions) -> Result<Arc<ServerConfig>, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut roots = RootCertStore::empty();
    let (_, bad) = roots.add_parsable_certificates(options.client_ca.iter().cloned());
    if bad > 0 && roots.is_empty() {
        return Err("a CA dos clientes não pôde ser lida".into());
    }
    let mut verifier =
        WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider.clone());
    if !options.require_client_cert {
        verifier = verifier.allow_unauthenticated();
    }
    let verifier = verifier.build().map_err(|e| e.to_string())?;
    let signer = options.key.signing_key().map_err(|e| e.to_string())?;
    let certified = CertifiedKey::new(options.chain.clone(), signer);
    let versions = tls_versions(options.tls_min, options.tls_max).map_err(|e| e.to_string())?;
    let mut config = ServerConfig::builder_with_details(provider, Arc::new(options.clock.clone()))
        .with_protocol_versions(&versions)
        .map_err(|e| e.to_string())?
        .with_client_cert_verifier(verifier)
        .with_cert_resolver(Arc::new(rustls::sign::SingleCertAndKey::from(certified)));
    config.send_tls13_tickets = 0;
    Ok(Arc::new(config))
}

/// O servidor de teste inteiro: o canal de controle ([`ServerControl`]) e o de dados, com o ping.
///
/// Recebe os pacotes IP do cliente ([`ServerSession::poll_ip`]) e envia os seus
/// ([`ServerSession::send_ip`]). Quem monta a rede de teste põe uma pilha TCP por cima.
pub struct ServerSession {
    control: ServerControl,
    data: DataChannel,
    ping: Option<Duration>,
    ping_at: Option<Instant>,
    events: VecDeque<ServerEvent>,
    out: VecDeque<Vec<u8>>,
    ip_in: VecDeque<Vec<u8>>,
    pings: u64,
    exits: u64,
}

impl ServerSession {
    /// Cria o servidor. Os ajustes do canal de dados (cifra, `peer-id`, compressão, `ping`) saem
    /// do `PUSH_REPLY` que ele mesmo vai empurrar.
    pub fn new(
        options: ServerOptions,
        authenticate: impl FnMut(&Attempt) -> Decision + 'static,
    ) -> Result<ServerSession, String> {
        let mut reader = PushReader::new(&[], false);
        let mut pushed = None;
        for message in &options.push {
            if let Some(p) = reader.feed(message).map_err(|e| e.to_string())? {
                pushed = Some(p);
            }
        }
        let pushed = pushed.ok_or("o PUSH_REPLY do servidor de teste não terminou")?;
        let cipher = pushed
            .cipher
            .as_deref()
            .and_then(DataCipher::from_name)
            .unwrap_or(DataCipher::Aes256Gcm);
        let data = DataChannel::new(DataOptions {
            cipher,
            peer_id: pushed.peer_id,
            compress: pushed.compress,
            replay_window: options.replay_window,
            tran_window: DEFAULT_TRAN_WINDOW,
            reneg_bytes: None,
            reneg_pkts: None,
        });
        let ping = pushed
            .ping
            .filter(|s| *s > 0)
            .map(|s| Duration::from_secs(u64::from(s)));
        let control = ServerControl::new(options, authenticate)?;
        Ok(ServerSession {
            control,
            data,
            ping,
            ping_at: None,
            events: VecDeque::new(),
            out: VecDeque::new(),
            ip_in: VecDeque::new(),
            pings: 0,
            exits: 0,
        })
    }

    /// Entrega um datagrama do cliente.
    pub fn handle_datagram(&mut self, raw: &[u8], now: Instant) {
        if let Datagram::Data(packet) = self.control.handle_datagram(raw, now) {
            match self.data.open(&packet, now) {
                Ok(Opened::Ip(ip)) => self.ip_in.push_back(ip),
                Ok(Opened::Ping) => self.pings += 1,
                Ok(Opened::Occ(occ)) => {
                    if occ.get(16) == Some(&crate::data::OCC_EXIT) {
                        self.exits += 1;
                        self.events.push_back(ServerEvent::Exit);
                    }
                }
                Err(_) => {}
            }
        }
        self.pump(now);
    }

    /// Cifra um pacote IP para o cliente (descartado sem chave).
    pub fn send_ip(&mut self, packet: &[u8]) {
        if let Ok(d) = self.data.seal(packet) {
            self.out.push_back(d);
        }
    }

    /// Cifra `framed` sem o enquadramento da compressão (para forjar o que o app nunca recebe de
    /// um servidor bem configurado).
    pub fn send_raw(&mut self, framed: &[u8]) {
        if let Ok(d) = self.data.seal_raw(framed) {
            self.out.push_back(d);
        }
    }

    /// O próximo datagrama a enviar.
    pub fn poll_transmit(&mut self, now: Instant) -> Option<Vec<u8>> {
        let datagram = self
            .control
            .poll_transmit(now)
            .or_else(|| self.out.pop_front());
        self.pump(now);
        if datagram.is_some()
            && let Some(p) = self.ping
        {
            self.ping_at = Some(now + p);
        }
        datagram
    }

    /// O próximo pacote IP vindo do cliente.
    pub fn poll_ip(&mut self) -> Option<Vec<u8>> {
        self.ip_in.pop_front()
    }

    /// O próximo evento.
    pub fn poll_event(&mut self) -> Option<ServerEvent> {
        self.events.pop_front()
    }

    /// Quando chamar [`ServerSession::handle_timeout`] de novo.
    pub fn next_timeout(&self) -> Option<Instant> {
        [
            self.control.next_timeout(),
            self.ping_at,
            self.data.next_expiry(),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// Avisa que o tempo passou: manda o ping, se a linha ficou parada.
    pub fn handle_timeout(&mut self, now: Instant) {
        self.data.expire(now);
        if self.ping_at.is_some_and(|t| now >= t) {
            if let Ok(p) = self.data.seal_ping() {
                self.out.push_back(p);
            }
            self.ping_at = self.ping.map(|p| now + p);
        }
    }

    /// Manda uma mensagem de texto pelo canal de controle (`RESTART`, `HALT`...).
    pub fn send_message(&mut self, text: &str) {
        self.control.send_message(text);
    }

    /// Resolve uma tentativa segurada por [`Decision::Pending`].
    pub fn resolve_pending(&mut self, decision: Decision) {
        self.control.resolve_pending(decision);
    }

    /// Quantos pings chegaram do cliente.
    pub fn pings_received(&self) -> u64 {
        self.pings
    }

    /// Quantos avisos de saída (`OCC_EXIT`) chegaram.
    pub fn exits_received(&self) -> u64 {
        self.exits
    }

    /// Quantos pacotes de dados o servidor recusou.
    pub fn rejected_packets(&self) -> u64 {
        self.data.rejected()
    }

    fn pump(&mut self, now: Instant) {
        while let Some(event) = self.control.poll_event() {
            match event {
                ServerEvent::DataKeys { key_id, keys } => {
                    if self.data.install(key_id, &keys, now).is_ok() && self.ping_at.is_none() {
                        self.ping_at = self.ping.map(|p| now + p);
                    }
                    self.events
                        .push_back(ServerEvent::DataKeys { key_id, keys });
                }
                outro => self.events.push_back(outro),
            }
        }
    }
}
