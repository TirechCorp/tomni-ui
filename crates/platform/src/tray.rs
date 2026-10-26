//! A bandeja de verdade (M44). O modelo (`Tray`, `TrayHandle`, as closures e as ações) está em
//! `tomni_ui_core::tray`; aqui ficam os dois desenhistas que o [`install`] escolhe:
//!
//! - **Windows e macOS:** o `tray-icon` (com o menu do `muda` que ele reexporta). O ícone nasce
//!   na thread da interface, que é a do laço do `winit`: o macOS exige e, no Windows, o laço de
//!   mensagens do `winit` atende a janela escondida do ícone. Os cliques chegam pelos
//!   `set_event_handler` do `tray-icon` e voltam à interface por [`TraySender`]. A bandeja
//!   sempre existe.
//! - **Linux e BSDs:** o `ksni`, o protocolo `StatusNotifierItem` em D-Bus puro, sem GTK. Cada
//!   ícone tem uma thread, que fala com o D-Bus sem travar a interface. Com
//!   `assume_sni_available`, a falta do `StatusNotifierWatcher` não derruba o ícone: o `ksni`
//!   avisa (`watcher_offline`), a interface vê [`tomni_ui_core::tray::available`] cair e o
//!   ícone aparece quando o watcher volta (`watcher_online`).
//!
//! Nos outros sistemas (o Android) não há bandeja: nada é instalado e `available()` fica
//! `false`.

#[cfg(any(windows, target_os = "macos"))]
pub(crate) use desktop::install;
#[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
pub(crate) use linux::install;

/// Sem bandeja neste sistema.
#[cfg(not(any(windows, target_os = "macos", all(unix, not(target_os = "android")))))]
pub(crate) fn install() {}

/// Lado, em pixels, em que a bandeja rasteriza um ícone SVG (o PNG vai como está).
#[cfg(not(target_os = "android"))]
const SVG_SIDE: u32 = 64;

/// O ícone em RGBA8: os pixels, a largura e a altura. O SVG é rasterizado em [`SVG_SIDE`].
#[cfg(not(target_os = "android"))]
fn rgba(icon: &tomni_ui_core::tray::TrayImage) -> Option<(Vec<u8>, u32, u32)> {
    crate::icon::rgba(icon.png(), SVG_SIDE)
}

#[cfg(any(windows, target_os = "macos"))]
mod desktop {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    use tomni_ui_core::tray::{TrayBackend, TrayImage, TrayItemState, TraySender, TrayState};
    use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
    use tray_icon::{
        Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    };

    /// O item do menu como o `muda` o guarda.
    enum Entry {
        Plain(MenuItem),
        Check(CheckMenuItem),
        Separator,
    }

    /// O que identifica a forma do menu: se mudar, o menu é refeito; se não, só se atualiza o
    /// texto, a marca e o "habilitado" de cada item.
    #[derive(PartialEq)]
    struct Shape(Vec<(String, bool, bool)>);

    impl Shape {
        fn of(state: &TrayState) -> Self {
            Self(
                state
                    .items
                    .iter()
                    .map(|i| (i.id.clone(), i.toggle, i.separator))
                    .collect(),
            )
        }
    }

    struct Live {
        icon: TrayIcon,
        shape: Shape,
        entries: Vec<Entry>,
        image: TrayImage,
    }

    struct Desktop {
        live: RefCell<HashMap<u64, Live>>,
    }

