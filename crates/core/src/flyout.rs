//! `Flyout` e `MenuFlyout`: popups leves abertos por um botão ([`Button::flyout`]).
//!
//! [`Button::flyout`]: crate::Button::flyout

use std::cell::{Cell, RefCell};
use std::fmt;
use std::rc::Rc;

use std::time::Duration;

use kurbo::{Size, Vec2};
use tomni_ui_reactive::{Get, GetUntracked, Set, Signal, Update, signal, untrack};

use crate::button::ButtonState;
use crate::cx::EventCx;
use crate::density::Density;
use crate::event::{Event, Key};
use crate::press::Pressable;
use crate::prop::{IntoProp, Prop};
use crate::shortcut::{Accelerator, Shortcut};
use crate::symbol::Symbol;
use crate::widget::{
    Common, Flex, Insets, Layer, Look, Placement, PopupKind, View, Widget, common_builders,
};

/// `FlyoutContentPadding`, com a borda de 1 px somada.
const FLYOUT_PADDING: Insets = Insets::new(16.0, 17.0, 18.0, 17.0);
/// `MenuFlyoutPresenterThemePadding`, com a borda de 1 px somada.
const MENU_PADDING: Insets = Insets::new(3.0, 1.0, 3.0, 1.0);
/// Largura mínima do menu, escolhida pelo piloto.
const MENU_MIN_WIDTH: f64 = 96.0;
/// Tempo com o ponteiro parado sobre um item até o submenu abrir (o `MenuShowDelay` do
/// Windows).
const SUBMENU_DELAY: Duration = Duration::from_millis(400);

mod sealed {
    pub trait Sealed {}
}

/// O que um [`Button::flyout`](crate::Button::flyout) aceita: [`Flyout`] e [`MenuFlyout`].
pub trait FlyoutContent: Widget + sealed::Sealed {
    /// Signal que abre e fecha o popup.
    fn open_signal(&self) -> Signal<bool>;
}

/// Popup leve com qualquer conteúdo, aberto por um botão.
///
/// ```
/// use tomni_ui_core::{Button, Column, Flyout, Text};
/// use tomni_ui_reactive::prelude::*;
///
/// # let scope = Scope::root();
/// # scope.run(|| {
/// let ajuda = Button::new("Ajuda")
///     .id("ajuda")
///     .flyout(Flyout::new(Text::new("A senha precisa de 8 caracteres.")).id("ajuda-popup"));
/// # });
/// # scope.clear();
/// ```
///
/// Aparece embaixo do botão (em cima, se não couber) e fecha com Esc ou com um clique fora;
/// o foco vai para o primeiro widget de dentro que aceita foco e, ao fechar, volta ao botão.
/// Para fechar por código (um botão "OK" dentro dele), use [`Flyout::open`] com um signal seu.
pub struct Flyout {
    common: Common,
    open: Signal<bool>,
    child: Option<Box<dyn Widget>>,
}

impl Flyout {
    /// Popup com `content`, fechado.
    pub fn new(content: impl View) -> Self {
        Self {
            common: Common::default(),
            open: signal(false),
            child: Some(Box::new(content)),
        }
    }

    /// Usa `open` para abrir e fechar: `open.set(false)` fecha o popup.
    pub fn open(mut self, open: Signal<bool>) -> Self {
        self.open = open;
        self
    }
}

common_builders!(Flyout);

impl sealed::Sealed for Flyout {}

impl FlyoutContent for Flyout {
    fn open_signal(&self) -> Signal<bool> {
        self.open
    }
}

impl Widget for Flyout {
    fn kind(&self) -> &'static str {
        "flyout"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn visible(&self) -> bool {
        self.common.visible.as_ref().is_none_or(Prop::get) && self.open.get()
    }

    fn look(&self) -> Look {
        Look::Popup(PopupKind::Flyout)
    }

    fn flex(&self) -> Option<Flex> {
        Some(Flex {
            padding: FLYOUT_PADDING,
            ..Flex::default()
        })
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        self.child.take().into_iter().collect()
    }

    fn layer(&self) -> Option<Layer> {
        Some(Layer::default())
    }

    fn set_open(&mut self, open: bool) {
        self.open.set(open);
    }
}

impl fmt::Debug for Flyout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Flyout")
            .field("common", &self.common)
            .finish_non_exhaustive()
    }
}

