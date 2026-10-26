//! A árvore que o `DockHost` monta: o corpo (áreas fixadas, faixas e conteúdo), os painéis
//! abertos por cima, o painel maximizado e a casca de cada painel.

use std::rc::Rc;
use std::sync::atomic::Ordering;

use tomni_ui_reactive::{Get, GetUntracked, With, WithUntracked, effect, on_cleanup, untrack};

use super::parts::{DockSizer, HeaderButton, List, Part, StripButton, Timers, Veil};
use super::shared::{PaneSpec, Placement, Shared, keys_where, placement, strip_keys};
use super::{DOCK_HEADER_HEIGHT, DOCK_PANE_SIZE, DockController, DockLayout, DockMode, DockSide};
use crate::event::{Event, Key};
use crate::flyout::MenuFlyout;
use crate::split::SPLIT_SIZER;
use crate::symbol::Symbol;
use crate::text::Text;
use crate::widget::{Direction, Insets, Look, View, Widget, Within};
use crate::windows::WindowHandle;
use crate::{Form, strings};

fn boxed(widget: impl Widget + 'static) -> Box<dyn Widget> {
    Box::new(widget)
}

/// O eixo em que o painel do lado `side` cresce: a largura, ou a altura embaixo.
fn is_vertical(side: DockSide) -> bool {
    side == DockSide::Bottom
}

/// A chave do painel maximizado, se há um que aparece assim.
fn maximized_key(layout: &DockLayout) -> Option<String> {
    layout
        .maximized
        .clone()
        .filter(|key| placement(layout, key) == Some(Placement::Maximized))
}

/// As chaves dos painéis fixados no lado `side`, na ordem da área.
fn fixed_keys(layout: &DockLayout, side: DockSide) -> Vec<String> {
    keys_where(layout, side, |p| p == Placement::Fixed(side))
}

/// As chaves dos painéis auto-ocultar do lado `side` que ficam por cima do conteúdo.
fn overlay_keys(layout: &DockLayout, side: DockSide) -> Vec<String> {
    keys_where(layout, side, |p| p == Placement::Overlay(side))
}

impl Shared {
    /// O tamanho do centro no eixo do lado `side`: a largura, ou a altura embaixo.
    fn center_along(&self, side: DockSide) -> f64 {
        let size = self.center.get();
        if is_vertical(side) {
            size.height
        } else {
            size.width
        }
    }

    /// O menor tamanho que a área de `side` aceita: o maior `min_size` dos fixados.
    fn area_min(&self, layout: &DockLayout, side: DockSide) -> f64 {
        fixed_keys(layout, side)
            .iter()
            .filter_map(|key| self.spec(key))
            .map(|spec| spec.min_size)
            .fold(0.0, f64::max)
    }

    /// A largura (ou a altura) do painel `key` aberto por cima, limitada ao centro.
    fn overlay_extent(&self, key: &str) -> f64 {
        let Some(spec) = self.spec(key) else {
            return DOCK_PANE_SIZE;
        };
        let (side, size) = self
            .layout
            .with(|l| l.pane(key).map(|p| (p.side, p.overlay_size)))
            .unwrap_or((spec.side, spec.size));
        let size = size.max(spec.min_size);
        let available = self.center_along(side) - SPLIT_SIZER;
        if available > 0.0 {
            size.min(available.max(spec.min_size))
        } else {
            size
        }
    }

    /// Grava a largura (ou a altura) do painel `key` aberto por cima.
    fn set_overlay_size(&self, key: &str, value: f64) {
        let Some(spec) = self.spec(key) else {
            return;
        };
        let side = self.state(key).map_or(spec.side, |state| state.side);
        let available = untrack(|| self.center_along(side)) - SPLIT_SIZER;
        let high = if available > 0.0 {
            available.max(spec.min_size)
        } else {
            f64::MAX
        };
        let value = value.clamp(spec.min_size, high);
        self.edit(|l| match l.panes.get_mut(key) {
            Some(pane) if pane.overlay_size != value => {
                pane.overlay_size = value;
                true
            }
            _ => false,
        });
    }

