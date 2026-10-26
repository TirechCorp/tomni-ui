//! Pisca do cursor do campo com foco.

use std::time::{Duration, Instant};

/// Fase do cursor: aceso e apagado se alternam a cada [`Blink::INTERVAL`] enquanto há um campo
/// com foco, e o cursor volta aceso a cada tecla ou clique.
///
/// Com a janela sem o foco do sistema ([`Blink::set_active`]), o cursor fica apagado e não
/// pisca, como no WinUI 3: a janela parada não pede quadro nenhum.
#[derive(Clone, Copy, Debug)]
pub struct Blink {
    visible: bool,
    next: Option<Instant>,
    /// Há um campo com foco ([`Blink::restart`] até [`Blink::stop`]).
    field: bool,
    /// A janela tem o foco do sistema.
    active: bool,
}

impl Blink {
    /// Meio período do pisca: o padrão do Windows (`GetCaretBlinkTime`).
    pub const INTERVAL: Duration = Duration::from_millis(530);

    /// Parado, com o cursor aceso, numa janela ativa.
    pub fn new() -> Self {
        Self {
            visible: true,
            next: None,
            field: false,
            active: true,
        }
    }

    /// Há um campo com foco: cursor aceso, recomeçando a contar a partir de `now`. Na janela
    /// inativa, o cursor fica apagado e parado até ela voltar.
    pub fn restart(&mut self, now: Instant) {
        self.field = true;
        self.visible = self.active;
        self.next = self.active.then_some(now + Self::INTERVAL);
    }

    /// Para de piscar, com o cursor aceso: não há mais campo com foco.
    pub fn stop(&mut self) {
        self.field = false;
        self.visible = true;
        self.next = None;
    }

    /// A janela ganhou (`true`) ou perdeu o foco do sistema. Com um campo com foco, perder
    /// apaga o cursor e para o pisca; ganhar acende e recomeça a contar de `now`. Retorna se a
    /// fase mudou.
    pub fn set_active(&mut self, active: bool, now: Instant) -> bool {
        let before = self.visible;
        self.active = active;
        if self.field {
            self.restart(now);
        }
        self.visible != before
    }

    /// Se há um campo com foco, piscando ou, na janela inativa, apagado.
    pub fn running(&self) -> bool {
        self.field
    }

    /// Se o cursor está aceso.
    pub fn visible(&self) -> bool {
        self.visible
    }

    /// Próxima troca de fase, se está piscando.
    pub fn deadline(&self) -> Option<Instant> {
        self.next
    }

    /// Avança até `now`. Retorna se a fase mudou.
    pub fn tick(&mut self, now: Instant) -> bool {
        let before = self.visible;
        while let Some(next) = self.next.filter(|next| *next <= now) {
            self.visible = !self.visible;
            self.next = Some(next + Self::INTERVAL);
        }
        self.visible != before
    }
}

impl Default for Blink {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alterna_a_cada_intervalo() {
        let t = Instant::now();
        let i = Blink::INTERVAL;
        let mut b = Blink::new();
        assert!(!b.tick(t + i * 10), "parado não pisca");
        b.restart(t);
        assert!(!b.tick(t + i / 2));
        assert!(b.tick(t + i));
        assert!(!b.visible());
        assert!(b.tick(t + i * 2));
        assert!(b.visible());
        assert_eq!(b.deadline(), Some(t + i * 3));
    }

    #[test]
    fn atraso_par_mantem_a_fase_e_recomecar_acende() {
        let t = Instant::now();
        let i = Blink::INTERVAL;
        let mut b = Blink::new();
        b.restart(t);
        assert!(!b.tick(t + i * 2), "duas trocas se anulam");
        assert_eq!(b.deadline(), Some(t + i * 3));
        b.tick(t + i * 3);
        assert!(!b.visible());
        b.restart(t + i * 3);
        assert!(b.visible());
        b.stop();
        assert!(!b.running() && b.visible());
    }

    #[test]
    fn janela_inativa_apaga_o_cursor_e_para_de_piscar() {
        let t = Instant::now();
        let i = Blink::INTERVAL;
        let mut b = Blink::new();
        b.restart(t);
        assert!(
            b.set_active(false, t + i / 2),
            "perder o foco apaga o cursor"
        );
        assert!(!b.visible());
        assert_eq!(b.deadline(), None, "a janela inativa não agenda troca");
        assert!(!b.tick(t + i * 10));
        b.restart(t + i * 10);
        assert!(
            !b.visible() && b.deadline().is_none(),
            "um clique não acende"
        );
        assert!(b.running(), "o campo continua com foco");
        assert!(b.set_active(true, t + i * 11), "voltar acende");
        assert!(b.visible());
        assert_eq!(b.deadline(), Some(t + i * 12));
        assert!(b.set_active(false, t + i * 11));
        b.stop();
        assert!(b.visible() && b.deadline().is_none());
        assert!(!b.set_active(true, t + i * 12), "sem campo, nada muda");
        assert_eq!(b.deadline(), None);
    }
}
