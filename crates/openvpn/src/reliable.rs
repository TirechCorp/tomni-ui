//! A camada de confiabilidade do canal de controle.
//!
//! O TLS precisa de um transporte que entregue tudo, em ordem e sem repetir. O UDP não dá isso,
//! então o OpenVPN põe sua própria camada embaixo do TLS: cada mensagem de controle tem um id
//! sequencial (começando em 0), o outro lado confirma cada uma (ACK) e o remetente reenvia a que
//! não foi confirmada. Esta camada não conhece o TLS nem a rede: recebe e entrega mensagens, e o
//! relógio (`Instant`) é sempre um argumento, para que os testes controlem o tempo.
//!
//! # Regras
//!
//! - No máximo `send_window` mensagens enviadas e ainda sem confirmação (o padrão do protocolo é 6).
//! - Mensagens fora de ordem, até `recv_window` à frente da esperada, ficam guardadas até as
//!   anteriores chegarem (o padrão é 12). Mais longe que isso, o pacote é descartado sem ACK.
//! - Uma mensagem repetida (id já entregue ou guardado) não é entregue de novo, mas é confirmada
//!   de novo, porque o ACK anterior pode ter se perdido.
//! - O reenvio espera o tempo inicial (2 s) e dobra a cada tentativa, até `inicial << 6`.
//! - Em TCP não há reenvio por tempo: o transporte já garante a entrega. As confirmações
//!   continuam, porque o outro lado as espera.
//! - Cada geração de chaves (`key_id`) tem a sua própria camada, com ids começando em 0.

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

use crate::packet::{
    ControlPacket, MAX_ACKS_IN_ACK_PACKET, MAX_ACKS_IN_CONTROL, Opcode, SessionId,
};

/// Os parâmetros da camada.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReliableConfig {
    /// Quantas mensagens podem estar enviadas e sem confirmação (6 no protocolo).
    pub send_window: usize,
    /// Quantas mensagens à frente da esperada são guardadas (12 no protocolo).
    pub recv_window: usize,
    /// O tempo de espera do primeiro reenvio (2 s no protocolo).
    pub initial_timeout: Duration,
    /// Quantas vezes o tempo pode dobrar (6 no protocolo: o teto é `inicial << 6`).
    pub max_backoff_shift: u32,
    /// Se reenvia por tempo (UDP). Em TCP é `false`.
    pub retransmit: bool,
}

impl ReliableConfig {
    /// Os valores do protocolo para UDP.
    pub fn udp() -> ReliableConfig {
        ReliableConfig {
            send_window: 6,
            recv_window: 12,
            initial_timeout: Duration::from_secs(2),
            max_backoff_shift: 6,
            retransmit: true,
        }
    }

    /// Os valores do protocolo para TCP: sem reenvio por tempo.
    pub fn tcp() -> ReliableConfig {
        ReliableConfig {
            retransmit: false,
            ..ReliableConfig::udp()
        }
    }
}

/// Uma mensagem que a camada entregou, na ordem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Received {
    /// O opcode com que a mensagem veio.
    pub opcode: Opcode,
    /// A carga.
    pub payload: Vec<u8>,
}

/// O que a camada manda enviar. O resto do pacote (`key_id`, as sessões) é de quem chama.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transmit {
    /// O opcode: o da mensagem, ou `AckV1` para um pacote só de confirmações.
    pub opcode: Opcode,
    /// O id da mensagem (`None` no pacote só de confirmações).
    pub message_id: Option<u32>,
    /// As confirmações que vão junto.
    pub acks: Vec<u32>,
    /// A carga.
    pub payload: Vec<u8>,
}

impl Transmit {
    /// Monta o pacote completo. `remote_session_id` só vai se houver confirmações.
    pub fn into_packet(
        self,
        key_id: u8,
        session_id: SessionId,
        remote_session_id: Option<SessionId>,
    ) -> ControlPacket {
        let remote_session_id = if self.acks.is_empty() {
            None
        } else {
            remote_session_id
        };
        ControlPacket {
            opcode: self.opcode,
            key_id,
            session_id,
            acks: self.acks,
            remote_session_id,
            message_id: self.message_id,
            payload: self.payload,
        }
    }
}

#[derive(Debug)]
struct InFlight {
    id: u32,
    opcode: Opcode,
    payload: Vec<u8>,
    /// Quantas vezes já foi enviada.
    sends: u32,
    /// Quando precisa ser (re)enviada.
    due: Instant,
}

