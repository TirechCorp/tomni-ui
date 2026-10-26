//! Botões dentro de um campo, como os do `TextBox`, do `PasswordBox` e do `NumberBox` do
//! WinUI: revelar a senha do `Edit`, limpar e incrementar o `NumberBox` e o botão de busca do
//! `AutoSuggestBox`.

use std::fmt;
use std::rc::Rc;
use std::time::Duration;

use kurbo::Rect;

use crate::button::ButtonState;
use crate::cx::EventCx;
use crate::density::Density;
use crate::event::Event;
use crate::prop::Prop;
use crate::symbol::Symbol;
use crate::widget::{Common, Insets, Look, Widget};

/// Atraso antes de repetir e intervalo da repetição de um botão de incremento segurado: o
/// `Delay` e o `Interval` do `RepeatButton` do WinUI (250 ms cada).
pub const SPIN_REPEAT: Duration = Duration::from_millis(250);

/// Ícone de um botão dentro do campo.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FieldGlyph {
    /// Olho: revela a senha enquanto está pressionado.
    Reveal,
    /// X: limpa o campo.
    Clear,
    /// Seta para cima: aumenta o valor.
    Up,
    /// Seta para baixo: diminui o valor.
    Down,
    /// Setas para cima e para baixo: o indicador fixo do `NumberBox` com os botões de
    /// incremento num popup. Não é botão.
    UpDown,
    /// Um ícone do `Symbol`: o `QueryIcon` do `AutoSuggestBox`.
    Symbol(Symbol),
}

/// Tudo o que o `Drawer` precisa para pintar um botão dentro do campo.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldButtonVisual {
    /// Ícone.
    pub glyph: FieldGlyph,
    /// Estado.
    pub state: ButtonState,
    /// Botão do popup de incremento (ícone de 16); o de dentro do campo tem ícone de 12.
    pub large: bool,
    /// Espaço entre o retângulo do widget e a área pintada, como a `Margin` dos botões de
    /// dentro do `TextBox` e do `NumberBox` do WinUI (`0,4,4,4`): o retângulo do widget vai
    /// até as bordas do campo, e o fundo do botão fica dentro dele.
    pub margin: Insets,
}

/// Margem dos botões de dentro do campo: a `TextBoxInnerButtonMargin` do WinUI.
pub(crate) const INNER_MARGIN: Insets = Insets {
    top: 4.0,
    right: 4.0,
    bottom: 4.0,
    left: 0.0,
};

/// O que o botão faz.
#[derive(Clone)]
pub(crate) enum FieldAction {
    /// Enquanto está pressionado: `true` ao pressionar e `false` ao soltar (revelar a senha).
    Hold(Rc<dyn Fn(bool)>),
    /// Ao soltar sobre o botão (limpar).
    Click(Rc<dyn Fn()>),
    /// Ao pressionar e de novo a cada [`SPIN_REPEAT`] enquanto continua pressionado
    /// (incremento).
    Repeat(Rc<dyn Fn()>),
}

/// Botão sem foco dentro de um campo. O foco fica no campo: tocar no botão não o tira.
pub(crate) struct FieldButton {
    common: Common,
    label: String,
    glyph: FieldGlyph,
    large: bool,
    margin: Insets,
    enabled: Prop<bool>,
    action: FieldAction,
    hovered: bool,
    /// Pressionado; `true` enquanto o ponteiro continua sobre o botão.
    pressed: Option<bool>,
    elapsed: Duration,
    density: Density,
}

impl FieldButton {
    /// Botão com o ícone `glyph`, o nome `label` (o que o inspetor mostra) e o id `id`, que
    /// aparece enquanto `shown` disser que sim. Ocupa `width` px no fim do campo, com a
    /// margem [`INNER_MARGIN`] dentro dessa largura.
    pub(crate) fn new(
        id: Option<String>,
        label: impl Into<String>,
        glyph: FieldGlyph,
        width: f64,
        shown: Rc<dyn Fn() -> bool>,
        action: FieldAction,
    ) -> Self {
        let mut common = Common::generated(id);
        common.visible = Some(Prop::Dynamic(shown));
        common.width = Some(width);
        Self {
            common,
            label: label.into(),
            glyph,
            large: false,
            margin: INNER_MARGIN,
            enabled: Prop::Value(true),
            action,
            hovered: false,
            pressed: None,
            elapsed: Duration::ZERO,
            density: Density::Standard,
        }
    }