/// Menu de ações, aberto por um botão ("Mais opções"), pela barra de menus ou pelo clique
/// direito (`.context_flyout()`).
///
/// ```
/// use tomni_ui_core::{Button, MenuFlyout, Shortcut, Symbol};
/// use tomni_ui_reactive::prelude::*;
///
/// # let scope = Scope::root();
/// # scope.run(|| {
/// let quebra = signal(true);
/// let mais = Button::new("Mais opções").id("mais").flyout(
///     MenuFlyout::new()
///         .id("menu")
///         .item("novo", "Nova conexão", || println!("nova"))
///         .icon(Symbol::Add)
///         .shortcut(Shortcut::ctrl('N'))
///         .toggle("quebra", "Quebrar linhas", quebra)
///         .submenu(
///             "exportar",
///             "Exportar",
///             MenuFlyout::new()
///                 .id("exportar-menu")
///                 .item("csv", "CSV", || println!("csv"))
///                 .item("json", "JSON", || println!("json")),
///         )
///         .separator()
///         .item("sair", "Sair", || println!("saiu")),
/// );
/// # });
/// # scope.clear();
/// ```
///
/// Abre embaixo do botão (em cima, se não couber), com o foco no primeiro item. ↑ e ↓ andam
/// pelos itens, Home e End vão ao primeiro e ao último, Enter ou Espaço escolhem, e Esc ou um
/// clique fora fecham sem escolher. Escolher um item roda a ação e fecha o menu (e os menus de
/// onde ele saiu).
///
/// Um submenu ([`MenuFlyout::submenu`]) abre à direita do item (à esquerda, se não couber) com
/// →, Enter, o clique ou o ponteiro parado 400 ms sobre o item; ← e Esc fecham só ele. Um item
/// de marcar ([`MenuFlyout::toggle`]) troca o signal e mostra a marca. O texto do atalho
/// ([`MenuFlyout::shortcut`]) aparece à direita e o atalho funciona com o menu fechado.
pub struct MenuFlyout {
    common: Common,
    open: Signal<bool>,
    entries: Vec<Entry>,
    /// Os `open` dos menus de onde este submenu saiu, do pai para cima.
    chain: Vec<Signal<bool>>,
    /// Num menu da `MenuBar`, os `open` de todos os menus da barra e a posição deste.
    pub(crate) bar: Option<(Rc<[Signal<bool>]>, usize)>,
    /// Onde o menu raiz grava o item em realce ([`MenuFlyout::highlighted`]).
    highlighted: Option<Signal<Option<String>>>,
    /// O estado do realce, dividido com os submenus: criado na montagem do menu raiz.
    highlights: Option<Rc<Highlights>>,
    /// As teclas de acesso dos itens, montadas junto com eles.
    keys: Vec<AccessKey>,
}

/// Item ou separador, guardado até a montagem.
enum Entry {
    Item(ItemSpec),
    /// A linha que separa grupos, com a visibilidade própria (a do grupo que ela separa).
    Separator(Option<Prop<bool>>),
}

/// Um item antes da montagem.
struct ItemSpec {
    id: String,
    label: Prop<String>,
    icon: Option<Symbol>,
    shortcut: Option<Prop<Shortcut>>,
    visible: Option<Prop<bool>>,
    enabled: Option<Prop<bool>>,
    access_key: Option<char>,
    kind: SpecKind,
}

enum SpecKind {
    Action(Rc<RefCell<dyn FnMut()>>),
    Toggle(Signal<bool>),
    Radio(Radio),
    Submenu(Box<MenuFlyout>),
}

/// O item de rádio, sem o tipo do grupo: se está marcado e como escolher.
#[derive(Clone)]
struct Radio {
    checked: Rc<dyn Fn() -> bool>,
    select: Rc<RefCell<dyn FnMut()>>,
}

impl MenuFlyout {
    /// Menu vazio, fechado.
    pub fn new() -> Self {
        Self {
            common: Common::default(),
            open: signal(false),
            entries: Vec::new(),
            chain: Vec::new(),
            bar: None,
            highlighted: None,
            highlights: None,
            keys: Vec::new(),
        }
    }

    fn push(mut self, id: String, label: Prop<String>, kind: SpecKind) -> Self {
        self.entries.push(Entry::Item(ItemSpec {
            id,
            label,
            icon: None,
            shortcut: None,
            visible: None,
            enabled: None,
            access_key: None,
            kind,
        }));
        self
    }

    /// Acrescenta um item com o id `id`, o rótulo `label` (fixo, closure ou signal) e a ação
    /// `action`.
    pub fn item<M>(
        self,
        id: impl Into<String>,
        label: impl IntoProp<String, M>,
        action: impl FnMut() + 'static,
    ) -> Self {
        let action: Rc<RefCell<dyn FnMut()>> = Rc::new(RefCell::new(action));
        self.push(id.into(), label.into_prop(), SpecKind::Action(action))
    }

    /// Acrescenta um item de marcar, ligado a `checked`: escolher troca o valor e fecha o
    /// menu. Com ele, todos os itens do menu ganham a coluna da marca, como no WinUI
    /// (`ToggleMenuFlyoutItem`).
    pub fn toggle<M>(
        self,
        id: impl Into<String>,
        label: impl IntoProp<String, M>,
        checked: Signal<bool>,
    ) -> Self {
        self.push(id.into(), label.into_prop(), SpecKind::Toggle(checked))
    }