/// A camada de confiabilidade de uma geração de chaves.
#[derive(Debug)]
pub struct Reliable {
    config: ReliableConfig,
    /// O id da próxima mensagem a ser enfileirada.
    next_send_id: u32,
    /// Mensagens que ainda não couberam na janela.
    queue: VecDeque<(u32, Opcode, Vec<u8>)>,
    in_flight: Vec<InFlight>,
    /// O id da próxima mensagem a entregar.
    next_expected: u32,
    /// Mensagens fora de ordem, esperando as anteriores.
    buffered: BTreeMap<u32, Received>,
    delivered: VecDeque<Received>,
    /// Ids a confirmar, sem repetição, na ordem em que chegaram.
    pending_acks: Vec<u32>,
}

impl Reliable {
    /// Uma camada vazia.
    pub fn new(config: ReliableConfig) -> Reliable {
        Reliable {
            config,
            next_send_id: 0,
            queue: VecDeque::new(),
            in_flight: Vec::new(),
            next_expected: 0,
            buffered: BTreeMap::new(),
            delivered: VecDeque::new(),
            pending_acks: Vec::new(),
        }
    }

    /// Enfileira uma mensagem para enviar e devolve o id que ela terá.
    pub fn send(&mut self, opcode: Opcode, payload: Vec<u8>) -> u32 {
        let id = self.next_send_id;
        self.next_send_id = self.next_send_id.wrapping_add(1);
        self.queue.push_back((id, opcode, payload));
        id
    }

    /// Quantas mensagens estão enviadas e sem confirmação.
    pub fn in_flight(&self) -> usize {
        self.in_flight.len()
    }

    /// Quantas mensagens esperam por lugar na janela.
    pub fn queued(&self) -> usize {
        self.queue.len()
    }

    /// Se tudo o que foi enfileirado já foi confirmado.
    pub fn all_acked(&self) -> bool {
        self.queue.is_empty() && self.in_flight.is_empty()
    }

    /// Processa um pacote que chegou: tira da fila de reenvio o que ele confirma e, se ele traz
    /// uma mensagem, guarda e programa a confirmação.
    ///
    /// Confirmações de ids que não estão em voo (repetidas ou inventadas) são ignoradas.
    pub fn receive(&mut self, packet: &ControlPacket) {
        for ack in &packet.acks {
            self.in_flight.retain(|m| m.id != *ack);
        }
        let Some(id) = packet.message_id else { return };
        let distance = id.wrapping_sub(self.next_expected);
        // Um id "atrás" da esperada (a distância dá a volta) é repetição de algo já entregue.
        if distance >= u32::MAX / 2 {
            self.ack_later(id);
            return;
        }
        if distance as usize >= self.config.recv_window {
            // Longe demais: o remetente respeita a janela, então isto é lixo ou atraso enorme.
            return;
        }
        self.ack_later(id);
        if id != self.next_expected {
            self.buffered.entry(id).or_insert_with(|| Received {
                opcode: packet.opcode,
                payload: packet.payload.clone(),
            });
            return;
        }
        self.delivered.push_back(Received {
            opcode: packet.opcode,
            payload: packet.payload.clone(),
        });
        self.next_expected = self.next_expected.wrapping_add(1);
        while let Some(m) = self.buffered.remove(&self.next_expected) {
            self.delivered.push_back(m);
            self.next_expected = self.next_expected.wrapping_add(1);
        }
    }

    fn ack_later(&mut self, id: u32) {
        if !self.pending_acks.contains(&id) {
            self.pending_acks.push(id);
        }
    }

    /// A próxima mensagem entregue, na ordem.
    pub fn pop_received(&mut self) -> Option<Received> {
        self.delivered.pop_front()
    }

    /// O que enviar agora, se houver algo. Chame de novo até devolver `None`.
    pub fn poll_transmit(&mut self, now: Instant) -> Option<Transmit> {
        while self.in_flight.len() < self.config.send_window {
            let Some((id, opcode, payload)) = self.queue.pop_front() else {
                break;
            };
            self.in_flight.push(InFlight {
                id,
                opcode,
                payload,
                sends: 0,
                due: now,
            });
        }
        let due = self
            .in_flight
            .iter()
            .enumerate()
            .filter(|(_, m)| m.sends == 0 || (self.config.retransmit && m.due <= now))
            .min_by_key(|(posicao, m)| (m.due, *posicao))
            .map(|(i, _)| i);
        if let Some(i) = due {
            let take = self.pending_acks.len().min(MAX_ACKS_IN_CONTROL);
            let acks: Vec<u32> = self.pending_acks.drain(..take).collect();
            let sends = self.in_flight[i].sends + 1;
            let delay = self.retransmit_delay(sends);
            let m = &mut self.in_flight[i];
            m.sends = sends;
            m.due = now + delay;
            return Some(Transmit {
                opcode: m.opcode,
                message_id: Some(m.id),
                acks,
                payload: m.payload.clone(),
            });
        }
        if self.pending_acks.is_empty() {
            return None;
        }
        let take = self.pending_acks.len().min(MAX_ACKS_IN_ACK_PACKET);
        let acks: Vec<u32> = self.pending_acks.drain(..take).collect();
        Some(Transmit {
            opcode: Opcode::AckV1,
            message_id: None,
            acks,
            payload: Vec::new(),
        })
    }

