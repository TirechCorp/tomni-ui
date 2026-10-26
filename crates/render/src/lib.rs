//! Trait `Canvas` e seus backends: CPU headless via `vello_cpu` (M0) e GPU via `vello`/`wgpu` (M4).
//!
//! Os dois backends precisam produzir os mesmos pixels: a referência dos snapshots é a CPU.

mod cpu;
#[cfg(feature = "gpu")]
mod gpu;
pub mod image;
mod recording;

pub use cpu::CpuCanvas;
#[cfg(feature = "gpu")]
pub use gpu::{GpuCanvas, GpuError, GpuRenderer, new_renderer, render_params};
pub use image::{Decoded, ImageError, Svg, decode};
pub use peniko;
pub use peniko::ImageData;
pub use peniko::kurbo;
pub use recording::Recording;
#[cfg(feature = "gpu")]
pub use vello;
pub use vello_cpu::Pixmap;

use kurbo::{Affine, BezPath, Rect, Shape, Stroke};
use peniko::{Color, FontData, Gradient};

/// Tolerância usada ao converter curvas em caminhos.
pub const PATH_TOLERANCE: f64 = 0.1;

/// Tinta de preenchimento ou contorno.
#[derive(Clone, Debug, PartialEq)]
pub enum Paint {
    /// Cor sólida.
    Solid(Color),
    /// Gradiente (usado, por exemplo, na borda de elevação dos controles Fluent).
    Gradient(Gradient),
    /// Imagem (M14): o `Image` e os SVGs rasterizados.
    Image(ImagePaint),
}

/// Uma imagem usada como tinta.
#[derive(Clone, Debug, PartialEq)]
pub struct ImagePaint {
    /// Os pixels.
    pub image: ImageData,
    /// Leva as coordenadas da imagem (o pixel `(x, y)` ocupa `x..x+1`) às do canvas.
    pub transform: Affine,
}

impl ImagePaint {
    /// A imagem esticada para cobrir `rect`.
    pub fn fit(image: ImageData, rect: Rect) -> Self {
        let transform = Affine::translate(rect.origin().to_vec2())
            * Affine::scale_non_uniform(
                rect.width() / f64::from(image.width.max(1)),
                rect.height() / f64::from(image.height.max(1)),
            );
        Self { image, transform }
    }
}

impl From<Color> for Paint {
    fn from(color: Color) -> Self {
        Self::Solid(color)
    }
}

impl From<ImagePaint> for Paint {
    fn from(image: ImagePaint) -> Self {
        Self::Image(image)
    }
}

impl From<Gradient> for Paint {
    fn from(gradient: Gradient) -> Self {
        Self::Gradient(gradient)
    }
}

/// Um glifo posicionado, em pixels, relativo à origem do canvas.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
    /// Índice do glifo na fonte (não é um code point).
    pub id: u32,
    /// Posição horizontal.
    pub x: f32,
    /// Posição vertical da linha de base.
    pub y: f32,
}

/// Sequência de glifos com a mesma fonte e o mesmo tamanho.
#[derive(Clone, Copy, Debug)]
pub struct GlyphRun<'a> {
    /// Fonte dos glifos.
    pub font: &'a FontData,
    /// Tamanho em pixels.
    pub font_size: f32,
    /// Coordenadas de fontes variáveis (vazio para fontes estáticas).
    pub normalized_coords: &'a [i16],
    /// Glifos já posicionados.
    pub glyphs: &'a [Glyph],
    /// Inclinação do itálico sintético (a fonte não tem a face itálica): a tangente do ângulo
    /// de inclinação, `0.0` sem inclinar.
    pub skew: f32,
}

/// Superfície de desenho. O tema pinta só por meio deste trait, então trocar a GPU pela CPU
/// é trocar a implementação de `Canvas`.
pub trait Canvas {
    /// Largura e altura em pixels.
    fn size(&self) -> (u32, u32);

    /// Preenche um caminho.
    fn fill_path(&mut self, path: &BezPath, paint: &Paint);

    /// Preenche um retângulo alinhado aos eixos.
    fn fill_rect(&mut self, rect: Rect, paint: &Paint) {
        self.fill_path(&rect.to_path(PATH_TOLERANCE), paint);
    }

    /// Contorna um caminho.
    fn stroke_path(&mut self, path: &BezPath, stroke: &Stroke, paint: &Paint);

    /// Desenha uma sequência de glifos.
    fn draw_glyphs(&mut self, run: &GlyphRun<'_>, paint: &Paint);

    /// Restringe o desenho seguinte ao interior do caminho, até o `pop_clip` correspondente.
    fn push_clip(&mut self, path: &BezPath);

    /// Desfaz o último `push_clip`.
    fn pop_clip(&mut self);

    /// Pinta o desenho seguinte, até o `pop_opacity` correspondente, numa camada que é
    /// composta com a opacidade `alpha` (de 0 a 1): o que se sobrepõe dentro dela não soma.
    /// O padrão ignora a opacidade.
    fn push_opacity(&mut self, alpha: f32) {
        let _ = alpha;
    }

    /// Fecha a camada do último `push_opacity`.
    fn pop_opacity(&mut self) {}
}

/// Atalhos genéricos sobre qualquer `Canvas`, inclusive `dyn Canvas`.
pub trait CanvasExt: Canvas {
    /// Preenche qualquer forma do `kurbo` (`RoundedRect`, `Circle`, ...).
    fn fill(&mut self, shape: &impl Shape, paint: impl Into<Paint>) {
        let paint = paint.into();
        match shape.as_rect() {
            Some(rect) => self.fill_rect(rect, &paint),
            None => self.fill_path(&shape.to_path(PATH_TOLERANCE), &paint),
        }
    }

    /// Desenha `image` esticada para cobrir `rect`.
    fn draw_image(&mut self, image: &ImageData, rect: Rect) {
        self.fill_rect(rect, &Paint::Image(ImagePaint::fit(image.clone(), rect)));
    }

    /// Contorna qualquer forma do `kurbo`.
    fn stroke(&mut self, shape: &impl Shape, stroke: &Stroke, paint: impl Into<Paint>) {
        self.stroke_path(&shape.to_path(PATH_TOLERANCE), stroke, &paint.into());
    }
}

impl<C: Canvas + ?Sized> CanvasExt for C {}