    /// Acrescenta um item de rádio do grupo `group` (`RadioMenuFlyoutItem`): ele está marcado
    /// quando `group` vale `value`, e escolhê-lo faz `group.set(value)` e fecha o menu. Escolher
    /// o que já está marcado só fecha o menu. Os itens de rádio do mesmo menu que dividem o
    /// `group` formam o grupo. Com ele, todos os itens do menu ganham a coluna da marca, como
    /// com o [`MenuFlyout::toggle`], e o marcador é um ponto no lugar do visto.
    pub fn radio<T, M>(
        self,
        id: impl Into<String>,
        label: impl IntoProp<String, M>,
        group: Signal<T>,
        value: T,
    ) -> Self
    where
        T: Clone + PartialEq + Send + Sync + 'static,
    {
        let current = value.clone();
        let radio = Radio {
            checked: Rc::new(move || group.get() == current),
            select: Rc::new(RefCell::new(move || {
                if untrack(|| group.get()) != value {
                    group.set(value.clone());
                }
            })),
        };
        self.push(id.into(), label.into_prop(), SpecKind::Radio(radio))
    }

    /// Acrescenta um item que abre o submenu `menu` à direita (`MenuFlyoutSubItem`). O
    /// submenu fica com o id que você der a ele (`.id()` no `menu`).
    pub fn submenu<M>(
        self,
        id: impl Into<String>,
        label: impl IntoProp<String, M>,
        menu: MenuFlyout,
    ) -> Self {
        self.push(
            id.into(),
            label.into_prop(),
            SpecKind::Submenu(Box::new(menu)),
        )
    }

    /// Ícone de 16 px à esquerda do último item. Com ele, todos os itens do menu ganham a
    /// coluna do ícone.
    ///
    /// # Panics
    ///
    /// Sem item antes (menu vazio ou logo depois de um `.separator()`).
    #[track_caller]
    pub fn icon(mut self, icon: Symbol) -> Self {
        self.last("icon").icon = Some(icon);
        self
    }

    /// Atalho do último item (fixo, closure ou signal): o texto ("Ctrl+N") aparece à direita, e
    /// o atalho roda o item enquanto o menu (ou o dono dele) aparece, mesmo fechado. Com um
    /// signal, trocar o valor troca o atalho e o texto na hora, sem montar o menu de novo.
    ///
    /// # Panics
    ///
    /// Sem item antes, ou se o último é um `.submenu()`, que não tem ação.
    #[track_caller]
    pub fn shortcut<M>(mut self, shortcut: impl IntoProp<Shortcut, M>) -> Self {
        let last = self.last("shortcut");
        assert!(
            !matches!(last.kind, SpecKind::Submenu(_)),
            "`.shortcut()` vale para `.item()`, `.toggle()` e `.radio()`, não para `.submenu()`"
        );
        last.shortcut = Some(shortcut.into_prop());
        self
    }

    #[track_caller]
    fn last(&mut self, builder: &str) -> &mut ItemSpec {
        match self.entries.last_mut() {
            Some(Entry::Item(spec)) => spec,
            _ => panic!(
                "`.{builder}()` muda o último item: chame depois de `.item()`, `.toggle()`, `.radio()` ou `.submenu()`"
            ),
        }
    }

    /// Mostra o último item só enquanto `visible` for verdadeiro (fixo, closure ou signal). Item
    /// escondido não ocupa linha nem recebe foco. Serve a listas que mudam com o tempo, como as
    /// janelas abertas: reserve itens a mais e mostre os que existem.
    ///
    /// # Panics
    ///
    /// Sem item antes (menu vazio ou logo depois de um `.separator()`).
    #[track_caller]
    pub fn item_visible<M>(mut self, visible: impl IntoProp<bool, M>) -> Self {
        self.last("item_visible").visible = Some(visible.into_prop());
        self
    }

    /// Habilita o último item só enquanto `enabled` for verdadeiro (fixo, closure ou signal), como
    /// o `IsEnabled` do `MenuFlyoutItem` do WinUI. Desabilitado, o item fica com o texto e o
    /// ícone na cor de desabilitado, sem realce, e o clique, o atalho, a tecla de acesso e o
    /// Enter não fazem nada; o teclado passa por cima dele. Ele continua na linha, ao contrário
    /// do [`MenuFlyout::item_visible`]. Num submenu, não abre.
    ///
    /// # Panics
    ///
    /// Sem item antes (menu vazio ou logo depois de um `.separator()`).
    #[track_caller]
    pub fn item_enabled<M>(mut self, enabled: impl IntoProp<bool, M>) -> Self {
        self.last("item_enabled").enabled = Some(enabled.into_prop());
        self
    }

