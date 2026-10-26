//! `Split`: dois painéis com um divisor que o usuário arrasta entre eles.

use std::cell::Cell;
use std::fmt;
use std::rc::Rc;

use tomni_ui_reactive::{Get, GetUntracked, Set, Signal, signal};

use crate::button::ButtonState;
use crate::cx::EventCx;
use crate::event::{Cursor, Event, FocusSource, Key};
use crate::kurbo::Size;
use crate::slider::format_number;
use crate::widget::{Common, Direction, Flex, Look, View, Widget, common_builders};

/// Espessura do divisor (o `MinWidth`/`MinHeight` do `SizerBase`).
pub const SPLIT_SIZER: f64 = 8.0;
/// Quanto as setas movem o divisor (`KeyboardIncrement`).
pub const SPLIT_KEY_STEP: f64 = 8.0;
/// Tamanho do primeiro painel quando o app não dá um signal.
const DEFAULT_SIZE: f64 = 240.0;

/// Tudo o que o `Drawer` precisa saber do divisor de um [`Split`] para pintá-lo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SizerVisual {
    /// Estado: `Hover` com o ponteiro em cima, `Pressed` arrastando.
    pub state: ButtonState,
    /// Anel de foco: só com foco vindo do teclado.
    pub focus_ring: bool,
    /// Eixo dos painéis: [`Direction::Row`] (lado a lado, o divisor é uma barra em pé) ou
    /// [`Direction::Column`] (um em cima do outro, o divisor é deitado).
    pub direction: Direction,
}

/// O que o `Split` e o divisor dividem.
struct Shared {
    direction: Direction,
    size: Signal<f64>,
    /// Fração do primeiro painel no espaço sem o divisor; com ela, o `size` não é usado.
    ratio: Option<Signal<f64>>,
    min_first: Cell<f64>,
    min_second: Cell<f64>,
    /// Comprimento do `Split` no eixo dos painéis, do último layout (0 antes do primeiro).
    length: Cell<f64>,
}

impl Shared {
    /// `size` dentro dos mínimos dos dois painéis. Antes do primeiro layout, só o do primeiro.
    fn clamp(&self, size: f64) -> f64 {
        let min = self.min_first.get();
        let length = self.length.get();
        let max = if length > 0.0 {
            (length - SPLIT_SIZER - self.min_second.get()).max(min)
        } else {
            f64::INFINITY
        };
        size.round().clamp(min, max)
    }

    /// O `text()` do `Split` e do divisor: os px, ou a fração com duas casas.
    fn text(&self) -> String {
        match self.ratio {
            Some(ratio) => format!("{:.2}", ratio.get()),
            None => format_number(self.size.get()),
        }
    }

    fn set(&self, size: f64) {
        let size = self.clamp(size);
        if self.size.get_untracked() != size {
            self.size.set(size);
        }
    }

    /// Espaço que os dois painéis dividem: o comprimento sem o divisor (1 antes do primeiro
    /// layout, para a divisão não dar zero).
    fn room(&self) -> f64 {
        (self.length.get() - SPLIT_SIZER).max(1.0)
    }

    /// `ratio` dentro dos mínimos dos dois painéis. Antes do primeiro layout, só entre 0 e 1.
    fn clamp_ratio(&self, ratio: f64) -> f64 {
        if self.length.get() <= 0.0 {
            return ratio.clamp(0.0, 1.0);
        }
        let room = self.room();
        let min = (self.min_first.get() / room).min(1.0);
        let max = (1.0 - self.min_second.get() / room).max(min);
        ratio.clamp(min, max)
    }

    fn set_ratio(&self, ratio: f64) {
        let Some(signal) = self.ratio else {
            return;
        };
        let ratio = self.clamp_ratio(ratio);
        if signal.get_untracked() != ratio {
            signal.set(ratio);
        }
    }

    /// A posição do divisor, na unidade que o arrasto usa: px sem `ratio`, fração com ele.
    fn position(&self) -> f64 {
        match self.ratio {
            Some(ratio) => ratio.get_untracked(),
            None => self.size.get_untracked(),
        }
    }

    /// Move o divisor para `position` (na unidade de [`Shared::position`]) a partir de um
    /// deslocamento em px.
    fn move_to(&self, start: f64, delta: f64) {
        if self.ratio.is_some() {
            self.set_ratio(start + delta / self.room());
        } else {
            self.set(start + delta);
        }
    }

    fn step(&self, direction: f64) {
        self.move_to(self.position(), direction * SPLIT_KEY_STEP);
    }
}

