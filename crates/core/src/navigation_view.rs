//! `NavigationView`: painel de navegação à esquerda e, ao lado, a página do item escolhido.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::rc::Rc;

use kurbo::{Rect, Size, Vec2};
use tomni_ui_reactive::{Get, GetUntracked, Set, Signal, effect, signal, untrack};

use crate::auto_suggest_box::AutoSuggestBox;
use crate::border::Border;
use crate::button::ButtonState;
use crate::color::Token;
use crate::combo_box::option_slug;
use crate::containers::{Column, Row};
use crate::cx::{EventCx, FocusMove};
use crate::density::Density;
use crate::event::{Event, Key};
use crate::press::Pressable;
use crate::prop::{IntoProp, Prop};
use crate::scroll_viewer::ScrollViewer;
use crate::symbol::Symbol;
use crate::text::Text;
use crate::title_bar::PaneLink;
use crate::widget::{
    Align, Common, Direction, Flex, Insets, Layer, Look, Placement, PopupKind, View, Widget,
    common_builders,
};

/// Altura de um item: o alvo de toque mínimo do lint, no lugar dos 36 px do WinUI
/// (`NavigationViewItemOnLeftMinHeight`).
pub const NAV_ITEM_HEIGHT: f64 = 44.0;
/// Altura de um item na densidade compacta: o `NavigationViewItemOnLeftMinHeight` do
/// `Compact.xaml` do WinUI. O ☰ e a lupa ficam com 44×32 px.
pub const NAV_ITEM_HEIGHT_COMPACT: f64 = 32.0;
/// Largura do painel aberto (`OpenPaneLength`).
pub const NAV_PANE_WIDTH: f64 = 320.0;
/// Largura do painel compacto (`CompactPaneLength`).
pub const NAV_COMPACT_WIDTH: f64 = 48.0;
/// Largura a partir da qual o modo é [`NavigationMode::Expanded`]
/// (`ExpandedModeThresholdWidth`).
pub const NAV_EXPANDED_MIN_WIDTH: f64 = 1008.0;
/// Largura a partir da qual o modo é [`NavigationMode::Compact`]
/// (`CompactModeThresholdWidth`).
pub const NAV_COMPACT_MIN_WIDTH: f64 = 641.0;
/// Recuo de cada nível de um grupo (o `c_itemIndentation` do WinUI).
pub const NAV_INDENT: f64 = 31.0;
/// Espaço entre a borda do painel e os itens.
const LIST_PADDING: Insets = Insets::new(4.0, 2.0, 4.0, 2.0);
/// Espaço entre dois itens (a margem 4,2 do `NavigationViewItemButtonMargin`).
const ITEM_GAP: f64 = 4.0;
/// Altura de um título de seção (`NavigationViewItemHeader`).
const HEADER_HEIGHT: f64 = 40.0;
/// Margem do título de seção e da busca (`NavigationViewItemHeaderMargin`,
/// `AutoSuggestBoxMargin`).
const SIDE_MARGIN: f64 = 16.0;
/// Margem do popup dos filhos de um grupo no painel compacto.
const FLYOUT_PADDING: Insets = Insets::new(3.0, 1.0, 3.0, 1.0);
/// Distância entre o item e o popup dos filhos dele: o resto do painel compacto e 4 px.
const FLYOUT_OFFSET: f64 = 2.0 + 4.0;
/// Chave e rótulo do item de [`NavigationView::settings`].
const SETTINGS_KEY: &str = "configuracoes";

/// Como o painel aparece. Sem [`NavigationView::pane_mode`], o modo vem da largura do
/// `NavigationView`, como o `PaneDisplayMode="Auto"` do WinUI.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum NavigationMode {
    /// Painel aberto ao lado do conteúdo, com ícones e rótulos; o ☰ o fecha até a largura
    /// compacta. A partir de [`NAV_EXPANDED_MIN_WIDTH`].
    #[default]
    Expanded,
    /// Só os ícones, numa faixa de [`NAV_COMPACT_WIDTH`]; o ☰ abre o painel por cima do
    /// conteúdo. De [`NAV_COMPACT_MIN_WIDTH`] até a largura do expandido.
    Compact,
    /// Só o ☰, no canto; ele abre o painel por cima do conteúdo. Abaixo de
    /// [`NAV_COMPACT_MIN_WIDTH`].
    Minimal,
}

impl NavigationMode {
    /// O modo para um `NavigationView` de `width` pixels de largura.
    pub fn for_width(width: f64) -> Self {
        if width >= NAV_EXPANDED_MIN_WIDTH {
            Self::Expanded
        } else if width >= NAV_COMPACT_MIN_WIDTH {
            Self::Compact
        } else {
            Self::Minimal
        }
    }

    /// Nome na árvore: `expanded`, `compact` ou `minimal`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Expanded => "expanded",
            Self::Compact => "compact",
            Self::Minimal => "minimal",
        }
    }
}

/// Tudo o que o `Drawer` e a árvore precisam saber do `NavigationView`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct NavigationViewVisual {
    /// Modo atual.
    pub mode: NavigationMode,
    /// Se o painel está aberto (largo, com os rótulos).
    pub open: bool,
}

/// Tudo o que o `Drawer` precisa saber de um item do `NavigationView` (e do ☰) para pintá-lo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct NavigationItemVisual {
    /// Se tem o indicador de escolhido: o próprio item ou, com os filhos escondidos, o grupo
    /// de um item escolhido.
    pub selected: bool,
    /// Estado.
    pub state: ButtonState,
    /// Anel de foco: só com foco vindo do teclado.
    pub focus_ring: bool,
    /// Só o ícone, sem o rótulo (o painel fechado e o ☰).
    pub compact: bool,
    /// Nível dentro dos grupos: 0 no primeiro.
    pub depth: u8,
    /// Num grupo, se está aberto; a seta aparece fora do painel compacto.
    pub expanded: Option<bool>,
    /// Densidade: no compacto, o item tem 32 px de altura.
    pub density: Density,
}

impl NavigationItemVisual {
    /// Altura do item: [`NAV_ITEM_HEIGHT`] ou, no compacto, [`NAV_ITEM_HEIGHT_COMPACT`].
    pub fn height(self) -> f64 {
        item_height(self.density)
    }
}

fn item_height(density: Density) -> f64 {
    match density {
        Density::Standard => NAV_ITEM_HEIGHT,
        Density::Compact => NAV_ITEM_HEIGHT_COMPACT,
    }
}

