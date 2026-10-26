//! `ScrollViewer`: área rolável com a barra de rolagem do WinUI.

use std::cell::Cell;
use std::fmt;
use std::rc::Rc;

use kurbo::{Point, Rect, Size, Vec2};
use tomni_ui_reactive::{Get, GetUntracked, Set, Signal, signal};

use crate::cx::EventCx;
use crate::event::Event;
use crate::widget::{
    Common, Direction, Flex, Look, View, Widget, common_builders, density_builder,
};

/// Largura da barra vertical (e altura da horizontal), `ScrollBarSize` do WinUI.
pub const SCROLL_BAR_SIZE: f64 = 12.0;
/// Comprimento mínimo do polegar (`ScrollBarVerticalThumbMinHeight`).
pub const SCROLL_THUMB_MIN: f64 = 30.0;
/// Quanto as setas da barra e as setas do teclado rolam, em pixels.
pub const SCROLL_LINE: f64 = 16.0;

/// Em que eixos um [`ScrollViewer`] rola.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ScrollMode {
    /// De cima para baixo (padrão).
    #[default]
    Vertical,
    /// Da esquerda para a direita.
    Horizontal,
    /// Nos dois eixos.
    Both,
}

impl ScrollMode {
    /// Se rola na horizontal.
    pub fn x(self) -> bool {
        self != Self::Vertical
    }

    /// Se rola na vertical.
    pub fn y(self) -> bool {
        self != Self::Horizontal
    }
}

/// Eixo de uma barra de rolagem.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScrollAxis {
    /// Barra horizontal, embaixo.
    X,
    /// Barra vertical, à direita.
    Y,
}

/// Rolagem de um [`ScrollViewer`]: deslocamento, tamanho do conteúdo e da área visível.
///
/// O deslocamento fica sempre entre zero e [`Scroller::max_offset`], em pixels inteiros (o
/// texto continua alinhado à grade de pixels).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Scroller {
    mode: ScrollMode,
    offset: Vec2,
    content: Size,
    viewport: Size,
}

impl Scroller {
    /// Rolagem nos eixos de `mode`, no começo e sem conteúdo.
    pub fn new(mode: ScrollMode) -> Self {
        Self {
            mode,
            ..Self::default()
        }
    }

    /// Eixos que rolam.
    pub fn mode(&self) -> ScrollMode {
        self.mode
    }

    /// Quanto o conteúdo está rolado: `y` positivo mostra o que está mais abaixo.
    pub fn offset(&self) -> Vec2 {
        self.offset
    }

    /// Tamanho do conteúdo, nunca menor que a área visível.
    pub fn content(&self) -> Size {
        self.content
    }

    /// Tamanho da área visível (o do `ScrollViewer`).
    pub fn viewport(&self) -> Size {
        self.viewport
    }

    /// Maior deslocamento possível em cada eixo; zero no eixo que não rola.
    pub fn max_offset(&self) -> Vec2 {
        let max = |rolls: bool, content: f64, viewport: f64| {
            if rolls {
                (content - viewport).max(0.0)
            } else {
                0.0
            }
        };
        Vec2::new(
            max(self.mode.x(), self.content.width, self.viewport.width),
            max(self.mode.y(), self.content.height, self.viewport.height),
        )
    }

    /// Se o conteúdo passa da área visível no eixo `axis` (a barra aparece).
    pub fn overflows(&self, axis: ScrollAxis) -> bool {
        let max = self.max_offset();
        match axis {
            ScrollAxis::X => max.x > 0.0,
            ScrollAxis::Y => max.y > 0.0,
        }
    }

    /// Se dá para rolar em algum eixo.
    pub fn can_scroll(&self) -> bool {
        self.overflows(ScrollAxis::X) || self.overflows(ScrollAxis::Y)
    }

    /// Tamanhos medidos pelo layout. O deslocamento é ajustado se passar do novo máximo.
    /// Devolve `true` se algo mudou.
    pub fn set_extent(&mut self, content: Size, viewport: Size) -> bool {
        let old = *self;
        self.content = content;
        self.viewport = viewport;
        self.scroll_to(self.offset);
        *self != old
    }

