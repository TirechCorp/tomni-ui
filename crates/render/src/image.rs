//! Decodificação das imagens: PNG (`png`), JPEG (`zune-jpeg`), GIF e WebP (`image`) e SVG
//! (`resvg`).
//!
//! O formato vem dos primeiros bytes, não da extensão. PNG, JPEG, GIF e WebP viram pixels RGBA
//! na hora (o GIF dá só o primeiro quadro, e o WebP animado também); o SVG fica como árvore e é
//! rasterizado no tamanho em que vai aparecer, para não borrar.

use std::fmt;
use std::io::Cursor;
use std::sync::Arc;

use peniko::{Blob, ImageAlphaType, ImageData, ImageFormat};
use resvg::{tiny_skia, usvg};

use crate::kurbo::Size;

/// Lado máximo de uma imagem, em pixels: o `vello_cpu` não passa de `u16::MAX`, e uma foto de
/// celular cabe com folga.
pub const MAX_SIDE: u32 = 16_384;

/// Memória máxima para os pixels de uma imagem decodificada.
const MAX_BYTES: usize = 256 * 1024 * 1024;

/// Uma imagem decodificada.
#[derive(Clone, Debug)]
pub enum Decoded {
    /// PNG, JPEG, GIF ou WebP: os pixels, em RGBA sem alfa pré-multiplicado.
    Raster(ImageData),
    /// SVG, rasterizado em [`Svg::rasterize`].
    Svg(Svg),
}

impl Decoded {
    /// Tamanho natural: os pixels da imagem, ou o `width`/`height` do SVG.
    pub fn size(&self) -> Size {
        match self {
            Decoded::Raster(image) => Size::new(f64::from(image.width), f64::from(image.height)),
            Decoded::Svg(svg) => svg.size(),
        }
    }
}

/// Um SVG já lido, pronto para rasterizar em qualquer tamanho.
#[derive(Clone)]
pub struct Svg(Arc<usvg::Tree>);

impl Svg {
    /// Tamanho do documento (`width` e `height`, ou o `viewBox`).
    pub fn size(&self) -> Size {
        let size = self.0.size();
        Size::new(f64::from(size.width()), f64::from(size.height()))
    }

    /// Os pixels do SVG esticado para `width` × `height`, com alfa pré-multiplicado.
    pub fn rasterize(&self, width: u32, height: u32) -> ImageData {
        let (width, height) = (width.clamp(1, MAX_SIDE), height.clamp(1, MAX_SIDE));
        let mut pixmap = tiny_skia::Pixmap::new(width, height).expect("o tamanho não é zero");
        let size = self.0.size();
        let transform = tiny_skia::Transform::from_scale(
            width as f32 / size.width(),
            height as f32 / size.height(),
        );
        resvg::render(&self.0, transform, &mut pixmap.as_mut());
        ImageData {
            data: Blob::new(Arc::new(pixmap.take())),
            format: ImageFormat::Rgba8,
            alpha_type: ImageAlphaType::AlphaPremultiplied,
            width,
            height,
        }
    }
}

impl fmt::Debug for Svg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Svg").field(&self.size()).finish()
    }
}

/// Por que uma imagem não pôde ser lida. A mensagem é a que a árvore do app mostra.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageError(String);

impl ImageError {
    /// Erro com a mensagem `message`.
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    /// A mensagem, em português.
    pub fn message(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ImageError {}

/// Decodifica PNG, JPEG, GIF, WebP ou SVG (o GIF e o WebP animados dão o primeiro quadro).
pub fn decode(bytes: &[u8]) -> Result<Decoded, ImageError> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        decode_png(bytes).map(Decoded::Raster)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        decode_jpeg(bytes).map(Decoded::Raster)
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        decode_with_image(bytes, image::ImageFormat::Gif, "GIF").map(Decoded::Raster)
    } else if is_webp(bytes) {
        decode_with_image(bytes, image::ImageFormat::WebP, "WebP").map(Decoded::Raster)
    } else if looks_like_svg(bytes) {
        decode_svg(bytes).map(Decoded::Svg)
    } else if bytes.is_empty() {
        Err(ImageError::new("imagem vazia"))
    } else {
        Err(ImageError::new(
            "formato de imagem desconhecido: só PNG, JPEG, GIF, WebP e SVG",
        ))
    }
}

/// `RIFF`, 4 bytes de tamanho e `WEBP`.
fn is_webp(bytes: &[u8]) -> bool {
    bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP"
}

/// Um SVG começa, depois de espaços e de um BOM, com `<`.
fn looks_like_svg(bytes: &[u8]) -> bool {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    bytes
        .iter()
        .find(|b| !b.is_ascii_whitespace())
        .is_some_and(|&b| b == b'<')
}

fn check_size(width: u32, height: u32) -> Result<(), ImageError> {
    if width == 0 || height == 0 {
        return Err(ImageError::new("imagem sem pixels"));
    }
    if width > MAX_SIDE || height > MAX_SIDE {
        return Err(ImageError::new(format!(
            "imagem grande demais: {width}×{height} px (o máximo é {MAX_SIDE} px de lado)"
        )));
    }
    Ok(())
}

