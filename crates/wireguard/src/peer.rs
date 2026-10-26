//! Um par do WireGuard: o handshake, as chaves de sessão, as filas e os temporizadores.
//!
//! O `Peer` não abre soquete nem lê a hora: recebe bytes e um `Instant`, e devolve bytes.
//! O funcionamento (que segue o do WireGuard do Linux e do `wireguard-go`) é este:
//!
//! - [`Peer::encapsulate`] enfileira um pacote IP; sem sessão, começa um handshake.
//! - [`Peer::decapsulate`] recebe um datagrama da rede (tipos 2, 3 e 4; o tipo 1 passa antes
//!   pelo [`Responder`](crate::Responder) e entra por [`Peer::accept`]).
//! - [`Peer::update_timers`] precisa ser chamado quando o instante de
//!   [`Peer::next_deadline`] chegar.
//! - **Depois de qualquer chamada**, quem usa o `Peer` deve chamar [`Peer::poll_transmit`] até
//!   vir [`Output::Nothing`]: uma chamada só devolve a primeira coisa a enviar, e o resto (o
//!   keepalive que confirma a sessão, os pacotes que esperavam o handshake) sai nas seguintes.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::handshake::{
    Identity, InitiatorState, PeerKeys, TAMANHO_COOKIE, TAMANHO_RESPOSTA, TIPO_COOKIE,
    TIPO_RESPOSTA, TIPO_TRANSPORTE, abrir_resposta_de_cookie, carimbo_tai64n, consumir_resposta,
    criar_iniciacao, criar_resposta, mac1_confere, terminar_iniciacao,
};
use crate::indices::{IndexGuard, Indices};
use crate::key::{KeyError, PresharedKey, PublicKey, random_bytes};
use crate::responder::OpenedInitiation;
use crate::timers::{
    COOKIE_LIFETIME, INITIATION_MIN_INTERVAL, KEEPALIVE_TIMEOUT, REJECT_AFTER_MESSAGES,
    REJECT_AFTER_TIME, REKEY_AFTER_MESSAGES, REKEY_AFTER_TIME, REKEY_ATTEMPT_TIME, REKEY_JITTER_MS,
    REKEY_TIMEOUT, Timers, ZERO_KEYS_FACTOR, venceu,
};
use crate::transport::{Keypair, TAMANHO_KEEPALIVE};

/// A MTU padrão do túnel (a do `wg-quick`).
pub const MTU_PADRAO: usize = 1420;
/// Quantos pacotes esperam o handshake, no máximo (o mesmo limite do `wireguard-go`).
const FILA_MAX_PACOTES: usize = 128;
/// Quantos bytes esperam o handshake, no máximo.
const FILA_MAX_BYTES: usize = 64 * 1024;

/// A configuração de um par.
#[derive(Debug, Clone)]
pub struct PeerConfig {
    /// A chave pública do par.
    pub public_key: PublicKey,
    /// A chave pré-compartilhada, se houver.
    pub preshared_key: Option<PresharedKey>,
    /// O keepalive persistente (o `PersistentKeepalive` do `.conf`), se houver.
    pub persistent_keepalive: Option<Duration>,
    /// A MTU do túnel: o preenchimento do transporte não a ultrapassa.
    pub mtu: usize,
}

impl PeerConfig {
    /// A configuração mínima: só a chave pública.
    pub fn new(public_key: PublicKey) -> Self {
        PeerConfig {
            public_key,
            preshared_key: None,
            persistent_keepalive: None,
            mtu: MTU_PADRAO,
        }
    }

    /// Define a chave pré-compartilhada.
    pub fn with_preshared_key(mut self, psk: PresharedKey) -> Self {
        self.preshared_key = Some(psk);
        self
    }

    /// Define o keepalive persistente, em segundos (0 desliga).
    pub fn with_persistent_keepalive_secs(mut self, segundos: u16) -> Self {
        self.persistent_keepalive = if segundos == 0 {
            None
        } else {
            Some(Duration::from_secs(u64::from(segundos)))
        };
        self
    }
}

/// Um erro do par que quem o usa precisa saber.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerError {
    /// O par não respondeu ao handshake durante `REKEY_ATTEMPT_TIME` (90 s): os pacotes que
    /// esperavam foram descartados e o par está inalcançável.
    HandshakeTimeout,
    /// O sistema não forneceu números aleatórios.
    Random,
}