    /// Tecla de acesso do último item: com o menu (ou o submenu) aberto e o foco nele, a tecla
    /// `key` sozinha (sem Ctrl, Alt, Shift ou a tecla do sistema, e sem diferença entre maiúscula
    /// e minúscula) faz o mesmo que clicar no item. Um item comum roda a ação e fecha o menu, um
    /// submenu abre e um item de marcar troca o valor e fecha o menu. Item escondido
    /// ([`MenuFlyout::item_visible`]) não responde. Com duas teclas iguais no mesmo menu, vale
    /// a do primeiro item que aparece.
    ///
    /// **Adaptação:** a letra não aparece sublinhada no rótulo, como na [`MenuBar::access_key`]
    /// (o Win32 sublinha com Alt apertado; aqui não há a dica na tela).
    ///
    /// [`MenuBar::access_key`]: crate::MenuBar::access_key
    ///
    /// # Panics
    ///
    /// Sem item antes (menu vazio ou logo depois de um `.separator()`).
    #[track_caller]
    pub fn access_key(mut self, key: char) -> Self {
        self.last("access_key").access_key = key.to_lowercase().next();
        self
    }

    /// Grava em `signal` o id do item que a pessoa está prestes a escolher, só no menu raiz: o
    /// item **mais fundo** em realce, olhando os submenus abertos.
    ///
    /// - com um submenu aberto e um item dele sob o ponteiro ou com o foco, o id desse item;
    /// - com um submenu aberto e nenhum item dele em realce, o id do item que abriu o submenu;
    /// - senão, o id do item do menu raiz sob o ponteiro ou com o foco;
    /// - `None` quando não há nenhum, ou quando o menu fecha.
    ///
    /// Realce é o mesmo critério que pinta o fundo do item: ponteiro em cima ou foco vindo do
    /// teclado (o foco que o menu dá ao primeiro item quando abre pelo ponteiro não conta). Um signal
    /// só para a cadeia toda, e não um por submenu, porque na passagem do ponteiro de um menu
    /// para o outro a ordem das escritas deixaria um `None` no meio. Só grava quando o valor
    /// muda.
    pub fn highlighted(mut self, signal: Signal<Option<String>>) -> Self {
        self.highlighted = Some(signal);
        self
    }

    /// Acrescenta uma linha que separa grupos de itens.
    pub fn separator(mut self) -> Self {
        self.entries.push(Entry::Separator(None));
        self
    }

    /// Acrescenta uma linha que separa grupos e aparece só enquanto `visible` for verdadeiro
    /// (fixo, closure ou signal): a do grupo de itens que também aparecem e somem
    /// ([`MenuFlyout::item_visible`]).
    pub fn separator_visible<M>(mut self, visible: impl IntoProp<bool, M>) -> Self {
        self.entries
            .push(Entry::Separator(Some(visible.into_prop())));
        self
    }

    /// Põe os itens e os separadores de `head` antes dos deste menu. O `id`, o `open` e o
    /// resto de `head` não valem: só as linhas dele entram. Serve a quem monta um menu em duas
    /// partes, como o `Terminal`, que põe os itens do link sob o ponteiro antes do menu do app.
    pub fn prepend(mut self, head: MenuFlyout) -> Self {
        self.entries.splice(0..0, head.entries);
        self
    }

    /// Usa `open` para abrir e fechar: `open.set(false)` fecha o menu.
    pub fn open(mut self, open: Signal<bool>) -> Self {
        self.open = open;
        self
    }
}

impl MenuFlyout {
    /// Aciona o primeiro item que aparece e tem a tecla de acesso `typed`.
    fn press_access_key(&self, typed: char) -> bool {
        let Some(typed) = typed.to_lowercase().next() else {
            return false;
        };
        let Some(entry) = self.keys.iter().find(|entry| {
            entry.key == typed
                && untrack(|| {
                    entry.visible.as_ref().is_none_or(Prop::get)
                        && entry.enabled.as_ref().is_none_or(Prop::get)
                })
        }) else {
            return false;
        };
        match &entry.action {
            KeyAction::Run(action) => {
                let action = Rc::clone(action);
                close_all(&entry.close);
                if let Ok(mut action) = action.try_borrow_mut() {
                    action();
                }
            }
            KeyAction::Toggle(checked) => {
                close_all(&entry.close);
                checked.update(|c| *c = !*c);
            }
            KeyAction::Open(open) => open_submenu(*open, &entry.siblings),
        }
        true
    }
}

/// A tecla de acesso de um item.
struct AccessKey {
    /// Em minúscula.
    key: char,
    visible: Option<Prop<bool>>,
    enabled: Option<Prop<bool>>,
    action: KeyAction,
    close: Rc<[Signal<bool>]>,
    siblings: Rc<[Signal<bool>]>,
}

enum KeyAction {
    Run(Rc<RefCell<dyn FnMut()>>),
    Toggle(Signal<bool>),
    Open(Signal<bool>),
}

