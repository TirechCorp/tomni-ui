//! Backend GPU: `vello` sobre `wgpu`.
//!
//! O [`GpuCanvas`] monta uma `vello::Scene` em pixels lógicos e aplica a escala da tela (DPI)
//! ao gravar. O [`GpuRenderer`] rasteriza a cena numa textura e devolve um [`Pixmap`], para os
//! testes GPU × CPU; a janela (`tomni-ui-platform`) usa o mesmo canvas com uma superfície.

use std::fmt;

use vello::kurbo::Affine;
use vello::peniko::{ColorStop, Fill, ImageAlphaType, Mix};
use vello::{AaConfig, AaSupport, RenderParams, Renderer, RendererOptions, Scene, wgpu};
use vello_cpu::{PixelMetadata, Pixmap};

use crate::kurbo::{BezPath, Rect, Stroke};
use crate::peniko::{Brush, Color, ImageBrush};
use crate::{Canvas, GlyphRun, Paint};

/// Canvas que grava uma cena do `vello` para a GPU.
///
/// As coordenadas são lógicas, como no [`CpuCanvas`](crate::CpuCanvas); `scale` (o fator de
/// DPI da janela) multiplica tudo ao gravar. No texto, a escala vai para o tamanho da fonte e
/// para a posição dos glifos, e não para uma transformação: assim o hinting vale no tamanho
/// final, em pixels físicos.
pub struct GpuCanvas {
    scene: Scene,
    size: (u32, u32),
    scale: f64,
}

impl GpuCanvas {
    /// Canvas com `width` × `height` pixels lógicos e fator de escala `scale`.
    pub fn new(width: u32, height: u32, scale: f64) -> Self {
        Self {
            scene: Scene::new(),
            size: (width, height),
            scale,
        }
    }

    /// Limpa a cena para o próximo frame, com tamanho e escala novos.
    pub fn reset(&mut self, width: u32, height: u32, scale: f64) {
        self.scene.reset();
        self.size = (width, height);
        self.scale = scale;
    }

    /// Fator de escala.
    pub fn scale(&self) -> f64 {
        self.scale
    }

    /// Tamanho em pixels físicos: o lógico vezes a escala, arredondado para cima.
    pub fn physical_size(&self) -> (u32, u32) {
        let physical = |n: u32| (f64::from(n) * self.scale).ceil() as u32;
        (physical(self.size.0), physical(self.size.1))
    }

    /// Cena gravada até agora.
    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    fn transform(&self) -> Affine {
        Affine::scale(self.scale)
    }

    /// Meio pixel físico, em coordenadas lógicas: ver [`brush`].
    fn half_pixel(&self) -> Affine {
        let half = -0.5 / self.scale;
        Affine::translate((half, half))
    }
}

/// Pincel do `vello` e a transformação dele, em coordenadas lógicas.
///
/// Dois ajustes fazem o gradiente da GPU coincidir com o do `vello_cpu`:
/// - o `fine.wgsl` avalia o gradiente no canto do pixel, e o `vello_cpu` no centro: o gradiente
///   anda meio pixel físico (`half_pixel`). As imagens o `fine.wgsl` já amostra no centro;
/// - o ramp do `vello` 0.11 ignora o offset do primeiro stop e o trata como 0: um stop
///   repetido em 0 mantém a cor fixa até o primeiro offset.
fn brush(paint: &Paint, half_pixel: Affine) -> (Brush, Option<Affine>) {
    match paint {
        Paint::Solid(color) => (Brush::Solid(*color), None),
        Paint::Image(paint) => (
            Brush::Image(ImageBrush {
                image: paint.image.clone(),
                sampler: crate::cpu::sampler(),
            }),
            Some(paint.transform),
        ),
        Paint::Gradient(gradient) => {
            let mut gradient = gradient.clone();
            if let Some(first) = gradient.stops.first().copied()
                && first.offset > 0.0
            {
                gradient.stops.insert(
                    0,
                    ColorStop {
                        offset: 0.0,
                        ..first
                    },
                );
            }
            (Brush::Gradient(gradient), Some(half_pixel))
        }
    }
}

