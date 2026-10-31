//! `AreaChart`, `Sparkline`, `LevelMeter` e `HeatMap`: um histórico de valores, um nível em
//! quadradinhos e um ladrilho colorido por valor.
//!
//! O WinUI 3 não tem gráfico nem medidor de nível. O desenho segue a página Desempenho do
//! Gerenciador de Tarefas do Windows 11 (a linha da série sobre a área com a mesma cor
//! translúcida, a grade fina e a borda na cor da série) com os tokens do tema. O medidor segue os
//! medidores de nível em segmentos: os quadradinhos acendem da esquerda para a direita, do
//! amarelo-claro ao vermelho-escuro. O mapa de calor segue o que o Gerenciador mostra no lugar
//! dos gráficos por processador lógico quando eles são muitos: um ladrilho por processador, mais
//! cheio de cor quanto maior o uso. Adaptações registradas na spec.

use std::fmt;

use kurbo::{Point, Rect, Size};
use tomni_ui_reactive::{Get, GetUntracked, Set, Signal, signal, untrack};

use crate::button::ButtonState;
use crate::color::{ThemeColor, Token};
use crate::cx::EventCx;
use crate::event::Event;
use crate::press::Pressable;
use crate::prop::{IntoProp, Prop};
use crate::slider::format_number;
use crate::widget::{Common, Look, Widget, common_builders};

/// Quantos pontos cabem na largura do gráfico quando o app não diz: os 60 segundos do
/// Gerenciador de Tarefas, um ponto por segundo.
pub const CHART_CAPACITY: usize = 60;

/// Quantos quadradinhos o `LevelMeter` tem quando o app não diz.
pub const LEVEL_METER_SEGMENTS: u8 = 10;

/// A forma do gráfico.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ChartKind {
    /// O `AreaChart`: grade, borda, área e linha.
    #[default]
    Area,
    /// O `Sparkline`: borda, área e linha, sem grade.
    Sparkline,
}

/// Tudo o que o `Drawer` precisa saber de um gráfico para pintá-lo.
#[derive(Clone, Debug, PartialEq)]
pub struct ChartVisual {
    /// Os valores, do mais antigo ao mais novo. O mais novo fica na borda direita.
    pub values: Vec<f64>,
    /// O valor do topo do gráfico; o que passa dele fica no topo.
    pub max: f64,
    /// Quantos pontos cabem na largura: com menos valores, o gráfico começa no meio.
    pub capacity: usize,
    /// A cor da linha, da área (translúcida), da grade e da borda.
    pub color: ThemeColor,
    /// Área ou sparkline.
    pub kind: ChartKind,
    /// A segunda série, desenhada por cima da primeira na mesma escala (o tempo de kernel sob a
    /// CPU do Gerenciador de Tarefas); vazia, não aparece.
    pub secondary: Vec<f64>,
    /// A cor da linha e da área da segunda série.
    pub secondary_color: ThemeColor,
}

impl ChartVisual {
    /// Os valores que aparecem: os `capacity` mais novos, de 0 a 1 do topo (`NaN` vira 0).
    pub fn fractions(&self) -> Vec<f64> {
        self.scale(&self.values)
    }

    /// Os valores da segunda série que aparecem, como em [`fractions`](Self::fractions).
    pub fn secondary_fractions(&self) -> Vec<f64> {
        self.scale(&self.secondary)
    }

    fn scale(&self, values: &[f64]) -> Vec<f64> {
        let max = if self.max > 0.0 { self.max } else { 1.0 };
        let skip = values.len().saturating_sub(self.capacity.max(2));
        values[skip..]
            .iter()
            .map(|v| {
                if v.is_nan() {
                    0.0
                } else {
                    (v / max).clamp(0.0, 1.0)
                }
            })
            .collect()
    }
}

/// O que os dois gráficos têm em comum.
struct Chart {
    values: Prop<Vec<f64>>,
    max: Prop<f64>,
    capacity: usize,
    color: ThemeColor,
    kind: ChartKind,
    secondary: Prop<Vec<f64>>,
    secondary_color: ThemeColor,
}

impl Chart {
    fn new(values: Prop<Vec<f64>>, kind: ChartKind) -> Self {
        Self {
            values,
            max: Prop::Value(100.0),
            capacity: CHART_CAPACITY,
            color: ThemeColor::Token(Token::AccentFillColorDefault),
            kind,
            secondary: Prop::Value(Vec::new()),
            secondary_color: ThemeColor::Token(Token::SystemFillColorCritical),
        }
    }