/// O estado do [`MenuFlyout::highlighted`], dividido por um menu e os submenus dele.
struct Highlights {
    signal: Signal<Option<String>>,
    root_open: Signal<bool>,
    items: RefCell<Vec<Rc<HighlightItem>>>,
}

/// O que o realce precisa saber de um item.
struct HighlightItem {
    id: String,
    /// Quantos menus há acima do menu do item: 0 no menu raiz.
    depth: usize,
    /// O `open` do menu do item.
    menu_open: Signal<bool>,
    /// O `open` do submenu do item, se ele abre um.
    submenu_open: Option<Signal<bool>>,
    hovered: Cell<bool>,
    focused: Cell<bool>,
}

impl Highlights {
    /// O item mais fundo em realce. Em cada menu aberto vale, do mais forte ao mais fraco: o
    /// item que abriu um submenu aberto, o item sob o ponteiro e o item com o foco; e o menu
    /// mais fundo ganha do mais raso.
    fn current(&self) -> Option<String> {
        if !self.root_open.get_untracked() {
            return None;
        }
        let mut best: Option<((usize, u8), String)> = None;
        for item in self.items.borrow().iter() {
            if !item.menu_open.get_untracked() {
                continue;
            }
            let strength = if item.submenu_open.is_some_and(|open| open.get_untracked()) {
                3
            } else if item.hovered.get() {
                2
            } else if item.focused.get() {
                1
            } else {
                continue;
            };
            let rank = (item.depth, strength);
            if best.as_ref().is_none_or(|(top, _)| rank > *top) {
                best = Some((rank, item.id.clone()));
            }
        }
        best.map(|(_, id)| id)
    }

    /// Grava o item em realce no signal, se mudou.
    fn sync(&self) {
        let next = self.current();
        untrack(|| {
            if self.signal.get_untracked() != next {
                self.signal.set(next);
            }
        });
    }
}

impl Default for MenuFlyout {
    fn default() -> Self {
        Self::new()
    }
}

common_builders!(MenuFlyout);

impl sealed::Sealed for MenuFlyout {}

impl FlyoutContent for MenuFlyout {
    fn open_signal(&self) -> Signal<bool> {
        self.open
    }
}

impl Widget for MenuFlyout {
    fn kind(&self) -> &'static str {
        "menu_flyout"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn visible(&self) -> bool {
        self.common.visible.as_ref().is_none_or(Prop::get) && self.open.get()
    }

    fn look(&self) -> Look {
        Look::Popup(PopupKind::Menu)
    }

