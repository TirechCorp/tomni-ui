//! Teclas como bytes, no formato do xterm.

use tomni_ui_core::{Key, Modifiers};

use crate::emulator::Modes;

/// O número dos modificadores nas sequências do xterm (`ESC[1;5A` é Ctrl+↑): 1 mais 1 do
/// Shift, 2 do Alt e 4 do Ctrl.
fn modifier_code(mods: Modifiers) -> u8 {
    1 + u8::from(mods.shift) + 2 * u8::from(mods.alt) + 4 * u8::from(mods.ctrl)
}

/// Ctrl com uma tecla de caractere: os controles do ASCII (Ctrl+A é 0x01, Ctrl+[ é o ESC).
fn control(c: char) -> Option<u8> {
    Some(match c {
        'a'..='z' => c as u8 - b'a' + 1,
        '@' | ' ' | '2' => 0,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '^' | '6' => 0x1e,
        '_' | '-' | '/' | '7' => 0x1f,
        '?' | '8' => 0x7f,
        _ => return None,
    })
}

/// Bytes da tecla `key` com `mods`, ou `None` se ela não vai para o programa por aqui (o
/// texto comum chega pelo evento de texto; ver [`encode_text`]).
pub fn encode_key(key: Key, mods: Modifiers, modes: &Modes) -> Option<Vec<u8>> {
    let code = modifier_code(mods);
    let esc_if_alt = |bytes: &[u8]| {
        let mut out = Vec::with_capacity(bytes.len() + 1);
        if mods.alt {
            out.push(0x1b);
        }
        out.extend_from_slice(bytes);
        out
    };
    // Setas, Home e End: `CSI 1;m X` com modificador; sem, `SS3 X` no modo aplicação.
    let cursor_key = |final_byte: u8| {
        if code > 1 {
            format!("\x1b[1;{code}{}", final_byte as char).into_bytes()
        } else if modes.app_cursor {
            vec![0x1b, b'O', final_byte]
        } else {
            vec![0x1b, b'[', final_byte]
        }
    };
    let tilde = |number: u8| {
        if code > 1 {
            format!("\x1b[{number};{code}~").into_bytes()
        } else {
            format!("\x1b[{number}~").into_bytes()
        }
    };
    Some(match key {
        Key::Char(c) if mods.ctrl && !mods.alt && !mods.meta => vec![control(c)?],
        Key::Char(c) if mods.ctrl && mods.alt && c.is_ascii_lowercase() && !mods.meta => {
            // Ctrl+Alt+letra (no Linux e no macOS; no Windows é o AltGr e vem como texto).
            if cfg!(windows) {
                return None;
            }
            vec![0x1b, control(c)?]
        }
        Key::Space if mods.ctrl => vec![0],
        Key::Enter if modes.newline => esc_if_alt(b"\r\n"),
        Key::Enter => esc_if_alt(b"\r"),
        Key::Tab if mods.ctrl => return None,
        Key::Tab if mods.shift => b"\x1b[Z".to_vec(),
        Key::Tab => esc_if_alt(b"\t"),
        Key::Escape => esc_if_alt(b"\x1b"),
        Key::Backspace if mods.ctrl => esc_if_alt(b"\x08"),
        Key::Backspace => esc_if_alt(b"\x7f"),
        Key::Up => cursor_key(b'A'),
        Key::Down => cursor_key(b'B'),
        Key::Right => cursor_key(b'C'),
        Key::Left => cursor_key(b'D'),
        Key::Home => cursor_key(b'H'),
        Key::End => cursor_key(b'F'),
        Key::Insert => tilde(2),
        Key::Delete => tilde(3),
        Key::PageUp => tilde(5),
        Key::PageDown => tilde(6),
        Key::F(n @ 1..=4) => {
            let final_byte = b'P' + (n - 1);
            if code > 1 {
                format!("\x1b[1;{code}{}", final_byte as char).into_bytes()
            } else {
                vec![0x1b, b'O', final_byte]
            }
        }
        Key::F(n @ 5..=12) => tilde([15, 17, 18, 19, 20, 21, 23, 24][usize::from(n - 5)]),
        _ => return None,
    })
}

/// Bytes de um texto digitado: o UTF-8 dele, com ESC antes se o Alt estava pressionado (o
/// "meta" do xterm).
pub fn encode_text(text: &str, mods: Modifiers) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() + 1);
    if mods.alt && !mods.ctrl {
        out.push(0x1b);
    }
    out.extend_from_slice(text.as_bytes());
    out
}

