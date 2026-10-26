//! As peças que o `DockHost` monta: o contêiner genérico, a lista de painéis, o divisor, o
//! véu, o relógio do auto-ocultar e os botões.

use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use tomni_ui_reactive::{Get, GetUntracked, untrack};

use super::shared::Shared;
use super::{DOCK_HOVER_DELAY, DOCK_LEAVE_DELAY};
use crate::button::ButtonState;
use crate::cx::EventCx;
use crate::density::Density;
use crate::event::{Cursor, Event, FocusSource, Key};
use crate::kurbo::Size;
use crate::navigation_view::NavigationItemVisual;
use crate::number_box::format_number;
use crate::press::Pressable;
use crate::prop::Prop;
use crate::split::{SPLIT_KEY_STEP, SPLIT_SIZER, SizerVisual};
use crate::symbol::Symbol;
use crate::tab_view::{TabStripButtonKind, TabStripButtonVisual};
use crate::widget::{Common, Direction, Flex, Look, Slot, Widget, Within};

type Dyn<T> = Rc<dyn Fn() -> T>;
type EventHook = Box<dyn FnMut(&Event, &mut EventCx) -> bool>;
type Maker = Rc<dyn Fn(&str) -> Box<dyn Widget>>;

/// Contêiner genérico: o que muda com o estado vem de closures lidas dentro do efeito da
/// árvore.
pub(crate) struct Part {
    pub common: Common,
    kind: &'static str,
    pub flex: Flex,
    children: Vec<Box<dyn Widget>>,
    look: Option<Dyn<Look>>,
    width: Option<Dyn<f64>>,
    basis: Option<Dyn<(f64, f64)>>,
    grow: Option<Dyn<f64>>,
    raised: bool,
    interactive: bool,
    on_resize: Option<Box<dyn FnMut(Size)>>,
    on_event: Option<EventHook>,
    on_within: Option<Box<dyn FnMut(Within)>>,
}

impl Part {
    pub fn new(direction: Direction, kind: &'static str) -> Self {
        Self {
            common: Common::default(),
            kind,
            flex: Flex {
                direction,
                ..Flex::default()
            },
            children: Vec::new(),
            look: None,
            width: None,
            basis: None,
            grow: None,
            raised: false,
            interactive: false,
            on_resize: None,
            on_event: None,
            on_within: None,
        }
    }

    pub fn row(kind: &'static str) -> Self {
        Self::new(Direction::Row, kind)
    }

    pub fn column(kind: &'static str) -> Self {
        Self::new(Direction::Column, kind)
    }

    /// Id gerado (derivado do id do `DockHost`).
    pub fn id(mut self, id: String) -> Self {
        self.common = Common::generated(Some(id));
        self
    }

    pub fn grow(mut self, grow: f64) -> Self {
        self.common.grow = grow;
        self
    }

    pub fn fill(mut self) -> Self {
        self.flex.fill = true;
        self
    }

    pub fn height(mut self, height: f64) -> Self {
        self.common.height = Some(height);
        self
    }

    pub fn width(mut self, width: f64) -> Self {
        self.common.width = Some(width);
        self
    }

    pub fn raised(mut self) -> Self {
        self.raised = true;
        self
    }

    pub fn visible(mut self, visible: impl Fn() -> bool + 'static) -> Self {
        self.common.visible = Some(Prop::Dynamic(Rc::new(visible)));
        self
    }

    pub fn look(mut self, look: impl Fn() -> Look + 'static) -> Self {
        self.look = Some(Rc::new(look));
        self
    }

    pub fn dynamic_width(mut self, width: impl Fn() -> f64 + 'static) -> Self {
        self.width = Some(Rc::new(width));
        self
    }

    pub fn dynamic_basis(mut self, basis: impl Fn() -> (f64, f64) + 'static) -> Self {
        self.basis = Some(Rc::new(basis));
        self
    }

    pub fn dynamic_grow(mut self, grow: impl Fn() -> f64 + 'static) -> Self {
        self.grow = Some(Rc::new(grow));
        self
    }

    pub fn interactive(mut self) -> Self {
        self.interactive = true;
        self
    }

    pub fn on_resize(mut self, f: impl FnMut(Size) + 'static) -> Self {
        self.on_resize = Some(Box::new(f));
        self
    }

    pub fn on_event(mut self, f: impl FnMut(&Event, &mut EventCx) -> bool + 'static) -> Self {
        self.on_event = Some(Box::new(f));
        self
    }

    pub fn on_within(mut self, f: impl FnMut(Within) + 'static) -> Self {
        self.on_within = Some(Box::new(f));
        self
    }

    pub fn child(mut self, child: impl Widget) -> Self {
        self.children.push(Box::new(child));
        self
    }

