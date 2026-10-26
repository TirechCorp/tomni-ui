//! `TabView`: abas no alto e, embaixo, o conteúdo da aba escolhida.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::rc::Rc;
use std::time::Duration;

use kurbo::Size;
use tomni_ui_reactive::{Get, GetUntracked, Set, Signal, effect, signal, untrack};

use crate::button::ButtonState;
use crate::color::ThemeColor;
use crate::combo_box::option_slug;
use crate::containers::{Column, Row};
use crate::cx::{EventCx, FocusMove};
use crate::density::Density;
use crate::drag::{DragData, InsertMark};
use crate::event::{Event, FocusSource, Key};
use crate::flyout::{ContextFlyout, MenuFlyout};
use crate::fragments::For;
use crate::press::Pressable;
use crate::progress::ProgressStatus;
use crate::prop::{IntoProp, Prop};
use crate::scroll_viewer::{ScrollLink, ScrollViewer};
use crate::symbol::Symbol;
use crate::widget::{
    Align, Common, Direction, Flex, Insets, Justify, Look, View, Widget, common_builders,
    density_builder,
};

/// Altura de uma aba: o alvo de toque mínimo do lint, no lugar dos 32 px do WinUI
/// (`TabViewItemMinHeight`).
pub const TAB_HEIGHT: f64 = 44.0;
/// Altura de uma aba no compacto: a dos botões da faixa do WinUI (fechar, "+" e setas).
pub const TAB_HEIGHT_COMPACT: f64 = 24.0;
/// `TabViewHeaderPadding`: 8 px acima da faixa, só no padrão.
const STRIP_TOP: f64 = 8.0;
/// Margem dos dois lados da faixa.
const STRIP_SIDE: f64 = 8.0;
/// Altura da faixa das abas, do alto do `TabView` até a linha embaixo delas.
pub const TAB_STRIP_HEIGHT: f64 = STRIP_TOP + TAB_HEIGHT;
/// Altura da faixa no compacto, sem os 8 px de cima.
pub const TAB_STRIP_HEIGHT_COMPACT: f64 = TAB_HEIGHT_COMPACT;
/// Largura mínima de uma aba (`TabViewItemMinWidth`).
pub const TAB_MIN_WIDTH: f64 = 100.0;
/// Largura máxima de uma aba (`TabViewItemMaxWidth`).
pub const TAB_MAX_WIDTH: f64 = 240.0;
/// Botões da faixa (fechar, "+" e setas) no padrão: o alvo de toque mínimo do lint. O fundo
/// continua com os 32 × 24 do WinUI, centrado.
pub const TAB_BUTTON_SIZE: Size = Size::new(44.0, 44.0);
/// Botões da faixa no compacto: `TabViewItemAddButtonWidth` e `Height` (e os mesmos 32 × 24
/// do fechar e das setas).
pub const TAB_BUTTON_SIZE_COMPACT: Size = Size::new(32.0, 24.0);
/// Margem entre o título e o botão de fechar (a do `CloseButton`).
pub const TAB_CLOSE_GAP: f64 = 4.0;
/// Padding à direita do botão de fechar no compacto (o 4 do `TabViewItemHeaderPadding`).
const CLOSE_PADDING_COMPACT: f64 = 4.0;
/// Espaço entre as setas (e o "+") e as abas.
const BUTTON_GAP: f64 = 3.0;
/// Quanto cada clique numa seta rola (`c_scrollAmount` do WinUI).
pub const TAB_SCROLL_STEP: f64 = 50.0;
/// Espera da seta pressionada antes de repetir (`Delay` do `RepeatButton`).
const REPEAT_DELAY: Duration = Duration::from_millis(50);
/// Intervalo entre as repetições (`Interval` do `RepeatButton`).
const REPEAT_INTERVAL: Duration = Duration::from_millis(100);
/// Margem do conteúdo.
const CONTENT_PADDING: f64 = 16.0;
/// Lado do ícone da aba (`TabViewItemHeaderIconSize`).
pub const TAB_ICON_SIZE: f64 = 16.0;
/// Espaço entre o ícone e o título: a margem à direita do ícone
/// (`TabViewItemHeaderIconMargin`, 0,0,10,0).
pub const TAB_ICON_GAP: f64 = 10.0;
/// Diâmetro do ponto de atenção da aba (`.tab_attention`), como o `InfoBadge` em ponto.
pub const TAB_ATTENTION_DOT: f64 = 6.0;
/// Espaço entre o ponto de atenção e o título, numa aba sem ícone nem progresso.
pub const TAB_ATTENTION_GAP: f64 = 6.0;

/// Tamanho dos botões da faixa na densidade `density`.
fn button_size(density: Density) -> Size {
    match density {
        Density::Standard => TAB_BUTTON_SIZE,
        Density::Compact => TAB_BUTTON_SIZE_COMPACT,
    }
}

/// Como o `TabView` decide a largura das abas (`TabWidthMode` do WinUI).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TabWidth {
    /// Todas da mesma largura, dividindo a faixa, entre [`TAB_MIN_WIDTH`] e
    /// [`TAB_MAX_WIDTH`] (padrão).
    #[default]
    Equal,
    /// Cada uma do tamanho do título, entre os mesmos limites.
    SizeToContent,
}

/// O andamento de uma aba (`.tab_progress`): o anel de 16 px que ocupa o lugar do ícone.
///
/// ```
/// use tomni_ui_core::{ProgressStatus, TabProgress};
/// let indeterminado = TabProgress { value: None, status: ProgressStatus::Normal };
/// let pausado_em_60 = TabProgress { value: Some(0.6), status: ProgressStatus::Paused };
/// assert_ne!(indeterminado, pausado_em_60);
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabProgress {
    /// De 0 a 1; `None` é indeterminado.
    pub value: Option<f64>,
    /// Normal, pausado ou com erro: muda a cor do anel.
    pub status: ProgressStatus,
}

impl TabProgress {
    /// Se o anel gira sozinho: indeterminado e andando.
    fn spins(self) -> bool {
        self.value.is_none() && self.status == ProgressStatus::Normal
    }
}

/// Tudo o que o `Drawer` precisa saber do `TabView` para pintar o fundo.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TabViewVisual {
    /// Densidade: a altura da faixa.
    pub density: Density,
    /// Chave e título de cada aba, na ordem.
    pub tabs: Vec<(String, String)>,
    /// Se as abas têm botão de fechar (`.on_close`).
    pub closable: bool,
    /// Se as abas mudam de ordem arrastando (`.on_reorder`) ou vão para outro `TabView`
    /// (`.drag_group`).
    pub reorderable: bool,
}

impl TabViewVisual {
    /// Altura da faixa das abas.
    pub fn strip_height(&self) -> f64 {
        match self.density {
            Density::Standard => TAB_STRIP_HEIGHT,
            Density::Compact => TAB_STRIP_HEIGHT_COMPACT,
        }
    }
}

/// Tudo o que o `Drawer` precisa saber de uma aba para pintá-la.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TabHeaderVisual {
    /// Se é a aba escolhida.
    pub selected: bool,
    /// Se tem a linha de separação à direita: entre duas abas não escolhidas.
    pub separator: bool,
    /// Estado.
    pub state: ButtonState,
    /// Anel de foco: só com foco vindo do teclado.
    pub focus_ring: bool,
    /// Densidade: altura, fonte e padding.
    pub density: Density,
    /// Se a aba tem botão de fechar.
    pub closable: bool,
    /// Linha de onde a aba arrastada vai cair (`.on_reorder`): antes ou depois desta.
    pub insert: Option<InsertMark>,
    /// Ícone antes do título (`.tab_icon`), com [`TAB_ICON_SIZE`] de lado.
    pub icon: Option<Symbol>,
    /// A cor do ícone (`.tab_icon_tint`); `None` é a cor do título.
    pub icon_tint: Option<ThemeColor>,
    /// Anel de andamento (`.tab_progress`): ocupa o lugar do ícone, e o ícone não aparece.
    pub progress: Option<TabProgress>,
    /// Segundos desde a montagem, para o anel indeterminado girar. Zero se o anel não gira.
    pub phase: f64,
    /// Ponto de atenção (`.tab_attention`), no canto do espaço do ícone.
    pub attention: bool,
    /// Fundo e título da aba escolhida (`.selected_colors`), no lugar dos do tema. `None` nas
    /// outras abas.
    pub selected_colors: Option<(ThemeColor, ThemeColor)>,
    /// A barra de destaque embaixo da aba escolhida (`.selected_indicator`). `false` nas
    /// outras abas.
    pub indicator: bool,
}

