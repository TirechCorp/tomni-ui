//! Esquemas de cor: as 16 cores do ANSI, o texto, o fundo, o cursor e a seleção, com os
//! valores dos esquemas de mesmo nome do Windows Terminal (licença MIT).

use tomni_ui_core::Color as Rgba;
use tomni_ui_core::{ThemeColor, Token};

use crate::cell::{Cell, Color, Flags};

/// As cores de um esquema.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Palette {
    /// Fundo padrão.
    pub background: Rgba,
    /// Texto padrão.
    pub foreground: Rgba,
    /// Cor do cursor.
    pub cursor: Rgba,
    /// Cor pintada por cima do texto selecionado, translúcida.
    pub selection: Rgba,
    /// As 16 cores do ANSI: preto, vermelho, verde, amarelo, azul, roxo, ciano e branco, e as
    /// mesmas brilhantes.
    pub ansi: [Rgba; 16],
}

/// O "Campbell", o esquema padrão.
pub const CAMPBELL: Palette = Palette {
    background: Rgba::hex(0x0C0C0C),
    foreground: Rgba::hex(0xCCCCCC),
    cursor: Rgba::hex(0xFFFFFF),
    // Seleção branca, pintada com metade da opacidade.
    selection: Rgba::rgba(0xFF, 0xFF, 0xFF, 0x80),
    ansi: [
        Rgba::hex(0x0C0C0C),
        Rgba::hex(0xC50F1F),
        Rgba::hex(0x13A10E),
        Rgba::hex(0xC19C00),
        Rgba::hex(0x0037DA),
        Rgba::hex(0x881798),
        Rgba::hex(0x3A96DD),
        Rgba::hex(0xCCCCCC),
        Rgba::hex(0x767676),
        Rgba::hex(0xE74856),
        Rgba::hex(0x16C60C),
        Rgba::hex(0xF9F1A5),
        Rgba::hex(0x3B78FF),
        Rgba::hex(0xB4009E),
        Rgba::hex(0x61D6D6),
        Rgba::hex(0xF2F2F2),
    ],
};

/// O "One Half Light", usado no tema claro com [`Scheme::Theme`].
///
/// Adaptação: a seleção é o texto a 25% em vez do `selectionBackground` branco do Windows
/// Terminal, que quase não aparece sobre o fundo `#FAFAFA`.
pub const ONE_HALF_LIGHT: Palette = Palette {
    background: Rgba::hex(0xFAFAFA),
    foreground: Rgba::hex(0x383A42),
    cursor: Rgba::hex(0x4F525D),
    selection: Rgba::rgba(0x38, 0x3A, 0x42, 0x40),
    ansi: [
        Rgba::hex(0x383A42),
        Rgba::hex(0xE45649),
        Rgba::hex(0x50A14F),
        Rgba::hex(0xC18301),
        Rgba::hex(0x0184BC),
        Rgba::hex(0xA626A4),
        Rgba::hex(0x0997B3),
        Rgba::hex(0xFAFAFA),
        Rgba::hex(0x4F525D),
        Rgba::hex(0xDF6C75),
        Rgba::hex(0x98C379),
        Rgba::hex(0xE4C07A),
        Rgba::hex(0x61AFEF),
        Rgba::hex(0xC577DD),
        Rgba::hex(0x56B5C1),
        Rgba::hex(0xFFFFFF),
    ],
};

/// De onde vêm as cores do terminal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Scheme {
    /// O "Campbell": fundo quase preto nos dois temas.
    #[default]
    Campbell,
    /// O fundo (`SolidBackgroundFillColorBase`) e o texto (`TextFillColorPrimary`) do tema do
    /// Tomni UI, com as cores do Campbell no escuro e as do "One Half Light" no claro.
    Theme,
    /// Cores próprias.
    Custom(Palette),
}

impl Scheme {
    /// As cores no tema `dark`, com o fundo e o texto do tema para o [`Scheme::Theme`].
    pub fn palette(self, dark: bool, background: Rgba, foreground: Rgba) -> Palette {
        match self {
            Self::Campbell => CAMPBELL,
            Self::Custom(palette) => palette,
            Self::Theme => {
                let base = if dark { CAMPBELL } else { ONE_HALF_LIGHT };
                Palette {
                    background,
                    foreground,
                    cursor: foreground,
                    ..base
                }
            }
        }
    }

    /// O fundo e o texto do esquema como cores do Tomni UI: as do tema no [`Scheme::Theme`], as
    /// da paleta nos outros. É o que pinta a aba escolhida dos
    /// [`TerminalPanes`](crate::TerminalPanes) (`.tab_scheme`).
    pub fn tab_colors(self) -> (ThemeColor, ThemeColor) {
        match self {
            Self::Theme => (
                ThemeColor::Token(Token::SolidBackgroundFillColorBase),
                ThemeColor::Token(Token::TextFillColorPrimary),
            ),
            Self::Campbell => (
                ThemeColor::Fixed(CAMPBELL.background),
                ThemeColor::Fixed(CAMPBELL.foreground),
            ),
            Self::Custom(palette) => (
                ThemeColor::Fixed(palette.background),
                ThemeColor::Fixed(palette.foreground),
            ),
        }
    }

    /// Nome para a árvore e o lint (`campbell`, `theme` ou `custom`).
    pub fn name(self) -> &'static str {
        match self {
            Self::Campbell => "campbell",
            Self::Theme => "theme",
            Self::Custom(_) => "custom",
        }
    }
}

/// Os seis níveis do cubo de cores do xterm (16 a 231).
const CUBE: [u8; 6] = [0x00, 0x5F, 0x87, 0xAF, 0xD7, 0xFF];

