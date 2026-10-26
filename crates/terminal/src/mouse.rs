//! Eventos do mouse como bytes, no relato do mouse do xterm.
//!
//! O programa liga o relato com os modos do [`MouseTracking`] e escolhe a forma das posições
//! com o [`MouseEncoding`] (os dois ficam no [`Modes`] do emulador). Aqui fica só a
//! tradução de um evento em bytes; quem decide quando há evento é o widget.

use tomni_ui_core::Modifiers;

use crate::emulator::{Modes, MouseEncoding, MouseTracking};

/// O botão de um evento do mouse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    /// O esquerdo (o principal).
    Left,
    /// O do meio.
    Middle,
    /// O direito (o secundário).
    Right,
    /// Nenhum: o movimento sem botão pressionado.
    None,
    /// A roda para cima.
    WheelUp,
    /// A roda para baixo.
    WheelDown,
    /// A roda para a esquerda.
    WheelLeft,
    /// A roda para a direita.
    WheelRight,
}

impl MouseButton {
    /// O número do botão no relato: 0 a 2 os botões, 3 nenhum, 64 a 67 a roda.
    fn code(self) -> u32 {
        match self {
            Self::Left => 0,
            Self::Middle => 1,
            Self::Right => 2,
            Self::None => 3,
            Self::WheelUp => 64,
            Self::WheelDown => 65,
            Self::WheelLeft => 66,
            Self::WheelRight => 67,
        }
    }

    /// Se é a roda (que só tem o pressionar).
    pub fn is_wheel(self) -> bool {
        matches!(
            self,
            Self::WheelUp | Self::WheelDown | Self::WheelLeft | Self::WheelRight
        )
    }
}

/// O que aconteceu com o botão.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseAction {
    /// Pressionou (a roda só tem este).
    Press,
    /// Soltou.
    Release,
    /// Moveu para outra célula: com o botão pressionado (arrastar) ou com
    /// [`MouseButton::None`].
    Motion,
}

/// Um evento do mouse numa célula da grade.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseEvent {
    /// O botão.
    pub button: MouseButton,
    /// O que aconteceu.
    pub action: MouseAction,
    /// A coluna, a partir de 0.
    pub col: usize,
    /// A linha da tela, a partir de 0.
    pub row: usize,
    /// As teclas modificadoras (Shift soma 4, Alt e a tecla do sistema 8, Ctrl 16).
    pub mods: Modifiers,
}

/// A maior coluna (ou linha) que cabe na forma padrão: 255 menos os 32 somados.
const DEFAULT_LIMIT: usize = 223;
/// A maior coluna (ou linha) que cabe na forma UTF-8 (dois bytes, até U+07FF).
const UTF8_LIMIT: usize = 2015;

/// Se o programa pediu o mouse (algum modo do [`MouseTracking`] ligado).
pub fn reports_mouse(modes: &Modes) -> bool {
    modes.mouse != MouseTracking::Off
}