    pub fn boxed(mut self, child: Box<dyn Widget>) -> Self {
        self.children.push(child);
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
        match &self.look {
            Some(look) => look(),
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

    fn handle(&mut self, event: &Event, cx: &mut EventCx) -> bool {
        match &mut self.on_event {
            Some(f) => f(event, cx),
            None => false,
        }
    }

    fn interactive(&self) -> bool {
        self.interactive
    }

    fn dynamic_width(&self) -> Option<f64> {
        self.width.as_ref().map(|f| f())
    }

    fn dynamic_basis(&self) -> Option<(f64, f64)> {
        self.basis.as_ref().map(|f| f())
    }

    fn dynamic_grow(&self) -> Option<f64> {
        self.grow.as_ref().map(|f| f())
    }

    fn raised(&self) -> bool {
        self.raised
    }

    fn resized(&mut self, size: Size) {
        if let Some(f) = &mut self.on_resize {
            f(size);
        }
    }

    fn watches_within(&self) -> bool {
        self.on_within.is_some()
    }

    fn within(&mut self, change: Within) {
        if let Some(f) = &mut self.on_within {
            f(change);
        }
    }
}

/// Um fragmento que monta um widget por chave: as chaves vêm de `items`, e `make` monta o
/// widget de uma chave nova. Uma chave que continua na lista mantém o widget (e o estado).
pub(crate) struct List {
    common: Common,
    items: Box<dyn Fn() -> Vec<String>>,
    make: Maker,
    keys: Option<Vec<String>>,
}

impl List {
    pub fn new(
        items: impl Fn() -> Vec<String> + 'static,
        make: impl Fn(&str) -> Box<dyn Widget> + 'static,
    ) -> Self {
        Self {
            common: Common::default(),
            items: Box::new(items),
            make: Rc::new(make),
            keys: None,
        }
    }
}

impl Widget for List {
    fn kind(&self) -> &'static str {
        "dock_list"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::Fragment
    }

    fn fragment(&self) -> bool {
        true
    }

    fn track_children(&self) {
        let _ = (self.items)();
    }

    fn reconcile(&mut self) -> Option<Vec<Slot>> {
        let keys = (self.items)();
        if self.keys.as_ref() == Some(&keys) {
            return None;
        }
        let mut old: HashMap<&String, usize> = self
            .keys
            .iter()
            .flatten()
            .enumerate()
            .map(|(i, key)| (key, i))
            .collect();
        let slots = keys
            .iter()
            .map(|key| match old.remove(key) {
                Some(i) => Slot::Keep(i),
                None => {
                    let make = Rc::clone(&self.make);
                    let key = key.clone();
                    Slot::New(Box::new(move || make(&key)))
                }
            })
            .collect();
        self.keys = Some(keys);
        Some(slots)
    }
}

/// O divisor de uma área fixada ou da borda de um painel aberto por cima: lê e grava o
/// tamanho por closures. `sign` é +1 quando arrastar para a direita (ou para baixo) aumenta o
/// que ele controla, e −1 quando diminui.
pub(crate) struct DockSizer {
    common: Common,
    direction: Direction,
    sign: f64,
    get: Rc<dyn Fn() -> f64>,
    set: Rc<dyn Fn(f64)>,
    hovered: bool,
    drag: Option<(f64, f64)>,
    focus: Option<FocusSource>,
}

impl DockSizer {
    pub fn new(
        id: String,
        direction: Direction,
        sign: f64,
        get: impl Fn() -> f64 + 'static,
        set: impl Fn(f64) + 'static,
    ) -> Self {
        let mut common = Common::generated(Some(id));
        match direction {
            Direction::Row => common.width = Some(SPLIT_SIZER),
            Direction::Column => common.height = Some(SPLIT_SIZER),
        }
        Self {
            common,
            direction,
            sign,
            get: Rc::new(get),
            set: Rc::new(set),
            hovered: false,
            drag: None,
            focus: None,
        }
    }

    pub fn visible(mut self, visible: impl Fn() -> bool + 'static) -> Self {
        self.common.visible = Some(Prop::Dynamic(Rc::new(visible)));
        self
    }

