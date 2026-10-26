//! Os pacotes do OpenVPN: opcodes, o cabeçalho do canal de controle e o enquadramento do TCP.
//!
//! O primeiro byte é `opcode << 3 | key_id`. O pacote de controle (sem `tls-auth` nem
//! `tls-crypt`) segue o formato da descrição pública do protocolo:
//!
//! ```text
//! op|key_id (1) · session_id local (8)
//! · n_acks (1) · n × packet_id confirmado (4) · session_id remoto (8, só se n_acks > 0)
//! · packet_id da mensagem (4, não existe no P_ACK_V1)
//! · carga
//! ```
//!
//! Os envoltórios (`wrap`) separam o pacote em duas partes: o cabeçalho (`op|key_id` e
//! `session_id`) e o corpo (do `n_acks` em diante). Por isso [`ControlPacket`] sabe escrever e
//! ler as duas partes separadas.

use std::collections::VecDeque;
use std::fmt;

/// O tamanho do identificador de sessão (formato do protocolo).
pub const SESSION_ID_LEN: usize = 8;

/// Quantas confirmações cabem num `P_ACK_V1` (limite do protocolo).
pub const MAX_ACKS_IN_ACK_PACKET: usize = 8;

/// Quantas confirmações cabem num pacote de controle com carga (limite do protocolo).
pub const MAX_ACKS_IN_CONTROL: usize = 4;

/// O maior `key_id` (3 bits, formato do protocolo).
pub const MAX_KEY_ID: u8 = 7;

/// O tamanho do prefixo de tamanho do TCP (formato do protocolo: 2 bytes em big-endian).
pub const TCP_PREFIX_LEN: usize = 2;

/// O que o primeiro byte do pacote diz que ele é.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Opcode {
    /// Pede uma renegociação (`P_CONTROL_SOFT_RESET_V1`, 3).
    ControlSoftResetV1 = 3,
    /// Dados do TLS (`P_CONTROL_V1`, 4).
    ControlV1 = 4,
    /// Só confirmações (`P_ACK_V1`, 5).
    AckV1 = 5,
    /// Dados sem `peer-id` (`P_DATA_V1`, 6).
    DataV1 = 6,
    /// O começo da sessão, do cliente (`P_CONTROL_HARD_RESET_CLIENT_V2`, 7).
    HardResetClientV2 = 7,
    /// O começo da sessão, do servidor (`P_CONTROL_HARD_RESET_SERVER_V2`, 8).
    HardResetServerV2 = 8,
    /// Dados com `peer-id` de 24 bits (`P_DATA_V2`, 9).
    DataV2 = 9,
    /// O começo da sessão com `tls-crypt-v2` (`P_CONTROL_HARD_RESET_CLIENT_V3`, 10).
    HardResetClientV3 = 10,
    /// A negociação antecipada do 2.6 (`P_CONTROL_WKC_V1`, 11). O app não a usa.
    ControlWkcV1 = 11,
}

impl Opcode {
    /// O opcode do número, se for um que o app entende. Os números 1 e 2 (os hard resets
    /// antigos) e os desconhecidos dão `None`.
    pub fn from_u8(n: u8) -> Option<Opcode> {
        Some(match n {
            3 => Opcode::ControlSoftResetV1,
            4 => Opcode::ControlV1,
            5 => Opcode::AckV1,
            6 => Opcode::DataV1,
            7 => Opcode::HardResetClientV2,
            8 => Opcode::HardResetServerV2,
            9 => Opcode::DataV2,
            10 => Opcode::HardResetClientV3,
            11 => Opcode::ControlWkcV1,
            _ => return None,
        })
    }

    /// Se é um pacote do canal de dados.
    pub fn is_data(self) -> bool {
        matches!(self, Opcode::DataV1 | Opcode::DataV2)
    }

    /// Se é um pacote do canal de controle.
    pub fn is_control(self) -> bool {
        !self.is_data()
    }

    /// Se é um dos que começam uma sessão.
    pub fn is_hard_reset(self) -> bool {
        matches!(
            self,
            Opcode::HardResetClientV2 | Opcode::HardResetServerV2 | Opcode::HardResetClientV3
        )
    }

    /// Se o pacote carrega um `packet_id` de mensagem (todos os de controle, menos o ACK).
    pub fn has_message_id(self) -> bool {
        self.is_control() && self != Opcode::AckV1
    }
}

