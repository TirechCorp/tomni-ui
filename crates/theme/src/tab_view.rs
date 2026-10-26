//! `TabView` e as abas no visual do WinUI.

use tomni_ui_core::{
    ButtonState, Density, InsertMark, ProgressVisual, Symbol, TAB_ATTENTION_DOT, TAB_ICON_SIZE,
    TAB_MAX_WIDTH, TAB_MIN_WIDTH, TabHeaderVisual, TabStripButtonKind, TabStripButtonVisual,
    TabViewVisual,
};
use tomni_ui_render::kurbo::{
    BezPath, Circle, Point, Rect, RoundedRect, RoundedRectRadii, Shape, Size,
};
use tomni_ui_render::peniko::Color;
use tomni_ui_render::{Canvas, CanvasExt, PATH_TOLERANCE, Paint};
use tomni_ui_text::TextStyle;

use crate::{FluentDrawer, TextInfo, Tokens, icons, metrics};

/// `TabViewItemHeaderPaddingWithoutCloseButton` (8 dos lados), dentro da borda de 1 px.
const PADDING: f64 = 8.0 + metrics::BORDER;
/// `TabViewItemHeaderPadding` no compacto: 8 à esquerda e 4 à direita.
const PADDING_COMPACT: (f64, f64) = (8.0, 4.0);
/// `OverlayCornerRadius`, nos cantos de cima da aba escolhida.
const SELECTED_RADIUS: f64 = 8.0;
/// A barra de destaque da aba escolhida (`.selected_indicator`): a da `NavigationView` de cima
/// do WinUI, deitada.
const INDICATOR: Size = Size::new(16.0, 3.0);
/// O raio da barra.
const INDICATOR_RADIUS: f64 = 1.5;
/// A distância da barra até a borda de baixo da aba.
const INDICATOR_MARGIN: f64 = 2.0;
/// `TabViewItemSeparatorMargin`: o separador para 8 px antes das bordas de cima e de baixo.
const SEPARATOR_MARGIN: f64 = 8.0;
/// No compacto, 4 px: o separador fica com os mesmos 16 px de altura do WinUI.
const SEPARATOR_MARGIN_COMPACT: f64 = 4.0;
/// Fonte do título. O WinUI usa 12 px (`TabViewItemHeaderFontSize`) em abas de 32; aqui as
/// abas têm 44 para o toque, e o título fica no tamanho do corpo.
const STYLE: TextStyle = TextStyle::BODY;
/// Fonte do título no compacto: os 12 px do WinUI.
const STYLE_COMPACT: TextStyle = TextStyle::CAPTION;
/// Fundo dos botões da faixa (`TabViewItemAddButtonWidth` e `Height`), centrado no botão.
const BUTTON_FILL: Size = Size::new(32.0, 24.0);
/// Glifo do fechar e do "+" (`TabViewItemHeaderCloseFontSize`, `TabViewItemAddButtonFontSize`).
const GLYPH: f64 = 12.0;
/// Lado do ícone de um botão `Icon`, o dos botões de ícone dos outros widgets.
const ICON_GLYPH: f64 = 16.0;
/// Triângulo das setas: largura da base e altura, como os da `ScrollBar`.
const ARROW: (f64, f64) = (6.0, 3.5);
/// Linha de onde a aba arrastada vai cair: largura e folga em cima e embaixo. Adaptação: no
/// WinUI as abas abrem espaço; aqui a linha fica na borda da aba, no `AccentFillColorDefault`.
const INSERT_LINE: (f64, f64) = (2.0, 6.0);

fn style(density: Density) -> TextStyle {
    match density {
        Density::Standard => STYLE,
        Density::Compact => STYLE_COMPACT,
    }
}

/// Espaço do título à esquerda e à direita (com o do botão de fechar).
fn padding(visual: TabHeaderVisual) -> (f64, f64) {
    let (left, right) = match visual.density {
        Density::Standard => (PADDING, PADDING),
        Density::Compact => PADDING_COMPACT,
    };
    if visual.closable {
        (left, visual.close_space())
    } else {
        (left, right)
    }
}

/// Onde o ícone cabe, se a aba tem um: 16 px, antes do título e no meio da altura.
fn icon_area(rect: Rect, visual: TabHeaderVisual) -> Rect {
    let (left, _) = padding(visual);
    let y = rect.y0 + ((rect.height() - TAB_ICON_SIZE) / 2.0).floor();
    Rect::new(
        rect.x0 + left,
        y,
        rect.x0 + left + TAB_ICON_SIZE,
        y + TAB_ICON_SIZE,
    )
}