    fn visual(&self) -> ChartVisual {
        ChartVisual {
            values: self.values.get(),
            max: self.max.get(),
            capacity: self.capacity,
            color: self.color,
            kind: self.kind,
            secondary: self.secondary.get(),
            secondary_color: self.secondary_color,
        }
    }

    /// O valor mais novo, para a árvore e para os leitores de tela.
    fn text(&self) -> Option<String> {
        let values = self.values.get();
        Some(values.last().map(|v| format_number(*v)).unwrap_or_default())
    }
}

macro_rules! chart_widget {
    ($name:ident, $kind:literal, $chart_kind:ident, $doc:literal, $example:literal) => {
        #[doc = $doc]
        ///
        /// ```
        #[doc = concat!("use tomni_ui_core::", stringify!($name), ";")]
        /// use tomni_ui_reactive::prelude::*;
        ///
        /// # let scope = Scope::root();
        /// # scope.run(|| {
        /// let uso = signal(vec![12.0, 30.0, 18.0]);
        #[doc = $example]
        /// # });
        /// # scope.clear();
        /// ```
        ///
        /// Os valores vão do mais antigo ao mais novo, e o mais novo fica na borda direita: o
        /// app guarda o histórico (um `Signal<Vec<f64>>`, um ponto por amostra) e o gráfico
        /// anda para a esquerda quando ganha um ponto. Só os [`capacity`](Self::capacity) mais
        /// novos aparecem. O topo é [`max`](Self::max) (100 por padrão, para porcentagens).
        pub struct $name {
            common: Common,
            chart: Chart,
        }

        impl $name {
            /// O gráfico dos valores `values` (fixos, closure ou signal).
            pub fn new<M>(values: impl IntoProp<Vec<f64>, M>) -> Self {
                Self {
                    common: Common::default(),
                    chart: Chart::new(values.into_prop(), ChartKind::$chart_kind),
                }
            }

            /// O valor do topo (fixo, closure ou signal): 100 por padrão.
            pub fn max<M>(mut self, max: impl IntoProp<f64, M>) -> Self {
                self.chart.max = max.into_prop();
                self
            }

            /// Quantos pontos cabem na largura: 60 por padrão.
            pub fn capacity(mut self, capacity: usize) -> Self {
                self.chart.capacity = capacity.max(2);
                self
            }

            /// A cor da série: o destaque do tema por padrão. Um token, uma cor fixa ou um par
            /// claro/escuro (`ThemeColor::themed`).
            pub fn color(mut self, color: impl Into<ThemeColor>) -> Self {
                self.chart.color = color.into();
                self
            }

            /// Uma segunda série (fixa, closure ou signal), por cima da primeira e na mesma
            /// escala, com a linha e a área na [`secondary_color`](Self::secondary_color): o
            /// tempo de kernel embaixo da CPU, como no Gerenciador de Tarefas. Vazia, some.
            pub fn secondary<M>(mut self, values: impl IntoProp<Vec<f64>, M>) -> Self {
                self.chart.secondary = values.into_prop();
                self
            }

            /// A cor da segunda série: o vermelho do tema (`SystemFillColorCritical`) por padrão.
            pub fn secondary_color(mut self, color: impl Into<ThemeColor>) -> Self {
                self.chart.secondary_color = color.into();
                self
            }

            /// Estado para o `Drawer`.
            pub fn visual(&self) -> ChartVisual {
                self.chart.visual()
            }
        }

        common_builders!($name);

        impl Widget for $name {
            fn kind(&self) -> &'static str {
                $kind
            }

            fn common(&self) -> &Common {
                &self.common
            }

            fn look(&self) -> Look {
                Look::Chart(self.visual())
            }

            fn text(&self) -> Option<String> {
                self.chart.text()
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_struct(stringify!($name))
                    .field("common", &self.common)
                    .field("visual", &self.visual())
                    .finish()
            }
        }
    };
}

chart_widget!(
    AreaChart,
    "area_chart",
    Area,
    "Gráfico de área com grade, como os da página Desempenho do Gerenciador de Tarefas.",
    "let grafico = AreaChart::new(uso).id(\"cpu\").max(100.0);\n\
     assert_eq!(grafico.visual().values, vec![12.0, 30.0, 18.0]);"
);

