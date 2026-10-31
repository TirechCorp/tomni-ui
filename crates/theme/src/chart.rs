//! `AreaChart`, `Sparkline`, `LevelMeter` e `HeatMap`, e o calor das células do `DataGrid`.
//!
//! O WinUI 3 não tem gráfico nem medidor de nível: o desenho segue a página Desempenho do
//! Gerenciador de Tarefas do Windows 11 (a borda e a linha na cor da série, a área com a mesma
//! cor translúcida e a grade fina, e os ladrilhos por processador lógico quando são muitos) e o
//! "mapa de calor" das colunas da lista de processos dele.

use tomni_ui_core::{
    ButtonState, ChartKind, ChartVisual, HeatMapVisual, LevelMeterVisual, heat_map_cell,
};
use tomni_ui_render::kurbo::{BezPath, Line, Point, Rect, RoundedRect, Shape, Size, Stroke};
use tomni_ui_render::peniko::Color;
use tomni_ui_render::{Canvas, CanvasExt};

use crate::{Theme, Tokens, metrics};

/// Tamanho natural do `AreaChart`: num contêiner que estica, ele ocupa o espaço todo.
const AREA_SIZE: Size = Size::new(160.0, 100.0);
/// Tamanho natural do `Sparkline`, o das miniaturas da coluna da esquerda do Gerenciador.
const SPARK_SIZE: Size = Size::new(72.0, 40.0);
/// Espessura da linha da série.
const LINE: f64 = 1.5;
/// Opacidade da área sob a linha.
const AREA_ALPHA: f32 = 0.18;
/// Opacidade da grade.
const GRID_ALPHA: f32 = 0.16;
/// Altura mínima de uma faixa da grade: abaixo disso, a grade tem 4 faixas em vez de 10.
const GRID_MIN_ROW: f64 = 12.0;

/// Largura de um quadradinho do `LevelMeter`.
const SEGMENT_WIDTH: f64 = 5.0;
/// Altura de um quadradinho.
const SEGMENT_HEIGHT: f64 = 12.0;
/// Espaço entre os quadradinhos.
const SEGMENT_GAP: f64 = 2.0;
/// Margem dos lados, onde aparece o fundo de hover.
const METER_PADDING: f64 = 6.0;
/// Altura natural do medidor; com mais altura, o fundo de hover ocupa toda ela.
const METER_HEIGHT: f64 = 24.0;

/// Tamanho natural do `HeatMap`, o mesmo do `AreaChart`: ele também ocupa o espaço que tiver.
const HEAT_MAP_SIZE: Size = AREA_SIZE;
/// Opacidade do ladrilho sem uso: o lugar dele continua à vista.
const HEAT_MIN_ALPHA: f32 = 0.08;
/// Opacidade do ladrilho no máximo.
const HEAT_MAX_ALPHA: f32 = 0.9;
/// Opacidade da borda dos ladrilhos.
const HEAT_BORDER_ALPHA: f32 = 0.45;
/// Lado mínimo do ladrilho para ter borda: abaixo disso, a borda tomaria a cor toda.
const HEAT_BORDER_MIN: f64 = 6.0;

pub(crate) fn measure(visual: &ChartVisual) -> Size {
    match visual.kind {
        ChartKind::Area => AREA_SIZE,
        ChartKind::Sparkline => SPARK_SIZE,
    }
}

/// Os pontos de uma série (de 0 a 1 do topo) em `area`: o mais novo na borda direita, um passo
/// por ponto.
fn points(fractions: &[f64], capacity: usize, area: Rect) -> Vec<Point> {
    let capacity = capacity.max(2);
    let step = area.width() / (capacity - 1) as f64;
    let count = fractions.len();
    fractions
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let x = area.x1 - (count - 1 - i) as f64 * step;
            let y = area.y1 - f * area.height();
            Point::new(x, y)
        })
        .collect()
}