    /// Rola até `offset`, limitado ao conteúdo e arredondado. Devolve `true` se mudou.
    pub fn scroll_to(&mut self, offset: Vec2) -> bool {
        let max = self.max_offset();
        let offset = Vec2::new(
            offset.x.clamp(0.0, max.x).round(),
            offset.y.clamp(0.0, max.y).round(),
        );
        let changed = offset != self.offset;
        self.offset = offset;
        changed
    }

    /// Rola `delta` a partir de onde está. Devolve `true` se mudou.
    pub fn scroll_by(&mut self, delta: Vec2) -> bool {
        self.scroll_to(self.offset + delta)
    }

    /// Partes da barra do eixo `axis`, para um `ScrollViewer` em `rect`; `None` se o conteúdo
    /// cabe nesse eixo.
    ///
    /// A barra fica por cima do conteúdo, como no WinUI: a vertical na borda direita e a
    /// horizontal na de baixo. Com as duas, a vertical para antes do canto.
    pub fn bar(&self, rect: Rect, axis: ScrollAxis) -> Option<ScrollBarParts> {
        if !self.overflows(axis) {
            return None;
        }
        let size = SCROLL_BAR_SIZE;
        let max = self.max_offset();
        let (bar, ratio, progress) = match axis {
            ScrollAxis::Y => {
                let bottom = if self.overflows(ScrollAxis::X) {
                    rect.y1 - size
                } else {
                    rect.y1
                };
                let bar = Rect::new(rect.x1 - size, rect.y0, rect.x1, bottom);
                let ratio = self.viewport.height / self.content.height;
                (bar, ratio, self.offset.y / max.y)
            }
            ScrollAxis::X => {
                let right = if self.overflows(ScrollAxis::Y) {
                    rect.x1 - size
                } else {
                    rect.x1
                };
                let bar = Rect::new(rect.x0, rect.y1 - size, right, rect.y1);
                let ratio = self.viewport.width / self.content.width;
                (bar, ratio, self.offset.x / max.x)
            }
        };
        // As setas ocupam um quadrado em cada ponta; o polegar corre entre elas.
        let (start, end) = along(bar, axis);
        let track = ((end - start) - 2.0 * size).max(0.0);
        let thumb = (track * ratio).max(SCROLL_THUMB_MIN).min(track).round();
        let thumb_start = (start + size + (track - thumb) * progress).round();
        let span = |a: f64, b: f64| match axis {
            ScrollAxis::Y => Rect::new(bar.x0, a, bar.x1, b),
            ScrollAxis::X => Rect::new(a, bar.y0, b, bar.y1),
        };
        Some(ScrollBarParts {
            axis,
            bar,
            decrease: span(start, start + size),
            increase: span(end - size, end),
            thumb: span(thumb_start, thumb_start + thumb),
            travel: track - thumb,
        })
    }
}

/// Começo e fim de `rect` no eixo de `axis`.
fn along(rect: Rect, axis: ScrollAxis) -> (f64, f64) {
    match axis {
        ScrollAxis::X => (rect.x0, rect.x1),
        ScrollAxis::Y => (rect.y0, rect.y1),
    }
}

/// Coordenada de `point` no eixo de `axis`.
fn coord(point: Point, axis: ScrollAxis) -> f64 {
    match axis {
        ScrollAxis::X => point.x,
        ScrollAxis::Y => point.y,
    }
}

/// Vetor de comprimento `value` no eixo de `axis`.
fn vec_on(axis: ScrollAxis, value: f64) -> Vec2 {
    match axis {
        ScrollAxis::X => Vec2::new(value, 0.0),
        ScrollAxis::Y => Vec2::new(0.0, value),
    }
}

