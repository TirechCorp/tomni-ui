//! O ícone do app na bandeja do sistema (a área de notificação do Windows, a barra de menu do
//! macOS, o `StatusNotifierItem` do Linux).
//!
//! ```ignore
//! let _bandeja = Tray::new(TrayImage::from_png(include_bytes!("icone.png")))
//!     .tooltip(move || format!("Cliques: {}", cliques.get()))
//!     .item("somar", || "Somar".to_string(), move || cliques.update(|c| *c += 1))
//!     .separator()
//!     .item("sair", || "Sair".to_string(), || WindowHandle::main().close())
//!     .show();
//! ```
//!
//! O texto da dica, os rótulos, a marca e o "habilitado" são closures que leem sinais: a
//! bandeja é atualizada sozinha quando eles mudam. As ações rodam na thread da interface, no
//! frame seguinte ao clique, como as dos widgets.
//!
//! Quem desenha o ícone é um [`TrayBackend`]: a plataforma instala o de verdade ao abrir a
//! janela (Windows e macOS pelo `tray-icon`, Linux pelo `ksni`); o `TestApp`, um falso. O
//! estado (backend, bandejas e eventos) é por thread, então testes em paralelo não se misturam.
//!
//! [`available`] diz se o sistema mostra ícones agora. No Linux depende do
//! `StatusNotifierWatcher` do D-Bus (o GNOME sem a extensão não tem); nos outros é sempre
//! verdadeiro. Sem bandeja o app deve se comportar como se ela não existisse (fechar a janela
//! encerra o app).

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::rc::{Rc, Weak};
use std::sync::mpsc;

use tomni_ui_reactive::{Get, GetUntracked, Scope, Set, Signal, effect, signal};

use crate::wake::wake;

/// A imagem do ícone: um PNG ou um SVG embutido, que a plataforma converte para o que o sistema
/// quer (o SVG é rasterizado no tamanho da bandeja).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrayImage {
    png: &'static [u8],
}

impl TrayImage {
    /// O ícone a partir dos bytes de um PNG.
    pub const fn from_png(bytes: &'static [u8]) -> Self {
        Self { png: bytes }
    }

    /// O ícone a partir dos bytes de um SVG. A plataforma o rasteriza no tamanho que a bandeja usa.
    pub const fn from_svg(bytes: &'static [u8]) -> Self {
        Self { png: bytes }
    }

    /// Os bytes da imagem (o PNG ou o SVG).
    pub const fn png(&self) -> &'static [u8] {
        self.png
    }
}

/// Um item do menu como o sistema o mostra agora.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayItemState {
    /// O identificador dado em [`Tray::item`] ou [`Tray::check`]; vazio no separador.
    pub id: String,
    /// O rótulo; vazio no separador.
    pub label: String,
    /// Se tem marca de seleção (os de [`Tray::check`]).
    pub toggle: bool,
    /// Se está marcado (só vale com `toggle`).
    pub checked: bool,
    /// Se aceita o clique.
    pub enabled: bool,
    /// Se é uma linha separadora.
    pub separator: bool,
}

/// A bandeja como o sistema a mostra agora: a dica e o menu.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrayState {
    /// A dica do ícone.
    pub tooltip: String,
    /// Os itens do menu, na ordem.
    pub items: Vec<TrayItemState>,
}

/// Quem desenha a bandeja. Roda na thread da interface; o que vier do sistema (cliques, a
/// bandeja aparecer ou sumir) volta por [`TraySender`].
pub trait TrayBackend {
    /// Cria o ícone `id` ou atualiza o que já existe com `state`. `activatable` diz que o app
    /// tratou [`Tray::on_activate`]: o clique esquerdo avisa [`TraySender::activate`] em vez de
    /// abrir o menu (onde o sistema deixa escolher).
    fn show(&self, id: u64, icon: &TrayImage, state: &TrayState, activatable: bool);
    /// Tira o ícone `id`.
    fn remove(&self, id: u64);
}

/// O que o sistema avisa.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Event {
    Activate(u64),
    Item(u64, String),
    Available(bool),
}

/// Por onde o backend avisa os cliques e a disponibilidade, de qualquer thread. Cada aviso
/// acorda o laço de eventos.
#[derive(Clone)]
pub struct TraySender {
    tx: mpsc::Sender<Event>,
}

impl TraySender {
    /// O clique no ícone da bandeja `tray`.
    pub fn activate(&self, tray: u64) {
        self.send(Event::Activate(tray));
    }

