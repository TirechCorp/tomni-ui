//! `InfoBar`: barra de aviso com severidade, título, mensagem e botão de fechar opcional.

use std::fmt;

use kurbo::{Point, Rect};

use crate::button::ButtonState;
use crate::cx::EventCx;
use crate::event::{Event, FocusSource, Key};
use crate::press::Pressable;
use crate::prop::{IntoProp, Prop};
use crate::widget::{Common, Look, Widget, common_builders};

/// Severidade de uma [`InfoBar`]: define a cor do fundo e o ícone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum InfoBarSeverity {
    /// Informação neutra (padrão).
    #[default]
    Informational,
    /// Deu certo.
    Success,
    /// Atenção: algo pode dar errado.
    Warning,
    /// Deu errado.
    Error,
}

impl InfoBarSeverity {
    /// Nome em minúsculas (`informational`), usado na árvore do `tomni-ui tree`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Informational => "informational",
            Self::Success => "success",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

/// Tudo o que o `Drawer` precisa saber da barra para pintá-la.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct InfoBarVisual {
    /// Severidade.
    pub severity: InfoBarSeverity,
    /// Título, em semibold.
    pub title: String,
    /// Mensagem, ao lado do título ou abaixo dele se não couber.
    pub message: String,
    /// Se tem o botão de fechar.
    pub closable: bool,
    /// Estado do botão de fechar.
    pub close_state: ButtonState,
    /// Anel de foco no botão de fechar.
    pub focus_ring: bool,
    /// Rótulo do botão de ação ([`InfoBar::action`]), se houver.
    pub action: Option<String>,
    /// Id do botão de ação na árvore: `<id da barra>-acao`.
    pub action_id: Option<String>,
    /// Estado do botão de ação.
    pub action_state: ButtonState,
    /// Anel de foco no botão de ação.
    pub action_focus_ring: bool,
}

/// Uma parte da barra que se clica: o botão de ação ou o de fechar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Action,
    Close,
}

/// Onde o ponteiro está, nas partes da barra.
#[derive(Clone, Copy, Debug, Default)]
struct Over {
    action: bool,
    close: bool,
}

/// O botão de ação: o rótulo e o que ele faz.
struct Action {
    label: Prop<String>,
    on_click: Box<dyn FnMut()>,
}

/// Barra de aviso: um ícone na cor da severidade, título e mensagem.
///
/// ```
/// use tomni_ui_core::{InfoBar, InfoBarSeverity};
/// use tomni_ui_reactive::prelude::*;
///
/// # let scope = Scope::root();
/// # scope.run(|| {
/// let aberta = signal(true);
/// let aviso = InfoBar::new(InfoBarSeverity::Error)
///     .id("erro")
///     .title("Erro")
///     .message("Este e-mail já está cadastrado")
///     .visible(aberta)
///     .on_close(move || aberta.set(false));
/// # });
/// # scope.clear();
/// ```
///
/// O botão de fechar só aparece com [`InfoBar::on_close`]; a barra não se esconde sozinha:
/// junte com `.visible(signal)`, como acima. Com o botão, a barra entra no Tab, e Espaço ou
/// Enter fecham.
///
/// [`InfoBar::action`] põe um botão de ação depois da mensagem (o `ActionButton` do WinUI 3),
/// com o id `<id da barra>-acao`. Com o de ação e o de fechar, a barra continua com uma parada
/// no Tab: o Tab e o Shift+Tab passam de um botão ao outro antes de sair dela, e Enter ou
/// Espaço ativam o que tem o foco.
pub struct InfoBar {
    common: Common,
    severity: InfoBarSeverity,
    title: Prop<String>,
    message: Prop<String>,
    on_close: Option<Box<dyn FnMut()>>,
    press: Pressable,
    action: Option<Action>,
    action_press: Pressable,
    /// O botão com o foco do teclado, quando a barra tem o de ação.
    focused: Option<Part>,
    /// O botão em que o ponteiro foi pressionado, quando a barra tem o de ação.
    pressed: Option<Part>,
}

impl InfoBar {
    /// Barra vazia da severidade `severity`.
    pub fn new(severity: InfoBarSeverity) -> Self {
        Self {
            common: Common::default(),
            severity,
            title: Prop::default(),
            message: Prop::default(),
            on_close: None,
            press: Pressable::default(),
            action: None,
            action_press: Pressable::default(),
            focused: None,
            pressed: None,
        }
    }

    /// Título (fixo, closure ou signal).
    pub fn title<M>(mut self, title: impl IntoProp<String, M>) -> Self {
        self.title = title.into_prop();
        self
    }

    /// Mensagem (fixa, closure ou signal).
    pub fn message<M>(mut self, message: impl IntoProp<String, M>) -> Self {
        self.message = message.into_prop();
        self
    }

    /// Mostra o botão de fechar e roda `f` no clique.
    pub fn on_close(mut self, f: impl FnMut() + 'static) -> Self {
        self.on_close = Some(Box::new(f));
        self
    }