    /// Quanto esperar depois do envio número `sends` (1 é o primeiro) antes de reenviar.
    fn retransmit_delay(&self, sends: u32) -> Duration {
        let shift = sends.saturating_sub(1).min(self.config.max_backoff_shift);
        self.config.initial_timeout * (1u32 << shift)
    }

    /// Quando é preciso chamar [`Reliable::poll_transmit`] de novo por causa do relógio. `None`
    /// se não há reenvio programado (nada em voo, ou TCP).
    pub fn next_timeout(&self) -> Option<Instant> {
        if !self.config.retransmit {
            return None;
        }
        self.in_flight
            .iter()
            .filter(|m| m.sends > 0)
            .map(|m| m.due)
            .min()
    }

    /// Se há algo para enviar agora sem esperar o relógio (mensagem nova, janela com espaço ou
    /// confirmação pendente).
    pub fn has_work(&self) -> bool {
        !self.pending_acks.is_empty()
            || self.in_flight.iter().any(|m| m.sends == 0)
            || (!self.queue.is_empty() && self.in_flight.len() < self.config.send_window)
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    fn pacote(t: Transmit) -> ControlPacket {
        t.into_packet(0, SessionId([1; 8]), Some(SessionId([2; 8])))
    }

    fn drenar(r: &mut Reliable, now: Instant) -> Vec<ControlPacket> {
        let mut v = Vec::new();
        while let Some(t) = r.poll_transmit(now) {
            v.push(pacote(t));
        }
        v
    }

    #[test]
    fn mensagem_vai_com_id_zero_e_e_reenviada_ate_o_ack() {
        let t0 = Instant::now();
        let mut a = Reliable::new(ReliableConfig::udp());
        a.send(Opcode::HardResetClientV2, vec![]);
        let p = drenar(&mut a, t0);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].message_id, Some(0));
        assert_eq!(p[0].opcode, Opcode::HardResetClientV2);
        // Antes do tempo, nada.
        assert!(drenar(&mut a, t0 + Duration::from_millis(1999)).is_empty());
        // Aos 2 s, o mesmo id.
        let p = drenar(&mut a, t0 + Duration::from_secs(2));
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].message_id, Some(0));
        // Depois dobra: 4 s depois do segundo envio.
        assert!(drenar(&mut a, t0 + Duration::from_secs(5)).is_empty());
        assert_eq!(drenar(&mut a, t0 + Duration::from_secs(6)).len(), 1);
        assert_eq!(a.next_timeout(), Some(t0 + Duration::from_secs(6 + 8)));
    }

    #[test]
    fn o_tempo_para_de_dobrar_no_teto() {
        let t0 = Instant::now();
        let mut a = Reliable::new(ReliableConfig::udp());
        a.send(Opcode::ControlV1, vec![1]);
        let mut agora = t0;
        let mut esperas = vec![];
        for _ in 0..10 {
            assert_eq!(drenar(&mut a, agora).len(), 1);
            let proximo = a.next_timeout().unwrap();
            esperas.push((proximo - agora).as_secs());
            agora = proximo;
        }
        assert_eq!(esperas, vec![2, 4, 8, 16, 32, 64, 128, 128, 128, 128]);
    }

    #[test]
    fn o_ack_para_o_reenvio_e_ack_repetido_ou_inventado_e_ignorado() {
        let t0 = Instant::now();
        let mut a = Reliable::new(ReliableConfig::udp());
        a.send(Opcode::ControlV1, vec![1]);
        a.send(Opcode::ControlV1, vec![2]);
        assert_eq!(drenar(&mut a, t0).len(), 2);
        let ack = |ids: Vec<u32>| ControlPacket {
            opcode: Opcode::AckV1,
            key_id: 0,
            session_id: SessionId([2; 8]),
            acks: ids,
            remote_session_id: Some(SessionId([1; 8])),
            message_id: None,
            payload: vec![],
        };
        a.receive(&ack(vec![0, 0, 0, 99]));
        a.receive(&ack(vec![0]));
        assert_eq!(a.in_flight(), 1);
        let p = drenar(&mut a, t0 + Duration::from_secs(2));
        assert_eq!(
            p.iter().map(|p| p.message_id).collect::<Vec<_>>(),
            vec![Some(1)]
        );
        a.receive(&ack(vec![1]));
        assert!(a.all_acked());
        assert_eq!(a.next_timeout(), None);
        assert!(drenar(&mut a, t0 + Duration::from_secs(60)).is_empty());
    }

    #[test]
    fn a_janela_de_envio_segura_a_setima_mensagem() {
        let t0 = Instant::now();
        let mut a = Reliable::new(ReliableConfig::udp());
        for i in 0..10u8 {
            a.send(Opcode::ControlV1, vec![i]);
        }
        let p = drenar(&mut a, t0);
        assert_eq!(p.len(), 6);
        assert_eq!(a.queued(), 4);
        // Confirmar a 0 libera espaço para a 6.
        let ack = ControlPacket {
            opcode: Opcode::AckV1,
            key_id: 0,
            session_id: SessionId([2; 8]),
            acks: vec![0],
            remote_session_id: Some(SessionId([1; 8])),
            message_id: None,
            payload: vec![],
        };
        a.receive(&ack);
        let p = drenar(&mut a, t0);
        assert_eq!(
            p.iter().map(|p| p.message_id).collect::<Vec<_>>(),
            vec![Some(6)]
        );
    }

    fn msg(id: u32, dado: u8) -> ControlPacket {
        ControlPacket {
            opcode: Opcode::ControlV1,
            key_id: 0,
            session_id: SessionId([2; 8]),
            acks: vec![],
            remote_session_id: None,
            message_id: Some(id),
            payload: vec![dado],
        }
    }

    #[test]
    fn recebe_fora_de_ordem_e_entrega_em_ordem() {
        let mut b = Reliable::new(ReliableConfig::udp());
        b.receive(&msg(2, 22));
        b.receive(&msg(1, 11));
        assert!(b.pop_received().is_none());
        b.receive(&msg(0, 10));
        let mut v = vec![];
        while let Some(m) = b.pop_received() {
            v.push(m.payload[0]);
        }
        assert_eq!(v, vec![10, 11, 22]);
    }

    #[test]
    fn repetida_nao_e_entregue_de_novo_mas_e_confirmada_de_novo() {
        let t0 = Instant::now();
        let mut b = Reliable::new(ReliableConfig::udp());
        b.receive(&msg(0, 1));
        let t = b.poll_transmit(t0).unwrap();
        assert_eq!((t.opcode, t.acks.clone()), (Opcode::AckV1, vec![0]));
        assert!(b.poll_transmit(t0).is_none());
        b.receive(&msg(0, 1));
        assert_eq!(b.pop_received().map(|m| m.payload), Some(vec![1]));
        assert!(b.pop_received().is_none());
        assert_eq!(b.poll_transmit(t0).unwrap().acks, vec![0]);
        // Repetida na fila de confirmações não duplica o id.
        b.receive(&msg(0, 1));
        b.receive(&msg(0, 1));
        assert_eq!(b.poll_transmit(t0).unwrap().acks, vec![0]);
    }

    #[test]
    fn longe_demais_da_janela_e_descartada_sem_ack() {
        let t0 = Instant::now();
        let mut b = Reliable::new(ReliableConfig::udp());
        b.receive(&msg(12, 1));
        assert!(b.poll_transmit(t0).is_none());
        b.receive(&msg(11, 1));
        assert_eq!(b.poll_transmit(t0).unwrap().acks, vec![11]);
    }

    #[test]
    fn acks_vao_junto_da_mensagem_ate_o_limite_e_o_resto_em_pacote_proprio() {
        let t0 = Instant::now();
        let mut r = Reliable::new(ReliableConfig::udp());
        for i in 0..6 {
            r.receive(&msg(i, 0));
        }
        r.send(Opcode::ControlV1, vec![9]);
        let t = r.poll_transmit(t0).unwrap();
        assert_eq!(t.message_id, Some(0));
        assert_eq!(t.acks, vec![0, 1, 2, 3]);
        let t = r.poll_transmit(t0).unwrap();
        assert_eq!((t.opcode, t.acks), (Opcode::AckV1, vec![4, 5]));
        assert!(r.poll_transmit(t0).is_none());
        // Mais de 8 confirmações pendentes: dois pacotes só de ACK.
        let mut r = Reliable::new(ReliableConfig::udp());
        for i in 0..12 {
            r.receive(&msg(i, 0));
        }
        assert_eq!(r.poll_transmit(t0).unwrap().acks.len(), 8);
        assert_eq!(r.poll_transmit(t0).unwrap().acks.len(), 4);
    }

    #[test]
    fn tcp_nao_reenvia_por_tempo() {
        let t0 = Instant::now();
        let mut a = Reliable::new(ReliableConfig::tcp());
        a.send(Opcode::ControlV1, vec![1]);
        assert_eq!(drenar(&mut a, t0).len(), 1);
        assert_eq!(a.next_timeout(), None);
        assert!(drenar(&mut a, t0 + Duration::from_secs(3600)).is_empty());
        assert_eq!(a.in_flight(), 1);
    }

    #[test]
    fn a_confirmacao_so_leva_a_sessao_remota_quando_ha_acks() {
        let t = Transmit {
            opcode: Opcode::ControlV1,
            message_id: Some(0),
            acks: vec![],
            payload: vec![],
        };
        let p = t.into_packet(1, SessionId([1; 8]), Some(SessionId([2; 8])));
        assert_eq!(p.remote_session_id, None);
    }

    /// Um gerador simples e repetível, para os testes de rede ruim.
    struct Dado(u64);
    impl Dado {
        fn proximo(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn chance(&mut self, por_cento: u64) -> bool {
            self.proximo() % 100 < por_cento
        }
    }

    /// Dois lados ligados por uma rede que perde, duplica e atrasa (reordena) pacotes. O lado A
    /// manda 1 MiB em mensagens de 1000 bytes; o B precisa receber tudo, em ordem.
    fn transferir(semente: u64, perda: u64, duplicacao: u64, atraso: bool) {
        let t0 = Instant::now();
        let mut rng = Dado(semente);
        let mut a = Reliable::new(ReliableConfig::udp());
        let mut b = Reliable::new(ReliableConfig::udp());
        let total = 1_048_576usize / 1000 + 1;
        let mut esperado = Vec::new();
        for i in 0..total {
            let dado: Vec<u8> = (0..1000).map(|j| (i * 31 + j) as u8).collect();
            esperado.push(dado.clone());
            a.send(Opcode::ControlV1, dado);
        }
        // Pacotes no ar: (chega_em, destino_é_b, pacote).
        let mut no_ar: Vec<(Instant, bool, ControlPacket)> = Vec::new();
        let mut recebido: Vec<Vec<u8>> = Vec::new();
        let mut agora = t0;
        let passo = Duration::from_millis(50);
        let latencia = Duration::from_millis(100);
        for _ in 0..2_000_000 {
            if recebido.len() == total && a.all_acked() {
                break;
            }
            // O que cada lado quer enviar.
            for (de_a, lado) in [(true, &mut a), (false, &mut b)] {
                while let Some(t) = lado.poll_transmit(agora) {
                    assert!(lado.in_flight() <= 6, "passou da janela de envio");
                    let p = pacote(t);
                    if rng.chance(perda) {
                        continue;
                    }
                    let copias = if rng.chance(duplicacao) { 2 } else { 1 };
                    for _ in 0..copias {
                        let extra = if atraso {
                            Duration::from_millis(rng.proximo() % 700)
                        } else {
                            Duration::ZERO
                        };
                        no_ar.push((agora + latencia + extra, de_a, p.clone()));
                    }
                }
            }
            // O que chega.
            let (chegando, resto): (Vec<_>, Vec<_>) =
                no_ar.drain(..).partition(|(quando, _, _)| *quando <= agora);
            no_ar = resto;
            for (_, de_a, p) in chegando {
                let destino = if de_a { &mut b } else { &mut a };
                destino.receive(&p);
            }
            while let Some(m) = b.pop_received() {
                recebido.push(m.payload);
            }
            agora += passo;
        }
        assert_eq!(recebido.len(), total, "semente {semente}: faltou mensagem");
        assert_eq!(
            recebido, esperado,
            "semente {semente}: fora de ordem ou corrompido"
        );
    }

    #[test]
    fn um_mebibyte_atravessa_rede_perfeita() {
        transferir(1, 0, 0, false);
    }

    #[test]
    fn um_mebibyte_atravessa_rede_que_perde_duplica_e_reordena() {
        for semente in [3, 7, 42, 1234, 99_999] {
            transferir(semente, 20, 20, true);
        }
    }

    #[test]
    fn um_mebibyte_atravessa_rede_muito_ruim() {
        transferir(2026, 50, 30, true);
    }
}