    /// O clique no item `item` do menu da bandeja `tray`.
    pub fn item(&self, tray: u64, item: &str) {
        self.send(Event::Item(tray, item.to_owned()));
    }

    /// O sistema passou a mostrar (ou deixou de mostrar) ícones.
    pub fn available(&self, available: bool) {
        self.send(Event::Available(available));
    }

    fn send(&self, event: Event) {
        let _ = self.tx.send(event);
        wake();
    }
}

struct Registry {
    backend: Option<Rc<dyn TrayBackend>>,
    trays: HashMap<u64, Weak<Inner>>,
    next: u64,
    available: Signal<bool>,
    /// Sobe quando o backend muda, para as bandejas já criadas se mostrarem nele.
    generation: Signal<u32>,
    /// Mantém vivos os signals de cima.
    _scope: Scope,
    tx: mpsc::Sender<Event>,
    rx: mpsc::Receiver<Event>,
    ready: VecDeque<Event>,
}

impl Registry {
    fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let scope = Scope::detached();
        let (available, generation) = scope.run(|| (signal(false), signal(0u32)));
        Self {
            backend: None,
            trays: HashMap::new(),
            next: 0,
            available,
            generation,
            _scope: scope,
            tx,
            rx,
            ready: VecDeque::new(),
        }
    }

    fn collect(&mut self) {
        while let Ok(event) = self.rx.try_recv() {
            self.ready.push_back(event);
        }
    }
}

thread_local! {
    static REGISTRY: RefCell<Registry> = RefCell::new(Registry::new());
}

/// Se o sistema mostra ícones na bandeja agora. Lido num efeito, é rastreado: muda quando a
/// bandeja aparece ou some. Sem backend (nas ferramentas e no `lint`) é `false`.
pub fn available() -> bool {
    REGISTRY.with(|r| r.borrow().available).get()
}

/// Instala quem desenha as bandejas desta thread e diz se o sistema já mostra ícones. As
/// bandejas já criadas se mostram nele. A plataforma chama ao abrir a janela; o `TestApp`, com
/// um falso.
#[doc(hidden)]
pub fn set_backend(backend: Rc<dyn TrayBackend>, available: bool) {
    let (available_signal, generation) = REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        r.backend = Some(backend);
        (r.available, r.generation)
    });
    available_signal.set(available);
    generation.set(generation.get_untracked().wrapping_add(1));
}

/// Tira o backend (a janela fechou). As bandejas continuam registradas e voltam a se mostrar
/// no próximo [`set_backend`].
#[doc(hidden)]
pub fn clear_backend() {
    let available = REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        r.backend = None;
        r.available
    });
    available.set(false);
}

/// Muda o que [`available`] responde, sem passar pela fila de eventos. Para o `TestApp`.
#[doc(hidden)]
pub fn set_available(available: bool) {
    REGISTRY.with(|r| r.borrow().available).set(available);
}

/// Por onde o backend avisa o que o sistema fez.
#[doc(hidden)]
pub fn sender() -> TraySender {
    TraySender {
        tx: REGISTRY.with(|r| r.borrow().tx.clone()),
    }
}

/// Volta ao começo: sem backend, sem bandejas e sem eventos. O `TestApp` chama ao nascer.
#[doc(hidden)]
pub fn reset() {
    REGISTRY.with(|r| *r.borrow_mut() = Registry::new());
}

/// Roda as ações dos eventos que chegaram. O `Ui::frame` chama no começo. Retorna se algum
/// rodou.
#[doc(hidden)]
pub fn deliver() -> bool {
    let ready = REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        r.collect();
        std::mem::take(&mut r.ready)
    });
    let mut any = false;
    for event in ready {
        match event {
            Event::Available(available) => {
                set_available(available);
                any = true;
            }
            Event::Activate(id) => {
                if let Some(on_activate) = find(id).as_ref().and_then(|t| t.on_activate.as_ref()) {
                    on_activate();
                    any = true;
                }
            }
            Event::Item(id, item) => {
                if let Some(tray) = find(id) {
                    any |= tray.click(&item);
                }
            }
        }
    }
    any
}

/// Se há evento esperando para o próximo [`deliver`]. O `Ui::deadline` usa para a plataforma
/// rodar o frame.
#[doc(hidden)]
pub fn has_event() -> bool {
    REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        r.collect();
        !r.ready.is_empty()
    })
}

fn find(id: u64) -> Option<Rc<Inner>> {
    REGISTRY.with(|r| r.borrow().trays.get(&id).and_then(Weak::upgrade))
}