fn rgba(pixels: Vec<u8>, width: u32, height: u32) -> ImageData {
    ImageData {
        data: Blob::new(Arc::new(pixels)),
        format: ImageFormat::Rgba8,
        alpha_type: ImageAlphaType::Alpha,
        width,
        height,
    }
}

fn decode_png(bytes: &[u8]) -> Result<ImageData, ImageError> {
    let error = |e: png::DecodingError| ImageError::new(format!("PNG inválido: {e}"));
    let mut decoder =
        png::Decoder::new_with_limits(Cursor::new(bytes), png::Limits { bytes: MAX_BYTES });
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(error)?;
    let (width, height) = {
        let info = reader.info();
        (info.width, info.height)
    };
    check_size(width, height)?;
    let len = reader
        .output_buffer_size()
        .ok_or_else(|| ImageError::new("PNG inválido: tamanho grande demais"))?;
    let mut buffer = vec![0; len];
    let info = reader.next_frame(&mut buffer).map_err(error)?;
    buffer.truncate(info.buffer_size());
    let pixels = match info.color_type {
        png::ColorType::Rgba => buffer,
        png::ColorType::Rgb => buffer
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buffer
            .chunks_exact(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => buffer.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        // `normalize_to_color8` expande a paleta para RGB(A).
        png::ColorType::Indexed => {
            return Err(ImageError::new("PNG inválido: paleta não expandida"));
        }
    };
    Ok(rgba(pixels, width, height))
}

fn decode_jpeg(bytes: &[u8]) -> Result<ImageData, ImageError> {
    use zune_jpeg::zune_core::bytestream::ZCursor;
    use zune_jpeg::zune_core::colorspace::ColorSpace;
    use zune_jpeg::zune_core::options::DecoderOptions;

    let error = |e: zune_jpeg::errors::DecodeErrors| ImageError::new(format!("JPEG inválido: {e}"));
    let options = DecoderOptions::default()
        .jpeg_set_out_colorspace(ColorSpace::RGBA)
        .set_max_width(MAX_SIDE as usize)
        .set_max_height(MAX_SIDE as usize);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(bytes), options);
    decoder.decode_headers().map_err(error)?;
    let (width, height) = decoder
        .dimensions()
        .ok_or_else(|| ImageError::new("JPEG inválido: sem tamanho"))?;
    let (width, height) = (width as u32, height as u32);
    check_size(width, height)?;
    let pixels = decoder.decode().map_err(error)?;
    if pixels.len() != width as usize * height as usize * 4 {
        return Err(ImageError::new("JPEG inválido: pixels faltando"));
    }
    Ok(rgba(pixels, width, height))
}

/// GIF e WebP pelo crate `image`, com os mesmos limites do PNG e do JPEG.
fn decode_with_image(
    bytes: &[u8],
    format: image::ImageFormat,
    name: &str,
) -> Result<ImageData, ImageError> {
    let error = |e: image::ImageError| match e {
        image::ImageError::Limits(_) => ImageError::new(format!(
            "imagem grande demais (o máximo é {MAX_SIDE} px de lado)"
        )),
        e => ImageError::new(format!("{name} inválido: {e}")),
    };
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_BYTES as u64);
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits);
    let image = reader.decode().map_err(error)?;
    let (width, height) = (image.width(), image.height());
    check_size(width, height)?;
    Ok(rgba(image.into_rgba8().into_raw(), width, height))
}