impl Canvas for GpuCanvas {
    fn size(&self) -> (u32, u32) {
        self.size
    }

    fn fill_path(&mut self, path: &BezPath, paint: &Paint) {
        let (brush, brush_transform) = brush(paint, self.half_pixel());
        self.scene.fill(
            Fill::NonZero,
            self.transform(),
            &brush,
            brush_transform,
            path,
        );
    }

    fn fill_rect(&mut self, rect: Rect, paint: &Paint) {
        let (brush, brush_transform) = brush(paint, self.half_pixel());
        self.scene.fill(
            Fill::NonZero,
            self.transform(),
            &brush,
            brush_transform,
            &rect,
        );
    }

    fn stroke_path(&mut self, path: &BezPath, stroke: &Stroke, paint: &Paint) {
        let (brush, brush_transform) = brush(paint, self.half_pixel());
        self.scene
            .stroke(stroke, self.transform(), &brush, brush_transform, path);
    }

    fn draw_glyphs(&mut self, run: &GlyphRun<'_>, paint: &Paint) {
        let scale = self.scale as f32;
        // Os glifos vão sem transformação; o pincel volta para as coordenadas lógicas.
        let (brush, brush_transform) = brush(paint, self.half_pixel());
        let brush_transform = brush_transform.map(|t| self.transform() * t);
        let glyphs = run.glyphs.iter().map(|g| vello::Glyph {
            id: g.id,
            x: g.x * scale,
            y: g.y * scale,
        });
        self.scene
            .draw_glyphs(run.font)
            .font_size(run.font_size * scale)
            .normalized_coords(run.normalized_coords)
            .hint(true)
            .glyph_transform((run.skew != 0.0).then(|| Affine::skew(f64::from(run.skew), 0.0)))
            .brush(&brush)
            .brush_transform(brush_transform)
            .draw(Fill::NonZero, glyphs);
    }

    fn push_clip(&mut self, path: &BezPath) {
        let transform = self.transform();
        self.scene.push_clip_layer(Fill::NonZero, transform, path);
    }

    fn pop_clip(&mut self) {
        self.scene.pop_layer();
    }

    fn push_opacity(&mut self, alpha: f32) {
        let (width, height) = self.size;
        let screen = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
        let transform = self.transform();
        self.scene
            .push_layer(Fill::NonZero, Mix::Normal, alpha, transform, &screen);
    }

    fn pop_opacity(&mut self) {
        self.scene.pop_layer();
    }
}

impl fmt::Debug for GpuCanvas {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GpuCanvas")
            .field("size", &self.size)
            .field("scale", &self.scale)
            .finish_non_exhaustive()
    }
}

/// Erro ao iniciar ou usar a GPU.
#[derive(Debug)]
pub enum GpuError {
    /// Nenhum adaptador disponível (sem GPU e sem driver em software, como o lavapipe).
    NoAdapter(wgpu::RequestAdapterError),
    /// O adaptador recusou criar o dispositivo.
    Device(wgpu::RequestDeviceError),
    /// Falha do `vello` ao criar os shaders ou ao renderizar.
    Vello(vello::Error),
    /// Falha ao ler a textura de volta.
    Readback(String),
}

impl fmt::Display for GpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoAdapter(e) => write!(f, "nenhum adaptador de GPU disponível: {e}"),
            Self::Device(e) => write!(f, "falha ao criar o dispositivo da GPU: {e}"),
            Self::Vello(e) => write!(f, "falha no vello: {e}"),
            Self::Readback(e) => write!(f, "falha ao ler a imagem da GPU: {e}"),
        }
    }
}

impl std::error::Error for GpuError {}

impl From<vello::Error> for GpuError {
    fn from(e: vello::Error) -> Self {
        Self::Vello(e)
    }
}

/// Parâmetros de render iguais aos da CPU: antialiasing por área, como o `vello_cpu`.
pub fn render_params(width: u32, height: u32, background: Color) -> RenderParams {
    RenderParams {
        base_color: background,
        width,
        height,
        antialiasing_method: AaConfig::Area,
    }
}