/// Partes de uma barra de rolagem, na mesma coordenada do retângulo do `ScrollViewer`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollBarParts {
    /// Eixo da barra.
    pub axis: ScrollAxis,
    /// A faixa inteira, que vira o trilho quando a barra expande.
    pub bar: Rect,
    /// Seta do começo (de cima ou da esquerda).
    pub decrease: Rect,
    /// Seta do fim.
    pub increase: Rect,
    /// Caixa do polegar, com a largura da faixa. O `Drawer` pinta só o miolo.
    pub thumb: Rect,
    /// Quanto o polegar anda do começo ao fim do trilho.
    pub travel: f64,
}

/// Estado da barra de rolagem.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ScrollBarState {
    /// Fina, só o polegar, como o indicador do WinUI quando o mouse não está na barra.
    #[default]
    Rest,
    /// Mouse sobre a barra: expandida, com trilho e setas.
    Hover,
    /// Pressionada (arrastando o polegar ou segurando uma seta): expandida.
    Pressed,
}

impl ScrollBarState {
    /// Se a barra aparece expandida, com trilho e setas.
    pub fn expanded(self) -> bool {
        self != Self::Rest
    }
}

/// Tudo o que o `Drawer` precisa para pintar as barras de um `ScrollViewer`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrollViewerVisual {
    /// Rolagem atual.
    pub scroller: Scroller,
    /// Estado das barras.
    pub state: ScrollBarState,
    /// Sem barras: a rolagem é comandada por fora ([`ScrollViewer::link`]).
    pub bars_hidden: bool,
}

/// Rolagem horizontal de um [`ScrollViewer`] comandada por fora, como as setas da faixa de
/// abas do `TabView`: quem tem a ligação pede rolagens ([`ScrollLink::scroll_by`]) e lê, em
/// signals, se o conteúdo transborda e se a rolagem está numa das pontas.
///
/// O `ScrollViewer` ligado não mostra barras. A árvore atende os pedidos depois de cada evento
/// e de cada `tick` e publica o estado sempre que a rolagem muda.
pub struct ScrollLink {
    overflow: Signal<bool>,
    start: Signal<bool>,
    end: Signal<bool>,
    slack: Cell<f64>,
    request: Cell<f64>,
    request_y: Cell<f64>,
    offset_y: Signal<f64>,
    viewport_height: Signal<f64>,
    viewport_width: Signal<f64>,
}

impl ScrollLink {
    /// Ligação nova. `slack` é quanto a área visível perde quando o conteúdo transborda (a
    /// largura dos controles que aparecem): o conteúdo só volta a caber quando cabe sem eles,
    /// e mostrar e esconder os controles não fica alternando.
    pub fn new(slack: f64) -> Rc<Self> {
        Rc::new(Self {
            overflow: signal(false),
            start: signal(true),
            end: signal(true),
            slack: Cell::new(slack),
            request: Cell::new(0.0),
            request_y: Cell::new(0.0),
            offset_y: signal(0.0),
            viewport_height: signal(0.0),
            viewport_width: signal(0.0),
        })
    }

    /// Troca o `slack` (ele pode depender da densidade, que só se sabe na montagem).
    pub fn set_slack(&self, slack: f64) {
        self.slack.set(slack);
    }

    /// Se o conteúdo passa da área visível (rastreado).
    pub fn overflows(&self) -> bool {
        self.overflow.get()
    }

    /// Se a rolagem está no começo (rastreado).
    pub fn at_start(&self) -> bool {
        self.start.get()
    }

    /// Se a rolagem está no fim (rastreado).
    pub fn at_end(&self) -> bool {
        self.end.get()
    }

    /// Pede para rolar `dx` pixels; os pedidos se somam até a árvore atender.
    pub fn scroll_by(&self, dx: f64) {
        self.request.set(self.request.get() + dx);
    }

    /// Pede para rolar `dy` pixels na vertical; os pedidos se somam até a árvore atender (a
    /// rolagem automática do arrasto no `TreeView`).
    pub fn scroll_by_y(&self, dy: f64) {
        self.request_y.set(self.request_y.get() + dy);
    }

    /// Deslocamento vertical da rolagem (rastreado). A lista virtualizada do `TreeView` lê
    /// para saber quais linhas montar.
    pub fn offset_y(&self) -> f64 {
        self.offset_y.get()
    }

