//! As notificações do sistema de verdade. O modelo (`Notification`, o clique, o grupo) está em
//! `tomni_ui_core::notification`; aqui ficam os três backends que o [`install`] escolhe ao
//! abrir a janela principal:
//!
//! - **Linux e BSDs:** o `org.freedesktop.Notifications` em D-Bus puro (`zbus`), numa thread
//!   que não trava a interface. A ação `default` é o clique; o `ActionInvoked` chega por uma
//!   segunda thread que escuta o sinal. O ícone vai num PNG em `$XDG_RUNTIME_DIR/tomni-ui/`, e
//!   o `app_id` da janela no `desktop-entry`, por onde o GNOME e o KDE acham o atalho do app.
//! - **Windows:** as notificações do WinRT, pelo `tomni-ui-win32` (que grava o nome e o ícone do
//!   `AppUserModelID` no registro do usuário). O ícone vai num PNG em
//!   `%LOCALAPPDATA%\tomni-ui\`.
//! - **macOS:** o `UNUserNotificationCenter`, pelo `tomni-ui-macos`. Só num `.app`: no
//!   `cargo run` não há notificações.
//!
//! O nome do app é o de `tomni_ui_core::notification::set_app_name` ou, sem ele, o título
//! inicial da janela principal.

/// Lado, em pixels, do ícone gravado para a notificação.
#[cfg(any(
    windows,
    all(unix, not(target_os = "macos"), not(target_os = "android"))
))]
const ICON_SIDE: u32 = 128;

/// O nome do app para o cabeçalho da notificação.
#[cfg(any(
    windows,
    all(unix, not(target_os = "macos"), not(target_os = "android"))
))]
fn display_name(title: &str) -> String {
    tomni_ui_core::notification::app_name()
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| title.to_owned())
}

/// O identificador do app: o `app_id` da janela ou o nome do executável.
#[cfg(any(
    windows,
    all(unix, not(target_os = "macos"), not(target_os = "android"))
))]
fn identifier(app_id: Option<&str>) -> String {
    app_id
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
                .filter(|n| !n.is_empty())
        })
        .unwrap_or_else(|| "tomni-ui".to_owned())
}

/// Grava o ícone `icon` como PNG em `dir` e devolve o caminho. O nome leva um resumo do
/// desenho: um desenho novo é um arquivo novo, e o sistema não fica com o antigo em cache.
/// Grava num `.tmp` e renomeia, para ninguém ler o arquivo pela metade.
#[cfg(any(
    windows,
    all(unix, not(target_os = "macos"), not(target_os = "android"))
))]
fn write_icon(
    dir: &std::path::Path,
    name: &str,
    icon: &crate::WindowIcon,
) -> Option<std::path::PathBuf> {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let bytes = icon.for_side(ICON_SIDE);
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    let path = dir.join(format!("{name}-notification-{:016x}.png", hasher.finish()));
    if path.is_file() {
        return Some(path);
    }
    let (data, width, height) = crate::icon::rgba(bytes, ICON_SIDE)?;
    std::fs::create_dir_all(dir).ok()?;
    let mut png = Vec::new();
    let mut encoder = png::Encoder::new(&mut png, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().ok()?.write_image_data(&data).ok()?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&temporary, png).ok()?;
    std::fs::rename(&temporary, &path).ok()?;
    Some(path)
}

#[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
pub(crate) use linux::install;
#[cfg(target_os = "macos")]
pub(crate) use macos::install;
#[cfg(windows)]
pub(crate) use windows::install;

/// Sem notificações neste sistema (o Android).
#[cfg(not(any(windows, target_os = "macos", all(unix, not(target_os = "android")))))]
pub(crate) fn install(_icon: Option<crate::WindowIcon>, _app_id: Option<&str>, _title: &str) {}

#[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
mod linux {
    use std::collections::{BTreeMap, HashMap};
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::sync::{Arc, Mutex, mpsc};
    use std::thread;

    use tomni_ui_core::notification::{NotificationBackend, NotificationSender, NotificationState};
    use zbus::blocking::{Connection, MessageIterator, Proxy};
    use zbus::zvariant::Value;

