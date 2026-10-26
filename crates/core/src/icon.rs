//! `Icon`: um ícone dos Fluent System Icons, na cor do texto.

use std::fmt;

use crate::color::ThemeColor;
use crate::prop::{IntoProp, Prop};
use crate::symbol::Symbol;
use crate::widget::{Common, Look, Widget, common_builders};

/// Lado padrão do [`Icon`], em pixels.
pub const ICON_SIZE: u32 = 20;

/// Tudo o que o `Drawer` precisa saber do `Icon` para medi-lo e pintá-lo.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IconVisual {
    /// O ícone.
    pub symbol: Symbol,
    /// Lado do quadrado, em pixels.
    pub size: u32,
    /// Cor. `None` é a herdada (a de um `Card` pressionado ou desabilitado em volta) ou, sem
    /// ela, o `TextFillColorPrimary`.
    pub color: Option<ThemeColor>,
    /// Só enfeite: o lint não confere o contraste.
    pub decorative: bool,
}

/// Um ícone dos Fluent System Icons, num quadrado de 12, 16, 20 (o padrão) ou 24 px.
///
/// ```
/// use tomni_ui_core::{Icon, Symbol, Token};
///
/// let casa = Icon::new(Symbol::Home).id("casa");
/// assert_eq!(casa.visual().size, 20);
/// let aviso = Icon::new(Symbol::Warning).size(16).color(Token::SystemFillColorCaution);
/// assert_eq!(aviso.visual().size, 16);
/// ```
///
/// A cor padrão é a do texto, como o `Foreground` herdado do `FontIcon` do WinUI. Os desenhos
/// são os do Fluent em cada tamanho; num tamanho que o Fluent não desenhou para aquele ícone, o
/// mais próximo é escalado. Como um ícone passa informação, o lint confere o contraste de 3:1
/// com o fundo (WCAG 1.4.11), exceto com [`Icon::decorative`]. Não recebe foco nem cliques: um
/// ícone clicável é um `Button::icon_only`.
pub struct Icon {
    common: Common,
    symbol: Prop<Symbol>,
    size: u32,
    color: Option<ThemeColor>,
    decorative: bool,
}

impl Icon {
    /// O ícone `symbol` (fixo, closure ou signal) com [`ICON_SIZE`] de lado.
    pub fn new<M>(symbol: impl IntoProp<Symbol, M>) -> Self {
        Self {
            common: Common::default(),
            symbol: symbol.into_prop(),
            size: ICON_SIZE,
            color: None,
            decorative: false,
        }
    }

    /// Lado do quadrado: 12, 16, 20 ou 24 px, os tamanhos que o Fluent desenha. Outros
    /// valores escalam o desenho mais próximo.
    pub fn size(mut self, size: u32) -> Self {
        self.size = size.max(1);
        self
    }

    /// Cor: um [`Token`](crate::Token) do tema ou uma [`Color`](crate::Color) fixa. Sem ela, a
    /// do texto.
    pub fn color(mut self, color: impl Into<ThemeColor>) -> Self {
        self.color = Some(color.into());
        self
    }

    /// Só enfeite, sem informação que falte no texto ao lado: o lint não confere o contraste.
    pub fn decorative(mut self) -> Self {
        self.decorative = true;
        self
    }

    /// Ícone atual.
    pub fn symbol(&self) -> Symbol {
        self.symbol.get()
    }

    /// Estado para o `Drawer`.
    pub fn visual(&self) -> IconVisual {
        IconVisual {
            symbol: self.symbol(),
            size: self.size,
            color: self.color,
            decorative: self.decorative,
        }
    }
}

common_builders!(Icon);

impl Widget for Icon {
    fn kind(&self) -> &'static str {
        "icon"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::Icon(self.visual())
    }
}

impl fmt::Debug for Icon {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Icon")
            .field("common", &self.common)
            .field("symbol", &self.symbol)
            .field("size", &self.size)
            .field("color", &self.color)
            .field("decorative", &self.decorative)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use tomni_ui_reactive::prelude::*;

    use super::*;
    use crate::color::Token;

    #[test]
    fn builders_e_look() {
        let icone = Icon::new(Symbol::Home)
            .id("casa")
            .size(16)
            .color(Token::AccentTextFillColorPrimary)
            .decorative();
        assert_eq!(icone.common().id.as_deref(), Some("casa"));
        assert_eq!(
            icone.look(),
            Look::Icon(IconVisual {
                symbol: Symbol::Home,
                size: 16,
                color: Some(Token::AccentTextFillColorPrimary.into()),
                decorative: true,
            })
        );
        assert!(!icone.interactive());
        assert!(!icone.focusable());
        assert_eq!(Icon::new(Symbol::Add).size(0).visual().size, 1);
    }

    #[test]
    fn simbolo_reativo() {
        let scope = Scope::root();
        scope.run(|| {
            let tocando = signal(false);
            let icone = Icon::new(move || {
                if tocando.get() {
                    Symbol::Pause
                } else {
                    Symbol::Play
                }
            });
            assert_eq!(icone.symbol(), Symbol::Play);
            tocando.set(true);
            assert_eq!(icone.symbol(), Symbol::Pause);
        });
        scope.clear();
    }
}
