//! `SettingsCard`: uma linha de configuração, como as das Configurações do Windows 11.

use std::fmt;

use kurbo::Size;
use tomni_ui_reactive::{Get, GetUntracked, Set, Signal, signal, untrack};

use crate::button::ButtonState;
use crate::color::Token;
use crate::containers::{Column, Row};
use crate::cx::EventCx;
use crate::event::Event;
use crate::icon::Icon;
use crate::press::Pressable;
use crate::prop::{IntoProp, Prop};
use crate::symbol::Symbol;
use crate::text::Text;
use crate::widget::{
    Align, Common, Direction, Flex, Insets, Justify, Look, View, Widget, common_builders,
};

/// `SettingsCardMinHeight`.
pub const SETTINGS_CARD_MIN_HEIGHT: f64 = 68.0;
/// `SettingsCardPadding`.
pub const SETTINGS_CARD_PADDING: f64 = 16.0;
/// Espessura da borda (`SettingsCardBorderThickness`).
pub const SETTINGS_CARD_BORDER: f64 = 1.0;
/// Raio dos cantos: o `ControlCornerRadius`.
pub const SETTINGS_CARD_CORNER_RADIUS: f64 = 4.0;
/// `SettingsCardWrapThreshold`: abaixo dessa largura **do card**, o controle desce para baixo
/// do texto.
pub const SETTINGS_CARD_WRAP_THRESHOLD: f64 = 476.0;
/// `SettingsCardWrapNoIconThreshold`: abaixo dessa largura, o ícone some também.
pub const SETTINGS_CARD_WRAP_NO_ICON_THRESHOLD: f64 = 286.0;
/// Altura mínima de um filho de `SettingsExpander`.
pub const SETTINGS_EXPANDER_ITEM_MIN_HEIGHT: f64 = 52.0;

/// `SettingsCardMinWidth`: a base do bloco de texto com o controle ao lado.
const MIN_WIDTH: f64 = 148.0;
/// Espaço entre o bloco de texto e o controle (`SettingsCardHeaderMargin`).
const CONTENT_GAP: f64 = 24.0;
/// `SettingsCardVerticalHeaderContentSpacing`: o controle abaixo do texto.
const WRAPPED_GAP: f64 = 8.0;
/// `SettingsCardHeaderIconMaxSize`.
const ICON_SIZE: u32 = 20;
/// Margem do ícone: 2 à esquerda e 20 até o texto.
const ICON_MARGIN: (f64, f64) = (2.0, 20.0);
/// `SettingsCardActionIconMaxSize` e a margem à esquerda dele.
const ACTION_ICON_SIZE: u32 = 13;
const ACTION_ICON_MARGIN: f64 = 14.0;
/// Botão do chevron do cabeçalho do expander e o espaço que sobra do padding de 4.
const CHEVRON_BUTTON: f64 = 32.0;
const CHEVRON_GLYPH: u32 = 16;
/// Padding de um filho de `SettingsExpander` (`SettingsExpanderItemPadding`).
const ITEM_PADDING: Insets = Insets::new(8.0, 44.0, 8.0, 58.0);
/// `ClickableSettingsExpanderItemPadding`.
const CLICKABLE_ITEM_PADDING: Insets = Insets::new(8.0, 16.0, 8.0, 58.0);
/// `SettingsExpanderHeaderPadding`.
const HEADER_PADDING: Insets = Insets::new(16.0, 4.0, 16.0, 16.0);

/// O lugar do card: sozinho ou dentro de um `SettingsExpander`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SettingsCardPart {
    /// Um card comum, com fundo, borda e os quatro cantos arredondados.
    #[default]
    Alone,
    /// Cabeçalho de um `SettingsExpander`; aberto, só os cantos de cima são arredondados.
    ExpanderHeader {
        /// Se o corpo do expander está à mostra.
        open: bool,
    },
    /// Filho de um `SettingsExpander`: sem raio, com a borda só dos lados e em cima (o
    /// separador), e embaixo se for o último.
    ExpanderItem {
        /// Se é o último filho (fecha o expander com os cantos de baixo arredondados).
        last: bool,
    },
}