/// Item ou título de seção, guardado até a montagem.
enum Entry {
    Item {
        key: String,
        icon: Symbol,
        label: Prop<String>,
        page: Option<Box<dyn Widget>>,
        children: Vec<Entry>,
    },
    Header(Prop<String>),
}

/// Filhos de um grupo de [`NavigationView::group`].
///
/// ```
/// use tomni_ui_core::{NavigationView, Symbol, Text};
/// use tomni_ui_reactive::prelude::*;
///
/// # let scope = Scope::root();
/// # scope.run(|| {
/// let pagina = signal("fotos".to_string());
/// let nav = NavigationView::bind(pagina).id("nav").group(
///     "midia",
///     Symbol::Image,
///     "Mídia",
///     |g| {
///         g.item("fotos", Symbol::Camera, "Fotos", Text::new("Fotos"))
///             .item("videos", Symbol::Video, "Vídeos", Text::new("Vídeos"))
///     },
/// );
/// # });
/// # scope.clear();
/// ```
pub struct NavigationGroup {
    entries: Vec<Entry>,
}

impl NavigationGroup {
    /// Acrescenta um item: veja [`NavigationView::item`].
    pub fn item<M>(
        mut self,
        key: impl Into<String>,
        icon: Symbol,
        label: impl IntoProp<String, M>,
        page: impl View,
    ) -> Self {
        self.entries.push(Entry::Item {
            key: key.into(),
            icon,
            label: label.into_prop(),
            page: Some(Box::new(page)),
            children: Vec::new(),
        });
        self
    }

    /// Acrescenta um grupo dentro do grupo: veja [`NavigationView::group`].
    pub fn group<M>(
        mut self,
        key: impl Into<String>,
        icon: Symbol,
        label: impl IntoProp<String, M>,
        children: impl FnOnce(NavigationGroup) -> NavigationGroup,
    ) -> Self {
        self.entries.push(group_entry(key, icon, label, children));
        self
    }
}

impl fmt::Debug for NavigationGroup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NavigationGroup")
            .field("entries", &self.entries.len())
            .finish()
    }
}

fn group_entry<M>(
    key: impl Into<String>,
    icon: Symbol,
    label: impl IntoProp<String, M>,
    children: impl FnOnce(NavigationGroup) -> NavigationGroup,
) -> Entry {
    Entry::Item {
        key: key.into(),
        icon,
        label: label.into_prop(),
        page: None,
        children: children(NavigationGroup {
            entries: Vec::new(),
        })
        .entries,
    }
}

/// Um item na lista de [`Shared::items`].
struct Info {
    key: String,
    parent: Option<usize>,
    /// Nos grupos, se estão abertos.
    expanded: Option<Signal<bool>>,
}

/// Estado comum ao `NavigationView` e às partes que ele monta.
struct Shared {
    selected: Signal<String>,
    mode: Signal<NavigationMode>,
    open: Signal<bool>,
    width: Signal<f64>,
    /// Se o ☰ está na `TitleBar`: o do painel e o espaço dele no modo mínimo somem.
    external: Signal<bool>,
    items: RefCell<Vec<Info>>,
    /// Chaves dos itens com página, na ordem.
    pages: RefCell<Vec<String>>,
    /// Densidade do `NavigationView`, que vale para todos os itens.
    density: Cell<Density>,
}

impl Shared {
    /// Chave da página à mostra: a escolhida ou, se ela não é de nenhum item com página, a
    /// primeira.
    fn effective(&self) -> String {
        let value = self.selected.get();
        let pages = self.pages.borrow();
        if pages.contains(&value) {
            value
        } else {
            pages.first().cloned().unwrap_or_default()
        }
    }

    /// Altura de um item na densidade do `NavigationView`.
    fn item_height(&self) -> f64 {
        item_height(self.density.get())
    }

    /// Altura da faixa do ☰ (e do botão de busca no painel compacto): o botão e 2 px em
    /// volta.
    fn button_row_height(&self) -> f64 {
        self.item_height() + 4.0
    }

    /// Se o painel está estreito, só com os ícones.
    fn compact(&self) -> bool {
        !self.open.get()
    }

    /// Se o painel está aberto por cima do conteúdo.
    fn overlay(&self) -> bool {
        self.mode.get() != NavigationMode::Expanded && self.open.get()
    }

    /// Largura do lugar do painel ao lado do conteúdo.
    fn slot_width(&self) -> f64 {
        match self.mode.get() {
            NavigationMode::Expanded if self.open.get() => NAV_PANE_WIDTH,
            NavigationMode::Expanded | NavigationMode::Compact => NAV_COMPACT_WIDTH,
            NavigationMode::Minimal => 0.0,
        }
    }

    /// Largura do painel: pode passar do lugar dele, por cima do conteúdo.
    fn pane_width(&self) -> f64 {
        if self.open.get() {
            NAV_PANE_WIDTH
        } else {
            NAV_COMPACT_WIDTH
        }
    }

    /// O item que leva o indicador da escolha: o escolhido ou, se ele está escondido, o
    /// grupo aberto mais fundo que o contém. `below` limita aos itens dentro desse grupo
    /// (o popup dos filhos no painel compacto).
    fn representative(&self, compact: bool, below: Option<usize>) -> Option<usize> {
        let key = self.effective();
        let items = self.items.borrow();
        let target = items.iter().position(|i| i.key == key)?;
        let mut chain = vec![target];
        while let Some(parent) = items[*chain.last()?].parent {
            if Some(parent) == below {
                break;
            }
            chain.push(parent);
        }
        if below.is_some() && items[*chain.last()?].parent != below {
            return None;
        }
        chain.reverse();
        let last = chain.len() - 1;
        chain.iter().enumerate().find_map(|(n, &i)| {
            let hides = compact || items[i].expanded.is_some_and(|e| !e.get());
            (n == last || hides).then_some(i)
        })
    }

    /// Escolhe o item `key`: fecha o painel aberto por cima do conteúdo.
    fn choose(&self, key: &str) {
        if self.selected.get_untracked() != key {
            self.selected.set(key.to_owned());
        }
        if self.mode.get_untracked() != NavigationMode::Expanded && self.open.get_untracked() {
            self.open.set(false);
        }
    }
}

