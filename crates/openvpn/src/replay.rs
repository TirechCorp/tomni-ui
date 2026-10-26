//! A janela de replay: descarta o pacote repetido, o muito atrasado e o de uma sequência velha.
//!
//! O contador de pacotes do OpenVPN (o "packet-id") começa em 1 e cresce de um em um. No canal de
//! controle protegido (`tls-auth`, `tls-crypt`) ele vem com um tempo de 4 bytes, que é o instante
//! em que o remetente começou a sequência e não muda enquanto ela dura; no canal de dados em UDP
//! só há o contador. A janela guarda o maior id visto e quais dos `n` ids anteriores já chegaram
//! (o `replay-window n` do `.ovpn`).
//!
//! Diferença para o programa de referência: um tempo mais antigo que o atual é sempre recusado, sem
//! a tolerância de segundos do `replay-window n t`. É mais estrito e basta, porque o tempo só muda
//! quando o outro lado recomeça a sequência.

use std::fmt;

/// Por que o pacote foi recusado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayError {
    /// O id 0 não existe: a sequência começa em 1.
    Zero,
    /// Esse id já chegou.
    Duplicate,
    /// O id está atrás da janela.
    TooOld,
    /// O tempo é mais antigo que o da sequência atual.
    OldTime,
}

impl fmt::Display for ReplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Zero => "o packet-id 0 não é válido",
            Self::Duplicate => "pacote repetido (replay)",
            Self::TooOld => "pacote atrasado demais para a janela de replay",
            Self::OldTime => "pacote de uma sequência mais antiga (tempo velho)",
        })
    }
}

impl std::error::Error for ReplayError {}

/// A janela de replay de uma direção.
#[derive(Debug, Clone)]
pub struct ReplayWindow {
    /// Quantos ids a janela cobre (o `n` do `replay-window n`).
    size: u32,
    /// O tempo da sequência atual (0 quando o formato não tem tempo).
    time: u32,
    /// O maior id aceito.
    highest: u32,
    /// `seen[id % size]` diz se o id já chegou, para os `size` ids até `highest`.
    seen: Vec<bool>,
}

impl ReplayWindow {
    /// Uma janela de `size` ids (no mínimo 1).
    pub fn new(size: u32) -> ReplayWindow {
        let size = size.max(1);
        ReplayWindow {
            size,
            time: 0,
            highest: 0,
            seen: vec![false; size as usize],
        }
    }

    /// O tamanho da janela, em ids.
    pub fn size(&self) -> u32 {
        self.size
    }

    /// Só confere, sem registrar. Use [`ReplayWindow::accept`] depois que o pacote for autenticado:
    /// registrar antes deixaria um pacote forjado queimar um id verdadeiro.
    pub fn check(&self, id: u32, time: u32) -> Result<(), ReplayError> {
        if id == 0 {
            return Err(ReplayError::Zero);
        }
        if time < self.time {
            return Err(ReplayError::OldTime);
        }
        if time > self.time {
            // Sequência nova: tudo vale de novo.
            return Ok(());
        }
        if id > self.highest {
            return Ok(());
        }
        if self.highest - id >= self.size {
            return Err(ReplayError::TooOld);
        }
        if self.seen[(id % self.size) as usize] {
            return Err(ReplayError::Duplicate);
        }
        Ok(())
    }

    /// Registra o id como recebido. Chame só depois de `check` e da autenticação do pacote.
    pub fn accept(&mut self, id: u32, time: u32) {
        if time > self.time {
            self.time = time;
            self.highest = 0;
            self.seen.iter_mut().for_each(|s| *s = false);
        }
        if id > self.highest {
            let avanco = id - self.highest;
            if avanco >= self.size {
                self.seen.iter_mut().for_each(|s| *s = false);
            } else {
                for passo in 1..=avanco {
                    let alvo = self.highest + passo;
                    self.seen[(alvo % self.size) as usize] = false;
                }
            }
            self.highest = id;
        }
        self.seen[(id % self.size) as usize] = true;
    }

    /// `check` e `accept` de uma vez, para quem já autenticou o pacote.
    pub fn check_and_accept(&mut self, id: u32, time: u32) -> Result<(), ReplayError> {
        self.check(id, time)?;
        self.accept(id, time);
        Ok(())
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn ids_em_ordem_passam_e_repetidos_nao() {
        let mut w = ReplayWindow::new(64);
        for id in 1..=200 {
            assert_eq!(w.check_and_accept(id, 0), Ok(()), "id {id}");
        }
        assert_eq!(w.check_and_accept(200, 0), Err(ReplayError::Duplicate));
        assert_eq!(w.check_and_accept(150, 0), Err(ReplayError::Duplicate));
    }

    #[test]
    fn id_zero_e_recusado() {
        assert_eq!(ReplayWindow::new(8).check(0, 0), Err(ReplayError::Zero));
    }

    #[test]
    fn atrasado_dentro_da_janela_passa_uma_vez() {
        let mut w = ReplayWindow::new(8);
        w.check_and_accept(20, 0).unwrap();
        assert_eq!(w.check_and_accept(15, 0), Ok(()));
        assert_eq!(w.check_and_accept(15, 0), Err(ReplayError::Duplicate));
        // Fora da janela de 8: 20 - 12 = 8 >= 8.
        assert_eq!(w.check_and_accept(12, 0), Err(ReplayError::TooOld));
        assert_eq!(w.check_and_accept(13, 0), Ok(()));
    }

    #[test]
    fn salto_grande_limpa_a_janela() {
        let mut w = ReplayWindow::new(8);
        w.check_and_accept(5, 0).unwrap();
        w.check_and_accept(1000, 0).unwrap();
        assert_eq!(w.check_and_accept(5, 0), Err(ReplayError::TooOld));
        assert_eq!(w.check_and_accept(999, 0), Ok(()));
    }

    #[test]
    fn tempo_novo_recomeca_a_sequencia_e_tempo_velho_e_recusado() {
        let mut w = ReplayWindow::new(8);
        w.check_and_accept(10, 100).unwrap();
        assert_eq!(w.check_and_accept(1, 99), Err(ReplayError::OldTime));
        // Um tempo novo aceita ids baixos de novo.
        assert_eq!(w.check_and_accept(1, 101), Ok(()));
        assert_eq!(w.check_and_accept(10, 100), Err(ReplayError::OldTime));
        assert_eq!(w.check_and_accept(1, 101), Err(ReplayError::Duplicate));
    }

    #[test]
    fn check_sozinho_nao_registra() {
        let mut w = ReplayWindow::new(8);
        assert_eq!(w.check(3, 0), Ok(()));
        assert_eq!(w.check(3, 0), Ok(()));
        w.accept(3, 0);
        assert_eq!(w.check(3, 0), Err(ReplayError::Duplicate));
    }

    #[test]
    fn janela_de_um_so_aceita_o_proximo() {
        let mut w = ReplayWindow::new(1);
        assert_eq!(w.check_and_accept(1, 0), Ok(()));
        assert_eq!(w.check_and_accept(1, 0), Err(ReplayError::Duplicate));
        assert_eq!(w.check_and_accept(3, 0), Ok(()));
        assert_eq!(w.check_and_accept(2, 0), Err(ReplayError::TooOld));
    }
}
