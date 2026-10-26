//! Partes comuns dos testes de snapshot dos controles do M7.

// Cada teste usa só uma parte deste módulo.
#![allow(dead_code)]

use tomni_ui::PointerKind;
use tomni_ui::theme::Theme;
use tomni_ui_test::TestApp;

/// Nome do tema nos snapshots.
pub fn theme_name(theme: Theme) -> &'static str {
    match theme {
        Theme::Light => "claro",
        Theme::Dark => "escuro",
    }
}

/// Os cinco estados de um controle clicável, alcançados pelos eventos do `TestApp`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Rest,
    Hover,
    Pressed,
    Disabled,
    Focused,
}

impl State {
    pub const ALL: [Self; 5] = [
        Self::Rest,
        Self::Hover,
        Self::Pressed,
        Self::Disabled,
        Self::Focused,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Rest => "rest",
            Self::Hover => "hover",
            Self::Pressed => "pressed",
            Self::Disabled => "disabled",
            Self::Focused => "focused",
        }
    }

    /// Leva o widget `id` ao estado. `Disabled` vem do `.disabled(true)` montado na tela.
    pub fn apply(self, app: &mut TestApp, id: &str) {
        match self {
            Self::Rest | Self::Disabled => {}
            Self::Hover => {
                app.hover(id);
            }
            Self::Pressed => {
                // Pressiona e não solta: soltar alternaria o valor.
                let point = app.rect(id).center();
                app.ui().pointer_move(point, PointerKind::Mouse);
                app.ui().pointer_down(point, PointerKind::Mouse, 1);
                app.frame();
            }
            Self::Focused => {
                app.tab();
            }
        }
    }
}