/// Onde o ponto de atenção fica: no canto superior direito do espaço do ícone ou do anel, ou,
/// sem nenhum dos dois, sozinho no começo da aba e no meio da altura.
fn attention_dot(rect: Rect, visual: TabHeaderVisual) -> Rect {
    let area = icon_area(rect, visual);
    let (x, y) = if visual.has_icon_slot() {
        (area.x1 - TAB_ATTENTION_DOT + 2.0, area.y0 - 2.0)
    } else {
        (area.x0, area.center().y - TAB_ATTENTION_DOT / 2.0)
    };
    Rect::new(x, y, x + TAB_ATTENTION_DOT, y + TAB_ATTENTION_DOT)
}

/// Onde o título cabe.
fn label_area(rect: Rect, visual: TabHeaderVisual) -> Rect {
    let (left, right) = padding(visual);
    let left = left + visual.icon_space();
    Rect::new(
        rect.x0 + left,
        rect.y0,
        (rect.x1 - right).max(rect.x0 + left),
        rect.y1,
    )
}

/// Fundo do `TabView`: a faixa das abas fica sem fundo, com uma linha embaixo; o conteúdo,
/// no fundo da aba escolhida.
pub(crate) fn draw_view(canvas: &mut dyn Canvas, rect: Rect, visual: &TabViewVisual, t: &Tokens) {
    let strip = (rect.y0 + visual.strip_height()).min(rect.y1);
    if rect.y1 > strip {
        canvas.fill(
            &Rect::new(rect.x0, strip, rect.x1, rect.y1),
            t.solid_background_tertiary,
        );
    }
    // A aba escolhida pinta por cima desta linha e fica ligada ao conteúdo.
    canvas.fill(
        &Rect::new(rect.x0, strip - metrics::BORDER, rect.x1, strip),
        t.card_stroke_default,
    );
}

/// Fundo e cor do título.
fn colors(t: &Tokens, visual: TabHeaderVisual) -> (Color, Color) {
    if visual.selected {
        let (fill, text) = match visual.selected_colors {
            Some((fill, text)) => (t.resolve(fill), t.resolve(text)),
            None => (t.solid_background_tertiary, t.text_fill_primary),
        };
        let text = if visual.state == ButtonState::Disabled {
            t.text_fill_disabled
        } else {
            text
        };
        return (fill, text);
    }
    match visual.state {
        ButtonState::Rest => (Color::TRANSPARENT, t.text_fill_secondary),
        ButtonState::Hover => (t.layer_on_mica_base_alt_secondary, t.text_fill_secondary),
        ButtonState::Pressed => (t.layer_on_mica_base_alt_default, t.text_fill_tertiary),
        ButtonState::Disabled => (Color::TRANSPARENT, t.text_fill_disabled),
    }
}

fn label_origin(rect: Rect, visual: TabHeaderVisual) -> Point {
    let style = style(visual.density);
    Point::new(
        rect.x0 + padding(visual).0 + visual.icon_space(),
        rect.y0 + ((rect.height() - f64::from(style.line_height)) / 2.0).floor(),
    )
}

/// Tamanho de uma aba: `width`, se o `TabView` dividiu a faixa; senão o do título, entre 100
/// e 240 px.
pub(crate) fn measure_header(
    drawer: &FluentDrawer,
    label: &str,
    visual: TabHeaderVisual,
    width: Option<f64>,
) -> Size {
    let width = width.unwrap_or_else(|| {
        let text = f64::from(
            drawer
                .fonts
                .borrow_mut()
                .layout(label, style(visual.density), None)
                .width(),
        );
        let (left, right) = padding(visual);
        (left + visual.icon_space() + text + right)
            .ceil()
            .clamp(TAB_MIN_WIDTH, TAB_MAX_WIDTH)
    });
    Size::new(width, visual.height())
}

