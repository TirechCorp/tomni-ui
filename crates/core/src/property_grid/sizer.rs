//! O divisor do painel de descrição e o painel que ele redimensiona.

use std::fmt;
use std::rc::Rc;

use tomni_ui_reactive::prelude::*;

use super::shared::Shared;
use super::visual::PropertyGridSizerVisual;
use crate::button::ButtonState;
use crate::cx::EventCx;
use crate::event::{Cursor, Event};
use crate::widget::{Common, Flex, Look, Widget};

/// O divisor acima do painel de descrição: arrastado para cima, o painel cresce.
pub(crate) struct DescriptionSizer {
    common: Common,
    shared: Rc<Shared>,
    hovered: bool,
    /// De onde o arrasto saiu: o y do ponteiro e a altura do painel nesse momento.
    drag: Option<(f64, f64)>,
}

impl DescriptionSizer {
    pub(crate) fn new(shared: &Rc<Shared>) -> Self {
        Self {
            common: Common::generated(shared.sub("divisor")),
            shared: Rc::clone(shared),
            hovered: false,
            drag: None,
        }
    }
}

impl Widget for DescriptionSizer {
    fn kind(&self) -> &'static str {
        "property_grid_sizer"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::PropertyGridSizer(PropertyGridSizerVisual {
            state: if self.drag.is_some() {
                ButtonState::Pressed
            } else if self.hovered {
                ButtonState::Hover
            } else {
                ButtonState::Rest
            },
        })
    }

    fn handle(&mut self, event: &Event, cx: &mut EventCx) -> bool {
        match *event {
            Event::PointerMove { kind, inside } => {
                self.hovered = inside && kind.hovers();
                if let Some((from, height)) = self.drag {
                    self.shared
                        .set_description_height(height + from - cx.pointer().y);
                }
                true
            }
            Event::PointerDown { kind, .. } => {
                self.hovered = kind.hovers();
                self.drag = Some((
                    cx.pointer().y,
                    untrack(|| self.shared.description_height.get()),
                ));
                true
            }
            Event::PointerUp { kind, inside } => {
                self.hovered = inside && kind.hovers();
                self.drag = None;
                true
            }
            Event::PointerCancel => {
                self.hovered = false;
                self.drag = None;
                true
            }
            _ => false,
        }
    }

    fn interactive(&self) -> bool {
        true
    }

    fn cursor(&self) -> Option<Cursor> {
        Some(Cursor::NsResize)
    }
}

impl fmt::Debug for DescriptionSizer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DescriptionSizer")
            .field("dragging", &self.drag.is_some())
            .finish_non_exhaustive()
    }
}

/// O contêiner do painel de descrição: tem a altura que o divisor dá.
pub(crate) struct DescriptionPane {
    common: Common,
    shared: Rc<Shared>,
    child: Option<Box<dyn Widget>>,
}

impl DescriptionPane {
    pub(crate) fn new(shared: &Rc<Shared>, child: Box<dyn Widget>) -> Self {
        Self {
            common: Common::default(),
            shared: Rc::clone(shared),
            child: Some(child),
        }
    }
}

impl Widget for DescriptionPane {
    fn kind(&self) -> &'static str {
        "property_grid_description_pane"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::Column
    }

    fn flex(&self) -> Option<Flex> {
        Some(Flex {
            fill: true,
            ..Flex::default()
        })
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        self.child.take().into_iter().collect()
    }

    fn dynamic_basis(&self) -> Option<(f64, f64)> {
        Some((self.shared.description_height.get(), 0.0))
    }
}

impl fmt::Debug for DescriptionPane {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DescriptionPane").finish_non_exhaustive()
    }
}