    /// Grava o tamanho da área fixada de `side`, respeitando os mínimos.
    fn set_area_size(&self, side: DockSide, value: f64) {
        let current = self.layout.with_untracked(|l| l.area(side).size);
        let low = self.layout.with_untracked(|l| self.area_min(l, side));
        let available = untrack(|| self.center_along(side));
        let high = if available > 0.0 {
            current + available - self.min_content
        } else {
            f64::MAX
        };
        let value = value.clamp(low, high.max(low));
        self.edit(|l| {
            let area = l.area_mut(side);
            if area.size == value {
                return false;
            }
            area.size = value;
            true
        });
    }

    /// O tamanho medido do painel `key` no eixo da área.
    fn measured(&self, key: &str, side: DockSide) -> f64 {
        self.sizes.borrow().get(key).map_or(0.0, |size| {
            if is_vertical(side) {
                size.width
            } else {
                size.height
            }
        })
    }

    /// Passa o peso de `key` para o próximo fixado do lado, para que `key` fique com `wanted`.
    fn set_pane_extent(&self, key: &str, side: DockSide, wanted: f64) {
        let keys = self.layout.with_untracked(|l| fixed_keys(l, side));
        let Some(index) = keys.iter().position(|k| k == key) else {
            return;
        };
        let Some(next) = keys.get(index + 1) else {
            return;
        };
        let (a, b) = (self.measured(key, side), self.measured(next, side));
        let total = a + b;
        if total <= 0.0 {
            return;
        }
        let min_a = self.spec(key).map_or(0.0, |s| s.min_size);
        let min_b = self.spec(next).map_or(0.0, |s| s.min_size);
        let wanted = wanted.clamp(min_a, (total - min_b).max(min_a));
        self.edit(|l| {
            let (Some(wa), Some(wb)) = (
                l.pane(key).map(|p| p.weight),
                l.pane(next).map(|p| p.weight),
            ) else {
                return false;
            };
            let sum = wa + wb;
            let new_a = sum * wanted / total;
            let new_b = sum - new_a;
            if new_a <= 0.0 || new_b <= 0.0 {
                return false;
            }
            if let Some(pane) = l.panes.get_mut(key) {
                pane.weight = new_a;
            }
            if let Some(pane) = l.panes.get_mut(next) {
                pane.weight = new_b;
            }
            true
        });
    }
}

/// Os filhos do `DockHost`: o corpo, o painel maximizado e o que liga o resto.
pub(crate) fn root(
    shared: &Rc<Shared>,
    controller: Option<DockController>,
    content: Box<dyn Widget>,
) -> Vec<Box<dyn Widget>> {
    let s = Rc::clone(shared);
    let body = body(shared, content).visible(move || s.layout.with(|l| maximized_key(l).is_none()));

    let (items, maker) = (Rc::clone(shared), Rc::clone(shared));
    let maximized = Part::column("dock_maximized")
        .grow(1.0)
        .fill()
        .visible({
            let s = Rc::clone(shared);
            move || s.layout.with(|l| maximized_key(l).is_some())
        })
        .child(List::new(
            move || {
                items
                    .layout
                    .with(maximized_key)
                    .map(|key| vec![key])
                    .unwrap_or_default()
            },
            move |key| shell(&maker, key, Placement::Maximized),
        ));

    let s = Rc::clone(shared);
    let setup = List::new(
        || vec!["ligacao".to_string()],
        move |_| setup(&s, controller.clone()),
    );
    vec![boxed(body), boxed(maximized), boxed(setup)]
}