/// Painel de navegação à esquerda e, ao lado, a página do item escolhido, ligado a um
/// `Signal<String>` com a chave dele.
///
/// ```
/// use tomni_ui_core::{Form, NavigationView, Symbol, Text};
/// use tomni_ui_reactive::prelude::*;
///
/// # let scope = Scope::root();
/// # scope.run(|| {
/// let pagina = signal("inicio".to_string());
/// let tela = Form::new("Biblioteca").child(
///     NavigationView::bind(pagina)
///         .id("nav")
///         .item("inicio", Symbol::Home, "Início", Text::new("Bem-vindo."))
///         .header("Coleção")
///         .item("livros", Symbol::Book, "Livros", Text::new("Livros."))
///         .group("midia", Symbol::Image, "Mídia", |g| {
///             g.item("fotos", Symbol::Camera, "Fotos", Text::new("Fotos."))
///         })
///         .settings(Text::new("Preferências.")),
/// );
/// # });
/// # scope.clear();
/// ```
///
/// O modo vem da largura do próprio `NavigationView`, que precisa ocupar a janela (filho do
/// `Form`, como acima): [`NavigationMode::Expanded`] a partir de 1008 px, com o painel de 320
/// ao lado do conteúdo; [`NavigationMode::Compact`] de 641 a 1007, só com os ícones numa
/// faixa de 48; e [`NavigationMode::Minimal`] abaixo, só com o ☰ no canto. No expandido, o ☰
/// fecha e abre o painel ao lado do conteúdo; nos outros dois, abre o painel por cima dele,
/// que fecha com Esc, com um clique fora ou ao escolher um item. [`NavigationView::pane_mode`]
/// fixa um modo.
///
/// Um clique, Enter ou Espaço num item o escolhem; num grupo, abrem e fecham os filhos (no
/// painel compacto, num popup ao lado). Os itens são uma parada só de Tab (o escolhido); ↑ e
/// ↓ andam por eles, Home e End vão ao primeiro e ao último, ← fecha o grupo (ou vai ao
/// grupo de cima) e → abre. Ao contrário das abas do `TabView`, a escolha não segue o foco.
/// No painel compacto, o rótulo de cada item vira a dica dele.
///
/// Cada item tem o id `{id}-{chave}` e a página dele, `{id}-{chave}-conteudo` (a chave em
/// minúsculas, com o que não é letra ou número trocado por `-`); as cópias dos filhos no popup
/// do painel compacto, `{id}-{chave}-flyout`. O ☰ é `{id}-alternar`, o botão de busca do
/// painel compacto, `{id}-pesquisar`, e o item de [`NavigationView::settings`], de chave
/// `configuracoes`, é `{id}-configuracoes`. As páginas dos outros itens continuam montadas,
/// só escondidas: o estado delas não se perde ao trocar de item. Uma chave que não é de
/// nenhum item com página mostra a primeira.
pub struct NavigationView {
    common: Common,
    shared: Rc<Shared>,
    /// O modo pedido por `pane_mode`/`pane_mode_with`; `None` escolhe pela largura.
    fixed: Rc<Cell<Option<NavigationMode>>>,
    menu: Vec<Entry>,
    footer: Vec<Entry>,
    search: Option<AutoSuggestBox>,
}

impl NavigationView {
    /// `NavigationView` sem itens, mostrando a página do item de chave `selected`. Ocupa o
    /// espaço que sobra no pai (`.grow(1)`).
    pub fn bind(selected: Signal<String>) -> Self {
        Self {
            common: Common {
                grow: 1.0,
                ..Common::default()
            },
            shared: Rc::new(Shared {
                selected,
                mode: signal(NavigationMode::Expanded),
                open: signal(true),
                width: signal(0.0),
                external: signal(false),
                items: RefCell::new(Vec::new()),
                pages: RefCell::new(Vec::new()),
                density: Cell::new(Density::Standard),
            }),
            fixed: Rc::new(Cell::new(None)),
            menu: Vec::new(),
            footer: Vec::new(),
            search: None,
        }
    }

    /// Acrescenta um item no fim da lista: `key` identifica o item no signal e no id, `icon`
    /// aparece sempre, `label` (fixo, closure ou signal) com o painel aberto e `page` é o que
    /// ele mostra.
    pub fn item<M>(
        mut self,
        key: impl Into<String>,
        icon: Symbol,
        label: impl IntoProp<String, M>,
        page: impl View,
    ) -> Self {
        self.menu.push(Entry::Item {
            key: key.into(),
            icon,
            label: label.into_prop(),
            page: Some(Box::new(page)),
            children: Vec::new(),
        });
        self
    }

    /// Acrescenta um grupo no fim da lista: um item sem página, com a seta, que abre e fecha
    /// os filhos que `children` acrescenta. Começa fechado.
    pub fn group<M>(
        mut self,
        key: impl Into<String>,
        icon: Symbol,
        label: impl IntoProp<String, M>,
        children: impl FnOnce(NavigationGroup) -> NavigationGroup,
    ) -> Self {
        self.menu.push(group_entry(key, icon, label, children));
        self
    }

    /// Acrescenta um título de seção (fixo, closure ou signal) no fim da lista. Só aparece com
    /// o painel aberto.
    pub fn header<M>(mut self, text: impl IntoProp<String, M>) -> Self {
        self.menu.push(Entry::Header(text.into_prop()));
        self
    }

    /// Acrescenta um item no rodapé do painel, embaixo da lista, como em
    /// [`NavigationView::item`].
    pub fn footer_item<M>(
        mut self,
        key: impl Into<String>,
        icon: Symbol,
        label: impl IntoProp<String, M>,
        page: impl View,
    ) -> Self {
        self.footer.push(Entry::Item {
            key: key.into(),
            icon,
            label: label.into_prop(),
            page: Some(Box::new(page)),
            children: Vec::new(),
        });
        self
    }

    /// Acrescenta o item "Configurações", com a engrenagem, no fim do rodapé, mostrando
    /// `page`. A chave dele é `configuracoes`.
    pub fn settings(self, page: impl View) -> Self {
        self.footer_item(
            SETTINGS_KEY,
            Symbol::Settings,
            crate::strings().navigation_settings.to_string(),
            page,
        )
    }

    /// Caixa de busca no alto do painel, com o painel aberto; no compacto, vira um botão com a
    /// lupa que abre o painel. Como em qualquer `AutoSuggestBox`, use `.height(44)`.
    pub fn search(mut self, search: AutoSuggestBox) -> Self {
        self.search = Some(search);
        self
    }

    /// Fixa o modo, em vez de escolher pela largura.
    pub fn pane_mode(self, mode: NavigationMode) -> Self {
        self.pane_mode_with(Some(mode))
    }

