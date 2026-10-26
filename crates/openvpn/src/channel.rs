//! O canal de controle comum ao cliente e ao servidor de teste.
//!
//! Reúne o que os dois lados fazem igual: abrir e fechar o pacote com a proteção
//! (`tls-auth`/`tls-crypt`), a camada de confiabilidade de cada geração de chaves e a ligação
//! com o TLS do `rustls`, que roda "por cima" das mensagens `P_CONTROL_V1`. Quem decide o que
//! dizer por dentro do TLS (a troca de chaves, as senhas, o `PUSH`) é o [`crate::control`] (cliente)
//! ou o `test_server`.
//!
//! Tudo é sem E/S: o chamador entrega os datagramas e o relógio, e retira o que enviar.

use std::fmt;
use std::io::{Cursor, ErrorKind, Read};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rustls::pki_types::{ServerName, UnixTime};
use rustls::time_provider::TimeProvider;
use rustls::{ClientConfig, ClientConnection, Connection, ServerConfig, ServerConnection};

use crate::keymethod::{ClientKeyExchange, ServerKeyExchange};
use crate::packet::{ControlPacket, Opcode, SessionId, classify};
use crate::reliable::{Reliable, ReliableConfig};
use crate::wrap::{Role, Wrapper};

/// O relógio do motor: a hora de parede (para os certificados e para o tempo do `tls-auth`).
///
/// Os testes usam [`Clock::fixed`] para que os certificados das fixtures valham sempre.
#[derive(Clone)]
pub struct Clock(Arc<dyn Fn() -> SystemTime + Send + Sync>);

impl Clock {
    /// A hora do sistema.
    pub fn system() -> Clock {
        Clock(Arc::new(SystemTime::now))
    }

    /// Uma hora que não anda.
    pub fn fixed(time: SystemTime) -> Clock {
        Clock(Arc::new(move || time))
    }

    /// Um relógio feito por uma função (os testes que querem andar no tempo).
    pub fn new(f: impl Fn() -> SystemTime + Send + Sync + 'static) -> Clock {
        Clock(Arc::new(f))
    }

    /// A hora atual.
    pub fn now(&self) -> SystemTime {
        (self.0)()
    }

    /// Os segundos desde 1970, no tamanho de 32 bits que o protocolo usa.
    pub fn unix_secs(&self) -> u32 {
        let secs = self
            .now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        u32::try_from(secs).unwrap_or(u32::MAX)
    }
}

impl fmt::Debug for Clock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Clock")
    }
}

impl TimeProvider for Clock {
    fn current_time(&self) -> Option<UnixTime> {
        self.now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(UnixTime::since_unix_epoch)
    }
}

/// O maior cabeçalho de um pacote de controle: opcode (1), sessão (8), contagem de ACKs (1), até
/// 4 ACKs (16), a sessão do outro lado (8) e o id da mensagem (4). Vem de
/// [`crate::packet::MAX_ACKS_IN_CONTROL`] e do formato do pacote.
pub(crate) const MAX_CONTROL_HEADER: usize = 1
    + crate::packet::SESSION_ID_LEN
    + 1
    + 4 * crate::packet::MAX_ACKS_IN_CONTROL
    + crate::packet::SESSION_ID_LEN
    + 4;

/// Tamanho do buffer de leitura do texto claro do TLS.
const READ_CHUNK: usize = 4096;

/// De que lado do TLS estamos, com a configuração.
pub(crate) enum TlsSide {
    /// O cliente: a configuração e o nome (que não é enviado, mas o `rustls` exige).
    Client(Arc<ClientConfig>, ServerName<'static>),
    /// O servidor (só o de teste).
    #[cfg_attr(not(feature = "test-util"), allow(dead_code))] // só o servidor de teste usa
    Server(Arc<ServerConfig>),
}

/// Uma geração de chaves (um `key_id`) do canal de controle.
pub(crate) struct Slot {
    /// O `key_id` (0 a 7).
    pub key_id: u8,
    /// A confiabilidade desta geração.
    pub reliable: Reliable,
    /// O TLS desta geração.
    pub tls: Connection,
    /// Se o TLS já pode falar. O cliente espera o reset do servidor (um servidor 2.6 não guarda
    /// estado enquanto não vê o reset de volta).
    pub tls_enabled: bool,
    /// O texto claro que veio do TLS e ainda não foi usado.
    pub plain_in: Vec<u8>,
    /// Se já chegou o reset do outro lado.
    pub remote_reset: bool,
    /// Se é a primeira geração da sessão.
    pub initial: bool,
    /// Quando a janela de negociação (`hand-window`) acaba.
    pub deadline: Option<Instant>,
    /// Quando esta geração, já substituída, deixa de ser aceita (`tran-window`).
    pub retire_at: Option<Instant>,
    /// A troca de chaves do cliente (a nossa, no cliente; a do outro lado, no servidor).
    pub client_kx: Option<ClientKeyExchange>,
    /// A troca de chaves do servidor.
    pub server_kx: Option<ServerKeyExchange>,
    /// Se a nossa troca de chaves já foi enviada.
    pub kx_sent: bool,
    /// Se as chaves do canal de dados desta geração já foram entregues.
    pub keys_done: bool,
    /// Se o outro lado fechou o TLS (`close_notify`).
    pub peer_closed: bool,
}

impl Slot {
    /// Se esta geração ainda aceita pacotes.
    pub(crate) fn is_retired(&self, now: Instant) -> bool {
        self.retire_at.is_some_and(|t| now >= t)
    }
}

/// O que `handle_datagram` fez com o datagrama.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Datagram {
    /// Era do canal de controle e foi processado.
    Control,
    /// Era do canal de dados: o motor de dados (fatia seguinte) trata. Vai como chegou.
    Data(Vec<u8>),
    /// Foi descartado (inválido, repetido, de outra sessão ou de um `key_id` desconhecido).
    Dropped,
}

