//! `Button`: estados, variantes e a máquina de estados de clique.

use std::fmt;

use tomni_ui_reactive::{Set, Signal};

use crate::cx::EventCx;
use crate::density::Density;
use crate::event::Event;
use crate::flyout::FlyoutContent;
use crate::press::Pressable;
use crate::prop::{IntoProp, Prop};
use crate::symbol::Symbol;
use crate::widget::{Common, Look, Widget, common_builders};

/// Variante visual do botão.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ButtonVariant {
    /// Botão comum.
    #[default]
    Standard,
    /// Ação principal, na cor accent.
    Accent,
    /// Link: só o texto, na cor accent, sem fundo nem borda (o `HyperlinkButton` do WinUI).
    Hyperlink,
}

/// Estado visual do botão.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ButtonState {
    /// Parado.
    #[default]
    Rest,
    /// Ponteiro sobre o botão.
    Hover,
    /// Pressionado (ponteiro ou Espaço).
    Pressed,
    /// Desabilitado.
    Disabled,
}

/// Tudo o que o `Drawer` precisa saber do estado do botão para pintá-lo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ButtonVisual {
    /// Variante.
    pub variant: ButtonVariant,
    /// Estado.
    pub state: ButtonState,
    /// Anel de foco: só com foco vindo do teclado, e nunca desabilitado.
    pub focus_ring: bool,
    /// Ícone de 16 px: antes do rótulo ou, com `icon_only`, sozinho no meio.
    pub icon: Option<Symbol>,
    /// Botão só de ícone ([`Button::icon_only`]): o rótulo não aparece, só vai para a árvore.
    pub icon_only: bool,
    /// Densidade: no compacto, o botão só de ícone tem 32×24 px.
    pub density: Density,
}

/// Lado do ícone dentro de um botão, em pixels.
pub const BUTTON_ICON_SIZE: f64 = 16.0;

/// Espaço entre o ícone e o rótulo de um botão, em pixels.
pub const BUTTON_ICON_SPACING: f64 = 8.0;

/// Lado do botão só de ícone, em pixels: o alvo de toque mínimo do lint.
pub const ICON_BUTTON_SIZE: f64 = 44.0;

/// Largura e altura do botão só de ícone na densidade compacta: as dos botões da faixa de abas
/// do WinUI (`TabViewItemAddButtonWidth` e `TabViewItemAddButtonHeight`).
pub const ICON_BUTTON_SIZE_COMPACT: (f64, f64) = (32.0, 24.0);

/// Botão com rótulo, com ícone e rótulo ou só com ícone.
///
/// ```
/// use tomni_ui_core::{Button, Symbol};
///
/// let enviar = Button::new("Entrar")
///     .id("entrar")
///     .accent()
///     .on_click(|| println!("entrou"));
/// assert_eq!(enviar.current_label(), "Entrar");
///
/// let novo = Button::new("Novo").icon(Symbol::Add).id("novo");
/// let ajustes = Button::icon_only(Symbol::Settings)
///     .label("Configurações")
///     .id("ajustes");
/// assert_eq!(ajustes.current_label(), "Configurações");
/// ```
///
/// Regras (spec, seção Widgets): o clique dispara ao soltar sobre o botão, desde que o press
/// tenha começado nele; Enter dispara ao pressionar e Espaço ao soltar; desabilitado não
/// recebe foco nem eventos; no toque não há hover.
///
/// Com [`Button::icon`], um ícone de 16 px vai 8 px antes do rótulo, na cor do rótulo, como o
/// `Button` do WinUI com um `StackPanel` de `SymbolIcon` e `TextBlock`. O
/// [`Button::icon_only`] é um quadrado de 44 px só com o ícone (32×24 px na densidade
/// compacta): o [`Button::label`] dele não
/// aparece, mas vai para a árvore e vira a dica (sem `.label`, o lint avisa com
/// `missing-label`).
pub struct Button {
    common: Common,
    label: Prop<String>,
    icon: Option<Symbol>,
    icon_only: bool,
    variant: ButtonVariant,
    disabled: Prop<bool>,
    on_click: Option<Box<dyn FnMut()>>,
    flyout: Option<Box<dyn Widget>>,
    flyout_open: Option<Signal<bool>>,
    press: Pressable,
    density: Density,
}