    /// Um botão de ação depois da mensagem, como o `ActionButton` do WinUI 3: um botão padrão
    /// com o rótulo `label`, que roda `f` no clique, no Enter e no Espaço. Fica com o id
    /// `<id da barra>-acao` (a barra precisa de `.id()`).
    pub fn action<M>(mut self, label: impl IntoProp<String, M>, f: impl FnMut() + 'static) -> Self {
        self.action = Some(Action {
            label: label.into_prop(),
            on_click: Box::new(f),
        });
        self
    }

    /// Severidade.
    pub fn severity(&self) -> InfoBarSeverity {
        self.severity
    }

    /// Mensagem atual.
    pub fn message_text(&self) -> String {
        self.message.get()
    }

    /// Área que recebe o clique do botão de fechar, numa barra em `rect`: 44 × 44 px no canto
    /// de cima à direita, em volta do botão de 38 × 38 px.
    pub fn close_area(rect: Rect) -> Rect {
        Rect::new(rect.x1 - 46.0, rect.y0 + 2.0, rect.x1 - 2.0, rect.y0 + 46.0)
    }

    /// Estado para o `Drawer`.
    pub fn visual(&self) -> InfoBarVisual {
        InfoBarVisual {
            severity: self.severity,
            title: self.title.get(),
            message: self.message_text(),
            closable: self.on_close.is_some(),
            close_state: self.press.state(),
            focus_ring: self.press.focus_ring(),
            action: self.action.as_ref().map(|a| a.label.get()),
            action_id: self
                .action
                .as_ref()
                .and(self.common.id.as_ref())
                .map(|id| format!("{id}-acao")),
            action_state: self.action_press.state(),
            action_focus_ring: self.action_press.focus_ring(),
        }
    }

    /// Onde o ponteiro está, em `cx`, nas partes da barra.
    fn over(&self, cx: &mut EventCx) -> Over {
        let rect = cx.rect();
        let local = cx.pointer() - rect.origin().to_vec2();
        let action = self.action.as_ref().and_then(|_| {
            let visual = self.visual();
            cx.metrics().info_bar_action(&visual, rect.width())
        });
        Over {
            action: action.is_some_and(|a| a.contains(local)),
            close: self.on_close.is_some()
                && Self::close_area(Rect::from_origin_size(Point::ORIGIN, rect.size()))
                    .contains(local),
        }
    }

    /// As partes que a barra tem, na ordem do Tab.
    fn parts(&self) -> Vec<Part> {
        let mut parts = Vec::new();
        if self.action.is_some() {
            parts.push(Part::Action);
        }
        if self.on_close.is_some() {
            parts.push(Part::Close);
        }
        parts
    }

    fn press_of(&mut self, part: Part) -> &mut Pressable {
        match part {
            Part::Action => &mut self.action_press,
            Part::Close => &mut self.press,
        }
    }

    fn activate(&mut self, part: Part) {
        match part {
            Part::Action => {
                if let Some(action) = &mut self.action {
                    (action.on_click)();
                }
            }
            Part::Close => {
                if let Some(on_close) = &mut self.on_close {
                    on_close();
                }
            }
        }
    }

    /// Entrega `event` a `part`, e roda o que ele faz se foi clicado.
    fn deliver(&mut self, part: Part, event: &Event) -> bool {
        let (consumed, clicked) = self.press_of(part).handle(event, true);
        if clicked {
            self.activate(part);
        }
        consumed
    }

    /// Trata um evento numa barra com botão de ação, que tem duas partes a distinguir.
    fn handle_parts(&mut self, event: &Event, over: Over, shift: bool) -> bool {
        let over_part = |part: Part| match part {
            Part::Action => over.action,
            Part::Close => over.close,
        };
        match *event {
            Event::PointerMove { kind, inside } => {
                for part in self.parts() {
                    let event = Event::PointerMove {
                        kind,
                        inside: inside && over_part(part),
                    };
                    self.press_of(part).handle(&event, true);
                }
                true
            }
            Event::PointerDown { .. } => {
                let target = self.parts().into_iter().find(|&part| over_part(part));
                self.pressed = target;
                if let Some(part) = target {
                    self.deliver(part, event);
                }
                true
            }
            Event::PointerUp { kind, inside } => {
                let Some(pressed) = self.pressed.take() else {
                    return false;
                };
                for part in self.parts() {
                    let event = Event::PointerUp {
                        kind,
                        inside: inside && over_part(part),
                    };
                    if part == pressed {
                        self.deliver(part, &event);
                    } else {
                        self.press_of(part).handle(
                            &Event::PointerMove {
                                kind,
                                inside: inside && over_part(part),
                            },
                            true,
                        );
                    }
                }
                true
            }
            Event::PointerCancel => {
                self.pressed = None;
                for part in self.parts() {
                    self.deliver(part, event);
                }
                true
            }
            Event::KeyDown(Key::Tab) => {
                let parts = self.parts();
                let Some(current) = self
                    .focused
                    .and_then(|f| parts.iter().position(|&p| p == f))
                else {
                    return false;
                };
                let next = if shift {
                    current.checked_sub(1)
                } else {
                    Some(current + 1).filter(|&n| n < parts.len())
                };
                let Some(next) = next.map(|n| parts[n]) else {
                    return false;
                };
                if let Some(old) = self.focused {
                    self.press_of(old).handle(&Event::FocusOut, true);
                }
                self.press_of(next)
                    .handle(&Event::FocusIn(FocusSource::Keyboard), true);
                self.focused = Some(next);
                true
            }
            Event::KeyDown(Key::Enter | Key::Space) | Event::KeyUp(Key::Space) => {
                match self.focused {
                    Some(part) => self.deliver(part, event),
                    None => false,
                }
            }
            Event::FocusIn(source) => {
                let parts = self.parts();
                let pointed = (source == FocusSource::Pointer)
                    .then(|| parts.iter().copied().find(|&part| over_part(part)))
                    .flatten();
                let part = pointed.or(if shift && source == FocusSource::Keyboard {
                    parts.last().copied()
                } else {
                    parts.first().copied()
                });
                self.focused = part;
                if let Some(part) = part {
                    self.press_of(part).handle(event, true);
                }
                true
            }
            Event::FocusOut => {
                self.focused = None;
                for part in self.parts() {
                    self.press_of(part).handle(event, true);
                }
                true
            }
            _ => false,
        }
    }