impl std::fmt::Display for PeerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PeerError::HandshakeTimeout => write!(
                f,
                "o par não respondeu ao handshake em {} s",
                REKEY_ATTEMPT_TIME.as_secs()
            ),
            PeerError::Random => write!(f, "o sistema não forneceu números aleatórios"),
        }
    }
}

impl std::error::Error for PeerError {}

impl From<KeyError> for PeerError {
    fn from(_: KeyError) -> Self {
        PeerError::Random
    }
}

/// Por que uma iniciação foi recusada por [`Peer::accept`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcceptError {
    /// O carimbo de tempo não é maior que o da última iniciação aceita (repetição).
    Replay,
    /// Outra iniciação chegou há menos de 20 ms (enxurrada).
    Flood,
    /// O carimbo não abriu: a mensagem não é deste par.
    Crypto,
    /// O sistema não forneceu números aleatórios.
    Random,
}

impl std::fmt::Display for AcceptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AcceptError::Replay => write!(f, "iniciação repetida (carimbo de tempo velho)"),
            AcceptError::Flood => write!(f, "iniciações em enxurrada; esta foi descartada"),
            AcceptError::Crypto => write!(f, "a iniciação não abriu com as chaves deste par"),
            AcceptError::Random => write!(f, "o sistema não forneceu números aleatórios"),
        }
    }
}

impl std::error::Error for AcceptError {}

/// O que uma chamada do [`Peer`] produziu.
#[derive(Debug, PartialEq, Eq)]
pub enum Output<'a> {
    /// Nada (mais) a fazer.
    Nothing,
    /// Mandar este datagrama ao endpoint do par.
    Network(&'a [u8]),
    /// Entregar este pacote IP à pilha de rede.
    Ip(&'a [u8]),
    /// Algo que quem usa precisa tratar.
    Error(PeerError),
}

/// Os contadores de um par.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PeerStats {
    /// Bytes de datagramas enviados.
    pub tx_bytes: u64,
    /// Bytes de datagramas autenticados recebidos.
    pub rx_bytes: u64,
    /// Quando o último handshake terminou.
    pub last_handshake: Option<Instant>,
}

/// O estado de um par.
pub struct Peer {
    identity: Arc<Identity>,
    keys: PeerKeys,
    public: PublicKey,
    indices: Indices,
    mtu: usize,
    persistent: Option<Duration>,

    iniciacao: Option<(InitiatorState, IndexGuard)>,
    ultima_iniciacao: Option<Instant>,
    inicio_da_tentativa: Option<Instant>,
    ultimo_mac1: Option<[u8; 16]>,
    cookie: Option<([u8; 16], Instant)>,
    ultimo_carimbo: Option<[u8; 12]>,
    ultima_iniciacao_aceita: Option<Instant>,

    anterior: Option<Keypair>,
    atual: Option<Keypair>,
    proxima: Option<Keypair>,

    fila: VecDeque<Vec<u8>>,
    fila_bytes: usize,
    prontas: VecDeque<Vec<u8>>,
    quer_keepalive: bool,
    quer_handshake: bool,
    timers: Timers,
    mandou_handshake_no_ultimo_minuto: bool,
    autenticou: bool,

    saida: Vec<u8>,
    claro: Vec<u8>,

    ancora_instante: Instant,
    ancora_unix: Duration,

    stats: PeerStats,
}

