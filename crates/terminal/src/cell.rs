//! A célula da grade: o caractere, as cores e os atributos.

use std::num::NonZeroU32;

/// Cor de uma célula.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Color {
    /// A cor padrão do esquema (o texto ou o fundo).
    #[default]
    Default,
    /// Uma das 256 cores da paleta: 0 a 15 vêm do esquema, 16 a 255 são o cubo e os cinzas
    /// do xterm.
    Indexed(u8),
    /// Uma cor exata (`ESC[38;2;r;g;bm`).
    Rgb(u8, u8, u8),
}

/// Atributos de uma célula, combinados com `|`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Flags(u16);

impl Flags {
    /// Nenhum atributo.
    pub const NONE: Self = Self(0);
    /// Negrito (SGR 1).
    pub const BOLD: Self = Self(1);
    /// Apagado (SGR 2).
    pub const DIM: Self = Self(1 << 1);
    /// Itálico (SGR 3).
    pub const ITALIC: Self = Self(1 << 2);
    /// Sublinhado (SGR 4).
    pub const UNDERLINE: Self = Self(1 << 3);
    /// Piscando (SGR 5). Desenhado como texto comum.
    pub const BLINK: Self = Self(1 << 4);
    /// Cores trocadas (SGR 7).
    pub const INVERSE: Self = Self(1 << 5);
    /// Escondido (SGR 8): só o fundo.
    pub const HIDDEN: Self = Self(1 << 6);
    /// Tachado (SGR 9).
    pub const STRIKE: Self = Self(1 << 7);
    /// Sublinhado duplo (SGR 21).
    pub const DOUBLE_UNDERLINE: Self = Self(1 << 8);
    /// Primeira metade de um caractere largo (CJK): a célula seguinte é o
    /// [`Flags::WIDE_SPACER`].
    pub const WIDE: Self = Self(1 << 9);
    /// Segunda metade de um caractere largo: não tem caractere próprio.
    pub const WIDE_SPACER: Self = Self(1 << 10);
    /// Última coluna deixada em branco porque o caractere largo não coube nela e foi para a
    /// linha de baixo. Some ao refluir.
    pub const WIDE_PAD: Self = Self(1 << 11);

    /// Atributos que vêm do SGR (os outros são da grade).
    pub const SGR: Self = Self(0x1FF);

    /// Se tem todos os atributos de `other`.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Se tem algum dos atributos de `other`.
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    /// Liga ou desliga `other`.
    pub fn set(&mut self, other: Self, on: bool) {
        if on {
            self.0 |= other.0;
        } else {
            self.0 &= !other.0;
        }
    }

    /// Os bits.
    pub const fn bits(self) -> u16 {
        self.0
    }
}

impl std::ops::BitOr for Flags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for Flags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl std::ops::BitAnd for Flags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}

impl std::ops::Not for Flags {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

/// Cores e atributos com que o texto novo é escrito (a "caneta" do SGR).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Pen {
    /// Cor do texto.
    pub fg: Color,
    /// Cor do fundo.
    pub bg: Color,
    /// Atributos do SGR.
    pub flags: Flags,
}

/// Uma célula da grade.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Cell {
    /// O caractere; espaço nas células vazias e na segunda metade dos largos.
    pub c: char,
    /// Cor do texto.
    pub fg: Color,
    /// Cor do fundo.
    pub bg: Color,
    /// Atributos.
    pub flags: Flags,
    /// Acentos combinados depois do caractere, se houver.
    pub combining: Option<Box<String>>,
    /// O link (OSC 8) a que a célula pertence: o índice mais um na tabela do emulador. Não é
    /// atributo de desenho; quem lê o destino é o [`Emulator::link`](crate::Emulator::link).
    pub link: Option<NonZeroU32>,
}

impl Default for Cell {
    fn default() -> Self {
        Self::BLANK
    }
}

impl Cell {
    /// A célula vazia com as cores padrão.
    pub const BLANK: Self = Self {
        c: ' ',
        fg: Color::Default,
        bg: Color::Default,
        flags: Flags::NONE,
        combining: None,
        link: None,
    };

    /// Célula vazia com o fundo de `pen` (o "background color erase" do xterm).
    pub fn erased(pen: &Pen) -> Self {
        Self {
            bg: pen.bg,
            ..Self::BLANK
        }
    }

    /// `c` escrito com `pen`.
    pub fn with(c: char, pen: &Pen) -> Self {
        Self {
            c,
            fg: pen.fg,
            bg: pen.bg,
            flags: pen.flags,
            combining: None,
            link: None,
        }
    }

    /// Se é uma célula vazia, sem nada que apareça: espaço, cores padrão, sem atributos.
    pub fn is_blank(&self) -> bool {
        self.c == ' '
            && self.bg == Color::Default
            && self.combining.is_none()
            && !self
                .flags
                .intersects(Flags::INVERSE | Flags::UNDERLINE | Flags::STRIKE | Flags::WIDE)
    }

    /// O texto da célula: o caractere e os acentos combinados. Vazio na segunda metade dos
    /// largos.
    pub fn text(&self) -> String {
        if self.flags.contains(Flags::WIDE_SPACER) {
            return String::new();
        }
        let mut text = String::from(self.c);
        if let Some(combining) = &self.combining {
            text.push_str(combining);
        }
        text
    }

    /// Junta um acento combinado ao caractere.
    pub fn push_combining(&mut self, c: char) {
        self.combining.get_or_insert_with(Default::default).push(c);
    }
}

/// Quantas células `c` ocupa na grade: 2 para os largos (CJK, emoji), 0 para os acentos
/// combinados e os controles, 1 para o resto. É a conta do emulador.
pub fn char_width(c: char) -> usize {
    use unicode_width::UnicodeWidthChar;
    if (c as u32) < 0x7f {
        usize::from(c >= ' ')
    } else {
        c.width().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn celula_pequena() {
        assert!(
            std::mem::size_of::<Cell>() <= 32,
            "{}",
            std::mem::size_of::<Cell>()
        );
    }

    #[test]
    fn apagar_usa_o_fundo_da_caneta() {
        let pen = Pen {
            fg: Color::Indexed(1),
            bg: Color::Indexed(4),
            flags: Flags::BOLD,
        };
        let cell = Cell::erased(&pen);
        assert_eq!(cell.bg, Color::Indexed(4));
        assert_eq!(cell.fg, Color::Default);
        assert_eq!(cell.flags, Flags::NONE);
        assert!(!cell.is_blank());
        assert!(Cell::BLANK.is_blank());
    }

    #[test]
    fn acentos_combinados_ficam_na_celula() {
        let mut cell = Cell::with('e', &Pen::default());
        cell.push_combining('\u{301}');
        assert_eq!(cell.text(), "e\u{301}");
    }
}
