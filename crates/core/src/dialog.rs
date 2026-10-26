//! `ContentDialog`: caixa modal com título, conteúdo e até três botões.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use kurbo::Size;
use tomni_ui_reactive::{Get, Set, Signal};

use crate::button::Button;
use crate::containers::{Column, Row};
use crate::cx::EventCx;
use crate::density::Density;
use crate::event::{Event, Key};
use crate::prop::{IntoProp, Prop};
use crate::scroll_viewer::ScrollViewer;
use crate::text::{Text, TextRamp};
use crate::widget::{Common, Flex, Insets, Layer, Look, Placement, View, Widget, common_builders};

/// `ContentDialogPadding`.
const PADDING: f64 = 24.0;
/// `ContentDialogButtonSpacing`.
const BUTTON_SPACING: f64 = 8.0;
/// Altura dos botões: o alvo de toque mínimo do lint, no lugar dos 32 px do WinUI. Na
/// densidade compacta, os 32 do WinUI.
fn button_height(density: Density) -> f64 {
    match density {
        Density::Standard => 44.0,
        Density::Compact => 32.0,
    }
}
/// `ContentDialogMinWidth` e `ContentDialogMinHeight`.
const MIN_SIZE: Size = Size::new(320.0, 184.0);
/// `ContentDialogMaxWidth` e `ContentDialogMaxHeight`.
const MAX_SIZE: Size = Size::new(548.0, 756.0);

type Action = Rc<RefCell<Box<dyn FnMut()>>>;

/// Botão do rodapé, guardado até a montagem.
struct DialogButton {
    label: Prop<String>,
    action: Option<Action>,
    /// Quando o botão aparece; sem isso, sempre.
    visible: Option<Rc<dyn Fn() -> bool>>,
}

/// Caixa modal: escurece a janela, prende o foco e pede uma decisão.
///
/// ```
/// use tomni_ui_core::{ContentDialog, Text};
/// use tomni_ui_reactive::prelude::*;
///
/// # let scope = Scope::root();
/// # scope.run(|| {
/// let aberto = signal(false);
/// let dialogo = ContentDialog::new("Excluir conta?", aberto)
///     .id("excluir")
///     .content(Text::new("Seus dados serão apagados.").wrap())
///     .primary("Excluir", || println!("excluída"))
///     .close("Cancelar");
/// aberto.set(true); // abre
/// # });
/// # scope.clear();
/// ```
///
/// Abre quando `open` vira `true`, no centro da janela, com o foco no botão primário (ou no
/// primeiro widget que aceita foco). Enquanto está aberta, o resto da janela não recebe nada e
/// o Tab dá a volta dentro dela. Cada botão roda a ação dele e fecha a caixa; Enter, com o foco
/// num widget que não usa a tecla, faz o mesmo que o primário, e Esc fecha sem ação. Ao fechar,
/// o foco volta para onde estava.
///
/// Os botões têm os ids `{id}-primary`, `{id}-secondary` e `{id}-close`. Ponha a caixa em
/// qualquer lugar da tela: ela não ocupa espaço no layout.
pub struct ContentDialog {
    common: Common,
    title: Prop<String>,
    open: Signal<bool>,
    content: Option<Box<dyn Widget>>,
    primary: Option<DialogButton>,
    secondary: Option<DialogButton>,
    close: Option<DialogButton>,
    /// Ação do primário, para o Enter.
    enter: Option<Action>,
    /// Se tem a faixa dos botões (os botões saem daqui na montagem).
    footer: bool,
    density: Density,
}

impl ContentDialog {
    /// Caixa com o título `title` (fixo, closure ou signal), aberta enquanto `open` for `true`.
    pub fn new<M>(title: impl IntoProp<String, M>, open: Signal<bool>) -> Self {
        Self {
            common: Common::default(),
            title: title.into_prop(),
            open,
            content: None,
            primary: None,
            secondary: None,
            close: None,
            enter: None,
            footer: false,
            density: Density::Standard,
        }
    }

    /// Conteúdo, abaixo do título. Chamar de novo troca o anterior.
    pub fn content(mut self, content: impl View) -> Self {
        self.content = Some(Box::new(content));
        self
    }

    /// Botão primário (na cor accent): roda `action` e fecha. Enter faz o mesmo.
    pub fn primary<M>(
        mut self,
        label: impl IntoProp<String, M>,
        action: impl FnMut() + 'static,
    ) -> Self {
        let action: Action = Rc::new(RefCell::new(Box::new(action)));
        self.enter = Some(Rc::clone(&action));
        self.primary = Some(DialogButton {
            label: label.into_prop(),
            action: Some(action),
            visible: None,
        });
        self
    }

    /// Botão secundário: roda `action` e fecha.
    pub fn secondary<M>(
        mut self,
        label: impl IntoProp<String, M>,
        action: impl FnMut() + 'static,
    ) -> Self {
        self.secondary = Some(DialogButton {
            label: label.into_prop(),
            action: Some(Rc::new(RefCell::new(Box::new(action)))),
            visible: None,
        });
        self
    }

    /// Botão de fechar ("Cancelar"): só fecha, como o Esc.
    pub fn close<M>(mut self, label: impl IntoProp<String, M>) -> Self {
        self.close = Some(DialogButton {
            label: label.into_prop(),
            action: None,
            visible: None,
        });
        self
    }

    /// Como [`close`](Self::close), mas o botão roda `action` antes de fechar. O Esc continua
    /// fechando sem ação. Serve a uma terceira escolha além do primário e do secundário.
    pub fn close_with<M>(
        mut self,
        label: impl IntoProp<String, M>,
        action: impl FnMut() + 'static,
    ) -> Self {
        self.close = Some(DialogButton {
            label: label.into_prop(),
            action: Some(Rc::new(RefCell::new(Box::new(action)))),
            visible: None,
        });
        self
    }