/// O que roda uma vez, no escopo do item: liga os signals do controlador, abre e fecha as
/// janelas flutuantes e recolhe o painel que deixou de ser auto-ocultar.
fn setup(shared: &Rc<Shared>, controller: Option<DockController>) -> Box<dyn Widget> {
    let scope = tomni_ui_reactive::Scope::detached();
    *shared.wiring.borrow_mut() = Some(scope);
    if let Some(controller) = &controller {
        controller.wire(shared);
    }

    let s = Rc::clone(shared);
    effect(move || {
        let layout = s.layout.get();
        untrack(|| s.sync_windows(&layout));
    });

    let s = Rc::clone(shared);
    effect(move || {
        let layout = s.layout.get();
        untrack(|| {
            if let Some(key) = s.opened.get_untracked()
                && !matches!(placement(&layout, &key), Some(Placement::Overlay(_)))
            {
                s.collapse();
            }
        });
    });

    let disposed = Rc::clone(shared).disposed.clone();
    on_cleanup(move || disposed.store(true, Ordering::Relaxed));
    boxed(Timers::new(shared))
}

/// O corpo: a faixa, a área e o divisor de cada lado, e o centro.
fn body(shared: &Rc<Shared>, content: Box<dyn Widget>) -> Part {
    Part::row("dock_body")
        .grow(1.0)
        .child(strip(shared, DockSide::Left))
        .child(area(shared, DockSide::Left))
        .child(area_divider(shared, DockSide::Left))
        .child(center(shared, content))
        .child(area_divider(shared, DockSide::Right))
        .child(area(shared, DockSide::Right))
        .child(strip(shared, DockSide::Right))
}

/// O centro: o conteúdo com os painéis por cima, e embaixo a área e a faixa de baixo.
fn center(shared: &Rc<Shared>, content: Box<dyn Widget>) -> Part {
    let min = shared.min_content;
    Part::column("dock_center")
        .grow(1.0)
        .dynamic_basis(move || (0.0, min))
        .child(overlay_bottom(shared))
        .child(content_row(shared, content))
        .child(area_divider(shared, DockSide::Bottom))
        .child(area(shared, DockSide::Bottom))
        .child(strip(shared, DockSide::Bottom))
}

/// O conteúdo do app, com as camadas dos painéis abertos por cima.
fn content_row(shared: &Rc<Shared>, content: Box<dyn Widget>) -> Part {
    let (s, min) = (Rc::clone(shared), shared.min_content);
    let center_size = s.center;
    Part::row("dock_content_row")
        .grow(1.0)
        .dynamic_basis(move || (0.0, min))
        .on_resize(move |size| {
            if untrack(|| center_size.get()) != size {
                tomni_ui_reactive::Set::set(&center_size, size);
            }
        })
        .child(
            Part::row("dock_veil_slot")
                .raised()
                .dynamic_width(|| 0.0)
                .child(Veil::new(shared)),
        )
        .child(overlay_side(shared, DockSide::Left))
        .child(overlay_side(shared, DockSide::Right))
        .child(
            Part::column("dock_content")
                .id(shared.sub("conteudo"))
                .grow(1.0)
                .fill()
                .boxed(content),
        )
}

/// A camada dos painéis auto-ocultar da esquerda ou da direita: largura zero, e o painel
/// aberto transborda sobre o conteúdo.
fn overlay_side(shared: &Rc<Shared>, side: DockSide) -> Part {
    let mut slot = Part::row("dock_overlay_slot")
        .raised()
        .dynamic_width(|| 0.0);
    if side == DockSide::Right {
        // Empurra o painel para a borda direita do conteúdo.
        let s = Rc::clone(shared);
        slot = slot.child(Part::row("dock_spacer").dynamic_width(move || {
            let open = s.opened.get();
            match open {
                Some(key)
                    if s.layout.with(|l| {
                        placement(l, &key) == Some(Placement::Overlay(DockSide::Right))
                    }) =>
                {
                    (s.center_along(side) - s.overlay_extent(&key) - SPLIT_SIZER).max(0.0)
                }
                _ => 0.0,
            }
        }));
    }
    let (items, maker) = (Rc::clone(shared), Rc::clone(shared));
    slot.child(List::new(
        move || items.layout.with(|l| overlay_keys(l, side)),
        move |key| overlay(&maker, key, side),
    ))
}

