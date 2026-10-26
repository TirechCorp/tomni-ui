//! Tokens de cor e medidas do Fluent 2 para WinUI.
//!
//! Valores conferidos em `microsoft/microsoft-ui-xaml` (MIT):
//! `controls/dev/CommonStyles/Common_themeresources_any.xaml`, `Button_themeresources.xaml` e
//! `TextBox_themeresources.xaml`, `CheckBox_themeresources.xaml`,
//! `ToggleSwitch_themeresources.xaml`, `InfoBar_themeresources.xaml` e
//! `ScrollBar_themeresources.xaml`, `ContentDialog_themeresources.xaml`,
//! `FlyoutPresenter_themeresources.xaml`, `MenuFlyout_themeresources.xaml`,
//! `ComboBox_themeresources.xaml` e `ToolTip_themeresources.xaml`, `RadioButton_themeresources.xaml`,
//! `Slider_themeresources.xaml`, `ProgressBar_themeresources.xaml`, `Expander_themeresources.xaml`,
//! `HyperlinkButton_themeresources.xaml` e `TabView_themeresources.xaml`; o fundo dos popups e o trilho
//! da barra de rolagem vêm de `controls/dev/Materials/Acrylic/AcrylicBrush_themeresources.xaml`.
//! Os nomes seguem os recursos de lá, sem o sufixo `Color`.

use tomni_ui_core::{ThemeColor, Token};
use tomni_ui_render::peniko::Color;

/// Tema claro ou escuro.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Theme {
    /// Claro.
    #[default]
    Light,
    /// Escuro.
    Dark,
}

