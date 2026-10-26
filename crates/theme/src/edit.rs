//! Campo de texto no visual do `TextBox` do WinUI.

use std::cell::RefCell;
use std::rc::Rc;

use tomni_ui_core::{
    Density, EditState, EditVisual, EditorPoints, EditorText, InfoBarVisual, RichTextVisual,
    TextMetrics,
};
use tomni_ui_render::kurbo::{Point, Rect, RoundedRect, Shape};
use tomni_ui_render::peniko::Gradient;
use tomni_ui_render::{Canvas, CanvasExt, PATH_TOLERANCE, Paint};
use tomni_ui_text::{Fonts, TextLayout, TextStyle};

use crate::{FluentDrawer, metrics, ring};

/// Medidas de texto do `Edit` no [`FluentDrawer`]: a fonte do corpo de texto e o padding do
/// `TextBox`.
#[derive(Debug)]
pub struct FluentTextMetrics {
    fonts: Rc<RefCell<Fonts>>,
    editors: Rc<RefCell<crate::text_editor::EditorCache>>,
}

impl FluentTextMetrics {
    /// Medidas com um banco de fontes próprio. O `FluentDrawer` cria as suas com o banco
    /// dele, em [`Drawer::text_metrics`](crate::Drawer::text_metrics).
    pub fn new() -> Self {
        Self::shared(Rc::new(RefCell::new(Fonts::new())), Rc::default())
    }

    pub(crate) fn shared(
        fonts: Rc<RefCell<Fonts>>,
        editors: Rc<RefCell<crate::text_editor::EditorCache>>,
    ) -> Self {
        Self { fonts, editors }
    }
}

impl Default for FluentTextMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl TextMetrics for FluentTextMetrics {
    fn carets(&mut self, text: &str) -> Vec<(usize, f64)> {
        let layout = self.fonts.borrow_mut().layout(text, TextStyle::BODY, None);
        layout
            .carets()
            .into_iter()
            .map(|(byte, x)| (byte, f64::from(x)))
            .collect()
    }

    fn text_area(&self, rect: Rect, density: Density) -> Rect {
        text_area(rect, density)
    }

    fn caret_points(&mut self, text: &str) -> Vec<(usize, f64, usize)> {
        let layout = self.fonts.borrow_mut().layout(text, TextStyle::BODY, None);
        layout
            .caret_points(text)
            .into_iter()
            .map(|(byte, x, line)| (byte, f64::from(x), line))
            .collect()
    }

    fn line_height(&self) -> f64 {
        f64::from(TextStyle::BODY.line_height)
    }

    fn multiline_area(&self, rect: Rect, density: Density) -> Rect {
        multiline_area(rect, density)
    }

    fn editor_points(&mut self, text: &EditorText) -> Rc<EditorPoints> {
        crate::text_editor::editor_points(&self.fonts, &self.editors, text)
    }

    fn editor_area(&self, rect: Rect, density: Density) -> Rect {
        multiline_area(rect, density)
    }

    fn rich_links(&mut self, visual: &RichTextVisual, width: Option<f64>) -> Vec<Vec<Rect>> {
        crate::rich_text::links(&self.fonts, visual, width)
    }

    fn info_bar_action(&mut self, visual: &InfoBarVisual, width: f64) -> Option<Rect> {
        crate::info_bar::action_rect(&self.fonts, visual, width)
    }
}

/// Área do texto: o retângulo sem a borda de 1 px e sem `TextControlThemePadding` (o do
/// `Compact.xaml` na densidade compacta). Num campo mais alto que o natural, o texto desce para
/// ficar centralizado.
fn text_area(rect: Rect, density: Density) -> Rect {
    let (left, top, right, bottom) = match density {
        Density::Standard => metrics::TEXT_CONTROL_PADDING,
        Density::Compact => metrics::TEXT_CONTROL_PADDING_COMPACT,
    };
    let border = metrics::BORDER;
    let offset = crate::center_offset_from(rect, crate::control_height(density));
    Rect::new(
        rect.x0 + border + left,
        rect.y0 + border + top + offset,
        rect.x1 - border - right,
        rect.y1 - border - bottom,
    )
}

