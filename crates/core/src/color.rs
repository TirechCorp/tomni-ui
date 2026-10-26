//! Cores e pincéis: `Color` (fixa), `Token` (do tema, acompanha a troca de tema) e `Brush`
//! (cor sólida ou gradiente linear), usados pelo `Border`, pelo `Card` e pelo `Text`.

use kurbo::Point;

/// Cor fixa em sRGB, com alfa. Não muda com o tema: para isso, use um [`Token`].
///
/// ```
/// use tomni_ui_core::Color;
///
/// assert_eq!(Color::hex(0x0078D4), Color::rgb(0x00, 0x78, 0xD4));
/// assert_eq!(Color::rgba(0, 0, 0, 0x80).to_hex(), "#00000080");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Color {
    /// Vermelho.
    pub r: u8,
    /// Verde.
    pub g: u8,
    /// Azul.
    pub b: u8,
    /// Alfa: 0 é transparente, 255 é opaco.
    pub a: u8,
}

impl Color {
    /// Branco opaco.
    pub const WHITE: Self = Self::rgb(255, 255, 255);
    /// Preto opaco.
    pub const BLACK: Self = Self::rgb(0, 0, 0);

    /// Cor opaca.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// Cor com alfa.
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// Cor opaca a partir de `0xRRGGBB`, como no CSS.
    pub const fn hex(rgb: u32) -> Self {
        let [_, r, g, b] = rgb.to_be_bytes();
        Self::rgb(r, g, b)
    }

    /// `#RRGGBB`, ou `#RRGGBBAA` com transparência.
    pub fn to_hex(self) -> String {
        if self.a == 255 {
            format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
        } else {
            format!("#{:02X}{:02X}{:02X}{:02X}", self.r, self.g, self.b, self.a)
        }
    }
}

/// Cor do tema, com o nome do recurso do WinUI. O valor depende do tema (claro ou escuro) e
/// muda junto com ele.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(missing_docs, reason = "os nomes são os dos recursos do WinUI")]
pub enum Token {
    TextFillColorPrimary,
    TextFillColorSecondary,
    TextFillColorTertiary,
    TextFillColorDisabled,
    TextOnAccentFillColorPrimary,
    AccentTextFillColorPrimary,
    AccentFillColorDefault,
    ControlFillColorDefault,
    ControlFillColorSecondary,
    ControlFillColorTertiary,
    ControlStrokeColorDefault,
    CardBackgroundFillColorDefault,
    CardBackgroundFillColorSecondary,
    CardBackgroundFillColorTertiary,
    CardStrokeColorDefault,
    LayerFillColorDefault,
    LayerFillColorAlt,
    SolidBackgroundFillColorBase,
    SolidBackgroundFillColorTertiary,
    SubtleFillColorSecondary,
    SubtleFillColorTertiary,
    DividerStrokeColorDefault,
    SurfaceStrokeColorDefault,
    SystemFillColorSuccess,
    SystemFillColorCaution,
    SystemFillColorCritical,
    SystemFillColorAttention,
    SystemFillColorSuccessBackground,
    SystemFillColorCautionBackground,
    SystemFillColorCriticalBackground,
}

impl Token {
    /// Todos os tokens.
    pub const ALL: [Self; 30] = [
        Self::TextFillColorPrimary,
        Self::TextFillColorSecondary,
        Self::TextFillColorTertiary,
        Self::TextFillColorDisabled,
        Self::TextOnAccentFillColorPrimary,
        Self::AccentTextFillColorPrimary,
        Self::AccentFillColorDefault,
        Self::ControlFillColorDefault,
        Self::ControlFillColorSecondary,
        Self::ControlFillColorTertiary,
        Self::ControlStrokeColorDefault,
        Self::CardBackgroundFillColorDefault,
        Self::CardBackgroundFillColorSecondary,
        Self::CardBackgroundFillColorTertiary,
        Self::CardStrokeColorDefault,
        Self::LayerFillColorDefault,
        Self::LayerFillColorAlt,
        Self::SolidBackgroundFillColorBase,
        Self::SolidBackgroundFillColorTertiary,
        Self::SubtleFillColorSecondary,
        Self::SubtleFillColorTertiary,
        Self::DividerStrokeColorDefault,
        Self::SurfaceStrokeColorDefault,
        Self::SystemFillColorSuccess,
        Self::SystemFillColorCaution,
        Self::SystemFillColorCritical,
        Self::SystemFillColorAttention,
        Self::SystemFillColorSuccessBackground,
        Self::SystemFillColorCautionBackground,
        Self::SystemFillColorCriticalBackground,
    ];