type Text = Box<dyn Fn() -> String>;

enum Kind {
    Plain {
        label: Text,
        on_click: Box<dyn Fn()>,
    },
    Check {
        label: Text,
        checked: Box<dyn Fn() -> bool>,
        on_toggle: Box<dyn Fn(bool)>,
    },
    Separator,
}

struct Item {
    id: String,
    kind: Kind,
    enabled: Option<Box<dyn Fn() -> bool>>,
}

struct Inner {
    id: u64,
    icon: TrayImage,
    tooltip: Option<Text>,
    on_activate: Option<Box<dyn Fn()>>,
    items: Vec<Item>,
    alive: Cell<bool>,
}

impl Inner {
    /// O estado de agora. Lê as closures, então num efeito rastreia o que elas leem.
    fn snapshot(&self) -> TrayState {
        TrayState {
            tooltip: self.tooltip.as_ref().map(|t| t()).unwrap_or_default(),
            items: self
                .items
                .iter()
                .map(|item| {
                    let enabled = item.enabled.as_ref().is_none_or(|f| f());
                    match &item.kind {
                        Kind::Plain { label, .. } => TrayItemState {
                            id: item.id.clone(),
                            label: label(),
                            toggle: false,
                            checked: false,
                            enabled,
                            separator: false,
                        },
                        Kind::Check { label, checked, .. } => TrayItemState {
                            id: item.id.clone(),
                            label: label(),
                            toggle: true,
                            checked: checked(),
                            enabled,
                            separator: false,
                        },
                        Kind::Separator => TrayItemState {
                            id: String::new(),
                            label: String::new(),
                            toggle: false,
                            checked: false,
                            enabled: false,
                            separator: true,
                        },
                    }
                })
                .collect(),
        }
    }

    /// Roda a ação do item `id`, se existe e está habilitado. Retorna se rodou.
    fn click(&self, id: &str) -> bool {
        let Some(item) = self.items.iter().find(|i| i.id == id) else {
            return false;
        };
        let enabled = tomni_ui_reactive::untrack(|| item.enabled.as_ref().is_none_or(|f| f()));
        if !enabled {
            return false;
        }
        match &item.kind {
            Kind::Plain { on_click, .. } => on_click(),
            Kind::Check {
                checked, on_toggle, ..
            } => {
                let now = tomni_ui_reactive::untrack(checked);
                on_toggle(!now);
            }
            Kind::Separator => return false,
        }
        true
    }
}

/// A bandeja em construção; termina em [`Tray::show`].
pub struct Tray {
    icon: TrayImage,
    tooltip: Option<Text>,
    on_activate: Option<Box<dyn Fn()>>,
    items: Vec<Item>,
}

impl Tray {
    /// Uma bandeja com o ícone `icon` e sem itens.
    pub fn new(icon: TrayImage) -> Self {
        Self {
            icon,
            tooltip: None,
            on_activate: None,
            items: Vec::new(),
        }
    }

    /// A dica do ícone. Recalculada num efeito.
    pub fn tooltip(mut self, text: impl Fn() -> String + 'static) -> Self {
        self.tooltip = Some(Box::new(text));
        self
    }

    /// O clique no ícone (o esquerdo no Windows e no Linux; no macOS o clique abre o menu).
    pub fn on_activate(mut self, f: impl Fn() + 'static) -> Self {
        self.on_activate = Some(Box::new(f));
        self
    }

    /// Um item do menu. O `id` é o do protocolo e do `TestApp`.
    pub fn item(
        mut self,
        id: &str,
        label: impl Fn() -> String + 'static,
        f: impl Fn() + 'static,
    ) -> Self {
        self.items.push(Item {
            id: id.to_owned(),
            kind: Kind::Plain {
                label: Box::new(label),
                on_click: Box::new(f),
            },
            enabled: None,
        });
        self
    }

    /// Um item com marca de seleção. `f` recebe o valor novo (o contrário de `checked`).
    pub fn check(
        mut self,
        id: &str,
        label: impl Fn() -> String + 'static,
        checked: impl Fn() -> bool + 'static,
        f: impl Fn(bool) + 'static,
    ) -> Self {
        self.items.push(Item {
            id: id.to_owned(),
            kind: Kind::Check {
                label: Box::new(label),
                checked: Box::new(checked),
                on_toggle: Box::new(f),
            },
            enabled: None,
        });
        self
    }