fn decode_svg(bytes: &[u8]) -> Result<Svg, ImageError> {
    let tree = usvg::Tree::from_data(bytes, &usvg::Options::default())
        .map_err(|e| ImageError::new(format!("SVG inválido: {e}")))?;
    let size = tree.size();
    if size.width() > MAX_SIDE as f32 || size.height() > MAX_SIDE as f32 {
        return Err(ImageError::new(format!(
            "SVG grande demais: {}×{} (o máximo é {MAX_SIDE} de lado)",
            size.width(),
            size.height()
        )));
    }
    Ok(Svg(Arc::new(tree)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_desconhecidos_viram_erro() {
        let erro = decode(b"BMxxxxxxxx").unwrap_err();
        assert_eq!(
            erro.message(),
            "formato de imagem desconhecido: só PNG, JPEG, GIF, WebP e SVG"
        );
        assert_eq!(decode(b"").unwrap_err().message(), "imagem vazia");
    }

    #[test]
    fn png_cortado_vira_erro() {
        let erro = decode(b"\x89PNG\r\n\x1a\n\0\0").unwrap_err();
        assert!(erro.message().starts_with("PNG inválido"), "{erro}");
    }

    #[test]
    fn jpeg_cortado_vira_erro() {
        let erro = decode(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 16]).unwrap_err();
        assert!(erro.message().starts_with("JPEG inválido"), "{erro}");
    }

    #[test]
    fn svg_mal_formado_vira_erro() {
        let erro = decode(b"<svg").unwrap_err();
        assert!(erro.message().starts_with("SVG inválido"), "{erro}");
    }

    #[test]
    fn svg_rasteriza_no_tamanho_pedido() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="5"><rect width="10" height="5" fill="#ff0000"/></svg>"##;
        let Decoded::Svg(svg) = decode(svg).unwrap() else {
            panic!("não é SVG");
        };
        assert_eq!(svg.size(), Size::new(10.0, 5.0));
        let image = svg.rasterize(20, 10);
        assert_eq!((image.width, image.height), (20, 10));
        assert_eq!(&image.data.data()[..4], &[255, 0, 0, 255]);
    }

    const GIF_DOIS_QUADROS: &[u8] = include_bytes!("../tests/data/dois.gif");
    const WEBP_COM_PERDA: &[u8] = include_bytes!("../tests/data/lossy.webp");
    const WEBP_SEM_PERDA: &[u8] = include_bytes!("../tests/data/lossless.webp");

    fn raster(bytes: &[u8]) -> ImageData {
        match decode(bytes).unwrap() {
            Decoded::Raster(image) => image,
            Decoded::Svg(_) => panic!("não é raster"),
        }
    }

    /// Um GIF de `width` × `height` com os pixels pretos, pelo codificador do `image`.
    fn gif_de(width: u32, height: u32) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = image::codecs::gif::GifEncoder::new(&mut out);
        let frame = image::Frame::new(image::RgbaImage::new(width, height));
        encoder.encode_frame(frame).unwrap();
        drop(encoder);
        out
    }

    /// Um WebP sem perda de `width` × `height` (o formato não passa de 16 384 de lado).
    fn webp_de(width: u32, height: u32) -> Vec<u8> {
        let mut out = Vec::new();
        let pixels = vec![0u8; width as usize * height as usize * 4];
        image::codecs::webp::WebPEncoder::new_lossless(&mut out)
            .encode(&pixels, width, height, image::ExtendedColorType::Rgba8)
            .unwrap();
        out
    }

    fn webp_de_tentativa(width: u32, height: u32) -> Result<(), image::ImageError> {
        let pixels = vec![0u8; width as usize * height as usize * 4];
        image::codecs::webp::WebPEncoder::new_lossless(Vec::new()).encode(
            &pixels,
            width,
            height,
            image::ExtendedColorType::Rgba8,
        )
    }

    #[test]
    fn gif_de_dois_quadros_decodifica_o_primeiro() {
        let image = raster(GIF_DOIS_QUADROS);
        assert_eq!((image.width, image.height), (3, 2));
        let pixels = image.data.data();
        assert_eq!(&pixels[..4], &[0, 255, 0, 255], "o primeiro quadro é verde");
        assert_eq!(pixels.len(), 3 * 2 * 4);
    }

    #[test]
    fn webp_sem_perda_e_com_perda_decodificam() {
        for bytes in [WEBP_SEM_PERDA, WEBP_COM_PERDA] {
            let image = raster(bytes);
            assert_eq!(
                image.data.data().len(),
                (image.width * image.height * 4) as usize
            );
            let [r, g, b, a] = image.data.data()[..4] else {
                panic!("sem pixels");
            };
            assert!(
                r > 150 && g < 80 && b < 80 && a == 255,
                "vermelho: {r} {g} {b} {a}"
            );
        }
        assert_eq!(raster(WEBP_SEM_PERDA).width, 8);
        assert_eq!(raster(WEBP_COM_PERDA).height, 4);
    }

    #[test]
    fn gif_acima_do_maximo_da_erro_e_o_webp_no_limite_decodifica() {
        let erro = decode(&gif_de(MAX_SIDE + 1, 1)).unwrap_err();
        assert!(erro.message().contains("grande demais"), "{erro}");
        let erro = decode(&gif_de(1, MAX_SIDE + 1)).unwrap_err();
        assert!(erro.message().contains("grande demais"), "{erro}");
        assert_eq!(raster(&gif_de(MAX_SIDE, 1)).width, MAX_SIDE);
        // O WebP não passa de 16 384 de lado, o `MAX_SIDE`: o formato mesmo não representa
        // mais que isso, então o limite de lado nunca dispara nele (o de memória sim).
        assert_eq!(raster(&webp_de(16_000, 2)).width, 16_000);
        assert!(
            webp_de_tentativa(MAX_SIDE + 1, 1).is_err(),
            "o formato não representa"
        );
    }

    #[test]
    fn gif_e_webp_cortados_dao_erro_sem_panico() {
        for bytes in [GIF_DOIS_QUADROS, WEBP_SEM_PERDA, WEBP_COM_PERDA] {
            for corte in [6, 12, bytes.len() / 2, bytes.len() - 1] {
                // O corte pode até decodificar um GIF (o quadro pode estar completo); o que não
                // pode é entrar em pânico.
                let _ = decode(&bytes[..corte]);
            }
        }
        let erro = decode(&WEBP_COM_PERDA[..20]).unwrap_err();
        assert!(erro.message().starts_with("WebP inválido"), "{erro}");
        let erro = decode(&GIF_DOIS_QUADROS[..12]).unwrap_err();
        assert!(erro.message().starts_with("GIF inválido"), "{erro}");
    }
}
