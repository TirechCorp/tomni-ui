//! `ProgressBar` e `ProgressRing` no visual do WinUI.

use std::f64::consts::{PI, TAU};

use tomni_ui_core::{ProgressStatus, ProgressVisual};
use tomni_ui_render::kurbo::{Arc, Cap, Point, Rect, RoundedRect, Size, Stroke, Vec2};
use tomni_ui_render::peniko::Color;
use tomni_ui_render::{Canvas, CanvasExt, Paint};

use crate::Tokens;

/// `ProgressBarMinHeight`: a altura do indicador.
const BAR_HEIGHT: f64 = 3.0;
/// `ProgressBarTrackHeight`: o trilho é uma linha fina centrada no indicador.
const BAR_TRACK: f64 = 1.0;
/// Largura natural da barra; num contêiner que estica, ela ocupa a largura toda.
const BAR_WIDTH: f64 = 120.0;
/// Fração da barra que o segmento indeterminado ocupa.
const BAR_SEGMENT: f64 = 0.4;
/// Segundos para o segmento indeterminado atravessar a barra.
const BAR_CYCLE: f64 = 2.0;
/// Onde o segmento está na fase 0, em fração do ciclo: já dentro da barra, para os
/// screenshots mostrarem alguma coisa.
const BAR_START: f64 = 0.3;

/// Lado do anel (o tamanho padrão do `ProgressRing`).
const RING_SIZE: f64 = 32.0;
/// `ProgressRingStrokeThickness`.
const RING_STROKE: f64 = 4.0;
/// Espessura mínima do arco num anel pequeno.
const RING_STROKE_MIN: f64 = 2.0;
/// Segundos por volta do arco indeterminado.
const RING_TURN: f64 = 1.5;

pub(crate) fn measure_bar() -> Size {
    Size::new(BAR_WIDTH, BAR_HEIGHT)
}

pub(crate) fn measure_ring() -> Size {
    Size::new(RING_SIZE, RING_SIZE)
}

/// Faixa do indicador num `rect` mais alto que ele: centrada na vertical.
fn bar_band(rect: Rect) -> Rect {
    let y = rect.y0 + ((rect.height() - BAR_HEIGHT) / 2.0).floor().max(0.0);
    Rect::new(rect.x0, y, rect.x1, y + BAR_HEIGHT)
}

/// Pedaço da barra pintado, em fração da largura: do começo ao valor, ou o segmento que
/// corre no indeterminado (cortado nas pontas).
fn bar_span(visual: ProgressVisual) -> (f64, f64) {
    match visual.value {
        Some(value) => (0.0, value.clamp(0.0, 1.0)),
        // `IndeterminatePaused` e `IndeterminateError` do WinUI: a barra toda, parada.
        None if visual.status != ProgressStatus::Normal => (0.0, 1.0),
        None => {
            let cycle = (visual.phase / BAR_CYCLE + BAR_START).fract();
            // O segmento entra pela esquerda e sai pela direita.
            let start = cycle * (1.0 + BAR_SEGMENT) - BAR_SEGMENT;
            (start.max(0.0), (start + BAR_SEGMENT).min(1.0))
        }
    }
}

/// A cor do indicador: destaque, alerta (`SystemFillColorCaution`) ou erro
/// (`SystemFillColorCritical`).
fn bar_color(status: ProgressStatus, t: &Tokens) -> Color {
    match status {
        ProgressStatus::Normal => t.accent_fill_default,
        ProgressStatus::Paused => t.info_bar_warning_icon,
        ProgressStatus::Error => t.info_bar_error_icon,
    }
}

pub(crate) fn draw_bar(canvas: &mut dyn Canvas, rect: Rect, visual: ProgressVisual, t: &Tokens) {
    let band = bar_band(rect);
    let cy = band.center().y;
    canvas.fill(
        &Rect::new(band.x0, cy - BAR_TRACK / 2.0, band.x1, cy + BAR_TRACK / 2.0),
        t.control_strong_stroke_default,
    );
    let (from, to) = bar_span(visual);
    if to > from {
        let w = band.width();
        let fill = Rect::new(band.x0 + from * w, band.y0, band.x0 + to * w, band.y1);
        canvas.fill(
            &RoundedRect::from_rect(fill, BAR_HEIGHT / 2.0),
            bar_color(visual.status, t),
        );
    }
}