    /// O modo do painel vindo de um valor, closure ou signal: `None` escolhe pela largura, como
    /// sem `pane_mode`. Quando o valor muda, o painel muda de modo sem recriar o widget (e o
    /// painel aberto por cima do conteúdo fecha, como em qualquer troca de modo).
    pub fn pane_mode_with<M>(self, mode: impl IntoProp<Option<NavigationMode>, M>) -> Self {
        let mode = mode.into_prop();
        let shared = Rc::clone(&self.shared);
        let fixed = Rc::clone(&self.fixed);
        let apply = Rc::new(move |wanted: Option<NavigationMode>| {
            fixed.set(wanted);
            // Sem largura medida (antes do primeiro quadro), `None` espera o `resized`.
            let width = shared.width.get_untracked();
            let Some(wanted) =
                wanted.or_else(|| (width > 0.0).then(|| NavigationMode::for_width(width)))
            else {
                return;
            };
            if shared.mode.get_untracked() != wanted {
                shared.mode.set(wanted);
                shared.open.set(wanted == NavigationMode::Expanded);
            }
        });
        // O valor vale já na construção, sem esperar o primeiro efeito.
        apply(untrack(|| mode.get()));
        effect(move || apply(mode.get()));
        self
    }

    /// Chave da página à mostra: a escolhida ou, se ela não é de nenhum item com página, a
    /// primeira.
    pub fn selected(&self) -> String {
        self.shared.effective()
    }

    /// Modo atual.
    pub fn mode(&self) -> NavigationMode {
        self.shared.mode.get()
    }

    /// Se o painel está aberto (largo, com os rótulos).
    pub fn is_pane_open(&self) -> bool {
        self.shared.open.get()
    }

    /// Estado para o `Drawer` e a árvore.
    pub fn visual(&self) -> NavigationViewVisual {
        NavigationViewVisual {
            mode: self.shared.mode.get(),
            open: self.shared.open.get(),
        }
    }
}

common_builders!(NavigationView);

/// Um item já registrado em [`Shared::items`], para montar o original e as cópias.
struct Spec {
    index: usize,
    key: String,
    icon: Symbol,
    label: Prop<String>,
    children: Vec<Spec>,
}

/// O que vai na lista: um item ou um título de seção.
enum Line {
    Item(Spec),
    Header(Prop<String>),
}

/// Monta as partes do `NavigationView`, com os ids derivados do dele.
struct Builder {
    id: Option<String>,
    shared: Rc<Shared>,
    pages: Vec<(String, Box<dyn Widget>)>,
}

impl Builder {
    fn sub(&self, suffix: &str) -> Option<String> {
        self.id.as_ref().map(|id| format!("{id}-{suffix}"))
    }

    /// Registra os itens em [`Shared::items`] e guarda as páginas.
    fn register(&mut self, entries: Vec<Entry>, parent: Option<usize>) -> Vec<Line> {
        entries
            .into_iter()
            .map(|entry| match entry {
                Entry::Header(text) => Line::Header(text),
                Entry::Item {
                    key,
                    icon,
                    label,
                    page,
                    children,
                } => {
                    let index = {
                        let mut items = self.shared.items.borrow_mut();
                        items.push(Info {
                            key: key.clone(),
                            parent,
                            expanded: page.is_none().then(|| signal(false)),
                        });
                        items.len() - 1
                    };
                    if let Some(page) = page {
                        self.shared.pages.borrow_mut().push(key.clone());
                        self.pages.push((key.clone(), page));
                    }
                    let children = self
                        .register(children, Some(index))
                        .into_iter()
                        .filter_map(|line| match line {
                            Line::Item(spec) => Some(spec),
                            Line::Header(_) => None,
                        })
                        .collect();
                    Line::Item(Spec {
                        index,
                        key,
                        icon,
                        label,
                        children,
                    })
                }
            })
            .collect()
    }

    fn expanded(&self, index: usize) -> Option<Signal<bool>> {
        self.shared.items.borrow()[index].expanded
    }

    /// O item `spec` no painel (`copy` é `None`) ou no popup dos filhos do grupo `copy`.
    fn item(&self, spec: &Spec, depth: u8, copy: Option<FlyoutCopy>) -> Box<dyn Widget> {
        let slug = option_slug(&spec.key);
        let expanded = self.expanded(spec.index);
        let mut children: Vec<Box<dyn Widget>> = Vec::new();
        let mut flyout = None;
        if let Some(open) = expanded {
            let shared = Rc::clone(&self.shared);
            let in_flyout = copy.is_some();
            let mut column = Column::new()
                .padding(Insets::new(ITEM_GAP, 0.0, 0.0, 0.0))
                .gap(ITEM_GAP)
                .visible(move || open.get() && (in_flyout || !shared.compact()));
            for child in &spec.children {
                column = column.boxed(self.item(child, depth + 1, copy));
            }
            children.push(Box::new(column));
            if copy.is_none() && depth == 0 {
                let flyout_open = signal(false);
                let mut list = Column::new().gap(ITEM_GAP);
                let copy = FlyoutCopy {
                    flyout: flyout_open,
                    group: spec.index,
                };
                for child in &spec.children {
                    list = list.boxed(self.item(child, 0, Some(copy)));
                }
                children.push(Box::new(GroupFlyout {
                    common: Common::generated(self.sub(&format!("{slug}-filhos"))),
                    shared: Rc::clone(&self.shared),
                    open: flyout_open,
                    child: Some(Box::new(list)),
                }));
                flyout = Some(flyout_open);
            }
        }
        let tooltip = copy.is_none().then(|| {
            let shared = Rc::clone(&self.shared);
            let label = spec.label.clone();
            Prop::Dynamic(Rc::new(move || {
                if shared.compact() {
                    label.get()
                } else {
                    String::new()
                }
            }))
        });
        let suffix = if copy.is_some() {
            format!("{slug}-flyout")
        } else {
            slug
        };
        Box::new(NavItem {
            common: Common {
                id: self.sub(&suffix),
                tooltip,
                generated: true,
                ..Common::default()
            },
            shared: Rc::clone(&self.shared),
            index: spec.index,
            key: spec.key.clone(),
            label: spec.label.clone(),
            icon: spec.icon,
            depth,
            expanded,
            flyout,
            copy,
            children,
            press: Pressable::default(),
        })
    }

    /// A lista: itens e títulos de seção.
    fn list(&self, lines: &[Line], padding: Insets) -> Column {
        let mut column = Column::new().padding(padding).gap(ITEM_GAP);
        for line in lines {
            match line {
                Line::Item(spec) => column = column.boxed(self.item(spec, 0, None)),
                Line::Header(text) => {
                    let text = text.clone();
                    let shared = Rc::clone(&self.shared);
                    column = column.child(
                        Row::new()
                            .height(HEADER_HEIGHT)
                            .padding(Insets::new(0.0, SIDE_MARGIN, 0.0, SIDE_MARGIN))
                            .align(Align::Center)
                            .visible(move || !shared.compact())
                            .child(Text::new(move || text.get()).body_strong().secondary()),
                    );
                }
            }
        }
        column
    }
}

