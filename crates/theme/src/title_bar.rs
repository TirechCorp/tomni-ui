//! `TitleBar` no visual do WinUI: o voltar e o ☰ (`TitleBarBackButtonStyle` e
//! `TitleBarPaneToggleButtonStyle`) e os botões da janela, como os da moldura do Windows 11.

use tomni_ui_core::{ButtonState, CaptionButtonVisual, CaptionGlyph, Symbol, TitleBarButtonVisual};
use tomni_ui_render::kurbo::{BezPath, Line, Rect, RoundedRect, Shape, Stroke};
use tomni_ui_render::peniko::Color;
use tomni_ui_render::{Canvas, CanvasExt, PATH_TOLERANCE, Paint};

use crate::{FluentDrawer, GraphicInfo, Tokens, icon, metrics};

/// Margem do voltar e do ☰ em volta do fundo (`Margin="2"`, com `VerticalAlignment="Stretch"`).
const BUTTON_MARGIN: f64 = 2.0;
/// Lado do glifo do voltar e do ☰ (`FontSize="16"`).
const BUTTON_ICON: f64 = 16.0;
/// Lado dos glifos dos botões da janela (os `ChromeMinimize`, `ChromeMaximize`,
/// `ChromeRestore` e `ChromeClose` da Segoe Fluent Icons em 10 px).
const CAPTION_GLYPH: f64 = 10.0;
/// Fundo do fechar com hover (`CloseButtonBackgroundPointerOver` da moldura do Windows 11).
const CLOSE_RED: Color = Color::from_rgba8(0xC4, 0x2B, 0x1C, 0xFF);
/// Fundo do fechar pressionado: o mesmo vermelho a 90%.
const CLOSE_RED_PRESSED: Color = Color::from_rgba8(0xC4, 0x2B, 0x1C, 0xE6);
/// Glifo do fechar pressionado: branco a 70%.
const CLOSE_GLYPH_PRESSED: Color = Color::from_rgba8(0xFF, 0xFF, 0xFF, 0xB3);

/// Fundo e cor do glifo do voltar e do ☰, como no `TitleBar_themeresources.xaml`: transparente
/// em repouso, `SubtleFillColorSecondary` no hover, `SubtleFillColorTertiary` pressionado e o
/// glifo terciário com a janela inativa.
fn button_colors(t: &Tokens, visual: TitleBarButtonVisual) -> (Color, Color) {
    match visual.state {
        ButtonState::Disabled => (Color::TRANSPARENT, t.text_fill_disabled),
        ButtonState::Pressed => (t.subtle_fill_tertiary, t.text_fill_secondary),
        ButtonState::Hover => (t.subtle_fill_secondary, t.text_fill_primary),
        ButtonState::Rest if !visual.active => (Color::TRANSPARENT, t.text_fill_tertiary),
        ButtonState::Rest => (Color::TRANSPARENT, t.text_fill_primary),
    }
}

/// O fundo do botão, dentro da margem.
fn button_pill(rect: Rect) -> Rect {
    rect.inflate(-BUTTON_MARGIN, -BUTTON_MARGIN)
}

pub(crate) fn draw_button(
    canvas: &mut dyn Canvas,
    rect: Rect,
    symbol: Symbol,
    visual: TitleBarButtonVisual,
    t: &Tokens,
) {
    let pill = button_pill(rect);
    let radius = metrics::CONTROL_CORNER_RADIUS;
    let (fill, foreground) = button_colors(t, visual);
    if fill != Color::TRANSPARENT {
        canvas.fill(&RoundedRect::from_rect(pill, radius), fill);
    }
    icon::draw_symbol(
        canvas,
        symbol,
        icon::centered(pill, BUTTON_ICON),
        foreground,
    );
    if visual.focus_ring {
        crate::draw_focus_ring(canvas, pill, radius, t);
    }
}

pub(crate) fn button_graphic(
    drawer: &FluentDrawer,
    rect: Rect,
    symbol: Symbol,
    visual: TitleBarButtonVisual,
) -> Vec<GraphicInfo> {
    // Com a janela inativa, a barra toda fica apagada de propósito, como no Windows: o lint
    // olha a janela ativa.
    if !visual.active {
        return Vec::new();
    }
    let pill = button_pill(rect);
    let (fill, foreground) = button_colors(&drawer.tokens, visual);
    let background = crate::over(fill, drawer.backdrop());
    vec![GraphicInfo {
        name: symbol.name(),
        color: crate::over(foreground, background),
        background,
        bounds: icon::centered(pill, BUTTON_ICON),
    }]
}

/// Fundo e cor do glifo de um botão da janela: os da moldura do Windows 11, com o fechar
/// vermelho no hover e pressionado.
fn caption_colors(t: &Tokens, visual: CaptionButtonVisual) -> (Color, Color) {
    let close = visual.glyph == CaptionGlyph::Close;
    match visual.state {
        ButtonState::Hover if close => (CLOSE_RED, Color::WHITE),
        ButtonState::Pressed if close => (CLOSE_RED_PRESSED, CLOSE_GLYPH_PRESSED),
        ButtonState::Hover => (t.subtle_fill_secondary, t.text_fill_primary),
        ButtonState::Pressed => (t.subtle_fill_tertiary, t.text_fill_secondary),
        _ if !visual.active => (Color::TRANSPARENT, t.text_fill_disabled),
        _ => (Color::TRANSPARENT, t.text_fill_primary),
    }
}

