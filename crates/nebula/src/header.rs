//! O cabeçalho de 16 bytes de todo pacote do Nebula.
//!
//! Portado do Nebula (github.com/slackhq/nebula, licença MIT, `header/header.go`).
//!
//! ```text
//!  0                                                                       31
//! |-----------------------------------------------------------------------|
//! | Versão (4 bits) | Tipo (4 bits) | Subtipo (8 bits) | Reservado (16)   |
//! |-----------------------------------------------------------------------|
//! |                      Índice remoto (32 bits)                          |
//! |-----------------------------------------------------------------------|
//! |                     Contador da mensagem (64 bits)                    |
//! |-----------------------------------------------------------------------|
//! |                             carga...                                  |
//! ```

/// O tamanho do cabeçalho (do protocolo).
pub(crate) const LEN: usize = 16;

/// A versão do cabeçalho (a única que existe).
pub(crate) const VERSION: u8 = 1;

/// O tipo da mensagem (o nibble baixo do primeiro byte).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MessageType {
    /// O aperto de mão Noise.
    Handshake,
    /// Um pacote de dados (ou, com o subtipo 1, um pacote por relay).
    Message,
    /// "Não conheço esse índice": o outro lado precisa refazer o túnel.
    RecvError,
    /// O protocolo do lighthouse (dentro do túnel).
    LightHouse,
    /// Pedido ou resposta de teste (mantém o túnel vivo e confere o caminho).
    Test,
    /// O outro lado está fechando o túnel.
    CloseTunnel,
    /// Controle (reservado; o Nebula não o usa no 1.11).
    Control,
}

impl MessageType {
    fn from_nibble(n: u8) -> Option<MessageType> {
        Some(match n {
            0 => MessageType::Handshake,
            1 => MessageType::Message,
            2 => MessageType::RecvError,
            3 => MessageType::LightHouse,
            4 => MessageType::Test,
            5 => MessageType::CloseTunnel,
            6 => MessageType::Control,
            _ => return None,
        })
    }

    fn nibble(self) -> u8 {
        match self {
            MessageType::Handshake => 0,
            MessageType::Message => 1,
            MessageType::RecvError => 2,
            MessageType::LightHouse => 3,
            MessageType::Test => 4,
            MessageType::CloseTunnel => 5,
            MessageType::Control => 6,
        }
    }
}

/// O subtipo do handshake Noise IX com chave pré-compartilhada vazia (o único que existe no
/// 1.11).
pub(crate) const HANDSHAKE_IX_PSK0: u8 = 0;
/// O subtipo "pedido" do teste.
pub(crate) const TEST_REQUEST: u8 = 0;
/// O subtipo "resposta" do teste.
pub(crate) const TEST_REPLY: u8 = 1;
/// O subtipo "por relay" da mensagem.
#[allow(dead_code)] // os relays ficam para depois (M48, fatia 5 adiada)
pub(crate) const MESSAGE_RELAY: u8 = 1;

/// Um cabeçalho lido.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Header {
    pub(crate) version: u8,
    pub(crate) kind: MessageType,
    pub(crate) subtype: u8,
    pub(crate) remote_index: u32,
    pub(crate) counter: u64,
}

impl Header {
    /// Lê o cabeçalho do começo de `bytes`. `None` se é curto demais ou o tipo não existe.
    pub(crate) fn parse(bytes: &[u8]) -> Option<Header> {
        if bytes.len() < LEN {
            return None;
        }
        Some(Header {
            version: bytes[0] >> 4,
            kind: MessageType::from_nibble(bytes[0] & 0x0f)?,
            subtype: bytes[1],
            remote_index: u32::from_be_bytes(bytes[4..8].try_into().ok()?),
            counter: u64::from_be_bytes(bytes[8..16].try_into().ok()?),
        })
    }
}

/// Escreve um cabeçalho (versão 1, reservado zero).
pub(crate) fn encode(kind: MessageType, subtype: u8, remote_index: u32, counter: u64) -> [u8; LEN] {
    let mut out = [0u8; LEN];
    out[0] = (VERSION << 4) | kind.nibble();
    out[1] = subtype;
    out[4..8].copy_from_slice(&remote_index.to_be_bytes());
    out[8..16].copy_from_slice(&counter.to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escreve_e_le_o_cabecalho() {
        let bytes = encode(MessageType::Message, 0, 0x0102_0304, 0x1112_1314_1516_1718);
        assert_eq!(
            bytes,
            [
                0x11, 0, 0, 0, 1, 2, 3, 4, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18
            ]
        );
        let h = Header::parse(&bytes).unwrap();
        assert_eq!(h.version, 1);
        assert_eq!(h.kind, MessageType::Message);
        assert_eq!(h.remote_index, 0x0102_0304);
        assert_eq!(h.counter, 0x1112_1314_1516_1718);
    }

    #[test]
    fn o_tipo_vai_no_nibble_baixo_e_a_versao_no_alto() {
        for (tipo, n) in [
            (MessageType::Handshake, 0u8),
            (MessageType::Message, 1),
            (MessageType::RecvError, 2),
            (MessageType::LightHouse, 3),
            (MessageType::Test, 4),
            (MessageType::CloseTunnel, 5),
            (MessageType::Control, 6),
        ] {
            let bytes = encode(tipo, 0, 0, 0);
            assert_eq!(bytes[0], 0x10 | n);
            assert_eq!(Header::parse(&bytes).unwrap().kind, tipo);
        }
    }

    #[test]
    fn curto_ou_de_tipo_desconhecido_nao_le() {
        assert!(Header::parse(&[0x11; 15]).is_none());
        let mut bytes = encode(MessageType::Message, 0, 0, 0);
        bytes[0] = 0x1f;
        assert!(Header::parse(&bytes).is_none());
    }
}