pub(crate) fn draw(canvas: &mut dyn Canvas, rect: Rect, visual: &ChartVisual, t: &Tokens) {
    if rect.width() < 2.0 || rect.height() < 2.0 {
        return;
    }
    let color = t.resolve(visual.color);
    let inner = rect.inflate(-1.0, -1.0);
    if visual.kind == ChartKind::Area {
        let grid = color.with_alpha(GRID_ALPHA);
        let rows = if inner.height() / 10.0 >= GRID_MIN_ROW {
            10
        } else {
            4
        };
        for i in 1..rows {
            let y = (inner.y0 + inner.height() * f64::from(i) / f64::from(rows)).round() + 0.5;
            canvas.stroke(
                &Line::new((inner.x0, y), (inner.x1, y)),
                &Stroke::new(1.0),
                grid,
            );
        }
        let columns = (inner.width() / (inner.height() / f64::from(rows)))
            .round()
            .max(1.0);
        for i in 1..columns as u32 {
            let x = (inner.x0 + inner.width() * f64::from(i) / columns).round() + 0.5;
            canvas.stroke(
                &Line::new((x, inner.y0), (x, inner.y1)),
                &Stroke::new(1.0),
                grid,
            );
        }
    }
    draw_series(canvas, inner, visual, color);
    if !visual.secondary.is_empty() {
        let secondary = t.resolve(visual.secondary_color);
        draw_fractions(
            canvas,
            inner,
            &visual.secondary_fractions(),
            visual.capacity,
            secondary,
        );
    }
    canvas.stroke(&rect.inflate(-0.5, -0.5), &Stroke::new(1.0), color);
}

/// A área e a linha da série em `area`, sem borda nem grade (a célula `Sparkline` do
/// `DataGrid` desenha só isto).
pub(crate) fn draw_series(canvas: &mut dyn Canvas, area: Rect, visual: &ChartVisual, color: Color) {
    draw_fractions(canvas, area, &visual.fractions(), visual.capacity, color);
}

/// A área e a linha de uma série já na escala (de 0 a 1 do topo).
fn draw_fractions(
    canvas: &mut dyn Canvas,
    inner: Rect,
    fractions: &[f64],
    capacity: usize,
    color: Color,
) {
    let points = points(fractions, capacity, inner);
    if let (Some(first), Some(last)) = (points.first(), points.last()) {
        canvas.push_clip(&inner.to_path(0.1));
        let mut line = BezPath::new();
        line.move_to(*first);
        for p in &points[1..] {
            line.line_to(*p);
        }
        let mut area = line.clone();
        area.line_to((last.x, inner.y1));
        area.line_to((first.x, inner.y1));
        area.close_path();
        canvas.fill(&area, color.with_alpha(AREA_ALPHA));
        if points.len() > 1 {
            canvas.stroke(&line, &Stroke::new(LINE), color);
        }
        canvas.pop_clip();
    }
}

pub(crate) fn measure_level_meter(visual: &LevelMeterVisual) -> Size {
    let n = f64::from(visual.segments);
    Size::new(
        n * SEGMENT_WIDTH + (n - 1.0) * SEGMENT_GAP + 2.0 * METER_PADDING,
        METER_HEIGHT,
    )
}

/// As cores do medidor, do primeiro quadradinho ao último: do amarelo-claro ("começa
/// branco") ao vermelho-escuro. No escuro, o vermelho do fim é mais vivo, para não sumir no
/// fundo.
fn level_stops(theme: Theme) -> [Color; 5] {
    match theme {
        Theme::Light => [
            Color::from_rgb8(0xFF, 0xE3, 0x8F),
            Color::from_rgb8(0xF9, 0xC2, 0x4A),
            Color::from_rgb8(0xF0, 0x7E, 0x2C),
            Color::from_rgb8(0xD0, 0x2B, 0x22),
            Color::from_rgb8(0x8B, 0x0A, 0x0A),
        ],
        Theme::Dark => [
            Color::from_rgb8(0xFF, 0xF1, 0xC1),
            Color::from_rgb8(0xFF, 0xD1, 0x66),
            Color::from_rgb8(0xF4, 0x9A, 0x4A),
            Color::from_rgb8(0xE8, 0x55, 0x3F),
            Color::from_rgb8(0xC8, 0x1D, 0x25),
        ],
    }
}