    fn flex(&self) -> Option<Flex> {
        Some(Flex {
            padding: MENU_PADDING,
            ..Flex::default()
        })
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        // Montados só aqui: o `.open()` pode ter vindo depois dos itens.
        let entries = std::mem::take(&mut self.entries);
        let specs = || {
            entries.iter().filter_map(|entry| match entry {
                Entry::Item(spec) => Some(spec),
                Entry::Separator(_) => None,
            })
        };
        let columns = MenuColumns {
            check: specs()
                .any(|spec| matches!(spec.kind, SpecKind::Toggle(_) | SpecKind::Radio(_))),
            icon: specs().any(|spec| spec.icon.is_some()),
        };
        let siblings: Rc<[Signal<bool>]> = specs()
            .filter_map(|spec| match &spec.kind {
                SpecKind::Submenu(menu) => Some(menu.open),
                _ => None,
            })
            .collect();
        let close: Rc<[Signal<bool>]> = std::iter::once(self.open)
            .chain(self.chain.iter().copied())
            .collect();
        if self.chain.is_empty()
            && let Some(signal) = self.highlighted
        {
            self.highlights = Some(Rc::new(Highlights {
                signal,
                root_open: self.open,
                items: RefCell::new(Vec::new()),
            }));
        }
        let highlights = self.highlights.clone();
        let depth = self.chain.len();
        let menu_open = self.open;
        let mut keys: Vec<AccessKey> = Vec::new();
        let children = entries
            .into_iter()
            .map(|entry| -> Box<dyn Widget> {
                let spec = match entry {
                    Entry::Item(spec) => spec,
                    Entry::Separator(visible) => {
                        return Box::new(MenuFlyoutSeparator {
                            common: Common {
                                visible,
                                ..Common::default()
                            },
                        });
                    }
                };
                let mut common = Common {
                    id: Some(spec.id.clone()),
                    visible: spec.visible.clone(),
                    ..Common::default()
                };
                let enabled = spec.enabled.clone();
                let kind = match spec.kind {
                    SpecKind::Action(action) => {
                        if let Some(shortcut) = &spec.shortcut {
                            common
                                .accelerators
                                .push(Accelerator::shared(shortcut.clone(), Rc::clone(&action)));
                        }
                        ItemKind::Action(action)
                    }
                    SpecKind::Toggle(checked) => {
                        if let Some(shortcut) = spec.shortcut.clone() {
                            let toggle: Rc<RefCell<dyn FnMut()>> =
                                Rc::new(RefCell::new(move || checked.update(|c| *c = !*c)));
                            common
                                .accelerators
                                .push(Accelerator::shared(shortcut, toggle));
                        }
                        ItemKind::Toggle(checked)
                    }
                    SpecKind::Radio(radio) => {
                        if let Some(shortcut) = spec.shortcut.clone() {
                            common
                                .accelerators
                                .push(Accelerator::shared(shortcut, Rc::clone(&radio.select)));
                        }
                        ItemKind::Radio(radio)
                    }
                    SpecKind::Submenu(mut menu) => {
                        menu.chain = close.to_vec();
                        menu.highlights = highlights.clone();
                        ItemKind::Submenu {
                            open: menu.open,
                            menu: Some(menu),
                        }
                    }
                };
                if let Some(key) = spec.access_key {
                    keys.push(AccessKey {
                        key,
                        visible: spec.visible.clone(),
                        enabled: spec.enabled.clone(),
                        action: match &kind {
                            ItemKind::Action(action) => KeyAction::Run(Rc::clone(action)),
                            ItemKind::Toggle(checked) => KeyAction::Toggle(*checked),
                            ItemKind::Radio(radio) => KeyAction::Run(Rc::clone(&radio.select)),
                            ItemKind::Submenu { open, .. } => KeyAction::Open(*open),
                        },
                        close: Rc::clone(&close),
                        siblings: Rc::clone(&siblings),
                    });
                }
                let highlight = highlights.as_ref().map(|hub| {
                    let item = Rc::new(HighlightItem {
                        id: spec.id,
                        depth,
                        menu_open,
                        submenu_open: match &kind {
                            ItemKind::Submenu { open, .. } => Some(*open),
                            _ => None,
                        },
                        hovered: Cell::new(false),
                        focused: Cell::new(false),
                    });
                    hub.items.borrow_mut().push(Rc::clone(&item));
                    (Rc::clone(hub), item)
                });
                Box::new(MenuFlyoutItem {
                    common,
                    label: spec.label,
                    icon: spec.icon,
                    shortcut: spec.shortcut,
                    kind,
                    close: Rc::clone(&close),
                    siblings: Rc::clone(&siblings),
                    columns,
                    enabled,
                    press: Pressable::default(),
                    density: Density::Standard,
                    hover: Duration::ZERO,
                    highlight,
                })
            })
            .collect();
        self.keys = keys;
        children
    }

    fn layer(&self) -> Option<Layer> {
        let layer = Layer {
            arrows: true,
            min_size: Size::new(MENU_MIN_WIDTH, 0.0),
            ..Layer::default()
        };
        if self.chain.is_empty() {
            // O menu da barra deixa o ponteiro chegar aos outros itens dela.
            return Some(Layer {
                pass_through: self.bar.is_some(),
                ..layer
            });
        }
        // O submenu fica ao lado do item, com o primeiro item na altura dele, e deixa o
        // ponteiro chegar ao menu de onde saiu.
        Some(Layer {
            placement: Placement::Side,
            gap: MENU_PADDING.right,
            offset: Vec2::new(0.0, -MENU_PADDING.top),
            pass_through: true,
            ..layer
        })
    }

    fn set_open(&mut self, open: bool) {
        self.open.set(open);
    }

    fn layer_closed(&mut self) {
        if let Some(highlights) = &self.highlights {
            highlights.sync();
        }
    }

    fn handle(&mut self, event: &Event, cx: &mut EventCx) -> bool {
        if let Event::KeyDown(Key::Char(typed)) = *event {
            let m = cx.modifiers();
            return !(m.command() || m.alt || m.shift || m.meta) && self.press_access_key(typed);
        }
        let Event::KeyDown(key @ (Key::Left | Key::Right)) = *event else {
            return false;
        };
        if cx.modifiers().command() || cx.modifiers().alt {
            return false;
        }
        // ← que nenhum item usou fecha o submenu e devolve o foco ao item de onde ele saiu.
        if key == Key::Left && !self.chain.is_empty() {
            self.open.set(false);
            return true;
        }
        // Num menu da barra, ← e → trocam pelo menu do item vizinho, dando a volta.
        if let Some((menus, index)) = &self.bar
            && self.chain.is_empty()
        {
            let n = menus.len();
            let next = if key == Key::Left {
                (index + n - 1) % n
            } else {
                (index + 1) % n
            };
            crate::menu_bar::open_only(menus, next);
            return true;
        }
        false
    }

    fn accelerators_while_closed(&self) -> bool {
        true
    }
}

impl fmt::Debug for MenuFlyout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MenuFlyout")
            .field("common", &self.common)
            .field("entries", &self.entries.len())
            .finish_non_exhaustive()
    }
}

