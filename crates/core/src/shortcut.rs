//! Atalhos de teclado: [`Shortcut`] e o `.accelerator()` de todo widget.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use tomni_ui_reactive::untrack;

use crate::event::{Key, Modifiers};
use crate::prop::{IntoProp, Prop};

/// Combinação de teclas: uma tecla e os modificadores, como o `KeyboardAccelerator` do WinUI.
///
/// ```
/// use tomni_ui_core::{Key, Shortcut};
///
/// assert_eq!(Shortcut::ctrl('N').to_string(), "Ctrl+N");
/// assert_eq!(Shortcut::ctrl_shift('t').to_string(), "Ctrl+Shift+T");
/// assert_eq!(Shortcut::key(Key::F(5)).to_string(), "F5");
/// ```
///
/// As letras valem com ou sem o Caps Lock: `Shortcut::ctrl('N')` e `Shortcut::ctrl('n')` são o
/// mesmo atalho, o Ctrl+N. O Shift conta: o Ctrl+Shift+N é outro atalho.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Shortcut {
    /// Tecla.
    pub key: Key,
    /// Modificadores, exatamente estes.
    pub modifiers: Modifiers,
}

impl Shortcut {
    /// Nenhum atalho: nunca casa com tecla alguma e não aparece no item de menu. Serve de valor
    /// para uma ação que não tem atalho (de fábrica ou por escolha). As teclas que a lib não
    /// conhece (`Key::Other`) também não disparam atalho.
    pub const NONE: Self = Self {
        key: Key::Other,
        modifiers: Modifiers::NONE,
    };

    /// Se é [`Shortcut::NONE`] (ou qualquer atalho de tecla desconhecida, que não dispara).
    pub fn is_none(&self) -> bool {
        self.key == Key::Other
    }

    /// `key` com exatamente os modificadores `modifiers`.
    pub fn new(key: impl Into<Key>, modifiers: Modifiers) -> Self {
        Self {
            key: key.into(),
            modifiers,
        }
    }

    /// Só a tecla, sem modificador (F5, Delete).
    pub fn key(key: impl Into<Key>) -> Self {
        Self::new(key, Modifiers::NONE)
    }

    /// Ctrl e a tecla: `Shortcut::ctrl('N')`.
    pub fn ctrl(key: impl Into<Key>) -> Self {
        Self::new(key, Modifiers::CTRL)
    }

    /// Ctrl, Shift e a tecla: `Shortcut::ctrl_shift('T')`.
    pub fn ctrl_shift(key: impl Into<Key>) -> Self {
        Self::new(key, Modifiers::CTRL | Modifiers::SHIFT)
    }

    /// Alt e a tecla: `Shortcut::alt(Key::F(4))`.
    pub fn alt(key: impl Into<Key>) -> Self {
        Self::new(key, Modifiers::ALT)
    }

    /// Shift e a tecla: `Shortcut::shift(Key::Delete)`.
    pub fn shift(key: impl Into<Key>) -> Self {
        Self::new(key, Modifiers::SHIFT)
    }
}

impl From<char> for Key {
    /// Letra ou número como [`Key::Char`], em minúscula.
    fn from(c: char) -> Self {
        Key::Char(c.to_lowercase().next().unwrap_or(c))
    }
}

impl fmt::Display for Shortcut {
    /// Texto mostrado à direita do item de menu: `Ctrl+Shift+N`, na ordem do Windows (Ctrl,
    /// Alt, Shift e a tecla do sistema).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_none() {
            return Ok(());
        }
        let m = self.modifiers;
        let system = if cfg!(any(target_os = "macos", target_os = "ios")) {
            "Cmd"
        } else {
            "Win"
        };
        for (on, name) in [
            (m.ctrl, "Ctrl"),
            (m.alt, "Alt"),
            (m.shift, "Shift"),
            (m.meta, system),
        ] {
            if on {
                write!(f, "{name}+")?;
            }
        }
        match self.key {
            Key::Char(c) => write!(f, "{}", c.to_uppercase()),
            Key::F(n) => write!(f, "F{n}"),
            key => f.write_str(match key {
                Key::Enter => "Enter",
                Key::Space => "Space",
                Key::Tab => "Tab",
                Key::Escape => "Esc",
                Key::Backspace => "Backspace",
                Key::Delete => "Del",
                Key::Left => "Left",
                Key::Right => "Right",
                Key::Up => "Up",
                Key::Down => "Down",
                Key::Home => "Home",
                Key::End => "End",
                Key::PageUp => "PgUp",
                Key::PageDown => "PgDn",
                Key::Insert => "Ins",
                Key::Alt => "Alt",
                Key::ContextMenu => "Menu",
                Key::Char(_) | Key::F(_) | Key::Other => "?",
            }),
        }
    }
}