    /// Nome do recurso no WinUI (`CardBackgroundFillColorDefault`), usado na árvore.
    pub fn name(self) -> &'static str {
        match self {
            Self::TextFillColorPrimary => "TextFillColorPrimary",
            Self::TextFillColorSecondary => "TextFillColorSecondary",
            Self::TextFillColorTertiary => "TextFillColorTertiary",
            Self::TextFillColorDisabled => "TextFillColorDisabled",
            Self::TextOnAccentFillColorPrimary => "TextOnAccentFillColorPrimary",
            Self::AccentTextFillColorPrimary => "AccentTextFillColorPrimary",
            Self::AccentFillColorDefault => "AccentFillColorDefault",
            Self::ControlFillColorDefault => "ControlFillColorDefault",
            Self::ControlFillColorSecondary => "ControlFillColorSecondary",
            Self::ControlFillColorTertiary => "ControlFillColorTertiary",
            Self::ControlStrokeColorDefault => "ControlStrokeColorDefault",
            Self::CardBackgroundFillColorDefault => "CardBackgroundFillColorDefault",
            Self::CardBackgroundFillColorSecondary => "CardBackgroundFillColorSecondary",
            Self::CardBackgroundFillColorTertiary => "CardBackgroundFillColorTertiary",
            Self::CardStrokeColorDefault => "CardStrokeColorDefault",
            Self::LayerFillColorDefault => "LayerFillColorDefault",
            Self::LayerFillColorAlt => "LayerFillColorAlt",
            Self::SolidBackgroundFillColorBase => "SolidBackgroundFillColorBase",
            Self::SolidBackgroundFillColorTertiary => "SolidBackgroundFillColorTertiary",
            Self::SubtleFillColorSecondary => "SubtleFillColorSecondary",
            Self::SubtleFillColorTertiary => "SubtleFillColorTertiary",
            Self::DividerStrokeColorDefault => "DividerStrokeColorDefault",
            Self::SurfaceStrokeColorDefault => "SurfaceStrokeColorDefault",
            Self::SystemFillColorSuccess => "SystemFillColorSuccess",
            Self::SystemFillColorCaution => "SystemFillColorCaution",
            Self::SystemFillColorCritical => "SystemFillColorCritical",
            Self::SystemFillColorAttention => "SystemFillColorAttention",
            Self::SystemFillColorSuccessBackground => "SystemFillColorSuccessBackground",
            Self::SystemFillColorCautionBackground => "SystemFillColorCautionBackground",
            Self::SystemFillColorCriticalBackground => "SystemFillColorCriticalBackground",
        }
    }
}

/// Uma cor fixa, um token do tema ou um par de cores, uma para cada tema.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ThemeColor {
    /// A mesma nos dois temas.
    Fixed(Color),
    /// Acompanha o tema.
    Token(Token),
    /// Uma cor para cada tema (como os `ThemeDictionaries` do WinUI): `light` no tema claro e
    /// `dark` no escuro.
    Themed {
        /// A cor no tema claro.
        light: Color,
        /// A cor no tema escuro.
        dark: Color,
    },
}

impl ThemeColor {
    /// Uma cor para cada tema: `light` no claro e `dark` no escuro.
    ///
    /// ```
    /// use tomni_ui_core::{Brush, Color, ThemeColor};
    ///
    /// let fundo = Brush::linear(
    ///     (0.0, 0.0),
    ///     (1.0, 1.0),
    ///     [
    ///         (0.0, ThemeColor::themed(Color::hex(0xD4F1F9), Color::hex(0x0E3B4C))),
    ///         (1.0, ThemeColor::themed(Color::hex(0x8CC4EE), Color::hex(0x1C2A4A))),
    ///     ],
    /// );
    /// assert_eq!(
    ///     fundo.describe(),
    ///     "linear-gradient(#D4F1F9/#0E3B4C 0%, #8CC4EE/#1C2A4A 100%)"
    /// );
    /// ```
    pub fn themed(light: Color, dark: Color) -> Self {
        Self::Themed { light, dark }
    }

    /// O nome do token, `#RRGGBB` ou `#CLARO/#ESCURO`.
    pub fn describe(self) -> String {
        match self {
            Self::Fixed(color) => color.to_hex(),
            Self::Token(token) => token.name().to_owned(),
            Self::Themed { light, dark } => format!("{}/{}", light.to_hex(), dark.to_hex()),
        }
    }
}

impl From<Color> for ThemeColor {
    fn from(color: Color) -> Self {
        Self::Fixed(color)
    }
}

impl From<Token> for ThemeColor {
    fn from(token: Token) -> Self {
        Self::Token(token)
    }
}

/// Ponto de um gradiente: a cor em `offset`, de 0 (início) a 1 (fim).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradientStop {
    /// Posição ao longo do gradiente, de 0 a 1.
    pub offset: f64,
    /// Cor nesse ponto.
    pub color: ThemeColor,
}

/// Gradiente linear de `start` a `end`, em coordenadas relativas ao retângulo pintado: `(0, 0)`
/// é o canto superior esquerdo e `(1, 1)`, o inferior direito, como o `LinearGradientBrush` do
/// WinUI.
#[derive(Clone, Debug, PartialEq)]
pub struct LinearGradient {
    /// Início, relativo ao retângulo.
    pub start: Point,
    /// Fim, relativo ao retângulo.
    pub end: Point,
    /// Cores, em ordem de `offset`.
    pub stops: Vec<GradientStop>,
}