    use crate::WindowIcon;

    const SERVICE: &str = "org.freedesktop.Notifications";
    const PATH: &str = "/org/freedesktop/Notifications";
    const INTERFACE: &str = "org.freedesktop.Notifications";

    /// Quantas notificações o mapa do número do servidor para o nosso guarda.
    const KEPT: usize = 256;

    /// O que a interface manda para a thread do D-Bus.
    enum Command {
        Show(u64, NotificationState),
        Close(u64),
    }

    /// Como o app aparece no cabeçalho.
    struct Identity {
        name: String,
        desktop_entry: String,
        icon: Option<PathBuf>,
    }

    /// Do número que o servidor deu para o nosso.
    type Ours = Arc<Mutex<BTreeMap<u32, u64>>>;

    struct Linux {
        tx: mpsc::Sender<Command>,
    }

    impl NotificationBackend for Linux {
        fn show(&self, id: u64, state: &NotificationState) {
            let _ = self.tx.send(Command::Show(id, state.clone()));
        }

        fn close(&self, id: u64) {
            let _ = self.tx.send(Command::Close(id));
        }
    }

    /// A pasta do PNG do ícone: `$XDG_RUNTIME_DIR/tomni-ui`, ou a temporária sem ele.
    fn icon_dir() -> PathBuf {
        std::env::var_os("XDG_RUNTIME_DIR")
            .filter(|v| !v.is_empty())
            .map_or_else(std::env::temp_dir, PathBuf::from)
            .join("tomni-ui")
    }

