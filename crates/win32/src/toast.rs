//! As notificações do sistema do Windows (as do WinRT, que ficam na Central de Notificações).
//!
//! Um app sem pacote (MSIX) só mostra notificação com um `AppUserModelID` que o sistema
//! conheça: [`Toasts::new`] grava o nome e o ícone dele em
//! `HKCU\Software\Classes\AppUserModelId\<id>`, que é o que a notificação mostra no cabeçalho.
//! A gravação no registro é a única parte com `unsafe`; o resto é WinRT com API segura.
//!
//! O clique numa notificação chega pelo evento `Activated` dela, numa thread do sistema,
//! enquanto o app está aberto. As notificações mostradas ficam guardadas (as últimas
//! [`KEPT`]), senão o evento não chega e o [`Toasts::close`] não as acha.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Path;

use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::TypedEventHandler;
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager, ToastNotifier};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegSetValueExW,
};
use windows::core::{HSTRING, IInspectable, PCWSTR};

/// Quantas notificações ficam guardadas para o clique e o fechar.
const KEPT: usize = 64;

/// O grupo de todas as notificações do app: com a etiqueta, é o que faz a nova tomar o lugar
/// da anterior.
const GROUP: &str = "tomni-ui";

/// Quem mostra as notificações de um `AppUserModelID`.
pub struct Toasts {
    notifier: ToastNotifier,
    shown: RefCell<VecDeque<(u64, ToastNotification)>>,
}

impl std::fmt::Debug for Toasts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Toasts")
            .field("shown", &self.shown.borrow().len())
            .finish_non_exhaustive()
    }
}

impl Toasts {
    /// Registra o app `app_id` com o nome `name` e o ícone `icon` (um PNG ou ICO já gravado) e
    /// prepara as notificações dele. `None` se o sistema não tem notificações (o Windows sem
    /// WinRT, um Server Core).
    pub fn new(app_id: &str, name: &str, icon: Option<&Path>) -> Option<Self> {
        register(app_id, name, icon);
        let notifier =
            ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(app_id)).ok()?;
        Some(Self {
            notifier,
            shown: RefCell::new(VecDeque::new()),
        })
    }

    /// Mostra a notificação `id`. A de mesma `tag` toma o lugar da anterior. `clicked` roda
    /// numa thread do sistema quando o usuário clica nela. Devolve se o sistema aceitou.
    pub fn show(
        &self,
        id: u64,
        title: &str,
        body: &str,
        tag: Option<&str>,
        clicked: impl Fn() + Send + 'static,
    ) -> bool {
        self.try_show(id, title, body, tag, clicked).is_ok()
    }

    fn try_show(
        &self,
        id: u64,
        title: &str,
        body: &str,
        tag: Option<&str>,
        clicked: impl Fn() + Send + 'static,
    ) -> windows::core::Result<()> {
        let xml = XmlDocument::new()?;
        xml.LoadXml(&HSTRING::from(content(title, body)))?;
        let toast = ToastNotification::CreateToastNotification(&xml)?;
        if let Some(tag) = tag {
            toast.SetTag(&HSTRING::from(short_tag(tag)))?;
            toast.SetGroup(&HSTRING::from(GROUP))?;
        }
        toast.Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(
            move |_, _| {
                clicked();
                Ok(())
            },
        ))?;
        self.notifier.Show(&toast)?;
        let mut shown = self.shown.borrow_mut();
        shown.push_back((id, toast));
        while shown.len() > KEPT {
            shown.pop_front();
        }
        Ok(())
    }

    /// Tira a notificação `id` da tela e da Central de Notificações.
    pub fn close(&self, id: u64) {
        let toast = {
            let mut shown = self.shown.borrow_mut();
            let Some(at) = shown.iter().position(|(shown, _)| *shown == id) else {
                return;
            };
            shown.remove(at).map(|(_, toast)| toast)
        };
        if let Some(toast) = toast {
            let _ = self.notifier.Hide(&toast);
        }
    }
}

/// O XML da notificação: o título e, se houver, o texto.
fn content(title: &str, body: &str) -> String {
    let mut xml = String::from(
        "<toast><visual><binding template=\"ToastGeneric\"><text hint-maxLines=\"1\">",
    );
    xml.push_str(&escape(title));
    xml.push_str("</text>");
    if !body.is_empty() {
        xml.push_str("<text>");
        xml.push_str(&escape(body));
        xml.push_str("</text>");
    }
    xml.push_str("</binding></visual></toast>");
    xml
}

/// O texto com os caracteres especiais do XML escapados.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // Os de controle (fora a quebra de linha) não valem em XML 1.0.
            c if c.is_control() && c != '\n' && c != '\t' => {}
            c => out.push(c),
        }
    }
    out
}

/// A etiqueta em 16 caracteres: o Windows 10 antigo recusa as maiores.
fn short_tag(tag: &str) -> String {
    let mut hasher = DefaultHasher::new();
    tag.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Grava o nome e o ícone do app em `HKCU\Software\Classes\AppUserModelId\<app_id>`. Um erro
/// só deixa o cabeçalho da notificação sem o nome bonito.
fn register(app_id: &str, name: &str, icon: Option<&Path>) {
    let path = HSTRING::from(format!(r"Software\Classes\AppUserModelId\{app_id}"));
    let mut key = HKEY::default();
    // SAFETY: `path` é uma string larga terminada em zero que vive até o fim da chamada, e
    // `key` é um `HKEY` válido para receber a chave aberta.
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(path.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        )
    };
    if status != ERROR_SUCCESS {
        return;
    }
    set_string(key, "DisplayName", name);
    if let Some(icon) = icon {
        set_string(key, "IconUri", &icon.to_string_lossy());
    }
    // SAFETY: `key` foi aberta pelo `RegCreateKeyExW` acima e não é usada depois daqui.
    let _ = unsafe { RegCloseKey(key) };
}

/// Grava o texto `value` no valor `name` da chave `key`.
fn set_string(key: HKEY, name: &str, value: &str) {
    let name = HSTRING::from(name);
    // O `REG_SZ` são os bytes do UTF-16 com o zero do fim.
    let data: Vec<u8> = value
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect();
    // SAFETY: `key` é uma chave aberta com `KEY_SET_VALUE`, `name` é uma string larga
    // terminada em zero e `data` vive até o fim da chamada.
    let _ = unsafe { RegSetValueExW(key, PCWSTR(name.as_ptr()), None, REG_SZ, Some(&data)) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn o_xml_escapa_o_texto_e_omite_o_corpo_vazio() {
        assert_eq!(
            content("A & <B>", ""),
            "<toast><visual><binding template=\"ToastGeneric\"><text hint-maxLines=\"1\">\
             A &amp; &lt;B&gt;</text></binding></visual></toast>"
        );
        assert!(content("T", "x\u{7}y").contains("<text>xy</text>"));
    }

    #[test]
    fn a_etiqueta_curta_e_estavel() {
        assert_eq!(short_tag("aba-1"), short_tag("aba-1"));
        assert_ne!(short_tag("aba-1"), short_tag("aba-2"));
        assert_eq!(short_tag("uma etiqueta bem comprida demais").len(), 16);
    }
}