/// A camada dos painéis auto-ocultar de baixo.
fn overlay_bottom(shared: &Rc<Shared>) -> Part {
    let side = DockSide::Bottom;
    let (s, items, maker) = (Rc::clone(shared), Rc::clone(shared), Rc::clone(shared));
    Part::column("dock_overlay_slot")
        .raised()
        .dynamic_basis(|| (0.0, 0.0))
        .child(Part::row("dock_spacer").dynamic_basis(move || {
            let height = match s.opened.get() {
                Some(key)
                    if s.layout
                        .with(|l| placement(l, &key) == Some(Placement::Overlay(side))) =>
                {
                    (s.center_along(side) - s.overlay_extent(&key) - SPLIT_SIZER).max(0.0)
                }
                _ => 0.0,
            };
            (height, height)
        }))
        .child(List::new(
            move || items.layout.with(|l| overlay_keys(l, side)),
            move |key| overlay(&maker, key, side),
        ))
}

/// Um painel auto-ocultar: a casca com a borda que muda o tamanho. Só aparece enquanto está
/// aberto.
fn overlay(shared: &Rc<Shared>, key: &str, side: DockSide) -> Box<dyn Widget> {
    let direction = if is_vertical(side) {
        Direction::Column
    } else {
        Direction::Row
    };
    let (open, size) = (Rc::clone(shared), Rc::clone(shared));
    let name = key.to_string();
    let open_name = name.clone();
    let size_name = name.clone();
    let mut wrapper = Part::new(direction, "dock_overlay")
        .visible(move || open.opened.get().as_deref() == Some(open_name.as_str()));
    wrapper = if is_vertical(side) {
        wrapper.dynamic_basis(move || {
            let extent = size.overlay_extent(&size_name) + SPLIT_SIZER;
            (extent, extent)
        })
    } else {
        wrapper.dynamic_width(move || size.overlay_extent(&size_name) + SPLIT_SIZER)
    };
    let (get, set) = (Rc::clone(shared), Rc::clone(shared));
    let (get_key, set_key) = (name.clone(), name.clone());
    // Arrastar para dentro do conteúdo aumenta o painel: à esquerda para a direita, à
    // direita e embaixo para a esquerda e para cima.
    let (axis, sign) = match side {
        DockSide::Left => (Direction::Row, 1.0),
        DockSide::Right => (Direction::Row, -1.0),
        DockSide::Bottom => (Direction::Column, -1.0),
    };
    let border = DockSizer::new(
        shared.pane_id(key, Some("borda")),
        axis,
        sign,
        move || untrack(|| get.overlay_extent(&get_key)),
        move |value| set.set_overlay_size(&set_key, value),
    );
    let inner = shell(shared, key, Placement::Overlay(side));
    if side == DockSide::Left {
        wrapper.boxed(inner).child(border).into_box()
    } else {
        wrapper.child(border).boxed(inner).into_box()
    }
}

impl Part {
    fn into_box(self) -> Box<dyn Widget> {
        boxed(self)
    }
}

/// A faixa dos botões dos painéis auto-ocultar de `side`.
fn strip(shared: &Rc<Shared>, side: DockSide) -> Part {
    let (visible, items, maker) = (Rc::clone(shared), Rc::clone(shared), Rc::clone(shared));
    let part = if is_vertical(side) {
        Part::row("dock_strip").height(super::DOCK_STRIP_HEIGHT)
    } else {
        Part::column("dock_strip").width(super::DOCK_STRIP_WIDTH)
    };
    part.visible(move || visible.layout.with(|l| !strip_keys(l, side).is_empty()))
        .child(List::new(
            move || items.layout.with(|l| strip_keys(l, side)),
            move |key| boxed(StripButton::new(&maker, key, side)),
        ))
}

