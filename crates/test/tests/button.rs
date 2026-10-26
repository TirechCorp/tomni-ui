//! Marco M1: `Button` pintado pelo `FluentDrawer`, 5 estados × 3 variantes (o link é do M10)
//! × 2 temas.
//!
//! Cada estado é alcançado mandando eventos para o botão, como faria a plataforma: o snapshot
//! confere a máquina de estados e o tema juntos.

use tomni_ui_core::{Button, ButtonState, ButtonVariant, Event, FocusSource, PointerKind};
use tomni_ui_render::Pixmap;
use tomni_ui_render::kurbo::Rect;
use tomni_ui_test::{assert_snapshot, render};
use tomni_ui_theme::{Drawer, FluentDrawer, Theme};

/// Espaço em volta do botão, para caber o anel de foco.
const MARGIN: f64 = 8.0;

#[derive(Clone, Copy, Debug)]
enum Case {
    Rest,
    Hover,
    Pressed,
    Disabled,
    Focused,
}

impl Case {
    const ALL: [Self; 5] = [
        Self::Rest,
        Self::Hover,
        Self::Pressed,
        Self::Disabled,
        Self::Focused,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Rest => "rest",
            Self::Hover => "hover",
            Self::Pressed => "pressed",
            Self::Disabled => "disabled",
            Self::Focused => "focused",
        }
    }

    /// Monta o botão e leva até o estado pelos eventos.
    fn button(self, variant: ButtonVariant) -> Button {
        let mut button = Button::new("Entrar")
            .id("entrar")
            .disabled(matches!(self, Self::Disabled));
        match variant {
            ButtonVariant::Standard => {}
            ButtonVariant::Accent => button = button.accent(),
            ButtonVariant::Hyperlink => button = button.hyperlink(),
        }
        let mouse = PointerKind::Mouse;
        let events: &[Event] = match self {
            Self::Rest | Self::Disabled => &[],
            Self::Hover => &[Event::PointerMove {
                kind: mouse,
                inside: true,
            }],
            Self::Pressed => &[
                Event::PointerMove {
                    kind: mouse,
                    inside: true,
                },
                Event::PointerDown {
                    kind: mouse,
                    clicks: 1,
                },
            ],
            Self::Focused => &[Event::FocusIn(FocusSource::Keyboard)],
        };
        for event in events {
            button.handle(event);
        }
        let expected = match self {
            Self::Rest | Self::Focused => ButtonState::Rest,
            Self::Hover => ButtonState::Hover,
            Self::Pressed => ButtonState::Pressed,
            Self::Disabled => ButtonState::Disabled,
        };
        assert_eq!(button.visual().state, expected);
        assert_eq!(button.visual().focus_ring, matches!(self, Self::Focused));
        button
    }
}

fn draw(theme: Theme, variant: ButtonVariant, case: Case) -> Pixmap {
    let button = case.button(variant);
    let label = button.current_label();
    let mut drawer = FluentDrawer::new(theme);
    let size = drawer.measure_button(&label, button.visual());
    let rect = Rect::from_origin_size((MARGIN, MARGIN), size);
    let background = drawer.tokens().solid_background_base;
    render(
        (size.width + 2.0 * MARGIN) as u16,
        (size.height + 2.0 * MARGIN) as u16,
        background,
        |canvas| drawer.draw_button(canvas, rect, &label, button.visual()),
    )
}

fn check(theme: Theme, variant: ButtonVariant) {
    let theme_name = match theme {
        Theme::Light => "claro",
        Theme::Dark => "escuro",
    };
    let variant_name = match variant {
        ButtonVariant::Standard => "standard",
        ButtonVariant::Accent => "accent",
        ButtonVariant::Hyperlink => "hyperlink",
    };
    for case in Case::ALL {
        let name = format!("button_{variant_name}_{}_{theme_name}", case.name());
        assert_snapshot!(&name, draw(theme, variant, case));
    }
}

#[test]
fn standard_claro() {
    check(Theme::Light, ButtonVariant::Standard);
}

#[test]
fn standard_escuro() {
    check(Theme::Dark, ButtonVariant::Standard);
}

#[test]
fn accent_claro() {
    check(Theme::Light, ButtonVariant::Accent);
}

#[test]
fn accent_escuro() {
    check(Theme::Dark, ButtonVariant::Accent);
}

#[test]
fn hyperlink_claro() {
    check(Theme::Light, ButtonVariant::Hyperlink);
}

#[test]
fn hyperlink_escuro() {
    check(Theme::Dark, ButtonVariant::Hyperlink);
}