    /// Trata um evento. Devolve `true` se a barra o consumiu. Sem botão de fechar, não
    /// consome nada.
    pub fn handle(&mut self, event: &Event) -> bool {
        let Some(on_close) = &mut self.on_close else {
            return false;
        };
        let (consumed, clicked) = self.press.handle(event, true);
        if clicked {
            on_close();
        }
        consumed
    }
}

common_builders!(InfoBar);

impl Widget for InfoBar {
    fn kind(&self) -> &'static str {
        "info_bar"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::InfoBar(self.visual())
    }

    fn handle(&mut self, event: &Event, cx: &mut EventCx) -> bool {
        if self.action.is_none() {
            return InfoBar::handle(self, event);
        }
        let over = self.over(cx);
        let shift = cx.modifiers().shift;
        self.handle_parts(event, over, shift)
    }

    fn interactive(&self) -> bool {
        self.on_close.is_some() || self.action.is_some()
    }

    fn hit_rect(&self, rect: Rect) -> Rect {
        if self.on_close.is_none() && self.action.is_some() {
            rect
        } else {
            Self::close_area(rect)
        }
    }

    /// Com o botão de ação, qualquer ponto da barra chega a ela, que decide qual botão foi
    /// atingido; sem ele, só o botão de fechar.
    fn hits(&self, rect: Rect, point: Point) -> bool {
        if self.action.is_some() {
            rect.contains(point)
        } else {
            Self::close_area(rect).contains(point)
        }
    }

    fn focusable(&self) -> bool {
        self.on_close.is_some() || self.action.is_some()
    }

    fn captures_tab(&self) -> bool {
        self.on_close.is_some() && self.action.is_some()
    }

    fn part_ids(&self) -> Vec<String> {
        self.visual().action_id.into_iter().collect()
    }

    fn text(&self) -> Option<String> {
        Some(self.message_text())
    }
}

impl fmt::Debug for InfoBar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InfoBar")
            .field("common", &self.common)
            .field("visual", &self.visual())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;
    use crate::event::{FocusSource, Key};

    #[test]
    fn sem_on_close_nao_e_interativa() {
        let mut bar = InfoBar::new(InfoBarSeverity::Success).title("Pronto");
        assert!(!bar.interactive());
        assert!(!bar.focusable());
        assert!(!bar.handle(&Event::KeyDown(Key::Enter)));
        assert!(!bar.visual().closable);
        assert_eq!(bar.visual().title, "Pronto");
    }

    #[test]
    fn enter_e_espaco_fecham() {
        let closes = Rc::new(Cell::new(0));
        let counter = Rc::clone(&closes);
        let mut bar = InfoBar::new(InfoBarSeverity::Warning)
            .message("Atenção")
            .on_close(move || counter.set(counter.get() + 1));
        assert!(bar.focusable());
        assert_eq!(bar.text().as_deref(), Some("Atenção"));
        bar.handle(&Event::FocusIn(FocusSource::Keyboard));
        assert!(bar.visual().focus_ring);
        bar.handle(&Event::KeyDown(Key::Enter));
        bar.handle(&Event::KeyDown(Key::Space));
        bar.handle(&Event::KeyUp(Key::Space));
        assert_eq!(closes.get(), 2);
    }

    #[test]
    fn area_do_fechar_tem_44_px() {
        let area = InfoBar::close_area(Rect::new(10.0, 20.0, 410.0, 68.0));
        assert_eq!(area, Rect::new(364.0, 22.0, 408.0, 66.0));
        assert_eq!(area.size(), kurbo::Size::new(44.0, 44.0));
    }
}