    /// Instala a bandeja do `tray-icon` nesta thread (a da interface).
    pub(crate) fn install() {
        let sender = tomni_ui_core::tray::sender();
        let for_icon = sender.clone();
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            if let TrayIconEvent::Click {
                id,
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
                && let Ok(tray) = id.as_ref().parse::<u64>()
            {
                for_icon.activate(tray);
            }
        }));
        let for_menu: TraySender = sender;
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if let Some((tray, item)) = event.id.as_ref().split_once(':')
                && let Ok(tray) = tray.parse::<u64>()
            {
                for_menu.item(tray, item);
            }
        }));
        tomni_ui_core::tray::set_backend(
            Rc::new(Desktop {
                live: RefCell::new(HashMap::new()),
            }),
            true,
        );
    }

    fn icon(image: &TrayImage) -> Option<Icon> {
        let (pixels, width, height) = super::rgba(image)?;
        Icon::from_rgba(pixels, width, height).ok()
    }

    fn menu_id(tray: u64, item: &str) -> MenuId {
        MenuId::new(format!("{tray}:{item}"))
    }

    /// Monta o menu do zero para `state`.
    fn build(tray: u64, state: &TrayState) -> (Menu, Vec<Entry>) {
        let menu = Menu::new();
        let mut entries = Vec::with_capacity(state.items.len());
        for item in &state.items {
            let entry = if item.separator {
                let separator = PredefinedMenuItem::separator();
                let _ = menu.append(&separator);
                Entry::Separator
            } else if item.toggle {
                let check = CheckMenuItem::with_id(
                    menu_id(tray, &item.id),
                    &item.label,
                    item.enabled,
                    item.checked,
                    None,
                );
                let _ = menu.append(&check);
                Entry::Check(check)
            } else {
                let plain =
                    MenuItem::with_id(menu_id(tray, &item.id), &item.label, item.enabled, None);
                let _ = menu.append(&plain);
                Entry::Plain(plain)
            };
            entries.push(entry);
        }
        (menu, entries)
    }

    fn refresh(entries: &[Entry], items: &[TrayItemState]) {
        for (entry, item) in entries.iter().zip(items) {
            match entry {
                Entry::Plain(plain) => {
                    plain.set_text(&item.label);
                    plain.set_enabled(item.enabled);
                }
                Entry::Check(check) => {
                    check.set_text(&item.label);
                    check.set_enabled(item.enabled);
                    check.set_checked(item.checked);
                }
                Entry::Separator => {}
            }
        }
    }

    impl TrayBackend for Desktop {
        fn show(&self, id: u64, image: &TrayImage, state: &TrayState, activatable: bool) {
            let mut live = self.live.borrow_mut();
            let shape = Shape::of(state);
            if let Some(current) = live.get_mut(&id) {
                if current.image != *image {
                    let _ = current.icon.set_icon(icon(image));
                    current.image = *image;
                }
                let _ = current.icon.set_tooltip(Some(&state.tooltip));
                if current.shape == shape {
                    refresh(&current.entries, &state.items);
                } else {
                    let (menu, entries) = build(id, state);
                    current.icon.set_menu(Some(Box::new(menu)));
                    current.entries = entries;
                    current.shape = shape;
                }
                #[cfg(windows)]
                current.icon.set_show_menu_on_left_click(!activatable);
                return;
            }
            let (menu, entries) = build(id, state);
            let mut builder = TrayIconBuilder::new()
                .with_id(id.to_string())
                .with_tooltip(&state.tooltip)
                .with_menu(Box::new(menu));
            if let Some(icon) = icon(image) {
                builder = builder.with_icon(icon);
            }
            // No Windows, o clique esquerdo é do app quando ele tratou; o direito abre o menu.
            // No macOS o clique sempre abre o menu.
            #[cfg(windows)]
            {
                builder = builder.with_menu_on_left_click(!activatable);
            }
            #[cfg(not(windows))]
            let _ = activatable;
            if let Ok(created) = builder.build() {
                live.insert(
                    id,
                    Live {
                        icon: created,
                        shape,
                        entries,
                        image: *image,
                    },
                );
            }
        }

        fn remove(&self, id: u64) {
            self.live.borrow_mut().remove(&id);
        }
    }
}

#[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
mod linux {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;
    use std::sync::mpsc;
    use std::thread;

    use ksni::MenuItem;
    use ksni::blocking::TrayMethods;
    use ksni::menu::{CheckmarkItem, StandardItem};
    use tomni_ui_core::tray::{TrayBackend, TrayImage, TrayItemState, TraySender, TrayState};

    /// O que a interface manda para a thread de cada ícone.
    enum Message {
        Update(Box<View>),
        Remove,
    }

    /// O que o `ksni` precisa para desenhar o ícone.
    #[derive(Clone)]
    struct View {
        pixmap: Option<ksni::Icon>,
        state: TrayState,
        activatable: bool,
    }

    struct Sni {
        id: u64,
        name: String,
        view: View,
        sender: TraySender,
    }

    /// `_` num rótulo do D-Bus marca a tecla de acesso; `__` é um `_` de verdade.
    fn label(text: &str) -> String {
        text.replace('_', "__")
    }

    impl ksni::Tray for Sni {
        fn id(&self) -> String {
            self.name.clone()
        }

        fn title(&self) -> String {
            self.view.state.tooltip.clone()
        }

        fn tool_tip(&self) -> ksni::ToolTip {
            ksni::ToolTip {
                title: self.view.state.tooltip.clone(),
                ..ksni::ToolTip::default()
            }
        }

        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            self.view.pixmap.iter().cloned().collect()
        }

        fn activate(&mut self, _x: i32, _y: i32) {
            if self.view.activatable {
                self.sender.activate(self.id);
            }
        }