    /// Botão do popup de incremento: `size`×`size`, sem margem e com ícone de 16.
    pub(crate) fn large(mut self, size: f64) -> Self {
        self.large = true;
        self.margin = Insets::default();
        self.common.width = Some(size);
        self.common.height = Some(size);
        self
    }

    /// Troca a margem ([`FieldButtonVisual::margin`]).
    pub(crate) fn margin(mut self, margin: Insets) -> Self {
        self.margin = margin;
        self
    }

    /// Se está habilitado (fixo ou reativo).
    pub(crate) fn enabled(mut self, enabled: Prop<bool>) -> Self {
        self.enabled = enabled;
        self
    }

    fn is_enabled(&self) -> bool {
        self.enabled.get()
    }

    /// Solta o botão: o `Hold` volta a esconder.
    fn release(&mut self) {
        if self.pressed.take().is_some()
            && let FieldAction::Hold(hold) = &self.action
        {
            hold(false);
        }
    }
}

impl Widget for FieldButton {
    fn kind(&self) -> &'static str {
        "field_button"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        let state = if !self.is_enabled() {
            ButtonState::Disabled
        } else if self.pressed == Some(true) {
            ButtonState::Pressed
        } else if self.hovered || self.pressed.is_some() {
            ButtonState::Hover
        } else {
            ButtonState::Rest
        };
        Look::FieldButton(FieldButtonVisual {
            glyph: self.glyph,
            state,
            large: self.large,
            margin: self.margin,
        })
    }

    fn handle(&mut self, event: &Event, _: &mut EventCx) -> bool {
        if !self.is_enabled() {
            self.hovered = false;
            self.release();
            return false;
        }
        match *event {
            Event::PointerMove { kind, inside } => {
                self.hovered = inside && kind.hovers();
                if let Some(was) = &mut self.pressed {
                    *was = inside;
                }
                true
            }
            Event::PointerDown { kind, .. } => {
                self.hovered = kind.hovers();
                self.pressed = Some(true);
                self.elapsed = Duration::ZERO;
                match &self.action {
                    FieldAction::Hold(hold) => hold(true),
                    FieldAction::Repeat(step) => step(),
                    FieldAction::Click(_) => {}
                }
                true
            }
            Event::PointerUp { kind, inside } => {
                self.hovered = inside && kind.hovers();
                if self.pressed.is_none() {
                    return false;
                }
                self.release();
                if inside && let FieldAction::Click(click) = &self.action {
                    click();
                }
                true
            }
            Event::PointerCancel => {
                self.hovered = false;
                self.release();
                true
            }
            _ => false,
        }
    }

    fn interactive(&self) -> bool {
        true
    }

    /// O alvo cresce até o mínimo da densidade (44×44, ou 24×24 na compacta), centrado no
    /// botão.
    fn hit_rect(&self, rect: Rect) -> Rect {
        let min = self.density.min_target();
        let dx = ((min - rect.width()) / 2.0).max(0.0);
        let dy = ((min - rect.height()) / 2.0).max(0.0);
        rect.inflate(dx, dy)
    }

    fn set_density(&mut self, density: Density) {
        self.density = density;
    }

    fn enabled(&self) -> bool {
        self.is_enabled()
    }

    fn text(&self) -> Option<String> {
        Some(self.label.clone())
    }

    fn animating(&self) -> bool {
        matches!(self.action, FieldAction::Repeat(_))
    }

    fn ticking(&self) -> bool {
        self.pressed.is_some()
    }

    fn tick(&mut self, dt: Duration) {
        let FieldAction::Repeat(step) = &self.action else {
            return;
        };
        if self.pressed.is_none() {
            return;
        }
        if !self.is_enabled() {
            // Chegou ao limite: para de repetir.
            self.release();
            return;
        }
        self.elapsed += dt;
        while self.elapsed >= SPIN_REPEAT {
            self.elapsed -= SPIN_REPEAT;
            step();
        }
    }
}

/// Ícone fixo no fim do campo, sem ação: o indicador do `NumberBox` com os botões de
/// incremento num popup (`PopupIndicator` do WinUI).
pub(crate) struct FieldIndicator {
    common: Common,
    glyph: FieldGlyph,
    margin: Insets,
}