/// Tudo o que o `Drawer` precisa saber do `SettingsCard` para pintá-lo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SettingsCardVisual {
    /// Se tem `on_click`: só então tem hover e pressionado.
    pub clickable: bool,
    /// Estado, o mesmo do botão. Sem clique, fica em `Rest` (ou `Disabled`).
    pub state: ButtonState,
    /// Anel de foco: só com foco vindo do teclado, e nunca desabilitado.
    pub focus_ring: bool,
    /// Se o controle desceu para baixo do texto (card estreito).
    pub wrapped: bool,
    /// Se o ícone está à mostra (some em card muito estreito).
    pub show_icon: bool,
    /// Sozinho ou dentro de um `SettingsExpander`.
    pub part: SettingsCardPart,
}

/// Contêiner de uma linha que escolhe a base no eixo do pai pelo estado, para o card poder
/// quebrar pela largura dele.
struct Lead {
    common: Common,
    flex: Flex,
    children: Vec<Box<dyn Widget>>,
    basis: Box<dyn Fn() -> (f64, f64)>,
}

impl Widget for Lead {
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
        Some(self.flex)
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        std::mem::take(&mut self.children)
    }

    fn dynamic_basis(&self) -> Option<(f64, f64)> {
        Some((self.basis)())
    }
}

/// Uma linha de configuração: ícone, cabeçalho, descrição e, à direita, o controle.
///
/// ```
/// use tomni_ui_core::{SettingsCard, Symbol, Text};
///
/// let linha = SettingsCard::new("Tema")
///     .id("tema")
///     .description("Claro, escuro ou o do sistema.")
///     .icon(Symbol::Settings)
///     .content(Text::new("Escuro"));
/// assert_eq!(linha.header(), "Tema");
/// ```
///
/// É o `SettingsCard` do Windows Community Toolkit, o que as Configurações do Windows 11, a
/// WinUI Gallery e o PowerToys usam (o WinUI não tem um controle de linha de configuração).
/// Ícone, texto e controle ficam numa linha, com o controle centrado na vertical; a quebra
/// acompanha a largura **do próprio card**, não a da janela: abaixo de 476 px o controle desce
/// para baixo do texto, e abaixo de 286 o ícone some. Com `.on_click` o card inteiro é um
/// botão, com hover, pressionado, foco pelo teclado (Enter e Espaço) e o chevron à direita;
/// sem ele, não tem hover e o foco vai direto para o controle de dentro.
///
/// O texto do card na árvore é o cabeçalho. Num card clicável com `.content` e estreito, o
/// chevron desce junto com o controle.
pub struct SettingsCard {
    common: Common,
    header: Prop<String>,
    description: Option<Prop<String>>,
    icon: Option<Symbol>,
    content: Option<Box<dyn Widget>>,
    on_click: Option<Box<dyn FnMut()>>,
    action_icon: Prop<Symbol>,
    chevron_button: bool,
    disabled: Prop<bool>,
    press: Pressable,
    part: Prop<SettingsCardPart>,
    body: Option<Box<dyn Widget>>,
    width: Signal<f64>,
}

impl SettingsCard {
    /// Linha com o cabeçalho `header` (fixo, closure ou signal).
    pub fn new<M>(header: impl IntoProp<String, M>) -> Self {
        Self {
            common: Common::default(),
            header: header.into_prop(),
            description: None,
            icon: None,
            content: None,
            on_click: None,
            action_icon: Prop::Value(Symbol::ChevronRight),
            chevron_button: false,
            disabled: Prop::Value(false),
            press: Pressable::default(),
            part: Prop::Value(SettingsCardPart::Alone),
            body: None,
            width: signal(0.0),
        }
    }

