//! `Expander`: cabeçalho que mostra e esconde um conteúdo embaixo dele (sanfona).

use std::fmt;

use kurbo::Rect;
use tomni_ui_reactive::{Get, GetUntracked, Set, Signal};

use crate::button::ButtonState;
use crate::containers::{Column, Row};
use crate::cx::EventCx;
use crate::event::Event;
use crate::press::Pressable;
use crate::prop::{IntoProp, Prop};
use crate::text::Text;
use crate::widget::{Align, Common, Flex, Insets, Look, View, Widget, common_builders};

/// Altura do cabeçalho (`ExpanderMinHeight`).
pub const EXPANDER_HEADER_HEIGHT: f64 = 48.0;
/// Altura do cabeçalho com `.description`: o título (20 px de linha) e a descrição (16) com 14
/// em cima e embaixo. O WinUI cresce o cabeçalho com o conteúdo; aqui a altura é fixa.
pub const EXPANDER_DESCRIBED_HEADER_HEIGHT: f64 = 64.0;
/// `ExpanderHeaderPadding` (16 à esquerda) e, à direita, o lugar do chevron:
/// `ExpanderChevronMargin` (20 e 8) e o botão de 32.
const HEADER_PADDING: Insets = Insets::new(0.0, 20.0 + 32.0 + 8.0, 0.0, 16.0);
/// `ExpanderContentPadding`.
const CONTENT_PADDING: f64 = 16.0;

/// Tudo o que o `Drawer` precisa saber de um `Expander` para pintá-lo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ExpanderVisual {
    /// Se o conteúdo está à mostra.
    pub open: bool,
    /// Estado do cabeçalho.
    pub state: ButtonState,
    /// Anel de foco no cabeçalho: só com foco vindo do teclado.
    pub focus_ring: bool,
    /// Se o cabeçalho tem a linha da descrição (e a altura de
    /// [`EXPANDER_DESCRIBED_HEADER_HEIGHT`]).
    pub described: bool,
}

impl ExpanderVisual {
    /// Altura do cabeçalho.
    pub fn header_height(&self) -> f64 {
        if self.described {
            EXPANDER_DESCRIBED_HEADER_HEIGHT
        } else {
            EXPANDER_HEADER_HEIGHT
        }
    }
}

/// Cabeçalho que mostra e esconde um conteúdo embaixo dele, ligado a um `Signal<bool>`.
///
/// ```
/// use tomni_ui_core::{Column, Expander, Text};
/// use tomni_ui_reactive::prelude::*;
///
/// # let scope = Scope::root();
/// # scope.run(|| {
/// let aberto = signal(false);
/// let sanfona = Expander::new("Avançado", aberto)
///     .id("avancado")
///     .content(Column::new().gap(8).child(Text::new("Opções raras.")));
/// assert!(!sanfona.is_open());
/// # });
/// # scope.clear();
/// ```
///
/// Um clique no cabeçalho, Enter ou Espaço abrem e fecham; `open` acompanha. O cabeçalho tem
/// 48 px de altura, com o texto à esquerda e a seta à direita; o conteúdo fica num painel
/// embaixo, com 16 px de margem, e some do layout quando fechado. O painel tem o id
/// `{id}-conteudo`. Com `.description`, o cabeçalho ganha uma segunda linha (legenda, na cor
/// secundária) e passa a ter 64 px.
pub struct Expander {
    common: Common,
    header: Prop<String>,
    description: Option<Prop<String>>,
    open: Signal<bool>,
    content: Option<Box<dyn Widget>>,
    press: Pressable,
}

impl Expander {
    /// Expander com o cabeçalho `header` (fixo, closure ou signal), aberto enquanto `open` for
    /// `true`.
    pub fn new<M>(header: impl IntoProp<String, M>, open: Signal<bool>) -> Self {
        Self {
            common: Common::default(),
            header: header.into_prop(),
            description: None,
            open,
            content: None,
            press: Pressable::default(),
        }
    }

    /// Uma linha de descrição embaixo do título do cabeçalho (fixa, closure ou signal).
    pub fn description<M>(mut self, text: impl IntoProp<String, M>) -> Self {
        self.description = Some(text.into_prop());
        self
    }

    /// Conteúdo que aparece com o expander aberto. Chamar de novo troca o anterior.
    pub fn content(mut self, content: impl View) -> Self {
        self.content = Some(Box::new(content));
        self
    }

    /// Se está aberto agora.
    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    /// Altura do cabeçalho. Não lê o `open`: o layout e o clique pedem a altura fora de um
    /// efeito.
    fn header_height(&self) -> f64 {
        ExpanderVisual {
            described: self.description.is_some(),
            ..ExpanderVisual::default()
        }
        .header_height()
    }