/// A cor em `position` (de 0 a 1) ao longo de `stops`, igualmente espaçadas.
fn ramp(stops: &[Color], position: f64) -> Color {
    let position = position.clamp(0.0, 1.0) * (stops.len() - 1) as f64;
    let i = (position.floor() as usize).min(stops.len() - 2);
    let f = (position - i as f64) as f32;
    let [a, b] = [stops[i].components, stops[i + 1].components];
    Color::new([
        a[0] + (b[0] - a[0]) * f,
        a[1] + (b[1] - a[1]) * f,
        a[2] + (b[2] - a[2]) * f,
        1.0,
    ])
}

/// A cor do quadradinho `index` de `segments`, aceso.
pub(crate) fn segment_color(theme: Theme, index: u8, segments: u8) -> Color {
    let last = f64::from(segments.max(2) - 1);
    ramp(&level_stops(theme), f64::from(index) / last)
}

pub(crate) fn draw_level_meter(
    canvas: &mut dyn Canvas,
    rect: Rect,
    visual: &LevelMeterVisual,
    t: &Tokens,
) {
    let size = measure_level_meter(visual);
    let background = match visual.state {
        ButtonState::Hover => t.subtle_fill_secondary,
        ButtonState::Pressed => t.subtle_fill_tertiary,
        _ => Color::TRANSPARENT,
    };
    let pill = Rect::from_center_size(
        rect.center(),
        Size::new(size.width.min(rect.width()), rect.height()),
    );
    if background != Color::TRANSPARENT {
        canvas.fill(
            &RoundedRect::from_rect(pill, metrics::CONTROL_CORNER_RADIUS),
            background,
        );
    }
    let unlit = t.control_strong_stroke_default.multiply_alpha(0.35);
    let lit = visual.lit();
    let x0 = pill.x0 + METER_PADDING;
    let y0 = (pill.center().y - SEGMENT_HEIGHT / 2.0).round();
    for i in 0..visual.segments {
        let x = x0 + f64::from(i) * (SEGMENT_WIDTH + SEGMENT_GAP);
        let segment = Rect::new(x, y0, x + SEGMENT_WIDTH, y0 + SEGMENT_HEIGHT);
        let color = if i < lit {
            segment_color(t.theme, i, visual.segments)
        } else {
            unlit
        };
        canvas.fill(&RoundedRect::from_rect(segment, 1.0), color);
    }
    if visual.focus_ring {
        crate::draw_focus_ring(canvas, pill, metrics::CONTROL_CORNER_RADIUS, t);
    }
}

pub(crate) fn measure_heat_map() -> Size {
    HEAT_MAP_SIZE
}

/// Os ladrilhos: o fundo na cor com a opacidade do valor, a borda fina na mesma cor e, no
/// ladrilho sob o ponteiro, a borda forte na cor do texto. As bordas caem no pixel inteiro.
pub(crate) fn draw_heat_map(
    canvas: &mut dyn Canvas,
    rect: Rect,
    visual: &HeatMapVisual,
    t: &Tokens,
) {
    let count = visual.values.len();
    if count == 0 || rect.width() < 2.0 || rect.height() < 2.0 {
        return;
    }
    let color = t.resolve(visual.color);
    for n in 0..count {
        let cell = heat_map_cell(count, rect, n);
        let cell = Rect::new(
            cell.x0.round(),
            cell.y0.round(),
            cell.x1.round(),
            cell.y1.round(),
        );
        if cell.width() < 1.0 || cell.height() < 1.0 {
            continue;
        }
        let f = visual.fraction(n) as f32;
        let alpha = HEAT_MIN_ALPHA + (HEAT_MAX_ALPHA - HEAT_MIN_ALPHA) * f;
        canvas.fill(&cell, color.with_alpha(alpha));
        if visual.hovered == Some(n) {
            canvas.stroke(
                &cell.inflate(-0.5, -0.5),
                &Stroke::new(1.0),
                t.text_fill_primary,
            );
        } else if cell.width().min(cell.height()) >= HEAT_BORDER_MIN {
            canvas.stroke(
                &cell.inflate(-0.5, -0.5),
                &Stroke::new(1.0),
                color.with_alpha(HEAT_BORDER_ALPHA),
            );
        }
    }
}