    /// Uma view qualquer no lugar do texto, como filho de um `SettingsExpander` (uma lista,
    /// um aviso). Pinta e mede como um filho, sem cabeçalho.
    pub(crate) fn body_boxed(view: Box<dyn Widget>) -> Self {
        let mut card = Self::new("");
        card.body = Some(view);
        card
    }

    /// Descrição, em 12 px na cor secundária, que quebra linha (fixa, closure ou signal).
    pub fn description<M>(mut self, text: impl IntoProp<String, M>) -> Self {
        self.description = Some(text.into_prop());
        self
    }

    /// Ícone à esquerda, de 20 px.
    pub fn icon(mut self, symbol: Symbol) -> Self {
        self.icon = Some(symbol);
        self
    }

    /// O controle do lado direito (`ToggleSwitch`, `ComboBox`, `NumberBox`, `Button`...).
    /// Chamar de novo troca o anterior.
    pub fn content(mut self, view: impl View) -> Self {
        self.content = Some(Box::new(view));
        self
    }

    /// Como [`SettingsCard::content`], com a view já pronta.
    pub(crate) fn content_boxed(mut self, view: Box<dyn Widget>) -> Self {
        self.content = Some(view);
        self
    }

    /// Torna o card inteiro clicável, com o chevron à direita: `f` roda no clique, no Enter e
    /// no Espaço. Um card clicável sem `.content` é um botão grande; o texto dele é o rótulo.
    pub fn on_click(mut self, f: impl FnMut() + 'static) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }

    /// Troca o chevron do card clicável por outro glifo (o `ActionIcon` do Toolkit):
    /// [`Symbol::Open`] para "abre fora do app".
    pub fn action_icon<M>(mut self, symbol: impl IntoProp<Symbol, M>) -> Self {
        self.action_icon = symbol.into_prop();
        self
    }

    /// Desabilita o card (fixo, closure ou signal): não recebe foco nem cliques, e o texto
    /// de dentro fica com a cor desabilitada. O controle de `.content` se desabilita à parte.
    pub fn disabled<M>(mut self, disabled: impl IntoProp<bool, M>) -> Self {
        self.disabled = disabled.into_prop();
        self
    }

    /// O cabeçalho agora.
    pub fn header(&self) -> String {
        untrack(|| self.header.get())
    }

    /// Se tem `on_click`.
    pub fn is_clickable(&self) -> bool {
        self.on_click.is_some()
    }

    /// Se está desabilitado agora.
    pub fn is_disabled(&self) -> bool {
        self.disabled.get()
    }

    /// O `SettingsExpander` diz onde o card fica.
    pub(crate) fn part<M>(mut self, part: impl IntoProp<SettingsCardPart, M>) -> Self {
        self.part = part.into_prop();
        self
    }

    /// O cabeçalho do expander tem o botão do chevron de 32 px em vez do glifo solto.
    pub(crate) fn chevron_button(mut self) -> Self {
        self.chevron_button = true;
        self
    }

    fn part_now(&self) -> SettingsCardPart {
        untrack(|| self.part.get())
    }

    fn padding(&self) -> Insets {
        match self.part_now() {
            SettingsCardPart::Alone => Insets::all(SETTINGS_CARD_PADDING),
            SettingsCardPart::ExpanderHeader { .. } => HEADER_PADDING,
            SettingsCardPart::ExpanderItem { .. } if self.is_clickable() => CLICKABLE_ITEM_PADDING,
            SettingsCardPart::ExpanderItem { .. } => ITEM_PADDING,
        }
    }

    /// O espaço que as bordas ocupam, para o layout: o do cabeçalho de expander aberto
    /// guarda também a de baixo, que então não é pintada.
    fn borders(&self) -> Insets {
        let b = SETTINGS_CARD_BORDER;
        match self.part_now() {
            SettingsCardPart::Alone | SettingsCardPart::ExpanderHeader { .. } => Insets::all(b),
            SettingsCardPart::ExpanderItem { last } => {
                Insets::new(b, b, if last { b } else { 0.0 }, b)
            }
        }
    }

    fn min_height(&self) -> f64 {
        match self.part_now() {
            SettingsCardPart::ExpanderItem { .. } => SETTINGS_EXPANDER_ITEM_MIN_HEIGHT,
            _ => SETTINGS_CARD_MIN_HEIGHT,
        }
    }

    /// Largura do layout do card, a que decide a quebra.
    fn layout_width(&self) -> f64 {
        self.width.get()
    }

    /// Se o controle desceu para baixo do texto (e se o ícone aparece).
    fn wrapped_for(width: f64) -> (bool, bool) {
        (
            width < SETTINGS_CARD_WRAP_THRESHOLD,
            width >= SETTINGS_CARD_WRAP_NO_ICON_THRESHOLD,
        )
    }

    /// Estado para o `Drawer`.
    pub fn visual(&self) -> SettingsCardVisual {
        let clickable = self.is_clickable();
        let (wrapped, show_icon) = Self::wrapped_for(self.layout_width());
        let base = SettingsCardVisual {
            clickable,
            wrapped,
            show_icon: show_icon && self.icon.is_some(),
            part: self.part.get(),
            ..SettingsCardVisual::default()
        };
        if self.is_disabled() {
            return SettingsCardVisual {
                state: ButtonState::Disabled,
                ..base
            };
        }
        if !clickable {
            return base;
        }
        SettingsCardVisual {
            state: self.press.state(),
            focus_ring: self.press.focus_ring(),
            ..base
        }
    }

    /// Trata um evento. Devolve `true` se o card o consumiu.
    pub fn handle(&mut self, event: &Event) -> bool {
        if self.is_disabled() || !self.is_clickable() {
            self.press.reset();
            return false;
        }
        let (consumed, clicked) = self.press.handle(event, true);
        if clicked && let Some(f) = &mut self.on_click {
            f();
        }
        consumed
    }
}

common_builders!(SettingsCard);

impl Widget for SettingsCard {
    fn kind(&self) -> &'static str {
        "settings-card"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::SettingsCard(self.visual())
    }