impl TabHeaderVisual {
    /// Se o espaço do ícone está ocupado: pelo ícone ou pelo anel.
    pub fn has_icon_slot(&self) -> bool {
        self.icon.is_some() || self.progress.is_some()
    }

    /// O que o ícone (ou o anel) ocupa à esquerda do título: ele e a margem. Sem nenhum dos
    /// dois, o ponto de atenção ocupa só o dele; sem nada, zero.
    pub fn icon_space(&self) -> f64 {
        if self.has_icon_slot() {
            TAB_ICON_SIZE + TAB_ICON_GAP
        } else if self.attention {
            TAB_ATTENTION_DOT + TAB_ATTENTION_GAP
        } else {
            0.0
        }
    }

    /// Altura da aba.
    pub fn height(&self) -> f64 {
        match self.density {
            Density::Standard => TAB_HEIGHT,
            Density::Compact => TAB_HEIGHT_COMPACT,
        }
    }

    /// O que o botão de fechar ocupa à direita do título: a margem, o botão e o padding
    /// depois dele. Zero sem o botão.
    pub fn close_space(&self) -> f64 {
        if self.closable {
            TAB_CLOSE_GAP + button_size(self.density).width + close_padding(self.density)
        } else {
            0.0
        }
    }
}

/// Padding à direita do botão de fechar.
fn close_padding(density: Density) -> f64 {
    match density {
        Density::Standard => 0.0,
        Density::Compact => CLOSE_PADDING_COMPACT,
    }
}

/// Botão da faixa de abas.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TabStripButtonKind {
    /// O X de uma aba (`CloseButton`).
    #[default]
    Close,
    /// O "+" depois da última aba (`AddButton`).
    Add,
    /// A seta da esquerda (`ScrollDecreaseButton`).
    ScrollLeft,
    /// A seta da direita (`ScrollIncreaseButton`).
    ScrollRight,
    /// Botão discreto de 32×32 px com um ícone de 16 px, nos cabeçalhos do `DockHost`: o mesmo
    /// fundo de hover e de press dos outros, com o tamanho dado pelo próprio widget.
    Icon(Symbol),
}

/// Tudo o que o `Drawer` precisa saber de um botão da faixa de abas.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TabStripButtonVisual {
    /// Qual botão.
    pub kind: TabStripButtonKind,
    /// Estado; uma seta fica desabilitada na ponta da rolagem.
    pub state: ButtonState,
    /// Densidade: o tamanho do botão.
    pub density: Density,
    /// Anel de foco (só o "+" recebe foco).
    pub focus_ring: bool,
    /// Cor do glifo no lugar da do tema: o X da aba escolhida com `.selected_colors` usa a cor
    /// do título dela. O fundo do hover e do press sai da mesma cor, translúcido.
    pub color: Option<ThemeColor>,
}

impl TabStripButtonVisual {
    /// Tamanho do botão.
    pub fn size(&self) -> Size {
        button_size(self.density)
    }
}

type Keys = Rc<dyn Fn() -> Vec<String>>;
type Titles = Rc<dyn Fn() -> Vec<(String, String)>>;
type OnClose = Rc<dyn Fn(String)>;
type Condition = Rc<dyn Fn() -> bool>;
type GlyphColor = Rc<dyn Fn() -> Option<ThemeColor>>;
type TabMenu = Rc<dyn Fn(String) -> MenuFlyout>;
type OnReorder = Rc<dyn Fn(usize, usize)>;
type OnTabDrop = Rc<dyn Fn(String, usize)>;
type OnTabDropOutside = Rc<dyn Fn(String, Option<(i32, i32)>)>;
type TabIcon = Rc<dyn Fn(String) -> Option<Symbol>>;
type TabIconTint = Rc<dyn Fn(String) -> Option<ThemeColor>>;
type TabProgressFn = Rc<dyn Fn(String) -> Option<TabProgress>>;
type TabConnectingFn = Rc<dyn Fn(String) -> bool>;
type TabAttention = Rc<dyn Fn(String) -> bool>;
type TabTooltip = Rc<dyn Fn(String) -> Option<String>>;

/// O que o `TabView`, as abas, os painéis e os botões da faixa dividem.
struct Shared {
    id: RefCell<Option<String>>,
    selected: Signal<String>,
    /// Abas fixas (`.tab`): chave e título.
    fixed: RefCell<Vec<(String, Prop<String>)>>,
    /// Listas (`.items`): as chaves e os títulos, lidos dos itens.
    lists: RefCell<Vec<(Keys, Titles)>>,
    on_close: RefCell<Option<OnClose>>,
    /// Clique numa aba, mesmo na já escolhida (`.on_tab_click`).
    on_tab_click: RefCell<Option<OnClose>>,
    on_add: RefCell<Option<Rc<dyn Fn()>>>,
    /// Menu de contexto de cada aba (`.tab_context_flyout`).
    tab_menu: RefCell<Option<TabMenu>>,
    on_reorder: RefCell<Option<OnReorder>>,
    /// Grupo de arrasto (`.drag_group`): os `TabView` do mesmo grupo trocam abas.
    group: RefCell<Option<String>>,
    /// Aba que veio de outro `TabView` do grupo (`.on_tab_drop`).
    on_tab_drop: RefCell<Option<OnTabDrop>>,
    /// Aba solta fora de todas as janelas (`.on_tab_drop_outside`).
    on_tab_drop_outside: RefCell<Option<OnTabDropOutside>>,
    /// Ícone de cada aba (`.tab_icon`).
    icon: RefCell<Option<TabIcon>>,
    /// Cor do ícone de cada aba (`.tab_icon_tint`).
    icon_tint: RefCell<Option<TabIconTint>>,
    /// Fundo e título da aba escolhida (`.selected_colors`).
    selected_colors: RefCell<Prop<Option<(ThemeColor, ThemeColor)>>>,
    /// Barra de destaque na aba escolhida (`.selected_indicator`).
    selected_indicator: RefCell<Prop<bool>>,
    /// Andamento de cada aba (`.tab_progress`).
    progress: RefCell<Option<TabProgressFn>>,
    /// Se cada aba está conectando (`.tab_connecting`).
    connecting: RefCell<Option<TabConnectingFn>>,
    /// Ponto de atenção de cada aba (`.tab_attention`).
    attention: RefCell<Option<TabAttention>>,
    /// Dica de cada aba (`.tab_tooltip`).
    tooltip: RefCell<Option<TabTooltip>>,
    /// Durante um arrasto de aba: o vão entre as abas onde ela vai cair (0 é antes da
    /// primeira; o número de abas, depois da última).
    drop_gap: Signal<Option<usize>>,
    width: Cell<TabWidth>,
    density: Cell<Density>,
    /// Margem em volta do conteúdo de cada aba.
    content_padding: Cell<f64>,
    /// Largura do `TabView`, para a [`TabWidth::Equal`].
    available: Signal<f64>,
    link: Rc<ScrollLink>,
}