        fn menu(&self) -> Vec<MenuItem<Self>> {
            self.view
                .state
                .items
                .iter()
                .map(|item| entry(self.id, item))
                .collect()
        }
    }

    fn entry(tray: u64, item: &TrayItemState) -> MenuItem<Sni> {
        if item.separator {
            return MenuItem::Separator;
        }
        let id = item.id.clone();
        let activate: Box<dyn Fn(&mut Sni) + Send> =
            Box::new(move |sni: &mut Sni| sni.sender.item(tray, &id));
        if item.toggle {
            CheckmarkItem {
                label: label(&item.label),
                enabled: item.enabled,
                checked: item.checked,
                activate,
                ..CheckmarkItem::default()
            }
            .into()
        } else {
            StandardItem {
                label: label(&item.label),
                enabled: item.enabled,
                activate,
                ..StandardItem::default()
            }
            .into()
        }
    }

    /// O PNG como `Icon` do `ksni`: ARGB32 em ordem de rede.
    fn pixmap(image: &TrayImage) -> Option<ksni::Icon> {
        let (mut data, width, height) = super::rgba(image)?;
        for pixel in data.chunks_exact_mut(4) {
            pixel.rotate_right(1);
        }
        Some(ksni::Icon {
            width: i32::try_from(width).ok()?,
            height: i32::try_from(height).ok()?,
            data,
        })
    }

    /// O nome do app para o D-Bus: o do executável.
    fn app_name() -> String {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "tomni-ui".to_owned())
    }

    struct Linux {
        workers: RefCell<HashMap<u64, mpsc::Sender<Message>>>,
        sender: TraySender,
        name: String,
    }

    /// O nome do D-Bus de quem mostra os ícones (KDE, o AppIndicator do GNOME, o XFCE...).
    const WATCHER: &str = "org.kde.StatusNotifierWatcher";

    /// Acompanha o `StatusNotifierWatcher` e avisa a interface quando ele aparece ou some. É a
    /// única fonte do [`tomni_ui_core::tray::available`] no Linux: o GNOME sem a extensão não
    /// tem watcher, e o ícone não apareceria.
    fn watch_watcher(sender: &TraySender) -> Option<()> {
        use zbus::blocking::{Connection, fdo::DBusProxy};
        let connection = Connection::session().ok()?;
        let bus = DBusProxy::new(&connection).ok()?;
        let name = zbus::names::BusName::try_from(WATCHER).ok()?;
        let changes = bus
            .receive_name_owner_changed_with_args(&[(0, WATCHER)])
            .ok()?;
        sender.available(bus.name_has_owner(name).ok()?);
        for change in changes {
            let args = change.args().ok()?;
            sender.available(args.new_owner().is_some());
        }
        Some(())
    }

    /// Instala a bandeja do `ksni`. A disponibilidade começa em `false` e passa a seguir o
    /// watcher assim que a thread de vigia conecta ao D-Bus.
    pub(crate) fn install() {
        let sender = tomni_ui_core::tray::sender();
        thread::spawn({
            let sender = sender.clone();
            move || {
                if watch_watcher(&sender).is_none() {
                    sender.available(false);
                }
            }
        });
        tomni_ui_core::tray::set_backend(
            Rc::new(Linux {
                workers: RefCell::new(HashMap::new()),
                sender,
                name: app_name(),
            }),
            false,
        );
    }

    /// A thread de um ícone: sobe o serviço e aplica as atualizações até pedirem para remover.
    fn work(
        first: View,
        id: u64,
        name: String,
        sender: TraySender,
        messages: mpsc::Receiver<Message>,
    ) {
        let tray = Sni {
            id,
            name,
            view: first,
            sender,
        };
        // Com `assume_sni_available`, a falta do watcher não derruba o ícone: o `ksni` o
        // registra quando ele aparecer. Se nem o D-Bus de sessão existe, não há o que fazer.
        let Ok(handle) = tray.assume_sni_available(true).spawn() else {
            return;
        };
        while let Ok(message) = messages.recv() {
            match message {
                Message::Update(view) => {
                    let _ = handle.update(|sni| sni.view = *view);
                }
                Message::Remove => break,
            }
        }
        handle.shutdown().wait();
    }

    impl TrayBackend for Linux {
        fn show(&self, id: u64, image: &TrayImage, state: &TrayState, activatable: bool) {
            let view = View {
                pixmap: pixmap(image),
                state: state.clone(),
                activatable,
            };
            let mut workers = self.workers.borrow_mut();
            if let Some(worker) = workers.get(&id)
                && worker.send(Message::Update(Box::new(view.clone()))).is_ok()
            {
                return;
            }
            let (tx, rx) = mpsc::channel();
            let (sender, name) = (self.sender.clone(), self.name.clone());
            thread::spawn(move || work(view, id, name, sender, rx));
            workers.insert(id, tx);
        }

        fn remove(&self, id: u64) {
            if let Some(worker) = self.workers.borrow_mut().remove(&id) {
                let _ = worker.send(Message::Remove);
            }
        }
    }
}