pub(crate) fn draw_header(
    drawer: &FluentDrawer,
    canvas: &mut dyn Canvas,
    rect: Rect,
    label: &str,
    visual: TabHeaderVisual,
) {
    let t = &drawer.tokens;
    let (fill, text) = colors(t, visual);
    let top = RoundedRectRadii::new(SELECTED_RADIUS, SELECTED_RADIUS, 0.0, 0.0);
    if visual.selected {
        // Borda de 1 px em cima e dos lados, sem a de baixo: a aba se abre para o conteúdo.
        let border = metrics::BORDER;
        let inner = Rect::new(
            rect.x0 + border,
            rect.y0 + border,
            rect.x1 - border,
            rect.y1,
        );
        let inner_radii =
            RoundedRectRadii::new(SELECTED_RADIUS - border, SELECTED_RADIUS - border, 0.0, 0.0);
        canvas.fill(&RoundedRect::from_rect(inner, inner_radii), fill);
        let mut ring = RoundedRect::from_rect(rect, top).to_path(PATH_TOLERANCE);
        ring.extend(
            RoundedRect::from_rect(inner, inner_radii)
                .to_path(PATH_TOLERANCE)
                .reverse_subpaths(),
        );
        canvas.fill_path(&ring, &Paint::Solid(t.card_stroke_default));
    } else if fill != Color::TRANSPARENT {
        // Sem borda, mas com o mesmo arredondamento de cima, um pouco afastado da linha.
        let shape = Rect::new(rect.x0, rect.y0, rect.x1, rect.y1 - metrics::BORDER);
        canvas.fill(&RoundedRect::from_rect(shape, top), fill);
    }
    if visual.separator {
        let margin = match visual.density {
            Density::Standard => SEPARATOR_MARGIN,
            Density::Compact => SEPARATOR_MARGIN_COMPACT,
        };
        canvas.fill(
            &Rect::new(
                rect.x1 - metrics::BORDER,
                rect.y0 + margin,
                rect.x1,
                rect.y1 - margin,
            ),
            t.divider_stroke_default,
        );
    }

    // O anel de andamento ocupa o lugar do ícone, e o ponto de atenção vai por cima dos dois.
    if let Some(progress) = visual.progress {
        crate::progress::draw_ring(
            canvas,
            icon_area(rect, visual),
            ProgressVisual {
                value: progress.value,
                phase: visual.phase,
                status: progress.status,
            },
            t,
        );
    } else if let Some(symbol) = visual.icon {
        let cor = visual.icon_tint.map_or(text, |cor| t.resolve(cor));
        crate::icon::draw_symbol(canvas, symbol, icon_area(rect, visual), cor);
    }
    if visual.attention {
        let dot = attention_dot(rect, visual);
        canvas.fill(
            &Circle::new(dot.center(), TAB_ATTENTION_DOT / 2.0),
            t.accent_fill_default,
        );
    }

    // O título é cortado na largura da aba, antes do botão de fechar.
    canvas.push_clip(&label_area(rect, visual).to_path(PATH_TOLERANCE));
    drawer
        .fonts
        .borrow_mut()
        .layout(label, style(visual.density), None)
        .draw(canvas, label_origin(rect, visual), &Paint::Solid(text));
    canvas.pop_clip();

    if visual.indicator {
        let x0 = (rect.center().x - INDICATOR.width / 2.0).round();
        let y0 = rect.y1 - INDICATOR_MARGIN - INDICATOR.height;
        canvas.fill(
            &RoundedRect::from_rect(
                Rect::from_origin_size((x0, y0), INDICATOR),
                INDICATOR_RADIUS,
            ),
            t.accent_fill_default,
        );
    }

    if let Some(mark) = visual.insert {
        let (width, margin) = INSERT_LINE;
        let x0 = match mark {
            InsertMark::Before => rect.x0,
            InsertMark::After => rect.x1 - width,
        };
        canvas.fill(
            &RoundedRect::new(
                x0,
                rect.y0 + margin,
                x0 + width,
                rect.y1 - margin,
                width / 2.0,
            ),
            t.accent_fill_default,
        );
    }

    if visual.focus_ring {
        crate::draw_focus_ring(
            canvas,
            rect.inset(-metrics::FOCUS_MARGIN),
            metrics::CONTROL_CORNER_RADIUS,
            t,
        );
    }
}

pub(crate) fn header_info(
    drawer: &FluentDrawer,
    rect: Rect,
    label: &str,
    visual: TabHeaderVisual,
) -> Vec<TextInfo> {
    if label.is_empty() {
        return Vec::new();
    }
    let (fill, text) = colors(&drawer.tokens, visual);
    let background = crate::over(fill, drawer.backdrop());
    vec![crate::text::line_info(
        drawer,
        label,
        style(visual.density),
        label_origin(rect, visual),
        (text, background),
        label_area(rect, visual),
    )]
}