    fn along(&self, cx: &EventCx) -> f64 {
        let p = cx.pointer();
        match self.direction {
            Direction::Row => p.x,
            Direction::Column => p.y,
        }
    }
}

impl Widget for DockSizer {
    /// Como o divisor do `Split`: o lint não pede o alvo de toque (o teclado faz o mesmo).
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
            direction: self.direction,
        })
    }

    fn handle(&mut self, event: &Event, cx: &mut EventCx) -> bool {
        match *event {
            Event::PointerMove { kind, inside } => {
                self.hovered = inside && kind.hovers();
                if let Some((from, start)) = self.drag {
                    (self.set)(start + self.sign * (self.along(cx) - from));
                }
                true
            }
            Event::PointerDown { kind, .. } => {
                self.hovered = kind.hovers();
                self.drag = Some((self.along(cx), untrack(|| (self.get)())));
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
                let step = match (self.direction, key) {
                    (Direction::Row, Key::Left) | (Direction::Column, Key::Up) => -1.0,
                    (Direction::Row, Key::Right) | (Direction::Column, Key::Down) => 1.0,
                    _ => return false,
                };
                let now = untrack(|| (self.get)());
                (self.set)(now + self.sign * step * SPLIT_KEY_STEP);
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
        Some(match self.direction {
            Direction::Row => Cursor::EwResize,
            Direction::Column => Cursor::NsResize,
        })
    }

    fn text(&self) -> Option<String> {
        Some(format_number((self.get)(), 0))
    }
}

/// Área transparente ao lado do painel aberto por cima: um clique nela recolhe o painel.
pub(crate) struct Veil {
    common: Common,
    shared: Rc<Shared>,
}

impl Veil {
    pub fn new(shared: &Rc<Shared>) -> Self {
        Self {
            common: Common::generated(Some(shared.sub("fora"))),
            shared: Rc::clone(shared),
        }
    }
}

impl Widget for Veil {
    fn kind(&self) -> &'static str {
        "dock_dismiss"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn visible(&self) -> bool {
        self.shared.opened.get().is_some()
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
                self.shared.collapse();
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
        Some(self.shared.center.get().width)
    }
}

/// O relógio do auto-ocultar: abre o painel quando o ponteiro para no botão da faixa e
/// recolhe quando o ponteiro e o foco ficam fora do painel aberto. Invisível; só existe para
/// receber o tempo.
pub(crate) struct Timers {
    common: Common,
    shared: Rc<Shared>,
}

impl Timers {
    pub fn new(shared: &Rc<Shared>) -> Self {
        let mut common = Common::generated(Some(shared.sub("relogio")));
        common.width = Some(0.0);
        common.height = Some(0.0);
        Self {
            common,
            shared: Rc::clone(shared),
        }
    }
}

impl Shared {
    /// Se o ponteiro está parado num botão da faixa cujo painel ainda não abriu.
    fn hover_pending(&self) -> Option<String> {
        let opened = self.opened.get_untracked();
        let timing = self.timing.borrow();
        timing
            .hover_key
            .clone()
            .filter(|key| opened.as_ref() != Some(key))
    }

    /// Se há um painel aberto por cima sem ponteiro nem foco nele e na faixa.
    fn leaving(&self) -> bool {
        let timing = self.timing.borrow();
        self.opened.get_untracked().is_some()
            && timing.pointer_in.is_empty()
            && timing.focus_in.is_empty()
            && timing.strip_hover.is_none()
    }

    /// Passa o tempo `dt` nos atrasos de abrir e de recolher.
    pub fn advance(&self, dt: Duration) {
        if let Some(key) = self.hover_pending() {
            let ready = {
                let mut timing = self.timing.borrow_mut();
                timing.hover_elapsed += dt;
                timing.hover_elapsed >= DOCK_HOVER_DELAY
            };
            if ready {
                self.timing.borrow_mut().hover_elapsed = Duration::ZERO;
                self.show(&key, false);
            }
        } else {
            self.timing.borrow_mut().hover_elapsed = Duration::ZERO;
        }
        if self.leaving() {
            let ready = {
                let mut timing = self.timing.borrow_mut();
                timing.leave_elapsed += dt;
                timing.leave_elapsed >= DOCK_LEAVE_DELAY
            };
            if ready {
                self.collapse();
            }
        } else {
            self.timing.borrow_mut().leave_elapsed = Duration::ZERO;
        }
    }
}

impl Widget for Timers {
    fn kind(&self) -> &'static str {
        "dock_timers"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::Row
    }

    fn flex(&self) -> Option<Flex> {
        Some(Flex::default())
    }

    fn animating(&self) -> bool {
        true
    }

    fn ticking(&self) -> bool {
        untrack(|| self.shared.hover_pending().is_some() || self.shared.leaving())
    }

    fn tick(&mut self, dt: Duration) {
        untrack(|| self.shared.advance(dt));
    }

    fn decorative(&self) -> bool {
        true
    }
}

/// O botão de um painel auto-ocultar, na faixa da borda.
pub(crate) struct StripButton {
    common: Common,
    shared: Rc<Shared>,
    key: String,
    side: super::DockSide,
    press: Pressable,
}