/// Uma cópia de item no popup dos filhos de um grupo, no painel compacto.
#[derive(Clone, Copy)]
struct FlyoutCopy {
    /// Abre e fecha o popup.
    flyout: Signal<bool>,
    /// O grupo de onde vieram os filhos.
    group: usize,
}

impl Widget for NavigationView {
    fn kind(&self) -> &'static str {
        "navigation_view"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn set_density(&mut self, density: Density) {
        self.shared.density.set(density);
    }

    fn names_children(&self) -> bool {
        true
    }

    fn look(&self) -> Look {
        Look::NavigationView(self.visual())
    }

    fn flex(&self) -> Option<Flex> {
        Some(Flex {
            direction: Direction::Row,
            ..Flex::default()
        })
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        let mut builder = Builder {
            id: self.common.id.clone(),
            shared: Rc::clone(&self.shared),
            pages: Vec::new(),
        };
        let menu = builder.register(std::mem::take(&mut self.menu), None);
        let footer = builder.register(std::mem::take(&mut self.footer), None);
        let shared = &self.shared;
        let row_height = shared.button_row_height();

        // O painel: o ☰, a busca e as listas.
        let s = Rc::clone(shared);
        let toggle_row = Row::new()
            .height(row_height)
            .padding(2)
            .visible(move || !s.external.get())
            .child(PaneButton::new(&builder, PaneAction::Toggle));
        let mut pane = Part::new(Direction::Column, shared, "navigation_pane")
            .id(builder.sub("painel"))
            .width_from(Shared::pane_width)
            .look_from(|s| Look::NavigationPane {
                overlay: s.overlay(),
            });
        pane.children.push(Box::new(toggle_row));
        if let Some(search) = self.search.take() {
            let s = Rc::clone(shared);
            pane.children.push(Box::new(
                Row::new()
                    .height(row_height)
                    .padding(2)
                    .visible(move || s.compact() && s.mode.get() != NavigationMode::Minimal)
                    .child(PaneButton::new(&builder, PaneAction::Search)),
            ));
            let s = Rc::clone(shared);
            pane.children.push(Box::new(
                Row::new()
                    .padding(Insets::new(2.0, SIDE_MARGIN, 6.0, SIDE_MARGIN))
                    .visible(move || !s.compact())
                    .child(search.grow(1)),
            ));
        }
        let mut items = ScrollViewer::new()
            .grow(1)
            .child(builder.list(&menu, LIST_PADDING))
            .generated();
        if let Some(id) = builder.sub("itens") {
            items = items.id(id);
        }
        let mut scope = Part::new(Direction::Column, shared, "column");
        scope.common.grow = 1.0;
        scope.scope = true;
        scope.children.push(Box::new(items));
        if !footer.is_empty() {
            if !menu.is_empty() {
                scope.children.push(Box::new(
                    Column::new()
                        .padding(Insets::new(3.0, 0.0, 4.0, 0.0))
                        .child(
                            Border::new()
                                .height(1)
                                .background(Token::DividerStrokeColorDefault),
                        ),
                ));
            }
            scope.children.push(Box::new(builder.list(
                &footer,
                Insets::new(
                    0.0,
                    LIST_PADDING.right,
                    LIST_PADDING.bottom,
                    LIST_PADDING.left,
                ),
            )));
        }
        let s = Rc::clone(shared);
        scope.common.visible = Some(Prop::Dynamic(Rc::new(move || {
            s.mode.get() != NavigationMode::Minimal || s.open.get()
        })));
        pane.children.push(Box::new(scope));

        // O lugar do painel ao lado do conteúdo, por cima dele.
        let mut slot = Part::new(Direction::Row, shared, "row").width_from(Shared::slot_width);
        slot.raised = true;
        slot.children.push(Box::new(pane));
        slot.children.push(Box::new(Veil {
            common: Common::generated(builder.sub("fora")),
            shared: Rc::clone(shared),
        }));

        // O conteúdo, com as páginas.
        let mut content = Part::new(Direction::Column, shared, "navigation_content")
            .id(builder.sub("conteudo"))
            .look_from(|s| Look::NavigationContent {
                minimal: s.mode.get() == NavigationMode::Minimal,
            });
        content.common.grow = 1.0;
        content.flex.padding = Insets::new(1.0, 0.0, 0.0, 1.0);
        for (key, page) in std::mem::take(&mut builder.pages) {
            let s = Rc::clone(shared);
            let shown = key.clone();
            let mut panel = Column::new()
                .grow(1)
                .visible(move || s.effective() == shown)
                .boxed(page);
            if let Some(id) = builder.sub(&format!("{}-conteudo", option_slug(&key))) {
                panel = panel.id(id);
            }
            content.children.push(Box::new(panel));
        }
        let s = Rc::clone(shared);
        let main = Column::new()
            .grow(1)
            .child(
                Column::new()
                    .height(row_height)
                    .visible(move || s.mode.get() == NavigationMode::Minimal && !s.external.get()),
            )
            .boxed(Box::new(content));
        vec![Box::new(slot), Box::new(main)]
    }

    fn handle(&mut self, event: &Event, cx: &mut EventCx) -> bool {
        // Chega aqui o Esc que nenhum widget de dentro usou.
        if *event == Event::KeyDown(Key::Escape) && self.shared.overlay() {
            self.shared.open.set(false);
            cx.move_focus(FocusMove::Inside);
            return true;
        }
        false
    }

    fn text(&self) -> Option<String> {
        Some(self.selected())
    }

    fn pane_link(&self) -> Option<PaneLink> {
        Some(PaneLink::new(self.shared.open, self.shared.external))
    }

    fn resized(&mut self, size: Size) {
        let shared = &self.shared;
        if shared.width.get_untracked() != size.width {
            shared.width.set(size.width);
        }
        let mode = self
            .fixed
            .get()
            .unwrap_or_else(|| NavigationMode::for_width(size.width));
        if shared.mode.get_untracked() != mode {
            shared.mode.set(mode);
            shared.open.set(mode == NavigationMode::Expanded);
        }
    }
}