    /// Habilita ou não o último item adicionado. Recalculado num efeito.
    pub fn enabled(mut self, when: impl Fn() -> bool + 'static) -> Self {
        if let Some(item) = self.items.last_mut() {
            item.enabled = Some(Box::new(when));
        }
        self
    }

    /// Uma linha separadora.
    pub fn separator(mut self) -> Self {
        self.items.push(Item {
            id: String::new(),
            kind: Kind::Separator,
            enabled: None,
        });
        self
    }

    /// Mostra o ícone. Soltar o [`TrayHandle`] o remove. Sem backend o estado fica guardado e
    /// aparece quando a plataforma abre a janela.
    pub fn show(self) -> TrayHandle {
        let (id, generation) = REGISTRY.with(|r| {
            let mut r = r.borrow_mut();
            let id = r.next;
            r.next += 1;
            (id, r.generation)
        });
        let inner = Rc::new(Inner {
            id,
            icon: self.icon,
            tooltip: self.tooltip,
            on_activate: self.on_activate,
            items: self.items,
            alive: Cell::new(true),
        });
        REGISTRY.with(|r| r.borrow_mut().trays.insert(id, Rc::downgrade(&inner)));
        let watching = Rc::downgrade(&inner);
        effect(move || {
            // Lido sempre: trocar o backend refaz a bandeja nele.
            let _ = generation.get();
            let Some(inner) = watching.upgrade() else {
                return;
            };
            let state = inner.snapshot();
            if !inner.alive.get() {
                return;
            }
            let backend = REGISTRY.with(|r| r.borrow().backend.clone());
            if let Some(backend) = backend {
                backend.show(inner.id, &inner.icon, &state, inner.on_activate.is_some());
            }
        });
        TrayHandle { inner }
    }
}

/// A bandeja que [`Tray::show`] mostrou. Soltar remove o ícone.
#[must_use = "soltar o TrayHandle remove o ícone da bandeja"]
pub struct TrayHandle {
    inner: Rc<Inner>,
}

impl TrayHandle {
    /// Mantém a bandeja até o app acabar, sem guardar o `TrayHandle`. O ícone some quando o
    /// processo termina.
    pub fn keep(self) {
        std::mem::forget(self);
    }

    /// O número da bandeja, o que o backend recebe em [`TrayBackend::show`].
    pub fn id(&self) -> u64 {
        self.inner.id
    }
}

impl Drop for TrayHandle {
    fn drop(&mut self) {
        self.inner.alive.set(false);
        let backend = REGISTRY.with(|r| {
            let mut r = r.borrow_mut();
            r.trays.remove(&self.inner.id);
            r.backend.clone()
        });
        if let Some(backend) = backend {
            backend.remove(self.inner.id);
        }
    }
}