/// Área do texto de um campo de várias linhas: como a de uma linha, mas o texto começa no topo,
/// sem descer para o meio.
pub(crate) fn multiline_area(rect: Rect, density: Density) -> Rect {
    let (left, top, right, bottom) = match density {
        Density::Standard => metrics::TEXT_CONTROL_PADDING,
        Density::Compact => metrics::TEXT_CONTROL_PADDING_COMPACT,
    };
    let border = metrics::BORDER;
    Rect::new(
        rect.x0 + border + left,
        rect.y0 + border + top,
        rect.x1 - border - right,
        rect.y1 - border - bottom,
    )
}

/// Área do texto de um campo com botões à direita: os `visual.trailing` px da ponta direita
/// ficam para os botões.
fn field_area(rect: Rect, visual: &EditVisual) -> Rect {
    let x1 = (rect.x1 - visual.trailing).max(rect.x0);
    let rect = Rect::new(rect.x0, rect.y0, x1, rect.y1);
    if visual.multiline {
        multiline_area(rect, visual.density)
    } else {
        text_area(rect, visual.density)
    }
}

/// Retângulo de dentro da borda; com foco, a linha de baixo tem 2 px.
pub(crate) fn inner(rect: Rect, state: EditState) -> Rect {
    let border = metrics::BORDER;
    let bottom = if state == EditState::Focused {
        metrics::TEXT_CONTROL_FOCUSED_BOTTOM
    } else {
        border
    };
    Rect::new(
        rect.x0 + border,
        rect.y0 + border,
        rect.x1 - border,
        rect.y1 - bottom,
    )
}

/// Preenchimento do campo em cada estado.
pub(crate) fn fill(t: &crate::Tokens, state: EditState) -> tomni_ui_render::peniko::Color {
    match state {
        EditState::Rest => t.control_fill_default,
        EditState::Hover => t.control_fill_secondary,
        EditState::Focused => t.control_fill_input_active,
        EditState::Disabled => t.control_fill_disabled,
    }
}

/// Texto do campo já posicionado.
struct Placed {
    layout: TextLayout,
    /// `(byte, x, linha)`; num campo de uma linha, a linha é sempre 0.
    carets: Vec<(usize, f32, usize)>,
    /// Canto superior esquerdo do texto, já com a rolagem.
    origin: Point,
}

impl Placed {
    fn new(fonts: &RefCell<Fonts>, rect: Rect, visual: &EditVisual) -> Self {
        let area = field_area(rect, visual);
        let layout = fonts
            .borrow_mut()
            .layout(&visual.text, TextStyle::BODY, None);
        let line = f64::from(TextStyle::BODY.line_height);
        let (carets, scroll_y) = if visual.multiline {
            let carets = layout.caret_points(&visual.text);
            let lines = carets.last().map_or(0, |c| c.2) + 1;
            let max_y = (lines as f64 * line - area.height()).max(0.0);
            (carets, visual.scroll_y.clamp(0.0, max_y))
        } else {
            let carets = layout
                .carets()
                .into_iter()
                .map(|(byte, x)| (byte, x, 0))
                .collect();
            (carets, 0.0)
        };
        let total = carets.iter().map(|c| f64::from(c.1)).fold(0.0, f64::max);
        // O `Edit` rola ao tratar eventos; o tamanho pode ter mudado depois disso.
        let max = (total + metrics::CARET_WIDTH - area.width()).max(0.0);
        let scroll = visual.scroll.clamp(0.0, max);
        Self {
            layout,
            carets,
            origin: Point::new(area.x0 - scroll, area.y0 - scroll_y),
        }
    }

    /// x de `byte` na janela, arredondado para o pixel, e a linha dele.
    fn x(&self, byte: usize) -> (f64, usize) {
        let point = self
            .carets
            .iter()
            .find(|c| c.0 >= byte)
            .or(self.carets.last());
        let (x, line) = point.map_or((0.0, 0), |c| (f64::from(c.1), c.2));
        ((self.origin.x + x).round(), line)
    }

    /// Faixas entre os bytes `start` e `end`, uma por linha.
    fn spans(&self, start: usize, end: usize) -> Vec<Rect> {
        let line = f64::from(TextStyle::BODY.line_height);
        let top = |l: usize| self.origin.y + l as f64 * line;
        let (x0, l0) = self.x(start);
        let (x1, l1) = self.x(end);
        if l0 == l1 {
            return vec![Rect::new(x0, top(l0), x1, top(l0) + line)];
        }
        (l0..=l1)
            .map(|l| {
                let mut bytes = self.carets.iter().filter(|c| c.2 == l);
                let first = bytes.clone().next().map_or(0, |c| c.0);
                let last = bytes.next_back().map_or(0, |c| c.0);
                let (a, _) = self.x(start.max(first));
                let (b, _) = self.x(end.min(last));
                // A quebra de linha selecionada ganha uma sobra, como no WinUI.
                let tail = if l < l1 { 4.0 } else { 0.0 };
                Rect::new(
                    if l == l0 { x0 } else { a },
                    top(l),
                    if l == l1 { x1 } else { b + tail },
                    top(l) + line,
                )
            })
            .collect()
    }