/// Cores de um tema.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    /// Tema destes tokens.
    pub theme: Theme,
    /// `SolidBackgroundFillColorBase`: fundo da janela.
    pub solid_background_base: Color,
    /// `ControlFillColorDefault`.
    pub control_fill_default: Color,
    /// `ControlFillColorSecondary`: hover.
    pub control_fill_secondary: Color,
    /// `ControlFillColorTertiary`: pressionado.
    pub control_fill_tertiary: Color,
    /// `ControlFillColorDisabled`.
    pub control_fill_disabled: Color,
    /// `ControlFillColorInputActive`: fundo do campo de texto com foco.
    pub control_fill_input_active: Color,
    /// `ControlStrokeColorDefault`.
    pub control_stroke_default: Color,
    /// `ControlStrokeColorSecondary`: a parte mais marcada da borda de elevação.
    pub control_stroke_secondary: Color,
    /// `ControlStrongStrokeColorDefault`: linha de baixo do campo de texto.
    pub control_strong_stroke_default: Color,
    /// `ControlStrokeColorOnAccentDefault`.
    pub control_stroke_on_accent_default: Color,
    /// `ControlStrokeColorOnAccentSecondary`.
    pub control_stroke_on_accent_secondary: Color,
    /// `TextFillColorPrimary`.
    pub text_fill_primary: Color,
    /// `TextFillColorSecondary`.
    pub text_fill_secondary: Color,
    /// `TextFillColorDisabled`.
    pub text_fill_disabled: Color,
    /// `TextOnAccentFillColorPrimary`.
    pub text_on_accent_primary: Color,
    /// `TextOnAccentFillColorSecondary`.
    pub text_on_accent_secondary: Color,
    /// `TextOnAccentFillColorDisabled`.
    pub text_on_accent_disabled: Color,
    /// `AccentFillColorDefault`: a cor accent do tema.
    pub accent_fill_default: Color,
    /// `AccentFillColorSecondary`: hover (accent com 90% de opacidade).
    pub accent_fill_secondary: Color,
    /// `AccentFillColorTertiary`: pressionado (accent com 80% de opacidade).
    pub accent_fill_tertiary: Color,
    /// `AccentFillColorDisabled`.
    pub accent_fill_disabled: Color,
    /// `AccentFillColorSelectedTextBackground`: fundo do texto selecionado (`SystemAccentColor`,
    /// igual nos dois temas).
    pub accent_selected_text_background: Color,
    /// `TextOnAccentFillColorSelectedText`: texto selecionado.
    pub text_on_accent_selected_text: Color,
    /// `FocusStrokeColorOuter`: anel de foco externo, de 2 px.
    pub focus_stroke_outer: Color,
    /// `FocusStrokeColorInner`: anel de foco interno, de 1 px.
    pub focus_stroke_inner: Color,
    /// `ControlAltFillColorSecondary`: caixa desmarcada e trilho desligado, parados.
    pub control_alt_fill_secondary: Color,
    /// `ControlAltFillColorTertiary`: com hover.
    pub control_alt_fill_tertiary: Color,
    /// `ControlAltFillColorQuarternary`: pressionados.
    pub control_alt_fill_quarternary: Color,
    /// `ControlAltFillColorDisabled`: desabilitados (transparente).
    pub control_alt_fill_disabled: Color,
    /// `ControlStrongStrokeColorDisabled`: borda da caixa e do trilho desabilitados.
    pub control_strong_stroke_disabled: Color,
    /// `CardStrokeColorDefault`: borda da `InfoBar`.
    pub card_stroke_default: Color,
    /// `SubtleFillColorSecondary`: botão discreto com hover.
    pub subtle_fill_secondary: Color,
    /// `SubtleFillColorTertiary`: botão discreto pressionado.
    pub subtle_fill_tertiary: Color,
    /// `TextFillColorInverse`: o miolo dos ícones da `InfoBar`.
    pub text_fill_inverse: Color,
    /// `InfoBarInformationalSeverityBackgroundBrush`.
    pub info_bar_informational_background: Color,
    /// `InfoBarSuccessSeverityBackgroundBrush`.
    pub info_bar_success_background: Color,
    /// `InfoBarWarningSeverityBackgroundBrush`.
    pub info_bar_warning_background: Color,
    /// `InfoBarErrorSeverityBackgroundBrush`.
    pub info_bar_error_background: Color,
    /// `InfoBarSuccessSeverityIconBackground`: o informativo usa a accent.
    pub info_bar_success_icon: Color,
    /// `InfoBarWarningSeverityIconBackground`.
    pub info_bar_warning_icon: Color,
    /// `InfoBarErrorSeverityIconBackground`.
    pub info_bar_error_icon: Color,
    /// `ControlStrongFillColorDefault`: polegar e setas da barra de rolagem.
    pub control_strong_fill_default: Color,
    /// `ControlStrongFillColorDisabled`.
    pub control_strong_fill_disabled: Color,
    /// `AcrylicInAppFillColorDefaultBrush`: fundo dos popups e trilho da barra de rolagem
    /// expandida. O piloto não tem acrílico e usa o `FallbackColor` do pincel, opaco.
    pub acrylic_in_app_fill_default: Color,
    /// `SmokeFillColorDefault`: escurece a janela atrás de um `ContentDialog`.
    pub smoke_fill_default: Color,
    /// `SurfaceStrokeColorFlyout`: borda dos popups (flyout, menu, lista do `ComboBox`, dica).
    pub surface_stroke_flyout: Color,
    /// `SurfaceStrokeColorDefault`: borda do `ContentDialog`.
    pub surface_stroke_default: Color,
    /// `DividerStrokeColorDefault`: separador do menu.
    pub divider_stroke_default: Color,
    /// `LayerFillColorAlt`: a parte de cima do `ContentDialog`, sobre o fundo da janela.
    pub layer_fill_alt: Color,
    /// `TextFillColorTertiary`.
    pub text_fill_tertiary: Color,
    /// `AccentTextFillColorPrimary`: o texto do botão de link.
    pub accent_text_primary: Color,
    /// `AccentTextFillColorSecondary`: o texto do botão de link com hover.
    pub accent_text_secondary: Color,
    /// `AccentTextFillColorTertiary`: o texto do botão de link pressionado.
    pub accent_text_tertiary: Color,
    /// `AccentTextFillColorDisabled`.
    pub accent_text_disabled: Color,
    /// `ControlSolidFillColorDefault`: o polegar do `Slider`.
    pub control_solid_fill_default: Color,
    /// `CardBackgroundFillColorDefault`: o cabeçalho do `Expander`.
    pub card_background_default: Color,
    /// `CardBackgroundFillColorSecondary`: o conteúdo do `Expander`.
    pub card_background_secondary: Color,
    /// `CardBackgroundFillColorTertiary`.
    pub card_background_tertiary: Color,
    /// `LayerFillColorDefault`: camada sobre o fundo da janela, como a área de conteúdo de
    /// uma página.
    pub layer_fill_default: Color,
    /// `SolidBackgroundFillColorTertiary`: a aba escolhida e o conteúdo do `TabView`.
    pub solid_background_tertiary: Color,
    /// `LayerOnMicaBaseAltFillColorSecondary`: aba com hover.
    pub layer_on_mica_base_alt_secondary: Color,
    /// `LayerOnMicaBaseAltFillColorDefault`: aba pressionada.
    pub layer_on_mica_base_alt_default: Color,
}