impl Peer {
    /// Cria o par. `now` fixa a âncora do relógio do carimbo de tempo do handshake (o carimbo é
    /// a hora do sistema na criação mais o tempo decorrido, para andar sempre para a frente).
    pub fn new(
        identity: Arc<Identity>,
        indices: Indices,
        config: PeerConfig,
        now: Instant,
    ) -> Result<Self, KeyError> {
        let keys = PeerKeys::new(&identity, config.public_key, config.preshared_key.as_ref())?;
        let unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let mut peer = Peer {
            identity,
            keys,
            public: config.public_key,
            indices,
            mtu: config.mtu,
            persistent: config.persistent_keepalive,
            iniciacao: None,
            ultima_iniciacao: None,
            inicio_da_tentativa: None,
            ultimo_mac1: None,
            cookie: None,
            ultimo_carimbo: None,
            ultima_iniciacao_aceita: None,
            anterior: None,
            atual: None,
            proxima: None,
            fila: VecDeque::new(),
            fila_bytes: 0,
            prontas: VecDeque::new(),
            quer_keepalive: false,
            quer_handshake: false,
            timers: Timers::default(),
            mandou_handshake_no_ultimo_minuto: false,
            autenticou: false,
            saida: Vec::with_capacity(2048),
            claro: Vec::with_capacity(2048),
            ancora_instante: now,
            ancora_unix: unix,
            stats: PeerStats::default(),
        };
        if peer.persistent.is_some() {
            // Com keepalive persistente o par já nasce querendo estabelecer a sessão.
            peer.quer_keepalive = true;
        }
        Ok(peer)
    }

    /// A chave pública do par.
    pub fn public_key(&self) -> PublicKey {
        self.public
    }

    /// Os contadores do par.
    pub fn stats(&self) -> PeerStats {
        self.stats
    }

    /// Muda o keepalive persistente.
    pub fn set_persistent_keepalive(&mut self, intervalo: Option<Duration>, now: Instant) {
        self.persistent = intervalo;
        self.timers.persistent_keepalive = intervalo.map(|i| now + i);
        if intervalo.is_some() {
            self.quer_keepalive = true;
        }
    }

    /// Pede o estabelecimento da sessão (um keepalive que, sem sessão, dispara o handshake).
    pub fn connect(&mut self) {
        self.quer_keepalive = true;
    }

    /// Se há uma sessão que serve para mandar dados agora.
    pub fn has_session(&self, now: Instant) -> bool {
        self.pode_enviar(now)
    }

    /// Se um handshake está em andamento (iniciação mandada, sem resposta ainda).
    pub fn handshake_in_flight(&self) -> bool {
        self.iniciacao.is_some()
    }

    /// Diz (e zera) se desde a última consulta chegou algo autenticado deste par. O
    /// [`Device`](crate::Device) usa para atualizar o endpoint (o par pode ter mudado de
    /// endereço).
    pub fn take_authenticated(&mut self) -> bool {
        std::mem::take(&mut self.autenticou)
    }