/// Fundo e cor do glifo de um botão da faixa (`TabViewButton*` e `TabViewScrollButton*`).
fn button_colors(t: &Tokens, visual: TabStripButtonVisual) -> (Color, Color) {
    // Cor própria (o X da aba escolhida com `.selected_colors`): o hover e o press são ela,
    // translúcida, nas opacidades do `SubtleFillColorSecondary`/`Tertiary` e do
    // `TextFillColorSecondary`/`Disabled` do tema escuro, o caso comum (fundo de terminal).
    if let Some(color) = visual.color {
        let glyph = t.resolve(color);
        return match visual.state {
            ButtonState::Rest => (Color::TRANSPARENT, glyph),
            ButtonState::Hover => (glyph.with_alpha(0.0605), glyph),
            ButtonState::Pressed => (glyph.with_alpha(0.0419), glyph.multiply_alpha(0.786)),
            ButtonState::Disabled => (Color::TRANSPARENT, glyph.multiply_alpha(0.3628)),
        };
    }
    let arrow = matches!(
        visual.kind,
        TabStripButtonKind::ScrollLeft | TabStripButtonKind::ScrollRight
    );
    let rest = if arrow {
        t.text_fill_secondary
    } else {
        t.text_fill_primary
    };
    match visual.state {
        ButtonState::Rest => (Color::TRANSPARENT, rest),
        ButtonState::Hover => (t.subtle_fill_secondary, rest),
        ButtonState::Pressed if arrow => (t.subtle_fill_tertiary, rest),
        ButtonState::Pressed => (t.subtle_fill_tertiary, t.text_fill_secondary),
        ButtonState::Disabled => (Color::TRANSPARENT, t.text_fill_disabled),
    }
}

/// Triângulo cheio de uma seta, apontando para a esquerda ou para a direita.
fn arrow(center: Point, left: bool) -> BezPath {
    let (w, h) = ARROW;
    let (tip, base) = if left {
        (-h / 2.0, h / 2.0)
    } else {
        (h / 2.0, -h / 2.0)
    };
    let mut path = BezPath::new();
    path.move_to((center.x + tip, center.y));
    path.line_to((center.x + base, center.y + w / 2.0));
    path.line_to((center.x + base, center.y - w / 2.0));
    path.close_path();
    path
}