chart_widget!(
    Sparkline,
    "sparkline",
    Sparkline,
    "Gráfico pequeno, sem grade, para mostrar a tendência ao lado de um número.",
    "let mini = Sparkline::new(uso).id(\"cpu-mini\");\n\
     assert_eq!(mini.visual().capacity, 60);"
);

/// Tudo o que o `Drawer` precisa saber de um `LevelMeter` para pintá-lo.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LevelMeterVisual {
    /// O nível, de 0 a 1.
    pub value: f64,
    /// Quantos quadradinhos.
    pub segments: u8,
    /// Estado: hover e pressionado só quando é clicável.
    pub state: ButtonState,
    /// Anel de foco: só com foco vindo do teclado.
    pub focus_ring: bool,
}

impl LevelMeterVisual {
    /// Quantos quadradinhos ficam acesos: nenhum com o nível em zero, e pelo menos um com
    /// qualquer nível acima de zero.
    pub fn lit(&self) -> u8 {
        let segments = f64::from(self.segments);
        let lit = (self.value.clamp(0.0, 1.0) * segments).ceil();
        lit.clamp(0.0, segments) as u8
    }
}

/// Um nível em quadradinhos que acendem da esquerda para a direita, do amarelo-claro ao
/// vermelho-escuro: o uso de CPU de um programa, o volume de um microfone.
///
/// ```
/// use tomni_ui_core::LevelMeter;
/// use tomni_ui_reactive::prelude::*;
///
/// # let scope = Scope::root();
/// # scope.run(|| {
/// let uso = signal(0.35);
/// let medidor = LevelMeter::new(uso).id("cpu").tooltip("CPU: 35%");
/// assert_eq!(medidor.visual().lit(), 4);
/// # });
/// # scope.clear();
/// ```
///
/// `.value()` vai de 0 a 1. Com [`on_click`](Self::on_click), vira um controle: ganha foco, o
/// fundo de hover e de pressionado dos botões sutis, e Enter e Espaço clicam.
pub struct LevelMeter {
    common: Common,
    value: Prop<f64>,
    segments: u8,
    on_click: Option<Box<dyn FnMut()>>,
    press: Pressable,
}

impl LevelMeter {
    /// O medidor do nível `value`, de 0 a 1 (fixo, closure ou signal).
    pub fn new<M>(value: impl IntoProp<f64, M>) -> Self {
        Self {
            common: Common::default(),
            value: value.into_prop(),
            segments: LEVEL_METER_SEGMENTS,
            on_click: None,
            press: Pressable::default(),
        }
    }

    /// Quantos quadradinhos (de 2 a 40): 10 por padrão.
    pub fn segments(mut self, segments: u8) -> Self {
        self.segments = segments.clamp(2, 40);
        self
    }

    /// O que fazer no clique (ou com Enter e Espaço, com o foco).
    pub fn on_click(mut self, f: impl FnMut() + 'static) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }

    /// O nível agora, de 0 a 1.
    pub fn current(&self) -> f64 {
        let value = self.value.get();
        if value.is_nan() {
            0.0
        } else {
            value.clamp(0.0, 1.0)
        }
    }

    /// Estado para o `Drawer`.
    pub fn visual(&self) -> LevelMeterVisual {
        LevelMeterVisual {
            value: self.current(),
            segments: self.segments,
            state: if self.on_click.is_some() {
                self.press.state()
            } else {
                ButtonState::Rest
            },
            focus_ring: self.on_click.is_some() && self.press.focus_ring(),
        }
    }
}

common_builders!(LevelMeter);

impl Widget for LevelMeter {
    fn kind(&self) -> &'static str {
        "level_meter"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::LevelMeter(self.visual())
    }

    fn text(&self) -> Option<String> {
        Some(format_number(self.current()))
    }

    fn handle(&mut self, event: &Event, _: &mut EventCx) -> bool {
        if self.on_click.is_none() {
            return false;
        }
        let (consumed, clicked) = self.press.handle(event, true);
        if clicked && let Some(f) = &mut self.on_click {
            f();
        }
        consumed
    }

    fn interactive(&self) -> bool {
        self.on_click.is_some()
    }

    fn focusable(&self) -> bool {
        self.on_click.is_some()
    }
}

impl fmt::Debug for LevelMeter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LevelMeter")
            .field("common", &self.common)
            .field("visual", &self.visual())
            .finish_non_exhaustive()
    }
}