/// O erro de ler um pacote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketError {
    /// O pacote é menor que o cabeçalho pede.
    Short,
    /// O opcode não é um que o app entenda (o número vem junto).
    UnknownOpcode(u8),
    /// O pacote não é do canal de controle.
    NotControl,
    /// Mais confirmações do que cabem num pacote.
    TooManyAcks(usize),
    /// O `key_id` passa de 7.
    BadKeyId(u8),
}

impl fmt::Display for PacketError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Short => f.write_str("pacote curto demais"),
            Self::UnknownOpcode(n) => write!(f, "opcode {n} desconhecido"),
            Self::NotControl => f.write_str("o pacote não é do canal de controle"),
            Self::TooManyAcks(n) => write!(f, "{n} confirmações não cabem num pacote"),
            Self::BadKeyId(n) => write!(f, "key_id {n} fora do limite de 0 a {MAX_KEY_ID}"),
        }
    }
}

impl std::error::Error for PacketError {}

/// O primeiro byte de um pacote.
pub fn first_byte(opcode: Opcode, key_id: u8) -> u8 {
    ((opcode as u8) << 3) | (key_id & MAX_KEY_ID)
}

/// Separa o primeiro byte em opcode (o número, que pode ser desconhecido) e `key_id`.
pub fn split_first_byte(byte: u8) -> (u8, u8) {
    (byte >> 3, byte & MAX_KEY_ID)
}

/// Olha o primeiro byte e diz o opcode e o `key_id`.
pub fn classify(packet: &[u8]) -> Result<(Opcode, u8), PacketError> {
    let first = *packet.first().ok_or(PacketError::Short)?;
    let (raw, key_id) = split_first_byte(first);
    let opcode = Opcode::from_u8(raw).ok_or(PacketError::UnknownOpcode(raw))?;
    Ok((opcode, key_id))
}

/// O identificador de uma sessão (8 bytes sorteados por quem a começa).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SessionId(pub [u8; SESSION_ID_LEN]);

impl SessionId {
    /// Sorteia um identificador.
    pub fn random() -> SessionId {
        let mut bytes = [0u8; SESSION_ID_LEN];
        // Sem fonte de aleatoriedade o sistema está quebrado; os bytes zero valem como último
        // recurso e o servidor recusará a repetição.
        let _ = getrandom::fill(&mut bytes);
        SessionId(bytes)
    }
}

impl fmt::Debug for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SessionId(")?;
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        write!(f, ")")
    }
}

/// Um pacote do canal de controle, em campos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlPacket {
    /// O tipo.
    pub opcode: Opcode,
    /// A geração de chave (0 a 7).
    pub key_id: u8,
    /// A sessão de quem envia.
    pub session_id: SessionId,
    /// Os ids das mensagens do outro lado que este pacote confirma.
    pub acks: Vec<u32>,
    /// A sessão do outro lado, presente só quando há confirmações.
    pub remote_session_id: Option<SessionId>,
    /// O id desta mensagem (ausente no `P_ACK_V1`).
    pub message_id: Option<u32>,
    /// A carga (o que o TLS escreveu).
    pub payload: Vec<u8>,
}

impl ControlPacket {
    /// O cabeçalho: `op|key_id` e a sessão local (9 bytes).
    pub fn header(&self) -> [u8; 1 + SESSION_ID_LEN] {
        let mut out = [0u8; 1 + SESSION_ID_LEN];
        out[0] = first_byte(self.opcode, self.key_id);
        out[1..].copy_from_slice(&self.session_id.0);
        out
    }

    /// O corpo: confirmações, sessão remota, id da mensagem e carga.
    pub fn body(&self) -> Result<Vec<u8>, PacketError> {
        let limite = if self.opcode == Opcode::AckV1 {
            MAX_ACKS_IN_ACK_PACKET
        } else {
            MAX_ACKS_IN_CONTROL
        };
        if self.acks.len() > limite {
            return Err(PacketError::TooManyAcks(self.acks.len()));
        }
        let mut out =
            Vec::with_capacity(1 + 4 * self.acks.len() + SESSION_ID_LEN + 4 + self.payload.len());
        // O tamanho cabe em um byte: o limite é 8.
        out.push(self.acks.len() as u8);
        for ack in &self.acks {
            out.extend_from_slice(&ack.to_be_bytes());
        }
        if !self.acks.is_empty() {
            out.extend_from_slice(&self.remote_session_id.unwrap_or_default().0);
        }
        if let Some(id) = self.message_id {
            out.extend_from_slice(&id.to_be_bytes());
        }
        out.extend_from_slice(&self.payload);
        Ok(out)
    }

