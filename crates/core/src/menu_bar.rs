//! `MenuBar`: a barra de menus no alto da janela ("Arquivo", "Editar", "Exibir").

use std::fmt;
use std::rc::Rc;

use tomni_ui_reactive::{Get, GetUntracked, Set, Signal, untrack};

use crate::button::ButtonState;
use crate::combo_box::option_slug;
use crate::cx::{EventCx, FocusMove};
use crate::density::Density;
use crate::event::{Event, Key};
use crate::flyout::{FlyoutContent, MenuFlyout};
use crate::press::Pressable;
use crate::prop::{IntoProp, Prop};
use crate::shortcut::Shortcut;
use crate::widget::{
    Align, Common, Direction, Flex, Insets, Look, Widget, common_builders, density_builder,
};

/// Altura da barra (`MenuBarHeight`).
pub const MENU_BAR_HEIGHT: f64 = 40.0;
/// Altura da barra no compacto: a dos itens, sem a margem de cima e de baixo.
pub const MENU_BAR_HEIGHT_COMPACT: f64 = 32.0;
/// Margem de cada item (`MenuBarItemMargin`).
const ITEM_MARGIN: f64 = 4.0;

/// Tudo o que o `Drawer` precisa saber de um item da barra de menus para pintá-lo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct MenuBarItemVisual {
    /// Estado.
    pub state: ButtonState,
    /// Menu do item aberto: o fundo do pressionado (o estado `Selected` do WinUI).
    pub open: bool,
    /// Anel de foco: só com foco vindo do teclado.
    pub focus_ring: bool,
    /// Densidade.
    pub density: Density,
}

/// Um menu da barra, guardado até a montagem.
struct BarMenu {
    label: Prop<String>,
    menu: MenuFlyout,
    access_key: Option<char>,
}

/// Barra de menus, como a `MenuBar` do WinUI: um item por menu, cada um abrindo um
/// [`MenuFlyout`] embaixo dele.
///
/// ```
/// use tomni_ui_core::{MenuBar, MenuFlyout, Shortcut};
/// use tomni_ui_reactive::prelude::*;
///
/// # let scope = Scope::root();
/// # scope.run(|| {
/// let barra = MenuBar::new()
///     .id("barra")
///     .menu(
///         "Arquivo",
///         MenuFlyout::new()
///             .id("arquivo-menu")
///             .item("abrir", "Abrir...", || println!("abrir"))
///             .shortcut(Shortcut::ctrl('O'))
///             .separator()
///             .item("sair", "Sair", || println!("sair")),
///     )
///     .access_key('A')
///     .menu(
///         "Exibir",
///         MenuFlyout::new()
///             .id("exibir-menu")
///             .item("maior", "Aumentar o zoom", || println!("zoom")),
///     )
///     .access_key('E');
/// # });
/// # scope.clear();
/// ```
///
/// O clique num item abre o menu dele; com um menu aberto, passar o ponteiro sobre outro item
/// troca de menu, e ← e → também (dando a volta). O F10, ou o Alt pressionado e solto
/// sozinho, põe o foco no primeiro item; ali, ← e → andam pelos itens, e ↓, Enter ou Espaço
/// abrem o menu. O Esc fecha o menu e devolve o foco ao item; outro Esc (ou o F10, ou o Alt)
/// devolve o foco a quem tinha antes da barra. Com [`MenuBar::access_key`], o Alt e a letra
/// abrem o menu de qualquer lugar da janela.
///
/// Medidas do WinUI: 40 px de altura, itens com padding 10,4,10,4 e margem de 4 px, fundos
/// `SubtleFillColor*`. No compacto ([`Density::Compact`]), a barra tem 32 px, os itens sem a
/// margem de cima e de baixo; é uma adaptação, o WinUI não tem uma barra compacta. Os itens
/// têm 32 px de altura e ficam fora da regra `touch-target` do lint, como a `TitleBar`: a
/// barra de menus é feita para o mouse e o teclado, como no Windows. As dicas de tecla
/// (`KeyTip`) do WinUI ficam fora: a letra do Alt não aparece no item.
///
/// A barra é uma parada de Tab só (o primeiro item). Cada item tem o id `{id}-{rótulo}` (o
/// rótulo em minúsculas, com o que não é letra ou número trocado por `-`); os menus ficam com
/// os ids que você der a eles.
pub struct MenuBar {
    common: Common,
    menus: Vec<BarMenu>,
    density: Density,
}

impl MenuBar {
    /// Barra vazia.
    pub fn new() -> Self {
        Self {
            common: Common::default(),
            menus: Vec::new(),
            density: Density::Standard,
        }
    }

    /// Acrescenta um item com o rótulo `label` (fixo, closure ou signal), que abre `menu`.
    pub fn menu<M>(mut self, label: impl IntoProp<String, M>, menu: MenuFlyout) -> Self {
        self.menus.push(BarMenu {
            label: label.into_prop(),
            menu,
            access_key: None,
        });
        self
    }

    /// Faz o Alt e a letra `key` abrirem o menu do último item, de qualquer lugar da janela
    /// (o `AccessKey` do WinUI, sem a dica na tela).
    #[track_caller]
    pub fn access_key(mut self, key: char) -> Self {
        let Some(last) = self.menus.last_mut() else {
            panic!("`.access_key()` muda o último menu: chame depois de `.menu()`");
        };
        last.access_key = Some(key);
        self
    }
}

impl Default for MenuBar {
    fn default() -> Self {
        Self::new()
    }
}

