//! Onde e com que tamanho a janela abre (M65): o pedido, se cabe no monitor; senão, o maior que
//! cabe, centralizado nele.
//!
//! O `winit` 0.30 não informa a área útil (o monitor sem a barra de tarefas ou o painel), só o
//! tamanho do monitor. Por isso a conta desconta uma faixa fixa de [`RESERVED_HEIGHT`] pixels
//! lógicos e uma margem de [`MARGIN`] de cada lado. Tudo aqui é em pixels lógicos e puro, sem
//! tocar no sistema, para ser testado com números.

/// Altura, em pixels lógicos, reservada para a barra de tarefas do Windows ou o painel do Linux.
pub(crate) const RESERVED_HEIGHT: f64 = 48.0;

/// Fração do monitor deixada livre de cada lado (4%).
pub(crate) const MARGIN: f64 = 0.04;

/// Largura mínima, em pixels lógicos, a que uma janela é reduzida.
pub(crate) const MIN_WIDTH: f64 = 640.0;

/// Altura mínima, em pixels lógicos, a que uma janela é reduzida.
pub(crate) const MIN_HEIGHT: f64 = 400.0;

/// O retângulo de um monitor, em pixels lógicos.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MonitorArea {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl MonitorArea {
    /// Se o ponto está dentro do monitor.
    pub(crate) fn contains(&self, point: (f64, f64)) -> bool {
        point.0 >= self.x
            && point.0 < self.x + self.width
            && point.1 >= self.y
            && point.1 < self.y + self.height
    }

    /// A área de uso: o monitor sem a faixa da barra de tarefas e sem a margem. Devolve o canto
    /// de cima e à esquerda e o tamanho.
    fn usable(&self) -> ((f64, f64), (f64, f64)) {
        let free_height = (self.height - RESERVED_HEIGHT).max(0.0);
        let origin = (self.x + self.width * MARGIN, self.y + free_height * MARGIN);
        let size = (
            self.width * (1.0 - 2.0 * MARGIN),
            free_height * (1.0 - 2.0 * MARGIN),
        );
        (origin, size)
    }
}

/// O tamanho e a posição em que a janela abre: o pedido, se cabe no monitor; senão, o maior que
/// cabe, centralizado nele.
///
/// - `requested`: o tamanho pedido, lógico.
/// - `position`: a posição pedida pelo chamador (janela secundária em cascata), lógica.
/// - `monitor`: o monitor onde a janela vai abrir. Sem ele, nada muda.
///
/// Regras:
/// - o pedido cabe na área de uso e não veio posição: fica como está (o sistema escolhe onde);
/// - o pedido cabe e veio posição: a posição é ajustada para a janela inteira ficar no monitor;
/// - o pedido não cabe: cada lado que passa é reduzido à área de uso, nunca abaixo de 640×400
///   (ou do monitor inteiro, se ele for menor), e a janela vai para o centro da área de uso.
pub(crate) fn fit_to_monitor(
    requested: (f64, f64),
    position: Option<(f64, f64)>,
    monitor: Option<MonitorArea>,
) -> ((f64, f64), Option<(f64, f64)>) {
    let Some(monitor) = monitor else {
        return (requested, position);
    };
    let (origin, usable) = monitor.usable();
    let floor = (MIN_WIDTH.min(monitor.width), MIN_HEIGHT.min(monitor.height));
    let fits = requested.0 <= usable.0 && requested.1 <= usable.1;
    if fits {
        let Some((x, y)) = position else {
            return (requested, None);
        };
        // A janela inteira dentro do monitor (menos a faixa da barra de tarefas).
        let max_x = (monitor.x + monitor.width - requested.0).max(monitor.x);
        let max_y = (monitor.y + monitor.height - RESERVED_HEIGHT - requested.1).max(monitor.y);
        return (
            requested,
            Some((x.clamp(monitor.x, max_x), y.clamp(monitor.y, max_y))),
        );
    }
    let size = (
        requested.0.min(usable.0).max(floor.0),
        requested.1.min(usable.1).max(floor.1),
    );
    let centered = (
        (origin.0 + (usable.0 - size.0) / 2.0).max(monitor.x),
        (origin.1 + (usable.1 - size.1) / 2.0).max(monitor.y),
    );
    (size, Some(centered))
}

#[cfg(test)]
mod testes {
    use super::*;

    fn monitor(x: f64, y: f64, width: f64, height: f64) -> Option<MonitorArea> {
        Some(MonitorArea {
            x,
            y,
            width,
            height,
        })
    }

    fn perto(a: f64, b: f64) -> bool {
        (a - b).abs() < 1.0
    }

