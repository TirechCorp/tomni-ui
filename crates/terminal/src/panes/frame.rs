//! O quadro de um painel do `TerminalPanes`: a borda de 1 px que mostra o painel ativo e os que
//! recebem o envio para vários, e o alvo onde se solta uma aba arrastada.

use std::any::Any;
use std::fmt;
use std::rc::Rc;

use tomni_ui_core::{
    Common, CustomLook, CustomVisual, Direction, Event, EventCx, Flex, Insets, Look, Prop, Widget,
};

/// Espessura da borda do painel, também o espaço que o quadro reserva em volta do conteúdo.
pub const PANE_BORDER: f64 = 1.0;
/// Comprimento do traço e do vão da borda tracejada, em px.
pub const PANE_DASH: f64 = 4.0;

/// A borda de um painel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PaneBorder {
    /// Sem borda.
    #[default]
    None,
    /// Sólida, na cor de destaque: o painel ativo.
    Solid,
    /// Tracejada, na cor de destaque: um painel que recebe o que se digita no ativo.
    Dashed,
}

/// O que o `Drawer` precisa para pintar o quadro de um painel.
#[derive(Clone, Debug, PartialEq)]
pub struct PaneVisual {
    /// A borda.
    pub border: PaneBorder,
}

impl CustomVisual for PaneVisual {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn eq_visual(&self, other: &dyn CustomVisual) -> bool {
        other
            .as_any()
            .downcast_ref::<Self>()
            .is_some_and(|other| self == other)
    }
}

type Dropped = Rc<dyn Fn(String)>;

/// O quadro de um painel: um contêiner em coluna com 1 px de margem, onde a borda aparece.
/// Aceita uma aba arrastada (do grupo do `TerminalPanes`) solta fora da faixa de abas.
pub(crate) struct PaneFrame {
    common: Common,
    border: Prop<PaneBorder>,
    /// O `kind` do arrasto das abas do grupo.
    drag_kind: String,
    on_drop: Dropped,
    /// Uma aba está sobre o quadro.
    over: bool,
    children: Vec<Box<dyn Widget>>,
}

impl PaneFrame {
    pub(crate) fn new(
        common: Common,
        border: Prop<PaneBorder>,
        drag_kind: String,
        on_drop: Dropped,
        children: Vec<Box<dyn Widget>>,
    ) -> Self {
        Self {
            common,
            border,
            drag_kind,
            on_drop,
            over: false,
            children,
        }
    }
}

impl Widget for PaneFrame {
    fn kind(&self) -> &'static str {
        "pane_frame"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        let border = if self.over {
            PaneBorder::Solid
        } else {
            self.border.get()
        };
        Look::Custom(CustomLook::new(PaneVisual { border }))
    }

    fn flex(&self) -> Option<Flex> {
        Some(Flex {
            direction: Direction::Column,
            padding: Insets::all(PANE_BORDER),
            ..Flex::default()
        })
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        std::mem::take(&mut self.children)
    }

    fn handle(&mut self, event: &Event, _cx: &mut EventCx) -> bool {
        match event {
            Event::DragOver(data) if data.kind == self.drag_kind => {
                self.over = true;
                true
            }
            Event::DragLeave if self.over => {
                self.over = false;
                true
            }
            Event::Drop(data) if data.kind == self.drag_kind => {
                self.over = false;
                (self.on_drop)(data.key.clone());
                true
            }
            _ => false,
        }
    }
}

impl fmt::Debug for PaneFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PaneFrame")
            .field("common", &self.common)
            .field("over", &self.over)
            .finish_non_exhaustive()
    }
}