impl Button {
    /// Botão `standard` com o rótulo `label` (texto fixo, closure ou signal).
    pub fn new<M>(label: impl IntoProp<String, M>) -> Self {
        Self {
            common: Common::default(),
            label: label.into_prop(),
            icon: None,
            icon_only: false,
            variant: ButtonVariant::Standard,
            disabled: Prop::Value(false),
            on_click: None,
            flyout: None,
            flyout_open: None,
            press: Pressable::default(),
            density: Density::Standard,
        }
    }

    /// Botão só com o ícone `symbol`, num quadrado de 44 px (32×24 px na densidade compacta). Dê um nome com [`Button::label`]:
    /// ele vai para a árvore e para a dica.
    pub fn icon_only(symbol: Symbol) -> Self {
        let mut button = Self::new(String::new());
        button.icon = Some(symbol);
        button.icon_only = true;
        button
    }

    /// Ícone de 16 px antes do rótulo.
    pub fn icon(mut self, symbol: Symbol) -> Self {
        self.icon = Some(symbol);
        self
    }

    /// Rótulo (fixo, closure ou signal). No [`Button::icon_only`], é o nome do botão: não
    /// aparece, mas vai para a árvore e, sem `.tooltip`, vira a dica. Num botão com texto,
    /// troca o texto do `Button::new`.
    pub fn label<M>(mut self, label: impl IntoProp<String, M>) -> Self {
        self.label = label.into_prop();
        if self.icon_only && self.common.tooltip.is_none() {
            self.common.tooltip = Some(self.label.clone());
        }
        self
    }

    /// Variante `accent`, para a ação principal.
    pub fn accent(mut self) -> Self {
        self.variant = ButtonVariant::Accent;
        self
    }

    /// Variante link: só o texto, na cor accent, para ações leves como "Esqueci a senha". O
    /// fundo aparece no hover e no press.
    pub fn hyperlink(mut self) -> Self {
        self.variant = ButtonVariant::Hyperlink;
        self
    }

    /// Id derivado do id do grupo que monta o botão (a `PropertyGrid`).
    pub(crate) fn generated_id(mut self, id: Option<String>) -> Self {
        self.common = Common::generated(id);
        self
    }

    /// Desabilita o botão (valor fixo, closure ou signal).
    pub fn disabled<M>(mut self, disabled: impl IntoProp<bool, M>) -> Self {
        self.disabled = disabled.into_prop();
        self
    }