/// Cria o `vello::Renderer` usado pela janela e pelo [`GpuRenderer`].
pub fn new_renderer(device: &wgpu::Device) -> Result<Renderer, GpuError> {
    Ok(Renderer::new(
        device,
        RendererOptions {
            antialiasing_support: AaSupport::area_only(),
            ..RendererOptions::default()
        },
    )?)
}

/// Rasterizador GPU sem janela: renderiza um [`GpuCanvas`] e lê os pixels de volta.
///
/// Serve para comparar GPU e CPU nos testes. O adaptador vem do `wgpu`: a GPU do sistema ou,
/// sem ela, um driver em software (lavapipe no Linux, WARP no Windows). `WGPU_ADAPTER_NAME` e
/// `WGPU_BACKEND` escolhem outro.
pub struct GpuRenderer {
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: Renderer,
}

impl GpuRenderer {
    /// Abre o adaptador padrão e compila os shaders do `vello`.
    pub fn new() -> Result<Self, GpuError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::from_env().unwrap_or_default(),
            flags: wgpu::InstanceFlags::from_build_config().with_env(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(wgpu::util::initialize_adapter_from_env_or_default(
            &instance, None,
        ))
        .map_err(GpuError::NoAdapter)?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("tomni-ui"),
            ..Default::default()
        }))
        .map_err(GpuError::Device)?;
        let renderer = new_renderer(&device)?;
        Ok(Self {
            adapter,
            device,
            queue,
            renderer,
        })
    }

    /// Nome e backend do adaptador em uso, para as mensagens dos testes.
    pub fn adapter_info(&self) -> wgpu::AdapterInfo {
        self.adapter.get_info()
    }

    /// Rasteriza `canvas` sobre `background`, no tamanho físico dele.
    pub fn render(&mut self, canvas: &GpuCanvas, background: Color) -> Result<Pixmap, GpuError> {
        let (width, height) = canvas.physical_size();
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("tomni-ui alvo"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.renderer.render_to_texture(
            &self.device,
            &self.queue,
            canvas.scene(),
            &view,
            &render_params(width, height, background),
        )?;

        // Cada linha da cópia precisa ter um múltiplo de 256 bytes.
        let row = width * 4;
        let padded =
            row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("tomni-ui leitura"),
            size: u64::from(padded) * u64::from(height),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: None,
                },
            },
            size,
        );
        self.queue.submit([encoder.finish()]);

        let (sender, receiver) = std::sync::mpsc::channel();
        buffer.map_async(wgpu::MapMode::Read, .., move |result| {
            // O receptor só some se esta função já tiver retornado com erro.
            let _ = sender.send(result);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| GpuError::Readback(e.to_string()))?;
        receiver
            .recv()
            .map_err(|e| GpuError::Readback(e.to_string()))?
            .map_err(|e| GpuError::Readback(e.to_string()))?;

        let mapped = buffer
            .get_mapped_range(..)
            .map_err(|e| GpuError::Readback(e.to_string()))?;
        let mut data = Vec::with_capacity((row * height) as usize);
        for line in mapped.chunks(padded as usize) {
            data.extend_from_slice(&line[..row as usize]);
        }
        drop(mapped);
        buffer.unmap();

        let (Ok(w), Ok(h)) = (u16::try_from(width), u16::try_from(height)) else {
            return Err(GpuError::Readback(format!(
                "{width}x{height} não cabe num Pixmap"
            )));
        };
        // O vello grava cores com alfa pré-multiplicado, como o `vello_cpu`.
        Ok(Pixmap::from_parts(
            data,
            w,
            h,
            PixelMetadata {
                may_have_transparency: true,
                alpha_type: ImageAlphaType::AlphaPremultiplied,
            },
        ))
    }
}

impl fmt::Debug for GpuRenderer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GpuRenderer")
            .field("adapter", &self.adapter.get_info().name)
            .finish_non_exhaustive()
    }
}