impl std::fmt::Debug for TrayHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrayHandle")
            .field("id", &self.inner.id)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tomni_ui_reactive::{Update, run_effects};

    const PNG: &[u8] = &[0x89, b'P', b'N', b'G'];

    #[derive(Default)]
    struct Falso {
        atual: RefCell<HashMap<u64, TrayState>>,
        chamadas: Cell<u32>,
    }

    impl TrayBackend for Falso {
        fn show(&self, id: u64, _icon: &TrayImage, state: &TrayState, _activatable: bool) {
            self.chamadas.set(self.chamadas.get() + 1);
            self.atual.borrow_mut().insert(id, state.clone());
        }
        fn remove(&self, id: u64) {
            self.atual.borrow_mut().remove(&id);
        }
    }

    fn preparar() -> Rc<Falso> {
        reset();
        let falso = Rc::new(Falso::default());
        set_backend(falso.clone(), true);
        falso
    }

    fn icone() -> TrayImage {
        TrayImage::from_png(PNG)
    }

    #[test]
    fn a_dica_e_os_rotulos_acompanham_os_sinais() {
        let falso = preparar();
        let cliques = signal(0u32);
        let bandeja = Tray::new(icone())
            .tooltip(move || format!("Cliques: {}", cliques.get()))
            .item("somar", move || format!("Somar ({})", cliques.get()), || {})
            .show();
        run_effects();
        let estado = falso.atual.borrow()[&bandeja.id()].clone();
        assert_eq!(estado.tooltip, "Cliques: 0");
        assert_eq!(estado.items[0].label, "Somar (0)");
        cliques.update(|c| *c += 2);
        run_effects();
        let estado = falso.atual.borrow()[&bandeja.id()].clone();
        assert_eq!(estado.tooltip, "Cliques: 2");
        assert_eq!(estado.items[0].label, "Somar (2)");
    }

    #[test]
    fn o_clique_no_item_roda_a_acao_no_deliver() {
        let _falso = preparar();
        let cliques = signal(0u32);
        let bandeja = Tray::new(icone())
            .item(
                "somar",
                || "Somar".into(),
                move || cliques.update(|c| *c += 1),
            )
            .show();
        run_effects();
        sender().item(bandeja.id(), "somar");
        assert!(has_event());
        assert_eq!(cliques.get_untracked(), 0, "nunca síncrono");
        assert!(deliver());
        assert_eq!(cliques.get_untracked(), 1);
        assert!(!has_event());
    }

    #[test]
    fn item_desabilitado_nao_roda() {
        let _falso = preparar();
        let liberado = signal(false);
        let cliques = signal(0u32);
        let bandeja = Tray::new(icone())
            .item("a", || "A".into(), move || cliques.update(|c| *c += 1))
            .enabled(move || liberado.get())
            .show();
        run_effects();
        sender().item(bandeja.id(), "a");
        assert!(!deliver());
        assert_eq!(cliques.get_untracked(), 0);
        liberado.set(true);
        sender().item(bandeja.id(), "a");
        assert!(deliver());
        assert_eq!(cliques.get_untracked(), 1);
    }

    #[test]
    fn item_marcado_recebe_o_valor_novo() {
        let falso = preparar();
        let ligado = signal(false);
        let bandeja = Tray::new(icone())
            .check(
                "modo",
                || "Modo".into(),
                move || ligado.get(),
                move |v| ligado.set(v),
            )
            .show();
        run_effects();
        assert!(!falso.atual.borrow()[&bandeja.id()].items[0].checked);
        sender().item(bandeja.id(), "modo");
        deliver();
        run_effects();
        assert!(ligado.get_untracked());
        assert!(falso.atual.borrow()[&bandeja.id()].items[0].checked);
    }

    #[test]
    fn clicar_no_icone_chama_o_on_activate() {
        let _falso = preparar();
        let mostrou = signal(false);
        let bandeja = Tray::new(icone())
            .on_activate(move || mostrou.set(true))
            .show();
        sender().activate(bandeja.id());
        assert!(deliver());
        assert!(mostrou.get_untracked());
    }

    #[test]
    fn soltar_o_handle_remove_o_icone_e_ignora_cliques() {
        let falso = preparar();
        let cliques = signal(0u32);
        let bandeja = Tray::new(icone())
            .item("a", || "A".into(), move || cliques.update(|c| *c += 1))
            .show();
        run_effects();
        let id = bandeja.id();
        assert!(falso.atual.borrow().contains_key(&id));
        drop(bandeja);
        assert!(!falso.atual.borrow().contains_key(&id));
        sender().item(id, "a");
        assert!(!deliver());
        assert_eq!(cliques.get_untracked(), 0);
    }

    #[test]
    fn criada_antes_do_backend_se_mostra_quando_ele_chega() {
        reset();
        let bandeja = Tray::new(icone()).tooltip(|| "Oi".into()).show();
        run_effects();
        let falso = Rc::new(Falso::default());
        set_backend(falso.clone(), true);
        run_effects();
        assert_eq!(falso.atual.borrow()[&bandeja.id()].tooltip, "Oi");
    }

    #[test]
    fn a_disponibilidade_vem_dos_eventos_do_sistema() {
        let _falso = preparar();
        assert!(available());
        sender().available(false);
        assert!(deliver());
        assert!(!available());
        reset();
        assert!(!available(), "sem backend não há bandeja");
    }

    #[test]
    fn eventos_de_outra_thread_acordam_e_chegam() {
        let _falso = preparar();
        let ligado = signal(false);
        let bandeja = Tray::new(icone())
            .item("a", || "A".into(), move || ligado.set(true))
            .show();
        let emissor = sender();
        let id = bandeja.id();
        std::thread::spawn(move || emissor.item(id, "a"))
            .join()
            .unwrap();
        assert!(has_event());
        deliver();
        assert!(ligado.get_untracked());
    }

    #[test]
    fn o_separador_nao_tem_id_nem_acao() {
        let falso = preparar();
        let bandeja = Tray::new(icone())
            .item("a", || "A".into(), || {})
            .separator()
            .item("b", || "B".into(), || {})
            .show();
        run_effects();
        let itens = falso.atual.borrow()[&bandeja.id()].items.clone();
        assert_eq!(itens.len(), 3);
        assert!(itens[1].separator);
        assert!(itens[1].id.is_empty());
        assert!(!itens[0].separator);
    }
}