/// Cor `index` da paleta de 256 do xterm: as 16 do esquema, o cubo 6×6×6 e 24 cinzas.
pub fn indexed(palette: &Palette, index: u8) -> Rgba {
    match index {
        0..=15 => palette.ansi[usize::from(index)],
        16..=231 => {
            let i = index - 16;
            Rgba::rgb(
                CUBE[usize::from(i / 36)],
                CUBE[usize::from(i / 6 % 6)],
                CUBE[usize::from(i % 6)],
            )
        }
        232..=255 => {
            let level = 8 + 10 * (index - 232);
            Rgba::rgb(level, level, level)
        }
    }
}

/// Mistura `a` e `b` meio a meio.
fn half(a: Rgba, b: Rgba) -> Rgba {
    let mix = |x: u8, y: u8| ((u16::from(x) + u16::from(y)) / 2) as u8;
    Rgba::rgb(mix(a.r, b.r), mix(a.g, b.g), mix(a.b, b.b))
}

/// Cores com que a célula é pintada.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Resolved {
    /// Texto.
    pub fg: Rgba,
    /// Fundo; `None` quando é o fundo padrão (já pintado embaixo da grade).
    pub bg: Option<Rgba>,
}

impl Palette {
    /// As cores da célula, como o xterm pinta:
    ///
    /// - o negrito com uma das 8 primeiras cores usa a brilhante correspondente (a
    ///   fonte também vai para negrito);
    /// - o apagado (SGR 2) mistura o texto meio a meio com o fundo;
    /// - o inverso (SGR 7) troca o texto e o fundo, e o `reverse_video` (DECSCNM) troca de
    ///   novo;
    /// - o escondido (SGR 8) pinta o texto na cor do fundo.
    pub fn resolve(&self, cell: &Cell, reverse_video: bool) -> Resolved {
        let flags = cell.flags;
        let mut fg = match cell.fg {
            Color::Default => self.foreground,
            Color::Indexed(i) if i < 8 && flags.contains(Flags::BOLD) => {
                self.ansi[usize::from(i) + 8]
            }
            Color::Indexed(i) => indexed(self, i),
            Color::Rgb(r, g, b) => Rgba::rgb(r, g, b),
        };
        let mut bg = match cell.bg {
            Color::Default => None,
            Color::Indexed(i) => Some(indexed(self, i)),
            Color::Rgb(r, g, b) => Some(Rgba::rgb(r, g, b)),
        };
        if flags.contains(Flags::DIM) {
            fg = half(fg, bg.unwrap_or(self.background));
        }
        if flags.contains(Flags::INVERSE) != reverse_video {
            let back = bg.unwrap_or(self.background);
            bg = Some(fg);
            fg = back;
        }
        if flags.contains(Flags::HIDDEN) {
            fg = bg.unwrap_or(self.background);
        }
        Resolved { fg, bg }
    }

    /// O fundo da grade: o padrão, ou o texto padrão com o `reverse_video` (DECSCNM).
    pub fn screen_background(&self, reverse_video: bool) -> Rgba {
        if reverse_video {
            self.foreground
        } else {
            self.background
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::Pen;

    fn cell(fg: Color, bg: Color, flags: Flags) -> Cell {
        Cell::with('x', &Pen { fg, bg, flags })
    }

    #[test]
    fn paleta_de_256() {
        assert_eq!(indexed(&CAMPBELL, 1), Rgba::hex(0xC50F1F));
        assert_eq!(indexed(&CAMPBELL, 16), Rgba::hex(0x000000));
        assert_eq!(indexed(&CAMPBELL, 196), Rgba::hex(0xFF0000));
        assert_eq!(indexed(&CAMPBELL, 231), Rgba::hex(0xFFFFFF));
        assert_eq!(indexed(&CAMPBELL, 232), Rgba::hex(0x080808));
        assert_eq!(indexed(&CAMPBELL, 255), Rgba::hex(0xEEEEEE));
    }

    #[test]
    fn negrito_brilha_e_inverso_troca() {
        let p = CAMPBELL;
        let bold = p.resolve(&cell(Color::Indexed(1), Color::Default, Flags::BOLD), false);
        assert_eq!(bold.fg, Rgba::hex(0xE74856));
        let inv = p.resolve(&cell(Color::Default, Color::Default, Flags::INVERSE), false);
        assert_eq!(
            inv,
            Resolved {
                fg: p.background,
                bg: Some(p.foreground)
            }
        );
        let normal = p.resolve(&cell(Color::Default, Color::Default, Flags::INVERSE), true);
        assert_eq!(normal.fg, p.foreground);
        let hidden = p.resolve(
            &cell(Color::Default, Color::Indexed(4), Flags::HIDDEN),
            false,
        );
        assert_eq!(hidden.fg, Rgba::hex(0x0037DA));
        let dim = p.resolve(
            &cell(Color::Rgb(200, 200, 200), Color::Default, Flags::DIM),
            false,
        );
        assert_eq!(dim.fg, Rgba::rgb(106, 106, 106));
    }

    #[test]
    fn esquema_do_tema() {
        let bg = Rgba::hex(0x202020);
        let fg = Rgba::hex(0xFFFFFF);
        let dark = Scheme::Theme.palette(true, bg, fg);
        assert_eq!((dark.background, dark.foreground), (bg, fg));
        assert_eq!(dark.ansi, CAMPBELL.ansi);
        let light = Scheme::Theme.palette(false, bg, fg);
        assert_eq!(light.ansi, ONE_HALF_LIGHT.ansi);
        assert_eq!(Scheme::Campbell.palette(false, bg, fg), CAMPBELL);
    }
}