impl fmt::Debug for NavigationView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NavigationView")
            .field("common", &self.common)
            .field("fixed", &self.fixed.get())
            .field("menu", &self.menu.len())
            .field("footer", &self.footer.len())
            .finish_non_exhaustive()
    }
}

/// Contêiner que o `NavigationView` monta: o lugar do painel, o painel, a lista dos itens
/// e o conteúdo, com a largura e o visual que mudam com o modo.
struct Part {
    common: Common,
    kind: &'static str,
    flex: Flex,
    shared: Rc<Shared>,
    children: Vec<Box<dyn Widget>>,
    width: Option<fn(&Shared) -> f64>,
    look: Option<fn(&Shared) -> Look>,
    raised: bool,
    scope: bool,
}

impl Part {
    fn new(direction: Direction, shared: &Rc<Shared>, kind: &'static str) -> Self {
        Self {
            common: Common::default(),
            kind,
            flex: Flex {
                direction,
                ..Flex::default()
            },
            shared: Rc::clone(shared),
            children: Vec::new(),
            width: None,
            look: None,
            raised: false,
            scope: false,
        }
    }

    fn id(mut self, id: Option<String>) -> Self {
        self.common.id = id;
        self
    }

    fn width_from(mut self, width: fn(&Shared) -> f64) -> Self {
        self.width = Some(width);
        self
    }

    fn look_from(mut self, look: fn(&Shared) -> Look) -> Self {
        self.look = Some(look);
        self
    }
}

impl Widget for Part {
    fn kind(&self) -> &'static str {
        self.kind
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        match self.look {
            Some(look) => look(&self.shared),
            None if self.flex.direction == Direction::Row => Look::Row,
            None => Look::Column,
        }
    }

    fn flex(&self) -> Option<Flex> {
        Some(self.flex)
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        std::mem::take(&mut self.children)
    }

    fn dynamic_width(&self) -> Option<f64> {
        self.width.map(|width| width(&self.shared))
    }

    fn raised(&self) -> bool {
        self.raised
    }

    fn focus_scope(&self) -> bool {
        self.scope
    }
}

/// Área transparente ao lado do painel aberto por cima do conteúdo: um clique nela fecha o
/// painel.
struct Veil {
    common: Common,
    shared: Rc<Shared>,
}

impl Widget for Veil {
    fn kind(&self) -> &'static str {
        "navigation_dismiss"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn visible(&self) -> bool {
        self.shared.overlay()
    }

    fn look(&self) -> Look {
        Look::Row
    }

    /// Contêiner vazio, e não folha: passa da janela à direita, para cobrir o conteúdo todo.
    fn flex(&self) -> Option<Flex> {
        Some(Flex::default())
    }

    fn handle(&mut self, event: &Event, _: &mut EventCx) -> bool {
        match event {
            Event::PointerDown { .. } => {
                self.shared.open.set(false);
                true
            }
            Event::PointerMove { .. } | Event::PointerUp { .. } | Event::PointerCancel => true,
            _ => false,
        }
    }

    fn interactive(&self) -> bool {
        true
    }

    fn dynamic_width(&self) -> Option<f64> {
        Some(self.shared.width.get())
    }
}

/// O que um botão da faixa do alto do painel faz.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PaneAction {
    /// ☰: abre e fecha o painel.
    Toggle,
    /// Lupa, no painel compacto: abre o painel, com a busca.
    Search,
}

/// O ☰ e a lupa do painel compacto.
struct PaneButton {
    common: Common,
    shared: Rc<Shared>,
    action: PaneAction,
    press: Pressable,
}

impl PaneButton {
    fn new(builder: &Builder, action: PaneAction) -> Self {
        let shared = Rc::clone(&builder.shared);
        let tooltip = {
            let shared = Rc::clone(&shared);
            Prop::Dynamic(Rc::new(move || label(&shared, action)))
        };
        let suffix = match action {
            PaneAction::Toggle => "alternar",
            PaneAction::Search => "pesquisar",
        };
        Self {
            common: Common {
                id: builder.sub(suffix),
                width: Some(NAV_ITEM_HEIGHT),
                height: Some(shared.item_height()),
                tooltip: Some(tooltip),
                generated: true,
                ..Common::default()
            },
            shared,
            action,
            press: Pressable::default(),
        }
    }
}

/// Rótulo (e dica) de um botão da faixa do alto do painel.
fn label(shared: &Shared, action: PaneAction) -> String {
    let strings = crate::strings();
    match action {
        PaneAction::Toggle if shared.open.get() => strings.navigation_close.to_string(),
        PaneAction::Toggle => strings.navigation_open.to_string(),
        PaneAction::Search => strings.navigation_search.to_string(),
    }
}