impl Shared {
    /// Chaves das abas, na ordem (rastreado).
    fn keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.fixed.borrow().iter().map(|(k, _)| k.clone()).collect();
        let lists: Vec<Keys> = self
            .lists
            .borrow()
            .iter()
            .map(|(k, _)| Rc::clone(k))
            .collect();
        for list in lists {
            keys.extend(list());
        }
        keys
    }

    /// Chave e título das abas, na ordem (rastreado).
    fn tabs(&self) -> Vec<(String, String)> {
        let mut tabs: Vec<(String, String)> = self
            .fixed
            .borrow()
            .iter()
            .map(|(k, title)| (k.clone(), title.get()))
            .collect();
        let lists: Vec<Titles> = self
            .lists
            .borrow()
            .iter()
            .map(|(_, t)| Rc::clone(t))
            .collect();
        for list in lists {
            tabs.extend(list());
        }
        tabs
    }

    /// Chave da aba à mostra (rastreado).
    fn current(&self) -> String {
        effective(&self.keys(), &self.selected.get())
    }

    fn closable(&self) -> bool {
        self.on_close.borrow().is_some()
    }

    /// Pede ao app para fechar a aba `key`.
    fn close(&self, key: &str) {
        let on_close = self.on_close.borrow().clone();
        if let Some(on_close) = on_close {
            on_close(key.to_owned());
        }
    }

    /// Tipo do item no arrasto das abas: só este `TabView` aceita as abas dele, ou todos do
    /// mesmo grupo (`.drag_group`).
    fn drag_kind(&self) -> String {
        match &*self.group.borrow() {
            Some(group) => format!("tab:{group}"),
            None => format!("tab@{:p}", self as *const Self),
        }
    }

    /// Se as abas arrastam: com `.on_reorder` ou num grupo.
    fn draggable(&self) -> bool {
        self.on_reorder.borrow().is_some() || self.group.borrow().is_some()
    }

    /// Ícone da aba `key` (rastreado).
    fn icon_of(&self, key: &str) -> Option<Symbol> {
        let icon = self.icon.borrow().clone();
        icon.and_then(|icon| icon(key.to_owned()))
    }

    /// Cor do ícone da aba `key` (rastreado).
    fn icon_tint_of(&self, key: &str) -> Option<ThemeColor> {
        let tint = self.icon_tint.borrow().clone();
        tint.and_then(|tint| tint(key.to_owned()))
    }

    /// Andamento da aba `key` (rastreado).
    fn progress_of(&self, key: &str) -> Option<TabProgress> {
        let progress = self.progress.borrow().clone();
        progress.and_then(|progress| progress(key.to_owned()))
    }

    /// Se a aba `key` está conectando (rastreado).
    fn connecting_of(&self, key: &str) -> bool {
        let connecting = self.connecting.borrow().clone();
        connecting.is_some_and(|connecting| connecting(key.to_owned()))
    }

    /// Se a aba `key` pede atenção (rastreado).
    fn attention_of(&self, key: &str) -> bool {
        let attention = self.attention.borrow().clone();
        attention.is_some_and(|attention| attention(key.to_owned()))
    }

    fn set_gap(&self, gap: Option<usize>) {
        if self.drop_gap.get_untracked() != gap {
            self.drop_gap.set(gap);
        }
    }

    /// Id de uma parte: `{id}-{suffix}`.
    fn sub(&self, suffix: &str) -> Option<String> {
        self.id.borrow().as_ref().map(|id| format!("{id}-{suffix}"))
    }

    /// O que o "+" ocupa na faixa, com a margem.
    fn add_space(&self) -> f64 {
        if self.on_add.borrow().is_some() {
            BUTTON_GAP + button_size(self.density.get()).width
        } else {
            0.0
        }
    }

    /// Largura das abas com [`TabWidth::Equal`]; `None` com [`TabWidth::SizeToContent`]
    /// (rastreado).
    fn tab_width(&self) -> Option<f64> {
        if self.width.get() != TabWidth::Equal {
            return None;
        }
        let n = self.keys().len().max(1) as f64;
        let space = self.available.get() - 2.0 * STRIP_SIDE - self.add_space();
        Some((space / n).floor().clamp(TAB_MIN_WIDTH, TAB_MAX_WIDTH))
    }
}

/// Uma aba fixa, guardada até a montagem.
struct Tab {
    key: String,
    title: Prop<String>,
    content: Box<dyn Widget>,
    hot: Signal<bool>,
    close_hot: Signal<bool>,
}

/// Abas no alto e, embaixo, o conteúdo da aba escolhida, ligadas a um `Signal<String>` com a
/// chave dela, como o `TabView` do WinUI.
///
/// ```
/// use tomni_ui_core::{TabView, Text};
/// use tomni_ui_reactive::prelude::*;
///
/// # let scope = Scope::root();
/// # scope.run(|| {
/// let aba = signal("geral".to_string());
/// let abas = TabView::bind(aba)
///     .id("config")
///     .tab("geral", "Geral", Text::new("Opções do dia a dia."))
///     .tab("avancado", "Avançado", Text::new("Opções raras."));
/// assert_eq!(abas.selected(), "geral");
/// # });
/// # scope.clear();
/// ```
///
/// As abas vêm de `.tab` (fixas) e de `.items` (uma lista num signal, como no `For`): as fixas
/// aparecem primeiro. Com `.on_close`, cada aba ganha um botão de fechar (na escolhida e na
/// que está sob o ponteiro, como o `CloseButtonOverlayMode.Auto` do WinUI); o clique do meio
/// e o Ctrl+F4 também pedem para fechar. Quem fecha é o app, tirando o item da lista; se a
/// aba fechada era a escolhida, a escolha passa para a da direita (ou para a da esquerda,
/// quando era a última). Com `.on_add`, o "+" aparece depois da última aba.
///
/// As abas dividem a faixa em partes iguais, entre 100 e 240 px ([`TabWidth::Equal`]); com
/// [`TabWidth::SizeToContent`], cada uma fica do tamanho do título. Quando não cabem, as
/// setas ◀ e ▶ aparecem nas pontas da faixa e rolam 50 px por clique, repetindo enquanto
/// estão pressionadas; a roda do mouse também rola.
///
/// Um clique numa aba, Enter ou Espaço a escolhem. As abas são uma parada só de Tab (a
/// escolhida); ← e → vão à anterior e à seguinte, e Home e End à primeira e à última: pelo
/// teclado, a escolha segue o foco, como no WinUI. Com o foco em qualquer lugar do
/// `TabView`, Ctrl+Tab vai à aba seguinte e Ctrl+Shift+Tab à anterior, dando a volta. Uma
/// chave que não é de nenhuma aba mostra a primeira.
///
/// No compacto ([`Density::Compact`]), as abas têm 24 px, o título 12 px e o padding
/// 8,3,4,3 do `TabViewItemHeaderPadding`, sem os 8 px acima da faixa: é uma adaptação, o
/// WinUI não tem um `TabView` compacto, e os 24 px vêm dos botões da faixa dele. No padrão,
/// as abas e os botões têm 44 px, o alvo de toque do lint, com o fundo dos botões em 32 × 24.
///
/// Cada aba tem o id `{id}-{chave}`, e o painel dela, `{id}-{chave}-conteudo` (a chave em
/// minúsculas, com o que não é letra ou número trocado por `-`). Os botões são
/// `{id}-{chave}-fechar`, `{id}-adicionar`, `{id}-rolar-esquerda` e `{id}-rolar-direita`, e a
/// faixa rolável é `{id}-abas`: não use `adicionar`, `abas`, `rolar-esquerda` nem
/// `rolar-direita` como chave. Os painéis das outras abas continuam montados, só escondidos:
/// o estado deles não se perde ao trocar de aba.
pub struct TabView {
    common: Common,
    shared: Rc<Shared>,
    tabs: Vec<Tab>,
    /// Os `For` das abas e dos painéis de cada `.items`.
    lists: Vec<(Box<dyn Widget>, Box<dyn Widget>)>,
}