const fn argb(argb: u32) -> Color {
    let [a, r, g, b] = argb.to_be_bytes();
    Color::from_rgba8(r, g, b, a)
}

impl Tokens {
    /// Tokens do tema pedido, com a accent padrão do Windows.
    pub const fn new(theme: Theme) -> Self {
        match theme {
            Theme::Light => Self::LIGHT,
            Theme::Dark => Self::DARK,
        }
    }

    /// Valor do token `token` neste tema.
    pub fn get(&self, token: Token) -> Color {
        match token {
            Token::TextFillColorPrimary => self.text_fill_primary,
            Token::TextFillColorSecondary => self.text_fill_secondary,
            Token::TextFillColorTertiary => self.text_fill_tertiary,
            Token::TextFillColorDisabled => self.text_fill_disabled,
            Token::TextOnAccentFillColorPrimary => self.text_on_accent_primary,
            Token::AccentTextFillColorPrimary => self.accent_text_primary,
            Token::AccentFillColorDefault => self.accent_fill_default,
            Token::ControlFillColorDefault => self.control_fill_default,
            Token::ControlFillColorSecondary => self.control_fill_secondary,
            Token::ControlFillColorTertiary => self.control_fill_tertiary,
            Token::ControlStrokeColorDefault => self.control_stroke_default,
            Token::CardBackgroundFillColorDefault => self.card_background_default,
            Token::CardBackgroundFillColorSecondary => self.card_background_secondary,
            Token::CardBackgroundFillColorTertiary => self.card_background_tertiary,
            Token::CardStrokeColorDefault => self.card_stroke_default,
            Token::LayerFillColorDefault => self.layer_fill_default,
            Token::LayerFillColorAlt => self.layer_fill_alt,
            Token::SolidBackgroundFillColorBase => self.solid_background_base,
            Token::SolidBackgroundFillColorTertiary => self.solid_background_tertiary,
            Token::SubtleFillColorSecondary => self.subtle_fill_secondary,
            Token::SubtleFillColorTertiary => self.subtle_fill_tertiary,
            Token::DividerStrokeColorDefault => self.divider_stroke_default,
            Token::SurfaceStrokeColorDefault => self.surface_stroke_default,
            Token::SystemFillColorSuccess => self.info_bar_success_icon,
            Token::SystemFillColorCaution => self.info_bar_warning_icon,
            Token::SystemFillColorCritical => self.info_bar_error_icon,
            // Sem o XAML do WinUI à mão para conferir, vale a cor do acento (a do `InfoBadge`
            // de atenção). Registrado na Entrega do M49.
            Token::SystemFillColorAttention => self.accent_fill_default,
            Token::SystemFillColorSuccessBackground => self.info_bar_success_background,
            Token::SystemFillColorCautionBackground => self.info_bar_warning_background,
            Token::SystemFillColorCriticalBackground => self.info_bar_error_background,
        }
    }

