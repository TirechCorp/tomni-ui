//! `AreaChart`, `Sparkline` e `LevelMeter`: um histórico de valores e um nível em quadradinhos.
//!
//! O WinUI 3 não tem gráfico nem medidor de nível. O desenho segue a página Desempenho do
//! Gerenciador de Tarefas do Windows 11 (a linha da série sobre a área com a mesma cor
//! translúcida, a grade fina e a borda na cor da série) com os tokens do tema. O medidor segue os
//! medidores de nível em segmentos: os quadradinhos acendem da esquerda para a direita, do
//! amarelo-claro ao vermelho-escuro. Adaptações registradas na spec.

use std::fmt;

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
}

impl ChartVisual {
    /// Os valores que aparecem: os `capacity` mais novos, de 0 a 1 do topo (`NaN` vira 0).
    pub fn fractions(&self) -> Vec<f64> {
        let max = if self.max > 0.0 { self.max } else { 1.0 };
        let skip = self.values.len().saturating_sub(self.capacity.max(2));
        self.values[skip..]
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
}

impl Chart {
    fn new(values: Prop<Vec<f64>>, kind: ChartKind) -> Self {
        Self {
            values,
            max: Prop::Value(100.0),
            capacity: CHART_CAPACITY,
            color: ThemeColor::Token(Token::AccentFillColorDefault),
            kind,
        }
    }

    fn visual(&self) -> ChartVisual {
        ChartVisual {
            values: self.values.get(),
            max: self.max.get(),
            capacity: self.capacity,
            color: self.color,
            kind: self.kind,
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
        };
        assert_eq!(visual.fractions(), vec![0.5, 0.0, 1.0]);
    }
}
