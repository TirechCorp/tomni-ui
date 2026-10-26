//! `Icon` e os ícones do `Symbol` dentro dos botões.

use tomni_ui_core::{IconVisual, Symbol, Token};
use tomni_ui_render::kurbo::{BezPath, Point, Rect, Size};
use tomni_ui_render::peniko::Color;
use tomni_ui_render::{Canvas, Paint};

use crate::icons::Icon;
use crate::{FluentDrawer, GraphicInfo, symbols};

/// O desenho de `symbol` para um quadrado de `size` px: o do mesmo tamanho; senão, o menor
/// maior que ele (reduzir perde menos que ampliar); senão, o maior.
fn drawing(symbol: Symbol, size: f64) -> Icon {
    let sizes = symbols::sizes(symbol);
    let &(drawn, d) = sizes
        .iter()
        .find(|&&(drawn, _)| f64::from(drawn) >= size)
        .or(sizes.last())
        .expect("todo ícone tem pelo menos um tamanho");
    Icon::new(d, f64::from(drawn))
}

/// O caminho de `symbol` num quadrado de `size` px com o canto superior esquerdo em `origin`.
pub(crate) fn path(symbol: Symbol, origin: Point, size: f64) -> BezPath {
    drawing(symbol, size).path(origin, size)
}

/// Onde fica um ícone de `size` px centralizado em `rect`, no pixel inteiro.
pub(crate) fn centered(rect: Rect, size: f64) -> Rect {
    let x = rect.x0 + ((rect.width() - size) / 2.0).round();
    let y = rect.y0 + ((rect.height() - size) / 2.0).round();
    Rect::new(x, y, x + size, y + size)
}

/// Pinta `symbol` ocupando o quadrado `square`.
pub(crate) fn draw_symbol(canvas: &mut dyn Canvas, symbol: Symbol, square: Rect, color: Color) {
    canvas.fill_path(
        &path(symbol, square.origin(), square.width()),
        &Paint::Solid(color),
    );
}

/// Cor do ícone: a própria, ou a do texto. A herdada de um `Card` já chega no `visual`, posta
/// pelo `Ui`.
fn color(drawer: &FluentDrawer, visual: &IconVisual) -> Color {
    drawer
        .tokens
        .resolve(visual.color.unwrap_or(Token::TextFillColorPrimary.into()))
}

pub(crate) fn measure(visual: &IconVisual) -> Size {
    let size = f64::from(visual.size);
    Size::new(size, size)
}

pub(crate) fn draw(
    drawer: &FluentDrawer,
    canvas: &mut dyn Canvas,
    rect: Rect,
    visual: &IconVisual,
) {
    let square = centered(rect, f64::from(visual.size));
    draw_symbol(canvas, visual.symbol, square, color(drawer, visual));
}

pub(crate) fn graphic_info(
    drawer: &FluentDrawer,
    rect: Rect,
    visual: &IconVisual,
) -> Option<GraphicInfo> {
    if visual.decorative {
        return None;
    }
    let background = drawer.backdrop();
    Some(GraphicInfo {
        name: visual.symbol.name(),
        color: crate::over(color(drawer, visual), background),
        background,
        bounds: centered(rect, f64::from(visual.size)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escolhe_o_desenho_mais_proximo() {
        // `Search` tem os quatro tamanhos; `WindowConsole`, só o de 20.
        assert_eq!(drawing(Symbol::Search, 16.0).size(), 16.0);
        assert_eq!(drawing(Symbol::Search, 18.0).size(), 20.0);
        assert_eq!(drawing(Symbol::Search, 48.0).size(), 24.0);
        assert_eq!(drawing(Symbol::WindowConsole, 12.0).size(), 20.0);
        assert_eq!(drawing(Symbol::WindowConsole, 24.0).size(), 20.0);
    }

    #[test]
    fn todos_os_caminhos_sao_svg_valido() {
        for symbol in Symbol::ALL {
            for &(size, d) in symbols::sizes(symbol) {
                assert!(
                    BezPath::from_svg(d).is_ok(),
                    "{} em {size} px",
                    symbol.name()
                );
            }
        }
    }

    #[test]
    fn ficam_dentro_do_quadrado() {
        for symbol in Symbol::ALL {
            for size in [12.0, 16.0, 20.0, 24.0] {
                let bounds =
                    tomni_ui_render::kurbo::Shape::bounding_box(&path(symbol, Point::ZERO, size));
                assert!(
                    bounds.x0 >= -0.01
                        && bounds.y0 >= -0.01
                        && bounds.x1 <= size + 0.01
                        && bounds.y1 <= size + 0.01,
                    "{} em {size} px sai do quadrado: {bounds:?}",
                    symbol.name()
                );
            }
        }
    }
}