    fn caret(&self, visual: &EditVisual) -> Option<Rect> {
        let caret = visual.caret?;
        let mut rect = self.spans(caret, caret).first().copied()?;
        rect.x1 = rect.x0 + metrics::CARET_WIDTH;
        Some(rect)
    }
}

pub(crate) fn caret_rect(drawer: &FluentDrawer, rect: Rect, visual: &EditVisual) -> Option<Rect> {
    Placed::new(&drawer.fonts, rect, visual).caret(visual)
}

/// Cor do placeholder.
pub(crate) fn placeholder_color(
    t: &crate::Tokens,
    state: EditState,
) -> tomni_ui_render::peniko::Color {
    if state == EditState::Disabled {
        t.text_fill_disabled
    } else {
        t.text_fill_secondary
    }
}

/// Texto visível do campo, para o lint: o placeholder, que é cortado se não couber, ou o valor,
/// que rola.
pub(crate) fn text_info(
    drawer: &FluentDrawer,
    rect: Rect,
    visual: &EditVisual,
) -> Option<crate::TextInfo> {
    let t = &drawer.tokens;
    let background = crate::over(fill(t, visual.state), drawer.backdrop());
    let placeholder = visual.text.is_empty();
    let (text, color) = if placeholder {
        (&visual.placeholder, placeholder_color(t, visual.state))
    } else if visual.state == EditState::Disabled {
        (&visual.text, t.text_fill_disabled)
    } else {
        (&visual.text, t.text_fill_primary)
    };
    if text.is_empty() {
        return None;
    }
    let area = field_area(rect, visual);
    let inner = inner(rect, visual.state);
    let layout = drawer
        .fonts
        .borrow_mut()
        .layout(text, TextStyle::BODY, None);
    let width = f64::from(layout.width());
    let line = if visual.multiline && !placeholder {
        f64::from(layout.height())
    } else {
        f64::from(TextStyle::BODY.line_height)
    };
    Some(crate::TextInfo {
        text: text.clone(),
        color: crate::over(color, background),
        background,
        style: TextStyle::BODY,
        bounds: Rect::new(area.x0, area.y0, area.x0 + width, area.y0 + line),
        clip: placeholder.then(|| Rect::new(area.x0, inner.y0, area.x1, inner.y1)),
    })
}

/// Pinta a moldura do campo (preenchimento, borda e a linha de baixo do foco) e devolve o
/// retângulo de dentro dela.
pub(crate) fn draw_frame(
    t: &crate::Tokens,
    canvas: &mut dyn Canvas,
    rect: Rect,
    state: EditState,
) -> Rect {
    let radius = metrics::CONTROL_CORNER_RADIUS;
    let border = metrics::BORDER;

    // Com foco, a linha de baixo tem 2 px na cor accent.
    let inner = inner(rect, state);
    let bottom = rect.y1 - inner.y1;
    canvas.fill(
        &RoundedRect::from_rect(inner, radius - border),
        fill(t, state),
    );

    let edge = ring(rect, radius, inner, radius - border);
    match state {
        EditState::Disabled => canvas.fill_path(&edge, &Paint::Solid(t.control_stroke_default)),
        EditState::Focused => {
            canvas.fill_path(&edge, &Paint::Solid(t.control_stroke_default));
            canvas.push_clip(&edge);
            canvas.fill(
                &Rect::new(rect.x0, rect.y1 - bottom, rect.x1, rect.y1),
                t.accent_fill_default,
            );
            canvas.pop_clip();
        }
        EditState::Rest | EditState::Hover => {
            // `TextControlElevationBorderBrush`: a linha de baixo mais forte e, nos cantos, uma
            // transição de 1 px para a cor do resto da borda.
            let h = metrics::TEXT_CONTROL_BORDER_HEIGHT;
            let gradient =
                Gradient::new_linear(Point::new(0.0, rect.y1), Point::new(0.0, rect.y1 - h))
                    .with_stops([
                        (0.5, t.control_strong_stroke_default),
                        (1.0, t.control_stroke_default),
                    ]);
            canvas.fill_path(&edge, &Paint::Gradient(gradient));
        }
    }
    inner
}