    /// Cor de `color`: o token neste tema ou a cor fixa.
    pub fn resolve(&self, color: ThemeColor) -> Color {
        match color {
            ThemeColor::Token(token) => self.get(token),
            ThemeColor::Fixed(c) => Color::from_rgba8(c.r, c.g, c.b, c.a),
            ThemeColor::Themed { light, dark } => {
                let c = if self.theme == Theme::Dark {
                    dark
                } else {
                    light
                };
                Color::from_rgba8(c.r, c.g, c.b, c.a)
            }
        }
    }

    /// Tema claro.
    pub const LIGHT: Self = Self {
        theme: Theme::Light,
        solid_background_base: argb(0xFFF3F3F3),
        control_fill_default: argb(0xB3FFFFFF),
        control_fill_secondary: argb(0x80F9F9F9),
        control_fill_tertiary: argb(0x4DF9F9F9),
        control_fill_disabled: argb(0x4DF9F9F9),
        control_fill_input_active: argb(0xFFFFFFFF),
        control_stroke_default: argb(0x0F000000),
        control_stroke_secondary: argb(0x29000000),
        control_strong_stroke_default: argb(0x72000000),
        control_stroke_on_accent_default: argb(0x14FFFFFF),
        control_stroke_on_accent_secondary: argb(0x66000000),
        text_fill_primary: argb(0xE4000000),
        text_fill_secondary: argb(0x9E000000),
        text_fill_disabled: argb(0x5C000000),
        text_on_accent_primary: argb(0xFFFFFFFF),
        text_on_accent_secondary: argb(0xB3FFFFFF),
        text_on_accent_disabled: argb(0xFFFFFFFF),
        accent_fill_default: argb(0xFF005FB8),
        accent_fill_secondary: argb(0xFF005FB8).with_alpha(0.9),
        accent_fill_tertiary: argb(0xFF005FB8).with_alpha(0.8),
        accent_fill_disabled: argb(0x37000000),
        accent_selected_text_background: argb(0xFF0078D4),
        text_on_accent_selected_text: argb(0xFFFFFFFF),
        focus_stroke_outer: argb(0xE4000000),
        focus_stroke_inner: argb(0xB3FFFFFF),
        control_alt_fill_secondary: argb(0x06000000),
        control_alt_fill_tertiary: argb(0x0F000000),
        control_alt_fill_quarternary: argb(0x18000000),
        control_alt_fill_disabled: argb(0x00FFFFFF),
        control_strong_stroke_disabled: argb(0x37000000),
        card_stroke_default: argb(0x0F000000),
        subtle_fill_secondary: argb(0x09000000),
        subtle_fill_tertiary: argb(0x06000000),
        text_fill_inverse: argb(0xFFFFFFFF),
        info_bar_informational_background: argb(0x80F6F6F6),
        info_bar_success_background: argb(0xFFDFF6DD),
        info_bar_warning_background: argb(0xFFFFF4CE),
        info_bar_error_background: argb(0xFFFDE7E9),
        info_bar_success_icon: argb(0xFF0F7B0F),
        info_bar_warning_icon: argb(0xFF9D5D00),
        info_bar_error_icon: argb(0xFFC42B1C),
        control_strong_fill_default: argb(0x72000000),
        control_strong_fill_disabled: argb(0x51000000),
        acrylic_in_app_fill_default: argb(0xFFF9F9F9),
        smoke_fill_default: argb(0x4D000000),
        surface_stroke_flyout: argb(0x0F000000),
        surface_stroke_default: argb(0x66757575),
        divider_stroke_default: argb(0x0F000000),
        layer_fill_alt: argb(0xFFFFFFFF),
        text_fill_tertiary: argb(0x72000000),
        accent_text_primary: argb(0xFF003E92),
        accent_text_secondary: argb(0xFF001A68),
        accent_text_tertiary: argb(0xFF005FB8),
        accent_text_disabled: argb(0x5C000000),
        control_solid_fill_default: argb(0xFFFFFFFF),
        card_background_default: argb(0xB3FFFFFF),
        card_background_secondary: argb(0x80F6F6F6),
        card_background_tertiary: argb(0xFFFFFFFF),
        layer_fill_default: argb(0x80FFFFFF),
        solid_background_tertiary: argb(0xFFF9F9F9),
        layer_on_mica_base_alt_secondary: argb(0x0A000000),
        layer_on_mica_base_alt_default: argb(0xB3FFFFFF),
    };