    /// O pacote inteiro, sem envoltório.
    pub fn encode(&self) -> Result<Vec<u8>, PacketError> {
        let mut out = self.header().to_vec();
        out.extend_from_slice(&self.body()?);
        Ok(out)
    }

    /// Lê um pacote inteiro, sem envoltório.
    pub fn parse(packet: &[u8]) -> Result<ControlPacket, PacketError> {
        let (opcode, key_id) = classify(packet)?;
        if !opcode.is_control() {
            return Err(PacketError::NotControl);
        }
        let sid = packet
            .get(1..1 + SESSION_ID_LEN)
            .ok_or(PacketError::Short)?;
        let mut session_id = SessionId::default();
        session_id.0.copy_from_slice(sid);
        ControlPacket::parse_body(opcode, key_id, session_id, &packet[1 + SESSION_ID_LEN..])
    }

    /// Lê o corpo (do `n_acks` em diante) de um pacote cujo cabeçalho já foi lido.
    pub fn parse_body(
        opcode: Opcode,
        key_id: u8,
        session_id: SessionId,
        body: &[u8],
    ) -> Result<ControlPacket, PacketError> {
        let (&n, mut rest) = body.split_first().ok_or(PacketError::Short)?;
        let n = usize::from(n);
        let limite = if opcode == Opcode::AckV1 {
            MAX_ACKS_IN_ACK_PACKET
        } else {
            MAX_ACKS_IN_CONTROL
        };
        if n > limite {
            return Err(PacketError::TooManyAcks(n));
        }
        let mut acks = Vec::with_capacity(n);
        for _ in 0..n {
            let (bytes, depois) = rest.split_first_chunk::<4>().ok_or(PacketError::Short)?;
            acks.push(u32::from_be_bytes(*bytes));
            rest = depois;
        }
        let remote_session_id = if n > 0 {
            let (bytes, depois) = rest
                .split_first_chunk::<SESSION_ID_LEN>()
                .ok_or(PacketError::Short)?;
            rest = depois;
            Some(SessionId(*bytes))
        } else {
            None
        };
        let message_id = if opcode.has_message_id() {
            let (bytes, depois) = rest.split_first_chunk::<4>().ok_or(PacketError::Short)?;
            rest = depois;
            Some(u32::from_be_bytes(*bytes))
        } else {
            None
        };
        Ok(ControlPacket {
            opcode,
            key_id,
            session_id,
            acks,
            remote_session_id,
            message_id,
            payload: rest.to_vec(),
        })
    }
}

/// Os dois campos do cabeçalho do `P_DATA_V2`: `key_id` e `peer_id` (24 bits).
pub fn data_v2_header(packet: &[u8]) -> Result<(u8, u32), PacketError> {
    let (opcode, key_id) = classify(packet)?;
    if opcode != Opcode::DataV2 {
        return Err(PacketError::NotControl);
    }
    let peer = packet.get(1..4).ok_or(PacketError::Short)?;
    Ok((key_id, u32::from_be_bytes([0, peer[0], peer[1], peer[2]])))
}

/// O `peer_id` que significa "sem peer-id" (formato do protocolo: todos os 24 bits ligados).
pub const PEER_ID_UNDEFINED: u32 = 0x00FF_FFFF;

/// Põe o prefixo de tamanho do TCP. `None` se o pacote não cabe nos 16 bits do prefixo.
pub fn tcp_frame(packet: &[u8]) -> Option<Vec<u8>> {
    let len = u16::try_from(packet.len()).ok()?;
    let mut out = Vec::with_capacity(TCP_PREFIX_LEN + packet.len());
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(packet);
    Some(out)
}

/// Junta o que chega do TCP e devolve um pacote por vez.
///
/// O TCP entrega bytes, sem respeitar os limites dos pacotes: a leitura pode trazer meio pacote
/// ou vários.
#[derive(Debug, Default)]
pub struct TcpDeframer {
    buffer: VecDeque<u8>,
}

impl TcpDeframer {
    /// Um desenquadrador vazio.
    pub fn new() -> TcpDeframer {
        TcpDeframer::default()
    }

    /// Acrescenta bytes lidos.
    pub fn push(&mut self, bytes: &[u8]) {
        self.buffer.extend(bytes);
    }

    /// O próximo pacote inteiro, se já chegou. Pacotes de tamanho 0 são pulados.
    pub fn next_packet(&mut self) -> Option<Vec<u8>> {
        loop {
            if self.buffer.len() < TCP_PREFIX_LEN {
                return None;
            }
            let len = usize::from(u16::from_be_bytes([self.buffer[0], self.buffer[1]]));
            if self.buffer.len() < TCP_PREFIX_LEN + len {
                return None;
            }
            self.buffer.drain(..TCP_PREFIX_LEN);
            let packet: Vec<u8> = self.buffer.drain(..len).collect();
            if !packet.is_empty() {
                return Some(packet);
            }
        }
    }