impl Widget for PaneButton {
    fn kind(&self) -> &'static str {
        match self.action {
            PaneAction::Toggle => "navigation_toggle",
            PaneAction::Search => "navigation_search",
        }
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::NavigationItem {
            label: label(&self.shared, self.action),
            icon: match self.action {
                PaneAction::Toggle => Symbol::Navigation,
                PaneAction::Search => Symbol::Search,
            },
            visual: NavigationItemVisual {
                state: self.press.state(),
                focus_ring: self.press.focus_ring(),
                compact: true,
                density: self.shared.density.get(),
                ..NavigationItemVisual::default()
            },
        }
    }

    fn handle(&mut self, event: &Event, cx: &mut EventCx) -> bool {
        let (consumed, clicked) = self.press.handle(event, true);
        if clicked {
            let open = self.shared.open.get_untracked();
            match self.action {
                PaneAction::Toggle => self.shared.open.set(!open),
                // Como no WinUI, a lupa abre o painel e põe o foco na caixa de busca, que vem
                // logo depois dela.
                PaneAction::Search => {
                    if !open {
                        self.shared.open.set(true);
                    }
                    cx.move_focus(FocusMove::Forward);
                }
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

    fn text(&self) -> Option<String> {
        Some(label(&self.shared, self.action))
    }
}

/// Um item do `NavigationView`: o original, no painel, ou uma cópia, no popup dos filhos de
/// um grupo no painel compacto.
struct NavItem {
    common: Common,
    shared: Rc<Shared>,
    index: usize,
    key: String,
    label: Prop<String>,
    icon: Symbol,
    depth: u8,
    /// Nos grupos, se estão abertos.
    expanded: Option<Signal<bool>>,
    /// Num grupo do primeiro nível, o popup dos filhos no painel compacto.
    flyout: Option<Signal<bool>>,
    copy: Option<FlyoutCopy>,
    children: Vec<Box<dyn Widget>>,
    press: Pressable,
}

impl NavItem {
    /// Só o ícone: no painel compacto (nunca no popup).
    fn compact(&self) -> bool {
        self.copy.is_none() && self.shared.compact()
    }

    fn selected(&self) -> bool {
        let representative = match self.copy {
            Some(copy) => self.shared.representative(false, Some(copy.group)),
            None => self.shared.representative(self.shared.compact(), None),
        };
        representative == Some(self.index)
    }

    fn visual(&self) -> NavigationItemVisual {
        let compact = self.compact();
        NavigationItemVisual {
            selected: self.selected(),
            state: self.press.state(),
            focus_ring: self.press.focus_ring(),
            compact,
            depth: self.depth,
            expanded: self.expanded.map(|e| e.get()),
            density: self.shared.density.get(),
        }
    }

    /// Clique, Enter ou Espaço: abre ou fecha o grupo (no painel compacto, o popup), ou
    /// escolhe o item.
    fn activate(&self) {
        match (self.expanded, self.flyout) {
            (Some(_), Some(flyout)) if self.compact() => flyout.set(true),
            (Some(expanded), _) => expanded.set(!expanded.get_untracked()),
            (None, _) => {
                self.shared.choose(&self.key);
                if let Some(copy) = self.copy {
                    copy.flyout.set(false);
                }
            }
        }
    }

    /// ← e →, com o foco no item.
    fn arrow(&self, key: Key, cx: &mut EventCx) -> bool {
        let compact = untrack_compact(self);
        match (key, self.expanded) {
            (Key::Left, Some(expanded)) if !compact && expanded.get_untracked() => {
                expanded.set(false);
            }
            (Key::Left, _) if self.depth > 0 => cx.move_focus(FocusMove::Parent),
            (Key::Left, _) => match self.copy {
                Some(copy) => copy.flyout.set(false),
                None => return false,
            },
            (Key::Right, Some(expanded)) => match self.flyout {
                Some(flyout) if compact => flyout.set(true),
                _ if !expanded.get_untracked() => expanded.set(true),
                _ => {}
            },
            _ => return false,
        }
        true
    }
}

/// [`NavItem::compact`] sem registrar dependência.
fn untrack_compact(item: &NavItem) -> bool {
    item.copy.is_none() && !item.shared.open.get_untracked()
}

impl Widget for NavItem {
    fn kind(&self) -> &'static str {
        "navigation_item"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::NavigationItem {
            label: self.label.get(),
            icon: self.icon,
            visual: self.visual(),
        }
    }

    /// Os filhos de um grupo ficam embaixo do item (44 px, ou 32 no compacto).
    fn flex(&self) -> Option<Flex> {
        Some(Flex {
            padding: Insets::new(self.shared.item_height(), 0.0, 0.0, 0.0),
            ..Flex::default()
        })
    }

    /// No popup, o item tem a largura do rótulo.
    fn measured(&self) -> bool {
        self.copy.is_some()
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        std::mem::take(&mut self.children)
    }

    fn handle(&mut self, event: &Event, cx: &mut EventCx) -> bool {
        if let Event::KeyDown(key) = *event
            && self.press.is_focused()
            && !cx.modifiers().command()
            && !cx.modifiers().alt
        {
            match key {
                // No popup, quem anda é a camada.
                Key::Up | Key::Down | Key::Home | Key::End if self.copy.is_none() => {
                    cx.move_focus(match key {
                        Key::Up => FocusMove::Prev,
                        Key::Down => FocusMove::Next,
                        Key::Home => FocusMove::First,
                        _ => FocusMove::Last,
                    });
                    return true;
                }
                Key::Left | Key::Right if self.arrow(key, cx) => return true,
                _ => {}
            }
        }
        let (consumed, clicked) = self.press.handle(event, true);
        if clicked {
            self.activate();
        }
        consumed
    }

    fn interactive(&self) -> bool {
        true
    }

    fn hit_rect(&self, rect: Rect) -> Rect {
        Rect::new(
            rect.x0,
            rect.y0,
            rect.x1,
            rect.y1.min(rect.y0 + self.shared.item_height()),
        )
    }

    fn focusable(&self) -> bool {
        true
    }

    /// Os itens do painel são uma parada só de Tab: o que tem o indicador da escolha.
    fn tab_stop(&self) -> bool {
        self.copy.is_some() || self.selected()
    }

    /// No popup, o foco começa no escolhido.
    fn autofocus(&self) -> bool {
        self.copy.is_some() && self.selected()
    }

    fn text(&self) -> Option<String> {
        Some(self.label.get())
    }
}

/// Popup com os filhos de um grupo, no painel compacto.
struct GroupFlyout {
    common: Common,
    shared: Rc<Shared>,
    open: Signal<bool>,
    child: Option<Box<dyn Widget>>,
}

impl Widget for GroupFlyout {
    fn kind(&self) -> &'static str {
        "navigation_flyout"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn visible(&self) -> bool {
        self.open.get() && self.shared.compact()
    }

    fn look(&self) -> Look {
        Look::Popup(PopupKind::Menu)
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
        Some(Layer {
            arrows: true,
            placement: Placement::End,
            offset: Vec2::new(FLYOUT_OFFSET, 0.0),
            ..Layer::default()
        })
    }

    fn set_open(&mut self, open: bool) {
        self.open.set(open);
    }
}

#[cfg(test)]
mod tests {
    use kurbo::Point;
    use tomni_ui_reactive::prelude::*;

    use super::*;
    use crate::cx::{FixedMetrics, MemoryClipboard};
    use crate::event::{FocusSource, Modifiers, PointerKind::Mouse};

    fn send(w: &mut dyn Widget, event: Event) -> (bool, Option<FocusMove>) {
        let mut metrics = FixedMetrics::default();
        let mut clipboard = MemoryClipboard::default();
        let mut cx = EventCx::new(
            Rect::ZERO,
            Point::ZERO,
            Modifiers::NONE,
            &mut metrics,
            &mut clipboard,
        );
        let consumed = w.handle(&event, &mut cx);
        (consumed, cx.focus_request())
    }

    fn click(w: &mut dyn Widget) {
        send(
            w,
            Event::PointerDown {
                kind: Mouse,
                clicks: 1,
            },
        );
        send(
            w,
            Event::PointerUp {
                kind: Mouse,
                inside: true,
            },
        );
    }

    fn nav(s: Signal<String>) -> NavigationView {
        NavigationView::bind(s)
            .id("nav")
            .item("inicio", Symbol::Home, "Início", Text::new("1"))
            .header("Coleção")
            .group("midia", Symbol::Image, "Mídia", |g| {
                g.item("fotos", Symbol::Camera, "Fotos", Text::new("2"))
                    .group("mais", Symbol::Folder, "Mais", |g| {
                        g.item("Áudio Raro", Symbol::MusicNote, "Áudio", Text::new("3"))
                    })
            })
            .settings(Text::new("4"))
    }

    /// Os filhos, descendo pelo caminho `path` de índices.
    fn descend(children: &mut [Box<dyn Widget>], path: &[usize]) -> Vec<Box<dyn Widget>> {
        let mut current = children[path[0]].take_children();
        for &i in &path[1..] {
            current = current[i].take_children();
        }
        current
    }

    #[test]
    fn modo_pela_largura() {
        assert_eq!(NavigationMode::for_width(1008.0), NavigationMode::Expanded);
        assert_eq!(NavigationMode::for_width(1007.0), NavigationMode::Compact);
        assert_eq!(NavigationMode::for_width(641.0), NavigationMode::Compact);
        assert_eq!(NavigationMode::for_width(640.0), NavigationMode::Minimal);
        let scope = Scope::root();
        scope.run(|| {
            let mut n = nav(signal("x".to_string()));
            n.take_children();
            assert_eq!(
                n.selected(),
                "inicio",
                "chave desconhecida mostra a primeira"
            );
            n.resized(Size::new(800.0, 600.0));
            assert_eq!(n.mode(), NavigationMode::Compact);
            assert!(!n.is_pane_open());
            n.resized(Size::new(1280.0, 600.0));
            assert!(n.is_pane_open());
            let mut fixo = nav(signal("x".to_string())).pane_mode(NavigationMode::Minimal);
            fixo.resized(Size::new(1280.0, 600.0));
            assert_eq!(fixo.mode(), NavigationMode::Minimal);
        });
        scope.clear();
    }

    #[test]
    fn grupo_abre_e_item_escolhe() {
        let scope = Scope::root();
        scope.run(|| {
            let s = signal("inicio".to_string());
            let mut n = nav(s);
            let mut children = n.take_children();
            // slot → painel → [☰, escopo] → escopo → [itens, separador, rodapé].
            let mut scope_children = descend(&mut children, &[0, 0, 1]);
            let mut list = descend(&mut scope_children, &[0, 0]);
            assert_eq!(list.len(), 3, "início, título e mídia");
            assert_eq!(list[2].common().id.as_deref(), Some("nav-midia"));
            assert!(list[0].tab_stop() && !list[2].tab_stop());
            let mut midia_children = list[2].take_children();
            assert_eq!(midia_children.len(), 2, "filhos e popup");
            assert!(!midia_children[0].visible(), "começa fechado");
            click(list[2].as_mut());
            assert!(midia_children[0].visible());
            let mut filhos = midia_children[0].take_children();
            assert_eq!(filhos[0].common().id.as_deref(), Some("nav-fotos"));
            click(filhos[0].as_mut());
            assert_eq!(s.get(), "fotos");
            assert!(filhos[0].tab_stop() && !list[0].tab_stop());
            // Fechado, o grupo leva o indicador do filho escolhido.
            assert_eq!(
                send(list[2].as_mut(), Event::FocusIn(FocusSource::Keyboard)),
                (true, None)
            );
            assert_eq!(
                send(list[2].as_mut(), Event::KeyDown(Key::Left)),
                (true, None)
            );
            assert!(!midia_children[0].visible());
            assert!(list[2].tab_stop());
            // ↓ anda pelo escopo; ← num filho vai ao grupo.
            assert_eq!(
                send(list[2].as_mut(), Event::KeyDown(Key::Down)),
                (true, Some(FocusMove::Next))
            );
            send(filhos[0].as_mut(), Event::FocusIn(FocusSource::Keyboard));
            assert_eq!(
                send(filhos[0].as_mut(), Event::KeyDown(Key::Left)),
                (true, Some(FocusMove::Parent))
            );
        });
        scope.clear();
    }

    #[test]
    fn compacto_abre_o_popup_e_o_escolher_fecha() {
        let scope = Scope::root();
        scope.run(|| {
            let s = signal("inicio".to_string());
            let mut n = nav(s);
            n.resized(Size::new(800.0, 600.0));
            let mut children = n.take_children();
            let mut pane = descend(&mut children, &[0, 0]);
            let mut scope_children = descend(&mut pane, &[1]);
            let mut list = descend(&mut scope_children, &[0, 0]);
            assert!(!list[1].visible(), "o título some no compacto");
            let mut midia_children = list[2].take_children();
            let popup = &midia_children[1];
            assert_eq!(popup.common().id.as_deref(), Some("nav-midia-filhos"));
            assert!(!popup.visible());
            click(list[2].as_mut());
            assert!(midia_children[1].visible() && !midia_children[0].visible());
            let mut copias = descend(&mut midia_children, &[1, 0]);
            assert_eq!(copias[0].common().id.as_deref(), Some("nav-fotos-flyout"));
            assert!(copias[0].measured());
            click(copias[0].as_mut());
            assert_eq!(s.get(), "fotos");
            assert!(!midia_children[1].visible(), "escolher fecha o popup");
            // O ☰ abre o painel por cima; Esc no NavigationView fecha.
            let mut toggle = descend(&mut pane, &[0]);
            assert_eq!(toggle[0].text().as_deref(), Some("Abrir navegação"));
            click(toggle[0].as_mut());
            assert!(n.is_pane_open());
            assert!(n.shared.overlay());
            assert_eq!(
                send(&mut n, Event::KeyDown(Key::Escape)),
                (true, Some(FocusMove::Inside))
            );
            assert!(!n.is_pane_open());
        });
        scope.clear();
    }

    #[test]
    fn paginas_montadas_e_escondidas() {
        let scope = Scope::root();
        scope.run(|| {
            let s = signal("inicio".to_string());
            let mut n = nav(s);
            let mut children = n.take_children();
            let mut main = descend(&mut children, &[1]);
            let pages = main[1].take_children();
            let ids: Vec<_> = pages
                .iter()
                .map(|p| p.common().id.clone().unwrap_or_default())
                .collect();
            assert_eq!(
                ids,
                [
                    "nav-inicio-conteudo",
                    "nav-fotos-conteudo",
                    "nav-áudio-raro-conteudo",
                    "nav-configuracoes-conteudo"
                ]
            );
            assert!(pages[0].visible() && !pages[3].visible());
            s.set("configuracoes".into());
            assert!(!pages[0].visible() && pages[3].visible());
        });
        scope.clear();
    }
}
