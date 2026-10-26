//! `SettingsExpander`: um `SettingsCard` que abre e fecha uma lista de filhos embaixo dele.

use std::fmt;

use tomni_ui_reactive::{Get, GetUntracked, Set, Signal};

use crate::containers::Column;
use crate::prop::{IntoProp, Prop};
use crate::settings_card::{SettingsCard, SettingsCardPart};
use crate::symbol::Symbol;
use crate::widget::{Common, Flex, Look, View, Widget, common_builders};

enum Item {
    Card(Box<SettingsCard>),
    View(Box<dyn Widget>),
}

/// Um `SettingsCard` que mostra e esconde uma lista de filhos, ligado a um `Signal<bool>`.
///
/// ```
/// use tomni_ui_core::{SettingsCard, SettingsExpander, Symbol};
/// use tomni_ui_reactive::prelude::*;
///
/// # let scope = Scope::root();
/// # scope.run(|| {
/// let aberto = signal(false);
/// let grupo = SettingsExpander::new("Terminal", aberto)
///     .id("terminal")
///     .description("Fonte, cores e o cursor.")
///     .icon(Symbol::WindowConsole)
///     .item(SettingsCard::new("Tamanho da fonte"))
///     .item(SettingsCard::new("Formato do cursor"));
/// assert!(!grupo.is_open());
/// # });
/// # scope.clear();
/// ```
///
/// É o `SettingsExpander` do Windows Community Toolkit. O cabeçalho é um `SettingsCard`
/// clicável (em qualquer parte dele, menos no controle de `.content`) com o chevron à direita;
/// os filhos perdem o fundo próprio, ganham o recuo de 58 px que alinha o texto deles com o do
/// cabeçalho e um separador em cima. Aberto, o cabeçalho tem só os cantos de cima
/// arredondados e o último filho os de baixo. O cabeçalho tem o id `{id}-cabecalho` e o corpo
/// `{id}-conteudo`, que some do layout enquanto fechado.
pub struct SettingsExpander {
    common: Common,
    header: Prop<String>,
    description: Option<Prop<String>>,
    icon: Option<Symbol>,
    content: Option<Box<dyn Widget>>,
    open: Signal<bool>,
    items: Vec<Item>,
}

impl SettingsExpander {
    /// Expander com o cabeçalho `header` (fixo, closure ou signal), aberto enquanto `open`
    /// for `true`.
    pub fn new<M>(header: impl IntoProp<String, M>, open: Signal<bool>) -> Self {
        Self {
            common: Common::default(),
            header: header.into_prop(),
            description: None,
            icon: None,
            content: None,
            open,
            items: Vec::new(),
        }
    }

    /// Descrição do cabeçalho.
    pub fn description<M>(mut self, text: impl IntoProp<String, M>) -> Self {
        self.description = Some(text.into_prop());
        self
    }

    /// Ícone do cabeçalho.
    pub fn icon(mut self, symbol: Symbol) -> Self {
        self.icon = Some(symbol);
        self
    }

    /// Controle no cabeçalho, à esquerda do chevron (por exemplo o `ToggleSwitch` "ligado").
    pub fn content(mut self, view: impl View) -> Self {
        self.content = Some(Box::new(view));
        self
    }

    /// Um filho, em geral um `SettingsCard`: dentro do expander ele perde fundo, borda e raio e
    /// ganha o recuo de filho e o separador de cima.
    pub fn item(mut self, card: SettingsCard) -> Self {
        self.items.push(Item::Card(Box::new(card)));
        self
    }

    /// Qualquer view como filho (uma lista, um aviso), com o mesmo recuo.
    pub fn item_view(mut self, view: impl View) -> Self {
        self.items.push(Item::View(Box::new(view)));
        self
    }

    /// Se está aberto agora.
    pub fn is_open(&self) -> bool {
        self.open.get()
    }
}

common_builders!(SettingsExpander);

impl Widget for SettingsExpander {
    fn kind(&self) -> &'static str {
        "settings-expander"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::Column
    }

    fn flex(&self) -> Option<Flex> {
        Some(Flex::default())
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        let open = self.open;
        let header = self.header.clone();
        let mut head = SettingsCard::new(move || header.get())
            .part(move || SettingsCardPart::ExpanderHeader { open: open.get() })
            .chevron_button()
            .on_click(move || open.set(!open.get_untracked()))
            .action_icon(move || {
                if open.get() {
                    Symbol::ChevronUp
                } else {
                    Symbol::ChevronDown
                }
            });
        if let Some(description) = self.description.take() {
            head = head.description(move || description.get());
        }
        if let Some(icon) = self.icon {
            head = head.icon(icon);
        }
        if let Some(content) = self.content.take() {
            head = head.content_boxed(content);
        }
        let mut body = Column::new().visible(move || open.get());
        if let Some(id) = &self.common.id {
            head = head.id(format!("{id}-cabecalho"));
            body = body.id(format!("{id}-conteudo"));
        }
        let count = self.items.len();
        for (index, item) in std::mem::take(&mut self.items).into_iter().enumerate() {
            let part = SettingsCardPart::ExpanderItem {
                last: index + 1 == count,
            };
            body = match item {
                Item::Card(card) => body.child(card.part(part)),
                Item::View(view) => body.child(SettingsCard::body_boxed(view).part(part)),
            };
        }
        vec![Box::new(head), Box::new(body)]
    }

    fn text(&self) -> Option<String> {
        Some(self.header.get())
    }
}

impl fmt::Debug for SettingsExpander {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SettingsExpander")
            .field("common", &self.common)
            .field("header", &self.header)
            .field("open", &self.open)
            .finish_non_exhaustive()
    }
}