    /// Quantos bytes esperam por um pacote inteiro.
    pub fn pending(&self) -> usize {
        self.buffer.len()
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    fn exemplo(opcode: Opcode, acks: usize) -> ControlPacket {
        let tem_acks = acks > 0;
        ControlPacket {
            opcode,
            key_id: 3,
            session_id: SessionId([1, 2, 3, 4, 5, 6, 7, 8]),
            acks: (10..10 + acks as u32).collect(),
            remote_session_id: tem_acks.then_some(SessionId([9; 8])),
            message_id: opcode.has_message_id().then_some(77),
            payload: if opcode == Opcode::AckV1 {
                vec![]
            } else {
                vec![0xAA, 0xBB, 0xCC]
            },
        }
    }

    #[test]
    fn primeiro_byte_junta_opcode_e_key_id() {
        assert_eq!(first_byte(Opcode::ControlV1, 0), 0x20);
        assert_eq!(first_byte(Opcode::HardResetClientV2, 0), 0x38);
        assert_eq!(first_byte(Opcode::DataV2, 5), 0x4D);
        assert_eq!(split_first_byte(0x4D), (9, 5));
    }

    #[test]
    fn numeros_dos_opcodes_batem_com_o_protocolo() {
        for (n, op) in [
            (3, Opcode::ControlSoftResetV1),
            (4, Opcode::ControlV1),
            (5, Opcode::AckV1),
            (6, Opcode::DataV1),
            (7, Opcode::HardResetClientV2),
            (8, Opcode::HardResetServerV2),
            (9, Opcode::DataV2),
            (10, Opcode::HardResetClientV3),
            (11, Opcode::ControlWkcV1),
        ] {
            assert_eq!(Opcode::from_u8(n), Some(op));
            assert_eq!(op as u8, n);
        }
        // 1 e 2 são os hard resets antigos; 0 e 12 em diante não são pacotes de sessão.
        for n in [0, 1, 2, 12, 31] {
            assert_eq!(Opcode::from_u8(n), None, "{n}");
        }
    }

    #[test]
    fn cada_opcode_de_controle_vai_e_volta() {
        for op in [
            Opcode::ControlSoftResetV1,
            Opcode::ControlV1,
            Opcode::AckV1,
            Opcode::HardResetClientV2,
            Opcode::HardResetServerV2,
            Opcode::HardResetClientV3,
            Opcode::ControlWkcV1,
        ] {
            for acks in [0, 1, 4] {
                let p = exemplo(op, acks);
                let bytes = p.encode().unwrap();
                assert_eq!(
                    ControlPacket::parse(&bytes).unwrap(),
                    p,
                    "{op:?} com {acks} acks"
                );
            }
        }
    }

    #[test]
    fn ack_no_limite_do_pacote() {
        let p = exemplo(Opcode::AckV1, MAX_ACKS_IN_ACK_PACKET);
        let bytes = p.encode().unwrap();
        assert_eq!(ControlPacket::parse(&bytes).unwrap().acks.len(), 8);
        let p = exemplo(Opcode::AckV1, MAX_ACKS_IN_ACK_PACKET + 1);
        assert_eq!(p.encode(), Err(PacketError::TooManyAcks(9)));
    }

    #[test]
    fn pacote_com_carga_aceita_ate_quatro_acks() {
        let p = exemplo(Opcode::ControlV1, MAX_ACKS_IN_CONTROL);
        assert!(p.encode().is_ok());
        let p = exemplo(Opcode::ControlV1, MAX_ACKS_IN_CONTROL + 1);
        assert_eq!(p.encode(), Err(PacketError::TooManyAcks(5)));
        // Na leitura também: n_acks = 5 num P_CONTROL_V1 é recusado.
        let mut bytes = exemplo(Opcode::ControlV1, 4).encode().unwrap();
        bytes[9] = 5;
        assert_eq!(
            ControlPacket::parse(&bytes),
            Err(PacketError::TooManyAcks(5))
        );
    }

    #[test]
    fn formato_dos_bytes_do_controle() {
        let p = exemplo(Opcode::ControlV1, 1);
        let bytes = p.encode().unwrap();
        let mut esperado = vec![0x20 | 3];
        esperado.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        esperado.push(1);
        esperado.extend_from_slice(&10u32.to_be_bytes());
        esperado.extend_from_slice(&[9; 8]);
        esperado.extend_from_slice(&77u32.to_be_bytes());
        esperado.extend_from_slice(&[0xAA, 0xBB, 0xCC]);
        assert_eq!(bytes, esperado);
    }

    #[test]
    fn ack_nao_tem_id_de_mensagem() {
        let bytes = exemplo(Opcode::AckV1, 1).encode().unwrap();
        // op, sid, n, ack, sid remoto: 1 + 8 + 1 + 4 + 8.
        assert_eq!(bytes.len(), 22);
    }

    #[test]
    fn pacotes_truncados_dao_erro_em_qualquer_posicao() {
        let bytes = exemplo(Opcode::ControlV1, 2).encode().unwrap();
        let sem_carga = bytes.len() - 3;
        for corte in 0..sem_carga {
            assert!(
                ControlPacket::parse(&bytes[..corte]).is_err(),
                "corte em {corte}"
            );
        }
        assert!(ControlPacket::parse(&bytes[..sem_carga]).is_ok());
    }

    #[test]
    fn opcode_de_dados_nao_e_de_controle() {
        let mut bytes = vec![first_byte(Opcode::DataV2, 0)];
        bytes.extend_from_slice(&[0; 30]);
        assert_eq!(ControlPacket::parse(&bytes), Err(PacketError::NotControl));
        assert_eq!(classify(&[0x07 << 3]), Ok((Opcode::HardResetClientV2, 0)));
    }

    #[test]
    fn opcode_desconhecido_e_pacote_vazio() {
        assert_eq!(classify(&[]), Err(PacketError::Short));
        assert_eq!(classify(&[0x01 << 3]), Err(PacketError::UnknownOpcode(1)));
        assert_eq!(classify(&[0x0C << 3]), Err(PacketError::UnknownOpcode(12)));
    }

    #[test]
    fn cabecalho_do_data_v2() {
        let mut p = vec![first_byte(Opcode::DataV2, 2), 0x01, 0x02, 0x03, 0, 0, 0, 1];
        p.extend_from_slice(&[0; 16]);
        assert_eq!(data_v2_header(&p), Ok((2, 0x0001_0203)));
        assert_eq!(data_v2_header(&p[..3]), Err(PacketError::Short));
    }

    #[test]
    fn tcp_enquadra_e_desenquadra_em_pedacos() {
        let a: Vec<u8> = (0..100).collect();
        let b: Vec<u8> = vec![0x38; 1];
        let c: Vec<u8> = vec![7; 1300];
        let mut fluxo = Vec::new();
        for p in [&a, &b, &c] {
            fluxo.extend_from_slice(&tcp_frame(p).unwrap());
        }
        // De 1 em 1 byte.
        let mut d = TcpDeframer::new();
        let mut saida = Vec::new();
        for byte in &fluxo {
            d.push(std::slice::from_ref(byte));
            while let Some(p) = d.next_packet() {
                saida.push(p);
            }
        }
        assert_eq!(saida, vec![a.clone(), b.clone(), c.clone()]);
        assert_eq!(d.pending(), 0);
        // Tudo de uma vez.
        let mut d = TcpDeframer::new();
        d.push(&fluxo);
        assert_eq!(d.next_packet(), Some(a));
        assert_eq!(d.next_packet(), Some(b));
        assert_eq!(d.next_packet(), Some(c));
        assert_eq!(d.next_packet(), None);
    }

    #[test]
    fn tcp_espera_o_resto_do_pacote() {
        let mut d = TcpDeframer::new();
        d.push(&[0x00, 0x05, 1, 2]);
        assert_eq!(d.next_packet(), None);
        d.push(&[3, 4]);
        assert_eq!(d.next_packet(), None);
        d.push(&[5, 0x00]);
        assert_eq!(d.next_packet(), Some(vec![1, 2, 3, 4, 5]));
        assert_eq!(d.next_packet(), None);
    }

    #[test]
    fn tcp_prefixo_zero_e_pulado_e_pacote_enorme_nao_cabe() {
        let mut d = TcpDeframer::new();
        d.push(&[0, 0, 0, 1, 9]);
        assert_eq!(d.next_packet(), Some(vec![9]));
        assert!(tcp_frame(&vec![0; 65_536]).is_none());
        assert!(tcp_frame(&vec![0; 65_535]).is_some());
    }

    #[test]
    fn sessao_sorteada_nao_repete() {
        assert_ne!(SessionId::random(), SessionId::random());
    }
}