    /// Altura da área visível (rastreado).
    pub fn viewport_height(&self) -> f64 {
        self.viewport_height.get()
    }

    /// Largura da área visível (rastreado): o `DataGrid` divide as colunas por ela.
    pub fn viewport_width(&self) -> f64 {
        self.viewport_width.get()
    }

    /// Os pedidos pendentes, `(dx, dy)`, zerando-os.
    pub(crate) fn take_request(&self) -> (f64, f64) {
        (self.request.replace(0.0), self.request_y.replace(0.0))
    }

    /// Publica o estado de `scroller` nos signals que mudaram.
    pub(crate) fn publish(&self, scroller: &Scroller) {
        let content = scroller.content().width;
        let viewport = scroller.viewport().width;
        let overflow = if self.overflow.get_untracked() {
            content > viewport + self.slack.get()
        } else {
            content > viewport
        };
        let max = scroller.max_offset().x;
        let offset = scroller.offset().x;
        for (signal, value) in [
            (self.overflow, overflow),
            (self.start, offset <= 0.0),
            (self.end, offset >= max),
        ] {
            if signal.get_untracked() != value {
                signal.set(value);
            }
        }
        for (signal, value) in [
            (self.offset_y, scroller.offset().y),
            (self.viewport_height, scroller.viewport().height),
            (self.viewport_width, scroller.viewport().width),
        ] {
            if signal.get_untracked() != value {
                signal.set(value);
            }
        }
    }
}

impl fmt::Debug for ScrollLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScrollLink")
            .field("overflow", &self.overflow.get_untracked())
            .field("start", &self.start.get_untracked())
            .field("end", &self.end.get_untracked())
            .field("slack", &self.slack.get())
            .finish_non_exhaustive()
    }
}

/// Arrasto do polegar em andamento.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Drag {
    axis: ScrollAxis,
    /// Onde o ponteiro estava ao pressionar, no eixo da barra.
    from: f64,
    /// Deslocamento ao pressionar.
    start: Vec2,
}

/// Área rolável com um filho, como o `ScrollViewer` do WinUI.
///
/// ```
/// use tomni_ui_core::{Button, Column, ScrollViewer};
///
/// let lista = ScrollViewer::new().id("lista").grow(1).child(
///     Column::new()
///         .gap(8)
///         .padding(16)
///         .child(Button::new("Um"))
///         .child(Button::new("Dois")),
/// );
/// ```
///
/// O filho tem a largura do `ScrollViewer` (na rolagem vertical) e a altura que precisar; o
/// que passar da área visível rola com a roda do mouse, arrastando com o dedo, pela barra, com
/// PageUp/PageDown, Home/End e as setas, e quando o Tab leva o foco a um widget escondido.
///
/// Numa `Column`, o `ScrollViewer` ocupa a altura do conteúdo e encolhe até caber; com
/// `.grow(1)`, fica com todo o espaço que sobra. A barra fica por cima do conteúdo, na borda
/// direita: deixe uns 12 px de `padding` à direita se o conteúdo for até a borda.
pub struct ScrollViewer {
    common: Common,
    child: Option<Box<dyn Widget>>,
    scroller: Scroller,
    hovered: bool,
    pressed: bool,
    drag: Option<Drag>,
    link: Option<Rc<ScrollLink>>,
    /// Com a ligação, ainda mostra as barras (o `TreeView`).
    bars: bool,
}

impl ScrollViewer {
    /// Marca como parte montada por um grupo ([`Common::is_generated`]).
    pub(crate) fn generated(mut self) -> Self {
        self.common.generated = true;
        self
    }

    /// Rolagem vertical.
    pub fn new() -> Self {
        Self::with_mode(ScrollMode::Vertical)
    }

    /// Rolagem horizontal: o filho tem a altura do `ScrollViewer` e a largura que precisar.
    pub fn horizontal() -> Self {
        Self::with_mode(ScrollMode::Horizontal)
    }