/// Ângulos do arco: o começo (a partir do eixo x, sentido horário na tela) e o tamanho.
fn ring_arc(visual: ProgressVisual) -> (f64, f64) {
    let top = -PI / 2.0;
    match visual.value {
        Some(value) => (top, value.clamp(0.0, 1.0) * TAU),
        // Adaptação do M49: pausado ou com erro, sem valor, o anel inteiro e parado (como a
        // barra), porque a aba precisa mostrar esses estados em 16 px.
        None if visual.status != ProgressStatus::Normal => (top, TAU),
        None => {
            // Gira e respira: o arco cresce e encolhe enquanto dá a volta.
            let turn = visual.phase / RING_TURN;
            let sweep = PI * (0.8 + 0.5 * (PI * visual.phase).sin());
            (top + turn.fract() * TAU, sweep)
        }
    }
}

/// A espessura do arco: a do WinUI até o lado padrão; num anel menor, na proporção (com o
/// piso de 2 px), para o de 16 px de uma aba não virar um bloco.
fn ring_stroke(size: f64) -> f64 {
    (RING_STROKE * (size / RING_SIZE).min(1.0)).max(RING_STROKE_MIN)
}

pub(crate) fn draw_ring(canvas: &mut dyn Canvas, rect: Rect, visual: ProgressVisual, t: &Tokens) {
    let (start, sweep) = ring_arc(visual);
    if sweep <= 0.0 {
        return;
    }
    let size = rect.width().min(rect.height());
    let thickness = ring_stroke(size);
    let radius = (size - thickness) / 2.0;
    let center: Point = rect.center();
    let arc = Arc::new(center, Vec2::new(radius, radius), start, sweep, 0.0);
    let stroke = Stroke::new(thickness).with_caps(Cap::Round);
    canvas.stroke(&arc, &stroke, Paint::Solid(bar_color(visual.status, t)));
}

#[cfg(test)]
mod tests {
    use tomni_ui_render::CpuCanvas;

    use super::*;

    fn visual(value: Option<f64>, phase: f64) -> ProgressVisual {
        ProgressVisual {
            value,
            phase,
            ..Default::default()
        }
    }

    fn com_status(value: Option<f64>, status: ProgressStatus) -> ProgressVisual {
        ProgressVisual {
            value,
            status,
            ..Default::default()
        }
    }

    /// A cor do pixel (x, 1) de uma barra de 120 × 3 pintada sobre o fundo escuro do tema.
    fn pixel(visual: ProgressVisual, t: &Tokens, x: u16) -> [u8; 3] {
        let mut canvas = CpuCanvas::new(120, 3);
        draw_bar(&mut canvas, Rect::new(0.0, 0.0, 120.0, 3.0), visual, t);
        let cor = canvas.render(Color::BLACK).sample(x, 1);
        [cor.r, cor.g, cor.b]
    }

    fn rgb(cor: Color) -> [u8; 3] {
        let [r, g, b, _] = cor.to_rgba8().to_u8_array();
        [r, g, b]
    }

    #[test]
    fn a_barra_pausada_pinta_com_o_alerta_e_com_erro_com_o_erro() {
        for t in [&Tokens::LIGHT, &Tokens::DARK] {
            let normal = pixel(com_status(Some(0.5), ProgressStatus::Normal), t, 10);
            let pausada = pixel(com_status(Some(0.5), ProgressStatus::Paused), t, 10);
            let com_erro = pixel(com_status(Some(0.5), ProgressStatus::Error), t, 10);
            assert_eq!(normal, rgb(t.accent_fill_default));
            assert_eq!(pausada, rgb(t.info_bar_warning_icon));
            assert_eq!(com_erro, rgb(t.info_bar_error_icon));
            assert_ne!(pausada, normal);
            assert_ne!(com_erro, pausada);
        }
        // O amarelo escuro no claro e o claro no escuro, os do WinUI.
        assert_eq!(rgb(Tokens::LIGHT.info_bar_warning_icon), [0x9D, 0x5D, 0x00]);
        assert_eq!(rgb(Tokens::DARK.info_bar_warning_icon), [0xFC, 0xE1, 0x00]);
    }

