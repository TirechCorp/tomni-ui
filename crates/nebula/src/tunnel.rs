//! O estado criptográfico de um túnel: as duas cifras, o contador de envio e a janela
//! anti-repetição de recepção.
//!
//! Portado do Nebula (github.com/slackhq/nebula, licença MIT, `connection_state.go`,
//! `outside.go` e `inside.go`). O cabeçalho de 16 bytes é o dado associado do AEAD: adulterar
//! qualquer campo dele invalida o pacote.

use crate::header::{self, MessageType};
use crate::noise::{DataCipher, TAG_LEN};
use crate::replay::ReplayWindow;

/// Depois deste contador o túnel recusa enviar (o nonce não pode repetir). Vem do protocolo:
/// `RejectAfterMessages = u64::MAX - 2^40`, o mesmo do WireGuard.
pub(crate) const REJECT_AFTER_MESSAGES: u64 = u64::MAX - (1 << 40);

/// Com este contador o túnel pede um aperto de mão novo (`RehandshakeAfterMessages = 2^34`).
pub(crate) const REHANDSHAKE_AFTER_MESSAGES: u64 = 1 << 34;

/// Por que um pacote recebido não vale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DecryptError {
    /// O contador já chegou ou é velho demais para a janela.
    Replay,
    /// A etiqueta não confere: o pacote foi adulterado ou é de outra chave.
    Authentication,
}

/// O estado de transporte de um túnel.
pub(crate) struct Tunnel {
    send: DataCipher,
    recv: DataCipher,
    /// O último contador usado no envio.
    counter: u64,
    window: ReplayWindow,
}

impl Tunnel {
    /// O túnel que sai de um aperto de mão: o contador de envio segue o do aperto de mão e a
    /// janela já conta os contadores que ele usou (1 e 2).
    pub(crate) fn new(send: DataCipher, recv: DataCipher, message_index: u64) -> Tunnel {
        let mut window = ReplayWindow::default();
        for contador in 1..=message_index {
            window.update(contador);
        }
        Tunnel {
            send,
            recv,
            counter: message_index,
            window,
        }
    }

    /// O último contador de envio.
    #[cfg(test)]
    pub(crate) fn counter(&self) -> u64 {
        self.counter
    }

    /// Já passou do limite em que se pede um aperto de mão novo.
    pub(crate) fn needs_rehandshake(&self) -> bool {
        self.counter >= REHANDSHAKE_AFTER_MESSAGES
    }

    /// O contador esgotou: o túnel não envia mais.
    pub(crate) fn exhausted(&self) -> bool {
        self.counter >= REJECT_AFTER_MESSAGES
    }

    /// Cifra `payload` num pacote completo (cabeçalho, carga cifrada e etiqueta). `None` se o
    /// contador esgotou.
    pub(crate) fn encrypt(
        &mut self,
        kind: MessageType,
        subtype: u8,
        remote_index: u32,
        payload: &[u8],
    ) -> Option<Vec<u8>> {
        if self.exhausted() {
            return None;
        }
        self.counter += 1;
        let cabecalho = header::encode(kind, subtype, remote_index, self.counter);
        let cifrado = self.send.seal(self.counter, &cabecalho, payload)?;
        let mut pacote = Vec::with_capacity(header::LEN + cifrado.len());
        pacote.extend_from_slice(&cabecalho);
        pacote.extend_from_slice(&cifrado);
        Some(pacote)
    }