/// Menu de contexto guardado no [`Common`] até a montagem (`.context_flyout()`).
///
/// Dividido entre as cópias do `Common`: só a primeira montagem leva o menu.
#[derive(Clone)]
pub struct ContextFlyout(Rc<RefCell<Option<Box<dyn Widget>>>>);

impl ContextFlyout {
    pub(crate) fn new(menu: MenuFlyout) -> Self {
        Self(Rc::new(RefCell::new(Some(Box::new(menu)))))
    }

    /// Tira o menu, para a árvore montar.
    pub(crate) fn take(&self) -> Option<Box<dyn Widget>> {
        self.0.borrow_mut().take()
    }
}

impl fmt::Debug for ContextFlyout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ContextFlyout")
    }
}

/// Que item de menu é, para o `Drawer`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MenuItemKind {
    /// Item com uma ação (`MenuFlyoutItem`).
    #[default]
    Action,
    /// Item de marcar (`ToggleMenuFlyoutItem`), marcado ou não.
    Toggle {
        /// Marcado.
        checked: bool,
    },
    /// Item de rádio (`RadioMenuFlyoutItem`), marcado ou não.
    Radio {
        /// Marcado.
        checked: bool,
    },
    /// Item que abre um submenu (`MenuFlyoutSubItem`), com o chevron à direita.
    Submenu {
        /// Submenu aberto: o item fica com o fundo do hover.
        open: bool,
    },
}

/// Tudo o que o `Drawer` precisa saber de um item de menu para pintá-lo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct MenuItemVisual {
    /// Estado.
    pub state: ButtonState,
    /// Anel de foco: só com foco vindo do teclado.
    pub focus_ring: bool,
    /// Densidade: na compacta, o item tem 32 px, a altura do `MenuFlyoutItem` do WinUI.
    pub density: Density,
    /// Tipo do item.
    pub kind: MenuItemKind,
    /// Ícone à esquerda do texto.
    pub icon: Option<Symbol>,
    /// Atalho, escrito à direita ("Ctrl+N").
    pub shortcut: Option<Shortcut>,
    /// O menu tem um item de marcar: todos os itens deixam a coluna da marca (28 px).
    pub check_column: bool,
    /// O menu tem um item com ícone: todos os itens deixam a coluna do ícone (28 px).
    pub icon_column: bool,
}

/// Colunas que um menu reserva à esquerda do texto, iguais em todos os itens.
#[derive(Clone, Copy)]
struct MenuColumns {
    check: bool,
    icon: bool,
}

/// O que o item faz.
enum ItemKind {
    Action(Rc<RefCell<dyn FnMut()>>),
    Toggle(Signal<bool>),
    Radio(Radio),
    Submenu {
        open: Signal<bool>,
        /// Até a montagem.
        menu: Option<Box<MenuFlyout>>,
    },
}

/// Item de um [`MenuFlyout`].
struct MenuFlyoutItem {
    common: Common,
    label: Prop<String>,
    icon: Option<Symbol>,
    shortcut: Option<Prop<Shortcut>>,
    kind: ItemKind,
    /// Os `open` do menu do item e dos menus de onde ele saiu: escolher fecha todos.
    close: Rc<[Signal<bool>]>,
    /// Os `open` dos submenus do mesmo menu.
    siblings: Rc<[Signal<bool>]>,
    columns: MenuColumns,
    /// `.item_enabled()`: sem ele, o item está habilitado.
    enabled: Option<Prop<bool>>,
    press: Pressable,
    density: Density,
    /// Tempo com o ponteiro parado em cima.
    hover: Duration,
    /// O estado do realce do menu raiz e o deste item nele.
    highlight: Option<(Rc<Highlights>, Rc<HighlightItem>)>,
}

impl MenuFlyoutItem {
    /// Se o item está habilitado agora (rastreado).
    fn is_enabled(&self) -> bool {
        self.enabled.as_ref().is_none_or(Prop::get)
    }

    /// Fecha os submenus do mesmo menu, menos o deste item.
    fn close_siblings(&self) {
        let own = match self.kind {
            ItemKind::Submenu { open, .. } => Some(open),
            _ => None,
        };
        for &sibling in self.siblings.iter() {
            if Some(sibling) != own && sibling.get_untracked() {
                sibling.set(false);
            }
        }
    }

    fn open_submenu(&self) {
        if let ItemKind::Submenu { open, .. } = self.kind {
            open_submenu(open, &self.siblings);
        } else {
            self.close_siblings();
        }
    }

    /// Conta ao realce do menu o estado de ponteiro e foco do item.
    fn report(&self) {
        if let Some((highlights, item)) = &self.highlight {
            item.hovered.set(self.press.hovered());
            item.focused.set(self.press.focus_ring());
            highlights.sync();
        }
    }
}

