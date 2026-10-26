//! `Image`: decodifica uma vez, guarda e pinta no `Stretch` pedido.

use std::collections::HashMap;

use tomni_ui_core::{ImageVisual, Stretch};
use tomni_ui_render::kurbo::{Rect, RoundedRect, Size};
use tomni_ui_render::{Canvas, CanvasExt, Decoded, ImageData, ImagePaint};

use crate::FluentDrawer;

/// Quantas imagens decodificadas (e SVGs rasterizados) ficam guardadas antes de esvaziar.
const CACHE: usize = 64;

/// Imagens já decodificadas, pela chave do `ImageSource`, e os SVGs já rasterizados, pela
/// chave e pelo tamanho.
#[derive(Debug, Default)]
pub(crate) struct Images {
    decoded: HashMap<u64, Result<Decoded, String>>,
    svgs: HashMap<(u64, u32, u32), ImageData>,
}

impl Images {
    fn get(&mut self, visual: &ImageVisual) -> &Result<Decoded, String> {
        if self.decoded.len() >= CACHE {
            self.decoded.clear();
        }
        self.decoded.entry(visual.source.key()).or_insert_with(|| {
            let bytes = visual.source.bytes().map_err(str::to_owned)?;
            tomni_ui_render::decode(bytes).map_err(|e| e.message().to_owned())
        })
    }

    fn svg(&mut self, key: u64, svg: &tomni_ui_render::Svg, width: u32, height: u32) -> ImageData {
        if self.svgs.len() >= CACHE {
            self.svgs.clear();
        }
        self.svgs
            .entry((key, width, height))
            .or_insert_with(|| svg.rasterize(width, height))
            .clone()
    }
}

/// Tamanho natural, ou `None` se a imagem não abre.
fn natural(drawer: &FluentDrawer, visual: &ImageVisual) -> Option<Size> {
    drawer
        .images
        .borrow_mut()
        .get(visual)
        .as_ref()
        .ok()
        .map(Decoded::size)
}

/// Por que a imagem não abre, se não abre.
pub(crate) fn error(drawer: &FluentDrawer, visual: &ImageVisual) -> Option<String> {
    drawer
        .images
        .borrow_mut()
        .get(visual)
        .as_ref()
        .err()
        .cloned()
}

/// O tamanho natural (o que o app informou em `natural_size`, ou o da decodificação); com menos largura que ele, a imagem encolhe na proporção (exceto com
/// `Stretch::None`, que corta). Uma imagem que não abre não ocupa lugar.
pub(crate) fn measure(drawer: &FluentDrawer, visual: &ImageVisual, max_width: Option<f64>) -> Size {
    let informed = visual
        .natural_size
        .filter(|&(w, h)| w > 0 && h > 0)
        .map(|(w, h)| Size::new(f64::from(w), f64::from(h)));
    let Some(size) = informed.or_else(|| natural(drawer, visual)) else {
        return Size::ZERO;
    };
    match max_width {
        Some(width) if width < size.width && visual.stretch != Stretch::None => {
            let width = width.max(0.0);
            Size::new(width, (size.height * width / size.width).ceil())
        }
        _ => size,
    }
}

/// Onde a imagem inteira fica, em pixels inteiros: pode passar de `rect` no
/// `Stretch::UniformToFill` e no `Stretch::None`, e aí é cortada.
fn placement(rect: Rect, natural: Size, stretch: Stretch) -> Rect {
    let size = match stretch {
        Stretch::Fill => return rect,
        Stretch::None => natural,
        Stretch::Uniform | Stretch::UniformToFill => {
            let sx = rect.width() / natural.width;
            let sy = rect.height() / natural.height;
            let scale = if stretch == Stretch::Uniform {
                sx.min(sy)
            } else {
                sx.max(sy)
            };
            Size::new(
                (natural.width * scale).round(),
                (natural.height * scale).round(),
            )
        }
    };
    let x = rect.x0 + ((rect.width() - size.width) / 2.0).round();
    let y = rect.y0 + ((rect.height() - size.height) / 2.0).round();
    Rect::from_origin_size((x, y), size)
}

pub(crate) fn draw(
    drawer: &FluentDrawer,
    canvas: &mut dyn Canvas,
    rect: Rect,
    visual: &ImageVisual,
) {
    let mut images = drawer.images.borrow_mut();
    let decoded = match images.get(visual) {
        Ok(decoded) => decoded.clone(),
        Err(_) => return,
    };
    let natural = decoded.size();
    if rect.is_zero_area() || natural.is_zero_area() {
        return;
    }
    let dest = placement(rect, natural, visual.stretch);
    let shown = dest.intersect(rect);
    if shown.is_zero_area() {
        return;
    }
    let pixels = match decoded {
        Decoded::Raster(pixels) => pixels,
        Decoded::Svg(svg) => images.svg(
            visual.source.key(),
            &svg,
            dest.width().round() as u32,
            dest.height().round() as u32,
        ),
    };
    drop(images);
    let paint = ImagePaint::fit(pixels, dest);
    if visual.corner_radius > 0.0 {
        canvas.fill(&RoundedRect::from_rect(shown, visual.corner_radius), paint);
    } else {
        canvas.fill(&shown, paint);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECT: Rect = Rect::new(10.0, 10.0, 110.0, 60.0);

    #[test]
    fn posicao_em_cada_stretch() {
        // Uma imagem de 50×50 num retângulo de 100×50.
        let natural = Size::new(50.0, 50.0);
        assert_eq!(placement(RECT, natural, Stretch::Fill), RECT);
        assert_eq!(
            placement(RECT, natural, Stretch::Uniform),
            Rect::new(35.0, 10.0, 85.0, 60.0)
        );
        assert_eq!(
            placement(RECT, natural, Stretch::UniformToFill),
            Rect::new(10.0, -15.0, 110.0, 85.0)
        );
        assert_eq!(
            placement(RECT, Size::new(20.0, 20.0), Stretch::None),
            Rect::new(50.0, 25.0, 70.0, 45.0)
        );
    }
}