impl StripButton {
    pub fn new(shared: &Rc<Shared>, key: &str, side: super::DockSide) -> Self {
        let spec = shared.spec(key).map(Rc::clone);
        let mut common = Common::generated(Some(shared.pane_id(key, Some("faixa"))));
        if let Some(spec) = &spec {
            let spec = Rc::clone(spec);
            common.tooltip = Some(Prop::Dynamic(Rc::new(move || spec.title.get())));
        }
        match side {
            super::DockSide::Bottom => common.height = Some(super::DOCK_STRIP_HEIGHT),
            _ => {
                common.width = Some(super::DOCK_STRIP_WIDTH);
                common.height = Some(crate::navigation_view::NAV_ITEM_HEIGHT);
            }
        }
        Self {
            common,
            shared: Rc::clone(shared),
            key: key.to_string(),
            side,
            press: Pressable::default(),
        }
    }

    fn title(&self) -> String {
        self.shared
            .spec(&self.key)
            .map(|spec| spec.title.get())
            .unwrap_or_default()
    }
}

impl Widget for StripButton {
    fn kind(&self) -> &'static str {
        "dock_strip_button"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        let icon = self
            .shared
            .spec(&self.key)
            .map_or(Symbol::Pin, |spec| spec.icon);
        Look::NavigationItem {
            label: self.title(),
            icon,
            visual: NavigationItemVisual {
                selected: self.shared.opened.get().as_deref() == Some(self.key.as_str()),
                state: self.press.state(),
                focus_ring: self.press.focus_ring(),
                compact: self.side != super::DockSide::Bottom,
                density: if self.side == super::DockSide::Bottom {
                    Density::Compact
                } else {
                    Density::Standard
                },
                ..NavigationItemVisual::default()
            },
        }
    }

    fn handle(&mut self, event: &Event, _: &mut EventCx) -> bool {
        if let Event::PointerMove { kind, inside } = *event {
            let mut timing = self.shared.timing.borrow_mut();
            if inside && kind.hovers() {
                timing.strip_hover = Some(self.key.clone());
                if timing.suppress.as_deref() != Some(self.key.as_str()) {
                    timing.hover_key = Some(self.key.clone());
                    timing.hover_elapsed = Duration::ZERO;
                }
            } else {
                if timing.strip_hover.as_deref() == Some(self.key.as_str()) {
                    timing.strip_hover = None;
                }
                if timing.hover_key.as_deref() == Some(self.key.as_str()) {
                    timing.hover_key = None;
                    timing.hover_elapsed = Duration::ZERO;
                }
                if timing.suppress.as_deref() == Some(self.key.as_str()) {
                    timing.suppress = None;
                }
            }
        }
        if matches!(event, Event::KeyDown(Key::Escape))
            && self.shared.opened.get_untracked().as_deref() == Some(self.key.as_str())
        {
            self.shared.collapse();
            return true;
        }
        let (consumed, clicked) = self.press.handle(event, true);
        if clicked {
            self.shared.timing.borrow_mut().suppress = Some(self.key.clone());
            self.shared.timing.borrow_mut().hover_key = None;
            if self.shared.opened.get_untracked().as_deref() == Some(self.key.as_str()) {
                self.shared.collapse();
            } else {
                self.shared.show(&self.key, true);
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
        Some(self.title())
    }
}

/// Um botão de 32×32 do cabeçalho de um painel.
pub(crate) struct HeaderButton {
    common: Common,
    icon: Dyn<Symbol>,
    label: Dyn<String>,
    on_click: Rc<dyn Fn()>,
    press: Pressable,
}

impl HeaderButton {
    pub fn new(
        id: String,
        icon: impl Fn() -> Symbol + 'static,
        label: impl Fn() -> String + 'static,
        on_click: impl Fn() + 'static,
    ) -> Self {
        let label: Dyn<String> = Rc::new(label);
        let mut common = Common::generated(Some(id));
        common.width = Some(super::DOCK_HEADER_HEIGHT);
        common.height = Some(super::DOCK_HEADER_HEIGHT);
        let tip = Rc::clone(&label);
        common.tooltip = Some(Prop::Dynamic(Rc::new(move || tip())));
        Self {
            common,
            icon: Rc::new(icon),
            label,
            on_click: Rc::new(on_click),
            press: Pressable::default(),
        }
    }

    pub fn visible(mut self, visible: impl Fn() -> bool + 'static) -> Self {
        self.common.visible = Some(Prop::Dynamic(Rc::new(visible)));
        self
    }
}

impl Widget for HeaderButton {
    fn kind(&self) -> &'static str {
        "dock_header_button"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::TabStripButton(TabStripButtonVisual {
            kind: TabStripButtonKind::Icon((self.icon)()),
            state: self.press.state(),
            density: Density::Standard,
            focus_ring: self.press.focus_ring(),
            color: None,
        })
    }

    fn handle(&mut self, event: &Event, _: &mut EventCx) -> bool {
        let (consumed, clicked) = self.press.handle(event, true);
        if clicked {
            let action = Rc::clone(&self.on_click);
            untrack(|| action());
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
        Some((self.label)())
    }
}