common_builders!(MenuBar);
density_builder!(MenuBar);

/// Abre o menu `index` da barra e fecha os outros.
pub(crate) fn open_only(menus: &[Signal<bool>], index: usize) {
    for (i, &open) in menus.iter().enumerate() {
        if i != index && open.get_untracked() {
            open.set(false);
        }
    }
    if let Some(&open) = menus.get(index)
        && !open.get_untracked()
    {
        open.set(true);
    }
}

impl Widget for MenuBar {
    fn kind(&self) -> &'static str {
        "menu_bar"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::Row
    }

    fn flex(&self) -> Option<Flex> {
        let vertical = match self.density {
            Density::Standard => ITEM_MARGIN,
            Density::Compact => 0.0,
        };
        Some(Flex {
            direction: Direction::Row,
            gap: 2.0 * ITEM_MARGIN,
            padding: Insets::new(vertical, ITEM_MARGIN, vertical, ITEM_MARGIN),
            align: Align::Center,
            ..Flex::default()
        })
    }

    fn set_density(&mut self, density: Density) {
        self.density = density;
        self.common.height = Some(match density {
            Density::Standard => MENU_BAR_HEIGHT,
            Density::Compact => MENU_BAR_HEIGHT_COMPACT,
        });
    }

    fn names_children(&self) -> bool {
        true
    }

    fn focus_scope(&self) -> bool {
        true
    }

    fn menu_bar(&self) -> bool {
        true
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        let menus: Rc<[Signal<bool>]> = self.menus.iter().map(|m| m.menu.open_signal()).collect();
        std::mem::take(&mut self.menus)
            .into_iter()
            .enumerate()
            .map(|(index, bar_menu)| -> Box<dyn Widget> {
                let BarMenu {
                    label,
                    mut menu,
                    access_key,
                } = bar_menu;
                let id = self
                    .common
                    .id
                    .as_ref()
                    .map(|id| format!("{id}-{}", option_slug(&untrack(|| label.get()))));
                let mut common = Common::generated(id);
                if let Some(key) = access_key {
                    let menus = Rc::clone(&menus);
                    common.add_accelerator(Shortcut::alt(key), move || open_only(&menus, index));
                }
                menu.bar = Some((Rc::clone(&menus), index));
                Box::new(MenuBarItem {
                    common,
                    label,
                    open: menu.open_signal(),
                    menu: Some(menu),
                    menus: Rc::clone(&menus),
                    index,
                    press: Pressable::default(),
                    density: Density::Standard,
                })
            })
            .collect()
    }
}

impl fmt::Debug for MenuBar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MenuBar")
            .field("common", &self.common)
            .field("menus", &self.menus.len())
            .finish_non_exhaustive()
    }
}

/// Um item da [`MenuBar`]: o rótulo e, filho, o menu dele.
struct MenuBarItem {
    common: Common,
    label: Prop<String>,
    open: Signal<bool>,
    /// Até a montagem.
    menu: Option<MenuFlyout>,
    /// Os `open` de todos os menus da barra.
    menus: Rc<[Signal<bool>]>,
    index: usize,
    press: Pressable,
    density: Density,
}

impl MenuBarItem {
    fn open_menu(&self) {
        open_only(&self.menus, self.index);
    }
}

impl Widget for MenuBarItem {
    fn kind(&self) -> &'static str {
        "menu_bar_item"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::MenuBarItem {
            label: self.label.get(),
            visual: MenuBarItemVisual {
                state: if self.enabled() {
                    self.press.state()
                } else {
                    ButtonState::Disabled
                },
                open: self.open.get(),
                focus_ring: self.press.focus_ring(),
                density: self.density,
            },
        }
    }

    fn measured(&self) -> bool {
        true
    }

    fn set_density(&mut self, density: Density) {
        self.density = density;
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        self.menu
            .take()
            .map(|menu| Box::new(menu) as Box<dyn Widget>)
            .into_iter()
            .collect()
    }

    fn handle(&mut self, event: &Event, cx: &mut EventCx) -> bool {
        if !self.enabled() {
            self.press.reset();
            return false;
        }
        if let Event::KeyDown(key) = *event
            && self.press.is_focused()
            && !cx.modifiers().command()
            && !cx.modifiers().alt
        {
            match key {
                Key::Down => {
                    self.open_menu();
                    return true;
                }
                Key::Left | Key::Right => {
                    cx.move_focus(if key == Key::Left {
                        FocusMove::Prev
                    } else {
                        FocusMove::Next
                    });
                    return true;
                }
                _ => {}
            }
        }
        // Com o menu de outro item aberto, o ponteiro troca de menu.
        if let Event::PointerMove { kind, inside: true } = *event
            && kind.hovers()
            && !self.open.get_untracked()
            && self.menus.iter().any(|m| m.get_untracked())
        {
            self.open_menu();
        }
        let (consumed, clicked) = self.press.handle(event, true);
        if clicked {
            if self.open.get_untracked() {
                self.open.set(false);
            } else {
                self.open_menu();
            }
        }
        consumed
    }

    fn interactive(&self) -> bool {
        true
    }

    fn focusable(&self) -> bool {
        true
    }

    fn tab_stop(&self) -> bool {
        self.index == 0
    }

    fn text(&self) -> Option<String> {
        Some(self.label.get())
    }
}

impl fmt::Debug for MenuBarItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MenuBarItem")
            .field("common", &self.common)
            .field("index", &self.index)
            .finish_non_exhaustive()
    }
}