    /// Ação do clique.
    pub fn on_click(mut self, f: impl FnMut() + 'static) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }

    /// Abre `flyout` (um [`Flyout`](crate::Flyout) ou um [`MenuFlyout`](crate::MenuFlyout))
    /// no clique, embaixo do botão. O `on_click`, se houver, roda também.
    pub fn flyout(mut self, flyout: impl FlyoutContent) -> Self {
        self.flyout_open = Some(flyout.open_signal());
        self.flyout = Some(Box::new(flyout));
        self
    }

    /// Id definido com `.id()`.
    pub fn widget_id(&self) -> Option<&str> {
        self.common.id.as_deref()
    }

    /// Rótulo atual (no botão só de ícone, o nome).
    pub fn current_label(&self) -> String {
        self.label.get()
    }

    /// Variante.
    pub fn variant(&self) -> ButtonVariant {
        self.variant
    }

    /// Se está desabilitado agora.
    pub fn is_disabled(&self) -> bool {
        self.disabled.get()
    }

    /// Se pode receber foco agora.
    pub fn is_focusable(&self) -> bool {
        !self.is_disabled()
    }

    /// Se está com o foco.
    pub fn is_focused(&self) -> bool {
        self.press.is_focused()
    }

    /// Estado para o `Drawer`.
    pub fn visual(&self) -> ButtonVisual {
        let (state, focus_ring) = if self.is_disabled() {
            (ButtonState::Disabled, false)
        } else {
            (self.press.state(), self.press.focus_ring())
        };
        ButtonVisual {
            variant: self.variant,
            state,
            focus_ring,
            icon: self.icon,
            icon_only: self.icon_only,
            density: self.density,
        }
    }

    /// Trata um evento. Devolve `true` se o botão o consumiu.
    ///
    /// Desabilitado, ignora tudo e esquece hover, press e foco.
    pub fn handle(&mut self, event: &Event) -> bool {
        if self.is_disabled() {
            self.press.reset();
            return false;
        }
        let (consumed, clicked) = self.press.handle(event, true);
        if clicked {
            self.click();
        }
        consumed
    }

    fn click(&mut self) {
        if let Some(open) = self.flyout_open {
            open.set(true);
        }
        if let Some(f) = &mut self.on_click {
            f();
        }
    }
}

common_builders!(Button);

impl Widget for Button {
    fn kind(&self) -> &'static str {
        "button"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn set_density(&mut self, density: Density) {
        self.density = density;
    }

    fn look(&self) -> Look {
        Look::Button {
            label: self.current_label(),
            visual: self.visual(),
        }
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        self.flyout.take().into_iter().collect()
    }

    fn handle(&mut self, event: &Event, _: &mut EventCx) -> bool {
        Button::handle(self, event)
    }

    fn interactive(&self) -> bool {
        true
    }

    fn focusable(&self) -> bool {
        self.is_focusable()
    }

    fn enabled(&self) -> bool {
        !self.is_disabled()
    }
}

impl fmt::Debug for Button {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Button")
            .field("common", &self.common)
            .field("label", &self.label)
            .field("icon", &self.icon)
            .field("variant", &self.variant)
            .field("disabled", &self.disabled)
            .field("visual", &self.visual())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use tomni_ui_reactive::prelude::*;

    use super::*;
    use crate::event::PointerKind::{Mouse, Touch};
    use crate::event::{FocusSource, Key};

    /// Botão que conta os cliques.
    fn button() -> (Button, Rc<Cell<u32>>) {
        let clicks = Rc::new(Cell::new(0));
        let counter = Rc::clone(&clicks);
        let button = Button::new("Entrar").on_click(move || counter.set(counter.get() + 1));
        (button, clicks)
    }

    fn state(button: &Button) -> ButtonState {
        button.visual().state
    }

    fn send(button: &mut Button, events: &[Event]) {
        for event in events {
            button.handle(event);
        }
    }

    #[test]
    fn builder() {
        let b = Button::new("Entrar").id("entrar").accent();
        assert_eq!(b.widget_id(), Some("entrar"));
        assert_eq!(b.current_label(), "Entrar");
        assert_eq!(b.variant(), ButtonVariant::Accent);
        assert_eq!(
            b.visual(),
            ButtonVisual {
                variant: ButtonVariant::Accent,
                ..ButtonVisual::default()
            }
        );
    }

    #[test]
    fn clique_com_mouse() {
        let (mut b, clicks) = button();
        b.handle(&Event::PointerMove {
            kind: Mouse,
            inside: true,
        });
        assert_eq!(state(&b), ButtonState::Hover);
        b.handle(&Event::PointerDown {
            kind: Mouse,
            clicks: 1,
        });
        assert_eq!(state(&b), ButtonState::Pressed);
        assert_eq!(clicks.get(), 0, "clicou ao pressionar");
        b.handle(&Event::PointerUp {
            kind: Mouse,
            inside: true,
        });
        assert_eq!(clicks.get(), 1);
        assert_eq!(state(&b), ButtonState::Hover);
        b.handle(&Event::PointerMove {
            kind: Mouse,
            inside: false,
        });
        assert_eq!(state(&b), ButtonState::Rest);
    }