    /// Tema escuro.
    pub const DARK: Self = Self {
        theme: Theme::Dark,
        solid_background_base: argb(0xFF202020),
        control_fill_default: argb(0x0FFFFFFF),
        control_fill_secondary: argb(0x15FFFFFF),
        control_fill_tertiary: argb(0x08FFFFFF),
        control_fill_disabled: argb(0x0BFFFFFF),
        control_fill_input_active: argb(0xB31E1E1E),
        control_stroke_default: argb(0x12FFFFFF),
        control_stroke_secondary: argb(0x18FFFFFF),
        control_strong_stroke_default: argb(0x8BFFFFFF),
        control_stroke_on_accent_default: argb(0x14FFFFFF),
        control_stroke_on_accent_secondary: argb(0x23000000),
        text_fill_primary: argb(0xFFFFFFFF),
        text_fill_secondary: argb(0xC5FFFFFF),
        text_fill_disabled: argb(0x5DFFFFFF),
        text_on_accent_primary: argb(0xFF000000),
        text_on_accent_secondary: argb(0x80000000),
        text_on_accent_disabled: argb(0x87FFFFFF),
        accent_fill_default: argb(0xFF60CDFF),
        accent_fill_secondary: argb(0xFF60CDFF).with_alpha(0.9),
        accent_fill_tertiary: argb(0xFF60CDFF).with_alpha(0.8),
        accent_fill_disabled: argb(0x28FFFFFF),
        accent_selected_text_background: argb(0xFF0078D4),
        text_on_accent_selected_text: argb(0xFFFFFFFF),
        focus_stroke_outer: argb(0xFFFFFFFF),
        focus_stroke_inner: argb(0xB3000000),
        control_alt_fill_secondary: argb(0x19000000),
        control_alt_fill_tertiary: argb(0x0BFFFFFF),
        control_alt_fill_quarternary: argb(0x12FFFFFF),
        control_alt_fill_disabled: argb(0x00FFFFFF),
        control_strong_stroke_disabled: argb(0x28FFFFFF),
        card_stroke_default: argb(0x19000000),
        subtle_fill_secondary: argb(0x0FFFFFFF),
        subtle_fill_tertiary: argb(0x0AFFFFFF),
        text_fill_inverse: argb(0xE4000000),
        info_bar_informational_background: argb(0x08FFFFFF),
        info_bar_success_background: argb(0xFF393D1B),
        info_bar_warning_background: argb(0xFF433519),
        info_bar_error_background: argb(0xFF442726),
        info_bar_success_icon: argb(0xFF6CCB5F),
        info_bar_warning_icon: argb(0xFFFCE100),
        info_bar_error_icon: argb(0xFFFF99A4),
        control_strong_fill_default: argb(0x8BFFFFFF),
        control_strong_fill_disabled: argb(0x3FFFFFFF),
        acrylic_in_app_fill_default: argb(0xFF2C2C2C),
        smoke_fill_default: argb(0x4D000000),
        surface_stroke_flyout: argb(0x33000000),
        surface_stroke_default: argb(0x66757575),
        divider_stroke_default: argb(0x15FFFFFF),
        layer_fill_alt: argb(0x0DFFFFFF),
        text_fill_tertiary: argb(0x87FFFFFF),
        accent_text_primary: argb(0xFF99EBFF),
        accent_text_secondary: argb(0xFF99EBFF),
        accent_text_tertiary: argb(0xFF60CDFF),
        accent_text_disabled: argb(0x5DFFFFFF),
        control_solid_fill_default: argb(0xFF454545),
        card_background_default: argb(0x0DFFFFFF),
        card_background_secondary: argb(0x08FFFFFF),
        card_background_tertiary: argb(0x12FFFFFF),
        layer_fill_default: argb(0x4C3A3A3A),
        solid_background_tertiary: argb(0xFF282828),
        layer_on_mica_base_alt_secondary: argb(0x0FFFFFFF),
        layer_on_mica_base_alt_default: argb(0x733A3A3A),
    };
}

