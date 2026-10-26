//! A janela anti-repetição do transporte (artigo, seção 5.4.6).
//!
//! É a mesma do núcleo do Linux e do `wireguard-go`: um anel de 128 blocos de 64 bits, com
//! janela útil de 8128 contadores (127 blocos). Um contador já visto, ou mais velho que a
//! janela, é recusado.

/// Quantos blocos de 64 bits o anel tem.
const BLOCOS: usize = 128;
/// O tamanho da janela, em contadores (um bloco fica de folga para a limpeza).
pub const TAMANHO_DA_JANELA: u64 = ((BLOCOS - 1) * 64) as u64;

/// O filtro de repetição de uma chave de sessão.
#[derive(Clone)]
pub(crate) struct Replay {
    ultimo: u64,
    anel: [u64; BLOCOS],
}

impl Replay {
    pub(crate) fn new() -> Self {
        Replay {
            ultimo: 0,
            anel: [0; BLOCOS],
        }
    }

    /// Confere o contador e, se serve, o marca como visto. `limite` é o
    /// `REJECT_AFTER_MESSAGES`: contador nesse valor ou acima é recusado.
    ///
    /// Só chame depois de a etiqueta do pacote ter sido autenticada: um pacote falso não pode
    /// avançar a janela.
    pub(crate) fn validar(&mut self, contador: u64, limite: u64) -> bool {
        if contador >= limite {
            return false;
        }
        let indice = contador / 64;
        let mut indice_bloco = (indice % BLOCOS as u64) as usize;
        if contador > self.ultimo {
            // Avança a janela: zera os blocos que ficaram para trás.
            let atual = self.ultimo / 64;
            let diff = (indice - atual).min(BLOCOS as u64);
            for i in 1..=diff {
                let b = ((atual + i) % BLOCOS as u64) as usize;
                self.anel[b] = 0;
            }
            self.ultimo = contador;
        } else if self.ultimo - contador > TAMANHO_DA_JANELA {
            return false;
        }
        indice_bloco %= BLOCOS;
        let bit = 1u64 << (contador % 64);
        let visto = self.anel[indice_bloco] & bit != 0;
        self.anel[indice_bloco] |= bit;
        !visto
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    const LIMITE: u64 = u64::MAX - (1 << 13);

    #[test]
    fn aceita_em_ordem_e_recusa_a_repeticao() {
        let mut r = Replay::new();
        for c in 0..1000 {
            assert!(r.validar(c, LIMITE), "contador {c}");
        }
        for c in 0..1000 {
            assert!(!r.validar(c, LIMITE), "repetição {c}");
        }
    }

    #[test]
    fn aceita_fora_de_ordem_dentro_da_janela() {
        let mut r = Replay::new();
        assert!(r.validar(50_000, LIMITE));
        assert!(r.validar(49_999, LIMITE));
        assert!(r.validar(50_000 - TAMANHO_DA_JANELA, LIMITE));
        assert!(!r.validar(49_999, LIMITE));
    }

    #[test]
    fn recusa_o_que_ficou_para_fora_da_janela() {
        let mut r = Replay::new();
        assert!(r.validar(20_000, LIMITE));
        assert!(!r.validar(20_000 - TAMANHO_DA_JANELA - 1, LIMITE));
        assert!(!r.validar(0, LIMITE));
    }

    #[test]
    fn um_salto_grande_limpa_a_janela() {
        let mut r = Replay::new();
        for c in 0..200 {
            assert!(r.validar(c, LIMITE));
        }
        assert!(r.validar(1_000_000, LIMITE));
        // Tudo o que veio antes ficou fora da janela.
        assert!(!r.validar(199, LIMITE));
        // E o que está dentro da nova janela ainda não foi visto.
        assert!(r.validar(999_999, LIMITE));
        assert!(!r.validar(999_999, LIMITE));
        assert!(!r.validar(1_000_000, LIMITE));
    }

    #[test]
    fn o_anel_da_a_volta_sem_confundir_contadores_distantes() {
        let mut r = Replay::new();
        assert!(r.validar(10, LIMITE));
        // Um contador que cai no mesmo bloco do anel, 128 blocos depois.
        let longe = 10 + (BLOCOS as u64) * 64;
        assert!(r.validar(longe, LIMITE));
        assert!(!r.validar(10, LIMITE));
        assert!(!r.validar(longe, LIMITE));
    }

    #[test]
    fn o_limite_de_mensagens_e_recusado() {
        let mut r = Replay::new();
        assert!(!r.validar(LIMITE, LIMITE));
        assert!(!r.validar(u64::MAX, LIMITE));
        assert!(r.validar(LIMITE - 1, LIMITE));
    }
}