/// O que pode dar errado dentro do canal.
#[derive(Debug)]
pub(crate) enum CoreError {
    /// O TLS recusou (erro do `rustls`).
    Tls(rustls::Error),
}

/// O canal de controle de uma sessão.
pub(crate) struct Core {
    pub role: Role,
    pub wrapper: Wrapper,
    pub reliable_config: ReliableConfig,
    pub local_sid: SessionId,
    pub remote_sid: Option<SessionId>,
    pub slots: Vec<Slot>,
    pub side: TlsSide,
    /// Quanto cabe de TLS em cada `P_CONTROL_V1`.
    pub max_payload: usize,
    pub hand_window: Duration,
    /// Quantos pacotes foram descartados (inválidos, repetidos...).
    pub dropped: u64,
    /// Se já chegou algum pacote válido do outro lado.
    pub heard: bool,
}

impl Core {
    pub(crate) fn new(
        role: Role,
        wrapper: Wrapper,
        reliable_config: ReliableConfig,
        side: TlsSide,
        tls_mtu: usize,
        hand_window: Duration,
    ) -> Core {
        // Pelo menos 1 byte por pacote, mesmo com um `tls-mtu` absurdo, para o envio andar.
        let max_payload = tls_mtu
            .saturating_sub(MAX_CONTROL_HEADER + wrapper.overhead())
            .max(1);
        Core {
            role,
            wrapper,
            reliable_config,
            local_sid: SessionId::random(),
            remote_sid: None,
            slots: Vec::new(),
            side,
            max_payload,
            hand_window,
            dropped: 0,
            heard: false,
        }
    }

    /// Abre uma geração de chaves e enfileira o nosso reset (`opcode`).
    pub(crate) fn open_slot(
        &mut self,
        key_id: u8,
        reset: Opcode,
        initial: bool,
        now: Instant,
    ) -> Result<usize, rustls::Error> {
        let tls = match &self.side {
            TlsSide::Client(config, name) => {
                Connection::Client(ClientConnection::new(config.clone(), name.clone())?)
            }
            TlsSide::Server(config) => Connection::Server(ServerConnection::new(config.clone())?),
        };
        let mut reliable = Reliable::new(self.reliable_config);
        reliable.send(reset, Vec::new());
        self.slots.push(Slot {
            key_id,
            reliable,
            tls,
            tls_enabled: self.role == Role::Server,
            plain_in: Vec::new(),
            remote_reset: false,
            initial,
            deadline: Some(now + self.hand_window),
            retire_at: None,
            client_kx: None,
            server_kx: None,
            kx_sent: false,
            keys_done: false,
            peer_closed: false,
        });
        Ok(self.slots.len() - 1)
    }

    pub(crate) fn slot_index(&self, key_id: u8) -> Option<usize> {
        self.slots.iter().position(|s| s.key_id == key_id)
    }

    /// Processa um datagrama que chegou.
    pub(crate) fn handle_datagram(
        &mut self,
        raw: &[u8],
        now: Instant,
    ) -> Result<Datagram, CoreError> {
        let Ok((opcode, key_id)) = classify(raw) else {
            self.dropped += 1;
            return Ok(Datagram::Dropped);
        };
        if opcode.is_data() {
            return Ok(Datagram::Data(raw.to_vec()));
        }
        let packet = match self.wrapper.open(raw) {
            Ok(p) => p,
            Err(_) => {
                self.dropped += 1;
                return Ok(Datagram::Dropped);
            }
        };
        // Se o pacote confirma algo, a sessão que ele cita como "nossa" tem de ser a nossa.
        if !packet.acks.is_empty() && packet.remote_session_id != Some(self.local_sid) {
            self.dropped += 1;
            return Ok(Datagram::Dropped);
        }
        match self.remote_sid {
            None if packet.opcode.is_hard_reset() => self.remote_sid = Some(packet.session_id),
            None => {
                self.dropped += 1;
                return Ok(Datagram::Dropped);
            }
            Some(sid) if sid != packet.session_id => {
                self.dropped += 1;
                return Ok(Datagram::Dropped);
            }
            Some(_) => {}
        }
        self.heard = true;

        let index = match self.slot_index(key_id) {
            Some(i) => i,
            None => {
                // Só um reset abre uma geração nova. O servidor também abre pelo reset do
                // cliente; o cliente só pelo `SOFT_RESET` do servidor.
                let opens = match (self.role, packet.opcode) {
                    (Role::Server, op) if op.is_hard_reset() => {
                        Some((Opcode::HardResetServerV2, true))
                    }
                    (_, Opcode::ControlSoftResetV1) => Some((Opcode::ControlSoftResetV1, false)),
                    _ => None,
                };
                let Some((reset, initial)) = opens else {
                    self.dropped += 1;
                    return Ok(Datagram::Dropped);
                };
                self.open_slot(key_id, reset, initial && self.slots.is_empty(), now)
                    .map_err(CoreError::Tls)?
            }
        };
        self.receive_into(index, &packet)?;
        Ok(Datagram::Control)
    }