/// Abre o submenu `open` e fecha os outros do mesmo menu.
fn open_submenu(open: Signal<bool>, siblings: &[Signal<bool>]) {
    for &sibling in siblings {
        if sibling != open && sibling.get_untracked() {
            sibling.set(false);
        }
    }
    if !open.get_untracked() {
        open.set(true);
    }
}

impl Widget for MenuFlyoutItem {
    fn kind(&self) -> &'static str {
        match self.kind {
            ItemKind::Action(_) => "menu_item",
            ItemKind::Toggle(_) => "toggle_menu_item",
            ItemKind::Radio(_) => "radio_menu_item",
            ItemKind::Submenu { .. } => "menu_sub_item",
        }
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        let kind = match &self.kind {
            ItemKind::Action(_) => MenuItemKind::Action,
            ItemKind::Toggle(checked) => MenuItemKind::Toggle {
                checked: checked.get(),
            },
            ItemKind::Radio(radio) => MenuItemKind::Radio {
                checked: (radio.checked)(),
            },
            ItemKind::Submenu { open, .. } => MenuItemKind::Submenu { open: open.get() },
        };
        Look::MenuItem {
            label: self.label.get(),
            visual: MenuItemVisual {
                state: if self.is_enabled() {
                    self.press.state()
                } else {
                    ButtonState::Disabled
                },
                focus_ring: self.press.focus_ring() && self.is_enabled(),
                density: self.density,
                kind,
                icon: self.icon,
                shortcut: self
                    .shortcut
                    .as_ref()
                    .map(Prop::get)
                    .filter(|shortcut| !shortcut.is_none()),
                check_column: self.columns.check,
                icon_column: self.columns.icon,
            },
        }
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        match &mut self.kind {
            ItemKind::Submenu { menu, .. } => menu
                .take()
                .map(|menu| -> Box<dyn Widget> { menu })
                .into_iter()
                .collect(),
            _ => Vec::new(),
        }
    }

    fn set_density(&mut self, density: Density) {
        self.density = density;
    }

    fn handle(&mut self, event: &Event, _: &mut EventCx) -> bool {
        if !untrack(|| self.is_enabled()) {
            // Desabilitado: o ponteiro e o teclado passam por cima, sem realce nem ação.
            self.hover = Duration::ZERO;
            return false;
        }
        let consumed = self.handle_event(event);
        self.report();
        consumed
    }

    fn interactive(&self) -> bool {
        true
    }

    fn focusable(&self) -> bool {
        self.is_enabled()
    }

    fn enabled(&self) -> bool {
        self.is_enabled()
    }

    fn animating(&self) -> bool {
        matches!(self.kind, ItemKind::Submenu { .. }) || !self.siblings.is_empty()
    }

    fn ticking(&self) -> bool {
        self.press.hovered() && self.hover < SUBMENU_DELAY && untrack(|| self.is_enabled())
    }

    fn tick(&mut self, dt: Duration) {
        self.hover += dt;
        if self.hover >= SUBMENU_DELAY {
            // Parado sobre o item: abre o submenu dele e fecha os dos outros itens.
            self.open_submenu();
            self.report();
        }
    }

    fn text(&self) -> Option<String> {
        Some(self.label.get())
    }
}

impl MenuFlyoutItem {
    fn handle_event(&mut self, event: &Event) -> bool {
        if let ItemKind::Submenu { .. } = self.kind
            && *event == Event::KeyDown(Key::Right)
            && self.press.is_focused()
        {
            self.open_submenu();
            return true;
        }
        let (consumed, clicked) = self.press.handle(event, true);
        if !self.press.hovered() {
            self.hover = Duration::ZERO;
        }
        if clicked {
            match &self.kind {
                ItemKind::Action(action) => {
                    let action = Rc::clone(action);
                    close_all(&self.close);
                    if let Ok(mut action) = action.try_borrow_mut() {
                        action();
                    }
                }
                ItemKind::Toggle(checked) => {
                    close_all(&self.close);
                    checked.update(|c| *c = !*c);
                }
                ItemKind::Radio(radio) => {
                    let select = Rc::clone(&radio.select);
                    close_all(&self.close);
                    if let Ok(mut select) = select.try_borrow_mut() {
                        select();
                    }
                }
                ItemKind::Submenu { .. } => self.open_submenu(),
            }
        }
        consumed
    }
}

/// Fecha um menu e os menus de onde ele saiu.
fn close_all(menus: &[Signal<bool>]) {
    for &open in menus {
        open.set(false);
    }
}

/// Linha entre grupos de itens de um [`MenuFlyout`].
struct MenuFlyoutSeparator {
    common: Common,
}

impl Widget for MenuFlyoutSeparator {
    fn kind(&self) -> &'static str {
        "menu_separator"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::MenuSeparator
    }
}