    #[test]
    fn soltar_fora_nao_clica() {
        let (mut b, clicks) = button();
        b.handle(&Event::PointerDown {
            kind: Mouse,
            clicks: 1,
        });
        b.handle(&Event::PointerMove {
            kind: Mouse,
            inside: false,
        });
        assert_eq!(
            state(&b),
            ButtonState::Rest,
            "pressionado com o ponteiro fora"
        );
        // Voltar para dentro mostra pressionado de novo.
        b.handle(&Event::PointerMove {
            kind: Mouse,
            inside: true,
        });
        assert_eq!(state(&b), ButtonState::Pressed);
        b.handle(&Event::PointerMove {
            kind: Mouse,
            inside: false,
        });
        b.handle(&Event::PointerUp {
            kind: Mouse,
            inside: false,
        });
        assert_eq!(clicks.get(), 0);
        assert_eq!(state(&b), ButtonState::Rest);
    }

    #[test]
    fn soltar_sem_ter_pressionado_nao_clica() {
        // O press começou em outro widget e o ponteiro foi solto sobre este.
        let (mut b, clicks) = button();
        assert!(!b.handle(&Event::PointerUp {
            kind: Mouse,
            inside: true,
        }));
        assert_eq!(clicks.get(), 0);
    }

    #[test]
    fn cancelar_nao_clica() {
        let (mut b, clicks) = button();
        send(
            &mut b,
            &[
                Event::PointerDown {
                    kind: Mouse,
                    clicks: 1,
                },
                Event::PointerCancel,
                Event::PointerUp {
                    kind: Mouse,
                    inside: true,
                },
            ],
        );
        assert_eq!(clicks.get(), 0);
        assert_eq!(state(&b), ButtonState::Hover);
    }

    #[test]
    fn toque_nao_tem_hover() {
        let (mut b, clicks) = button();
        b.handle(&Event::PointerMove {
            kind: Touch,
            inside: true,
        });
        assert_eq!(state(&b), ButtonState::Rest);
        b.handle(&Event::PointerDown {
            kind: Touch,
            clicks: 1,
        });
        assert_eq!(state(&b), ButtonState::Pressed);
        b.handle(&Event::PointerUp {
            kind: Touch,
            inside: true,
        });
        assert_eq!(clicks.get(), 1);
        assert_eq!(
            state(&b),
            ButtonState::Rest,
            "hover depois de soltar o dedo"
        );
    }

    #[test]
    fn enter_clica_ao_pressionar_e_espaco_ao_soltar() {
        let (mut b, clicks) = button();
        b.handle(&Event::FocusIn(FocusSource::Keyboard));
        b.handle(&Event::KeyDown(Key::Enter));
        assert_eq!(clicks.get(), 1);

        b.handle(&Event::KeyDown(Key::Space));
        assert_eq!(state(&b), ButtonState::Pressed);
        assert_eq!(clicks.get(), 1);
        b.handle(&Event::KeyUp(Key::Space));
        assert_eq!(clicks.get(), 2);
        assert_eq!(state(&b), ButtonState::Rest);
    }

    #[test]
    fn teclas_sem_foco_nao_clicam() {
        let (mut b, clicks) = button();
        assert!(!b.handle(&Event::KeyDown(Key::Enter)));
        assert!(!b.handle(&Event::KeyDown(Key::Space)));
        assert!(!b.handle(&Event::KeyUp(Key::Space)));
        assert_eq!(clicks.get(), 0);
    }

    #[test]
    fn perder_o_foco_com_espaco_segurado_nao_clica() {
        let (mut b, clicks) = button();
        send(
            &mut b,
            &[
                Event::FocusIn(FocusSource::Keyboard),
                Event::KeyDown(Key::Space),
                Event::FocusOut,
                Event::KeyUp(Key::Space),
            ],
        );
        assert_eq!(clicks.get(), 0);
        assert_eq!(state(&b), ButtonState::Rest);
    }