    fn receive_into(&mut self, index: usize, packet: &ControlPacket) -> Result<(), CoreError> {
        let role = self.role;
        self.slots[index].reliable.receive(packet);
        while let Some(msg) = self.slots[index].reliable.pop_received() {
            let slot = &mut self.slots[index];
            if msg.opcode.is_hard_reset() || msg.opcode == Opcode::ControlSoftResetV1 {
                slot.remote_reset = true;
                if role == Role::Client {
                    slot.tls_enabled = true;
                }
            } else if msg.opcode == Opcode::ControlV1 {
                if !slot.tls_enabled {
                    self.dropped += 1;
                    continue;
                }
                if let Err(e) = feed_tls(slot, &msg.payload) {
                    // Deixa o alerta do TLS sair antes de falhar.
                    pump_out(slot, self.max_payload);
                    return Err(CoreError::Tls(e));
                }
            }
        }
        Ok(())
    }

    /// Escreve texto claro no TLS da geração.
    pub(crate) fn write_plain(&mut self, index: usize, bytes: &[u8]) -> Result<(), rustls::Error> {
        use std::io::Write;
        let max = self.max_payload;
        let slot = &mut self.slots[index];
        slot.tls
            .writer()
            .write_all(bytes)
            .map_err(|e| rustls::Error::General(e.to_string()))?;
        pump_out(slot, max);
        Ok(())
    }

    /// O próximo datagrama a enviar (sem o prefixo de tamanho do TCP).
    pub(crate) fn poll_transmit(&mut self, now: Instant) -> Option<Vec<u8>> {
        loop {
            let mut found = None;
            for i in 0..self.slots.len() {
                if self.slots[i].tls_enabled {
                    pump_out(&mut self.slots[i], self.max_payload);
                }
                if let Some(tx) = self.slots[i].reliable.poll_transmit(now) {
                    found = Some((self.slots[i].key_id, tx));
                    break;
                }
            }
            let (key_id, tx) = found?;
            let packet = tx.into_packet(key_id, self.local_sid, self.remote_sid);
            match self.wrapper.seal(&packet) {
                Ok(bytes) => return Some(bytes),
                Err(_) => self.dropped += 1,
            }
        }
    }

    /// O próximo reenvio por falta de confirmação (sem os prazos de negociação e de aposentadoria,
    /// que só o cliente cumpre).
    #[cfg_attr(not(feature = "test-util"), allow(dead_code))] // só o servidor de teste usa
    pub(crate) fn next_retransmit(&self) -> Option<Instant> {
        self.slots
            .iter()
            .filter_map(|s| s.reliable.next_timeout())
            .min()
    }

    /// O próximo instante em que o canal precisa de atenção.
    pub(crate) fn next_timeout(&self) -> Option<Instant> {
        self.slots
            .iter()
            .flat_map(|s| [s.reliable.next_timeout(), s.deadline, s.retire_at])
            .flatten()
            .min()
    }
}

/// Entrega ao TLS o que veio num `P_CONTROL_V1` e junta o texto claro que sair dele.
fn feed_tls(slot: &mut Slot, payload: &[u8]) -> Result<(), rustls::Error> {
    let mut cursor = Cursor::new(payload);
    let total = payload.len() as u64;
    while cursor.position() < total {
        match slot.tls.read_tls(&mut cursor) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) => return Err(rustls::Error::General(e.to_string())),
        }
        slot.tls.process_new_packets()?;
        let mut buf = [0u8; READ_CHUNK];
        loop {
            match slot.tls.reader().read(&mut buf) {
                Ok(0) => {
                    slot.peer_closed = true;
                    break;
                }
                Ok(n) => slot.plain_in.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) => return Err(rustls::Error::General(e.to_string())),
            }
        }
    }
    Ok(())
}

/// Passa o que o TLS quer enviar para a confiabilidade, em pedaços que cabem no pacote.
pub(crate) fn pump_out(slot: &mut Slot, max_payload: usize) {
    while slot.tls.wants_write() {
        let mut out = Vec::new();
        if slot.tls.write_tls(&mut out).is_err() || out.is_empty() {
            break;
        }
        for chunk in out.chunks(max_payload) {
            slot.reliable.send(Opcode::ControlV1, chunk.to_vec());
        }
    }
}