/// Dois painéis, lado a lado ([`Split::row`]) ou um em cima do outro ([`Split::column`]),
/// com um divisor entre eles, como o `GridSplitter` (o `SizerBase`) do Windows Community
/// Toolkit.
///
/// ```
/// use tomni_ui_core::{Column, ScrollViewer, Split, Text};
/// use tomni_ui_reactive::prelude::*;
///
/// # let scope = Scope::root();
/// # scope.run(|| {
/// let largura = signal(240.0);
/// let tela = Split::row(
///     ScrollViewer::new().child(Column::new().child(Text::new("Arquivos"))),
///     Text::new("Conteúdo"),
/// )
/// .id("divisao")
/// .size(largura)
/// .min_first(160.0)
/// .min_second(240.0);
/// # });
/// # scope.clear();
/// ```
///
/// [`Split::size`] guarda o tamanho do primeiro painel, em px: o arrasto e o teclado mudam o
/// signal, e mudar o signal move o divisor. O segundo painel fica com o resto e, ao
/// redimensionar a janela, absorve a diferença; se ele chegar ao mínimo, o primeiro encolhe
/// até o dele. Os painéis esticam o conteúdo nos dois eixos. Um `Split` dentro de outro monta
/// uma grade.
///
/// O divisor tem 8 px (fundo transparente, `ControlAltFillColorTertiary` no hover e
/// `Quarternary` arrastando) e um polegar de 4 × 24 px com raio 2 no meio; o ponteiro sobre
/// ele vira a seta de redimensionar. É uma parada de Tab: com o foco, as setas do eixo dos
/// painéis movem 8 px. O arrasto anda de 1 em 1 px. Ele fica fora da regra `touch-target` do
/// lint: os divisores do Windows têm de 4 a 8 px, e o teclado faz o mesmo sem arrastar.
///
/// [`Split::ratio`] guarda a fração do primeiro painel no lugar dos px: ela se mantém quando a
/// janela muda de tamanho, e o divisor anda pelo arrasto e pelas setas do mesmo jeito.
///
/// O que não cabe num painel passa por cima do vizinho: ponha um `ScrollViewer` no painel
/// que pode encolher. Os ids das partes são `{id}-primeiro`, `{id}-divisor` e
/// `{id}-segundo`.
pub struct Split {
    common: Common,
    shared: Rc<Shared>,
    first: Option<Box<dyn Widget>>,
    second: Option<Box<dyn Widget>>,
}

impl Split {
    fn new(direction: Direction, first: impl View, second: impl View) -> Self {
        Self::from_boxes(direction, Box::new(first), Box::new(second))
    }

    fn from_boxes(direction: Direction, first: Box<dyn Widget>, second: Box<dyn Widget>) -> Self {
        Self {
            common: Common::default(),
            shared: Rc::new(Shared {
                direction,
                size: signal(DEFAULT_SIZE),
                ratio: None,
                min_first: Cell::new(0.0),
                min_second: Cell::new(0.0),
                length: Cell::new(0.0),
            }),
            first: Some(first),
            second: Some(second),
        }
    }

    /// `first` à esquerda e `second` à direita, com o divisor em pé entre eles.
    pub fn row(first: impl View, second: impl View) -> Self {
        Self::new(Direction::Row, first, second)
    }

    /// `first` em cima e `second` embaixo, com o divisor deitado entre eles.
    pub fn column(first: impl View, second: impl View) -> Self {
        Self::new(Direction::Column, first, second)
    }

    /// Como [`Split::row`] (`row` verdadeiro) ou [`Split::column`], para partes que já estão
    /// numa `Box`: o widget que monta a árvore de divisões em tempo de execução (o
    /// `TerminalPanes`) não tem os tipos concretos.
    pub fn boxed(row: bool, first: Box<dyn Widget>, second: Box<dyn Widget>) -> Self {
        let direction = if row {
            Direction::Row
        } else {
            Direction::Column
        };
        Self::from_boxes(direction, first, second)
    }

    /// Signal com o tamanho do primeiro painel, em px (a largura no [`Split::row`], a altura
    /// no [`Split::column`]). Sem ele, o primeiro painel começa com 240 px.
    pub fn size(mut self, size: Signal<f64>) -> Self {
        self.shared_mut().size = size;
        self
    }

    /// Signal com a fração do primeiro painel no espaço que sobra sem o divisor (de 0 a 1).
    /// Ao contrário do [`Split::size`], a fração se mantém quando o `Split` muda de tamanho.
    /// Quando o app dá os dois, vale a fração.
    pub fn ratio(mut self, ratio: Signal<f64>) -> Self {
        self.shared_mut().ratio = Some(ratio);
        self
    }

    /// Menor tamanho do primeiro painel que o divisor aceita (padrão 0).
    pub fn min_first(self, px: f64) -> Self {
        self.shared.min_first.set(px.max(0.0));
        self
    }

    /// Menor tamanho do segundo painel que o divisor aceita (padrão 0).
    pub fn min_second(self, px: f64) -> Self {
        self.shared.min_second.set(px.max(0.0));
        self
    }

    fn shared_mut(&mut self) -> &mut Shared {
        Rc::get_mut(&mut self.shared).expect("o `Split` ainda não foi montado")
    }

    fn part(&self, name: &str) -> Common {
        Common::generated(self.common.id.as_ref().map(|id| format!("{id}-{name}")))
    }
}

common_builders!(Split);

