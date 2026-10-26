//! Contagem de cliques: o winit entrega cada clique isolado, e o duplo e o triplo são contados
//! aqui.

use std::time::{Duration, Instant};

use tomni_ui_core::kurbo::Point;

/// Conta cliques seguidos no mesmo lugar: 1, 2 (duplo), 3 (triplo) e de volta a 1.
#[derive(Clone, Copy, Debug, Default)]
pub struct ClickCounter {
    last: Option<(Instant, Point)>,
    count: u8,
}

impl ClickCounter {
    /// Intervalo máximo entre dois cliques da mesma sequência. É o padrão do Windows; o winit
    /// não expõe o valor configurado no sistema.
    pub const INTERVAL: Duration = Duration::from_millis(500);

    /// Distância máxima, em pixels lógicos, entre dois cliques da mesma sequência.
    pub const SLOP: f64 = 4.0;

    /// Contador zerado.
    pub fn new() -> Self {
        Self::default()
    }

    /// Botão pressionado em `point` no instante `now`: devolve a posição dele na sequência.
    pub fn press(&mut self, now: Instant, point: Point) -> u8 {
        let near = self.last.is_some_and(|(at, at_point)| {
            now.saturating_duration_since(at) <= Self::INTERVAL
                && at_point.distance(point) <= Self::SLOP
        });
        self.count = if near && self.count < 3 {
            self.count + 1
        } else {
            1
        };
        self.last = Some((now, point));
        self.count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conta_ate_tres_e_volta_para_um() {
        let t = Instant::now();
        let p = Point::new(10.0, 10.0);
        let ms = Duration::from_millis;
        let mut c = ClickCounter::new();
        let seq: Vec<u8> = (0..5).map(|i| c.press(t + ms(100 * i), p)).collect();
        assert_eq!(seq, [1, 2, 3, 1, 2]);
    }

    #[test]
    fn longe_ou_devagar_recomeca() {
        let t = Instant::now();
        let p = Point::new(10.0, 10.0);
        let mut c = ClickCounter::new();
        assert_eq!(c.press(t, p), 1);
        assert_eq!(c.press(t, Point::new(14.0, 10.0)), 2);
        assert_eq!(c.press(t, Point::new(19.0, 10.0)), 1);
        let depois = t + ClickCounter::INTERVAL + Duration::from_millis(1);
        assert_eq!(c.press(depois, Point::new(19.0, 10.0)), 1);
    }
}
