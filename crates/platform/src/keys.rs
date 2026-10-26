//! Tradução do teclado do `winit` para os tipos do core.

use tomni_ui_core::{Key, Modifiers};
use winit::keyboard::{self, ModifiersState, NamedKey};

/// Tecla lógica do `winit` (já com o layout do teclado) como [`Key`]. Letras viram minúsculas,
/// para os atalhos não dependerem do Shift nem do Caps Lock.
pub fn key(key: &keyboard::Key) -> Key {
    match key {
        keyboard::Key::Named(named) => match named {
            NamedKey::Enter => Key::Enter,
            NamedKey::Space => Key::Space,
            NamedKey::Tab => Key::Tab,
            NamedKey::Escape => Key::Escape,
            NamedKey::Backspace => Key::Backspace,
            NamedKey::Delete => Key::Delete,
            NamedKey::ArrowLeft => Key::Left,
            NamedKey::ArrowRight => Key::Right,
            NamedKey::ArrowUp => Key::Up,
            NamedKey::ArrowDown => Key::Down,
            NamedKey::Home => Key::Home,
            NamedKey::End => Key::End,
            NamedKey::PageUp => Key::PageUp,
            NamedKey::PageDown => Key::PageDown,
            NamedKey::Insert => Key::Insert,
            NamedKey::F1 => Key::F(1),
            NamedKey::F2 => Key::F(2),
            NamedKey::F3 => Key::F(3),
            NamedKey::F4 => Key::F(4),
            NamedKey::F5 => Key::F(5),
            NamedKey::F6 => Key::F(6),
            NamedKey::F7 => Key::F(7),
            NamedKey::F8 => Key::F(8),
            NamedKey::F9 => Key::F(9),
            NamedKey::F10 => Key::F(10),
            NamedKey::F11 => Key::F(11),
            NamedKey::F12 => Key::F(12),
            NamedKey::Alt => Key::Alt,
            NamedKey::ContextMenu => Key::ContextMenu,
            _ => Key::Other,
        },
        keyboard::Key::Character(text) => {
            let mut chars = text.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Key::Char(c.to_lowercase().next().unwrap_or(c)),
                _ => Key::Other,
            }
        }
        _ => Key::Other,
    }
}

/// Modificadores do `winit`. `super` é a tecla do sistema: Cmd no macOS, Windows no Windows.
pub fn modifiers(state: ModifiersState) -> Modifiers {
    Modifiers {
        shift: state.shift_key(),
        ctrl: state.control_key(),
        alt: state.alt_key(),
        meta: state.super_key(),
    }
}

/// Texto de uma tecla, quando ela digita algo.
///
/// Fica de fora o que não é texto: caracteres de controle (Enter, Tab e Backspace também trazem
/// texto no `winit`) e os atalhos com Ctrl ou com a tecla do sistema. Ctrl junto com Alt passa,
/// porque é o AltGr do Windows (`@` no teclado ABNT2, por exemplo).
pub fn text(text: Option<&str>, mods: Modifiers) -> Option<&str> {
    let text = text.filter(|t| !t.is_empty())?;
    let shortcut = mods.meta || (mods.ctrl && !mods.alt);
    (!shortcut && !text.chars().any(char::is_control)).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(s: &str) -> keyboard::Key {
        keyboard::Key::Character(s.into())
    }

    #[test]
    fn teclas_nomeadas_e_letras() {
        assert_eq!(key(&keyboard::Key::Named(NamedKey::Tab)), Key::Tab);
        assert_eq!(key(&keyboard::Key::Named(NamedKey::ArrowLeft)), Key::Left);
        assert_eq!(
            key(&keyboard::Key::Named(NamedKey::PageDown)),
            Key::PageDown
        );
        assert_eq!(key(&keyboard::Key::Named(NamedKey::F1)), Key::F(1));
        assert_eq!(key(&keyboard::Key::Named(NamedKey::Insert)), Key::Insert);
        assert_eq!(key(&keyboard::Key::Named(NamedKey::F13)), Key::Other);
        assert_eq!(key(&keyboard::Key::Named(NamedKey::Alt)), Key::Alt);
        assert_eq!(
            key(&keyboard::Key::Named(NamedKey::ContextMenu)),
            Key::ContextMenu
        );
        assert_eq!(key(&ch("A")), Key::Char('a'));
        assert_eq!(key(&ch("ç")), Key::Char('ç'));
        assert_eq!(key(&ch("ab")), Key::Other);
        assert_eq!(key(&keyboard::Key::Dead(Some('´'))), Key::Other);
    }

    #[test]
    fn modificadores() {
        let m = modifiers(ModifiersState::SHIFT | ModifiersState::SUPER);
        assert_eq!(
            m,
            Modifiers {
                shift: true,
                meta: true,
                ..Modifiers::NONE
            }
        );
    }

    #[test]
    fn so_texto_de_verdade() {
        let ctrl = Modifiers {
            ctrl: true,
            ..Modifiers::NONE
        };
        let altgr = Modifiers { alt: true, ..ctrl };
        let cmd = Modifiers {
            meta: true,
            ..Modifiers::NONE
        };
        assert_eq!(text(Some("á"), Modifiers::SHIFT), Some("á"));
        assert_eq!(text(Some("@"), altgr), Some("@"));
        assert_eq!(text(Some("c"), ctrl), None);
        assert_eq!(text(Some("c"), cmd), None);
        assert_eq!(text(Some("\r"), Modifiers::NONE), None);
        assert_eq!(text(Some("\t"), Modifiers::NONE), None);
        assert_eq!(text(Some("\u{8}"), Modifiers::NONE), None);
        assert_eq!(text(Some(""), Modifiers::NONE), None);
        assert_eq!(text(None, Modifiers::NONE), None);
    }
}