    #[test]
    fn a_barra_com_status_so_pinta_ate_o_valor() {
        let t = &Tokens::LIGHT;
        let visual = com_status(Some(0.25), ProgressStatus::Paused);
        assert_eq!(pixel(visual, t, 10), rgb(t.info_bar_warning_icon));
        assert_ne!(pixel(visual, t, 100), rgb(t.info_bar_warning_icon));
    }

    #[test]
    fn indeterminada_com_status_enche_a_barra_e_fica_parada() {
        for status in [ProgressStatus::Paused, ProgressStatus::Error] {
            assert_eq!(bar_span(com_status(None, status)), (0.0, 1.0));
            let mut depois = com_status(None, status);
            depois.phase = 1.3;
            assert_eq!(bar_span(depois), (0.0, 1.0));
        }
        assert!(bar_span(com_status(None, ProgressStatus::Normal)).1 < 1.0);
    }

    #[test]
    fn barra_determinada_vai_do_comeco_ao_valor() {
        assert_eq!(bar_span(visual(Some(0.25), 0.0)), (0.0, 0.25));
        assert_eq!(bar_span(visual(Some(3.0), 0.0)), (0.0, 1.0));
    }

    #[test]
    fn barra_indeterminada_aparece_na_fase_zero_e_anda() {
        let (a, b) = bar_span(visual(None, 0.0));
        assert!(b - a > 0.3, "{a}..{b}");
        let (c, _) = bar_span(visual(None, 0.5));
        assert!(c > a);
    }

    #[test]
    fn anel_determinado_comeca_no_topo() {
        let (start, sweep) = ring_arc(visual(Some(0.5), 0.0));
        assert_eq!(start, -PI / 2.0);
        assert_eq!(sweep, PI);
        assert!(ring_arc(visual(None, 0.0)).1 > 0.0);
    }

    #[test]
    fn anel_indeterminado_pausado_ou_com_erro_e_inteiro_e_parado() {
        for status in [ProgressStatus::Paused, ProgressStatus::Error] {
            let a = ring_arc(com_status(None, status));
            let mut depois = com_status(None, status);
            depois.phase = 0.9;
            assert_eq!(a, (-PI / 2.0, TAU));
            assert_eq!(ring_arc(depois), a);
        }
        // Com valor, o status só muda a cor: o arco é o mesmo.
        assert_eq!(
            ring_arc(com_status(Some(0.6), ProgressStatus::Paused)),
            ring_arc(com_status(Some(0.6), ProgressStatus::Normal))
        );
    }

    #[test]
    fn o_anel_pausado_e_com_erro_pinta_com_a_cor_do_status() {
        for t in [&Tokens::LIGHT, &Tokens::DARK] {
            for (status, cor) in [
                (ProgressStatus::Normal, t.accent_fill_default),
                (ProgressStatus::Paused, t.info_bar_warning_icon),
                (ProgressStatus::Error, t.info_bar_error_icon),
            ] {
                let mut canvas = CpuCanvas::new(16, 16);
                let rect = Rect::new(0.0, 0.0, 16.0, 16.0);
                draw_ring(&mut canvas, rect, com_status(Some(1.0), status), t);
                // O topo do anel cheio, no meio da espessura.
                let px = canvas.render(Color::BLACK).sample(8, 1);
                assert_eq!([px.r, px.g, px.b], rgb(cor), "{status:?}");
            }
        }
    }

    #[test]
    fn a_espessura_do_anel_acompanha_o_tamanho_ate_o_piso() {
        assert_eq!(ring_stroke(32.0), 4.0);
        assert_eq!(ring_stroke(64.0), 4.0);
        assert_eq!(ring_stroke(16.0), 2.0);
        assert_eq!(ring_stroke(8.0), 2.0);
    }
}