    #[test]
    fn anel_de_foco_so_pelo_teclado() {
        let (mut b, _) = button();
        b.handle(&Event::FocusIn(FocusSource::Pointer));
        assert!(b.is_focused());
        assert!(!b.visual().focus_ring);
        b.handle(&Event::FocusOut);
        b.handle(&Event::FocusIn(FocusSource::Keyboard));
        assert!(b.visual().focus_ring);
        b.handle(&Event::FocusOut);
        assert!(!b.visual().focus_ring);
    }

    #[test]
    fn desabilitado_ignora_eventos_e_foco() {
        let (b, clicks) = button();
        let mut b = b.disabled(true);
        assert!(!b.is_focusable());
        for event in [
            Event::FocusIn(FocusSource::Keyboard),
            Event::PointerMove {
                kind: Mouse,
                inside: true,
            },
            Event::PointerDown {
                kind: Mouse,
                clicks: 1,
            },
            Event::PointerUp {
                kind: Mouse,
                inside: true,
            },
            Event::KeyDown(Key::Enter),
        ] {
            assert!(!b.handle(&event), "{event:?} foi consumido");
        }
        assert_eq!(clicks.get(), 0);
        assert!(!b.is_focused());
        assert_eq!(
            b.visual(),
            ButtonVisual {
                state: ButtonState::Disabled,
                ..ButtonVisual::default()
            }
        );
    }

    #[test]
    fn disabled_reativo() {
        let scope = Scope::root();
        scope.run(|| {
            let email = signal(String::new());
            let valido = memo(move || email.get().contains('@'));
            let (b, clicks) = button();
            let mut b = b.accent().disabled(move || !valido.get());
            assert_eq!(state(&b), ButtonState::Disabled);

            email.set("ana@x.com".into());
            assert_eq!(state(&b), ButtonState::Rest);
            send(
                &mut b,
                &[
                    Event::PointerDown {
                        kind: Mouse,
                        clicks: 1,
                    },
                    Event::PointerUp {
                        kind: Mouse,
                        inside: true,
                    },
                ],
            );
            assert_eq!(clicks.get(), 1);

            // Desabilitar no meio do press cancela o clique.
            b.handle(&Event::PointerDown {
                kind: Mouse,
                clicks: 1,
            });
            email.set(String::new());
            b.handle(&Event::PointerUp {
                kind: Mouse,
                inside: true,
            });
            email.set("ana@x.com".into());
            assert_eq!(state(&b), ButtonState::Rest);
            assert_eq!(clicks.get(), 1);
        });
        scope.clear();
    }

    #[test]
    fn rotulo_reativo() {
        let scope = Scope::root();
        scope.run(|| {
            let n = signal(0);
            let b = Button::new(move || format!("Cliques: {}", n.get()));
            n.set(3);
            assert_eq!(b.current_label(), "Cliques: 3");
        });
        scope.clear();
    }

    #[test]
    fn icone_e_so_icone() {
        let b = Button::new("Novo").icon(Symbol::Add);
        assert_eq!(b.visual().icon, Some(Symbol::Add));
        assert!(!b.visual().icon_only);
        assert!(b.common().tooltip.is_none());

        let b = Button::icon_only(Symbol::Settings);
        assert_eq!(b.current_label(), "");
        assert!(b.visual().icon_only);
        assert!(b.common().tooltip.is_none());

        // O nome vira a dica, a não ser que ela já exista.
        let b = Button::icon_only(Symbol::Settings).label("Configurações");
        assert_eq!(b.current_label(), "Configurações");
        let dica = b.common().tooltip.as_ref().map(Prop::get);
        assert_eq!(dica.as_deref(), Some("Configurações"));
        let b = Button::icon_only(Symbol::Settings)
            .tooltip("Abrir as configurações")
            .label("Configurações");
        let dica = b.common().tooltip.as_ref().map(Prop::get);
        assert_eq!(dica.as_deref(), Some("Abrir as configurações"));
    }
}