    /// Estado para o `Drawer`.
    pub fn visual(&self) -> ExpanderVisual {
        ExpanderVisual {
            open: self.open.get(),
            state: self.press.state(),
            focus_ring: self.press.focus_ring(),
            described: self.description.is_some(),
        }
    }
}

common_builders!(Expander);

impl Widget for Expander {
    fn kind(&self) -> &'static str {
        "expander"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::Expander(self.visual())
    }

    fn flex(&self) -> Option<Flex> {
        Some(Flex::default())
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        let header = self.header.clone();
        let open = self.open;
        let mut content = Column::new()
            .padding(CONTENT_PADDING)
            .visible(move || open.get());
        if let Some(id) = &self.common.id {
            content = content.id(format!("{id}-conteudo"));
        }
        if let Some(child) = self.content.take() {
            content = content.boxed(child);
        }
        let title = Text::new(move || header.get());
        let head = match self.description.clone() {
            Some(description) => Row::new().child(
                Column::new().grow(1).child(title).child(
                    Text::new(move || description.get())
                        .caption()
                        .secondary()
                        .max_lines(1),
                ),
            ),
            None => Row::new().child(title),
        };
        vec![
            Box::new(
                head.height(self.header_height())
                    .padding(HEADER_PADDING)
                    .align(Align::Center),
            ),
            Box::new(content),
        ]
    }

    fn handle(&mut self, event: &Event, _: &mut EventCx) -> bool {
        let (consumed, clicked) = self.press.handle(event, true);
        if clicked {
            self.open.set(!self.open.get_untracked());
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
            rect.y1.min(rect.y0 + self.header_height()),
        )
    }

    fn focusable(&self) -> bool {
        true
    }

    fn text(&self) -> Option<String> {
        Some(self.header.get())
    }
}

impl fmt::Debug for Expander {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Expander")
            .field("common", &self.common)
            .field("header", &self.header)
            .field("visual", &self.visual())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use kurbo::Point;
    use tomni_ui_reactive::prelude::*;

    use super::*;
    use crate::cx::{FixedMetrics, MemoryClipboard};
    use crate::event::{FocusSource, Key, Modifiers, PointerKind::Mouse};

    fn send(e: &mut Expander, event: Event) {
        let mut metrics = FixedMetrics::default();
        let mut clipboard = MemoryClipboard::default();
        let mut cx = EventCx::new(
            Rect::ZERO,
            Point::ZERO,
            Modifiers::NONE,
            &mut metrics,
            &mut clipboard,
        );
        e.handle(&event, &mut cx);
    }

    #[test]
    fn clique_enter_e_espaco_alternam() {
        let scope = Scope::root();
        scope.run(|| {
            let aberto = signal(false);
            let mut e = Expander::new("Mais", aberto).id("mais");
            send(
                &mut e,
                Event::PointerDown {
                    kind: Mouse,
                    clicks: 1,
                },
            );
            send(
                &mut e,
                Event::PointerUp {
                    kind: Mouse,
                    inside: true,
                },
            );
            assert!(aberto.get());
            assert!(e.visual().open);
            send(&mut e, Event::FocusIn(FocusSource::Keyboard));
            send(&mut e, Event::KeyDown(Key::Enter));
            assert!(!aberto.get());
            send(&mut e, Event::KeyDown(Key::Space));
            send(&mut e, Event::KeyUp(Key::Space));
            assert!(aberto.get());
        });
        scope.clear();
    }

    #[test]
    fn so_o_cabecalho_recebe_o_ponteiro() {
        let scope = Scope::root();
        scope.run(|| {
            let mut e = Expander::new("Mais", signal(true)).id("mais");
            let rect = Rect::new(0.0, 0.0, 300.0, 200.0);
            assert_eq!(e.hit_rect(rect), Rect::new(0.0, 0.0, 300.0, 48.0));
            let children = e.take_children();
            assert_eq!(children[1].common().id.as_deref(), Some("mais-conteudo"));
            assert!(children[1].visible());
        });
        scope.clear();
    }

    #[test]
    fn descricao_aumenta_o_cabecalho() {
        let scope = Scope::root();
        scope.run(|| {
            let mut e = Expander::new("Geral", signal(false))
                .id("geral")
                .description("Tema e idioma");
            assert!(e.visual().described);
            let rect = Rect::new(0.0, 0.0, 300.0, 200.0);
            assert_eq!(e.hit_rect(rect).height(), EXPANDER_DESCRIBED_HEADER_HEIGHT);
            assert_eq!(
                Expander::new("Geral", signal(false))
                    .visual()
                    .header_height(),
                48.0
            );
            let children = e.take_children();
            assert_eq!(children.len(), 2);
        });
        scope.clear();
    }
}