pub(crate) fn draw(
    drawer: &FluentDrawer,
    canvas: &mut dyn Canvas,
    rect: Rect,
    visual: &EditVisual,
) {
    let t = &drawer.tokens;
    let state = visual.state;
    let inner = draw_frame(t, canvas, rect, state);

    let disabled = state == EditState::Disabled;
    let area = field_area(rect, visual);
    let clip = Rect::new(area.x0, inner.y0, area.x1, inner.y1).to_path(PATH_TOLERANCE);
    canvas.push_clip(&clip);

    if visual.text.is_empty() {
        let color = placeholder_color(t, state);
        let placeholder =
            drawer
                .fonts
                .borrow_mut()
                .layout(&visual.placeholder, TextStyle::BODY, None);
        placeholder.draw(canvas, area.origin(), &Paint::Solid(color));
    }

    let placed = Placed::new(&drawer.fonts, rect, visual);
    let color = if disabled {
        t.text_fill_disabled
    } else {
        t.text_fill_primary
    };
    placed
        .layout
        .draw(canvas, placed.origin, &Paint::Solid(color));

    if let Some((start, end)) = visual.selection {
        for span in placed.spans(start, end) {
            canvas.fill(&span, t.accent_selected_text_background);
            canvas.push_clip(&span.to_path(PATH_TOLERANCE));
            placed.layout.draw(
                canvas,
                placed.origin,
                &Paint::Solid(t.text_on_accent_selected_text),
            );
            canvas.pop_clip();
        }
    }

    if let Some((start, end)) = visual.composing {
        for span in placed.spans(start, end) {
            let y = span.y0 + metrics::COMPOSITION_UNDERLINE;
            canvas.fill(&Rect::new(span.x0, y, span.x1, y + 1.0), color);
        }
    }

    if visual.selection.is_none()
        && let Some(caret) = placed.caret(visual)
    {
        canvas.fill(&caret, t.text_fill_primary);
    }

    canvas.pop_clip();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Drawer, Theme};

    #[test]
    fn area_desconta_borda_e_padding() {
        let metrics = FluentTextMetrics::new();
        let area = metrics.text_area(Rect::new(0.0, 0.0, 200.0, 32.0), Density::Standard);
        assert_eq!(area, Rect::new(11.0, 6.0, 193.0, 25.0));
    }

    #[test]
    fn area_compacta_usa_o_padding_do_compact_xaml() {
        let metrics = FluentTextMetrics::new();
        let area = metrics.text_area(Rect::new(0.0, 0.0, 200.0, 24.0), Density::Compact);
        assert_eq!(area, Rect::new(3.0, 3.0, 193.0, 22.0));
        // Mais alto que 24, o texto desce para ficar no meio.
        let alto = metrics.text_area(Rect::new(0.0, 0.0, 200.0, 32.0), Density::Compact);
        assert_eq!(alto.y0, 7.0);
    }

    #[test]
    fn carets_respeitam_clusters() {
        let mut metrics = FluentTextMetrics::new();
        let carets = metrics.carets("ae\u{301}b");
        let bytes: Vec<usize> = carets.iter().map(|c| c.0).collect();
        assert_eq!(bytes, [0, 1, 4, 5]);
        assert!(carets.windows(2).all(|w| w[0].1 < w[1].1));
    }

    #[test]
    fn cursor_acompanha_o_texto_e_a_rolagem() {
        let mut drawer = FluentDrawer::new(Theme::Light);
        let rect = Rect::new(0.0, 0.0, 200.0, 32.0);
        let mut visual = EditVisual {
            text: "Ana".into(),
            caret: Some(0),
            state: EditState::Focused,
            ..EditVisual::default()
        };
        let inicio = drawer.caret_rect(rect, &visual).unwrap();
        assert_eq!(inicio, Rect::new(11.0, 6.0, 12.0, 26.0));

        visual.caret = Some(3);
        let fim = drawer.caret_rect(rect, &visual).unwrap();
        assert!(fim.x0 > inicio.x0 + 15.0);

        // A rolagem só vale até onde o texto passa da área: aqui ele cabe e ela é ignorada.
        visual.scroll = 50.0;
        assert_eq!(drawer.caret_rect(rect, &visual).unwrap(), fim);

        visual.caret = None;
        assert_eq!(drawer.caret_rect(rect, &visual), None);
    }
}