/// Espaço entre os ladrilhos do [`HeatMap`].
pub const HEAT_MAP_GAP: f64 = 2.0;

/// Colunas e linhas de `count` ladrilhos em `size`: as que deixam o lado menor do ladrilho o
/// maior possível, com os ladrilhos esticados para ocupar a área toda. Com o mesmo número de
/// linhas, fica o de menos colunas (os ladrilhos mais largos e menos buracos na última linha).
pub fn heat_map_grid(count: usize, size: Size) -> (usize, usize) {
    if count == 0 {
        return (0, 0);
    }
    let mut best = (count, 1, f64::MIN);
    for columns in 1..=count {
        let rows = count.div_ceil(columns);
        if columns > 1 && count.div_ceil(columns - 1) == rows {
            continue;
        }
        let (width, height) = heat_map_cell_size(size, columns, rows);
        let side = width.min(height);
        if side > best.2 {
            best = (columns, rows, side);
        }
    }
    (best.0, best.1)
}

fn heat_map_cell_size(size: Size, columns: usize, rows: usize) -> (f64, f64) {
    let gaps = |n: usize| HEAT_MAP_GAP * n.saturating_sub(1) as f64;
    (
        (size.width - gaps(columns)) / columns.max(1) as f64,
        (size.height - gaps(rows)) / rows.max(1) as f64,
    )
}

/// O retângulo do ladrilho `index` de `count` num mapa de calor em `rect`, da esquerda para a
/// direita e de cima para baixo.
pub fn heat_map_cell(count: usize, rect: Rect, index: usize) -> Rect {
    let (columns, rows) = heat_map_grid(count, rect.size());
    let (width, height) = heat_map_cell_size(rect.size(), columns, rows);
    let columns = columns.max(1);
    let x = rect.x0 + (index % columns) as f64 * (width + HEAT_MAP_GAP);
    let y = rect.y0 + (index / columns) as f64 * (height + HEAT_MAP_GAP);
    Rect::new(x, y, x + width.max(0.0), y + height.max(0.0))
}

/// O ladrilho de `count` sob `point` num mapa de calor em `rect`. O espaço entre dois
/// ladrilhos conta como o da esquerda (ou o de cima), para a dica não piscar entre eles.
pub fn heat_map_index_at(count: usize, rect: Rect, point: Point) -> Option<usize> {
    if count == 0 || !rect.contains(point) {
        return None;
    }
    let (columns, rows) = heat_map_grid(count, rect.size());
    let (width, height) = heat_map_cell_size(rect.size(), columns, rows);
    let column = ((point.x - rect.x0) / (width + HEAT_MAP_GAP))
        .floor()
        .max(0.0) as usize;
    let row = ((point.y - rect.y0) / (height + HEAT_MAP_GAP))
        .floor()
        .max(0.0) as usize;
    let index = row.min(rows - 1) * columns + column.min(columns - 1);
    (index < count).then_some(index)
}

/// Tudo o que o `Drawer` precisa saber de um `HeatMap` para pintá-lo.
#[derive(Clone, Debug, PartialEq)]
pub struct HeatMapVisual {
    /// Um valor por ladrilho, na ordem.
    pub values: Vec<f64>,
    /// O valor do ladrilho mais cheio de cor; o que passa dele fica igual.
    pub max: f64,
    /// A cor dos ladrilhos.
    pub color: ThemeColor,
    /// O ladrilho sob o ponteiro, que ganha a borda forte.
    pub hovered: Option<usize>,
}

impl HeatMapVisual {
    /// O valor do ladrilho `index` de 0 a 1 do máximo (`NaN` e o que falta viram 0).
    pub fn fraction(&self, index: usize) -> f64 {
        let max = if self.max > 0.0 { self.max } else { 1.0 };
        match self.values.get(index) {
            Some(v) if !v.is_nan() => (v / max).clamp(0.0, 1.0),
            _ => 0.0,
        }
    }
}