    /// Rolagem nos dois eixos.
    pub fn both() -> Self {
        Self::with_mode(ScrollMode::Both)
    }

    fn with_mode(mode: ScrollMode) -> Self {
        Self {
            common: Common::default(),
            child: None,
            scroller: Scroller::new(mode),
            hovered: false,
            pressed: false,
            drag: None,
            link: None,
            bars: false,
        }
    }

    /// Liga a rolagem a `link`, que passa a comandá-la: as barras somem (a roda do mouse e o
    /// foco pelo teclado continuam rolando).
    pub fn link(mut self, link: Rc<ScrollLink>) -> Self {
        self.link = Some(link);
        self
    }

    /// Como [`ScrollViewer::link`], mas as barras continuam: a ligação só lê a posição e pede
    /// rolagens (a lista do `TreeView`, a conversa do chat de um app que a segue até o fim).
    pub fn link_with_bars(mut self, link: Rc<ScrollLink>) -> Self {
        self.link = Some(link);
        self.bars = true;
        self
    }

    /// Se as barras estão escondidas: a rolagem é comandada por fora.
    fn bars_hidden(&self) -> bool {
        self.link.is_some() && !self.bars
    }

    /// Conteúdo. Chamar de novo troca o anterior.
    pub fn child(mut self, child: impl View) -> Self {
        self.child = Some(Box::new(child));
        self
    }

    /// Estado para o `Drawer`.
    pub fn visual(&self) -> ScrollViewerVisual {
        let state = if self.pressed {
            ScrollBarState::Pressed
        } else if self.hovered {
            ScrollBarState::Hover
        } else {
            ScrollBarState::Rest
        };
        ScrollViewerVisual {
            scroller: self.scroller,
            state,
            bars_hidden: self.bars_hidden(),
        }
    }

    /// Barra sob `point`, para um `ScrollViewer` em `rect`.
    fn bar_at(&self, rect: Rect, point: Point) -> Option<ScrollBarParts> {
        if self.bars_hidden() {
            return None;
        }
        [ScrollAxis::Y, ScrollAxis::X]
            .into_iter()
            .filter_map(|axis| self.scroller.bar(rect, axis))
            .find(|parts| parts.bar.contains(point))
    }

    fn press(&mut self, rect: Rect, point: Point) -> bool {
        let Some(parts) = self.bar_at(rect, point) else {
            return false;
        };
        self.drag = press_bar(&mut self.scroller, parts, point);
        self.pressed = true;
        true
    }

    fn drag_to(&mut self, rect: Rect, point: Point) {
        if let Some(drag) = self.drag {
            drag_bar(&mut self.scroller, drag, rect, point);
        }
    }
}

/// Pressionou a barra `parts` em `point`: rola uma linha (setas) ou uma página (trilho) e,
/// sobre o polegar, devolve o arrasto que começou.
pub(crate) fn press_bar(
    scroller: &mut Scroller,
    parts: ScrollBarParts,
    point: Point,
) -> Option<Drag> {
    let axis = parts.axis;
    let page = match axis {
        ScrollAxis::X => scroller.viewport().width,
        ScrollAxis::Y => scroller.viewport().height,
    };
    let at = coord(point, axis);
    if parts.decrease.contains(point) {
        scroller.scroll_by(vec_on(axis, -SCROLL_LINE));
    } else if parts.increase.contains(point) {
        scroller.scroll_by(vec_on(axis, SCROLL_LINE));
    } else if parts.thumb.contains(point) {
        return Some(Drag {
            axis,
            from: at,
            start: scroller.offset(),
        });
    } else if at < along(parts.thumb, axis).0 {
        scroller.scroll_by(vec_on(axis, -page));
    } else {
        scroller.scroll_by(vec_on(axis, page));
    }
    None
}