/// O fundo de uma célula com calor `heat` (de 0 a 1), como as colunas da lista de processos
/// do Gerenciador de Tarefas: do amarelo-claro ao laranja-avermelhado no tema claro, e tons
/// escuros dos mesmos no escuro, para o texto primário continuar legível. `None` sem calor.
pub(crate) fn heat_fill(theme: Theme, heat: f64) -> Option<Color> {
    if heat.is_nan() || heat <= 0.0 {
        return None;
    }
    let stops = match theme {
        Theme::Light => [
            Color::from_rgb8(0xFF, 0xF4, 0xC4),
            Color::from_rgb8(0xFF, 0xD2, 0x7A),
            Color::from_rgb8(0xF6, 0x8E, 0x6B),
        ],
        Theme::Dark => [
            Color::from_rgb8(0x3D, 0x36, 0x22),
            Color::from_rgb8(0x6B, 0x4A, 0x1C),
            Color::from_rgb8(0x8A, 0x2E, 0x25),
        ],
    };
    Some(ramp(&stops, heat))
}

#[cfg(test)]
mod tests {
    use tomni_ui_core::{ThemeColor, Token};
    use tomni_ui_render::CpuCanvas;

    use super::*;

    fn rgb(cor: Color) -> [u8; 3] {
        let [r, g, b, _] = cor.to_rgba8().to_u8_array();
        [r, g, b]
    }

    #[test]
    fn os_quadradinhos_vao_do_claro_ao_vermelho_escuro() {
        for theme in [Theme::Light, Theme::Dark] {
            let primeiro = segment_color(theme, 0, 10);
            let ultimo = segment_color(theme, 9, 10);
            assert_eq!(rgb(primeiro), rgb(level_stops(theme)[0]));
            assert_eq!(rgb(ultimo), rgb(level_stops(theme)[4]));
            // Vermelho no fim: muito mais vermelho que verde.
            let [r, g, _] = rgb(ultimo);
            assert!(r > 3 * g.max(1) / 2 && r > 120, "{:?}", rgb(ultimo));
        }
    }

    #[test]
    fn so_os_quadradinhos_do_nivel_acendem() {
        let t = &Tokens::LIGHT;
        let visual = LevelMeterVisual {
            value: 0.3,
            segments: 10,
            ..Default::default()
        };
        let size = measure_level_meter(&visual);
        let mut canvas = CpuCanvas::new(size.width as u16, size.height as u16);
        draw_level_meter(
            &mut canvas,
            Rect::from_origin_size((0.0, 0.0), size),
            &visual,
            t,
        );
        let imagem = canvas.render(Color::WHITE);
        let centro = |i: u8| {
            let x = METER_PADDING + f64::from(i) * (SEGMENT_WIDTH + SEGMENT_GAP) + 2.0;
            let px = imagem.sample(x as u16, (size.height / 2.0) as u16);
            [px.r, px.g, px.b]
        };
        assert_eq!(centro(0), rgb(segment_color(Theme::Light, 0, 10)));
        assert_eq!(centro(2), rgb(segment_color(Theme::Light, 2, 10)));
        assert_ne!(centro(3), rgb(segment_color(Theme::Light, 3, 10)));
    }

    #[test]
    fn a_area_fica_sob_a_linha_e_o_mais_novo_na_direita() {
        let t = &Tokens::LIGHT;
        let visual = ChartVisual {
            values: vec![0.0, 100.0],
            max: 100.0,
            capacity: 2,
            color: ThemeColor::Token(Token::AccentFillColorDefault),
            kind: ChartKind::Sparkline,
            secondary: Vec::new(),
            secondary_color: ThemeColor::Token(Token::SystemFillColorCritical),
        };
        let mut canvas = CpuCanvas::new(40, 40);
        draw(&mut canvas, Rect::new(0.0, 0.0, 40.0, 40.0), &visual, t);
        let imagem = canvas.render(Color::WHITE);
        let branco = |x: u16, y: u16| {
            let px = imagem.sample(x, y);
            [px.r, px.g, px.b] == [255, 255, 255]
        };
        // A reta vai do canto de baixo à esquerda ao de cima à direita: embaixo dela, área.
        assert!(!branco(30, 30));
        assert!(branco(8, 20));
    }