    /// Decifra um pacote recebido (com o cabeçalho já lido). Confere a janela primeiro, depois
    /// a etiqueta, e só então registra o contador: um pacote forjado não mexe na janela.
    pub(crate) fn decrypt(
        &mut self,
        pacote: &[u8],
        contador: u64,
    ) -> Result<Vec<u8>, DecryptError> {
        if pacote.len() < header::LEN + TAG_LEN {
            return Err(DecryptError::Authentication);
        }
        if !self.window.check(contador) {
            return Err(DecryptError::Replay);
        }
        let (cabecalho, cifrado) = pacote.split_at(header::LEN);
        let aberto = self
            .recv
            .open(contador, cabecalho, cifrado)
            .ok_or(DecryptError::Authentication)?;
        if !self.window.update(contador) {
            return Err(DecryptError::Replay);
        }
        Ok(aberto)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Cipher;
    use crate::header::Header;

    fn par(cifra: Cipher) -> (Tunnel, Tunnel) {
        let chave_a = [1u8; 32];
        let chave_b = [2u8; 32];
        (
            Tunnel::new(
                DataCipher::new(cifra, &chave_a),
                DataCipher::new(cifra, &chave_b),
                2,
            ),
            Tunnel::new(
                DataCipher::new(cifra, &chave_b),
                DataCipher::new(cifra, &chave_a),
                2,
            ),
        )
    }

    fn receber(t: &mut Tunnel, pacote: &[u8]) -> Result<Vec<u8>, DecryptError> {
        let h = Header::parse(pacote).unwrap();
        t.decrypt(pacote, h.counter)
    }

    #[test]
    fn o_primeiro_pacote_de_dados_leva_o_contador_3() {
        for cifra in [Cipher::Aes, Cipher::ChaChaPoly] {
            let (mut a, mut b) = par(cifra);
            let pacote = a.encrypt(MessageType::Message, 0, 0x1234, b"ola").unwrap();
            let h = Header::parse(&pacote).unwrap();
            assert_eq!(
                (h.kind, h.remote_index, h.counter),
                (MessageType::Message, 0x1234, 3)
            );
            assert_eq!(pacote.len(), header::LEN + 3 + TAG_LEN);
            assert_eq!(receber(&mut b, &pacote).unwrap(), b"ola");
            assert_eq!(a.counter(), 3);
        }
    }

    #[test]
    fn repetir_ou_adulterar_o_pacote_e_recusado() {
        let (mut a, mut b) = par(Cipher::Aes);
        let pacote = a.encrypt(MessageType::Message, 0, 9, b"dados").unwrap();
        assert!(receber(&mut b, &pacote).is_ok());
        assert_eq!(receber(&mut b, &pacote), Err(DecryptError::Replay));

        // Qualquer byte do cabeçalho (o dado associado) ou da carga que mude invalida.
        let pacote = a.encrypt(MessageType::Message, 0, 9, b"dados").unwrap();
        for posicao in [1, 4, 7, 16, pacote.len() - 1] {
            let mut ruim = pacote.clone();
            ruim[posicao] ^= 1;
            assert_eq!(
                b.decrypt(&ruim, Header::parse(&pacote).unwrap().counter),
                Err(DecryptError::Authentication),
                "byte {posicao}"
            );
        }
        // O pacote forjado não gastou o contador: o verdadeiro ainda passa.
        assert!(receber(&mut b, &pacote).is_ok());
    }

    #[test]
    fn a_ordem_trocada_dentro_da_janela_passa() {
        let (mut a, mut b) = par(Cipher::ChaChaPoly);
        let pacotes: Vec<_> = (0..5)
            .map(|i| a.encrypt(MessageType::Message, 0, 1, &[i]).unwrap())
            .collect();
        for indice in [3, 0, 4, 2, 1] {
            assert_eq!(receber(&mut b, &pacotes[indice]).unwrap(), [indice as u8]);
        }
        for pacote in &pacotes {
            assert_eq!(receber(&mut b, pacote), Err(DecryptError::Replay));
        }
    }

    #[test]
    fn os_contadores_do_aperto_de_mao_nao_valem_como_dados() {
        let (_, mut b) = par(Cipher::Aes);
        // O contador 1 e o 2 já foram usados pelo aperto de mão.
        assert_eq!(b.decrypt(&[0u8; 40], 1), Err(DecryptError::Replay));
        assert_eq!(b.decrypt(&[0u8; 40], 2), Err(DecryptError::Replay));
    }

    #[test]
    fn o_contador_esgotado_nao_envia_e_pede_novo_aperto_antes() {
        let (mut a, _) = par(Cipher::Aes);
        a.counter = REHANDSHAKE_AFTER_MESSAGES;
        assert!(a.needs_rehandshake() && !a.exhausted());
        a.counter = REJECT_AFTER_MESSAGES;
        assert!(a.exhausted());
        assert!(a.encrypt(MessageType::Message, 0, 1, b"x").is_none());
    }

    #[test]
    fn pacote_curto_demais_nao_e_lido() {
        let (_, mut b) = par(Cipher::Aes);
        assert_eq!(b.decrypt(&[0u8; 20], 5), Err(DecryptError::Authentication));
    }
}