/// Os bytes do evento para o programa, ou `None` quando ele não vai: relato desligado,
/// evento que o modo não pede (o soltar no X10, o movimento sem o 1002 ou o 1003) ou
/// posição além do que a forma escolhida consegue escrever.
pub fn encode_mouse(event: MouseEvent, modes: &Modes) -> Option<Vec<u8>> {
    let tracking = modes.mouse;
    let wanted = match (tracking, event.action) {
        (MouseTracking::Off, _) => false,
        (MouseTracking::X10, action) => action == MouseAction::Press,
        (_, MouseAction::Press) => true,
        (_, MouseAction::Release) => !event.button.is_wheel(),
        (MouseTracking::Buttons, MouseAction::Motion) => false,
        (MouseTracking::Drag, MouseAction::Motion) => event.button != MouseButton::None,
        (MouseTracking::Motion, MouseAction::Motion) => true,
    };
    if !wanted {
        return None;
    }
    let sgr = modes.mouse_encoding == MouseEncoding::Sgr;
    let mut code = if event.action == MouseAction::Release && !sgr {
        MouseButton::None.code()
    } else {
        event.button.code()
    };
    if event.action == MouseAction::Motion {
        code += 32;
    }
    if tracking != MouseTracking::X10 {
        let mods = event.mods;
        code += 4 * u32::from(mods.shift)
            + 8 * u32::from(mods.alt || mods.meta)
            + 16 * u32::from(mods.ctrl);
    }
    let (x, y) = (event.col + 1, event.row + 1);
    match modes.mouse_encoding {
        MouseEncoding::Sgr => {
            let end = if event.action == MouseAction::Release {
                'm'
            } else {
                'M'
            };
            Some(format!("\x1b[<{code};{x};{y}{end}").into_bytes())
        }
        MouseEncoding::Urxvt => Some(format!("\x1b[{};{x};{y}M", code + 32).into_bytes()),
        MouseEncoding::Default => {
            if x > DEFAULT_LIMIT || y > DEFAULT_LIMIT {
                return None;
            }
            let mut bytes = b"\x1b[M".to_vec();
            for value in [code as usize, x, y] {
                bytes.push((value + 32) as u8);
            }
            Some(bytes)
        }
        MouseEncoding::Utf8 => {
            if x > UTF8_LIMIT || y > UTF8_LIMIT {
                return None;
            }
            let mut text = String::from("\x1b[M");
            for value in [code as usize, x, y] {
                text.push(char::from_u32((value + 32) as u32)?);
            }
            Some(text.into_bytes())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modos(mouse: MouseTracking, mouse_encoding: MouseEncoding) -> Modes {
        Modes {
            mouse,
            mouse_encoding,
            ..Modes::default()
        }
    }

    fn evento(button: MouseButton, action: MouseAction, col: usize, row: usize) -> MouseEvent {
        MouseEvent {
            button,
            action,
            col,
            row,
            mods: Modifiers::NONE,
        }
    }

    fn texto(event: MouseEvent, modes: &Modes) -> Option<String> {
        encode_mouse(event, modes).map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
    }

    #[test]
    fn sgr_com_roda_botoes_e_soltar() {
        let m = modos(MouseTracking::Buttons, MouseEncoding::Sgr);
        let roda = evento(MouseButton::WheelUp, MouseAction::Press, 4, 2);
        assert_eq!(texto(roda, &m).as_deref(), Some("\x1b[<64;5;3M"));
        let baixo = evento(MouseButton::WheelDown, MouseAction::Press, 0, 0);
        assert_eq!(texto(baixo, &m).as_deref(), Some("\x1b[<65;1;1M"));
        let direita = evento(MouseButton::WheelRight, MouseAction::Press, 0, 0);
        assert_eq!(texto(direita, &m).as_deref(), Some("\x1b[<67;1;1M"));
        let solta = evento(MouseButton::Right, MouseAction::Release, 9, 9);
        assert_eq!(texto(solta, &m).as_deref(), Some("\x1b[<2;10;10m"));
        // A roda não tem o soltar.
        let roda_solta = evento(MouseButton::WheelUp, MouseAction::Release, 0, 0);
        assert_eq!(texto(roda_solta, &m), None);
    }

    #[test]
    fn modificadores_somam_ao_botao() {
        let m = modos(MouseTracking::Buttons, MouseEncoding::Sgr);
        let mut e = evento(MouseButton::Left, MouseAction::Press, 0, 0);
        e.mods = Modifiers {
            shift: true,
            ctrl: true,
            alt: true,
            meta: false,
        };
        assert_eq!(texto(e, &m).as_deref(), Some("\x1b[<28;1;1M"));
        e.mods = Modifiers {
            meta: true,
            ..Modifiers::NONE
        };
        assert_eq!(texto(e, &m).as_deref(), Some("\x1b[<8;1;1M"));
    }

    #[test]
    fn forma_padrao_e_o_limite_de_223() {
        let m = modos(MouseTracking::Buttons, MouseEncoding::Default);
        let e = evento(MouseButton::Left, MouseAction::Press, 4, 2);
        assert_eq!(encode_mouse(e, &m), Some(b"\x1b[M\x20\x25\x23".to_vec()));
        // O soltar da forma padrão não diz o botão: é o 3.
        let solta = evento(MouseButton::Left, MouseAction::Release, 4, 2);
        assert_eq!(
            encode_mouse(solta, &m),
            Some(b"\x1b[M\x23\x25\x23".to_vec())
        );
        let ultima = evento(MouseButton::Left, MouseAction::Press, 222, 222);
        assert_eq!(
            encode_mouse(ultima, &m),
            Some(b"\x1b[M\x20\xff\xff".to_vec())
        );
        let alem = evento(MouseButton::Left, MouseAction::Press, 223, 0);
        assert_eq!(encode_mouse(alem, &m), None);
    }

    #[test]
    fn utf8_e_urxvt_passam_de_223() {
        let utf8 = modos(MouseTracking::Buttons, MouseEncoding::Utf8);
        let e = evento(MouseButton::Left, MouseAction::Press, 299, 0);
        assert_eq!(texto(e, &utf8).as_deref(), Some("\x1b[M \u{14c}!"));
        let longe = evento(MouseButton::Left, MouseAction::Press, 2015, 0);
        assert_eq!(texto(longe, &utf8), None);
        let urxvt = modos(MouseTracking::Buttons, MouseEncoding::Urxvt);
        assert_eq!(texto(e, &urxvt).as_deref(), Some("\x1b[32;300;1M"));
    }

    #[test]
    fn x10_so_pressionar_sem_modificadores() {
        let m = modos(MouseTracking::X10, MouseEncoding::Sgr);
        let mut e = evento(MouseButton::Left, MouseAction::Press, 0, 0);
        e.mods = Modifiers::SHIFT;
        assert_eq!(texto(e, &m).as_deref(), Some("\x1b[<0;1;1M"));
        e.action = MouseAction::Release;
        assert_eq!(texto(e, &m), None);
        e.action = MouseAction::Motion;
        assert_eq!(texto(e, &m), None);
    }

    #[test]
    fn movimento_conforme_o_modo() {
        let arrasta = evento(MouseButton::Left, MouseAction::Motion, 1, 1);
        let solto = evento(MouseButton::None, MouseAction::Motion, 1, 1);
        let botoes = modos(MouseTracking::Buttons, MouseEncoding::Sgr);
        assert_eq!(texto(arrasta, &botoes), None);
        let drag = modos(MouseTracking::Drag, MouseEncoding::Sgr);
        assert_eq!(texto(arrasta, &drag).as_deref(), Some("\x1b[<32;2;2M"));
        assert_eq!(texto(solto, &drag), None);
        let motion = modos(MouseTracking::Motion, MouseEncoding::Sgr);
        assert_eq!(texto(solto, &motion).as_deref(), Some("\x1b[<35;2;2M"));
        let desligado = Modes::default();
        assert_eq!(texto(arrasta, &desligado), None);
        assert!(!reports_mouse(&desligado) && reports_mouse(&motion));
    }
}