/// Erro de [`Shortcut::from_str`](std::str::FromStr): o texto não é um atalho conhecido.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseShortcutError(String);

impl fmt::Display for ParseShortcutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ParseShortcutError {}

impl std::str::FromStr for Shortcut {
    type Err = ParseShortcutError;

    /// Lê o texto que o `Display` escreve: `Ctrl+Shift+T`, `Alt+Shift+-`, `F5`, `Ctrl+PageUp`.
    /// Maiúsculas ou minúsculas valem igual, `Control` é o mesmo que `Ctrl`, `Win`, `Cmd`,
    /// `Super` e `Meta` são a tecla do sistema, e a ordem dos modificadores não importa.
    ///
    /// ```
    /// use tomni_ui_core::{Key, Shortcut};
    ///
    /// assert_eq!("ctrl+shift+t".parse(), Ok(Shortcut::ctrl_shift('t')));
    /// assert_eq!("F5".parse(), Ok(Shortcut::key(Key::F(5))));
    /// assert_eq!(
    ///     "Ctrl+Xyz".parse::<Shortcut>().unwrap_err().to_string(),
    ///     "tecla desconhecida: Ctrl+Xyz"
    /// );
    /// ```
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let unknown = || ParseShortcutError(format!("tecla desconhecida: {text}"));
        let text = text.trim();
        // A tecla `+` fica no fim e não cabe no `split`: "Ctrl++".
        let (mods_text, key_text) = if let Some(rest) = text.strip_suffix('+') {
            match rest.strip_suffix('+') {
                Some(mods) => (mods, "+"),
                None if rest.is_empty() => ("", "+"),
                None => return Err(unknown()),
            }
        } else {
            match text.rsplit_once('+') {
                Some((mods, key)) => (mods, key),
                None => ("", text),
            }
        };
        let mut modifiers = Modifiers::NONE;
        for name in mods_text.split('+').filter(|name| !name.is_empty()) {
            match name.trim().to_lowercase().as_str() {
                "ctrl" | "control" => modifiers.ctrl = true,
                "alt" => modifiers.alt = true,
                "shift" => modifiers.shift = true,
                "win" | "cmd" | "super" | "meta" => modifiers.meta = true,
                _ => return Err(unknown()),
            }
        }
        let lower = key_text.trim().to_lowercase();
        let key = match lower.as_str() {
            "enter" | "return" => Key::Enter,
            "space" => Key::Space,
            "tab" => Key::Tab,
            "esc" | "escape" => Key::Escape,
            "backspace" => Key::Backspace,
            "del" | "delete" => Key::Delete,
            "left" => Key::Left,
            "right" => Key::Right,
            "up" => Key::Up,
            "down" => Key::Down,
            "home" => Key::Home,
            "end" => Key::End,
            "pgup" | "pageup" => Key::PageUp,
            "pgdn" | "pagedown" => Key::PageDown,
            "ins" | "insert" => Key::Insert,
            "alt" => Key::Alt,
            "menu" => Key::ContextMenu,
            other => {
                let mut chars = other.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) => Key::Char(c),
                    (Some('f'), Some(_)) => match other[1..].parse::<u8>() {
                        Ok(n) if (1..=24).contains(&n) => Key::F(n),
                        _ => return Err(unknown()),
                    },
                    _ => return Err(unknown()),
                }
            }
        };
        Ok(Self { key, modifiers })
    }
}