/// Bytes de um texto colado: as quebras de linha viram CR (o Enter), e com o modo de colar
/// entre marcas (`ESC[?2004h`) o texto vai entre `ESC[200~` e `ESC[201~`, sem os ESC dele
/// próprio, para o programa não confundir com teclas.
pub fn encode_paste(text: &str, modes: &Modes) -> Vec<u8> {
    let normalized = text.replace("\r\n", "\r").replace('\n', "\r");
    if modes.bracketed_paste {
        let clean: String = normalized.chars().filter(|&c| c != '\x1b').collect();
        let mut out = b"\x1b[200~".to_vec();
        out.extend_from_slice(clean.as_bytes());
        out.extend_from_slice(b"\x1b[201~");
        out
    } else {
        normalized.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(key: Key, mods: Modifiers) -> Vec<u8> {
        encode_key(key, mods, &Modes::default()).unwrap_or_default()
    }

    #[test]
    fn controles_com_ctrl() {
        assert_eq!(enc(Key::Char('c'), Modifiers::CTRL), [3]);
        assert_eq!(enc(Key::Char('a'), Modifiers::CTRL), [1]);
        assert_eq!(enc(Key::Char('['), Modifiers::CTRL), [0x1b]);
        assert_eq!(enc(Key::Space, Modifiers::CTRL), [0]);
        assert_eq!(
            encode_key(Key::Char('c'), Modifiers::NONE, &Modes::default()),
            None
        );
    }

    #[test]
    fn teclas_de_edicao() {
        assert_eq!(enc(Key::Enter, Modifiers::NONE), b"\r");
        assert_eq!(enc(Key::Backspace, Modifiers::NONE), [0x7f]);
        assert_eq!(enc(Key::Backspace, Modifiers::CTRL), [0x08]);
        assert_eq!(enc(Key::Tab, Modifiers::NONE), b"\t");
        assert_eq!(enc(Key::Tab, Modifiers::SHIFT), b"\x1b[Z");
        assert_eq!(
            encode_key(Key::Tab, Modifiers::CTRL, &Modes::default()),
            None
        );
        assert_eq!(enc(Key::Delete, Modifiers::NONE), b"\x1b[3~");
        assert_eq!(enc(Key::Insert, Modifiers::NONE), b"\x1b[2~");
        assert_eq!(enc(Key::PageUp, Modifiers::NONE), b"\x1b[5~");
        assert_eq!(enc(Key::PageDown, Modifiers::CTRL), b"\x1b[6;5~");
    }

    #[test]
    fn setas_nos_dois_modos() {
        let app = Modes {
            app_cursor: true,
            ..Modes::default()
        };
        assert_eq!(enc(Key::Up, Modifiers::NONE), b"\x1b[A");
        assert_eq!(
            encode_key(Key::Up, Modifiers::NONE, &app).unwrap(),
            b"\x1bOA"
        );
        assert_eq!(
            encode_key(Key::Home, Modifiers::NONE, &app).unwrap(),
            b"\x1bOH"
        );
        assert_eq!(enc(Key::End, Modifiers::NONE), b"\x1b[F");
        assert_eq!(enc(Key::Right, Modifiers::CTRL), b"\x1b[1;5C");
        let shift_alt = Modifiers {
            shift: true,
            alt: true,
            ..Modifiers::NONE
        };
        assert_eq!(
            encode_key(Key::Left, shift_alt, &app).unwrap(),
            b"\x1b[1;4D"
        );
    }

    #[test]
    fn teclas_de_funcao() {
        assert_eq!(enc(Key::F(1), Modifiers::NONE), b"\x1bOP");
        assert_eq!(enc(Key::F(4), Modifiers::NONE), b"\x1bOS");
        assert_eq!(enc(Key::F(5), Modifiers::NONE), b"\x1b[15~");
        assert_eq!(enc(Key::F(12), Modifiers::NONE), b"\x1b[24~");
        assert_eq!(enc(Key::F(2), Modifiers::SHIFT), b"\x1b[1;2Q");
        assert_eq!(enc(Key::F(10), Modifiers::CTRL), b"\x1b[21;5~");
    }

    #[test]
    fn alt_poe_esc_antes() {
        let alt = Modifiers {
            alt: true,
            ..Modifiers::NONE
        };
        assert_eq!(encode_text("x", alt), b"\x1bx");
        assert_eq!(encode_text("ç", Modifiers::NONE), "ç".as_bytes());
        assert_eq!(enc(Key::Backspace, alt), b"\x1b\x7f");
    }

    #[test]
    fn colar_troca_quebras_e_respeita_as_marcas() {
        let modes = Modes::default();
        assert_eq!(encode_paste("a\nb\r\nc", &modes), b"a\rb\rc");
        let marcas = Modes {
            bracketed_paste: true,
            ..modes
        };
        assert_eq!(encode_paste("x\x1by", &marcas), b"\x1b[200~xy\x1b[201~");
    }
}