    /// Instala o backend do D-Bus. A conexão nasce na thread dele: sem D-Bus de sessão, as
    /// notificações somem sem erro.
    pub(crate) fn install(icon: Option<WindowIcon>, app_id: Option<&str>, title: &str) {
        let desktop_entry = super::identifier(app_id);
        let identity = Identity {
            name: super::display_name(title),
            icon: icon.and_then(|icon| super::write_icon(&icon_dir(), &desktop_entry, &icon)),
            desktop_entry,
        };
        let sender = tomni_ui_core::notification::sender();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || work(&rx, &sender, &identity));
        tomni_ui_core::notification::set_backend(Rc::new(Linux { tx }));
    }

    /// O texto com o que o `body-markup` interpretaria escapado.
    fn escape(text: &str) -> String {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    fn work(rx: &mpsc::Receiver<Command>, sender: &NotificationSender, identity: &Identity) {
        let Ok(connection) = Connection::session() else {
            return;
        };
        let Ok(proxy) = Proxy::new(&connection, SERVICE, PATH, INTERFACE) else {
            return;
        };
        let ours: Ours = Arc::default();
        thread::spawn({
            let (connection, ours, sender) = (connection.clone(), ours.clone(), sender.clone());
            move || listen(&connection, &ours, &sender)
        });
        let icon = identity
            .icon
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        // O número que o servidor deu à última notificação de cada grupo.
        let mut tags: HashMap<String, u32> = HashMap::new();
        while let Ok(command) = rx.recv() {
            match command {
                Command::Show(id, state) => {
                    let replaces = state
                        .tag
                        .as_ref()
                        .and_then(|tag| tags.get(tag))
                        .copied()
                        .unwrap_or(0);
                    let mut hints: HashMap<&str, Value> = HashMap::new();
                    hints.insert(
                        "desktop-entry",
                        Value::from(identity.desktop_entry.as_str()),
                    );
                    hints.insert("urgency", Value::from(1_u8));
                    let actions = ["default", ""];
                    let reply: zbus::Result<u32> = proxy.call(
                        "Notify",
                        &(
                            identity.name.as_str(),
                            replaces,
                            icon.as_str(),
                            state.title.as_str(),
                            escape(&state.body).as_str(),
                            &actions[..],
                            hints,
                            -1_i32,
                        ),
                    );
                    let Ok(server) = reply else {
                        continue;
                    };
                    if let Ok(mut ours) = ours.lock() {
                        ours.remove(&replaces);
                        ours.insert(server, id);
                        while ours.len() > KEPT {
                            ours.pop_first();
                        }
                    }
                    if let Some(tag) = state.tag {
                        tags.insert(tag, server);
                    }
                }
                Command::Close(id) => {
                    let server = ours.lock().ok().and_then(|ours| {
                        ours.iter()
                            .find(|(_, ours)| **ours == id)
                            .map(|(server, _)| *server)
                    });
                    if let Some(server) = server {
                        let _: zbus::Result<()> = proxy.call("CloseNotification", &(server,));
                    }
                }
            }
        }
    }

    /// Escuta o `ActionInvoked` e avisa a interface do clique numa notificação nossa.
    fn listen(connection: &Connection, ours: &Ours, sender: &NotificationSender) {
        let Ok(rule) = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface(INTERFACE)
            .and_then(|rule| rule.member("ActionInvoked"))
            .map(|rule| rule.build())
        else {
            return;
        };
        let Ok(messages) = MessageIterator::for_match_rule(rule, connection, None) else {
            return;
        };
        for message in messages {
            let Ok(message) = message else {
                continue;
            };
            let Ok((server, action)) = message.body().deserialize::<(u32, String)>() else {
                continue;
            };
            if action != "default" {
                continue;
            }
            let id = ours.lock().ok().and_then(|ours| ours.get(&server).copied());
            if let Some(id) = id {
                sender.clicked(id);
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn o_texto_escapa_o_markup() {
            assert_eq!(escape("a < b & c > d"), "a &lt; b &amp; c &gt; d");
        }
    }
}

#[cfg(windows)]
mod windows {
    use std::path::PathBuf;
    use std::rc::Rc;

    use tomni_ui_core::notification::{NotificationBackend, NotificationState};
    use tomni_ui_win32::Toasts;

    use crate::WindowIcon;

    struct Windows {
        toasts: Toasts,
    }

    impl NotificationBackend for Windows {
        fn show(&self, id: u64, state: &NotificationState) {
            let sender = tomni_ui_core::notification::sender();
            self.toasts.show(
                id,
                &state.title,
                &state.body,
                state.tag.as_deref(),
                move || sender.clicked(id),
            );
        }

        fn close(&self, id: u64) {
            self.toasts.close(id);
        }
    }

    /// A pasta do PNG do ícone: `%LOCALAPPDATA%\tomni-ui`, ou a temporária sem ela.
    fn icon_dir() -> PathBuf {
        std::env::var_os("LOCALAPPDATA")
            .filter(|v| !v.is_empty())
            .map_or_else(std::env::temp_dir, PathBuf::from)
            .join("tomni-ui")
    }

    /// Registra o app e instala o backend do WinRT. Sem WinRT, nada é instalado.
    pub(crate) fn install(icon: Option<WindowIcon>, app_id: Option<&str>, title: &str) {
        let id = super::identifier(app_id);
        let icon = icon.and_then(|icon| super::write_icon(&icon_dir(), &id, &icon));
        if let Some(toasts) = Toasts::new(&id, &super::display_name(title), icon.as_deref()) {
            tomni_ui_core::notification::set_backend(Rc::new(Windows { toasts }));
        }
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::rc::Rc;

    use tomni_ui_core::notification::{NotificationBackend, NotificationState};
    use tomni_ui_macos::Notifications;

    use crate::WindowIcon;

    struct Macos {
        notifications: Notifications,
    }

    impl NotificationBackend for Macos {
        fn show(&self, id: u64, state: &NotificationState) {
            self.notifications
                .show(id, &state.title, &state.body, state.tag.as_deref());
        }

        fn close(&self, id: u64) {
            self.notifications.close(id);
        }
    }

    /// Instala o backend do `UNUserNotificationCenter`. O nome e o ícone são os do pacote. Fora
    /// de um `.app`, nada é instalado.
    pub(crate) fn install(_icon: Option<WindowIcon>, _app_id: Option<&str>, _title: &str) {
        let sender = tomni_ui_core::notification::sender();
        if let Some(notifications) = Notifications::new(move |id| sender.clicked(id)) {
            tomni_ui_core::notification::set_backend(Rc::new(Macos { notifications }));
        }
    }
}