/// A área fixada de `side`: os painéis lado a lado (embaixo) ou empilhados (nos lados), com um
/// divisor entre dois vizinhos.
fn area(shared: &Rc<Shared>, side: DockSide) -> Part {
    let (visible, basis, items, maker) = (
        Rc::clone(shared),
        Rc::clone(shared),
        Rc::clone(shared),
        Rc::clone(shared),
    );
    let direction = if is_vertical(side) {
        Direction::Row
    } else {
        Direction::Column
    };
    Part::new(direction, "dock_area")
        .visible(move || visible.layout.with(|l| !fixed_keys(l, side).is_empty()))
        .dynamic_basis(move || {
            basis.layout.with(|l| {
                let min = basis.area_min(l, side);
                (l.area(side).size.max(min), min)
            })
        })
        .child(List::new(
            move || {
                items.layout.with(|l| {
                    let keys = fixed_keys(l, side);
                    let mut entries = Vec::new();
                    for (index, key) in keys.iter().enumerate() {
                        entries.push(format!("p:{key}"));
                        if index + 1 < keys.len() {
                            entries.push(format!("d:{key}"));
                        }
                    }
                    entries
                })
            },
            move |entry| match entry.split_once(':') {
                Some(("d", key)) => pane_divider(&maker, key, side),
                Some((_, key)) => shell(&maker, key, Placement::Fixed(side)),
                None => boxed(Part::row("dock_empty")),
            },
        ))
}

/// O divisor entre a área fixada e o centro.
fn area_divider(shared: &Rc<Shared>, side: DockSide) -> DockSizer {
    let (get, set, visible) = (Rc::clone(shared), Rc::clone(shared), Rc::clone(shared));
    let (axis, sign) = match side {
        DockSide::Left => (Direction::Row, 1.0),
        DockSide::Right => (Direction::Row, -1.0),
        DockSide::Bottom => (Direction::Column, -1.0),
    };
    DockSizer::new(
        shared.sub(&format!("{}-divisor", side.id_name())),
        axis,
        sign,
        move || get.layout.with_untracked(|l| l.area(side).size),
        move |value| set.set_area_size(side, value),
    )
    .visible(move || visible.layout.with(|l| !fixed_keys(l, side).is_empty()))
}

/// O divisor entre o painel `key` e o seguinte da mesma área.
fn pane_divider(shared: &Rc<Shared>, key: &str, side: DockSide) -> Box<dyn Widget> {
    let (get, set) = (Rc::clone(shared), Rc::clone(shared));
    let (get_key, set_key) = (key.to_string(), key.to_string());
    let axis = if is_vertical(side) {
        Direction::Row
    } else {
        Direction::Column
    };
    boxed(DockSizer::new(
        shared.pane_id(key, Some("divisor")),
        axis,
        1.0,
        move || get.measured(&get_key, side),
        move |value| set.set_pane_extent(&set_key, side, value),
    ))
}