    #[test]
    fn a_segunda_serie_fica_por_cima_na_cor_dela() {
        let t = &Tokens::LIGHT;
        let mut visual = ChartVisual {
            values: vec![100.0, 100.0],
            max: 100.0,
            capacity: 2,
            color: ThemeColor::Token(Token::AccentFillColorDefault),
            kind: ChartKind::Sparkline,
            secondary: Vec::new(),
            secondary_color: ThemeColor::Token(Token::SystemFillColorCritical),
        };
        let pixel = |visual: &ChartVisual| {
            let mut canvas = CpuCanvas::new(40, 40);
            draw(&mut canvas, Rect::new(0.0, 0.0, 40.0, 40.0), visual, t);
            let px = canvas.render(Color::WHITE).sample(20, 34);
            [px.r, px.g, px.b]
        };
        let sem = pixel(&visual);
        visual.secondary = vec![50.0, 50.0];
        let com = pixel(&visual);
        // Embaixo da metade, a área vermelha por cima da azul: o pixel chega mais perto do
        // vermelho do tema (que é mais escuro que o azul-claro, então o R também pode cair).
        let vermelho = rgb(t.get(Token::SystemFillColorCritical));
        let distancia = |px: [u8; 3]| {
            px.iter()
                .zip(vermelho)
                .map(|(a, b)| (i32::from(*a) - i32::from(b)).pow(2))
                .sum::<i32>()
        };
        assert!(distancia(com) < distancia(sem), "{sem:?} {com:?}");
        assert!(com[2] < sem[2], "{sem:?} {com:?}");
    }

    #[test]
    fn o_ladrilho_mais_usado_e_o_mais_cheio_de_cor() {
        let t = &Tokens::LIGHT;
        let visual = HeatMapVisual {
            values: vec![0.0, 50.0, 100.0, 100.0],
            max: 100.0,
            color: ThemeColor::Token(Token::AccentFillColorDefault),
            hovered: Some(3),
        };
        let rect = Rect::new(0.0, 0.0, 80.0, 20.0);
        let mut canvas = CpuCanvas::new(80, 20);
        draw_heat_map(&mut canvas, rect, &visual, t);
        let imagem = canvas.render(Color::WHITE);
        let centro = |n: usize| {
            let c = heat_map_cell(4, rect, n).center();
            let px = imagem.sample(c.x as u16, c.y as u16);
            u32::from(px.r) + u32::from(px.g) + u32::from(px.b)
        };
        // Sobre o branco, mais cor é mais escuro; o sem uso ainda não é branco.
        assert!(centro(0) < 3 * 255);
        assert!(centro(1) < centro(0) && centro(2) < centro(1));
        // O de baixo do ponteiro tem a borda na cor do texto (escura, no tema claro); o outro
        // cheio, a borda na cor dele.
        let borda = |n: usize| {
            let c = heat_map_cell(4, rect, n);
            let px = imagem.sample(c.x0.round() as u16, c.center().y as u16);
            u32::from(px.r) + u32::from(px.g) + u32::from(px.b)
        };
        assert!(borda(3) + 60 < borda(2), "{} {}", borda(3), borda(2));
    }

    #[test]
    fn sem_calor_a_celula_fica_sem_fundo() {
        assert_eq!(heat_fill(Theme::Light, 0.0), None);
        assert_eq!(heat_fill(Theme::Dark, f64::NAN), None);
        let pouco = heat_fill(Theme::Light, 0.05).unwrap();
        let muito = heat_fill(Theme::Light, 1.0).unwrap();
        // Mais calor, mais escuro.
        let soma = |c: Color| rgb(c).iter().map(|v| u32::from(*v)).sum::<u32>();
        assert!(soma(muito) < soma(pouco));
    }
}