    fn flex(&self) -> Option<Flex> {
        let padding = self.padding();
        let borders = self.borders();
        Some(Flex {
            direction: Direction::Row,
            wrap: true,
            align: Align::Center,
            padding: Insets::new(
                padding.top + borders.top,
                padding.right + borders.right,
                padding.bottom + borders.bottom,
                padding.left + borders.left,
            ),
            ..Flex::default()
        })
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        let padding = self.padding();
        let borders = self.borders();
        let width = self.width;
        let inner_padding = (
            padding.left + padding.right + borders.left + borders.right,
            padding.top + padding.bottom + borders.top + borders.bottom,
        );
        let wrapped = move || width.get() < SETTINGS_CARD_WRAP_THRESHOLD;
        let show_icon = move || width.get() >= SETTINGS_CARD_WRAP_NO_ICON_THRESHOLD;
        let mut children: Vec<Box<dyn Widget>> = Vec::new();

        // Garante a altura mínima: um item sem largura que só tem altura.
        children.push(Box::new(
            Row::new()
                .width(0.0)
                .height((self.min_height() - inner_padding.1).max(0.0)),
        ));

        if let Some(body) = self.body.take() {
            children.push(Box::new(Lead {
                common: Common {
                    grow: 1.0,
                    ..Common::default()
                },
                flex: Flex::default(),
                children: vec![body],
                basis: Box::new(move || {
                    let line = (width.get() - inner_padding.0).max(0.0);
                    (line, 0.0)
                }),
            }));
            return children;
        }

        // Ícone, cabeçalho e descrição.
        let mut lead: Vec<Box<dyn Widget>> = Vec::new();
        if let Some(symbol) = self.icon {
            lead.push(Box::new(
                Row::new()
                    .padding(Insets::new(0.0, ICON_MARGIN.1, 0.0, ICON_MARGIN.0))
                    .visible(show_icon)
                    .child(Icon::new(symbol).size(ICON_SIZE)),
            ));
        }
        let header = self.header.clone();
        let mut texts = Column::new()
            .grow(1.0)
            .child(Text::new(move || header.get()).wrap());
        if let Some(description) = self.description.clone() {
            let secondary = Text::new(move || description.get())
                .caption()
                .color(Token::TextFillColorSecondary)
                .wrap();
            texts = texts.child(secondary);
        }
        lead.push(Box::new(texts));
        children.push(Box::new(Lead {
            common: Common {
                grow: 1.0,
                ..Common::default()
            },
            flex: Flex {
                direction: Direction::Row,
                align: Align::Center,
                ..Flex::default()
            },
            children: lead,
            basis: Box::new(move || {
                if width.get() < SETTINGS_CARD_WRAP_THRESHOLD {
                    // Ocupa a linha inteira: o controle vai para a de baixo.
                    let line = (width.get() - inner_padding.0).max(0.0);
                    (line, 0.0)
                } else {
                    (MIN_WIDTH, 0.0)
                }
            }),
        }));

        // O controle, com 24 de espaço à esquerda ou, quebrado, 8 de espaço em cima.
        if let Some(content) = self.content.take() {
            children.push(Box::new(
                Column::new()
                    .align(Align::Start)
                    .child(Row::new().height(WRAPPED_GAP).visible(wrapped))
                    .child(
                        Row::new()
                            .align(Align::Center)
                            .child(Row::new().width(CONTENT_GAP).visible(move || !wrapped()))
                            .child(Row::new().boxed(content)),
                    ),
            ));
        }

        // O chevron do card clicável.
        if self.is_clickable() {
            let action = self.action_icon.clone();
            if self.chevron_button {
                children.push(Box::new(
                    Row::new()
                        .width(CHEVRON_BUTTON)
                        .height(CHEVRON_BUTTON)
                        .align(Align::Center)
                        .justify(Justify::Center)
                        .child(Icon::new(move || action.get()).size(CHEVRON_GLYPH)),
                ));
            } else {
                children.push(Box::new(
                    Row::new()
                        .padding(Insets::new(0.0, 0.0, 0.0, ACTION_ICON_MARGIN))
                        .child(Icon::new(move || action.get()).size(ACTION_ICON_SIZE)),
                ));
            }
        }
        children
    }