    #[test]
    fn pedido_maior_que_o_monitor_de_1280_por_800_abre_reduzido_e_centralizado() {
        let (tamanho, posicao) =
            fit_to_monitor((1280.0, 720.0), None, monitor(0.0, 0.0, 1280.0, 800.0));
        // 1177,6 × 691,84: o plano arredonda para 1178×691.
        assert!(perto(tamanho.0, 1178.0), "{tamanho:?}");
        assert!(perto(tamanho.1, 691.0), "{tamanho:?}");
        let (x, y) = posicao.expect("centralizada");
        assert!(perto(x, 51.2) && perto(y, 30.0), "{x} {y}");
        // Centralizada na área de uso: as folgas dos dois lados são iguais.
        assert!(perto(x, 1280.0 - tamanho.0 - x));
    }

    #[test]
    fn pedido_que_cabe_num_1920_por_1080_fica_como_esta_sem_posicao() {
        let r = fit_to_monitor((1280.0, 720.0), None, monitor(0.0, 0.0, 1920.0, 1080.0));
        assert_eq!(r, ((1280.0, 720.0), None));
    }

    #[test]
    fn tela_de_1920_por_1080_a_150_por_cento_vira_1280_por_720_logicos_e_e_reduzida() {
        // 1920×1080 físicos a 150% = 1280×720 lógicos.
        let (tamanho, posicao) =
            fit_to_monitor((1280.0, 720.0), None, monitor(0.0, 0.0, 1280.0, 720.0));
        assert!(tamanho.0 < 1280.0 && tamanho.1 < 720.0, "{tamanho:?}");
        assert!(posicao.is_some());
    }

    #[test]
    fn posicao_em_cascata_que_passa_da_borda_volta_para_dentro() {
        let m = monitor(0.0, 0.0, 1920.0, 1080.0);
        let (tamanho, posicao) = fit_to_monitor((1280.0, 720.0), Some((1000.0, 700.0)), m);
        assert_eq!(tamanho, (1280.0, 720.0));
        let (x, y) = posicao.unwrap();
        assert!(x + 1280.0 <= 1920.0, "{x}");
        assert!(y + 720.0 <= 1080.0 - RESERVED_HEIGHT, "{y}");
        // Uma posição que já cabe não muda.
        let (_, intacta) = fit_to_monitor((800.0, 600.0), Some((100.0, 50.0)), m);
        assert_eq!(intacta, Some((100.0, 50.0)));
    }

    #[test]
    fn posicao_em_um_segundo_monitor_fica_nele() {
        let m = monitor(1920.0, 0.0, 1920.0, 1080.0);
        let (_, posicao) = fit_to_monitor((1280.0, 720.0), Some((3000.0, 600.0)), m);
        let (x, _) = posicao.unwrap();
        assert!((1920.0..=1920.0 + 640.0).contains(&x), "{x}");
    }

    #[test]
    fn sem_monitor_nada_muda() {
        let r = fit_to_monitor((1280.0, 720.0), Some((10.0, 20.0)), None);
        assert_eq!(r, ((1280.0, 720.0), Some((10.0, 20.0))));
        assert_eq!(
            fit_to_monitor((1280.0, 720.0), None, None),
            ((1280.0, 720.0), None)
        );
    }

    #[test]
    fn monitor_de_800_por_600_reduz_a_janela_acima_do_minimo() {
        let (tamanho, _) = fit_to_monitor((1280.0, 720.0), None, monitor(0.0, 0.0, 800.0, 600.0));
        assert!(perto(tamanho.0, 736.0), "{tamanho:?}");
        assert!(
            tamanho.0 >= MIN_WIDTH && tamanho.1 >= MIN_HEIGHT,
            "{tamanho:?}"
        );
    }

    #[test]
    fn monitor_pequeno_respeita_o_minimo_ou_o_monitor_inteiro() {
        // A área de uso (588×~397) ficaria abaixo de 640×400: vale o mínimo.
        let (tamanho, _) = fit_to_monitor((1280.0, 720.0), None, monitor(0.0, 0.0, 640.0, 480.0));
        assert_eq!(tamanho, (640.0, 400.0));
        // Menor que o mínimo: o monitor inteiro.
        let (tamanho, _) = fit_to_monitor((1280.0, 720.0), None, monitor(0.0, 0.0, 600.0, 360.0));
        assert_eq!(tamanho, (600.0, 360.0));
    }

    #[test]
    fn contem_o_ponto_so_dentro_do_retangulo() {
        let m = MonitorArea {
            x: 100.0,
            y: 0.0,
            width: 200.0,
            height: 100.0,
        };
        assert!(m.contains((100.0, 0.0)));
        assert!(!m.contains((300.0, 10.0)));
        assert!(!m.contains((99.0, 10.0)));
    }
}
