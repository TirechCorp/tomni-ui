//! `ToolTip`: a dica de `.tooltip()`, que a árvore cria, abre e fecha.

use std::fmt;

use kurbo::Size;
use tomni_ui_reactive::{Get, Set, Signal, signal};

use crate::prop::Prop;
use crate::text::{Text, TextRamp};
use crate::widget::{Common, Flex, Insets, Look, Placement, PopupKind, Widget};

/// `ToolTipBorderThemePadding`, com a borda de 1 px somada.
const PADDING: Insets = Insets::new(6.0, 9.0, 8.0, 9.0);
/// `ToolTipMaxWidth`.
const MAX_WIDTH: f64 = 320.0;

/// Dica de um widget, criada pela árvore a partir do `.tooltip()` como filha dele.
pub(crate) struct ToolTip {
    common: Common,
    open: Signal<bool>,
    content: Prop<String>,
    text: Option<Text>,
}

impl ToolTip {
    /// Dica com `text`, fechada. `owner` é o id do widget dono, se tiver.
    pub(crate) fn new(text: Prop<String>, owner: Option<&str>) -> Self {
        Self {
            common: Common {
                id: owner.map(|id| format!("{id}-tooltip")),
                ..Common::default()
            },
            open: signal(false),
            content: text.clone(),
            text: Some(Text::new(move || text.get()).ramp(TextRamp::Caption).wrap()),
        }
    }
}

impl Widget for ToolTip {
    fn kind(&self) -> &'static str {
        "tooltip"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    /// Aberta e com texto: uma dica vazia (a do item do `NavigationView` com o painel
    /// aberto) não aparece.
    fn visible(&self) -> bool {
        self.open.get() && !self.content.get().is_empty()
    }

    fn look(&self) -> Look {
        Look::Popup(PopupKind::ToolTip)
    }

    fn flex(&self) -> Option<Flex> {
        Some(Flex {
            padding: PADDING,
            ..Flex::default()
        })
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        self.text
            .take()
            .map(|t| Box::new(t) as Box<dyn Widget>)
            .into_iter()
            .collect()
    }

    fn layer(&self) -> Option<crate::Layer> {
        Some(crate::Layer {
            light_dismiss: false,
            input: false,
            placement: Placement::Above,
            max_size: Size::new(MAX_WIDTH, f64::INFINITY),
            ..crate::Layer::default()
        })
    }

    fn set_open(&mut self, open: bool) {
        self.open.set(open);
    }
}

impl fmt::Debug for ToolTip {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToolTip")
            .field("common", &self.common)
            .finish_non_exhaustive()
    }
}