/// Medidas comuns aos controles.
pub mod metrics {
    /// `ControlCornerRadius`.
    pub const CONTROL_CORNER_RADIUS: f64 = 4.0;
    /// Altura padrão de controle.
    pub const CONTROL_HEIGHT: f64 = 32.0;
    /// Espessura da borda dos controles.
    pub const BORDER: f64 = 1.0;
    /// Altura do gradiente da borda de elevação (`ControlElevationBorderBrush`).
    pub const ELEVATION_BORDER_HEIGHT: f64 = 3.0;
    /// Onde começa a transição do gradiente de elevação, de 0 a 1.
    pub const ELEVATION_BORDER_STOP: f32 = 0.33;
    /// `ButtonPadding` (esquerda, topo, direita, base), dentro da borda.
    pub const BUTTON_PADDING: (f64, f64, f64, f64) = (11.0, 5.0, 11.0, 6.0);
    /// O anel de foco fica até 3 px fora do controle (`FocusVisualMargin="-3"`).
    pub const FOCUS_MARGIN: f64 = 3.0;
    /// Espessura do anel de foco externo (`FocusVisualPrimaryThickness`).
    pub const FOCUS_OUTER: f64 = 2.0;
    /// Espessura do anel de foco interno (`FocusVisualSecondaryThickness`).
    pub const FOCUS_INNER: f64 = 1.0;
    /// `TextControlThemePadding` (esquerda, topo, direita, base), dentro da borda.
    pub const TEXT_CONTROL_PADDING: (f64, f64, f64, f64) = (10.0, 5.0, 6.0, 6.0);
    /// `TextControlThemePadding` do `Compact.xaml` (densidade compacta).
    pub const TEXT_CONTROL_PADDING_COMPACT: (f64, f64, f64, f64) = (2.0, 2.0, 6.0, 1.0);
    /// `TextControlThemeMinHeight` do `Compact.xaml`: a altura dos campos e do `ComboBox` na
    /// densidade compacta.
    pub const CONTROL_HEIGHT_COMPACT: f64 = 24.0;
    /// `TextControlThemeMinWidth`.
    pub const TEXT_CONTROL_MIN_WIDTH: f64 = 64.0;
    /// Linha de baixo do campo de texto com foco (`TextControlBorderThemeThicknessFocused`).
    pub const TEXT_CONTROL_FOCUSED_BOTTOM: f64 = 2.0;
    /// Altura do gradiente da borda do campo de texto (`TextControlElevationBorderBrush`), a
    /// partir da base.
    pub const TEXT_CONTROL_BORDER_HEIGHT: f64 = 2.0;
    /// Largura do cursor de texto.
    pub const CARET_WIDTH: f64 = 1.0;
    /// Distância do topo da linha ao sublinhado da composição do IME em 14/20: 1 px abaixo da
    /// base dos glifos da Noto Sans JP, que descem um pouco além da linha de base da Selawik.
    pub const COMPOSITION_UNDERLINE: f64 = 18.0;
}