/// Pinta um botão da faixa: o fundo de 32 × 24 no meio e o glifo.
pub(crate) fn draw_button(
    canvas: &mut dyn Canvas,
    rect: Rect,
    visual: TabStripButtonVisual,
    t: &Tokens,
) {
    let (fill, glyph) = button_colors(t, visual);
    let center = rect.center();
    let (area, glyph_size) = match visual.kind {
        TabStripButtonKind::Icon(_) => (rect, ICON_GLYPH),
        _ => (Rect::from_center_size(center, BUTTON_FILL), GLYPH),
    };
    if fill != Color::TRANSPARENT {
        canvas.fill(
            &RoundedRect::from_rect(area, metrics::CONTROL_CORNER_RADIUS),
            fill,
        );
    }
    let square = crate::icon::centered(area, glyph_size);
    match visual.kind {
        TabStripButtonKind::Close => canvas.fill_path(
            &icons::DISMISS_12.path(square.origin(), GLYPH),
            &Paint::Solid(glyph),
        ),
        TabStripButtonKind::Add => crate::icon::draw_symbol(canvas, Symbol::Add, square, glyph),
        TabStripButtonKind::Icon(symbol) => crate::icon::draw_symbol(canvas, symbol, square, glyph),
        TabStripButtonKind::ScrollLeft | TabStripButtonKind::ScrollRight => canvas.fill_path(
            &arrow(area.center(), visual.kind == TabStripButtonKind::ScrollLeft),
            &Paint::Solid(glyph),
        ),
    }
    if visual.focus_ring {
        crate::draw_focus_ring(
            canvas,
            area.inset(-metrics::FOCUS_MARGIN),
            metrics::CONTROL_CORNER_RADIUS,
            t,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;
    use tomni_ui_core::ThemeColor;

    #[test]
    fn escolhida_tem_fundo_e_texto_primario() {
        let t = Tokens::new(Theme::Dark);
        let visual = |selected, state| TabHeaderVisual {
            selected,
            separator: false,
            state,
            focus_ring: false,
            density: Density::Standard,
            closable: false,
            insert: None,
            ..TabHeaderVisual::default()
        };
        assert_eq!(
            colors(&t, visual(true, ButtonState::Rest)),
            (t.solid_background_tertiary, t.text_fill_primary)
        );
        assert_eq!(
            colors(&t, visual(false, ButtonState::Rest)),
            (Color::TRANSPARENT, t.text_fill_secondary)
        );
        assert_eq!(
            colors(&t, visual(false, ButtonState::Hover)).0,
            t.layer_on_mica_base_alt_secondary
        );
    }

    #[test]
    fn escolhida_com_cores_proprias_usa_as_dela() {
        let t = Tokens::new(Theme::Dark);
        let (fundo, titulo) = (
            ThemeColor::Fixed(tomni_ui_core::Color::hex(0x0C0C0C)),
            ThemeColor::Fixed(tomni_ui_core::Color::hex(0xCCCCCC)),
        );
        let visual = TabHeaderVisual {
            selected: true,
            selected_colors: Some((fundo, titulo)),
            ..TabHeaderVisual::default()
        };
        let (fundo, titulo) = (t.resolve(fundo), t.resolve(titulo));
        assert_eq!(colors(&t, visual), (fundo, titulo));
        let desabilitada = TabHeaderVisual {
            state: ButtonState::Disabled,
            ..visual
        };
        assert_eq!(colors(&t, desabilitada), (fundo, t.text_fill_disabled));
    }

    #[test]
    fn largura_reserva_o_botao_de_fechar() {
        let visual = |density, closable| TabHeaderVisual {
            density,
            closable,
            ..TabHeaderVisual::default()
        };
        assert_eq!(padding(visual(Density::Standard, false)), (9.0, 9.0));
        assert_eq!(padding(visual(Density::Standard, true)), (9.0, 48.0));
        assert_eq!(padding(visual(Density::Compact, false)), (8.0, 4.0));
        assert_eq!(padding(visual(Density::Compact, true)), (8.0, 40.0));
    }

    #[test]
    fn icone_desloca_o_titulo_e_alarga_a_aba() {
        let sem = TabHeaderVisual {
            density: Density::Compact,
            ..TabHeaderVisual::default()
        };
        let com = TabHeaderVisual {
            icon: Some(Symbol::Broadcast),
            ..sem
        };
        let rect = Rect::new(0.0, 0.0, 120.0, 24.0);
        assert_eq!(label_origin(rect, com).x - label_origin(rect, sem).x, 26.0);
        let area = icon_area(rect, com);
        assert_eq!((area.width(), area.height()), (16.0, 16.0));
        assert_eq!(area.x0, 8.0);
        assert_eq!(label_area(rect, com).x0, 8.0 + 26.0);
    }

    #[test]
    fn setas_usam_o_texto_secundario() {
        let t = Tokens::new(Theme::Light);
        let visual = |kind, state| TabStripButtonVisual {
            kind,
            state,
            ..TabStripButtonVisual::default()
        };
        assert_eq!(
            button_colors(
                &t,
                visual(TabStripButtonKind::ScrollLeft, ButtonState::Rest)
            )
            .1,
            t.text_fill_secondary
        );
        assert_eq!(
            button_colors(&t, visual(TabStripButtonKind::Close, ButtonState::Hover)),
            (t.subtle_fill_secondary, t.text_fill_primary)
        );
        assert_eq!(
            button_colors(&t, visual(TabStripButtonKind::Add, ButtonState::Disabled)).1,
            t.text_fill_disabled
        );
    }

    #[test]
    fn o_x_com_cor_propria_usa_ela_no_glifo_e_no_hover() {
        let t = Tokens::new(Theme::Light);
        let branco = tomni_ui_core::Color::hex(0xCCCCCC);
        let cor = t.resolve(ThemeColor::Fixed(branco));
        let visual = |state| TabStripButtonVisual {
            kind: TabStripButtonKind::Close,
            state,
            color: Some(ThemeColor::Fixed(branco)),
            ..TabStripButtonVisual::default()
        };
        assert_eq!(
            button_colors(&t, visual(ButtonState::Rest)),
            (Color::TRANSPARENT, cor)
        );
        assert_eq!(
            button_colors(&t, visual(ButtonState::Hover)),
            (cor.with_alpha(0.0605), cor)
        );
    }
}
