//! Densidade dos controles: a padrão, com alvos de toque de 44 px, e a compacta do
//! `Compact.xaml` do WinUI, para telas cheias de controles (um gerenciador de conexões, um
//! painel de propriedades).

/// Densidade de uma subárvore. Um contêiner com `.density(Density::Compact)` (`Form`, `Column`,
/// `Row`, `Card`, `UniformGrid` ou `ScrollViewer`) vale para tudo o que está dentro dele, até
/// outro contêiner trocar de novo.
///
/// Na compacta, os controles seguem o `Compact.xaml` do WinUI 3: campos e `ComboBox` de 24 px,
/// itens de menu, de lista e do `NavigationView` de 32, abas de 24. O `Button`, o `CheckBox`, o
/// `ToggleSwitch`, o `Slider` e o `Expander` não mudam. O alvo de toque mínimo que o lint
/// confere cai de 44×44 para 24×24 ([`Density::min_target`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Density {
    /// Controles com alvos de toque de 44 px.
    #[default]
    Standard,
    /// Controles do `Compact.xaml` do WinUI, para uso com mouse e teclado.
    Compact,
}

impl Density {
    /// Nome que a árvore do inspetor mostra: `standard` ou `compact`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Compact => "compact",
        }
    }

    /// Menor lado do alvo de toque que o lint aceita: 44 na padrão e 24 na compacta (a
    /// `TextControlThemeMinHeight` do `Compact.xaml`).
    pub fn min_target(self) -> f64 {
        match self {
            Self::Standard => 44.0,
            Self::Compact => 24.0,
        }
    }

    /// Se é a compacta.
    pub fn is_compact(self) -> bool {
        self == Self::Compact
    }
}