impl Widget for Split {
    fn kind(&self) -> &'static str {
        "split"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        match self.shared.direction {
            Direction::Row => Look::Row,
            Direction::Column => Look::Column,
        }
    }

    fn flex(&self) -> Option<Flex> {
        Some(Flex {
            direction: self.shared.direction,
            ..Flex::default()
        })
    }

    fn names_children(&self) -> bool {
        true
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        let (Some(first), Some(second)) = (self.first.take(), self.second.take()) else {
            return Vec::new();
        };
        vec![
            Box::new(SplitPane {
                common: self.part("primeiro"),
                shared: Rc::clone(&self.shared),
                first: true,
                child: Some(first),
            }),
            Box::new(Sizer {
                common: self.part("divisor"),
                shared: Rc::clone(&self.shared),
                hovered: false,
                drag: None,
                focus: None,
            }),
            Box::new(SplitPane {
                common: Common {
                    grow: 1.0,
                    ..self.part("segundo")
                },
                shared: Rc::clone(&self.shared),
                first: false,
                child: Some(second),
            }),
        ]
    }

    fn resized(&mut self, size: Size) {
        self.shared.length.set(match self.shared.direction {
            Direction::Row => size.width,
            Direction::Column => size.height,
        });
    }

    fn text(&self) -> Option<String> {
        Some(self.shared.text())
    }
}

impl fmt::Debug for Split {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Split")
            .field("common", &self.common)
            .field("direction", &self.shared.direction)
            .finish_non_exhaustive()
    }
}

/// Um painel do [`Split`]: estica o conteúdo e tem o tamanho que o divisor dá.
struct SplitPane {
    common: Common,
    shared: Rc<Shared>,
    first: bool,
    child: Option<Box<dyn Widget>>,
}

impl Widget for SplitPane {
    fn kind(&self) -> &'static str {
        "split_pane"
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
        Some(match (self.first, self.shared.ratio) {
            (true, None) => (self.shared.size.get(), self.shared.min_first.get()),
            (true, Some(_)) => (0.0, self.shared.min_first.get()),
            (false, _) => (0.0, self.shared.min_second.get()),
        })
    }

    fn dynamic_grow(&self) -> Option<f64> {
        let ratio = self.shared.ratio?.get().clamp(0.0, 1.0);
        Some(if self.first { ratio } else { 1.0 - ratio })
    }
}

impl fmt::Debug for SplitPane {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SplitPane")
            .field("common", &self.common)
            .field("first", &self.first)
            .finish_non_exhaustive()
    }
}

/// Onde o arrasto do divisor começou.
#[derive(Clone, Copy, Debug)]
struct Drag {
    /// Coordenada do ponteiro no eixo dos painéis.
    from: f64,
    /// Posição do divisor ([`Shared::position`]).
    start: f64,
}

/// O divisor do [`Split`].
struct Sizer {
    common: Common,
    shared: Rc<Shared>,
    hovered: bool,
    drag: Option<Drag>,
    focus: Option<FocusSource>,
}

impl Sizer {
    fn along(&self, cx: &EventCx) -> f64 {
        let p = cx.pointer();
        match self.shared.direction {
            Direction::Row => p.x,
            Direction::Column => p.y,
        }
    }
}

impl Widget for Sizer {
    fn kind(&self) -> &'static str {
        "split_sizer"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::Sizer(SizerVisual {
            state: if self.drag.is_some() {
                ButtonState::Pressed
            } else if self.hovered {
                ButtonState::Hover
            } else {
                ButtonState::Rest
            },
            focus_ring: self.focus == Some(FocusSource::Keyboard),
            direction: self.shared.direction,
        })
    }

    fn handle(&mut self, event: &Event, cx: &mut EventCx) -> bool {
        match *event {
            Event::PointerMove { kind, inside } => {
                self.hovered = inside && kind.hovers();
                if let Some(drag) = self.drag {
                    self.shared.move_to(drag.start, self.along(cx) - drag.from);
                }
                true
            }
            Event::PointerDown { kind, .. } => {
                self.hovered = kind.hovers();
                self.drag = Some(Drag {
                    from: self.along(cx),
                    start: self.shared.position(),
                });
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
            Event::FocusIn(source) => {
                self.focus = Some(source);
                true
            }
            Event::FocusOut => {
                self.focus = None;
                true
            }
            Event::KeyDown(key) if self.focus.is_some() && !cx.modifiers().command() => {
                let step = match (self.shared.direction, key) {
                    (Direction::Row, Key::Left) | (Direction::Column, Key::Up) => -SPLIT_KEY_STEP,
                    (Direction::Row, Key::Right) | (Direction::Column, Key::Down) => SPLIT_KEY_STEP,
                    _ => return false,
                };
                self.shared.step(step.signum());
                true
            }
            _ => false,
        }
    }

    fn interactive(&self) -> bool {
        true
    }

    fn focusable(&self) -> bool {
        true
    }

    fn cursor(&self) -> Option<Cursor> {
        Some(match self.shared.direction {
            Direction::Row => Cursor::EwResize,
            Direction::Column => Cursor::NsResize,
        })
    }

    fn text(&self) -> Option<String> {
        Some(self.shared.text())
    }
}

impl fmt::Debug for Sizer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Sizer")
            .field("common", &self.common)
            .field("dragging", &self.drag.is_some())
            .finish_non_exhaustive()
    }
}