    /// Quando [`Peer::update_timers`] precisa ser chamado de novo.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.timers.next_deadline()
    }

    // ---------------------------------------------------------------- entrada e saída

    /// Enfileira um pacote IP para o par. Devolve a primeira coisa a mandar (veja o aviso do
    /// topo do módulo sobre [`Peer::poll_transmit`]).
    pub fn encapsulate(&mut self, pacote: &[u8], now: Instant) -> Output<'_> {
        if !pacote.is_empty() {
            self.enfileirar(pacote.to_vec());
        }
        self.poll_transmit(now)
    }

    /// Processa um datagrama que chegou da rede para este par (tipos 2, 3 e 4).
    pub fn decapsulate(&mut self, datagrama: &[u8], now: Instant) -> Output<'_> {
        match self.processar(datagrama, now) {
            Processado::Ip(n) => Output::Ip(&self.claro[..n]),
            Processado::Nada => self.poll_transmit(now),
        }
    }

    /// Enfileira um pacote IP sem tentar mandar nada (o chamador usa [`Peer::poll_transmit`]).
    pub fn queue(&mut self, pacote: &[u8]) {
        if !pacote.is_empty() {
            self.enfileirar(pacote.to_vec());
        }
    }

    /// Processa a entrada sem devolver saída: o pacote IP, se houver, fica em
    /// [`Peer::plain`].
    pub(crate) fn processar(&mut self, datagrama: &[u8], now: Instant) -> Processado {
        if datagrama.len() < 4 {
            return Processado::Nada;
        }
        let tipo = u32::from_le_bytes([datagrama[0], datagrama[1], datagrama[2], datagrama[3]]);
        match tipo {
            TIPO_RESPOSTA => self.ao_receber_resposta(datagrama, now),
            TIPO_COOKIE => self.ao_receber_cookie(datagrama, now),
            TIPO_TRANSPORTE => {
                if let Some(n) = self.ao_receber_transporte(datagrama, now) {
                    return Processado::Ip(n);
                }
            }
            _ => {}
        }
        Processado::Nada
    }

    /// Os primeiros `n` bytes do último pacote IP aberto.
    pub(crate) fn plain(&self, n: usize) -> &[u8] {
        &self.claro[..n]
    }

    /// Aceita uma iniciação aberta pelo [`Responder`](crate::Responder): confere o carimbo de
    /// tempo, cria a resposta e a sessão (ainda não confirmada). A resposta sai em
    /// [`Peer::poll_transmit`].
    pub fn accept(&mut self, aberta: OpenedInitiation, now: Instant) -> Result<(), AcceptError> {
        let parcial = aberta.parcial;
        if parcial.peer != self.public {
            return Err(AcceptError::Crypto);
        }
        let (carimbo, estado) =
            terminar_iniciacao(&self.keys, parcial).ok_or(AcceptError::Crypto)?;
        if let Some(ultimo) = self.ultimo_carimbo
            && carimbo <= ultimo
        {
            return Err(AcceptError::Replay);
        }
        if let Some(t) = self.ultima_iniciacao_aceita
            && now.saturating_duration_since(t) <= INITIATION_MIN_INTERVAL
        {
            return Err(AcceptError::Flood);
        }
        self.ultimo_carimbo = Some(carimbo);
        self.ultima_iniciacao_aceita = Some(now);

        let indice = self
            .indices
            .allocate(self.public)
            .map_err(|_| AcceptError::Random)?;
        let cookie = self.cookie_valido(now);
        let (msg, chaves, mac1) = criar_resposta(&self.keys, &estado, indice.id(), cookie.as_ref())
            .map_err(|_| AcceptError::Random)?;
        self.ultimo_mac1 = Some(mac1);
        // Um handshake próprio em andamento é mantido (diferente do wireguard-go, que o
        // sobrescreve): com os dois lados iniciando ao mesmo tempo, cada um ainda consome a
        // resposta à sua iniciação e as duas pontas chegam a uma sessão sem esperar a
        // retransmissão (5 s).
        let par = Keypair::new(chaves, indice, estado.remote_index, now, false);
        self.proxima = Some(par);
        self.anterior = None;
        self.timers.zero_key_material = Some(now + REJECT_AFTER_TIME * ZERO_KEYS_FACTOR);
        self.ao_receber_autenticado(now);
        self.prontas.push_back(msg.to_vec());
        Ok(())
    }

    /// Pede um keepalive (vai com o próximo envio; sem sessão, começa o handshake).
    pub fn send_keepalive(&mut self) {
        self.quer_keepalive = true;
    }

    /// Devolve o próximo datagrama a mandar, se houver. Chame até vir [`Output::Nothing`].
    pub fn poll_transmit(&mut self, now: Instant) -> Output<'_> {
        // 1. Mensagens de handshake prontas (a resposta a uma iniciação).
        if let Some(m) = self.prontas.pop_front() {
            self.saida = m;
            self.ao_enviar(now);
            self.stats.tx_bytes += self.saida.len() as u64;
            return Output::Network(&self.saida);
        }
        // 2. Uma iniciação pedida.
        if self.quer_handshake {
            self.quer_handshake = false;
            match self.tentar_iniciacao(now) {
                Ok(true) => return Output::Network(&self.saida),
                Ok(false) => {}
                Err(e) => return Output::Error(e),
            }
        }
        // 3. Dados (ou o keepalive).
        if !self.fila.is_empty() || self.quer_keepalive {
            if self.pode_enviar(now) {
                let (carga, e_keepalive) = match self.fila.pop_front() {
                    Some(p) => {
                        self.fila_bytes -= p.len();
                        (p, false)
                    }
                    None => (Vec::new(), true),
                };
                let mtu = self.mtu;
                let Some(par) = self.atual.as_mut() else {
                    return Output::Nothing;
                };
                if par.sealar(&carga, mtu, &mut self.saida) {
                    self.quer_keepalive = false;
                    self.ao_enviar(now);
                    if !e_keepalive {
                        self.ao_enviar_dados(now);
                    }
                    self.manter_chave_fresca_ao_enviar(now);
                    self.stats.tx_bytes += self.saida.len() as u64;
                    return Output::Network(&self.saida);
                }
                // O contador da sessão esgotou: o pacote espera a sessão nova.
                if !e_keepalive {
                    self.fila_bytes += carga.len();
                    self.fila.push_front(carga);
                }
            }
            // Com um handshake em andamento, quem reenvia é o relógio de retransmissão.
            if self.iniciacao.is_none() {
                match self.tentar_iniciacao(now) {
                    Ok(true) => return Output::Network(&self.saida),
                    Ok(false) => {}
                    Err(e) => return Output::Error(e),
                }
            }
        }
        Output::Nothing
    }

    /// Avança os relógios. Chame quando [`Peer::next_deadline`] chegar (chamar antes também é
    /// inofensivo). Devolve [`Output::Error`] com [`PeerError::HandshakeTimeout`] quando o par
    /// foi dado como inalcançável.
    pub fn update_timers(&mut self, now: Instant) -> Output<'_> {
        if venceu(self.timers.zero_key_material, now) {
            self.timers.zero_key_material = None;
            self.apagar_chaves();
        }
        if venceu(self.timers.retransmit_handshake, now) {
            self.timers.retransmit_handshake = None;
            let esgotou = self
                .inicio_da_tentativa
                .is_some_and(|t| now.saturating_duration_since(t) > REKEY_ATTEMPT_TIME);
            if esgotou {
                self.inicio_da_tentativa = None;
                self.timers.send_keepalive = None;
                self.timers.need_another_keepalive = false;
                self.fila.clear();
                self.fila_bytes = 0;
                self.quer_keepalive = false;
                self.quer_handshake = false;
                self.iniciacao = None;
                if self.timers.zero_key_material.is_none() {
                    self.timers.zero_key_material =
                        Some(now + REJECT_AFTER_TIME * ZERO_KEYS_FACTOR);
                }
                return Output::Error(PeerError::HandshakeTimeout);
            }
            self.quer_handshake = true;
        }
        if venceu(self.timers.new_handshake, now) {
            self.timers.new_handshake = None;
            self.quer_handshake = true;
        }
        if venceu(self.timers.send_keepalive, now) {
            self.timers.send_keepalive = None;
            self.quer_keepalive = true;
            if self.timers.need_another_keepalive {
                self.timers.need_another_keepalive = false;
                self.timers.send_keepalive = Some(now + KEEPALIVE_TIMEOUT);
            }
        }
        if venceu(self.timers.persistent_keepalive, now) {
            self.timers.persistent_keepalive = self.persistent.map(|i| now + i);
            if self.persistent.is_some() {
                self.quer_keepalive = true;
            }
        }
        self.poll_transmit(now)
    }

    // ---------------------------------------------------------------- recepção

    fn ao_receber_resposta(&mut self, datagrama: &[u8], now: Instant) {
        let Ok(msg) = <&[u8; TAMANHO_RESPOSTA]>::try_from(datagrama) else {
            return;
        };
        if !mac1_confere(&self.identity, msg) {
            return;
        }
        let Some((estado, _)) = self.iniciacao.as_ref() else {
            return;
        };
        let Some((chaves, indice_remoto)) =
            consumir_resposta(&self.identity, &self.keys, estado, msg)
        else {
            return;
        };
        let Some((_, indice)) = self.iniciacao.take() else {
            return;
        };
        // BeginSymmetricSession como iniciador.
        let par = Keypair::new(chaves, indice, indice_remoto, now, true);
        self.anterior = None;
        if let Some(proxima) = self.proxima.take() {
            self.anterior = Some(proxima);
            self.atual = None;
        } else {
            self.anterior = self.atual.take();
        }
        self.atual = Some(par);
        self.timers.zero_key_material = Some(now + REJECT_AFTER_TIME * ZERO_KEYS_FACTOR);
        self.handshake_completo(now);
        self.ao_receber_autenticado(now);
        self.stats.rx_bytes += datagrama.len() as u64;
        // Confirma a sessão para o respondedor: dados que esperavam ou um keepalive.
        self.quer_keepalive = true;
    }

    fn ao_receber_cookie(&mut self, datagrama: &[u8], now: Instant) {
        if datagrama.len() != TAMANHO_COOKIE {
            return;
        }
        let Some(mac1) = self.ultimo_mac1 else {
            return;
        };
        if let Some((_, cookie)) = abrir_resposta_de_cookie(&self.keys, datagrama, &mac1) {
            // A próxima retransmissão da iniciação leva o `mac2`.
            self.cookie = Some((cookie, now));
        }
    }

    /// Abre um pacote de transporte. Devolve o tamanho do pacote IP em `self.claro`, ou `None`
    /// (keepalive, ou pacote descartado).
    fn ao_receber_transporte(&mut self, datagrama: &[u8], now: Instant) -> Option<usize> {
        if datagrama.len() < TAMANHO_KEEPALIVE {
            return None;
        }
        let receptor = u32::from_le_bytes([datagrama[4], datagrama[5], datagrama[6], datagrama[7]]);
        let e_proxima = self
            .proxima
            .as_ref()
            .is_some_and(|k| k.local.id() == receptor);
        let par = (if self
            .atual
            .as_ref()
            .is_some_and(|k| k.local.id() == receptor)
        {
            self.atual.as_mut()
        } else if self
            .anterior
            .as_ref()
            .is_some_and(|k| k.local.id() == receptor)
        {
            self.anterior.as_mut()
        } else if e_proxima {
            self.proxima.as_mut()
        } else {
            None
        })?;
        if now.saturating_duration_since(par.created) >= REJECT_AFTER_TIME {
            return None;
        }
        par.abrir(datagrama, &mut self.claro).ok()?;
        self.stats.rx_bytes += datagrama.len() as u64;

        // ReceivedWithKeypair: a primeira mensagem na sessão "próxima" a confirma.
        let promovida = e_proxima;
        if promovida {
            self.anterior = self.atual.take();
            self.atual = self.proxima.take();
        }
        self.ao_receber_autenticado(now);
        if promovida {
            self.handshake_completo(now);
        }
        self.manter_chave_fresca_ao_receber(now);
        if self.claro.is_empty() {
            // Keepalive.
            return None;
        }
        self.ao_receber_dados(now);
        tamanho_do_pacote_ip(&self.claro)
    }

    // ---------------------------------------------------------------- relógios e eventos

    fn ao_enviar(&mut self, now: Instant) {
        self.timers.send_keepalive = None;
        if let Some(i) = self.persistent {
            self.timers.persistent_keepalive = Some(now + i);
        }
    }

    fn ao_enviar_dados(&mut self, now: Instant) {
        if self.timers.new_handshake.is_none() {
            self.timers.new_handshake = Some(now + KEEPALIVE_TIMEOUT + REKEY_TIMEOUT);
        }
    }

    fn ao_receber_autenticado(&mut self, now: Instant) {
        self.timers.new_handshake = None;
        self.autenticou = true;
        if let Some(i) = self.persistent {
            self.timers.persistent_keepalive = Some(now + i);
        }
    }

    fn ao_receber_dados(&mut self, now: Instant) {
        if self.timers.send_keepalive.is_none() {
            self.timers.need_another_keepalive = false;
            self.timers.send_keepalive = Some(now + KEEPALIVE_TIMEOUT);
        } else {
            self.timers.need_another_keepalive = true;
        }
    }

    fn handshake_completo(&mut self, now: Instant) {
        self.timers.retransmit_handshake = None;
        self.inicio_da_tentativa = None;
        self.mandou_handshake_no_ultimo_minuto = false;
        self.stats.last_handshake = Some(now);
    }

    fn manter_chave_fresca_ao_enviar(&mut self, now: Instant) {
        let Some(atual) = self.atual.as_ref() else {
            return;
        };
        if atual.send_counter > REKEY_AFTER_MESSAGES
            || (atual.is_initiator
                && now.saturating_duration_since(atual.created) > REKEY_AFTER_TIME)
        {
            self.quer_handshake = true;
        }
    }

    fn manter_chave_fresca_ao_receber(&mut self, now: Instant) {
        if self.mandou_handshake_no_ultimo_minuto {
            return;
        }
        let Some(atual) = self.atual.as_ref() else {
            return;
        };
        if atual.is_initiator
            && now.saturating_duration_since(atual.created)
                > REJECT_AFTER_TIME - KEEPALIVE_TIMEOUT - REKEY_TIMEOUT
        {
            self.mandou_handshake_no_ultimo_minuto = true;
            self.quer_handshake = true;
        }
    }

    fn apagar_chaves(&mut self) {
        self.iniciacao = None;
        self.anterior = None;
        self.atual = None;
        self.proxima = None;
        self.fila.clear();
        self.fila_bytes = 0;
        self.quer_keepalive = false;
    }

    // ---------------------------------------------------------------- ajudantes

    fn pode_enviar(&self, now: Instant) -> bool {
        self.atual.as_ref().is_some_and(|k| {
            k.send_counter < REJECT_AFTER_MESSAGES
                && now.saturating_duration_since(k.created) < REJECT_AFTER_TIME
        })
    }

    fn enfileirar(&mut self, pacote: Vec<u8>) {
        while !self.fila.is_empty()
            && (self.fila.len() >= FILA_MAX_PACOTES
                || self.fila_bytes + pacote.len() > FILA_MAX_BYTES)
        {
            if let Some(velho) = self.fila.pop_front() {
                self.fila_bytes -= velho.len();
            }
        }
        self.fila_bytes += pacote.len();
        self.fila.push_back(pacote);
    }

    fn cookie_valido(&self, now: Instant) -> Option<[u8; 16]> {
        self.cookie.and_then(|(c, quando)| {
            (now.saturating_duration_since(quando) < COOKIE_LIFETIME).then_some(c)
        })
    }

    fn carimbo(&self, now: Instant) -> [u8; 12] {
        let d = self.ancora_unix + now.saturating_duration_since(self.ancora_instante);
        carimbo_tai64n(d.as_secs(), d.subsec_nanos())
    }

    /// Monta uma iniciação em `self.saida`, se o intervalo mínimo desde a última passou.
    fn tentar_iniciacao(&mut self, now: Instant) -> Result<bool, PeerError> {
        if let Some(t) = self.ultima_iniciacao
            && now.saturating_duration_since(t) < REKEY_TIMEOUT
        {
            return Ok(false);
        }
        let indice = self.indices.allocate(self.public)?;
        let carimbo = self.carimbo(now);
        let cookie = self.cookie_valido(now);
        let (msg, estado) = criar_iniciacao(
            &self.identity,
            &self.keys,
            indice.id(),
            &carimbo,
            cookie.as_ref(),
        )?;
        self.ultimo_mac1 = Some(estado.mac1);
        // Soltar o handshake anterior devolve o índice dele ao registro.
        self.iniciacao = Some((estado, indice));
        self.ultima_iniciacao = Some(now);
        if self.inicio_da_tentativa.is_none() {
            self.inicio_da_tentativa = Some(now);
        }
        self.saida.clear();
        self.saida.extend_from_slice(&msg);
        self.ao_enviar(now);
        self.timers.retransmit_handshake = Some(now + REKEY_TIMEOUT + sorteio_de_variacao());
        self.stats.tx_bytes += self.saida.len() as u64;
        Ok(true)
    }
}

/// O resultado de processar um datagrama.
pub(crate) enum Processado {
    /// Nada para entregar.
    Nada,
    /// Um pacote IP de tamanho `n`, em [`Peer::plain`].
    Ip(usize),
}

/// A variação aleatória (de 0 a 333 ms) somada ao `REKEY_TIMEOUT`.
fn sorteio_de_variacao() -> Duration {
    let ms = random_bytes::<2>()
        .map(|b| u64::from(u16::from_le_bytes(b)) % (REKEY_JITTER_MS + 1))
        .unwrap_or(0);
    Duration::from_millis(ms)
}

/// O tamanho do pacote IP (sem o preenchimento de zeros) no início de `claro`, se o cabeçalho
/// for de IPv4 ou IPv6 e o tamanho declarado couber.
fn tamanho_do_pacote_ip(claro: &[u8]) -> Option<usize> {
    let primeiro = *claro.first()?;
    let total = match primeiro >> 4 {
        4 if claro.len() >= 20 => usize::from(u16::from_be_bytes([claro[2], claro[3]])),
        6 if claro.len() >= 40 => 40 + usize::from(u16::from_be_bytes([claro[4], claro[5]])),
        _ => return None,
    };
    (total >= 20 && total <= claro.len()).then_some(total)
}