impl TabView {
    /// `TabView` sem abas, mostrando a aba de chave `selected`.
    pub fn bind(selected: Signal<String>) -> Self {
        let shared = Rc::new(Shared {
            id: RefCell::new(None),
            selected,
            fixed: RefCell::new(Vec::new()),
            lists: RefCell::new(Vec::new()),
            on_close: RefCell::new(None),
            on_tab_click: RefCell::new(None),
            on_add: RefCell::new(None),
            tab_menu: RefCell::new(None),
            on_reorder: RefCell::new(None),
            group: RefCell::new(None),
            on_tab_drop: RefCell::new(None),
            on_tab_drop_outside: RefCell::new(None),
            icon: RefCell::new(None),
            icon_tint: RefCell::new(None),
            selected_colors: RefCell::new(Prop::default()),
            selected_indicator: RefCell::new(Prop::default()),
            progress: RefCell::new(None),
            connecting: RefCell::new(None),
            attention: RefCell::new(None),
            tooltip: RefCell::new(None),
            drop_gap: signal(None),
            width: Cell::new(TabWidth::Equal),
            density: Cell::new(Density::Standard),
            content_padding: Cell::new(CONTENT_PADDING),
            available: signal(0.0),
            link: ScrollLink::new(0.0),
        });
        // A aba escolhida saiu da lista: a escolha passa para a vizinha (a que ficou no lugar
        // dela ou, se era a última, a da esquerda).
        let watch = Rc::clone(&shared);
        let last: RefCell<Option<(String, usize)>> = RefCell::new(None);
        effect(move || {
            let keys = watch.keys();
            let value = watch.selected.get();
            if let Some(index) = keys.iter().position(|k| *k == value) {
                *last.borrow_mut() = Some((value, index));
                return;
            }
            let removed = last.borrow_mut().take();
            if let Some((old, index)) = removed
                && old == value
                && !keys.is_empty()
            {
                watch.selected.set(keys[index.min(keys.len() - 1)].clone());
            }
        });
        Self {
            common: Common::default(),
            shared,
            tabs: Vec::new(),
            lists: Vec::new(),
        }
    }

    /// Acrescenta uma aba fixa: `key` identifica a aba no signal e no id, `title` (fixo,
    /// closure ou signal) aparece nela e `content` é o que ela mostra.
    pub fn tab<M>(
        mut self,
        key: impl Into<String>,
        title: impl IntoProp<String, M>,
        content: impl View,
    ) -> Self {
        let key = key.into();
        let title = title.into_prop();
        self.shared
            .fixed
            .borrow_mut()
            .push((key.clone(), title.clone()));
        self.tabs.push(Tab {
            key,
            title,
            content: Box::new(content),
            hot: signal(false),
            close_hot: signal(false),
        });
        self
    }

    /// Uma aba para cada item de `items` (valor fixo, closure ou signal), como o `For`:
    /// `key(&item)` identifica a aba (no signal da escolhida e no id), `title(&item)` (fixo,
    /// closure ou signal) aparece nela e `content(item)` é o que ela mostra.
    ///
    /// Um item novo ganha uma aba; um item que sai tem a aba descartada (widgets, estado e
    /// efeitos); os que mudam de lugar mantêm a aba e o conteúdo.
    ///
    /// ```
    /// use tomni_ui_core::{Edit, TabView};
    /// use tomni_ui_reactive::prelude::*;
    ///
    /// #[derive(Clone)]
    /// struct Nota {
    ///     id: u32,
    ///     titulo: Signal<String>,
    ///     texto: Signal<String>,
    /// }
    ///
    /// # let scope = Scope::root();
    /// # scope.run(|| {
    /// let notas = signal(Vec::<Nota>::new());
    /// let aberta = signal(String::new());
    /// let abas = TabView::bind(aberta)
    ///     .id("notas")
    ///     .items(
    ///         notas,
    ///         |n| n.id,
    ///         |n| n.titulo,
    ///         |n| Edit::bind(n.texto).id(format!("texto-{}", n.id)),
    ///     )
    ///     .on_close(move |chave| notas.update(|v| v.retain(|n| n.id.to_string() != chave)));
    /// # });
    /// # scope.clear();
    /// ```
    pub fn items<T, K, P, M, MT, V>(
        mut self,
        items: impl IntoProp<Vec<T>, M>,
        key: impl Fn(&T) -> K + 'static,
        title: impl Fn(&T) -> P + 'static,
        content: impl Fn(T) -> V + 'static,
    ) -> Self
    where
        T: Clone + 'static,
        K: ToString,
        P: IntoProp<String, MT>,
        V: View,
    {
        let items: Prop<Vec<T>> = items.into_prop();
        let key: Rc<dyn Fn(&T) -> String> = Rc::new(move |t| key(t).to_string());
        let title: Rc<dyn Fn(&T) -> _> = Rc::new(move |t| title(t).into_prop());

        let keys: Keys = {
            let (items, key) = (items.clone(), Rc::clone(&key));
            Rc::new(move || items.get().iter().map(|t| key(t)).collect())
        };
        let titles: Titles = {
            let (items, key, title) = (items.clone(), Rc::clone(&key), Rc::clone(&title));
            Rc::new(move || {
                items
                    .get()
                    .iter()
                    .map(|t| (key(t), title(t).get()))
                    .collect()
            })
        };
        self.shared.lists.borrow_mut().push((keys, titles));

        let headers = {
            let (items, shared) = (items.clone(), Rc::clone(&self.shared));
            let (key, header_key) = (Rc::clone(&key), Rc::clone(&key));
            For::each(
                move || items.get(),
                move |t| key(t),
                move |t| {
                    TabHeader::new(
                        &shared,
                        header_key(&t),
                        title(&t),
                        signal(false),
                        signal(false),
                    )
                },
            )
        };
        let panels = {
            let shared = Rc::clone(&self.shared);
            let panel_key = Rc::clone(&key);
            For::each(
                move || items.get(),
                move |t| key(t),
                move |t| {
                    let key = panel_key(&t);
                    panel(&shared, key, Box::new(content(t)))
                },
            )
        };
        self.lists.push((Box::new(headers), Box::new(panels)));
        self
    }

    /// Mostra um botão de fechar nas abas: `f(chave)` roda quando o botão, o clique do meio na
    /// aba ou o Ctrl+F4 pedem para fechar (o `TabCloseRequested` do WinUI). Quem fecha é o
    /// app, tirando o item da lista.
    pub fn on_close(self, f: impl Fn(String) + 'static) -> Self {
        *self.shared.on_close.borrow_mut() = Some(Rc::new(f));
        self
    }

    /// O clique (ou Enter e Espaço) numa aba: `f(chave)` roda depois de ela ser escolhida, e
    /// também quando ela já era a escolhida, que não muda a escolha nem avisa o signal (o
    /// `Tapped` do `TabViewItem`, que o WinUI dispara mesmo sem o `SelectionChanged`). Serve
    /// para o app reagir a "a pessoa voltou a esta aba". Escolher pelo teclado (←/→) ou pelo
    /// código não chama `f`.
    pub fn on_tab_click(self, f: impl Fn(String) + 'static) -> Self {
        *self.shared.on_tab_click.borrow_mut() = Some(Rc::new(f));
        self
    }

    /// Mostra o "+" depois da última aba: `f` roda no clique (o `AddTabButtonClick` do
    /// WinUI). Quem cria a aba é o app, pondo um item na lista.
    pub fn on_add(self, f: impl Fn() + 'static) -> Self {
        *self.shared.on_add.borrow_mut() = Some(Rc::new(f));
        self
    }

    /// Dá a cada aba um menu de contexto: `f(chave)` monta o menu da aba `chave`, que abre
    /// com o botão direito sobre ela, com Shift+F10, com a tecla Menu e com o toque longo,
    /// como o `.context_flyout()` de qualquer widget (o `ContextFlyout` do `TabViewItem`).
    ///
    /// Os ids dos itens precisam ser únicos na tela: ponha a chave neles. Um `.shortcut()`
    /// num item valeria uma vez por aba (e o lint acusa o atalho repetido): mostre o atalho
    /// com `.accelerator()` num widget só.
    ///
    /// ```
    /// use tomni_ui_core::{MenuFlyout, TabView, Text};
    /// use tomni_ui_reactive::prelude::*;
    ///
    /// # let scope = Scope::root();
    /// # scope.run(|| {
    /// let aba = signal("a".to_string());
    /// let abas = TabView::bind(aba)
    ///     .id("sessoes")
    ///     .tab("a", "Servidor A", Text::new("..."))
    ///     .tab_context_flyout(|chave| {
    ///         MenuFlyout::new()
    ///             .id(format!("menu-{chave}"))
    ///             .item(format!("duplicar-{chave}"), "Duplicar", move || {})
    ///     });
    /// # });
    /// # scope.clear();
    /// ```
    pub fn tab_context_flyout(self, f: impl Fn(String) -> MenuFlyout + 'static) -> Self {
        *self.shared.tab_menu.borrow_mut() = Some(Rc::new(f));
        self
    }