/// Um ladrilho por valor, mais cheio de cor quanto maior o valor: o uso de cada processador
/// lógico quando são dezenas ou centenas, como o Gerenciador de Tarefas mostra no lugar dos
/// gráficos.
///
/// ```
/// use tomni_ui_core::HeatMap;
/// use tomni_ui_reactive::prelude::*;
///
/// # let scope = Scope::root();
/// # scope.run(|| {
/// let nucleos = signal(vec![40.0, 10.0, 100.0]);
/// let mapa = HeatMap::new(nucleos)
///     .id("nucleos")
///     .cell_tooltip(|n, uso| format!("Núcleo {n}: {uso:.0}%"));
/// assert_eq!(mapa.visual().fraction(2), 1.0);
/// # });
/// # scope.clear();
/// ```
///
/// Os ladrilhos ocupam a área toda, em colunas e linhas que deixam cada um o mais perto de
/// quadrado ([`heat_map_grid`]). O topo é [`max`](Self::max) (100 por padrão, para
/// porcentagens). O ladrilho sob o ponteiro ganha a borda forte, e a dica de
/// [`cell_tooltip`](Self::cell_tooltip) fala dele.
pub struct HeatMap {
    common: Common,
    values: Prop<Vec<f64>>,
    max: Prop<f64>,
    color: ThemeColor,
    hovered: Signal<Option<usize>>,
}

impl HeatMap {
    /// O mapa dos valores `values` (fixos, closure ou signal), um por ladrilho.
    pub fn new<M>(values: impl IntoProp<Vec<f64>, M>) -> Self {
        Self {
            common: Common::default(),
            values: values.into_prop(),
            max: Prop::Value(100.0),
            color: ThemeColor::Token(Token::AccentFillColorDefault),
            hovered: signal(None),
        }
    }

    /// O valor do ladrilho mais cheio (fixo, closure ou signal): 100 por padrão.
    pub fn max<M>(mut self, max: impl IntoProp<f64, M>) -> Self {
        self.max = max.into_prop();
        self
    }

    /// A cor dos ladrilhos: o destaque do tema por padrão.
    pub fn color(mut self, color: impl Into<ThemeColor>) -> Self {
        self.color = color.into();
        self
    }

    /// A dica do ladrilho sob o ponteiro: `f` recebe a posição dele (de 0) e o valor. Fica no
    /// lugar da [`tooltip`](Self::tooltip), em cima do ladrilho.
    pub fn cell_tooltip(mut self, f: impl Fn(usize, f64) -> String + 'static) -> Self {
        let (values, hovered) = (self.values.clone(), self.hovered);
        let tip = move || match hovered.get() {
            Some(n) => values.get().get(n).map(|v| f(n, *v)).unwrap_or_default(),
            None => String::new(),
        };
        self.common.tooltip = Some(tip.into_prop());
        self
    }

    /// O ladrilho sob o ponteiro.
    pub fn hovered(&self) -> Option<usize> {
        self.hovered.get_untracked()
    }

    /// Estado para o `Drawer`.
    pub fn visual(&self) -> HeatMapVisual {
        HeatMapVisual {
            values: self.values.get(),
            max: self.max.get(),
            color: self.color,
            hovered: self.hovered.get(),
        }
    }
}

common_builders!(HeatMap);

impl Widget for HeatMap {
    fn kind(&self) -> &'static str {
        "heat_map"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::HeatMap(self.visual())
    }

    fn handle(&mut self, event: &Event, cx: &mut EventCx) -> bool {
        let hovered = match event {
            Event::PointerMove { inside: true, .. } => {
                let count = untrack(|| self.values.get().len());
                heat_map_index_at(count, cx.rect(), cx.pointer())
            }
            Event::PointerMove { inside: false, .. } | Event::PointerCancel => None,
            _ => return false,
        };
        if self.hovered.get_untracked() != hovered {
            self.hovered.set(hovered);
        }
        false
    }

    fn interactive(&self) -> bool {
        true
    }

    fn tip_anchor(&self, rect: Rect) -> Option<Rect> {
        let n = self.hovered.get_untracked()?;
        let count = untrack(|| self.values.get().len());
        Some(heat_map_cell(count, rect, n))
    }
}

