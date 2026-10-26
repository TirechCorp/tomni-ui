//! A janela anti-repetição dos contadores recebidos.
//!
//! Portada do Nebula (github.com/slackhq/nebula, licença MIT, `bits.go`): um bitmap circular
//! empacotado em palavras de 64 bits.

/// O tamanho da janela, em contadores (o `ReplayWindow` do Nebula: 8192).
pub(crate) const WINDOW: u64 = 8192;

/// Quem já chegou dentro da janela.
#[derive(Clone)]
pub(crate) struct ReplayWindow {
    length: u64,
    mask: u64,
    current: u64,
    bits: Vec<u64>,
    lost: u64,
}

impl Default for ReplayWindow {
    fn default() -> Self {
        Self::new(WINDOW)
    }
}

impl ReplayWindow {
    /// Uma janela de `length` contadores (potência de dois).
    pub(crate) fn new(length: u64) -> ReplayWindow {
        assert!(length.is_power_of_two(), "a janela é potência de dois");
        let words = (length / 64).max(1) as usize;
        let mut bits = vec![0u64; words];
        // Não existe o contador 0: marcado, para não contar como perdido depois.
        bits[0] = 1;
        ReplayWindow {
            length,
            mask: length - 1,
            current: 0,
            bits,
            lost: 0,
        }
    }

    /// Quantos pacotes saíram da janela sem chegar.
    #[allow(dead_code)] // só os testes e as estatísticas leem
    pub(crate) fn lost(&self) -> u64 {
        self.lost
    }

    fn get(&self, i: u64) -> bool {
        let pos = i & self.mask;
        self.bits[(pos >> 6) as usize] & (1u64 << (pos & 63)) != 0
    }

    fn set(&mut self, i: u64) {
        let pos = i & self.mask;
        self.bits[(pos >> 6) as usize] |= 1u64 << (pos & 63);
    }

    /// Zera `count` bits a partir da posição circular `start` e devolve quantos estavam ligados.
    fn clear_range(&mut self, start: u64, count: u64) -> u64 {
        if count >= self.length {
            let was: u64 = self.bits.iter().map(|w| u64::from(w.count_ones())).sum();
            self.bits.fill(0);
            return was;
        }
        let mut was = 0;
        let mut pos = start;
        let mut remaining = count;
        // O pedaço de palavra antes de alinhar.
        let word = (pos >> 6) as usize;
        let bit = pos & 63;
        let take = (64 - bit).min(remaining).min(self.length - pos);
        let mask = if take == 64 {
            u64::MAX
        } else {
            ((1u64 << take) - 1) << bit
        };
        was += u64::from((self.bits[word] & mask).count_ones());
        self.bits[word] &= !mask;
        remaining -= take;
        pos = (pos + take) & self.mask;
        // As palavras inteiras.
        while remaining >= 64 {
            let word = (pos >> 6) as usize;
            was += u64::from(self.bits[word].count_ones());
            self.bits[word] = 0;
            remaining -= 64;
            pos = (pos + 64) & self.mask;
        }
        // O pedaço final.
        if remaining > 0 {
            let word = (pos >> 6) as usize;
            let mask = (1u64 << remaining) - 1;
            was += u64::from((self.bits[word] & mask).count_ones());
            self.bits[word] &= !mask;
        }
        was
    }

    fn strictly_within(&self, i: u64) -> bool {
        // A janela ainda não deslizou: evita o estouro da subtração.
        if i < self.length && self.current < self.length {
            return true;
        }
        i > self.current.wrapping_sub(self.length)
    }

    /// Diz se `i` é novo (dentro da janela e sem repetição, ou à frente dela), sem registrar.
    pub(crate) fn check(&self, i: u64) -> bool {
        if i > self.current {
            return true;
        }
        if self.strictly_within(i) {
            return !self.get(i);
        }
        false
    }

    /// Registra `i`. `false` se é repetido ou velho demais.
    pub(crate) fn update(&mut self, i: u64) -> bool {
        if i == self.current.wrapping_add(1) {
            let pos = i & self.mask;
            let word = (pos >> 6) as usize;
            let bit = 1u64 << (pos & 63);
            if i > self.length && self.bits[word] & bit == 0 {
                self.lost += 1;
            }
            self.bits[word] |= bit;
            self.current = i;
            return true;
        }
        if i > self.current {
            let end = i.min(self.current.saturating_add(self.length));
            let count = end - self.current;
            let start = (self.current + 1) & self.mask;
            let mut lost: u64;
            if self.current >= self.length {
                let was = self.clear_range(start, count);
                lost = count - was;
            } else {
                lost = 0;
                for n in (self.current + 1)..=end {
                    if !self.get(n) && n > self.length {
                        lost += 1;
                    }
                }
                self.clear_range(start, count);
            }
            if i > self.current.saturating_add(self.length) {
                lost += i - self.current - self.length;
            }
            self.lost += lost;
            self.set(i);
            self.current = i;
            return true;
        }
        if self.strictly_within(i) {
            if self.current == i || self.get(i) {
                return false;
            }
            self.set(i);
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aceita_em_ordem_e_recusa_repetido() {
        let mut j = ReplayWindow::new(16);
        for i in 1..=40 {
            assert!(j.check(i));
            assert!(j.update(i), "{i}");
            assert!(!j.check(i));
            assert!(!j.update(i));
        }
        assert_eq!(j.lost(), 0);
    }

    #[test]
    fn aceita_fora_de_ordem_dentro_da_janela() {
        let mut j = ReplayWindow::new(16);
        assert!(j.update(1));
        assert!(j.update(5));
        assert!(j.update(3));
        assert!(!j.update(3));
        assert!(j.update(2));
        assert!(j.update(4));
        assert!(!j.update(5));
    }

    #[test]
    fn recusa_o_que_ficou_para_tras_da_janela() {
        let mut j = ReplayWindow::new(16);
        assert!(j.update(1));
        assert!(j.update(100));
        // 100 - 16 = 84: tudo até 84 saiu da janela.
        assert!(!j.check(84));
        assert!(!j.update(50));
        assert!(j.update(85));
        assert!(j.update(99));
        assert!(!j.update(99));
    }

    #[test]
    fn um_salto_maior_que_a_janela_limpa_tudo() {
        let mut j = ReplayWindow::new(64);
        for i in 1..=10 {
            assert!(j.update(i));
        }
        assert!(j.update(1_000_000));
        assert!(!j.update(1_000_000));
        assert!(!j.update(10));
        assert!(j.update(999_990));
    }

    #[test]
    fn o_zero_nunca_vale() {
        let mut j = ReplayWindow::new(16);
        assert!(!j.check(0));
        assert!(!j.update(0));
    }

    #[test]
    fn conta_os_perdidos() {
        let mut j = ReplayWindow::new(16);
        for i in 1..=20 {
            if i != 18 {
                assert!(j.update(i));
            }
        }
        // 18 ainda cabe na janela e pode chegar; depois de deslizar vira perdido.
        assert!(j.update(50));
        assert!(j.lost() > 0);
    }

    #[test]
    fn a_janela_de_8192_funciona_como_a_do_nebula() {
        let mut j = ReplayWindow::default();
        assert!(j.update(1));
        assert!(j.update(10_000));
        assert!(j.update(2_000 + 1));
        assert!(!j.update(1_000));
        assert!(!j.update(10_000));
        assert!(j.update(9_999));
    }
}