/// Como pintar uma área: uma cor (fixa ou do tema) ou um gradiente linear.
///
/// ```
/// use tomni_ui_core::{Brush, Color, Token};
///
/// let card = Brush::token(Token::CardBackgroundFillColorDefault);
/// let destaque = Brush::linear(
///     (0.0, 0.0),
///     (1.0, 0.0),
///     [(0.0, Color::hex(0xCCE4F7)), (1.0, Color::hex(0xF3E5F5))],
/// );
/// assert_eq!(card.describe(), "CardBackgroundFillColorDefault");
/// assert_eq!(destaque.describe(), "linear-gradient(#CCE4F7 0%, #F3E5F5 100%)");
/// ```
#[derive(Clone, Debug, PartialEq)]
pub enum Brush {
    /// Cor sólida.
    Solid(ThemeColor),
    /// Gradiente linear.
    Linear(LinearGradient),
}

impl Brush {
    /// Cor fixa.
    pub fn color(color: Color) -> Self {
        Self::Solid(ThemeColor::Fixed(color))
    }

    /// Token do tema, que acompanha a troca de tema.
    pub fn token(token: Token) -> Self {
        Self::Solid(ThemeColor::Token(token))
    }

    /// Gradiente linear de `start` a `end` (relativos ao retângulo, de `(0, 0)` a `(1, 1)`),
    /// com as cores `(offset, cor)`. Cada cor é fixa ou um token.
    pub fn linear<C: Into<ThemeColor>>(
        start: impl Into<Point>,
        end: impl Into<Point>,
        stops: impl IntoIterator<Item = (f64, C)>,
    ) -> Self {
        let mut stops: Vec<GradientStop> = stops
            .into_iter()
            .map(|(offset, color)| GradientStop {
                offset: offset.clamp(0.0, 1.0),
                color: color.into(),
            })
            .collect();
        stops.sort_by(|a, b| a.offset.total_cmp(&b.offset));
        Self::Linear(LinearGradient {
            start: start.into(),
            end: end.into(),
            stops,
        })
    }

    /// O nome do token, `#RRGGBB` ou `linear-gradient(...)`, como a árvore mostra.
    pub fn describe(&self) -> String {
        match self {
            Self::Solid(color) => color.describe(),
            Self::Linear(gradient) => {
                let stops: Vec<String> = gradient
                    .stops
                    .iter()
                    .map(|s| format!("{} {}%", s.color.describe(), (s.offset * 100.0).round()))
                    .collect();
                format!("linear-gradient({})", stops.join(", "))
            }
        }
    }

    /// Cor do pincel no ponto `at` (relativo ao retângulo, de `(0, 0)` a `(1, 1)`): a sólida
    /// ou, num gradiente, a cor da ponta mais próxima de `at`. É a que o lint usa no contraste.
    pub fn nearest_color(&self, at: Point) -> Option<ThemeColor> {
        match self {
            Self::Solid(color) => Some(*color),
            Self::Linear(gradient) => {
                let (first, last) = (gradient.stops.first()?, gradient.stops.last()?);
                let axis = gradient.end - gradient.start;
                let len2 = axis.hypot2();
                let t = if len2 == 0.0 {
                    0.0
                } else {
                    (at - gradient.start).dot(axis) / len2
                };
                Some(if t < 0.5 { first.color } else { last.color })
            }
        }
    }
}

impl From<Color> for Brush {
    fn from(color: Color) -> Self {
        Self::color(color)
    }
}

impl From<Token> for Brush {
    fn from(token: Token) -> Self {
        Self::token(token)
    }
}

impl From<ThemeColor> for Brush {
    fn from(color: ThemeColor) -> Self {
        Self::Solid(color)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_e_descricao() {
        assert_eq!(Color::hex(0x112233), Color::rgb(0x11, 0x22, 0x33));
        assert_eq!(Color::hex(0x112233).to_hex(), "#112233");
        assert_eq!(
            Brush::token(Token::LayerFillColorAlt).describe(),
            "LayerFillColorAlt"
        );
        let g = Brush::linear(
            (0.0, 0.0),
            (0.0, 1.0),
            [
                (1.0, ThemeColor::from(Token::AccentFillColorDefault)),
                (0.0, ThemeColor::from(Color::WHITE)),
            ],
        );
        assert_eq!(
            g.describe(),
            "linear-gradient(#FFFFFF 0%, AccentFillColorDefault 100%)",
            "as cores ficam em ordem de offset"
        );
    }

    #[test]
    fn ponta_mais_proxima_do_gradiente() {
        let g = Brush::linear(
            (0.0, 0.0),
            (1.0, 0.0),
            [(0.0, Color::BLACK), (1.0, Color::WHITE)],
        );
        let at = |x| g.nearest_color(Point::new(x, 0.5));
        assert_eq!(at(0.1), Some(ThemeColor::Fixed(Color::BLACK)));
        assert_eq!(at(0.9), Some(ThemeColor::Fixed(Color::WHITE)));
        assert_eq!(
            Brush::token(Token::LayerFillColorDefault).nearest_color(Point::ZERO),
            Some(ThemeColor::Token(Token::LayerFillColorDefault))
        );
    }

    #[test]
    fn nomes_unicos() {
        let mut names: Vec<_> = Token::ALL.iter().map(|t| t.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), Token::ALL.len());
    }
}