/// A casca de um painel: o cabeçalho e o corpo com o conteúdo.
pub(crate) fn shell(shared: &Rc<Shared>, key: &str, place: Placement) -> Box<dyn Widget> {
    let Some(spec) = shared.spec(key).map(Rc::clone) else {
        return boxed(Part::row("dock_empty"));
    };
    let body = Part::column("dock_pane_body")
        .id(shared.pane_id(key, Some("corpo")))
        .grow(1.0)
        .fill()
        .boxed((spec.content)());

    let mut part = Part::column("dock_pane")
        .id(shared.pane_id(key, None))
        .grow(1.0)
        .child(header(shared, &spec, place))
        .child(body);
    part = match place {
        Placement::Overlay(side) => part.look(move || Look::DockOverlay { side }),
        _ => part,
    };
    if let Placement::Fixed(_) = place {
        let (weight, min) = (Rc::clone(shared), spec.min_size);
        let name = key.to_string();
        part = part
            .dynamic_grow(move || {
                weight
                    .layout
                    .with(|l| l.pane(&name).map_or(1.0, |pane| pane.weight))
                    .max(f64::EPSILON)
            })
            .dynamic_basis(move || (0.0, min));
    }

    let (sizes, name) = (Rc::clone(shared), key.to_string());
    part = part.on_resize(move |size| {
        sizes.sizes.borrow_mut().insert(name.clone(), size);
    });

    let (s, name) = (Rc::clone(shared), key.to_string());
    part = part.on_event(move |event, cx| {
        if *event != Event::KeyDown(Key::Escape) {
            return false;
        }
        match place {
            Placement::Overlay(_)
                if untrack(|| s.opened.get()).as_deref() == Some(name.as_str()) =>
            {
                s.collapse();
                cx.focus_later(s.pane_id(&name, Some("faixa")));
                true
            }
            Placement::Maximized => {
                s.restore();
                true
            }
            _ => false,
        }
    });

    if let Placement::Overlay(_) = place {
        // Interativo para que o ponteiro sobre uma parte vazia do painel também conte como dentro.
        part = part.interactive();
        let (s, name) = (Rc::clone(shared), key.to_string());
        part = part.on_within(move |change| {
            let mut timing = s.timing.borrow_mut();
            let (set, inside) = match change {
                Within::Pointer(inside) => (&mut timing.pointer_in, inside),
                Within::Focus(inside) => (&mut timing.focus_in, inside),
            };
            if inside {
                set.insert(name.clone());
            } else {
                set.remove(&name);
            }
        });
    }
    boxed(part)
}