    /// Deixa o usuário reordenar as abas arrastando (o `CanReorderTabs` do WinUI): soltar a
    /// aba de índice `de` faz ela ir para o índice `para` (a posição dela depois da mudança),
    /// e `f(de, para)` roda. Quem muda a ordem é o app, na lista (as abas fixas de `.tab` vêm
    /// antes das de `.items`, e os índices contam todas).
    ///
    /// O arrasto começa depois de o mouse andar 4 px com o botão pressionado sobre a aba (no
    /// toque, depois de segurar a aba); uma etiqueta com o título segue o ponteiro, e uma
    /// linha na faixa mostra onde a aba vai cair. Soltar fora da faixa não muda nada, e o Esc
    /// cancela. A linha é uma adaptação: no WinUI, as outras abas abrem espaço, animadas.
    ///
    /// ```
    /// use tomni_ui_core::{TabView, Text};
    /// use tomni_ui_reactive::prelude::*;
    ///
    /// # let scope = Scope::root();
    /// # scope.run(|| {
    /// let arquivos = signal(vec!["app.log".to_string(), "erro.log".to_string()]);
    /// let aberto = signal("app.log".to_string());
    /// let abas = TabView::bind(aberto)
    ///     .id("logs")
    ///     .items(arquivos, |a| a.clone(), |a| a.clone(), |a| Text::new(a))
    ///     .on_reorder(move |de, para| {
    ///         arquivos.update(|v| {
    ///             let a = v.remove(de);
    ///             v.insert(para, a);
    ///         })
    ///     });
    /// # });
    /// # scope.clear();
    /// ```
    pub fn on_reorder(self, f: impl Fn(usize, usize) + 'static) -> Self {
        *self.shared.on_reorder.borrow_mut() = Some(Rc::new(f));
        self
    }

    /// Põe este `TabView` num grupo de arrasto: as abas passam a ir para qualquer `TabView`
    /// do mesmo `group` (o `TabDroppedOutside` do WinUI, dentro da janela), e não só dentro
    /// do que as arrasta. Quem recebe a aba é o [`TabView::on_tab_drop`]; quem tira a aba
    /// do `TabView` de origem é o app. Sem `.on_reorder`, as abas do grupo ainda arrastam
    /// para outro `TabView`.
    ///
    /// ```
    /// use tomni_ui_core::{TabView, Text};
    /// use tomni_ui_reactive::prelude::*;
    ///
    /// # let scope = Scope::root();
    /// # scope.run(|| {
    /// let abas = TabView::bind(signal("a".to_string()))
    ///     .id("painel-0")
    ///     .tab("a", "Servidor A", Text::new("..."))
    ///     .drag_group("sessoes")
    ///     .on_tab_drop(move |chave, indice| {
    ///         let _ = (chave, indice); // mover a aba `chave` para cá, na posição `indice`
    ///     });
    /// # });
    /// # scope.clear();
    /// ```
    pub fn drag_group(self, group: impl Into<String>) -> Self {
        *self.shared.group.borrow_mut() = Some(group.into());
        self
    }

    /// Uma aba de outro `TabView` do [`TabView::drag_group`] foi solta na faixa deste:
    /// `f(chave, índice)` roda com a chave dela e a posição onde caiu (0 é antes da primeira
    /// aba; o número de abas, depois da última).
    pub fn on_tab_drop(self, f: impl Fn(String, usize) + 'static) -> Self {
        *self.shared.on_tab_drop.borrow_mut() = Some(Rc::new(f));
        self
    }

    /// Uma aba deste `TabView` foi solta fora de todas as janelas (o `TabDroppedOutside` do
    /// WinUI): `f(chave, ponto)` roda com a chave dela e o ponto na tela onde caiu, ou `None`
    /// quando o sistema não informa a posição (Wayland). É o gancho de "destacar a aba numa
    /// janela nova"; quem tira a aba deste `TabView` é o app.
    pub fn on_tab_drop_outside(self, f: impl Fn(String, Option<(i32, i32)>) + 'static) -> Self {
        *self.shared.on_tab_drop_outside.borrow_mut() = Some(Rc::new(f));
        self
    }

    /// Dá um ícone de 16 px às abas, antes do título (o `IconSource` do `TabViewItem`):
    /// `f(chave)` devolve o ícone da aba `chave`, ou `None` para a aba ficar só com o título.
    /// A closure é reativa: lê signals, e o ícone aparece e some com eles.
    pub fn tab_icon(self, f: impl Fn(String) -> Option<Symbol> + 'static) -> Self {
        *self.shared.icon.borrow_mut() = Some(Rc::new(f));
        self
    }

    /// Pinta o ícone de `.tab_icon` com uma cor: `f(chave)` devolve a cor da aba `chave`, ou
    /// `None` para o ícone ficar na cor do título. A closure é reativa. É o que dá a cada
    /// sessão de assistente a cor dela na marca da aba.
    pub fn tab_icon_tint(self, f: impl Fn(String) -> Option<ThemeColor> + 'static) -> Self {
        *self.shared.icon_tint.borrow_mut() = Some(Rc::new(f));
        self
    }

    /// Pinta a aba escolhida com `(fundo, título)` no lugar das cores do tema; `None` volta às
    /// do tema. É o que o Windows Terminal faz: a aba escolhida tem o fundo do terminal e parece
    /// sair dele. A borda e o resto das abas não mudam. Reativo.
    pub fn selected_colors<M>(
        self,
        colors: impl IntoProp<Option<(ThemeColor, ThemeColor)>, M>,
    ) -> Self {
        *self.shared.selected_colors.borrow_mut() = colors.into_prop();
        self
    }

    /// Uma barra de destaque (16 × 3 px, na cor de destaque) embaixo da aba escolhida, como a da
    /// `NavigationView` de cima do WinUI. Serve para marcar, entre vários `TabView`, o que tem o
    /// foco (o painel ativo de uma grade). Reativo.
    pub fn selected_indicator<M>(self, on: impl IntoProp<bool, M>) -> Self {
        *self.shared.selected_indicator.borrow_mut() = on.into_prop();
        self
    }

    /// Mostra o andamento de cada aba: quando `f(chave)` devolve `Some`, o espaço do ícone
    /// ganha um [`ProgressRing`](crate::ProgressRing) de 16 px, no lugar do ícone do
    /// [`tab_icon`](Self::tab_icon). Sem ícone, o anel abre o espaço, como o ícone abriria. O
    /// anel vale mais que o ícone; um ícone de aviso que a tela queira mais forte que o anel
    /// ela mesma escolhe, devolvendo `None` aqui. A closure é reativa. O nome da aba (o
    /// [`text`](Widget::text)) ganha o estado: "45%", "trabalhando", "pausado" ou "erro".
    pub fn tab_progress(self, f: impl Fn(String) -> Option<TabProgress> + 'static) -> Self {
        *self.shared.progress.borrow_mut() = Some(Rc::new(f));
        self
    }

    /// Diz quais abas estão conectando: `f(chave)` é `true` enquanto a aba `chave` abre a
    /// conexão. Só muda o nome da aba (o [`text`](Widget::text)), que diz "conectando" no lugar
    /// de "trabalhando" quando o anel é indeterminado; o anel continua sendo o de
    /// [`tab_progress`](Self::tab_progress). A closure é reativa.
    pub fn tab_connecting(self, f: impl Fn(String) -> bool + 'static) -> Self {
        *self.shared.connecting.borrow_mut() = Some(Rc::new(f));
        self
    }

    /// Acende um ponto de atenção de 6 px (na cor `SystemFillColorAttention`, como o
    /// `InfoBadge` em ponto do WinUI) no canto do espaço do ícone das abas em que `f(chave)`
    /// é `true`. Aparece por cima do ícone ou do anel; sozinho, abre o próprio espaço. A
    /// closure é reativa. O nome da aba ganha "pede atenção".
    pub fn tab_attention(self, f: impl Fn(String) -> bool + 'static) -> Self {
        *self.shared.attention.borrow_mut() = Some(Rc::new(f));
        self
    }

