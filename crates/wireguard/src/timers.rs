//! As constantes de tempo do WireGuard (artigo, seção 6.1) e os relógios de um par.
//!
//! Nenhum relógio lê a hora do sistema: o `Instant` entra sempre por parâmetro, e os testes
//! andam com o tempo à mão.

use std::time::{Duration, Instant};

/// Depois de quantas mensagens a chave de sessão deve ser trocada (2^60).
pub const REKEY_AFTER_MESSAGES: u64 = 1 << 60;
/// Com quantas mensagens a chave de sessão morre de vez (2^64 − 2^13 − 1).
pub const REJECT_AFTER_MESSAGES: u64 = u64::MAX - (1 << 13);
/// Depois de quanto tempo quem iniciou troca a chave de sessão.
pub const REKEY_AFTER_TIME: Duration = Duration::from_secs(120);
/// Com quanto tempo a chave de sessão morre de vez.
pub const REJECT_AFTER_TIME: Duration = Duration::from_secs(180);
/// Quanto tempo se insiste em um handshake antes de desistir.
pub const REKEY_ATTEMPT_TIME: Duration = Duration::from_secs(90);
/// O intervalo entre as retransmissões de uma iniciação (mais até 333 ms de variação).
pub const REKEY_TIMEOUT: Duration = Duration::from_secs(5);
/// Quanto tempo depois de receber dados, sem ter o que mandar, se manda um keepalive.
pub const KEEPALIVE_TIMEOUT: Duration = Duration::from_secs(10);
/// A vida do cookie recebido (e a troca do segredo do cookie que damos).
pub const COOKIE_LIFETIME: Duration = Duration::from_secs(120);
/// O intervalo mínimo entre duas iniciações aceitas do mesmo par (proteção contra enxurrada).
pub const INITIATION_MIN_INTERVAL: Duration = Duration::from_millis(20);
/// A variação máxima somada ao `REKEY_TIMEOUT`, em milissegundos.
pub const REKEY_JITTER_MS: u64 = 333;
/// Depois de quantas vidas de sessão sem notícias o material de chave é apagado.
pub const ZERO_KEYS_FACTOR: u32 = 3;

/// Os relógios de um par: cada um é o instante em que vence, se estiver armado.
#[derive(Debug, Default, Clone)]
pub(crate) struct Timers {
    /// Retransmitir a iniciação.
    pub(crate) retransmit_handshake: Option<Instant>,
    /// Iniciar um handshake novo porque mandamos dados e ninguém respondeu.
    pub(crate) new_handshake: Option<Instant>,
    /// Mandar um keepalive porque recebemos dados e não tínhamos o que responder.
    pub(crate) send_keepalive: Option<Instant>,
    /// Há um keepalive passivo a mais para armar quando o atual vencer.
    pub(crate) need_another_keepalive: bool,
    /// O keepalive persistente.
    pub(crate) persistent_keepalive: Option<Instant>,
    /// Apagar o material de chave.
    pub(crate) zero_key_material: Option<Instant>,
}

impl Timers {
    /// O primeiro instante em que algum relógio vence.
    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        [
            self.retransmit_handshake,
            self.new_handshake,
            self.send_keepalive,
            self.persistent_keepalive,
            self.zero_key_material,
        ]
        .into_iter()
        .flatten()
        .min()
    }
}

/// Se o relógio `t` venceu em `now`.
pub(crate) fn venceu(t: Option<Instant>, now: Instant) -> bool {
    matches!(t, Some(t) if now >= t)
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn as_constantes_sao_as_do_artigo() {
        assert_eq!(REKEY_AFTER_MESSAGES, 1 << 60);
        assert_eq!(REJECT_AFTER_MESSAGES, 0xffff_ffff_ffff_dfff);
        assert_eq!(REKEY_AFTER_TIME.as_secs(), 120);
        assert_eq!(REJECT_AFTER_TIME.as_secs(), 180);
        assert_eq!(REKEY_ATTEMPT_TIME.as_secs(), 90);
        assert_eq!(REKEY_TIMEOUT.as_secs(), 5);
        assert_eq!(KEEPALIVE_TIMEOUT.as_secs(), 10);
    }

    #[test]
    fn o_proximo_vencimento_e_o_menor() {
        let t0 = Instant::now();
        let mut t = Timers::default();
        assert_eq!(t.next_deadline(), None);
        t.new_handshake = Some(t0 + Duration::from_secs(15));
        t.send_keepalive = Some(t0 + Duration::from_secs(10));
        assert_eq!(t.next_deadline(), Some(t0 + Duration::from_secs(10)));
        assert!(venceu(t.send_keepalive, t0 + Duration::from_secs(10)));
        assert!(!venceu(t.send_keepalive, t0 + Duration::from_secs(9)));
        assert!(!venceu(None, t0));
    }
}