/// O cabeçalho: o título, os botões do app e os da lib.
fn header(shared: &Rc<Shared>, spec: &Rc<PaneSpec>, place: Placement) -> Part {
    let key = spec.key.as_str();
    let title = Rc::clone(spec);
    let mut part = Part::row("dock_header")
        .id(shared.pane_id(key, Some("cabecalho")))
        .height(DOCK_HEADER_HEIGHT)
        .interactive();
    part.flex.padding = Insets::new(0.0, 4.0, 0.0, 12.0);
    part.flex.align = crate::widget::Align::Center;
    part = part.child(
        Text::new(move || title.title.get())
            .body_strong()
            .max_lines(1)
            .id(shared.pane_id(key, Some("titulo")))
            .grow(1),
    );
    if let Some(actions) = &spec.actions {
        part = part.boxed(actions());
    }

    // Flutuar (ou encaixar, se já flutua).
    let (mode, icon, label, click) = (
        Rc::clone(shared),
        Rc::clone(shared),
        Rc::clone(shared),
        Rc::clone(shared),
    );
    let (mode_key, icon_key, label_key, click_key) = (
        key.to_string(),
        key.to_string(),
        key.to_string(),
        key.to_string(),
    );
    let floatable = spec.floatable;
    part = part.child(
        HeaderButton::new(
            shared.pane_id(key, Some("flutuar")),
            move || {
                if icon.mode_of(&icon_key) == Some(DockMode::Floating) {
                    match icon.layout.with(|l| l.pane(&icon_key).map(|p| p.side)) {
                        Some(DockSide::Right) => Symbol::PanelRight,
                        Some(DockSide::Bottom) => Symbol::PanelBottom,
                        _ => Symbol::PanelLeft,
                    }
                } else {
                    Symbol::WindowNew
                }
            },
            move || {
                if label.mode_of(&label_key) == Some(DockMode::Floating) {
                    strings().dock_dock.to_string()
                } else {
                    strings().dock_float.to_string()
                }
            },
            move || {
                if click.mode_of(&click_key) == Some(DockMode::Floating) {
                    untrack(|| click.dock(&click_key));
                } else {
                    untrack(|| click.float(&click_key));
                }
            },
        )
        .visible(move || match mode.mode_of(&mode_key) {
            Some(DockMode::Floating) => true,
            Some(DockMode::Pinned | DockMode::AutoHide) => floatable,
            _ => false,
        }),
    );

    // O alfinete.
    let (mode, icon, label, click) = (
        Rc::clone(shared),
        Rc::clone(shared),
        Rc::clone(shared),
        Rc::clone(shared),
    );
    let (mode_key, icon_key, label_key, click_key) = (
        key.to_string(),
        key.to_string(),
        key.to_string(),
        key.to_string(),
    );
    part = part.child(
        HeaderButton::new(
            shared.pane_id(key, Some("alfinete")),
            move || {
                if icon.mode_of(&icon_key) == Some(DockMode::Pinned) {
                    Symbol::PinOff
                } else {
                    Symbol::Pin
                }
            },
            move || {
                if label.mode_of(&label_key) == Some(DockMode::Pinned) {
                    strings().dock_unpin.to_string()
                } else {
                    strings().dock_pin.to_string()
                }
            },
            move || {
                let pinned = untrack(|| click.state(&click_key))
                    .is_some_and(|state| state.mode == DockMode::Pinned);
                if pinned {
                    click.unpin(&click_key);
                } else {
                    click.pin(&click_key);
                }
            },
        )
        .visible(move || {
            matches!(
                mode.mode_of(&mode_key),
                Some(DockMode::Pinned | DockMode::AutoHide)
            )
        }),
    );

    // Fechar.
    if spec.closable {
        let click = Rc::clone(shared);
        let click_key = key.to_string();
        part = part.child(HeaderButton::new(
            shared.pane_id(key, Some("fechar")),
            || Symbol::Dismiss,
            || strings().dock_close.to_string(),
            move || {
                click.close(&click_key);
            },
        ));
    }

    // Duplo clique no cabeçalho maximiza e restaura.
    let (s, name) = (Rc::clone(shared), key.to_string());
    part = part.on_event(move |event, _| {
        if !matches!(event, Event::PointerDown { clicks: 2, .. }) {
            return false;
        }
        let maximized = untrack(|| {
            s.layout
                .with(|l| l.maximized.as_deref() == Some(name.as_str()))
        });
        if maximized {
            s.restore();
        } else {
            s.maximize(&name);
        }
        true
    });
    part.common.set_context_flyout(menu(shared, spec, place));
    part
}

