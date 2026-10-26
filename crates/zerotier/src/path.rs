//! Um caminho físico até um par: o endereço UDP remoto e o que se sabe dele (quando saiu e
//! chegou algo, a latência). Os tempos são milissegundos de um relógio que o chamador fornece:
//! o crate não lê o relógio, o que deixa o VL1 testável sem esperar.

use std::net::SocketAddr;

/// Intervalo do heartbeat de um caminho em uso (`ZT_PATH_HEARTBEAT_PERIOD`, `Constants.hpp`).
pub const HEARTBEAT_PERIOD: u64 = 14_000;

/// Um caminho vive enquanto chega algo autenticado por ele: HELLO completo a cada 60 s, quatro
/// vezes e mais 3 s de folga (`ZT_PEER_PATH_EXPIRATION`, `Constants.hpp`).
pub const PATH_EXPIRATION: u64 = 60_000 * 4 + 3_000;

/// Um caminho está "vivo" (serve para mandar) por um heartbeat e 5 s depois do último recebido
/// (`Path::alive`, `Path.hpp`).
pub const ALIVE_WINDOW: u64 = HEARTBEAT_PERIOD + 5_000;

/// Intervalo mínimo entre dois ECHO respondidos no mesmo caminho: um sexto do limite geral de
/// 1 s (`Path::rateGateEchoRequest`, `Path.hpp`).
const ECHO_RATE_GATE: u64 = 1_000 / 6;

/// Um caminho físico até um par.
#[derive(Clone, Debug)]
pub struct Path {
    addr: SocketAddr,
    last_send: u64,
    last_receive: u64,
    last_echo_request: u64,
    latency: Option<u32>,
}

impl Path {
    /// Um caminho que acabou de receber algo, em `now`.
    pub fn new(addr: SocketAddr, now: u64) -> Self {
        Self {
            addr,
            last_send: 0,
            last_receive: now,
            last_echo_request: 0,
            latency: None,
        }
    }

    /// O endereço remoto.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Chegou algo autenticado.
    pub fn received(&mut self, now: u64) {
        self.last_receive = now;
    }

    /// Saiu algo.
    pub fn sent(&mut self, now: u64) {
        self.last_send = now;
    }

    /// Quando chegou algo pela última vez.
    pub fn last_receive(&self) -> u64 {
        self.last_receive
    }

    /// Chegou algo há pouco: serve para mandar.
    pub fn alive(&self, now: u64) -> bool {
        now.saturating_sub(self.last_receive) < ALIVE_WINDOW
    }

    /// Passou do prazo e o caminho deve ser esquecido.
    pub fn expired(&self, now: u64) -> bool {
        now.saturating_sub(self.last_receive) >= PATH_EXPIRATION
    }

    /// Faz mais de um heartbeat que não sai nada por ele.
    pub fn needs_heartbeat(&self, now: u64) -> bool {
        now.saturating_sub(self.last_send) >= HEARTBEAT_PERIOD
    }

    /// A latência medida, em milissegundos.
    pub fn latency(&self) -> Option<u32> {
        self.latency
    }

    /// Guarda uma medida de latência. Com mais de uma, fica a média móvel de meio a meio,
    /// que suaviza sem lembrar demais (`Path::updateLatency`, `Path.hpp`).
    pub fn set_latency(&mut self, ms: u32) {
        self.latency = Some(match self.latency {
            Some(antiga) => (antiga + ms) / 2,
            None => ms,
        });
    }

    /// Vale responder a um ECHO agora? Registra o pedido se valer.
    pub fn allow_echo_reply(&mut self, now: u64) -> bool {
        if now.saturating_sub(self.last_echo_request) >= ECHO_RATE_GATE {
            self.last_echo_request = now;
            true
        } else {
            false
        }
    }

    /// A nota do caminho para escolher o melhor; menor é melhor. Vivo antes de morto, depois a
    /// menor latência conhecida (sem medida conta como alta), depois o mais recente.
    pub fn score(&self, now: u64) -> (bool, u32, u64) {
        (
            !self.alive(now),
            self.latency.unwrap_or(u32::MAX),
            u64::MAX - self.last_receive,
        )
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    fn endereco() -> SocketAddr {
        "192.0.2.1:9993".parse().unwrap()
    }

    #[test]
    fn o_caminho_vive_pouco_para_mandar_e_muito_para_lembrar() {
        let mut c = Path::new(endereco(), 1_000);
        assert!(c.alive(1_000 + ALIVE_WINDOW - 1));
        assert!(!c.alive(1_000 + ALIVE_WINDOW));
        assert!(!c.expired(1_000 + PATH_EXPIRATION - 1));
        assert!(c.expired(1_000 + PATH_EXPIRATION));
        c.received(50_000);
        assert!(c.alive(50_001));
    }

    #[test]
    fn o_heartbeat_sai_a_cada_14_segundos() {
        let mut c = Path::new(endereco(), 0);
        c.sent(100_000);
        assert!(!c.needs_heartbeat(100_000 + HEARTBEAT_PERIOD - 1));
        assert!(c.needs_heartbeat(100_000 + HEARTBEAT_PERIOD));
    }

    #[test]
    fn a_latencia_vira_media_movel() {
        let mut c = Path::new(endereco(), 0);
        assert_eq!(c.latency(), None);
        c.set_latency(40);
        assert_eq!(c.latency(), Some(40));
        c.set_latency(60);
        assert_eq!(c.latency(), Some(50));
    }

    #[test]
    fn o_eco_respondido_tem_limite_de_ritmo() {
        let mut c = Path::new(endereco(), 0);
        assert!(c.allow_echo_reply(1_000));
        assert!(!c.allow_echo_reply(1_050));
        assert!(c.allow_echo_reply(1_000 + ECHO_RATE_GATE));
    }

    #[test]
    fn o_vivo_ganha_do_morto_e_a_menor_latencia_do_resto() {
        let mut vivo_lento = Path::new(endereco(), 100_000);
        vivo_lento.set_latency(90);
        let mut vivo_rapido = Path::new("192.0.2.2:9993".parse().unwrap(), 100_000);
        vivo_rapido.set_latency(10);
        let morto = Path::new("192.0.2.3:9993".parse().unwrap(), 0);
        let agora = 101_000;
        assert!(vivo_rapido.score(agora) < vivo_lento.score(agora));
        assert!(vivo_lento.score(agora) < morto.score(agora));
    }
}