    fn handle(&mut self, event: &Event, _: &mut EventCx) -> bool {
        SettingsCard::handle(self, event)
    }

    fn interactive(&self) -> bool {
        self.is_clickable()
    }

    fn focusable(&self) -> bool {
        self.is_clickable() && !self.is_disabled()
    }

    fn enabled(&self) -> bool {
        !self.is_disabled()
    }

    fn text(&self) -> Option<String> {
        Some(self.header.get()).filter(|text| !text.is_empty())
    }

    fn foreground(&self) -> Option<crate::color::ThemeColor> {
        match self.visual().state {
            ButtonState::Rest | ButtonState::Hover => None,
            ButtonState::Pressed => Some(Token::TextFillColorSecondary.into()),
            ButtonState::Disabled => Some(Token::TextFillColorDisabled.into()),
        }
    }

    fn passes_foreground(&self) -> bool {
        true
    }

    fn resized(&mut self, size: Size) {
        if (self.width.get_untracked() - size.width).abs() > 0.5 {
            self.width.set(size.width);
        }
    }
}

impl fmt::Debug for SettingsCard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SettingsCard")
            .field("common", &self.common)
            .field("header", &self.header)
            .field("disabled", &self.disabled)
            .field("visual", &self.visual())
            .finish_non_exhaustive()
    }
}