    /// Esconde o botão de fechar enquanto `visible` devolver `false` (reativo). Vale depois de
    /// [`close`](Self::close) ou [`close_with`](Self::close_with).
    pub fn close_visible(mut self, visible: impl Fn() -> bool + 'static) -> Self {
        if let Some(close) = &mut self.close {
            close.visible = Some(Rc::new(visible));
        }
        self
    }

    /// Esconde o botão secundário enquanto `visible` devolver `false` (reativo). Vale depois de
    /// [`secondary`](Self::secondary). O Enter e o Esc não mudam.
    pub fn secondary_visible(mut self, visible: impl Fn() -> bool + 'static) -> Self {
        if let Some(secondary) = &mut self.secondary {
            secondary.visible = Some(Rc::new(visible));
        }
        self
    }

    /// Se está aberta agora.
    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    fn has_buttons(&self) -> bool {
        self.footer || self.primary.is_some() || self.secondary.is_some() || self.close.is_some()
    }
}

common_builders!(ContentDialog);

/// Roda a ação e fecha.
fn run(action: Option<&Action>, open: Signal<bool>) {
    open.set(false);
    if let Some(action) = action {
        (action.borrow_mut())();
    }
}

impl Widget for ContentDialog {
    fn kind(&self) -> &'static str {
        "content_dialog"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn visible(&self) -> bool {
        self.common.visible.as_ref().is_none_or(Prop::get) && self.open.get()
    }

    fn title(&self) -> Option<String> {
        Some(self.title.get())
    }

    fn look(&self) -> Look {
        Look::ContentDialog {
            footer: if self.has_buttons() {
                PADDING + button_height(self.density) + PADDING
            } else {
                0.0
            },
        }
    }

    fn flex(&self) -> Option<Flex> {
        Some(Flex {
            padding: Insets::all(1.0),
            ..Flex::default()
        })
    }

    fn set_density(&mut self, density: Density) {
        self.density = density;
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        let id = self.common.id.clone();
        let sub = |suffix: &str| id.as_ref().map(|id| format!("{id}-{suffix}"));

        let title = self.title.clone();
        let mut top = Column::new().padding(PADDING).gap(12).child(
            Text::new(move || title.get())
                .ramp(TextRamp::Subtitle)
                .wrap(),
        );
        if let Some(content) = self.content.take() {
            top = top.boxed(content);
        }
        let mut scroll = ScrollViewer::new().child(top);
        if let Some(id) = sub("rolagem") {
            scroll = scroll.id(id);
        }
        let mut children: Vec<Box<dyn Widget>> = vec![Box::new(scroll)];
        if !self.has_buttons() {
            return children;
        }
        self.footer = true;

        let buttons = [
            ("primary", self.primary.take()),
            ("secondary", self.secondary.take()),
            ("close", self.close.take()),
        ];
        let count = buttons.iter().filter(|(_, b)| b.is_some()).count();
        let mut footer = Row::new()
            .padding(Insets::new(PADDING, PADDING, PADDING - 1.0, PADDING))
            .gap(BUTTON_SPACING);
        if count == 1 {
            // Um botão só fica na metade da direita, como no WinUI.
            footer = footer.child(Column::new().grow(1));
        }
        let open = self.open;
        for (suffix, button) in buttons {
            let Some(DialogButton {
                label,
                action,
                visible,
            }) = button
            else {
                continue;
            };
            let mut b = Button::new(move || label.get())
                .grow(1)
                .height(button_height(self.density))
                .on_click(move || run(action.as_ref(), open));
            if suffix == "primary" {
                b = b.accent();
            }
            if let Some(visible) = visible {
                b = b.visible(move || visible());
            }
            if let Some(id) = sub(suffix) {
                b = b.id(id);
            }
            footer = footer.child(DialogButtonWidget {
                button: b,
                primary: suffix == "primary",
            });
        }
        children.push(Box::new(footer));
        children
    }

    fn handle(&mut self, event: &Event, _: &mut EventCx) -> bool {
        // Chega aqui o Enter que o widget focado não usou.
        if *event == Event::KeyDown(Key::Enter)
            && let Some(enter) = &self.enter
        {
            run(Some(enter), self.open);
            return true;
        }
        false
    }

    fn layer(&self) -> Option<Layer> {
        Some(Layer {
            modal: true,
            light_dismiss: false,
            placement: Placement::Center,
            min_size: MIN_SIZE,
            max_size: MAX_SIZE,
            ..Layer::default()
        })
    }

    fn set_open(&mut self, open: bool) {
        self.open.set(open);
    }
}

impl fmt::Debug for ContentDialog {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ContentDialog")
            .field("common", &self.common)
            .field("title", &self.title)
            .finish_non_exhaustive()
    }
}

/// Botão do rodapé: um `Button` que, no primário, recebe o foco quando a caixa abre.
struct DialogButtonWidget {
    button: Button,
    primary: bool,
}

impl Widget for DialogButtonWidget {
    fn kind(&self) -> &'static str {
        "button"
    }

    fn common(&self) -> &Common {
        self.button.common()
    }

    fn look(&self) -> Look {
        self.button.look()
    }

    fn handle(&mut self, event: &Event, cx: &mut EventCx) -> bool {
        Widget::handle(&mut self.button, event, cx)
    }

    fn interactive(&self) -> bool {
        true
    }

    fn focusable(&self) -> bool {
        self.button.focusable()
    }

    fn enabled(&self) -> bool {
        self.button.enabled()
    }

    fn autofocus(&self) -> bool {
        self.primary
    }
}
