use std::collections::HashMap;

use vello_cpu::{
    Image, ImageSource, Level, PaintType, Pixmap, RasterizerSettings, RenderContext, RenderMode,
    RenderSettings, Resources, TargetInit,
};

use crate::kurbo::{Affine, BezPath, Rect, Stroke};
use crate::peniko::{Color, Extend, ImageQuality, ImageSampler};
use crate::{Canvas, GlyphRun, Paint};

/// Canvas em CPU, sem janela nem GPU.
///
/// Os testes e as ferramentas desenham em pixels (escala 1). A janela, quando o adaptador da
/// GPU é um driver em software, desenha em coordenadas lógicas com o fator de DPI
/// ([`CpuCanvas::scaled`]), como o [`GpuCanvas`](crate::GpuCanvas).
#[derive(Debug)]
pub struct CpuCanvas {
    ctx: RenderContext,
    scale: f64,
    resources: Resources,
    render_mode: RenderMode,
    /// As imagens já convertidas para o `vello_cpu`, pelo id dos pixels.
    images: HashMap<u64, ImageSource>,
    /// Se a última tinta foi uma imagem, com a transformação dela no contexto.
    paint_transform: bool,
}

/// Quantas imagens convertidas o canvas guarda antes de esvaziar o cache.
const IMAGE_CACHE: usize = 64;

/// Amostragem das imagens: bilinear, com a borda repetida fora da imagem.
pub(crate) fn sampler() -> ImageSampler {
    ImageSampler {
        x_extend: Extend::Pad,
        y_extend: Extend::Pad,
        quality: ImageQuality::Medium,
        alpha: 1.0,
    }
}

impl CpuCanvas {
    /// Canvas de `width` × `height` pixels para uso em produção: SIMD detectado em tempo de
    /// execução e pipeline `u8`.
    pub fn new(width: u16, height: u16) -> Self {
        let level = Level::try_detect().unwrap_or(Level::baseline());
        Self::with(width, height, level, RenderMode::OptimizeSpeed)
    }

    /// Canvas de referência dos snapshots: nível escalar (sem SIMD), uma thread e pipeline
    /// `f32`, para que x86_64 e ARM gerem os mesmos pixels.
    ///
    /// O LLVM ainda pode autovetorizar o caminho escalar, e algumas funções de ponto flutuante
    /// vêm da libm de cada sistema; a igualdade entre arquiteturas é confirmada pela matriz de CI.
    #[cfg(feature = "deterministic")]
    pub fn deterministic(width: u16, height: u16) -> Self {
        Self::with(
            width,
            height,
            Level::fallback(),
            RenderMode::OptimizeQuality,
        )
    }

    fn with(width: u16, height: u16, level: Level, render_mode: RenderMode) -> Self {
        let settings = RenderSettings {
            level,
            num_threads: 0,
        };
        Self {
            ctx: RenderContext::new_with(width, height, settings),
            scale: 1.0,
            resources: Resources::new(),
            render_mode,
            images: HashMap::new(),
            paint_transform: false,
        }
    }

    /// Canvas de produção com `width` × `height` pixels físicos e fator de escala `scale`: o
    /// desenho vem em coordenadas lógicas e `size` devolve o tamanho lógico.
    pub fn scaled(width: u16, height: u16, scale: f64) -> Self {
        let mut canvas = Self::new(width, height);
        canvas.set_scale(scale);
        canvas
    }

    /// Limpa o desenho para o próximo frame, com tamanho (em pixels físicos) e escala novos.
    pub fn reset(&mut self, width: u16, height: u16, scale: f64) {
        if (width, height) == (self.ctx.width(), self.ctx.height()) {
            self.ctx.reset();
        } else {
            self.ctx.reset_and_resize(width, height);
        }
        self.paint_transform = false;
        self.set_scale(scale);
    }

    fn set_scale(&mut self, scale: f64) {
        self.scale = scale;
        self.ctx.set_transform(Affine::scale(scale));
    }