/// O menu do cabeçalho.
fn menu(shared: &Rc<Shared>, spec: &Rc<PaneSpec>, _place: Placement) -> MenuFlyout {
    let key = spec.key.as_str();
    let id = |name: &str| format!("{}-menu-{name}", shared.pane_id(key, None));
    let mut menu = MenuFlyout::new().id(shared.pane_id(key, Some("menu")));

    let (label, action, visible) = (Rc::clone(shared), Rc::clone(shared), Rc::clone(shared));
    let (label_key, action_key, visible_key) = (key.to_string(), key.to_string(), key.to_string());
    menu = menu
        .item(
            id("alfinete"),
            move || {
                if label.mode_of(&label_key) == Some(DockMode::Pinned) {
                    strings().dock_unpin.to_string()
                } else {
                    strings().dock_pin.to_string()
                }
            },
            move || {
                let pinned = action
                    .state(&action_key)
                    .is_some_and(|state| state.mode == DockMode::Pinned);
                if pinned {
                    action.unpin(&action_key);
                } else {
                    action.pin(&action_key);
                }
            },
        )
        .icon(Symbol::Pin)
        .item_visible(move || {
            matches!(
                visible.mode_of(&visible_key),
                Some(DockMode::Pinned | DockMode::AutoHide)
            )
        });

    let (label, action, visible) = (Rc::clone(shared), Rc::clone(shared), Rc::clone(shared));
    let (label_key, action_key, visible_key) = (key.to_string(), key.to_string(), key.to_string());
    let floatable = spec.floatable;
    menu = menu
        .item(
            id("flutuar"),
            move || {
                if label.mode_of(&label_key) == Some(DockMode::Floating) {
                    strings().dock_dock.to_string()
                } else {
                    strings().dock_float.to_string()
                }
            },
            move || {
                let floating = action
                    .state(&action_key)
                    .is_some_and(|state| state.mode == DockMode::Floating);
                if floating {
                    action.dock(&action_key);
                } else {
                    action.float(&action_key);
                }
            },
        )
        .icon(Symbol::WindowNew)
        .item_visible(move || match visible.mode_of(&visible_key) {
            Some(DockMode::Floating) => true,
            Some(DockMode::Pinned | DockMode::AutoHide) => floatable,
            _ => false,
        });

    for (name, side, icon) in [
        ("esquerda", DockSide::Left, Symbol::PanelLeft),
        ("direita", DockSide::Right, Symbol::PanelRight),
        ("embaixo", DockSide::Bottom, Symbol::PanelBottom),
    ] {
        let (action, visible) = (Rc::clone(shared), Rc::clone(shared));
        let (action_key, visible_key) = (key.to_string(), key.to_string());
        menu = menu
            .item(
                id(name),
                move || match side {
                    DockSide::Left => strings().dock_side_left.to_string(),
                    DockSide::Right => strings().dock_side_right.to_string(),
                    DockSide::Bottom => strings().dock_side_bottom.to_string(),
                },
                move || {
                    action.move_to(&action_key, side);
                },
            )
            .icon(icon)
            .item_visible(move || {
                let state = visible.layout.with(|l| l.pane(&visible_key).cloned());
                state.is_some_and(|state| {
                    matches!(state.mode, DockMode::Pinned | DockMode::AutoHide)
                        && state.side != side
                })
            });
    }

    let (label, action, visible) = (Rc::clone(shared), Rc::clone(shared), Rc::clone(shared));
    let (action_key, visible_key) = (key.to_string(), key.to_string());
    let label_key = key.to_string();
    menu = menu
        .item(
            id("maximizar"),
            move || {
                let maximized = label
                    .layout
                    .with(|l| l.maximized.as_deref() == Some(label_key.as_str()));
                if maximized {
                    strings().dock_restore.to_string()
                } else {
                    strings().dock_maximize.to_string()
                }
            },
            move || {
                let maximized = action
                    .layout
                    .with_untracked(|l| l.maximized.as_deref() == Some(action_key.as_str()));
                if maximized {
                    action.restore();
                } else {
                    action.maximize(&action_key);
                }
            },
        )
        .icon(Symbol::FullScreenMaximize)
        .item_visible(move || {
            matches!(
                visible.mode_of(&visible_key),
                Some(DockMode::Pinned | DockMode::AutoHide)
            )
        });

    if spec.closable {
        let (action, visible) = (Rc::clone(shared), Rc::clone(shared));
        let (action_key, visible_key) = (key.to_string(), key.to_string());
        menu = menu
            .separator()
            .item(
                id("fechar"),
                || strings().dock_close.to_string(),
                move || {
                    action.close(&action_key);
                },
            )
            .icon(Symbol::Dismiss)
            .item_visible(move || visible.mode_of(&visible_key) != Some(DockMode::Closed));
    }
    menu
}

/// A tela da janela de um painel flutuando.
pub(crate) fn floating(shared: Rc<Shared>, pane: Rc<PaneSpec>) -> impl View {
    let handle = WindowHandle::current();
    let (hook, key) = (Rc::clone(&shared), pane.key.clone());
    crate::on_close_requested(move || hook.floating_closed(&key, handle));
    let title = Rc::clone(&pane);
    Form::new(move || title.title.get()).child(Part::column("dock_floating").fill().boxed(shell(
        &shared,
        &pane.key,
        Placement::Floating,
    )))
}