impl fmt::Debug for HeatMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HeatMap")
            .field("common", &self.common)
            .field("visual", &untrack(|| self.visual()))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn medidor(value: f64) -> LevelMeterVisual {
        LevelMeterVisual {
            value,
            segments: 10,
            ..Default::default()
        }
    }

    #[test]
    fn o_medidor_acende_pelo_menos_um_com_qualquer_uso() {
        assert_eq!(medidor(0.0).lit(), 0);
        assert_eq!(medidor(0.001).lit(), 1);
        assert_eq!(medidor(0.35).lit(), 4);
        assert_eq!(medidor(1.0).lit(), 10);
        assert_eq!(medidor(7.0).lit(), 10);
    }

    #[test]
    fn o_grafico_mostra_os_mais_novos_na_escala_do_topo() {
        let visual = ChartVisual {
            values: vec![10.0, 50.0, f64::NAN, 200.0],
            max: 100.0,
            capacity: 3,
            color: ThemeColor::Token(Token::AccentFillColorDefault),
            kind: ChartKind::Area,
            secondary: vec![5.0, 20.0],
            secondary_color: ThemeColor::Token(Token::SystemFillColorCritical),
        };
        assert_eq!(visual.fractions(), vec![0.5, 0.0, 1.0]);
        assert_eq!(visual.secondary_fractions(), vec![0.05, 0.2]);
    }

    #[test]
    fn os_ladrilhos_ficam_o_mais_perto_de_quadrados() {
        // 4 numa faixa larga: uma linha; 12 numa área 5 vezes mais larga que alta: 2 linhas de 6.
        assert_eq!(heat_map_grid(4, Size::new(600.0, 130.0)), (4, 1));
        assert_eq!(heat_map_grid(12, Size::new(650.0, 130.0)), (6, 2));
        // 512 cabem e ninguém fica com o lado menor que 10 px.
        let (colunas, linhas) = heat_map_grid(512, Size::new(650.0, 130.0));
        assert!(colunas * linhas >= 512 && colunas * (linhas - 1) < 512);
        let rect = Rect::new(0.0, 0.0, 650.0, 130.0);
        let ultimo = heat_map_cell(512, rect, 511);
        assert!(ultimo.width().min(ultimo.height()) >= 10.0, "{ultimo:?}");
        assert!(ultimo.y1 <= 130.0 + 1e-9 && ultimo.x1 <= 650.0 + 1e-9);
        assert_eq!(heat_map_grid(0, rect.size()), (0, 0));
    }

    #[test]
    fn o_ponteiro_acha_o_ladrilho_de_baixo() {
        let rect = Rect::new(10.0, 10.0, 110.0, 50.0);
        // 8 em 100×40: 4 colunas e 2 linhas, de 23,5×19 px.
        assert_eq!(heat_map_grid(8, rect.size()), (4, 2));
        for n in 0..8 {
            let centro = heat_map_cell(8, rect, n).center();
            assert_eq!(heat_map_index_at(8, rect, centro), Some(n));
        }
        // No vão entre dois, o da esquerda; fora, nenhum; na última linha incompleta, nenhum.
        let primeiro = heat_map_cell(8, rect, 0);
        assert_eq!(
            heat_map_index_at(8, rect, Point::new(primeiro.x1 + 1.0, primeiro.center().y)),
            Some(0)
        );
        assert_eq!(heat_map_index_at(8, rect, Point::new(5.0, 20.0)), None);
        assert_eq!(
            heat_map_index_at(7, rect, heat_map_cell(8, rect, 7).center()),
            None
        );
    }

    #[test]
    fn a_dica_fala_do_ladrilho_sob_o_ponteiro() {
        use tomni_ui_reactive::prelude::*;
        let scope = Scope::root();
        scope.run(|| {
            let mapa = HeatMap::new(vec![40.0, f64::NAN, 250.0])
                .cell_tooltip(|n, uso| format!("Núcleo {n}: {uso:.0}%"));
            let dica = mapa.common().tooltip.clone().unwrap();
            assert_eq!(dica.get(), "");
            mapa.hovered.set(Some(2));
            assert_eq!(dica.get(), "Núcleo 2: 250%");
            let visual = mapa.visual();
            assert_eq!(visual.hovered, Some(2));
            assert_eq!(
                [visual.fraction(0), visual.fraction(1), visual.fraction(2)],
                [0.4, 0.0, 1.0]
            );
            assert_eq!(visual.fraction(9), 0.0);
        });
        scope.clear();
    }

    #[test]
    fn a_segunda_serie_segue_o_signal() {
        use tomni_ui_reactive::prelude::*;
        let scope = Scope::root();
        scope.run(|| {
            let kernel = signal(vec![1.0]);
            let grafico = AreaChart::new(vec![10.0]).secondary(kernel);
            assert_eq!(grafico.visual().secondary, vec![1.0]);
            kernel.set(vec![1.0, 2.0]);
            assert_eq!(grafico.visual().secondary, vec![1.0, 2.0]);
            assert!(Sparkline::new(vec![1.0]).visual().secondary.is_empty());
        });
        scope.clear();
    }
}