/// O ponteiro foi a `point` com o polegar preso: o polegar o segue.
pub(crate) fn drag_bar(scroller: &mut Scroller, drag: Drag, rect: Rect, point: Point) {
    let Some(parts) = scroller.bar(rect, drag.axis) else {
        return;
    };
    if parts.travel <= 0.0 {
        return;
    }
    // O polegar segue o ponteiro: cada pixel do trilho vale `max / travel` do conteúdo.
    let max = scroller.max_offset();
    let max = match drag.axis {
        ScrollAxis::X => max.x,
        ScrollAxis::Y => max.y,
    };
    let moved = (coord(point, drag.axis) - drag.from) * max / parts.travel;
    scroller.scroll_to(drag.start + vec_on(drag.axis, moved));
}

impl Default for ScrollViewer {
    fn default() -> Self {
        Self::new()
    }
}

common_builders!(ScrollViewer);
density_builder!(ScrollViewer);

impl Widget for ScrollViewer {
    fn kind(&self) -> &'static str {
        "scroll_viewer"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::ScrollViewer(self.visual())
    }

    fn flex(&self) -> Option<Flex> {
        let direction = match self.scroller.mode() {
            ScrollMode::Horizontal => Direction::Row,
            ScrollMode::Vertical | ScrollMode::Both => Direction::Column,
        };
        Some(Flex {
            direction,
            fill: true,
            ..Flex::default()
        })
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        self.child.take().into_iter().collect()
    }

    fn handle(&mut self, event: &Event, cx: &mut EventCx) -> bool {
        let (rect, point) = (cx.rect(), cx.pointer());
        match *event {
            Event::PointerMove { kind, inside } => {
                self.hovered = inside && kind.hovers();
                self.drag_to(rect, point);
                true
            }
            Event::PointerDown { kind, .. } => {
                self.hovered = kind.hovers();
                self.press(rect, point)
            }
            Event::PointerUp { kind, inside } => {
                self.hovered = inside && kind.hovers();
                self.pressed = false;
                self.drag = None;
                true
            }
            Event::PointerCancel => {
                self.hovered = false;
                self.pressed = false;
                self.drag = None;
                true
            }
            _ => false,
        }
    }

    fn interactive(&self) -> bool {
        true
    }

    fn hit_rect(&self, rect: Rect) -> Rect {
        if self.bars_hidden() {
            return Rect::ZERO;
        }
        [ScrollAxis::Y, ScrollAxis::X]
            .into_iter()
            .find_map(|axis| self.scroller.bar(rect, axis))
            .map_or(Rect::ZERO, |parts| parts.bar)
    }

    fn hits(&self, rect: Rect, point: Point) -> bool {
        self.bar_at(rect, point).is_some()
    }

    fn scroller(&self) -> Option<&Scroller> {
        Some(&self.scroller)
    }

    fn scroller_mut(&mut self) -> Option<&mut Scroller> {
        Some(&mut self.scroller)
    }

    fn scroll_link(&self) -> Option<&Rc<ScrollLink>> {
        self.link.as_ref()
    }
}