    /// Rasteriza tudo o que foi desenhado sobre um fundo `background`.
    pub fn render(&mut self, background: Color) -> Pixmap {
        let mut pixmap = Pixmap::new(self.ctx.width(), self.ctx.height());
        let settings = RasterizerSettings {
            render_mode: self.render_mode,
            target_init: TargetInit::Clear(background),
            ..RasterizerSettings::default()
        };
        self.ctx.flush();
        self.ctx
            .render_with(&mut pixmap, &mut self.resources, settings);
        pixmap
    }

    /// `outer` é a parte da escala que a transformação do contexto não aplica: só os glifos,
    /// que vão em pixels físicos, a passam.
    fn set_paint(&mut self, paint: &Paint, outer: Affine) {
        let transform = match paint {
            Paint::Image(paint) => outer * paint.transform,
            _ => outer,
        };
        if transform != Affine::IDENTITY {
            self.ctx.set_paint_transform(transform);
            self.paint_transform = true;
        } else if self.paint_transform {
            self.ctx.reset_paint_transform();
            self.paint_transform = false;
        }
        let paint: PaintType = match paint {
            Paint::Solid(color) => (*color).into(),
            Paint::Gradient(gradient) => gradient.clone().into(),
            Paint::Image(paint) => {
                if self.images.len() >= IMAGE_CACHE {
                    self.images.clear();
                }
                let image = self
                    .images
                    .entry(paint.image.data.id())
                    .or_insert_with(|| ImageSource::from_peniko_image_data(&paint.image))
                    .clone();
                Image {
                    image,
                    sampler: sampler(),
                }
                .into()
            }
        };
        self.ctx.set_paint(paint);
    }
}

impl Canvas for CpuCanvas {
    fn size(&self) -> (u32, u32) {
        let logical = |n: u16| (f64::from(n) / self.scale).round() as u32;
        (logical(self.ctx.width()), logical(self.ctx.height()))
    }

    fn fill_path(&mut self, path: &BezPath, paint: &Paint) {
        self.set_paint(paint, Affine::IDENTITY);
        self.ctx.fill_path(path);
    }

    fn fill_rect(&mut self, rect: Rect, paint: &Paint) {
        self.set_paint(paint, Affine::IDENTITY);
        self.ctx.fill_rect(&rect);
    }

    fn stroke_path(&mut self, path: &BezPath, stroke: &Stroke, paint: &Paint) {
        self.set_paint(paint, Affine::IDENTITY);
        self.ctx.set_stroke(stroke.clone());
        self.ctx.stroke_path(path);
    }

    fn draw_glyphs(&mut self, run: &GlyphRun<'_>, paint: &Paint) {
        // Como no `GpuCanvas`: a escala vai para o tamanho da fonte e para a posição dos
        // glifos, e não para a transformação, para o hinting valer no tamanho em pixels.
        let scale = self.scale as f32;
        let outer = Affine::scale(self.scale);
        self.ctx.reset_transform();
        self.set_paint(paint, outer);
        let glyphs = run.glyphs.iter().map(|g| vello_cpu::Glyph {
            id: g.id,
            x: g.x * scale,
            y: g.y * scale,
        });
        let mut builder = self
            .ctx
            .glyph_run(&mut self.resources, run.font)
            .font_size(run.font_size * scale)
            .normalized_coords(run.normalized_coords)
            .hint(true);
        if run.skew != 0.0 {
            builder = builder.glyph_transform(Affine::skew(f64::from(run.skew), 0.0));
        }
        let result = builder.fill_glyphs(glyphs);
        self.ctx.set_transform(outer);
        // Só falha com fonte corrompida; as fontes do projeto vêm embutidas.
        debug_assert!(result.is_ok(), "glifos não renderizados: {result:?}");
    }

    fn push_clip(&mut self, path: &BezPath) {
        self.ctx.push_clip_path(path);
    }

    fn pop_clip(&mut self) {
        self.ctx.pop_clip();
    }

    fn push_opacity(&mut self, alpha: f32) {
        self.ctx.push_opacity_layer(alpha);
    }

    fn pop_opacity(&mut self) {
        self.ctx.pop_layer();
    }
}