/// O quadrado do glifo, centralizado no botão.
fn caption_square(rect: Rect) -> Rect {
    icon::centered(rect, CAPTION_GLYPH)
}

/// Traços do glifo, de 1 px, no meio dos pixels (`+0,5`) para saírem nítidos.
fn caption_path(glyph: CaptionGlyph, square: Rect) -> BezPath {
    let (x, y) = (square.x0, square.y0);
    let size = CAPTION_GLYPH;
    let mut path = BezPath::new();
    let rounded = |path: &mut BezPath, rect: Rect| {
        path.extend(RoundedRect::from_rect(rect, 1.0).path_elements(PATH_TOLERANCE));
    };
    match glyph {
        CaptionGlyph::Minimize => {
            let middle = (y + size / 2.0).floor() + 0.5;
            path.move_to((x, middle));
            path.line_to((x + size, middle));
        }
        CaptionGlyph::Maximize => {
            rounded(
                &mut path,
                Rect::new(x + 0.5, y + 0.5, x + size - 0.5, y + size - 0.5),
            );
        }
        CaptionGlyph::Restore => {
            // A janela da frente, embaixo à esquerda, e a borda de cima e da direita da de trás.
            rounded(
                &mut path,
                Rect::new(x + 0.5, y + 2.5, x + size - 2.5, y + size - 0.5),
            );
            path.move_to((x + 2.5, y + 2.5));
            path.line_to((x + 2.5, y + 0.5));
            path.line_to((x + size - 0.5, y + 0.5));
            path.line_to((x + size - 0.5, y + size - 2.5));
            path.line_to((x + size - 2.5, y + size - 2.5));
        }
        CaptionGlyph::Close => {
            for line in [
                Line::new((x, y), (x + size, y + size)),
                Line::new((x + size, y), (x, y + size)),
            ] {
                path.move_to(line.p0);
                path.line_to(line.p1);
            }
        }
    }
    path
}

pub(crate) fn draw_caption(
    canvas: &mut dyn Canvas,
    rect: Rect,
    visual: CaptionButtonVisual,
    t: &Tokens,
) {
    let (fill, foreground) = caption_colors(t, visual);
    if fill != Color::TRANSPARENT {
        canvas.fill(&rect, fill);
    }
    canvas.stroke_path(
        &caption_path(visual.glyph, caption_square(rect)),
        &Stroke::new(1.0),
        &Paint::Solid(foreground),
    );
}

/// Nome do glifo, para o lint.
fn caption_name(glyph: CaptionGlyph) -> &'static str {
    match glyph {
        CaptionGlyph::Minimize => "ChromeMinimize",
        CaptionGlyph::Maximize => "ChromeMaximize",
        CaptionGlyph::Restore => "ChromeRestore",
        CaptionGlyph::Close => "ChromeClose",
    }
}

pub(crate) fn caption_graphic(
    drawer: &FluentDrawer,
    rect: Rect,
    visual: CaptionButtonVisual,
) -> Vec<GraphicInfo> {
    if !visual.active {
        return Vec::new();
    }
    let (fill, foreground) = caption_colors(&drawer.tokens, visual);
    let background = crate::over(fill, drawer.backdrop());
    vec![GraphicInfo {
        name: caption_name(visual.glyph),
        color: crate::over(foreground, background),
        background,
        bounds: caption_square(rect),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;
    use tomni_ui_render::kurbo::Point;

    #[test]
    fn fechar_fica_vermelho_no_hover() {
        let t = Tokens::new(Theme::Light);
        let visual = CaptionButtonVisual {
            glyph: CaptionGlyph::Close,
            state: ButtonState::Hover,
            active: true,
        };
        assert_eq!(caption_colors(&t, visual), (CLOSE_RED, Color::WHITE));
        let maximize = CaptionButtonVisual {
            glyph: CaptionGlyph::Maximize,
            ..visual
        };
        assert_eq!(caption_colors(&t, maximize).0, t.subtle_fill_secondary);
    }

    #[test]
    fn janela_inativa_apaga_os_glifos() {
        let t = Tokens::new(Theme::Dark);
        let caption = CaptionButtonVisual {
            glyph: CaptionGlyph::Minimize,
            state: ButtonState::Rest,
            active: false,
        };
        assert_eq!(caption_colors(&t, caption).1, t.text_fill_disabled);
        let back = TitleBarButtonVisual {
            active: false,
            ..TitleBarButtonVisual::default()
        };
        assert_eq!(button_colors(&t, back).1, t.text_fill_tertiary);
    }

    #[test]
    fn glifo_centralizado_no_botao() {
        let rect = Rect::new(0.0, 0.0, 46.0, 32.0);
        assert_eq!(caption_square(rect).center(), Point::new(23.0, 16.0));
    }
}