impl fmt::Debug for ScrollViewer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScrollViewer")
            .field("common", &self.common)
            .field("visual", &self.visual())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cx::{FixedMetrics, MemoryClipboard};
    use crate::event::{Modifiers, PointerKind::Mouse};

    fn scroller(content: f64) -> Scroller {
        let mut s = Scroller::new(ScrollMode::Vertical);
        s.set_extent(Size::new(200.0, content), Size::new(200.0, 100.0));
        s
    }

    #[test]
    fn limita_e_arredonda() {
        let mut s = scroller(400.0);
        assert_eq!(s.max_offset(), Vec2::new(0.0, 300.0));
        assert!(s.scroll_by(Vec2::new(50.0, 10.4)));
        assert_eq!(
            s.offset(),
            Vec2::new(0.0, 10.0),
            "x não rola e y é arredondado"
        );
        s.scroll_to(Vec2::new(0.0, 1000.0));
        assert_eq!(s.offset().y, 300.0);
        assert!(!s.scroll_by(Vec2::new(0.0, 5.0)), "já está no fim");
        assert!(s.set_extent(Size::new(200.0, 250.0), Size::new(200.0, 100.0)));
        assert_eq!(s.offset().y, 150.0, "o conteúdo encolheu");
        assert!(
            scroller(100.0)
                .bar(Rect::new(0.0, 0.0, 200.0, 100.0), ScrollAxis::Y)
                .is_none()
        );
    }

    #[test]
    fn geometria_da_barra() {
        let mut s = scroller(400.0);
        let rect = Rect::new(0.0, 0.0, 200.0, 100.0);
        let parts = s.bar(rect, ScrollAxis::Y).unwrap();
        assert_eq!(parts.bar, Rect::new(188.0, 0.0, 200.0, 100.0));
        assert_eq!(parts.decrease, Rect::new(188.0, 0.0, 200.0, 12.0));
        assert_eq!(parts.increase, Rect::new(188.0, 88.0, 200.0, 100.0));
        // Trilho de 76 px, um quarto do conteúdo à vista: 19 px, mas o mínimo é 30.
        assert_eq!(parts.thumb, Rect::new(188.0, 12.0, 200.0, 42.0));
        assert_eq!(parts.travel, 46.0);
        s.scroll_to(Vec2::new(0.0, 300.0));
        let parts = s.bar(rect, ScrollAxis::Y).unwrap();
        assert_eq!(
            parts.thumb.y1, 88.0,
            "no fim, o polegar encosta na seta de baixo"
        );
    }

    #[test]
    fn setas_trilho_e_arrasto() {
        let mut sv = ScrollViewer::new();
        sv.scroller = scroller(400.0);
        let rect = Rect::new(0.0, 0.0, 200.0, 100.0);
        let mut metrics = FixedMetrics::default();
        let mut clipboard = MemoryClipboard::default();
        let mut send = |sv: &mut ScrollViewer, event: Event, point: Point| {
            let mut cx = EventCx::new(rect, point, Modifiers::NONE, &mut metrics, &mut clipboard);
            sv.handle(&event, &mut cx)
        };
        let down = Event::PointerDown {
            kind: Mouse,
            clicks: 1,
        };
        let up = Event::PointerUp {
            kind: Mouse,
            inside: true,
        };
        let at = |y| Point::new(194.0, y);

        send(&mut sv, down.clone(), at(95.0));
        assert_eq!(sv.scroller.offset().y, 16.0, "seta de baixo");
        assert_eq!(sv.visual().state, ScrollBarState::Pressed);
        send(&mut sv, up.clone(), at(95.0));
        assert_eq!(sv.visual().state, ScrollBarState::Hover);

        send(&mut sv, down.clone(), at(80.0));
        assert_eq!(
            sv.scroller.offset().y,
            116.0,
            "trilho depois do polegar: uma página"
        );
        send(&mut sv, up.clone(), at(80.0));

        // O polegar está em 12 + 46 × 116/300 ≈ 30; arrastar 23 px rola metade do máximo.
        let thumb = sv.scroller.bar(rect, ScrollAxis::Y).unwrap().thumb;
        send(&mut sv, down, at(thumb.center().y));
        send(
            &mut sv,
            Event::PointerMove {
                kind: Mouse,
                inside: false,
            },
            at(thumb.center().y + 23.0),
        );
        assert_eq!(sv.scroller.offset().y, 266.0);
        assert_eq!(
            sv.visual().state,
            ScrollBarState::Pressed,
            "segue fora da barra"
        );
        send(&mut sv, up, at(0.0));
        assert!(
            !sv.hits(rect, Point::new(100.0, 50.0)),
            "o conteúdo não é da barra"
        );
        assert!(sv.hits(rect, at(50.0)));
    }

    #[test]
    fn a_ligacao_com_barras_mantem_as_barras() {
        let sem = ScrollViewer::new().link(ScrollLink::new(0.0));
        assert!(sem.visual().bars_hidden, "`link` esconde as barras");
        let com = ScrollViewer::new().link_with_bars(ScrollLink::new(0.0));
        assert!(!com.visual().bars_hidden, "`link_with_bars` as mantém");
        assert!(!ScrollViewer::new().visual().bars_hidden);
    }
}