    /// Dá a cada aba uma dica, que abre com o ponteiro parado em cima do cabeçalho: `f(chave)`
    /// devolve o texto da aba `chave`, ou `None` para ficar o padrão, que é o título da aba (o
    /// `TabViewItem` do WinUI usa o `Header` como dica). A closure é reativa: lê signals, e o
    /// texto muda com eles. A dica tem o id `{id da aba}-tooltip`. Texto vazio não abre.
    pub fn tab_tooltip(self, f: impl Fn(String) -> Option<String> + 'static) -> Self {
        *self.shared.tooltip.borrow_mut() = Some(Rc::new(f));
        self
    }

    /// Margem em volta do conteúdo de cada aba, em px (padrão 16, a `Margin` do conteúdo do
    /// `TabView` do WinUI). `0` deixa o conteúdo encostar na faixa e nas bordas, como num
    /// painel de terminal.
    pub fn content_padding(self, padding: f64) -> Self {
        self.shared.content_padding.set(padding.max(0.0));
        self
    }

    /// Como as abas decidem a largura: [`TabWidth::Equal`] (padrão) ou
    /// [`TabWidth::SizeToContent`].
    pub fn tab_width(self, width: TabWidth) -> Self {
        self.shared.width.set(width);
        self
    }

    /// Chave da aba à mostra: a escolhida ou, se ela não existe, a primeira.
    pub fn selected(&self) -> String {
        self.shared.current()
    }

    fn visual(&self) -> TabViewVisual {
        TabViewVisual {
            density: self.shared.density.get(),
            tabs: self.shared.tabs(),
            closable: self.shared.closable(),
            reorderable: self.shared.draggable(),
        }
    }
}

common_builders!(TabView);
density_builder!(TabView);

/// A chave à mostra: `value`, se for de uma aba; senão a primeira.
fn effective(keys: &[String], value: &str) -> String {
    if keys.iter().any(|k| k == value) {
        value.to_owned()
    } else {
        keys.first().cloned().unwrap_or_default()
    }
}

/// Índice da aba à mostra.
fn effective_index(keys: &[String], value: &str) -> usize {
    keys.iter().position(|k| k == value).unwrap_or(0)
}

/// Painel de uma aba, visível enquanto ela é a escolhida.
fn panel(shared: &Rc<Shared>, key: String, content: Box<dyn Widget>) -> Column {
    let id = shared.sub(&format!("{}-conteudo", option_slug(&key)));
    let watch = Rc::clone(shared);
    let mut panel = Column::new()
        .padding(shared.content_padding.get())
        .grow(1)
        .visible(move || watch.current() == key)
        .boxed(content);
    if let Some(id) = id {
        panel = panel.id(id);
    }
    panel
}

impl Widget for TabView {
    fn kind(&self) -> &'static str {
        "tab_view"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn names_children(&self) -> bool {
        true
    }

    fn look(&self) -> Look {
        Look::TabView(self.visual())
    }

    fn flex(&self) -> Option<Flex> {
        Some(Flex::default())
    }

    fn set_density(&mut self, density: Density) {
        self.shared.density.set(density);
        // As duas setas, com as margens: o que a área das abas perde quando elas aparecem.
        let arrows = 2.0 * (button_size(density).width + BUTTON_GAP);
        self.shared.link.set_slack(arrows);
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        *self.shared.id.borrow_mut() = self.common.id.clone();
        let shared = &self.shared;
        let density = shared.density.get();
        let top = match density {
            Density::Standard => STRIP_TOP,
            Density::Compact => 0.0,
        };

        let mut tabs = TabRow::default();
        let mut panels: Vec<Box<dyn Widget>> = Vec::new();
        for tab in std::mem::take(&mut self.tabs) {
            tabs.children.push(Box::new(TabHeader::new(
                shared,
                tab.key.clone(),
                tab.title,
                tab.hot,
                tab.close_hot,
            )));
            panels.push(Box::new(panel(shared, tab.key, tab.content)));
        }
        for (headers, list_panels) in std::mem::take(&mut self.lists) {
            tabs.children.push(headers);
            panels.push(list_panels);
        }

        let mut scroll = ScrollViewer::horizontal()
            .link(Rc::clone(&shared.link))
            .generated()
            .child(tabs);
        if let Some(id) = shared.sub("abas") {
            scroll = scroll.id(id);
        }
        let arrow = |kind: TabStripButtonKind, padding: Insets| {
            let link = Rc::clone(&shared.link);
            Row::new()
                .padding(padding)
                .visible(move || link.overflows())
                .child(TabStripButton::new(shared, kind))
        };
        let mut strip = Row::new()
            .padding(Insets::new(top, STRIP_SIDE, 0.0, STRIP_SIDE))
            .child(arrow(
                TabStripButtonKind::ScrollLeft,
                Insets::new(0.0, BUTTON_GAP, 0.0, 0.0),
            ))
            .child(scroll)
            .child(arrow(
                TabStripButtonKind::ScrollRight,
                Insets::new(0.0, 0.0, 0.0, BUTTON_GAP),
            ));
        if shared.on_add.borrow().is_some() {
            strip = strip.child(
                Row::new()
                    .padding(Insets::new(0.0, 0.0, 0.0, BUTTON_GAP))
                    .child(TabStripButton::new(shared, TabStripButtonKind::Add)),
            );
        }
        let mut children: Vec<Box<dyn Widget>> = vec![Box::new(strip)];
        children.extend(panels);
        children
    }

    fn handle(&mut self, event: &Event, cx: &mut EventCx) -> bool {
        let shared = &self.shared;
        match event {
            // A aba sob o ponteiro já marcou o vão; fora da faixa, a aba não cai.
            Event::DragOver(data) if data.kind == shared.drag_kind() => {
                let strip = TabViewVisual {
                    density: shared.density.get(),
                    ..TabViewVisual::default()
                }
                .strip_height();
                let inside = cx.pointer().y < cx.rect().y0 + strip;
                if !inside {
                    shared.set_gap(None);
                }
                return inside;
            }
            Event::DragLeave => {
                shared.set_gap(None);
                return true;
            }
            Event::Drop(data) if data.kind == shared.drag_kind() => {
                let gap = shared.drop_gap.get_untracked();
                shared.set_gap(None);
                let keys = shared.keys();
                match keys.iter().position(|k| *k == data.key) {
                    Some(from) => {
                        let on_reorder = shared.on_reorder.borrow().clone();
                        if let (Some(gap), Some(on_reorder)) = (gap, on_reorder) {
                            let to = if gap > from { gap - 1 } else { gap };
                            if to != from {
                                on_reorder(from, to);
                            }
                        }
                    }
                    // Uma aba de outro `TabView` do grupo.
                    None => {
                        let on_tab_drop = shared.on_tab_drop.borrow().clone();
                        if let Some(on_tab_drop) = on_tab_drop {
                            on_tab_drop(data.key.clone(), gap.unwrap_or(keys.len()));
                        }
                    }
                }
                return true;
            }
            _ => {}
        }
        // Chega aqui o Ctrl+Tab que nenhum widget de dentro usou.
        if *event == Event::KeyDown(Key::Tab) && cx.modifiers().ctrl {
            let keys = shared.keys();
            if keys.is_empty() {
                return false;
            }
            let n = keys.len();
            let current = effective_index(&keys, &shared.selected.get_untracked());
            let next = if cx.modifiers().shift {
                (current + n - 1) % n
            } else {
                (current + 1) % n
            };
            if next != current {
                shared.selected.set(keys[next].clone());
            }
            cx.move_focus(FocusMove::Inside);
            return true;
        }
        // Ctrl+F4 pede para fechar a aba escolhida, como no WinUI.
        if *event == Event::KeyDown(Key::F(4)) && cx.modifiers().ctrl && shared.closable() {
            let keys = shared.keys();
            if keys.is_empty() {
                return false;
            }
            shared.close(&effective(&keys, &shared.selected.get_untracked()));
            return true;
        }
        false
    }

    fn resized(&mut self, size: Size) {
        if self.shared.available.get_untracked() != size.width {
            self.shared.available.set(size.width);
        }
    }

    fn text(&self) -> Option<String> {
        Some(self.selected())
    }
}

impl fmt::Debug for TabView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TabView")
            .field("common", &self.common)
            .field("keys", &self.shared.keys())
            .finish_non_exhaustive()
    }
}