/// Atalho de um widget (`.accelerator()`): as combinações e a ação.
///
/// As combinações são uma [`Prop`]: lidas na hora da tecla (sem registrar dependência), então
/// trocar o signal troca o atalho sem montar a tela de novo.
#[derive(Clone)]
pub struct Accelerator {
    keys: Prop<Vec<Shortcut>>,
    action: Rc<RefCell<dyn FnMut()>>,
}

impl Accelerator {
    /// Atalho `shortcut` (fixo, closure ou signal) que roda `action`.
    pub fn new<M>(shortcut: impl IntoProp<Shortcut, M>, action: impl FnMut() + 'static) -> Self {
        Self::shared(shortcut.into_prop(), Rc::new(RefCell::new(action)))
    }

    /// Várias combinações para a mesma ação, que podem mudar de número: a lista é lida na hora
    /// da tecla.
    pub fn for_keys(keys: Prop<Vec<Shortcut>>, action: impl FnMut() + 'static) -> Self {
        Self {
            keys,
            action: Rc::new(RefCell::new(action)),
        }
    }

    /// Atalho que roda uma ação dividida com outro dono (a do item de menu).
    pub(crate) fn shared(shortcut: Prop<Shortcut>, action: Rc<RefCell<dyn FnMut()>>) -> Self {
        let keys = match shortcut {
            Prop::Value(shortcut) => Prop::Value(vec![shortcut]),
            Prop::Dynamic(f) => Prop::Dynamic(Rc::new(move || vec![f()])),
        };
        Self { keys, action }
    }

    /// As combinações de agora.
    pub fn shortcuts(&self) -> Vec<Shortcut> {
        untrack(|| self.keys.get())
    }

    /// Se `shortcut` é uma das combinações de agora.
    pub fn matches(&self, shortcut: Shortcut) -> bool {
        if shortcut.is_none() {
            return false;
        }
        untrack(|| match &self.keys {
            Prop::Value(keys) => keys.contains(&shortcut),
            Prop::Dynamic(f) => f().contains(&shortcut),
        })
    }

    /// Roda a ação. Uma ação que já está rodando (o atalho chamado de dentro dela) não roda de
    /// novo.
    pub fn run(&self) {
        if let Ok(mut action) = self.action.try_borrow_mut() {
            action();
        }
    }
}

impl fmt::Debug for Accelerator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let keys: Vec<String> = self.shortcuts().iter().map(Shortcut::to_string).collect();
        f.debug_struct("Accelerator")
            .field("shortcuts", &keys)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texto_na_ordem_do_windows() {
        assert_eq!(Shortcut::ctrl('n').to_string(), "Ctrl+N");
        assert_eq!(
            Shortcut::new('k', Modifiers::SHIFT | Modifiers::ALT | Modifiers::CTRL).to_string(),
            "Ctrl+Alt+Shift+K"
        );
        assert_eq!(Shortcut::key(Key::Delete).to_string(), "Del");
        assert_eq!(Shortcut::alt(Key::F(4)).to_string(), "Alt+F4");
    }

    #[test]
    fn letra_maiuscula_e_minuscula_sao_o_mesmo_atalho() {
        assert_eq!(Shortcut::ctrl('N'), Shortcut::ctrl('n'));
        assert_ne!(Shortcut::ctrl('n'), Shortcut::ctrl_shift('n'));
    }

    #[test]
    fn nenhum_atalho_nao_casa_nem_aparece() {
        assert!(Shortcut::NONE.is_none());
        assert_eq!(Shortcut::NONE.to_string(), "");
        let ligado = Accelerator::new(Shortcut::NONE, || {});
        assert!(!ligado.matches(Shortcut::NONE));
        // Uma tecla que a lib não conhece (F13, tecla morta) também não dispara.
        assert!(!ligado.matches(Shortcut::key(Key::Other)));
        assert!(!Shortcut::ctrl('n').is_none());
    }
}