impl FieldIndicator {
    /// Ícone `glyph` com `width` px de largura no fim do campo, `margin` incluída.
    pub(crate) fn new(glyph: FieldGlyph, width: f64, margin: Insets) -> Self {
        let mut common = Common::generated(None);
        common.width = Some(width);
        Self {
            common,
            glyph,
            margin,
        }
    }
}

impl Widget for FieldIndicator {
    fn kind(&self) -> &'static str {
        "field_indicator"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::FieldButton(FieldButtonVisual {
            glyph: self.glyph,
            state: ButtonState::Rest,
            large: false,
            margin: self.margin,
        })
    }
}

impl fmt::Debug for FieldIndicator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FieldIndicator")
            .field("glyph", &self.glyph)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for FieldButton {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FieldButton")
            .field("common", &self.common)
            .field("glyph", &self.glyph)
            .field("pressed", &self.pressed)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use kurbo::Point;

    use super::*;
    use crate::cx::{FixedMetrics, MemoryClipboard};
    use crate::event::{Modifiers, PointerKind::Mouse};

    fn send(w: &mut dyn Widget, event: Event) -> bool {
        let mut metrics = FixedMetrics::default();
        let mut clipboard = MemoryClipboard::default();
        let mut cx = EventCx::new(
            Rect::new(0.0, 0.0, 44.0, 30.0),
            Point::ZERO,
            Modifiers::NONE,
            &mut metrics,
            &mut clipboard,
        );
        w.handle(&event, &mut cx)
    }

    const DOWN: Event = Event::PointerDown {
        kind: Mouse,
        clicks: 1,
    };

    fn up(inside: bool) -> Event {
        Event::PointerUp {
            kind: Mouse,
            inside,
        }
    }

    #[test]
    fn segurar_revela_e_soltar_esconde() {
        let shown = Rc::new(Cell::new(false));
        let s = Rc::clone(&shown);
        let mut b = FieldButton::new(
            None,
            "Mostrar senha",
            FieldGlyph::Reveal,
            32.0,
            Rc::new(|| true),
            FieldAction::Hold(Rc::new(move |on| s.set(on))),
        );
        send(&mut b, DOWN);
        assert!(shown.get());
        send(&mut b, up(false));
        assert!(!shown.get());
        send(&mut b, DOWN);
        send(&mut b, Event::PointerCancel);
        assert!(!shown.get(), "cancelar também esconde");
    }

    #[test]
    fn repete_a_cada_250_ms_enquanto_pressionado() {
        let count = Rc::new(Cell::new(0));
        let c = Rc::clone(&count);
        let mut b = FieldButton::new(
            None,
            "Aumentar",
            FieldGlyph::Up,
            40.0,
            Rc::new(|| true),
            FieldAction::Repeat(Rc::new(move || c.set(c.get() + 1))),
        );
        assert!(b.animating());
        assert!(!b.ticking());
        send(&mut b, DOWN);
        assert_eq!(count.get(), 1, "age ao pressionar");
        assert!(b.ticking());
        b.tick(Duration::from_millis(200));
        assert_eq!(count.get(), 1, "espera o Delay do RepeatButton");
        b.tick(Duration::from_millis(550));
        assert_eq!(count.get(), 4);
        send(&mut b, up(true));
        assert!(!b.ticking());
        b.tick(Duration::from_millis(500));
        assert_eq!(count.get(), 4, "solto, para");
    }

    #[test]
    fn clique_so_ao_soltar_em_cima() {
        let count = Rc::new(Cell::new(0));
        let c = Rc::clone(&count);
        let mut b = FieldButton::new(
            None,
            "Limpar",
            FieldGlyph::Clear,
            40.0,
            Rc::new(|| true),
            FieldAction::Click(Rc::new(move || c.set(c.get() + 1))),
        );
        send(&mut b, DOWN);
        send(&mut b, up(false));
        assert_eq!(count.get(), 0);
        send(&mut b, DOWN);
        send(&mut b, up(true));
        assert_eq!(count.get(), 1);
    }

    #[test]
    fn alvo_de_toque_tem_44_px() {
        let b = FieldButton::new(
            None,
            "Limpar",
            FieldGlyph::Clear,
            40.0,
            Rc::new(|| true),
            FieldAction::Click(Rc::new(|| {})),
        );
        let hit = b.hit_rect(Rect::new(100.0, 0.0, 140.0, 32.0));
        assert_eq!(hit, Rect::new(98.0, -6.0, 142.0, 38.0));
    }
}