/// A fileira das abas, dentro da rolagem: ←, →, Home e End andam entre todas elas, das
/// fixas e das listas.
#[derive(Default)]
struct TabRow {
    common: Common,
    children: Vec<Box<dyn Widget>>,
}

impl Widget for TabRow {
    fn kind(&self) -> &'static str {
        "row"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::Row
    }

    fn flex(&self) -> Option<Flex> {
        Some(Flex {
            direction: Direction::Row,
            ..Flex::default()
        })
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        std::mem::take(&mut self.children)
    }

    fn focus_scope(&self) -> bool {
        true
    }
}

impl fmt::Debug for TabRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TabRow")
            .field("common", &self.common)
            .finish_non_exhaustive()
    }
}

/// O estado de uma aba em palavras, para o nome dela: o andamento e a atenção.
fn state_text(progress: Option<TabProgress>, connecting: bool, attention: bool) -> Option<String> {
    let strings = crate::strings();
    let mut parts: Vec<String> = Vec::new();
    if let Some(progress) = progress {
        let percent = progress
            .value
            .map(|value| format!("{}%", (value.clamp(0.0, 1.0) * 100.0).round()));
        match (progress.status, percent) {
            (ProgressStatus::Normal, Some(percent)) => parts.push(percent),
            (ProgressStatus::Normal, None) if connecting => {
                parts.push(strings.tab_connecting.to_string());
            }
            (ProgressStatus::Normal, None) => parts.push(strings.tab_progress_working.to_string()),
            (status, percent) => {
                parts.push(
                    match status {
                        ProgressStatus::Error => &strings.tab_progress_error,
                        _ => &strings.tab_progress_paused,
                    }
                    .to_string(),
                );
                parts.extend(percent);
            }
        }
    }
    if attention {
        parts.push(strings.tab_attention.to_string());
    }
    (!parts.is_empty()).then(|| parts.join(", "))
}

/// Uma aba de um [`TabView`]: o título, desenhado por ela, e o botão de fechar, filho.
struct TabHeader {
    common: Common,
    key: String,
    title: Prop<String>,
    shared: Rc<Shared>,
    press: Pressable,
    density: Density,
    /// Ponteiro sobre a aba (fora do botão de fechar).
    hot: Signal<bool>,
    /// Ponteiro sobre o botão de fechar.
    close_hot: Signal<bool>,
    close: Option<TabStripButton>,
    /// Segundos desde a montagem, para o anel indeterminado.
    phase: f64,
}

impl TabHeader {
    fn new(
        shared: &Rc<Shared>,
        key: String,
        title: Prop<String>,
        hot: Signal<bool>,
        close_hot: Signal<bool>,
    ) -> Self {
        let slug = option_slug(&key);
        let close = shared.closable().then(|| {
            let (watch, close_key) = (Rc::clone(shared), key.clone());
            let shown_key = key.clone();
            let shown = Rc::clone(shared);
            let mut button = TabStripButton::new(shared, TabStripButtonKind::Close);
            button.common.id = shared.sub(&format!("{slug}-fechar"));
            button.common.visible = Some(
                (move || shown.current() == shown_key || hot.get() || close_hot.get()).into_prop(),
            );
            button.action = Rc::new(move || watch.close(&close_key));
            button.hot = Some(close_hot);
            // Na escolhida com cores próprias, o X acompanha o título.
            let (colored, colored_key) = (Rc::clone(shared), key.clone());
            button.color = Some(Rc::new(move || {
                if colored.current() != colored_key {
                    return None;
                }
                let colors = colored.selected_colors.borrow().clone();
                colors.get().map(|(_, text)| text)
            }));
            button
        });
        let mut common = Common::generated(shared.sub(&slug));
        let tip = shared.tooltip.borrow().clone();
        let (tip_key, tip_title) = (key.clone(), title.clone());
        common.tooltip = Some(Prop::Dynamic(Rc::new(move || {
            tip.as_ref()
                .and_then(|tip| tip(tip_key.clone()))
                .unwrap_or_else(|| tip_title.get())
        })));
        let menu = shared.tab_menu.borrow().clone();
        if let Some(menu) = menu {
            common.context_flyout = Some(ContextFlyout::new(menu(key.clone())));
        }
        Self {
            common,
            key,
            title,
            shared: Rc::clone(shared),
            press: Pressable::default(),
            density: Density::Standard,
            hot,
            close_hot,
            close,
            phase: 0.0,
        }
    }

    /// Posição da aba e da escolhida (rastreado).
    fn positions(&self) -> (Option<usize>, usize, usize) {
        let keys = self.shared.keys();
        let current = effective_index(&keys, &self.shared.selected.get());
        let index = keys.iter().position(|k| *k == self.key);
        (index, current, keys.len())
    }

    fn visual(&self) -> TabHeaderVisual {
        let progress = self.shared.progress_of(&self.key);
        let (index, current, n) = self.positions();
        let selected = index == Some(current);
        let state = match self.press.state() {
            ButtonState::Rest if self.close_hot.get() => ButtonState::Hover,
            state => state,
        };
        let gap = self.shared.drop_gap.get();
        let insert = index.and_then(|i| {
            if gap == Some(i) {
                Some(InsertMark::Before)
            } else if gap == Some(n) && i + 1 == n {
                Some(InsertMark::After)
            } else {
                None
            }
        });
        TabHeaderVisual {
            selected,
            separator: index.is_some_and(|i| !selected && i + 1 != current && i + 1 < n),
            state,
            focus_ring: self.press.focus_ring(),
            density: self.density,
            closable: self.shared.closable(),
            insert,
            icon: self.shared.icon_of(&self.key),
            icon_tint: self.shared.icon_tint_of(&self.key),
            progress,
            phase: if progress.is_some_and(TabProgress::spins) {
                self.phase
            } else {
                0.0
            },
            attention: self.shared.attention_of(&self.key),
            selected_colors: if selected {
                let colors = self.shared.selected_colors.borrow().clone();
                colors.get()
            } else {
                None
            },
            indicator: selected && {
                let on = self.shared.selected_indicator.borrow().clone();
                on.get()
            },
        }
    }

    fn choose(&self) {
        if self.shared.selected.get_untracked() != self.key {
            self.shared.selected.set(self.key.clone());
        }
    }

    fn set_hot(&self, hot: bool) {
        if self.hot.get_untracked() != hot {
            self.hot.set(hot);
        }
    }
}

