//! O ícone da janela (M49): o que a barra de tarefas, o alternador de janelas e o dock mostram.
//!
//! O app dá os bytes de um PNG ou de um SVG embutido; aqui eles viram os pixels RGBA que o
//! `winit` pede, no tamanho de cada uso.

use tomni_ui_render::{Decoded, decode};

/// Lado, em pixels, do ícone pequeno: o da moldura da janela e do alternador de janelas.
pub(crate) const SMALL_SIDE: u32 = 32;

/// Lado, em pixels, do ícone grande no X11 (`_NET_WM_ICON`): o dock e o alternador de janelas
/// dos ambientes que escolhem o maior. Só o ramo do X11 usa.
#[cfg(all(unix, not(any(target_os = "macos", target_os = "android"))))]
pub(crate) const LARGE_SIDE: u32 = 128;

/// Lado, em pixels, do ícone da barra de tarefas do Windows (a janela pede o grande).
#[cfg(windows)]
pub(crate) const TASKBAR_SIDE: u32 = 64;

/// O ícone de uma janela: o grande e, se o desenho muda de tamanho para ficar legível, o pequeno.
///
/// Cada um é um PNG ou um SVG. O SVG é rasterizado no tamanho de cada uso; o PNG vai como está
/// (o sistema o reduz).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowIcon {
    /// O desenho usado em todo lugar onde não há o pequeno.
    pub large: &'static [u8],
    /// O desenho dos tamanhos pequenos (a moldura da janela), com menos detalhe. Sem ele, vale o
    /// `large`.
    pub small: Option<&'static [u8]>,
}

impl WindowIcon {
    /// Um ícone só, para todos os tamanhos.
    pub const fn new(large: &'static [u8]) -> Self {
        Self { large, small: None }
    }

    /// Acrescenta o desenho dos tamanhos pequenos.
    pub const fn with_small(mut self, small: &'static [u8]) -> Self {
        self.small = Some(small);
        self
    }

    /// O desenho para o lado `side` (em pixels): o pequeno quando o lado é de até 32 px.
    pub(crate) fn for_side(&self, side: u32) -> &'static [u8] {
        match self.small {
            Some(small) if side <= SMALL_SIDE => small,
            _ => self.large,
        }
    }
}

/// Os pixels RGBA (sem alfa pré-multiplicado), a largura e a altura de `bytes` para um ícone de
/// `size` × `size`.
///
/// O SVG é rasterizado em `size`; o PNG (e os demais formatos raster) vai no tamanho que tem.
/// `None` se `bytes` não é uma imagem que a lib lê.
pub fn rgba(bytes: &[u8], size: u32) -> Option<(Vec<u8>, u32, u32)> {
    match decode(bytes).ok()? {
        Decoded::Raster(image) => Some((image.data.data().to_vec(), image.width, image.height)),
        Decoded::Svg(svg) => {
            let image = svg.rasterize(size, size);
            let mut data = image.data.data().to_vec();
            // O resvg entrega o alfa pré-multiplicado, e o sistema espera o direto.
            for pixel in data.chunks_exact_mut(4) {
                let alpha = u32::from(pixel[3]);
                if alpha != 0 && alpha != 255 {
                    for channel in &mut pixel[..3] {
                        *channel = ((u32::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
                    }
                }
            }
            Some((data, image.width, image.height))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
        <rect width="10" height="10" fill="#ff0000"/></svg>"##;
    const SVG_MEIO: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
        <rect width="10" height="10" fill="#ff0000" fill-opacity="0.5"/></svg>"##;

    #[test]
    fn rgba_rasteriza_o_svg_no_tamanho_pedido() {
        let (data, width, height) = rgba(SVG, 32).unwrap();
        assert_eq!((width, height), (32, 32));
        assert_eq!(data.len(), 32 * 32 * 4);
        assert_eq!(&data[..4], &[255, 0, 0, 255]);
    }

    #[test]
    fn rgba_devolve_o_svg_com_alfa_direto() {
        let (data, _, _) = rgba(SVG_MEIO, 8).unwrap();
        let [r, g, b, a] = [data[0], data[1], data[2], data[3]];
        assert!((i32::from(a) - 128).abs() <= 1, "alfa {a}");
        assert!(r >= 254 && g == 0 && b == 0, "cor {r} {g} {b}");
    }

    #[test]
    fn rgba_deixa_o_png_como_esta() {
        let (svg, _, _) = rgba(SVG, 16).unwrap();
        let png = {
            let mut out = Vec::new();
            let mut encoder = png::Encoder::new(&mut out, 16, 16);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&svg)
                .unwrap();
            out
        };
        // O pedido de 64 px não estica o PNG.
        let (data, width, height) = rgba(&png, 64).unwrap();
        assert_eq!((width, height), (16, 16));
        assert_eq!(data, svg);
    }

    #[test]
    fn rgba_recusa_o_que_nao_e_imagem() {
        assert!(rgba(b"isto nao e uma imagem", 32).is_none());
        assert!(rgba(b"", 32).is_none());
    }

    #[test]
    fn o_pequeno_vale_so_nos_lados_pequenos() {
        let icone = WindowIcon::new(b"grande").with_small(b"pequeno");
        assert_eq!(icone.for_side(16), b"pequeno");
        assert_eq!(icone.for_side(32), b"pequeno");
        assert_eq!(icone.for_side(64), b"grande");
        assert_eq!(WindowIcon::new(b"grande").for_side(16), b"grande");
    }
}