impl Widget for TabHeader {
    fn kind(&self) -> &'static str {
        "tab"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::TabHeader {
            label: self.title.get(),
            visual: self.visual(),
            width: self.shared.tab_width(),
        }
    }

    fn flex(&self) -> Option<Flex> {
        Some(Flex {
            direction: Direction::Row,
            padding: Insets::new(0.0, close_padding(self.density), 0.0, 0.0),
            align: Align::Center,
            justify: Justify::End,
            ..Flex::default()
        })
    }

    fn measured(&self) -> bool {
        true
    }

    fn dynamic_width(&self) -> Option<f64> {
        self.shared.tab_width()
    }

    fn set_density(&mut self, density: Density) {
        self.density = density;
        self.common.height = Some(match density {
            Density::Standard => TAB_HEIGHT,
            Density::Compact => TAB_HEIGHT_COMPACT,
        });
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        self.close
            .take()
            .map(|b| Box::new(b) as Box<dyn Widget>)
            .into_iter()
            .collect()
    }

    fn handle(&mut self, event: &Event, cx: &mut EventCx) -> bool {
        // A aba arrastada sobre esta: o vão é o da esquerda ou o da direita, conforme a
        // metade. Quem aceita é o `TabView`.
        if let Event::DragOver(data) = event
            && data.kind == self.shared.drag_kind()
        {
            let keys = self.shared.keys();
            if let Some(i) = keys.iter().position(|k| *k == self.key) {
                let right = cx.pointer().x > cx.rect().center().x;
                self.shared.set_gap(Some(i + usize::from(right)));
            }
            return false;
        }
        if let Event::DropOutside { data, at } = event
            && data.kind == self.shared.drag_kind()
        {
            let f = self.shared.on_tab_drop_outside.borrow().clone();
            if let Some(f) = f {
                f(self.key.clone(), *at);
            }
            self.shared.set_gap(None);
            return true;
        }
        if *event == Event::MiddleClick {
            if !self.shared.closable() {
                return false;
            }
            self.shared.close(&self.key);
            return true;
        }
        if let Event::KeyDown(key @ (Key::Left | Key::Right | Key::Home | Key::End)) = event
            && self.press.is_focused()
            && !cx.modifiers().command()
        {
            cx.move_focus(match key {
                Key::Left => FocusMove::Prev,
                Key::Right => FocusMove::Next,
                Key::Home => FocusMove::First,
                _ => FocusMove::Last,
            });
            return true;
        }
        // Pelo teclado, a escolha segue o foco (como no WinUI): ←/→ trocam de aba. O Tab chega
        // sempre na aba escolhida, então não muda nada.
        if *event == Event::FocusIn(FocusSource::Keyboard) {
            self.choose();
        }
        match *event {
            Event::PointerMove { kind, inside } | Event::PointerUp { kind, inside } => {
                self.set_hot(inside && kind.hovers());
            }
            Event::PointerDown { kind, .. } => self.set_hot(kind.hovers()),
            Event::PointerCancel => self.set_hot(false),
            _ => {}
        }
        let (consumed, clicked) = self.press.handle(event, true);
        if clicked {
            self.choose();
            let on_click = self.shared.on_tab_click.borrow().clone();
            if let Some(f) = on_click {
                untrack(|| f(self.key.clone()));
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
        let (index, current, _) = self.positions();
        index == Some(current)
    }

    fn text(&self) -> Option<String> {
        let title = self.title.get();
        let state = state_text(
            self.shared.progress_of(&self.key),
            self.shared.connecting_of(&self.key),
            self.shared.attention_of(&self.key),
        );
        Some(match state {
            Some(state) => format!("{title}, {state}"),
            None => title,
        })
    }

    /// O anel indeterminado gira: a árvore lê isto uma vez, na montagem.
    fn animating(&self) -> bool {
        self.shared.progress.borrow().is_some()
    }

    fn ticking(&self) -> bool {
        untrack(|| self.shared.progress_of(&self.key)).is_some_and(TabProgress::spins)
    }

    fn tick(&mut self, dt: Duration) {
        self.phase += dt.as_secs_f64();
    }

    fn drag_data(&self) -> Option<DragData> {
        if !self.shared.draggable() {
            return None;
        }
        Some(DragData {
            kind: self.shared.drag_kind(),
            key: self.key.clone(),
            label: self.title.get(),
        })
    }
}

impl fmt::Debug for TabHeader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TabHeader")
            .field("common", &self.common)
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

/// Botão da faixa de abas: o X de uma aba, o "+" e as setas de rolagem.
struct TabStripButton {
    common: Common,
    kind: TabStripButtonKind,
    press: Pressable,
    density: Density,
    action: Rc<dyn Fn()>,
    enabled: Option<Condition>,
    /// Signal que acompanha o ponteiro sobre o botão (o X acende a aba).
    hot: Option<Signal<bool>>,
    /// Tempo com a seta pressionada e quantas vezes ela repetiu.
    repeat: Option<(Duration, u32)>,
    /// Cor do glifo (rastreado; o X da aba escolhida com `.selected_colors`).
    color: Option<GlyphColor>,
}

impl TabStripButton {
    fn new(shared: &Rc<Shared>, kind: TabStripButtonKind) -> Self {
        let link = Rc::clone(&shared.link);
        let (suffix, action, enabled): (&str, Rc<dyn Fn()>, Option<Condition>) = match kind {
            TabStripButtonKind::Close => ("fechar", Rc::new(|| {}), None),
            TabStripButtonKind::Add => {
                let on_add = shared.on_add.borrow().clone();
                (
                    "adicionar",
                    Rc::new(move || {
                        if let Some(on_add) = &on_add {
                            on_add();
                        }
                    }),
                    None,
                )
            }
            TabStripButtonKind::Icon(_) => ("icone", Rc::new(|| {}), None),
            TabStripButtonKind::ScrollLeft => {
                let watch = Rc::clone(&link);
                (
                    "rolar-esquerda",
                    Rc::new(move || link.scroll_by(-TAB_SCROLL_STEP)),
                    Some(Rc::new(move || !watch.at_start())),
                )
            }
            TabStripButtonKind::ScrollRight => {
                let watch = Rc::clone(&link);
                (
                    "rolar-direita",
                    Rc::new(move || link.scroll_by(TAB_SCROLL_STEP)),
                    Some(Rc::new(move || !watch.at_end())),
                )
            }
        };
        Self {
            common: Common::generated(shared.sub(suffix)),
            kind,
            press: Pressable::default(),
            density: Density::Standard,
            action,
            enabled,
            hot: None,
            repeat: None,
            color: None,
        }
    }

    fn is_enabled(&self) -> bool {
        self.enabled.as_ref().is_none_or(|enabled| enabled())
    }

    fn repeats(&self) -> bool {
        matches!(
            self.kind,
            TabStripButtonKind::ScrollLeft | TabStripButtonKind::ScrollRight
        )
    }

    fn visual(&self) -> TabStripButtonVisual {
        TabStripButtonVisual {
            kind: self.kind,
            state: if self.is_enabled() {
                self.press.state()
            } else {
                ButtonState::Disabled
            },
            density: self.density,
            focus_ring: self.press.focus_ring(),
            color: self.color.as_ref().and_then(|color| color()),
        }
    }

    fn set_hot(&self) {
        if let Some(hot) = self.hot {
            let now = self.press.state() != ButtonState::Rest;
            if hot.get_untracked() != now {
                hot.set(now);
            }
        }
    }
}

impl Widget for TabStripButton {
    fn kind(&self) -> &'static str {
        "tab_button"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::TabStripButton(self.visual())
    }

    fn set_density(&mut self, density: Density) {
        self.density = density;
        let size = button_size(density);
        self.common.width = Some(size.width);
        self.common.height = Some(size.height);
    }

    fn handle(&mut self, event: &Event, _cx: &mut EventCx) -> bool {
        if !self.is_enabled() {
            self.press.reset();
            self.repeat = None;
            return false;
        }
        let (consumed, clicked) = self.press.handle(event, true);
        self.set_hot();
        if self.repeats() {
            // Como o `RepeatButton`: rola ao pressionar e repete enquanto segura.
            match event {
                Event::PointerDown { .. } => {
                    (self.action)();
                    self.repeat = Some((Duration::ZERO, 0));
                }
                Event::PointerUp { .. } | Event::PointerCancel => self.repeat = None,
                _ => {}
            }
        } else if clicked {
            (self.action)();
        }
        consumed
    }

    fn interactive(&self) -> bool {
        true
    }

    fn focusable(&self) -> bool {
        self.kind == TabStripButtonKind::Add
    }

    fn enabled(&self) -> bool {
        self.is_enabled()
    }

    fn animating(&self) -> bool {
        self.repeats()
    }

    fn ticking(&self) -> bool {
        self.repeat.is_some() && self.press.state() == ButtonState::Pressed && self.is_enabled()
    }

    fn tick(&mut self, dt: Duration) {
        let Some((elapsed, fired)) = &mut self.repeat else {
            return;
        };
        *elapsed += dt;
        while *elapsed >= REPEAT_DELAY + REPEAT_INTERVAL * *fired {
            *fired += 1;
            (self.action)();
        }
    }

    fn text(&self) -> Option<String> {
        let strings = crate::strings();
        Some(
            match self.kind {
                TabStripButtonKind::Close => strings.tab_close.clone(),
                TabStripButtonKind::Add => strings.tab_add.clone(),
                TabStripButtonKind::ScrollLeft => strings.tab_scroll_left.clone(),
                TabStripButtonKind::ScrollRight => strings.tab_scroll_right.clone(),
                TabStripButtonKind::Icon(_) => Default::default(),
            }
            .into_owned(),
        )
    }
}

impl fmt::Debug for TabStripButton {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TabStripButton")
            .field("common", &self.common)
            .field("visual", &self.visual())
            .finish_non_exhaustive()
    }
}
